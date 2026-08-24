//! Deterministic rule engine: pure planning with no I/O and no LLM judgment
//! (plan rule R2). Every decision is table-testable against a fold snapshot.
//!
//! Three rule families:
//!
//! - **Capture** — pin reactions (config emoji) in monitored channels become
//!   triage-issue creation plans, collapsed by deterministic UUIDv5 request IDs
//!   plus a `source_event_id` pre-check. Un-pin/reaction-removal is a NO-OP:
//!   the store never watches deletions (binding amendment 1, append-only).
//! - **Triage arrival / label auto-routing** — unassigned triage/todo issues
//!   matching the routing table are dispatched via envelope-v2 + kind-40023
//!   sharing one request ID; the dispatch PREEMPTS the arrival notice.
//!   Exclusively UNROUTED triage arrivals get one arrival notice per triage
//!   entry ("a human needs to triage this").
//! - **Escalation ladder** — stable threshold IDs, at-most-once firing per
//!   `(issue, threshold, qualifying-interval epoch)`.
//!
//! M19: issues carrying import provenance never trigger any rule.

use std::collections::HashSet;

use buzz_sdk::tracker::{
    due_has_passed, pin_capture_request_id, route_request_id, threshold_request_id,
    IssueProjection, IssueStatus, TrackerFold, DISPATCH_REQUEST_NAMESPACE,
};
use nostr::Kind;
use uuid::Uuid;

use crate::config::{DispatcherConfig, Threshold, ThresholdAction, ThresholdCondition};
use crate::projection::ProjectionStore;

/// One planned pin-capture issue creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePlan {
    /// Channel where the pin reaction happened; becomes the issue home channel.
    pub channel_id: Uuid,
    /// Hex id of the pinned message.
    pub source_event_id: String,
    /// Deterministic idempotency request id (`pin:<source>`).
    pub request_id: Uuid,
    /// Title for the captured issue.
    pub title: String,
}

/// Issue identity carried by every planned fold action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    /// Stable issue UUID.
    pub issue_id: Uuid,
    /// Immutable root event id.
    pub root_event: String,
    /// Human-facing identifier.
    pub identifier: String,
    /// Issue title.
    pub title: String,
}

/// One planned fold-driven effect. Emission is the loop's job; planning is
/// pure so replays with an unchanged fired-set produce identical plans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    /// Arrival notice for an UNROUTED triage arrival (plan rule 1): "a human
    /// needs to triage this". Routed arrivals are preempted by the dispatch
    /// envelope, which already carries the owner wake.
    NotifyTriage {
        /// Target issue.
        issue: IssueRef,
        /// Channel receiving the notice.
        channel_id: Uuid,
        /// Idempotency key recorded when the notice is sent.
        request_id: Uuid,
    },
    /// Label auto-routing via the dispatch path (envelope-v2 + kind-40023).
    RouteAssign {
        /// Target issue.
        issue: IssueRef,
        /// Matched routing label.
        label: String,
        /// Owner pubkey receiving the dispatch.
        owner: String,
        /// Channel receiving the dispatch message.
        channel_id: Uuid,
        /// Request id shared by the envelope and the kind-40023 action.
        request_id: Uuid,
    },
    /// Threshold escalation fire.
    Escalate {
        /// Target issue.
        issue: IssueRef,
        /// Stable threshold config id.
        threshold_id: String,
        /// Channel receiving the escalation.
        channel_id: Uuid,
        /// Configured effect (immediate notice, digest coalescing, decision).
        action: ThresholdAction,
        /// Deterministic `(issue, threshold, epoch)` request id.
        request_id: Uuid,
    },
}

/// Plan pin captures from persisted reaction events.
///
/// A capture plan is emitted once per `(channel, source_event_id)`: duplicate
/// reactions in the same batch collapse on the request id, and reactions whose
/// source already has a root (or whose request id was already fired) are
/// skipped.
pub fn plan_captures(config: &DispatcherConfig, store: &ProjectionStore) -> Vec<CapturePlan> {
    let reaction_kind = Kind::Custom(7);
    let mut plans = Vec::new();
    let mut planned: HashSet<Uuid> = HashSet::new();

    for event in store.events() {
        if event.kind != reaction_kind || !emoji_matches(&event.content, &config.pin_emoji) {
            continue;
        }
        let channel_id = match first_tag_uuid(event, "h") {
            Some(channel_id) if config.monitored_channels.contains(&channel_id) => channel_id,
            _ => continue,
        };
        let Some(source_event_id) = first_tag_value(event, "e") else {
            continue;
        };
        // B2: e-tags are publisher-controlled. Only a 64-char lowercase-hex
        // event id may become a capture source; anything else is skipped
        // here (never sliced, never warned about per tick).
        if !is_event_id_hex(&source_event_id) {
            continue;
        }

        let request_id = pin_capture_request_id(&source_event_id);
        // Idempotency belt-and-suspenders: fired ledger first, then the
        // source pre-check that also covers UUIDv5 namespace drift.
        if store.request_fired(&request_id) || store.has_root_with_source(&source_event_id) {
            continue;
        }
        if !planned.insert(request_id) {
            continue;
        }
        let title = format!("Pin capture: {source_event_id}");
        plans.push(CapturePlan {
            channel_id,
            source_event_id,
            request_id,
            title,
        });
    }
    plans
}

/// Plan fold-driven actions for the current projection snapshot.
///
/// Deterministic order: issues by `(stream, issue)` key (BTreeMap iteration),
/// thresholds in config order. `now` is the evaluation timestamp; `fired`
/// request ids (own ledger plus observed external 40023s) suppress replays.
pub fn plan_fold_actions(
    config: &DispatcherConfig,
    fold: &TrackerFold,
    store: &ProjectionStore,
    now: u64,
) -> Vec<RuleAction> {
    let suppressed = store.imported_issue_ids();
    let mut actions = Vec::new();

    for ((stream_id, _), issue) in &fold.issues {
        if *stream_id != config.stream_id || suppressed.contains(&issue.issue_id) {
            continue;
        }
        if issue.status.is_terminal() {
            continue;
        }
        let issue_ref = IssueRef {
            issue_id: issue.issue_id,
            root_event: issue.root_event.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
        };

        // Label auto-routing (plan rule 2): dispatch path only (M18),
        // envelope-v2 + kind-40023 sharing one request id. Routing PREEMPTS
        // the arrival notice (Helm 2026-08-24): the dispatch envelope is
        // channel-visible and carries the owner wake, so a separate notice
        // would be a second ping for the same fact.
        let route = if issue.assignee.is_none()
            && matches!(issue.status, IssueStatus::Triage | IssueStatus::Todo)
        {
            matching_route(config, issue)
        } else {
            None
        };

        if let Some(route) = route {
            let request_id = route_request_id(issue.issue_id, &route.label);
            if !store.request_fired(&request_id) {
                if let Some(channel_id) = route.channel.or(config.default_channel) {
                    actions.push(RuleAction::RouteAssign {
                        issue: issue_ref.clone(),
                        label: route.label.clone(),
                        owner: route.owner.clone(),
                        channel_id,
                        request_id,
                    });
                }
            }
        } else if issue.assignee.is_none() && issue.status == IssueStatus::Triage {
            // Arrival notice (plan rule 1), exclusively for UNROUTED arrivals:
            // "a human needs to triage this". No routing label matched, so no
            // owner wake — the notice is a human-facing surface.
            let request_id = triage_notify_request_id(issue.issue_id, issue.entered_status_at);
            if !store.request_fired(&request_id) {
                if let Some(channel_id) = issue.home_channel_id.or(config.default_channel) {
                    actions.push(RuleAction::NotifyTriage {
                        issue: issue_ref.clone(),
                        channel_id,
                        request_id,
                    });
                }
            }
        }

        // Escalation ladder.
        for threshold in &config.thresholds {
            let Some(entry_time) = qualifies(issue, threshold, now) else {
                continue;
            };
            let epoch = entry_time / threshold.window_secs.max(1);
            let request_id = threshold_request_id(issue.issue_id, &threshold.id, epoch);
            if store.request_fired(&request_id) {
                continue;
            }
            let channel_id = match threshold.action {
                ThresholdAction::DecisionSurface => Some(config.decision_surface.channel),
                ThresholdAction::NotifyHome | ThresholdAction::Digest => {
                    issue.home_channel_id.or(config.default_channel)
                }
            };
            let Some(channel_id) = channel_id else {
                continue;
            };
            actions.push(RuleAction::Escalate {
                issue: issue_ref.clone(),
                threshold_id: threshold.id.clone(),
                channel_id,
                action: threshold.action,
                request_id,
            });
        }
    }
    actions
}

/// First routing rule (config order) whose label the issue carries.
fn matching_route<'a>(
    config: &'a DispatcherConfig,
    issue: &IssueProjection,
) -> Option<&'a crate::config::RouteRule> {
    config
        .routing
        .iter()
        .find(|rule| issue.labels.contains(&rule.label))
}

/// Entry time when `threshold` qualifies `issue`, else `None`.
///
/// B3: unassigned conditions qualify ONLY once the full window has elapsed
/// since status entry (`now >= entered_status_at + window_secs`). The window
/// is the qualification period, not just the dedup epoch — a fresh triage
/// issue must NOT fire `unassigned_24h` at T+0 (plan: "unassigned:T+24h").
fn qualifies(issue: &IssueProjection, threshold: &Threshold, now: u64) -> Option<u64> {
    match &threshold.condition {
        ThresholdCondition::Unassigned => {
            let window_elapsed = now
                >= issue
                    .entered_status_at
                    .saturating_add(threshold.window_secs);
            (issue.assignee.is_none() && window_elapsed).then_some(issue.entered_status_at)
        }
        ThresholdCondition::UnassignedWithLabel { label } => {
            let window_elapsed = now
                >= issue
                    .entered_status_at
                    .saturating_add(threshold.window_secs);
            (issue.assignee.is_none() && issue.labels.contains(label) && window_elapsed)
                .then_some(issue.entered_status_at)
        }
        ThresholdCondition::DeadlinePassed => issue
            .due
            .as_ref()
            .filter(|due| issue.assignee.is_some() && due_has_passed(due, now))
            .map(|due| due.value),
    }
}

/// Idempotency key for triage arrival notices: distinct per genuine entry
/// into triage (re-entry carries a new `entered_status_at`).
fn triage_notify_request_id(issue_id: Uuid, entered_status_at: u64) -> Uuid {
    Uuid::new_v5(
        &DISPATCH_REQUEST_NAMESPACE,
        format!("triage:{issue_id}:{entered_status_at}").as_bytes(),
    )
}

fn first_tag_value(event: &nostr::Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        if slice.first().is_some_and(|first| first == name) {
            slice.get(1).cloned()
        } else {
            None
        }
    })
}

fn first_tag_uuid(event: &nostr::Event, name: &str) -> Option<Uuid> {
    first_tag_value(event, name).and_then(|value| Uuid::parse_str(&value).ok())
}

/// Reaction-emoji match tolerant of the U+FE0F variation selector that real
/// clients append to emoji-presentation reactions (📌 vs 📌️). Both sides are
/// normalized before comparison.
fn emoji_matches(content: &str, pin_emoji: &str) -> bool {
    normalize_reaction(content) == normalize_reaction(pin_emoji)
}

fn normalize_reaction(value: &str) -> String {
    value.chars().filter(|c| *c != '\u{FE0F}').collect()
}

/// Strict event-id shape: exactly 64 lowercase hex chars. Publisher-controlled
/// e-tags must pass this before they are sliced or persisted (B2).
fn is_event_id_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DecisionSurface, LivenessConfig, RouteRule};
    use crate::projection::tests::TEST_NOW;
    use buzz_sdk::tracker::{Due, DuePrecision};
    use nostr::{Event, EventBuilder, Keys, Tag, Timestamp};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    const OWNER_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OTHER_HEX: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn stream_id() -> Uuid {
        Uuid::new_v4()
    }

    fn channel() -> Uuid {
        Uuid::new_v4()
    }

    fn base_config(state_dir: PathBuf, stream: Uuid, home: Uuid) -> DispatcherConfig {
        DispatcherConfig {
            relay_url: Some("http://localhost:3000".to_string()),
            service_id: "triage-dispatcher".to_string(),
            stream_id: stream,
            poll_interval_secs: 30,
            monitored_channels: vec![home],
            pin_emoji: "\u{1F4CC}".to_string(),
            default_channel: Some(home),
            routing: vec![RouteRule {
                label: "area:payments".to_string(),
                owner: OWNER_HEX.to_string(),
                channel: None,
            }],
            thresholds: Vec::new(),
            roles: std::collections::BTreeMap::new(),
            decision_surface: DecisionSurface { channel: home },
            liveness: LivenessConfig {
                channel: home,
                owner: None,
                stale_after_secs: 300,
                signer_allowlist: vec!["c".repeat(64)],
            },
            state_dir,
        }
    }

    fn issue(stream: Uuid, status: IssueStatus, labels: &[&str]) -> IssueProjection {
        let mut set = BTreeSet::new();
        for label in labels {
            set.insert((*label).to_string());
        }
        IssueProjection {
            stream_id: stream,
            issue_id: Uuid::new_v4(),
            root_event: "a".repeat(64),
            root_created_at: 100,
            tip_event: "a".repeat(64),
            tip_created_at: 100,
            revision: 0,
            entered_status_at: 100,
            identifier: "WS-1".to_string(),
            title: "issue".to_string(),
            body: String::new(),
            status,
            priority: None,
            assignee: None,
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            workflow: "default".to_string(),
            labels: set,
            external_refs: BTreeSet::new(),
            relations: BTreeSet::new(),
        }
    }

    fn fold_with(stream: Uuid, issue: IssueProjection) -> TrackerFold {
        let mut fold = TrackerFold::default();
        fold.issues.insert((stream, issue.issue_id), issue);
        fold
    }

    fn reaction(channel_id: Uuid, source_event_id: &str, emoji: &str) -> Event {
        let keys = Keys::generate();
        let tags = vec![
            Tag::parse(["e", source_event_id]).expect("tag"),
            Tag::parse(["h", &channel_id.to_string()]).expect("tag"),
        ];
        EventBuilder::new(Kind::Custom(7), emoji)
            .tags(tags)
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .expect("signed")
    }

    #[test]
    fn pin_reactions_collapse_to_one_capture_per_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let config = base_config(dir.path().to_path_buf(), stream_id(), home);
        let source = "c".repeat(64);

        // Three reactions (initial pin + two re-pins) on the same message,
        // plus one reaction in an unmonitored channel and one wrong emoji.
        store.ingest(
            vec![
                reaction(home, &source, &config.pin_emoji),
                reaction(home, &source, &config.pin_emoji),
                reaction(home, &source, &config.pin_emoji),
                reaction(channel(), &source, &config.pin_emoji),
                reaction(home, &source, "\u{1F44D}"),
            ],
            TEST_NOW,
        );

        let plans = plan_captures(&config, &store);
        assert_eq!(plans.len(), 1, "duplicates and unmonitored collapse");
        assert_eq!(plans[0].source_event_id, source);
        assert_eq!(plans[0].channel_id, home);

        // Fired ledger suppresses re-planning across restarts.
        store.mark_request_fired(plans[0].request_id);
        assert!(plan_captures(&config, &store).is_empty());
    }

    #[test]
    fn root_precheck_suppresses_capture_even_without_fired_ledger() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let config = base_config(dir.path().to_path_buf(), stream_id(), home);
        let source = "d".repeat(64);

        store.ingest(vec![reaction(home, &source, &config.pin_emoji)], TEST_NOW);
        assert_eq!(plan_captures(&config, &store).len(), 1);

        // A root already exists for this source (e.g. namespace drift): no plan.
        let root = crate::projection::tests::sample_root_event(Some(&source));
        store.ingest(vec![root], TEST_NOW);
        assert!(plan_captures(&config, &store).is_empty());
    }

    #[test]
    fn routed_arrival_gets_dispatch_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let config = base_config(dir.path().to_path_buf(), stream, home);

        let fold = fold_with(
            stream,
            issue(stream, IssueStatus::Triage, &["area:payments"]),
        );
        let plans = plan_fold_actions(&config, &fold, &store, 1_000);

        // Exclusivity (Helm 2026-08-24): routed arrival emits ONLY the
        // dispatch; the envelope carries the owner wake.
        assert_eq!(plans.len(), 1);
        match &plans[0] {
            RuleAction::RouteAssign {
                label,
                owner,
                channel_id,
                request_id,
                ..
            } => {
                assert_eq!(label, "area:payments");
                assert_eq!(owner, OWNER_HEX);
                assert_eq!(*channel_id, home);
                assert_eq!(
                    *request_id,
                    route_request_id(plans_issue_id(&plans), "area:payments")
                );
            }
            other => panic!("expected RouteAssign, got {other:?}"),
        }
    }

    #[test]
    fn unroutable_triage_gets_one_arrival_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let config = base_config(dir.path().to_path_buf(), stream, home);

        // No routing label matches: UNROUTED arrival notice only.
        let fold = fold_with(stream, issue(stream, IssueStatus::Triage, &["bug"]));
        let plans = plan_fold_actions(&config, &fold, &store, 1_000);

        assert_eq!(plans.len(), 1);
        match &plans[0] {
            RuleAction::NotifyTriage { channel_id, .. } => {
                assert_eq!(*channel_id, home);
            }
            other => panic!("expected NotifyTriage, got {other:?}"),
        }
        assert!(
            plans
                .iter()
                .all(|action| !matches!(action, RuleAction::RouteAssign { .. })),
            "unrouted arrival must not dispatch"
        );
    }

    #[test]
    fn todo_status_gets_routing_but_no_arrival_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let config = base_config(dir.path().to_path_buf(), stream, home);

        let fold = fold_with(stream, issue(stream, IssueStatus::Todo, &["area:payments"]));
        let plans = plan_fold_actions(&config, &fold, &store, 1_000);
        assert_eq!(plans.len(), 1, "routing yes, arrival notice is triage-only");
        assert!(matches!(plans[0], RuleAction::RouteAssign { .. }));
    }

    #[test]
    fn non_area_label_routes_with_dispatch_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.routing.push(RouteRule {
            label: "misc".to_string(),
            owner: OTHER_HEX.to_string(),
            channel: None,
        });

        let fold = fold_with(stream, issue(stream, IssueStatus::Triage, &["misc"]));
        let plans = plan_fold_actions(&config, &fold, &store, 1_000);

        // Non-area routing labels also preempt the arrival notice.
        assert_eq!(plans.len(), 1);
        match &plans[0] {
            RuleAction::RouteAssign { owner, label, .. } => {
                assert_eq!(owner, OTHER_HEX);
                assert_eq!(label, "misc");
            }
            other => panic!("expected RouteAssign, got {other:?}"),
        }
    }

    #[test]
    fn unassigned_threshold_qualifies_only_after_window_not_at_t0() {
        // B3 contract (Helm round 1): the window is the qualification
        // period. NO fire at T+0, fire once at T+window.
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "unassigned_24h".to_string(),
            condition: ThresholdCondition::Unassigned,
            window_secs: 24 * 3600,
            action: ThresholdAction::NotifyHome,
        });

        let mut target = issue(stream, IssueStatus::Triage, &[]);
        target.entered_status_at = 1_000;
        target.home_channel_id = Some(home);
        let fold = fold_with(stream, target);

        fn escalation_count(plans: &[RuleAction]) -> usize {
            plans
                .iter()
                .filter(|action| matches!(action, RuleAction::Escalate { .. }))
                .count()
        }

        // T+0: arrival may notice, but the threshold must NOT fire.
        assert_eq!(
            escalation_count(&plan_fold_actions(&config, &fold, &store, 1_000)),
            0
        );
        // Window minus one second: still nothing.
        assert_eq!(
            escalation_count(&plan_fold_actions(&config, &fold, &store, 1_000 + 86_399)),
            0
        );
        // Exactly T+window: qualifies.
        assert_eq!(
            escalation_count(&plan_fold_actions(&config, &fold, &store, 1_000 + 86_400)),
            1
        );
    }

    #[test]
    fn labeled_threshold_qualifies_only_after_window() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "p0_unassigned_4h".to_string(),
            condition: ThresholdCondition::UnassignedWithLabel {
                label: "priority:p0".to_string(),
            },
            window_secs: 4 * 3600,
            action: ThresholdAction::DecisionSurface,
        });

        let mut target = issue(stream, IssueStatus::Triage, &["priority:p0"]);
        target.entered_status_at = 1_000;
        target.home_channel_id = Some(home);
        let fold = fold_with(stream, target);

        // T+0 must not page the decision surface (B3). The arrival notice
        // still fires — it is the threshold that must wait out the window.
        assert!(plan_fold_actions(&config, &fold, &store, 1_000)
            .iter()
            .all(|action| !matches!(action, RuleAction::Escalate { .. })));
        let plans = plan_fold_actions(&config, &fold, &store, 1_000 + 4 * 3600);
        let escalations: Vec<_> = plans
            .iter()
            .filter(|action| matches!(action, RuleAction::Escalate { .. }))
            .collect();
        assert_eq!(escalations.len(), 1);
        match escalations[0] {
            RuleAction::Escalate {
                threshold_id,
                action,
                ..
            } => {
                assert_eq!(threshold_id, "p0_unassigned_4h");
                assert_eq!(*action, ThresholdAction::DecisionSurface);
            }
            other => panic!("expected Escalate, got {other:?}"),
        }
    }

    #[test]
    fn threshold_fires_once_per_interval_and_refires_on_reentry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "unassigned_24h".to_string(),
            condition: ThresholdCondition::Unassigned,
            window_secs: 24 * 3600,
            action: ThresholdAction::NotifyHome,
        });

        let mut first = issue(stream, IssueStatus::Triage, &[]);
        first.entered_status_at = 1_000;
        first.home_channel_id = Some(home);
        let fold = fold_with(stream, first.clone());
        let qualified_now = 1_000 + 24 * 3600;

        let plans = plan_fold_actions(&config, &fold, &store, qualified_now);
        let escalations: Vec<_> = plans
            .iter()
            .filter(|action| matches!(action, RuleAction::Escalate { .. }))
            .collect();
        assert_eq!(escalations.len(), 1);

        // Mark fired: the same snapshot produces no escalation.
        if let RuleAction::Escalate { request_id, .. } = &escalations[0] {
            store.mark_request_fired(*request_id);
        }
        let replay = plan_fold_actions(&config, &fold, &store, qualified_now + 1_000);
        assert!(replay
            .iter()
            .all(|action| !matches!(action, RuleAction::Escalate { .. })));

        // Genuine re-entry: new entered_status_at in a later interval epoch.
        let mut reentered = first.clone();
        reentered.issue_id = first.issue_id;
        reentered.entered_status_at = 1_000 + 25 * 3600; // next epoch
        let fold2 = fold_with(stream, reentered);
        let plans2 = plan_fold_actions(
            &config,
            &fold2,
            &store,
            1_000 + 25 * 3600 + 24 * 3600, // past the re-entry window
        );
        assert_eq!(
            plans2
                .iter()
                .filter(|action| matches!(action, RuleAction::Escalate { .. }))
                .count(),
            1,
            "re-entry refires"
        );
    }

    #[test]
    fn deadline_threshold_uses_due_value_and_fires_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "deadline_breach".to_string(),
            condition: ThresholdCondition::DeadlinePassed,
            window_secs: 3600,
            action: ThresholdAction::DecisionSurface,
        });

        let mut assigned = issue(stream, IssueStatus::InProgress, &[]);
        assigned.assignee = Some(OWNER_HEX.to_string());
        assigned.home_channel_id = Some(home);
        assigned.due = Some(Due {
            value: 5_000,
            precision: DuePrecision::Instant,
            timezone: "UTC".to_string(),
        });
        let fold = fold_with(stream, assigned);

        // Before the due instant: no fire.
        assert!(plan_fold_actions(&config, &fold, &store, 4_999)
            .iter()
            .all(|action| !matches!(action, RuleAction::Escalate { .. })));

        let plans = plan_fold_actions(&config, &fold, &store, 5_001);
        let escalations: Vec<_> = plans
            .iter()
            .filter(|action| matches!(action, RuleAction::Escalate { .. }))
            .collect();
        assert_eq!(escalations.len(), 1);
        match &escalations[0] {
            RuleAction::Escalate {
                channel_id, action, ..
            } => {
                assert_eq!(*channel_id, home, "decision surface channel from config");
                assert_eq!(*action, ThresholdAction::DecisionSurface);
            }
            _ => unreachable!(),
        }

        if let RuleAction::Escalate { request_id, .. } = &escalations[0] {
            store.mark_request_fired(*request_id);
        }
        assert!(plan_fold_actions(&config, &fold, &store, 9_999)
            .iter()
            .all(|action| !matches!(action, RuleAction::Escalate { .. })));
    }

    #[test]
    fn digest_thresholds_are_flagged_for_coalescing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "unassigned_24h_digest".to_string(),
            condition: ThresholdCondition::Unassigned,
            window_secs: 24 * 3600,
            action: ThresholdAction::Digest,
        });

        let mut target = issue(stream, IssueStatus::Todo, &[]);
        target.home_channel_id = Some(home);
        let fold = fold_with(stream, target);
        // Past the qualification window (entered_status_at 100 + 24h).
        let plans = plan_fold_actions(&config, &fold, &store, 100 + 24 * 3600);
        match &plans[0] {
            RuleAction::Escalate { action, .. } => assert_eq!(*action, ThresholdAction::Digest),
            other => panic!("expected digest Escalate, got {other:?}"),
        }
    }

    /// B2 regression guard: publisher-controlled e-tags that are not 64-char
    /// lowercase hex are skipped, not sliced — a short or multibyte e-tag
    /// used to panic the emitter and crash-loop the process.
    #[test]
    fn invalid_e_tags_are_skipped_not_panicked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let config = base_config(dir.path().to_path_buf(), stream_id(), home);

        store.ingest(
            vec![
                reaction(home, "short", &config.pin_emoji),
                reaction(home, "", &config.pin_emoji),
                reaction(home, &"F".repeat(64), &config.pin_emoji), // uppercase
                reaction(home, &"z".repeat(64), &config.pin_emoji), // non-hex
                reaction(home, &"é".repeat(32), &config.pin_emoji), // multibyte
            ],
            TEST_NOW,
        );
        assert!(
            plan_captures(&config, &store).is_empty(),
            "no capture plan from malformed e-tags"
        );
    }

    /// Real clients emit the pin emoji with the U+FE0F variation selector;
    /// matching normalizes both sides.
    #[test]
    fn pin_emoji_with_variation_selector_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let config = base_config(dir.path().to_path_buf(), stream_id(), home);
        let source = "c".repeat(64);

        store.ingest(
            vec![reaction(
                home,
                &source,
                &format!("{}\u{FE0F}", config.pin_emoji),
            )],
            TEST_NOW,
        );
        let plans = plan_captures(&config, &store);
        assert_eq!(plans.len(), 1, "FE0F variant of the pin emoji captures");
        assert_eq!(plans[0].source_event_id, source);
    }

    #[test]
    fn imported_issues_never_trigger_rules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let mut config = base_config(dir.path().to_path_buf(), stream, home);
        config.thresholds.push(Threshold {
            id: "unassigned_24h".to_string(),
            condition: ThresholdCondition::Unassigned,
            window_secs: 24 * 3600,
            action: ThresholdAction::NotifyHome,
        });

        let imported = issue(stream, IssueStatus::Triage, &["area:payments"]);
        let root = crate::projection::tests::imported_root_event(imported.issue_id);
        store.ingest(vec![root], TEST_NOW);

        let fold = fold_with(stream, imported);
        assert!(
            plan_fold_actions(&config, &fold, &store, 1_000).is_empty(),
            "M19 import suppression"
        );
        assert!(plan_captures(&config, &store).is_empty());
    }

    #[test]
    fn assigned_and_terminal_issues_get_no_actions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ProjectionStore::open(dir.path()).expect("open");
        let home = channel();
        let stream = stream_id();
        let config = base_config(dir.path().to_path_buf(), stream, home);

        let mut assigned = issue(stream, IssueStatus::Triage, &["area:payments"]);
        assigned.assignee = Some(OWNER_HEX.to_string());
        let mut done = issue(stream, IssueStatus::Done, &["area:payments"]);
        done.assignee = None;

        for candidate in [assigned, done] {
            let fold = fold_with(stream, candidate);
            assert!(plan_fold_actions(&config, &fold, &store, 1_000).is_empty());
        }
    }

    fn plans_issue_id(plans: &[RuleAction]) -> Uuid {
        match &plans[0] {
            RuleAction::RouteAssign { issue, .. }
            | RuleAction::NotifyTriage { issue, .. }
            | RuleAction::Escalate { issue, .. } => issue.issue_id,
        }
    }
}
