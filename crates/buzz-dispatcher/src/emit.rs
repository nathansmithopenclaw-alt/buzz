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
//! Every recorded emission follows persist-before-publish (M2, per
//! DESIGN_C_CONSUMERS.md): sign → persist the signed bytes to the outbox →
//! publish → make the fired ledger durable → clear the outbox entry. A crash
//! at any point replays the outbox on restart; the fired ledger keeps each
//! request at-most-once. Digest escalations flush the human-visible message
//! BEFORE their 40023 fires (M4), so a failed flush leaves the epoch
//! un-fired and it re-qualifies next tick instead of being lost forever.

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

/// Bound for degraded-heartbeat details (the SDK caps `detail` at
/// MAX_CURSOR_BYTES = 512; stay comfortably under).
const MAX_HEARTBEAT_DETAIL_LEN: usize = 400;

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

    /// Sign and submit one builder; error unless the relay accepts it. Used
    /// for UNRECORDED surfaces (heartbeat, digest flush) that carry no
    /// request id.
    async fn sign_and_submit(&self, builder: EventBuilder) -> Result<Event, DispatchError> {
        let event = builder
            .sign_with_keys(self.relay.keys())
            .map_err(|error| DispatchError::Identity(format!("signing failed: {error}")))?;
        self.submit_accepted(&event).await?;
        Ok(event)
    }

    /// Submit one already-signed event and require an accepted receipt.
    async fn submit_accepted(&self, event: &Event) -> Result<(), DispatchError> {
        let raw = self.relay.submit_event(event).await?;
        let receipt = parse_submit_receipt(&raw)?;
        if !receipt.accepted {
            return Err(DispatchError::Relay(format!(
                "relay rejected {}: {}",
                event.id.to_hex(),
                receipt.message
            )));
        }
        Ok(())
    }

    /// Persist-before-publish pipeline for one recorded request (M2): sign
    /// all builders, persist the signed bytes to the outbox, publish in
    /// order, then make the fired ledger durable and clear the outbox entry.
    /// A crash between publish and ledger-commit replays from the outbox on
    /// restart (duplicate-tolerant), never re-plans the request.
    async fn publish_recorded(
        &self,
        store: &mut ProjectionStore,
        request_id: Uuid,
        builders: Vec<EventBuilder>,
    ) -> Result<(), DispatchError> {
        let mut signed = Vec::with_capacity(builders.len());
        for builder in builders {
            signed.push(
                builder
                    .sign_with_keys(self.relay.keys())
                    .map_err(|error| DispatchError::Identity(format!("signing failed: {error}")))?,
            );
        }
        let mut payloads = Vec::with_capacity(signed.len());
        for event in &signed {
            payloads.push(serde_json::to_value(event)?);
        }
        store.queue_outbox(request_id, payloads)?;
        for event in &signed {
            self.submit_accepted(event).await?;
        }
        store.mark_request_fired(request_id);
        store.persist_ledger()?;
        store.complete_outbox(&request_id)?;
        Ok(())
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
            identifier: format!(
                "PIN-{}",
                plan.source_event_id
                    .get(..8)
                    .unwrap_or(&plan.source_event_id)
            ),
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
        self.publish_recorded(store, plan.request_id, vec![builder])
            .await
    }

    /// Emit all planned fold actions. Digest escalations coalesce into one
    /// message per channel (R4 attention budget); each escalation still gets
    /// its own kind-40023 action for the at-most-once ledger.
    ///
    /// M4 ordering: the coalesced digest message flushes BEFORE its
    /// escalations fire the 40023 + mark the ledger. A failed flush leaves
    /// those requests un-fired, so the epoch re-qualifies on a later tick —
    /// the escalation is delayed, never lost.
    pub async fn emit_rule_actions(
        &self,
        store: &mut ProjectionStore,
        actions: &[RuleAction],
    ) -> Result<(), DispatchError> {
        // Digest work staged per channel: human-visible lines plus the
        // escalation fires that belong to that digest.
        let mut digest_lines: std::collections::BTreeMap<Uuid, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut digest_fires: std::collections::BTreeMap<
            Uuid,
            Vec<(crate::rules::IssueRef, String, Uuid)>,
        > = std::collections::BTreeMap::new();

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
                    let builder = build_message(*channel_id, &content, None, &[], false, &[], &[])?;
                    self.publish_recorded(store, *request_id, vec![builder])
                        .await?;
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
                    let builders =
                        self.route_builders(issue, label, owner, *channel_id, *request_id)?;
                    self.publish_recorded(store, *request_id, builders).await?;
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
                            digest_lines
                                .entry(*channel_id)
                                .or_default()
                                .push(format!("- **{}** — {}", issue.identifier, issue.title));
                            digest_fires.entry(*channel_id).or_default().push((
                                issue.clone(),
                                threshold_id.clone(),
                                *request_id,
                            ));
                        }
                        _ => {
                            let content = format!(
                                "Escalation `{threshold_id}`: **{}** — {}",
                                issue.identifier, issue.title
                            );
                            let message =
                                build_message(*channel_id, &content, None, &[], false, &[], &[])?;
                            let record = escalation_action_builder(
                                self.config.stream_id,
                                issue,
                                threshold_id,
                                *request_id,
                            )?;
                            // Notice first, then the 40023 record — published
                            // atomically under one outbox entry.
                            self.publish_recorded(store, *request_id, vec![message, record])
                                .await?;
                        }
                    }
                }
            }
        }

        // Flush each channel digest, then fire its escalations (M4).
        for (channel_id, lines) in digest_lines {
            let fires = digest_fires.remove(&channel_id).unwrap_or_default();
            let content = format!("Dispatcher digest:\n{}", lines.join("\n"));
            let builder = build_message(channel_id, &content, None, &[], false, &[], &[])?;
            if let Err(error) = self.sign_and_submit(builder).await {
                // Leave this channel's fires un-marked: the epoch
                // re-qualifies next tick and re-joins the digest.
                tracing::warn!(
                    %error,
                    channel = %channel_id,
                    count = fires.len(),
                    "digest flush failed; escalations deferred to next tick"
                );
                continue;
            }
            for (issue, threshold_id, request_id) in fires {
                let record = escalation_action_builder(
                    self.config.stream_id,
                    &issue,
                    &threshold_id,
                    request_id,
                )?;
                self.publish_recorded(store, request_id, vec![record])
                    .await?;
            }
        }
        Ok(())
    }

    /// Envelope-v2 + kind-40023 builders sharing `request_id` (M18), in
    /// publication order.
    fn route_builders(
        &self,
        issue: &crate::rules::IssueRef,
        label: &str,
        owner: &str,
        channel_id: Uuid,
        request_id: Uuid,
    ) -> Result<Vec<EventBuilder>, DispatchError> {
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
        let envelope_builder =
            task_envelope::build_task_message(channel_id, &human, &envelope, None, &[owner])?;

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
        let action_builder = build_dispatch_action(self.config.stream_id, &action)?;
        Ok(vec![envelope_builder, action_builder])
    }

    /// Kind-40024 heartbeat carrying the composite cursor. `degraded` names
    /// emission failures observed this tick — the heartbeat reports them
    /// instead of a hardcoded Healthy, so liveness consumers see impairment.
    pub async fn emit_heartbeat(
        &self,
        store: &ProjectionStore,
        degraded: Option<String>,
    ) -> Result<(), DispatchError> {
        let health = match degraded {
            Some(mut detail) => {
                detail.truncate(MAX_HEARTBEAT_DETAIL_LEN);
                ServiceHealth {
                    status: ServiceHealthStatus::Degraded,
                    detail: Some(detail),
                }
            }
            None => ServiceHealth {
                status: ServiceHealthStatus::Healthy,
                detail: None,
            },
        };
        let heartbeat = ServiceHeartbeat {
            v: TRACKER_VERSION,
            service_id: self.config.service_id.clone(),
            instance_id: self.instance_id,
            observed_at: current_unix_seconds(),
            cursor: store.cursor_payload()?,
            health,
        };
        let builder = build_service_heartbeat(self.config.stream_id, &heartbeat)?;
        self.sign_and_submit(builder).await?;
        Ok(())
    }
}

/// Kind-40023 escalation action builder (stable threshold_id required).
fn escalation_action_builder(
    stream_id: Uuid,
    issue: &crate::rules::IssueRef,
    threshold_id: &str,
    request_id: Uuid,
) -> Result<EventBuilder, DispatchError> {
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
    Ok(build_dispatch_action(stream_id, &action)?)
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
