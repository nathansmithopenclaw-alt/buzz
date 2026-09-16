use buzz_sdk_pkg::tracker::{FoldDiagnostic, TrackerEvent, TrackerPayload};
use serde_json::{json, Value};

pub(super) fn diagnostic_json(diagnostic: &FoldDiagnostic, claims: &[TrackerEvent]) -> Value {
    let (issue_id, event_id, canonical) = match diagnostic {
        FoldDiagnostic::InvalidEvent { event_id, .. } => (
            claims
                .iter()
                .find(|claim| claim.event_id == *event_id)
                .map(|claim| claim.issue_id),
            Some(event_id.as_str()),
            None,
        ),
        FoldDiagnostic::LosingBranch {
            issue_id,
            event_id,
            canonical_tip,
        } => (
            Some(*issue_id),
            Some(event_id.as_str()),
            Some(canonical_tip.as_str()),
        ),
        FoldDiagnostic::RootCollision {
            issue_id,
            canonical,
            rejected,
        } => (
            Some(*issue_id),
            Some(rejected.as_str()),
            Some(canonical.as_str()),
        ),
        FoldDiagnostic::IdentifierCollision { conflicting, .. } => (Some(*conflicting), None, None),
        FoldDiagnostic::InvalidRelation { issue_id, .. }
        | FoldDiagnostic::BlocksCycle { issue_id, .. } => (Some(*issue_id), None, None),
    };
    let claim = event_id.and_then(|id| claims.iter().find(|event| event.event_id == id));
    let proposed = if matches!(diagnostic, FoldDiagnostic::LosingBranch { .. }) {
        claim.and_then(|event|match &event.payload {TrackerPayload::Revision(revision)=>Some(json!({"set":revision.set,"clear":revision.clear,"relations":revision.relations,"external_refs":revision.external_refs})),_=>None})
    } else {
        None
    };
    json!({"kind":format!("{diagnostic:?}").split_whitespace().next().unwrap_or("FoldDiagnostic"),"message":format!("{diagnostic:?}"),"issueId":issue_id,"eventId":event_id,"canonicalEvent":canonical,"authorPubkey":claim.map(|value|&value.author_pubkey),"proposed":proposed})
}
