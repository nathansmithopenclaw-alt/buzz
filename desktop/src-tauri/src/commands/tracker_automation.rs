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
    let signers: BTreeSet<String> = services.into_iter().map(|(_, key)| key).collect();
    let mut actions = events
        .iter()
        .filter(|event| signers.contains(&event.pubkey.to_hex()))
        .filter_map(|event| parse_dispatch_action(event, now).ok())
        .filter(|value| value.stream_id == workspace)
        .collect::<Vec<_>>();
    actions.sort_by_key(|value| std::cmp::Reverse((value.created_at, value.event_id.clone())));
    let actions=actions.into_iter().take(100).map(|value|json!({"eventId":value.event_id,"signer":value.author_pubkey,"createdAt":value.created_at,"issueId":value.action.issue_id,"requestId":value.action.request_id,"action":value.action.action,"result":value.action.result})).collect::<Vec<_>>();
    json!({"heartbeats":heartbeats,"actions":actions})
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
        let value = observations(&[real.clone(), fake], &records, workspace, now + 1);
        assert_eq!(value["heartbeats"].as_array().unwrap().len(), 1);
        assert_eq!(value["heartbeats"][0]["eventId"], real.id.to_hex());
    }
}
