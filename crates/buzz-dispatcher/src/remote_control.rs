//! Approval-gated automation. This path never replays work before reading current controls.
use crate::{
    config::DispatcherConfig,
    emit::Emitter,
    error::DispatchError,
    projection::ProjectionStore,
    relay_http::{parse_submit_receipt, RelayHttp},
    remote_plan, rollout_budget,
    run::query_paginated_events,
};
use buzz_sdk::{
    dispatch_audience::validate_dispatch_audience,
    dispatch_automation::AutomationControls,
    dispatch_record::{
        active_dispatch_records, fold_dispatch_records, DispatchRecordPayload,
        DispatchRecordProjection,
    },
    task_authority::fold_authorized_task_events,
    task_envelope::{TaskEnvelopeEvent, TaskStatus},
    tracker::{fold_tracker_events, parse_tracker_event, TrackerAuthorization, TrackerRole},
};
use nostr::{Event, Kind};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

fn fail(message: impl Into<String>) -> DispatchError {
    DispatchError::Other(message.into())
}
fn tag(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).cloned())
            .flatten()
    })
}
async fn membership(relay: &RelayHttp, channel: Uuid) -> Result<Event, DispatchError> {
    let mut events =
        query_paginated_events(relay, json!({"kinds":[39002],"#d":[channel.to_string()]})).await?;
    events.retain(|event| {
        event.kind.as_u16() == 39002
            && tag(event, "d") == Some(channel.to_string())
            && event.verify().is_ok()
    });
    events
        .into_iter()
        .max_by_key(|event| (event.created_at, event.id))
        .ok_or_else(|| fail(format!("Current membership is unavailable for {channel}")))
}
fn member(event: &Event, key: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("p")
            && values.get(1).map(String::as_str) == Some(key)
    })
}
fn authorization(event: &Event, config: &DispatcherConfig) -> TrackerAuthorization {
    // Remote configuration may only narrow host roles. A revoked member cannot
    // keep issuing automation approvals using an old local role entry.
    let current = workspace_authorization(event);
    let roles = config.roles.iter().filter_map(|(key, role)| {
        current.role(key).map(|current| {
            (
                key.clone(),
                if current == TrackerRole::Maintainer {
                    role.to_tracker_role()
                } else {
                    TrackerRole::Member
                },
            )
        })
    });
    TrackerAuthorization::new(roles)
}
fn workspace_authorization(event: &Event) -> TrackerAuthorization {
    let mut roles = vec![(event.pubkey.to_hex(), TrackerRole::Maintainer)];
    roles.extend(event.tags.iter().filter_map(|tag| {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("p") {
            return None;
        };
        let key = values.get(1)?.clone();
        let role = match values.get(3).map(String::as_str) {
            Some("owner" | "admin" | "bot") => TrackerRole::Maintainer,
            _ => TrackerRole::Member,
        };
        Some((key, role))
    }));
    TrackerAuthorization::new(roles)
}
async fn current_approval(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    now: u64,
) -> Result<(Option<DispatchRecordProjection>, Event, Vec<Event>), DispatchError> {
    let membership = membership(relay, config.stream_id).await?;
    if !member(&membership, &relay.pubkey_hex()) {
        return Err(fail("The service is no longer a workspace member"));
    }
    let events = query_paginated_events(
        relay,
        json!({"kinds":[40026,40027],"#h":[config.stream_id.to_string()]}),
    )
    .await?;
    let present = authorization(&membership, config);
    let mut historical = workspace_authorization(&membership);
    buzz_sdk::dispatch_admission::apply_dispatch_admissions(
        &events,
        config.stream_id,
        membership.pubkey,
        &mut historical,
        now,
    );
    let records = fold_dispatch_records(events.clone(), config.stream_id, &historical, now);
    let approval=active_dispatch_records(&records.records).into_iter().filter(|value|present.role(&value.author)==Some(TrackerRole::Maintainer)).find(|value|matches!(&value.record.payload,DispatchRecordPayload::Automation {controls} if controls.service_id==config.service_id && controls.service_pubkey==relay.pubkey_hex())).cloned();
    Ok((approval, membership, events))
}
fn controls(value: &DispatchRecordProjection) -> &AutomationControls {
    match &value.record.payload {
        DispatchRecordPayload::Automation { controls } => controls,
        _ => unreachable!("selected automation record"),
    }
}
fn host_scope(
    config: &DispatcherConfig,
    controls: &AutomationControls,
) -> Result<(), DispatchError> {
    if controls
        .capture_channels
        .iter()
        .any(|channel| !config.monitored_channels.contains(channel))
    {
        return Err(fail("Approval capture channels exceed the host allowlist"));
    }
    if controls.notification_channel != config.decision_surface.channel {
        return Err(fail(
            "Approval notice channel differs from the host decision surface",
        ));
    }
    if controls.routing.iter().any(|route| {
        !config.routing.iter().any(|allowed| {
            allowed.owner == route.executor
                && allowed.label == route.label
                && allowed.channel.or(config.default_channel) == Some(route.channel)
        })
    }) {
        return Err(fail("Approval routing exceeds the host allowlist"));
    }
    Ok(())
}
/// A complete tick uses fresh membership, accepted history and a durable allowance.
/// Preview and paused modes emit health observations only. No assignment is implied
/// by writing an approval; a separately configured host must consume it.
pub async fn tick(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    emitter: &Emitter<'_>,
    store: &mut ProjectionStore,
) -> Result<(), DispatchError> {
    let now = unix();
    let (approval, workspace, record_events) = current_approval(config, relay, now).await?;
    let Some(approval) = approval else {
        return emitter
            .emit_heartbeat_detail(store, false, Some("controls:missing; effects held".into()))
            .await;
    };
    let control = controls(&approval);
    host_scope(config, control)?;
    let mut auth = workspace_authorization(&workspace);
    // Full bounded accepted history prevents a newly enabled source family from
    // inheriting an unrelated high-water cursor and skipping older observations.
    let events = query_paginated_events(
        relay,
        json!({"kinds":[40020,40021,40022,40023,40027],"#h":[config.stream_id.to_string()]}),
    )
    .await?;
    let history = events
        .iter()
        .chain(record_events.iter())
        .cloned()
        .collect::<Vec<_>>();
    let admission = buzz_sdk::dispatch_admission::apply_dispatch_admissions(
        &history,
        config.stream_id,
        workspace.pubkey,
        &mut auth,
        now,
    );
    if controls(&approval).mode == buzz_sdk::dispatch_automation::AutomationMode::Active
        && (admission.unsealed > 0 || !admission.diagnostics.is_empty())
    {
        return Err(fail("Automation is held: historical authorization needs complete relay admission evidence or an explicit legacy baseline"));
    }
    store.ingest(events.clone(), now);
    store.observe_fired_requests(
        events
            .iter()
            .filter(|event| event.pubkey.to_hex() == relay.pubkey_hex())
            .filter_map(|event| buzz_sdk::tracker::parse_dispatch_action(event, now).ok())
            .map(|action| action.action.request_id),
    );
    let fold = fold_tracker_events(
        events
            .iter()
            .filter_map(|event| parse_tracker_event(event, now).ok()),
        &auth,
    );
    let records = fold_dispatch_records(record_events.clone(), config.stream_id, &auth, now);
    let mut memberships = BTreeMap::from([(config.stream_id, workspace.clone())]);
    let channels: BTreeSet<Uuid> = config
        .routing
        .iter()
        .filter_map(|route| route.channel.or(config.default_channel))
        .chain(std::iter::once(config.stream_id))
        .collect();
    let mut task_events = Vec::new();
    for channel in channels {
        let members = if channel == config.stream_id {
            workspace.clone()
        } else {
            membership(relay, channel).await?
        };
        if members.pubkey != workspace.pubkey || !member(&members, &relay.pubkey_hex()) {
            return Err(fail(
                "Task history has an unavailable or different membership authority",
            ));
        }
        let fetched = query_paginated_events(
            relay,
            json!({"kinds":[9],"#h":[channel.to_string()],"#t":["buzz-task"]}),
        )
        .await?;
        let receipts =
            query_paginated_events(relay, json!({"kinds":[40027],"#h":[channel.to_string()]}))
                .await?;
        let task_history = fetched.into_iter().chain(receipts).collect::<Vec<_>>();
        let (admitted, coverage) = buzz_sdk::dispatch_admission::admitted_task_events(
            &task_history,
            channel,
            &members,
            now,
        );
        if control.mode == buzz_sdk::dispatch_automation::AutomationMode::Active
            && (coverage.unsealed > 0 || !coverage.diagnostics.is_empty())
        {
            return Err(fail("Automation is held: task history needs complete relay admission evidence or an explicit legacy baseline"));
        }
        task_events.extend(admitted);
        memberships.insert(channel, members);
    }
    let tasks = fold_authorized_task_events(task_events);
    let candidates = remote_plan::plan(
        control,
        config.stream_id,
        &fold,
        &records,
        &tasks,
        store,
        now,
    );
    let ready = candidates
        .iter()
        .filter(|candidate| candidate.action.is_some())
        .count();
    let mut actions = 0usize;
    if control.allows_effects(now) {
        let reserved = rollout_budget::requests(&config.state_dir, approval.record.record_id)?;
        for pending in store.pending_outbox().to_vec() {
            if !reserved.contains(&pending.request_id) {
                continue;
            }
            if actions >= usize::from(control.max_actions_per_tick) {
                break;
            }
            confirm_approval(config, relay, &approval).await?;
            publish_pending(config, relay, store, &pending, &workspace, &memberships).await?;
            actions += 1;
        }
        for candidate in &candidates {
            let Some(action) = candidate.action.as_ref() else {
                continue;
            };
            if actions >= usize::from(control.max_actions_per_tick) {
                break;
            }
            let request = remote_plan::request_id(action);
            if store.request_fired(&request)
                || store
                    .pending_outbox()
                    .iter()
                    .any(|value| value.request_id == request)
            {
                continue;
            }
            confirm_approval(config, relay, &approval).await?;
            let issue = fold
                .issues
                .get(&(config.stream_id, candidate.issue_id))
                .ok_or_else(|| fail("Candidate disappeared"))?;
            let (channel, readers) = match action {
                crate::rules::RuleAction::RouteAssign {
                    channel_id, owner, ..
                } => (
                    *channel_id,
                    vec![owner.clone(), issue.assignee.clone().unwrap_or_default()],
                ),
                crate::rules::RuleAction::NotifyTriage { channel_id, .. }
                | crate::rules::RuleAction::Escalate { channel_id, .. } => (*channel_id, vec![]),
            };
            let destination = membership(relay, channel).await?;
            validate_dispatch_audience(
                &workspace,
                &destination,
                config.stream_id,
                channel,
                &relay.pubkey_hex(),
                &readers,
            )
            .map_err(fail)?;
            if !rollout_budget::reserve(
                &config.state_dir,
                approval.record.record_id,
                request,
                usize::from(control.max_actions_total),
            )? {
                break;
            }
            if matches!(action, crate::rules::RuleAction::RouteAssign { .. }) {
                let signed = emitter.bounded_route_events(issue, action)?;
                store.queue_outbox(
                    request,
                    signed
                        .iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()?,
                )?;
                let pending = store
                    .pending_outbox()
                    .iter()
                    .find(|entry| entry.request_id == request)
                    .cloned()
                    .ok_or_else(|| fail("Signed route was not retained"))?;
                publish_pending(config, relay, store, &pending, &workspace, &memberships).await?;
            } else {
                emitter
                    .emit_rule_actions(store, std::slice::from_ref(action))
                    .await?;
            }
            actions += 1;
        }
        // Pins are evaluated only after issue actions; the shared total and per-tick
        // allowance bounds both families, including retries after restart.
        if actions < usize::from(control.max_actions_per_tick) {
            let approval_start = record_events
                .iter()
                .find(|event| event.id.to_hex() == approval.root_event)
                .map(|event| event.created_at.as_secs())
                .ok_or_else(|| fail("Approval root is unavailable"))?;
            for channel in &control.capture_channels {
                let source_membership = membership(relay, *channel).await?;
                validate_dispatch_audience(
                    &source_membership,
                    &workspace,
                    *channel,
                    config.stream_id,
                    &relay.pubkey_hex(),
                    &[],
                )
                .map_err(fail)?;
                let reactions = query_paginated_events(
                    relay,
                    json!({"kinds":[7],"#h":[channel.to_string()],"since":approval_start}),
                )
                .await?;
                for reaction in reactions {
                    if actions >= usize::from(control.max_actions_per_tick) {
                        break;
                    }
                    if reaction.verify().is_err()
                        || !member(&source_membership, &reaction.pubkey.to_hex())
                        || reaction.content.trim().trim_end_matches('\u{fe0f}')
                            != config.pin_emoji.trim().trim_end_matches('\u{fe0f}')
                    {
                        continue;
                    }
                    let Some(source_id) = tag(&reaction, "e") else {
                        continue;
                    };
                    if source_id.len() != 64
                        || !source_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        continue;
                    }
                    let request = buzz_sdk::tracker::pin_capture_request_id(&source_id);
                    if store.request_fired(&request)
                        || store.has_root_with_source(&source_id)
                        || store
                            .pending_outbox()
                            .iter()
                            .any(|entry| entry.request_id == request)
                    {
                        continue;
                    }
                    let source = query_paginated_events(
                        relay,
                        json!({"ids":[source_id],"kinds":[9],"#h":[channel.to_string()]}),
                    )
                    .await?
                    .into_iter()
                    .find(|event| {
                        event.id.to_hex() == source_id
                            && event.verify().is_ok()
                            && tag(event, "h") == Some(channel.to_string())
                    });
                    let Some(source) = source else { continue };
                    if source.content.trim().is_empty() || source.content.len() > 16000 {
                        continue;
                    }
                    confirm_approval(config, relay, &approval).await?;
                    if !rollout_budget::reserve(
                        &config.state_dir,
                        approval.record.record_id,
                        request,
                        usize::from(control.max_actions_total),
                    )? {
                        break;
                    }
                    let title = source
                        .content
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("Pinned conversation")
                        .trim()
                        .chars()
                        .take(180)
                        .collect();
                    let plan = crate::rules::CapturePlan {
                        channel_id: *channel,
                        source_event_id: source_id,
                        request_id: request,
                        title,
                    };
                    emitter
                        .emit_capture_with_context(store, &plan, &source.content)
                        .await?;
                    actions += 1;
                }
            }
        }
    }
    store.commit()?;
    let spent = rollout_budget::requests(&config.state_dir, approval.record.record_id)?.len();
    let detail=json!({"controls":control.mode,"approval":approval.event_id,"ready":ready,"actions":actions,"spent":spent,"limit":control.max_actions_total,"pending":store.pending_outbox().len(),"expired":control.expires_at.is_some_and(|deadline|deadline<=now)}).to_string();
    emitter
        .emit_heartbeat_detail(store, false, Some(detail))
        .await
}
async fn confirm_approval(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    expected: &DispatchRecordProjection,
) -> Result<(), DispatchError> {
    let now = unix();
    let (current, _, _) = current_approval(config, relay, now).await?;
    if current.as_ref().is_none_or(|current| {
        current.event_id != expected.event_id || !controls(current).allows_effects(now)
    }) {
        return Err(fail(
            "Automation approval changed, paused or expired; pending effects were retained",
        ));
    }
    Ok(())
}
async fn publish_pending(
    config: &DispatcherConfig,
    relay: &RelayHttp,
    store: &mut ProjectionStore,
    pending: &crate::projection::OutboxEntry,
    _workspace: &Event,
    _memberships: &BTreeMap<Uuid, Event>,
) -> Result<(), DispatchError> {
    let workspace = membership(relay, config.stream_id).await?;
    if !member(&workspace, &relay.pubkey_hex()) {
        return Err(fail("Service membership was revoked; pending effects held"));
    }
    for raw in &pending.events {
        let event: Event = serde_json::from_value(raw.clone())?;
        if event.pubkey.to_hex() != relay.pubkey_hex() || event.verify().is_err() {
            return Err(fail(
                "Outbox signer or signature differs from the configured service",
            ));
        }
        if event.kind == Kind::Custom(40020) {
            let root: buzz_sdk::tracker::IssueRoot = serde_json::from_str(&event.content)?;
            let source_channel = root
                .home_channel_id
                .ok_or_else(|| fail("Pending capture has no source conversation"))?;
            let source_members = membership(relay, source_channel).await?;
            validate_dispatch_audience(
                &source_members,
                &workspace,
                source_channel,
                config.stream_id,
                &relay.pubkey_hex(),
                &[],
            )
            .map_err(fail)?;
        }
        if event.kind == Kind::Custom(9) {
            let channel = tag(&event, "h")
                .and_then(|value| Uuid::parse_str(&value).ok())
                .ok_or_else(|| fail("Outbox message has no channel"))?;
            let destination = membership(relay, channel).await?;
            validate_dispatch_audience(
                &workspace,
                &destination,
                config.stream_id,
                channel,
                &relay.pubkey_hex(),
                &[],
            )
            .map_err(fail)?;
            if let Ok(task) = TaskEnvelopeEvent::from_event(&event) {
                if task.envelope.status != TaskStatus::Assigned {
                    return Err(fail(
                        "Automation outbox contains an unexpected task transition",
                    ));
                }
                let issue_id = task
                    .envelope
                    .tracker_issue_id
                    .ok_or_else(|| fail("Assignment has no issue"))?;
                let history = query_paginated_events(
                    relay,
                    json!({"kinds":[40020,40021,40022],"#h":[config.stream_id.to_string()]}),
                )
                .await?;
                let fold = fold_tracker_events(
                    history
                        .iter()
                        .filter_map(|event| parse_tracker_event(event, unix()).ok()),
                    &workspace_authorization(&workspace),
                );
                let linked = fold
                    .issues
                    .get(&(config.stream_id, issue_id))
                    .is_some_and(|issue| {
                        issue.external_refs.iter().any(|reference| {
                            reference.kind == "task_event"
                                && reference.reference == event.id.to_hex()
                        })
                    });
                if !linked {
                    return Err(fail("Assignment was retained but its issue link is not canonical; no executor was woken"));
                }
                let reviewer = task
                    .envelope
                    .authority
                    .as_ref()
                    .and_then(|value| value.reviewer.clone())
                    .ok_or_else(|| fail("Assignment has no independent reviewer"))?;
                validate_dispatch_audience(
                    &workspace,
                    &destination,
                    config.stream_id,
                    channel,
                    &relay.pubkey_hex(),
                    &[task.envelope.assignee, reviewer],
                )
                .map_err(fail)?;
            }
        }
        let receipt = parse_submit_receipt(&relay.submit_event(&event).await?)?;
        if !receipt.accepted && !receipt.message.starts_with("duplicate") {
            return Err(fail(format!(
                "Pending automation was rejected: {}",
                receipt.message
            )));
        }
    }
    store.mark_request_fired(pending.request_id);
    store.persist_ledger()?;
    store.complete_outbox(&pending.request_id)?;
    Ok(())
}
fn unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(u64::MAX)
}
