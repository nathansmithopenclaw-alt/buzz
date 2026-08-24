//! Emitters: turn approved rule-engine plans into signed relay events.
//!
//! All effects flow through here (plan: three emitters + liveness):
//!
//! - **Capture** — kind-40020 triage root with `source_event_id` and the
//!   deterministic pin request ID, signed by the service identity.
//! - **Routing** — task-envelope v2 + kind-40023 dispatch action sharing ONE
//!   `dispatch_request_id` (M18 dispatch path only).
//! - **Escalation / arrival** — channel notices (escalations pair with a
//!   kind-40023 action; arrival notices are message-only).
//! - **Liveness** — kind-40024 heartbeat every tick.
//!
//! Every emission re-checks the fired ledger before submitting and marks it
//! after an accepted receipt, so replays stay at-most-once.

use buzz_sdk::tracker::{
    build_dispatch_action, build_service_heartbeat, build_tracker_issue, DispatchAction,
    DispatchActionKind, IssueRoot, IssueStatus, ServiceHealth, ServiceHealthStatus,
    ServiceHeartbeat, TRACKER_VERSION,
};
use buzz_sdk::{build_message, task_envelope};
use nostr::{Event, EventBuilder};
use uuid::Uuid;

use crate::config::DispatcherConfig;
use crate::error::DispatchError;
use crate::projection::ProjectionStore;
use crate::relay_http::{parse_submit_receipt, RelayHttp};
use crate::rules::{CapturePlan, RuleAction};

/// Signed-event submission helper bound to one relay + config.
pub struct Emitter<'a> {
    relay: &'a RelayHttp,
    config: &'a DispatcherConfig,
    /// Per-process identity for heartbeats.
    instance_id: Uuid,
}

impl<'a> Emitter<'a> {
    /// Build an emitter.
    pub fn new(relay: &'a RelayHttp, config: &'a DispatcherConfig, instance_id: Uuid) -> Self {
        Self {
            relay,
            config,
            instance_id,
        }
    }

    /// Sign and submit one builder; error unless the relay accepts it.
    async fn sign_and_submit(&self, builder: EventBuilder) -> Result<Event, DispatchError> {
        let event = builder
            .sign_with_keys(self.relay.keys())
            .map_err(|error| DispatchError::Identity(format!("signing failed: {error}")))?;
        let raw = self.relay.submit_event(&event).await?;
        let receipt = parse_submit_receipt(&raw)?;
        if !receipt.accepted {
            return Err(DispatchError::Relay(format!(
                "relay rejected {}: {}",
                event.id.to_hex(),
                receipt.message
            )));
        }
        Ok(event)
    }

    /// Emit one pin-capture root. Re-checks the fired ledger + source
    /// pre-check so a stale plan is a no-op.
    pub async fn emit_capture(
        &self,
        store: &mut ProjectionStore,
        plan: &CapturePlan,
    ) -> Result<(), DispatchError> {
        if store.request_fired(&plan.request_id)
            || store.has_root_with_source(&plan.source_event_id)
        {
            return Ok(());
        }
        let root = IssueRoot {
            v: TRACKER_VERSION,
            op: "create".to_string(),
            issue_id: capture_issue_id(&plan.source_event_id),
            identifier: format!("PIN-{}", &plan.source_event_id[..8]),
            title: plan.title.clone(),
            body: format!(
                "Captured from a pin reaction. Source event: `{}`.",
                plan.source_event_id
            ),
            status: IssueStatus::Triage,
            priority: None,
            assignee: None,
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: Some(plan.channel_id),
            source_event_id: Some(plan.source_event_id.clone()),
            workflow: "default".to_string(),
            labels: Vec::new(),
            external_refs: Vec::new(),
            request_id: Some(plan.request_id),
            import: None,
        };
        let builder = build_tracker_issue(self.config.stream_id, &root)?;
        self.sign_and_submit(builder).await?;
        store.mark_request_fired(plan.request_id);
        Ok(())
    }

    /// Emit all planned fold actions. Digest escalations coalesce into one
    /// message per channel (R4 attention budget); each escalation still gets
    /// its own kind-40023 action for the at-most-once ledger.
    pub async fn emit_rule_actions(
        &self,
        store: &mut ProjectionStore,
        actions: &[RuleAction],
    ) -> Result<(), DispatchError> {
        let mut digest_by_channel: std::collections::BTreeMap<Uuid, Vec<String>> =
            std::collections::BTreeMap::new();

        for action in actions {
            match action {
                RuleAction::NotifyTriage {
                    issue,
                    channel_id,
                    request_id,
                } => {
                    if store.request_fired(request_id) {
                        continue;
                    }
                    let content = format!(
                        "New triage issue needs a human: **{}** — {}",
                        issue.identifier, issue.title
                    );
                    let builder = build_message(*channel_id, &content, None, &[], false, &[])?;
                    self.sign_and_submit(builder).await?;
                    store.mark_request_fired(*request_id);
                }
                RuleAction::RouteAssign {
                    issue,
                    label,
                    owner,
                    channel_id,
                    request_id,
                } => {
                    if store.request_fired(request_id) {
                        continue;
                    }
                    self.emit_route(store, issue, label, owner, *channel_id, *request_id)
                        .await?;
                }
                RuleAction::Escalate {
                    issue,
                    threshold_id,
                    channel_id,
                    action: effect,
                    request_id,
                } => {
                    if store.request_fired(request_id) {
                        continue;
                    }
                    match effect {
                        crate::config::ThresholdAction::Digest => {
                            digest_by_channel
                                .entry(*channel_id)
                                .or_default()
                                .push(format!("- **{}** — {}", issue.identifier, issue.title));
                            // Fire the 40023 now; the coalesced message goes out below.
                            self.emit_escalation_action(issue, threshold_id, *request_id)
                                .await?;
                            store.mark_request_fired(*request_id);
                        }
                        _ => {
                            let content = format!(
                                "Escalation `{threshold_id}`: **{}** — {}",
                                issue.identifier, issue.title
                            );
                            let builder =
                                build_message(*channel_id, &content, None, &[], false, &[])?;
                            self.sign_and_submit(builder).await?;
                            self.emit_escalation_action(issue, threshold_id, *request_id)
                                .await?;
                            store.mark_request_fired(*request_id);
                        }
                    }
                }
            }
        }

        for (channel_id, lines) in digest_by_channel {
            let content = format!("Dispatcher digest:\n{}", lines.join("\n"));
            let builder = build_message(channel_id, &content, None, &[], false, &[])?;
            self.sign_and_submit(builder).await?;
        }
        Ok(())
    }

    /// Envelope-v2 + kind-40023 pair sharing `request_id` (M18).
    async fn emit_route(
        &self,
        store: &mut ProjectionStore,
        issue: &crate::rules::IssueRef,
        label: &str,
        owner: &str,
        channel_id: Uuid,
        request_id: Uuid,
    ) -> Result<(), DispatchError> {
        let envelope = task_envelope::TaskEnvelope {
            v: task_envelope::TASK_ENVELOPE_VERSION,
            task_id: route_task_id(&request_id),
            status: task_envelope::TaskStatus::Assigned,
            assignee: owner.to_string(),
            assigner: self.relay.pubkey_hex(),
            title: format!("Auto-route {}: {}", issue.identifier, issue.title),
            deliverables: Vec::new(),
            verification: task_envelope::TaskVerification {
                method: "Automated label routing by the triage dispatcher.".to_string(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: Some(format!("Routed by label `{label}`.")),
            tracker_issue_id: Some(issue.issue_id),
            dispatch_request_id: Some(request_id),
        };
        let human = format!(
            "Auto-routing **{}** to the owner (label `{label}`).",
            issue.identifier
        );
        let builder =
            task_envelope::build_task_message(channel_id, &human, &envelope, None, &[owner])?;
        self.sign_and_submit(builder).await?;

        let action = DispatchAction {
            v: TRACKER_VERSION,
            issue_id: issue.issue_id,
            root_event: issue.root_event.clone(),
            request_id,
            action: DispatchActionKind::Assign,
            threshold_id: None,
            observed_tip: None,
            result: format!("auto-routed by label {label}"),
        };
        let builder = build_dispatch_action(self.config.stream_id, &action)?;
        self.sign_and_submit(builder).await?;
        store.mark_request_fired(request_id);
        Ok(())
    }

    /// Kind-40023 escalation action (stable threshold_id required).
    async fn emit_escalation_action(
        &self,
        issue: &crate::rules::IssueRef,
        threshold_id: &str,
        request_id: Uuid,
    ) -> Result<(), DispatchError> {
        let action = DispatchAction {
            v: TRACKER_VERSION,
            issue_id: issue.issue_id,
            root_event: issue.root_event.clone(),
            request_id,
            action: DispatchActionKind::Escalate,
            threshold_id: Some(threshold_id.to_string()),
            observed_tip: None,
            result: format!("threshold {threshold_id} fired"),
        };
        let builder = build_dispatch_action(self.config.stream_id, &action)?;
        self.sign_and_submit(builder).await?;
        Ok(())
    }

    /// Kind-40024 heartbeat carrying the composite cursor.
    pub async fn emit_heartbeat(&self, store: &ProjectionStore) -> Result<(), DispatchError> {
        let heartbeat = ServiceHeartbeat {
            v: TRACKER_VERSION,
            service_id: self.config.service_id.clone(),
            instance_id: self.instance_id,
            observed_at: current_unix_seconds(),
            cursor: store.cursor_payload()?,
            health: ServiceHealth {
                status: ServiceHealthStatus::Healthy,
                detail: None,
            },
        };
        let builder = build_service_heartbeat(self.config.stream_id, &heartbeat)?;
        self.sign_and_submit(builder).await?;
        Ok(())
    }
}

/// Deterministic issue UUID for a pin capture.
pub fn capture_issue_id(source_event_id: &str) -> Uuid {
    Uuid::new_v5(
        &buzz_sdk::tracker::DISPATCH_REQUEST_NAMESPACE,
        format!("pin-issue:{source_event_id}").as_bytes(),
    )
}

/// Deterministic task UUID for a routing dispatch, stable per request ID so a
/// re-emitted envelope folds to the same task.
pub fn route_task_id(request_id: &Uuid) -> Uuid {
    Uuid::new_v5(
        &buzz_sdk::tracker::DISPATCH_REQUEST_NAMESPACE,
        format!("task:{request_id}").as_bytes(),
    )
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_and_route_ids_are_deterministic() {
        let source = "ab".repeat(32);
        assert_eq!(capture_issue_id(&source), capture_issue_id(&source));
        assert_ne!(
            capture_issue_id(&source),
            capture_issue_id(&"cd".repeat(32))
        );
        let request = Uuid::new_v4();
        assert_eq!(route_task_id(&request), route_task_id(&request));
    }
}
