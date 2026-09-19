//! Relay-owned Dispatch workflow effects. No HTTP loopback and no agent credentials.
use crate::state::AppState;
use buzz_core::{tenant::TenantContext, CommunityId};
use buzz_db::dispatch_workflow::{DispatchBatch, DispatchLedgerEntry, DispatchSnapshot};
use buzz_sdk::{
    dispatch_admission::*,
    dispatch_record::{
        active_dispatch_records, fold_dispatch_records, DispatchRecord, DispatchRecordPayload,
        DispatchRecordProjection,
    },
    tracker::*,
};
use buzz_workflow::{ActionDef, ActionSinkError};
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;
mod assign;
mod capture;
mod lifecycle;
#[cfg(test)]
mod postgres_tests;
pub(crate) use lifecycle::transition;

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn tag(key: &str, value: &str) -> Result<Tag> {
    Tag::parse([key, value]).map_err(err)
}
fn scope(event: &Event) -> Option<Uuid> {
    event
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some("h"))
        .and_then(|t| t.content())
        .and_then(|s| s.parse().ok())
}

pub(crate) async fn execute(
    state: &Arc<AppState>,
    community: CommunityId,
    run_id: Uuid,
    action: ActionDef,
) -> std::result::Result<serde_json::Value, ActionSinkError> {
    execute_inner(state, community, run_id, action)
        .await
        .map_err(ActionSinkError::Database)
}
async fn execute_inner(
    state: &Arc<AppState>,
    community: CommunityId,
    run_id: Uuid,
    action: ActionDef,
) -> Result<serde_json::Value> {
    let run = state
        .db
        .get_workflow_run(community, run_id)
        .await
        .map_err(err)?;
    let workflow = state
        .db
        .get_workflow(community, run.workflow_id)
        .await
        .map_err(err)?;
    let channel = workflow
        .channel_id
        .ok_or("Dispatch requires a channel-scoped workflow")?;
    if !workflow.enabled || workflow.status != buzz_db::workflow::WorkflowStatus::Active {
        return Ok(serde_json::json!({"skipped":"workflow disabled"}));
    }
    let def: buzz_workflow::WorkflowDef =
        serde_json::from_value(workflow.definition.clone()).map_err(err)?;
    def.validate().map_err(err)?;
    if !def
        .steps
        .iter()
        .any(|step| serde_json::to_value(&step.action).ok() == serde_json::to_value(&action).ok())
    {
        return Err("Dispatch action no longer belongs to the current definition".into());
    }
    let mut ids = Vec::new();
    if let Some(id) = run.trigger_event_id {
        let event = state
            .db
            .get_event_by_id_for_event_write(community, &id)
            .await
            .map_err(err)?
            .ok_or("trigger event missing")?;
        let ctx = buzz_workflow::build_trigger_context(&event);
        if matches!(action, ActionDef::DispatchCreateIssue {})
            && (event.event.kind.as_u16() != 7
                || ctx.emoji != "📌"
                || event.channel_id != Some(channel))
        {
            return Err("capture requires its persisted pin reaction".into());
        }
        ids.push(id);
        if let Ok(source) = hex::decode(ctx.message_id) {
            ids.push(source);
        }
    } else if matches!(action, ActionDef::DispatchCreateIssue {}) {
        return Err("capture requires a reaction event".into());
    }
    let channels = discover_scopes(state, community, channel).await?;
    let owner = hex::encode(&workflow.owner_pubkey);
    let (events, output) = state
        .db
        .dispatch_workflow_transaction(
            community,
            &channels,
            &ids,
            Some((workflow.id, workflow.definition_hash)),
            |snapshot| {
                let mut p = Planner::new(state, snapshot);
                // Includes capture destinations, even when assignment is paused.
                // One shared budget bounds recovery across all locked scopes.
                for workspace in &channels {
                    p.project_lifecycle(*workspace, None)?;
                }
                let record_type = match action {
                    ActionDef::DispatchCreateIssue {} => "create_issue",
                    ActionDef::DispatchEscalate { .. } => "escalate",
                    ActionDef::DispatchAssign {} => "assign",
                    _ => return Err("not a Dispatch action".into()),
                };
                if !p.member(channel, &owner) {
                    p.skip(
                        channel,
                        &p.trigger_identity(&ids, run_id),
                        record_type,
                        "signer not authorized",
                    )?;
                    return Ok(p.batch);
                }
                let records = p.records(channel);
                let mut applicable = false;
                for record in &records {
                    let selected = matches!(
                        (&record.record.payload, &action),
                        (
                            DispatchRecordPayload::Capture { .. },
                            ActionDef::DispatchCreateIssue {}
                        ) | (
                            DispatchRecordPayload::UnownedAlert { .. },
                            ActionDef::DispatchEscalate { .. }
                        ) | (
                            DispatchRecordPayload::Autopilot { .. },
                            ActionDef::DispatchAssign {}
                        )
                    );
                    if !selected {
                        continue;
                    }
                    applicable = true;
                    let identity = p.trigger_identity(&ids, run_id);
                    if !p.admin(channel, &record.author) {
                        p.skip(channel, &identity, record_type, "signer not authorized")?;
                        continue;
                    }
                    match &action {
                        ActionDef::DispatchCreateIssue {} => p.capture(channel, record, &ids)?,
                        ActionDef::DispatchEscalate { after_hours } => {
                            p.escalate(channel, record, *after_hours)?
                        }
                        ActionDef::DispatchAssign {} => p.assign(channel, record)?,
                        _ => {}
                    }
                }
                if !applicable {
                    let identity = p.trigger_identity(&ids, run_id);
                    p.skip(channel, &identity, record_type, "no authorization record")?;
                }
                if p.batch.output.is_null() {
                    p.batch.output = serde_json::json!({"effects":p.batch.ledger.len()});
                }
                Ok(p.batch)
            },
        )
        .await
        .map_err(err)?;
    publish(state, community, events).await?;
    Ok(output)
}

async fn discover_scopes(
    state: &AppState,
    community: CommunityId,
    channel: Uuid,
) -> Result<Vec<Uuid>> {
    let mut scopes = BTreeSet::from([channel]);
    let initial = preliminary_planner(state, community, channel).await?;
    for record in initial.records(channel) {
        if initial.admin(channel, &record.author) {
            if let DispatchRecordPayload::Capture { workspace, .. } = record.record.payload {
                scopes.insert(workspace);
            }
        }
    }
    // Only accepted projections discover destinations; forged or stale revisions
    // cannot manufacture scope. The transaction rechecks every authorization.
    for workspace in scopes.clone() {
        let preliminary = preliminary_planner(state, community, workspace).await?;
        for issue in preliminary.issues(workspace).issues.values() {
            if let Some(home) = issue.home_channel_id {
                scopes.insert(home);
            }
        }
        for record in preliminary.records(workspace) {
            if preliminary.admin(workspace, &record.author) {
                if let DispatchRecordPayload::UnownedAlert {
                    channel: Some(destination),
                    ..
                } = record.record.payload
                {
                    scopes.insert(destination);
                }
            }
        }
    }
    Ok(scopes.into_iter().collect())
}
async fn preliminary_planner(
    state: &AppState,
    community: CommunityId,
    channel: Uuid,
) -> Result<Planner<'_>> {
    let events = state
        .db
        .dispatch_workflow_history(community, channel)
        .await
        .map_err(err)?;
    let members = state
        .db
        .get_members_for_event_write(community, channel)
        .await
        .map_err(err)?
        .into_iter()
        .map(|member| (channel, hex::encode(member.pubkey), member.role))
        .collect();
    Ok(Planner::new(
        state,
        DispatchSnapshot {
            events,
            members,
            ledger: vec![],
            channels: vec![channel],
            now: chrono::Utc::now().timestamp(),
        },
    ))
}

async fn publish(
    state: &Arc<AppState>,
    community: CommunityId,
    events: Vec<buzz_core::StoredEvent>,
) -> Result<()> {
    let host = state
        .db
        .lookup_community_host(community)
        .await
        .map_err(err)?
        .ok_or("community host missing")?;
    let tenant = TenantContext::resolved(community, host);
    for event in events {
        crate::handlers::event::dispatch_persistent_event(
            &tenant,
            state,
            &event,
            event.event.kind.as_u16() as u32,
            &state.relay_keypair.public_key().to_hex(),
            None,
        )
        .await;
    }
    Ok(())
}

struct Planner<'a> {
    state: &'a AppState,
    snapshot: DispatchSnapshot,
    batch: DispatchBatch,
}
impl<'a> Planner<'a> {
    fn new(state: &'a AppState, snapshot: DispatchSnapshot) -> Self {
        Self {
            state,
            snapshot,
            batch: DispatchBatch::default(),
        }
    }
    fn now(&self) -> u64 {
        self.snapshot.now.max(0) as u64
    }
    fn events(&self) -> impl Iterator<Item = &Event> {
        self.snapshot
            .events
            .iter()
            .map(|stored| &stored.event)
            .chain(self.batch.events.iter().map(|(event, _)| event))
    }
    fn member(&self, channel: Uuid, key: &str) -> bool {
        self.snapshot
            .members
            .iter()
            .any(|(c, k, _)| *c == channel && k == key)
    }
    fn admin(&self, channel: Uuid, key: &str) -> bool {
        self.snapshot
            .members
            .iter()
            .any(|(c, k, r)| *c == channel && k == key && matches!(r.as_str(), "owner" | "admin"))
    }
    fn audience(&self, source: Uuid, destination: Uuid) -> bool {
        self.snapshot.channels.contains(&destination)
            && self
                .snapshot
                .members
                .iter()
                .filter(|(c, _, _)| *c == destination)
                .all(|(_, key, _)| self.member(source, key))
    }
    fn authorization(&self, channel: Uuid) -> (TrackerAuthorization, Vec<Event>) {
        let mut auth = TrackerAuthorization::new(
            self.snapshot
                .members
                .iter()
                .filter(|(c, _, _)| *c == channel)
                .map(|(_, key, role)| {
                    (
                        key.clone(),
                        if matches!(role.as_str(), "owner" | "admin" | "bot") {
                            TrackerRole::Maintainer
                        } else {
                            TrackerRole::Member
                        },
                    )
                })
                .chain([(
                    self.state.relay_keypair.public_key().to_hex(),
                    TrackerRole::Maintainer,
                )]),
        );
        // Later actions in the same transaction see recovered lifecycle revisions
        // and their admission receipts, including the new status-age anchor.
        let events = self.events().cloned().collect::<Vec<_>>();
        let coverage = apply_dispatch_admissions(
            &events,
            channel,
            self.state.relay_keypair.public_key(),
            &mut auth,
            self.now(),
        );
        let admitted = events
            .into_iter()
            .filter(|e| {
                scope(e) == Some(channel)
                    && (!requires_dispatch_event_admission(e)
                        || coverage.sealed.contains(&e.id.to_hex()))
            })
            .collect();
        (auth, admitted)
    }
    fn admission_times(&self, channel: Uuid) -> std::collections::BTreeMap<String, u64> {
        let mut auth = TrackerAuthorization::new([]);
        apply_dispatch_admissions(
            &self.events().cloned().collect::<Vec<_>>(),
            channel,
            self.state.relay_keypair.public_key(),
            &mut auth,
            self.now(),
        )
        .observed_at
    }
    fn records(&self, channel: Uuid) -> Vec<DispatchRecordProjection> {
        let (auth, events) = self.authorization(channel);
        let fold = fold_dispatch_records(events, channel, &auth, self.now());
        active_dispatch_records(&fold.records)
            .into_iter()
            .cloned()
            .collect()
    }
    fn issues(&self, workspace: Uuid) -> TrackerFold {
        let (auth, events) = self.authorization(workspace);
        fold_tracker_events(
            events
                .iter()
                .filter_map(|e| parse_tracker_event(e, self.now()).ok()),
            &auth,
        )
    }
    fn has(&self, workspace: Uuid, key: &str) -> bool {
        self.snapshot
            .ledger
            .iter()
            .chain(self.batch.ledger.iter())
            .any(|e| e.workspace == workspace && e.key == key)
    }
    fn claim(&mut self, workspace: Uuid, key: String, action: &str) -> bool {
        if self.has(workspace, &key) {
            return false;
        }
        self.batch.ledger.push(DispatchLedgerEntry {
            key,
            workspace,
            action: action.into(),
            project: None,
            executor: None,
            activation: None,
            created_at: self.snapshot.now,
            confirmation_event: None,
        });
        true
    }
    fn emit(&mut self, channel: Uuid, builder: EventBuilder) -> Result<Event> {
        self.emit_at(channel, builder, self.now())
    }
    fn emit_at(&mut self, channel: Uuid, builder: EventBuilder, timestamp: u64) -> Result<Event> {
        let event = builder
            .custom_created_at(Timestamp::from(timestamp))
            .sign_with_keys(&self.state.relay_keypair)
            .map_err(err)?;
        self.batch.events.push((event.clone(), channel));
        if requires_dispatch_event_admission(&event) {
            let receipt = build_dispatch_admission(&DispatchAdmission {
                v: 1,
                workspace: channel,
                event_id: event.id.to_hex(),
                author: event.pubkey.to_hex(),
                role: "maintainer".into(),
                observed_at: self.now(),
                basis: AdmissionBasis::Admission,
            })?
            .custom_created_at(Timestamp::from(self.now()))
            .sign_with_keys(&self.state.relay_keypair)
            .map_err(err)?;
            self.batch.events.push((receipt, channel));
        }
        Ok(event)
    }
    fn trigger_identity(&self, ids: &[Vec<u8>], run: Uuid) -> String {
        ids.get(1)
            .map(hex::encode)
            .unwrap_or_else(|| run.to_string())
    }
    fn outcome(
        &mut self,
        channel: Uuid,
        key: &str,
        source: &str,
        action: &str,
        result: &str,
        confirmation: Option<String>,
    ) -> Result<()> {
        let value = buzz_sdk::dispatch_autopilot::WorkflowOutcome {
            v: 1,
            record_type: "workflow_outcome".into(),
            action: format!("dispatch.{action}"),
            request_key: key.into(),
            source: source.into(),
            result: result.into(),
            confirmation_required: confirmation.is_some(),
            confirmation_event: confirmation,
        };
        self.emit(
            channel,
            EventBuilder::new(
                Kind::Custom(40023),
                serde_json::to_string(&value).map_err(err)?,
            )
            .tags([
                tag("h", &channel.to_string())?,
                tag("t", "dispatch-workflow-outcome")?,
            ]),
        )?;
        Ok(())
    }
    fn skip(&mut self, channel: Uuid, identity: &str, action: &str, reason: &str) -> Result<()> {
        let key = format!("skip:{action}:{identity}:{reason}");
        if !self.claim(channel, key.clone(), "skipped") {
            return Ok(());
        }
        self.outcome(
            channel,
            &key,
            identity,
            action,
            &format!("skipped: {reason}"),
            None,
        )?;
        self.batch.output = serde_json::json!({"skipped":reason});
        Ok(())
    }

    fn revision(
        &mut self,
        workspace: Uuid,
        issue: &IssueProjection,
        set: RevisionSet,
        refs: Vec<ExternalRef>,
        request: Uuid,
        reason: &str,
    ) -> Result<()> {
        let timestamp = self.now().max(issue.tip_created_at.saturating_add(1));
        if timestamp > self.now().saturating_add(MAX_FUTURE_SKEW_SECONDS) {
            return Err("Tracker revision clock exceeds accepted skew".into());
        }
        self.emit_at(
            workspace,
            build_tracker_revision(
                workspace,
                &IssueRevision {
                    v: 1,
                    op: "revise".into(),
                    issue_id: issue.issue_id,
                    root_event: issue.root_event.clone(),
                    parent_event: issue.tip_event.clone(),
                    revision: issue.revision + 1,
                    set,
                    clear: vec![],
                    relations: SetOperations::default(),
                    external_refs: SetOperations {
                        add: refs,
                        remove: vec![],
                    },
                    reason: Some(reason.into()),
                    request_id: Some(request),
                    import: None,
                },
            )
            .map_err(err)?,
            timestamp,
        )?;
        Ok(())
    }
    fn action_record(
        &mut self,
        workspace: Uuid,
        issue: &IssueProjection,
        request: Uuid,
        action: DispatchActionKind,
        threshold: Option<String>,
        result: &str,
    ) -> Result<String> {
        let event = self.emit(
            workspace,
            build_dispatch_action(
                workspace,
                &DispatchAction {
                    v: 1,
                    issue_id: issue.issue_id,
                    root_event: issue.root_event.clone(),
                    request_id: request,
                    action,
                    threshold_id: threshold,
                    observed_tip: Some(issue.tip_event.clone()),
                    result: result.into(),
                },
            )
            .map_err(err)?,
        )?;
        Ok(event.id.to_hex())
    }
}
