//! Verified automation health and action observations for the human Dispatch UI.
use buzz_sdk_pkg::{
    dispatch_record::{active_dispatch_records, DispatchRecordFold, DispatchRecordPayload},
    tracker::{parse_dispatch_action, parse_service_heartbeat},
};
use nostr::Event;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub(super) fn observations(
    events: &[Event],
    records: &DispatchRecordFold,
    workspace: Uuid,
    now: u64,
    relay_signer: nostr::PublicKey,
) -> Value {
    let services: BTreeSet<(String, String)> = active_dispatch_records(&records.records)
        .iter()
        .filter_map(|value| match &value.record.payload {
            DispatchRecordPayload::Automation { controls } => {
                Some((controls.service_id.clone(), controls.service_pubkey.clone()))
            }
            _ => None,
        })
        .collect();
    let mut latest = BTreeMap::new();
    for event in events {
        let Ok(value) = parse_service_heartbeat(event, now) else {
            continue;
        };
        let key = (
            value.heartbeat.service_id.clone(),
            value.author_pubkey.clone(),
        );
        if value.stream_id != workspace || !services.contains(&key) {
            continue;
        }
        if latest
            .get(&key)
            .is_none_or(|old: &buzz_sdk_pkg::tracker::TrackerServiceHeartbeat| {
                (value.created_at, &value.event_id) > (old.created_at, &old.event_id)
            })
        {
            latest.insert(key, value);
        }
    }
    let heartbeats=latest.into_values().map(|value|json!({"eventId":value.event_id,"signer":value.author_pubkey,"createdAt":value.created_at,"serviceId":value.heartbeat.service_id,"observedAt":value.heartbeat.observed_at,"health":value.heartbeat.health})).collect::<Vec<_>>();
    let mut signers: BTreeSet<String> = services.into_iter().map(|(_, key)| key).collect();
    signers.insert(relay_signer.to_hex());
    let mut actions = events
        .iter()
        .filter(|event| signers.contains(&event.pubkey.to_hex()))
        .filter_map(|event| parse_dispatch_action(event, now).ok())
        .filter(|value| value.stream_id == workspace)
        .collect::<Vec<_>>();
    actions.sort_by_key(|value| std::cmp::Reverse((value.created_at, value.event_id.clone())));
    let actions=actions.into_iter().take(100).map(|value|json!({"eventId":value.event_id,"signer":value.author_pubkey,"createdAt":value.created_at,"issueId":value.action.issue_id,"requestId":value.action.request_id,"action":value.action.action,"result":value.action.result})).collect::<Vec<_>>();
    let mut outcomes = events
        .iter()
        .filter(|event| {
            event.kind.as_u16() == 40023
                && event.pubkey == relay_signer
                && event.created_at.as_secs() <= now
                && event.verify().is_ok()
                && event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["h", &workspace.to_string()])
                && event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["t", "dispatch-workflow-outcome"])
        })
        .filter_map(|event| {
            let value: Value = serde_json::from_str(&event.content).ok()?;
            if value["v"] != 1
                || value["type"] != "workflow_outcome"
                || value["action"] != "dispatch.assign"
            {
                return None;
            }
            let key = value["request_key"].as_str()?;
            let activation = key.strip_prefix("confirmation:")?;
            Uuid::parse_str(activation).ok()?;
            let confirmation = value["confirmation_event"].as_str()?;
            nostr::EventId::from_hex(confirmation).ok()?;
            if value["confirmation_required"] != true {
                return None;
            }
            Some((
                event.created_at,
                event.id,
                json!({"eventId":event.id.to_hex(), "requestKey":key,
            "source":value["source"], "result":value["result"], "confirmationRequired":true,
            "confirmationEvent":confirmation}),
            ))
        })
        .collect::<Vec<_>>();
    outcomes.sort_by_key(|(created, id, _)| std::cmp::Reverse((*created, *id)));
    let outcomes = outcomes
        .into_iter()
        .take(100)
        .map(|(_, _, value)| value)
        .collect::<Vec<_>>();
    json!({"heartbeats":heartbeats,"actions":actions,"relaySigner":relay_signer.to_hex(),"outcomes":outcomes})
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk_pkg::{
        dispatch_automation::{AutomationControls, AutomationMode},
        dispatch_record::{build_dispatch_record, fold_dispatch_records, DispatchRecord},
        tracker::{
            build_service_heartbeat, ServiceHealth, ServiceHealthStatus, ServiceHeartbeat,
            TrackerAuthorization, TrackerRole,
        },
    };
    use nostr::{Keys, Timestamp};
    #[test]
    fn a_service_name_cannot_spoof_the_expected_heartbeat_signer() {
        let owner = Keys::generate();
        let service = Keys::generate();
        let outsider = Keys::generate();
        let workspace = Uuid::new_v4();
        let now = 100;
        let controls = DispatchRecord {
            v: 1,
            record_id: Uuid::new_v4(),
            revision: 0,
            parent_event: None,
            request_id: Uuid::new_v4(),
            archived: false,
            payload: DispatchRecordPayload::Automation {
                controls: AutomationControls {
                    service_id: "dispatcher".into(),
                    service_pubkey: service.public_key().to_hex(),
                    mode: AutomationMode::Preview,
                    expires_at: None,
                    max_actions_total: 1,
                    max_actions_per_tick: 1,
                    issue_ids: vec![],
                    capture_channels: vec![],
                    routing: vec![],
                    notify_triage: false,
                    unassigned_after_secs: None,
                    notification_channel: workspace,
                },
            },
        };
        let event = build_dispatch_record(workspace, &controls)
            .unwrap()
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(&owner)
            .unwrap();
        let records = fold_dispatch_records(
            [event],
            workspace,
            &TrackerAuthorization::new([(owner.public_key().to_hex(), TrackerRole::Maintainer)]),
            now,
        );
        let beat = ServiceHeartbeat {
            v: 1,
            service_id: "dispatcher".into(),
            instance_id: Uuid::new_v4(),
            observed_at: now,
            cursor: "".into(),
            health: ServiceHealth {
                status: ServiceHealthStatus::Healthy,
                detail: None,
            },
        };
        let real = build_service_heartbeat(workspace, &beat)
            .unwrap()
            .custom_created_at(Timestamp::from(now))
            .sign_with_keys(&service)
            .unwrap();
        let fake = build_service_heartbeat(workspace, &beat)
            .unwrap()
            .custom_created_at(Timestamp::from(now + 1))
            .sign_with_keys(&outsider)
            .unwrap();
        let value = observations(
            &[real.clone(), fake],
            &records,
            workspace,
            now + 1,
            owner.public_key(),
        );
        assert_eq!(value["heartbeats"].as_array().unwrap().len(), 1);
        assert_eq!(value["heartbeats"][0]["eventId"], real.id.to_hex());
    }
    #[test]
    fn confirmation_requires_the_relay_signature_and_workspace() {
        let relay = Keys::generate();
        let outsider = Keys::generate();
        let workspace = Uuid::new_v4();
        let activation = Uuid::new_v4();
        let content = json!({"v":1,"type":"workflow_outcome","action":"dispatch.assign",
            "request_key":format!("confirmation:{activation}"),"source":"a".repeat(64),
            "result":"Autopilot made 3 assignments in the first hour. Keep it on?",
            "confirmation_required":true,"confirmation_event":"b".repeat(64)})
        .to_string();
        let build = |keys: &Keys, scope: Uuid| {
            nostr::EventBuilder::new(nostr::Kind::Custom(40023), &content)
                .tags([
                    nostr::Tag::parse(["h", &scope.to_string()]).unwrap(),
                    nostr::Tag::parse(["t", "dispatch-workflow-outcome"]).unwrap(),
                ])
                .custom_created_at(Timestamp::from(100))
                .sign_with_keys(keys)
                .unwrap()
        };
        let real = build(&relay, workspace);
        let records = fold_dispatch_records([], workspace, &TrackerAuthorization::new([]), 100);
        let value = observations(
            &[
                real.clone(),
                build(&outsider, workspace),
                build(&relay, Uuid::new_v4()),
            ],
            &records,
            workspace,
            100,
            relay.public_key(),
        );
        assert_eq!(value["outcomes"].as_array().unwrap().len(), 1);
        assert_eq!(value["outcomes"][0]["eventId"], real.id.to_hex());
        assert_eq!(value["outcomes"][0]["confirmationEvent"], "b".repeat(64));
    }
}
