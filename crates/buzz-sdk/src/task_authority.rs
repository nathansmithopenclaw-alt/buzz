//! Assignment-anchored lifecycle authority shared by every task consumer.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::task_envelope::{TaskEnvelopeEvent, TaskStatus};

/// A rejected claim is retained as a diagnostic, never presented as current work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAuthorityDiagnostic {
    /// Task whose history contains the rejected claim.
    pub task_id: Uuid,
    /// Signed event making the claim.
    pub event_id: String,
    /// Human-readable rejection reason.
    pub reason: String,
}

/// Authorized state and the rejected claims needed to explain incomplete history.
#[derive(Debug, Default)]
pub struct TaskAuthorityFold {
    /// Original authorized assignment for each unambiguous attempt.
    pub assignments: BTreeMap<Uuid, TaskEnvelopeEvent>,
    /// Latest authorized transition for each unambiguous assignment.
    pub current: BTreeMap<Uuid, TaskEnvelopeEvent>,
    /// Accepted lifecycle history in causal order, including the assignment.
    pub history: BTreeMap<Uuid, Vec<TaskEnvelopeEvent>>,
    /// Claims that did not establish lifecycle authority.
    pub diagnostics: Vec<TaskAuthorityDiagnostic>,
}

fn reviewer(event: &TaskEnvelopeEvent) -> Option<&str> {
    event.envelope.authority.as_ref()?.reviewer.as_deref()
}

/// Validate a transition against the original signed assignment and accepted current state.
pub fn validate_task_transition(
    root: &TaskEnvelopeEvent,
    current: &TaskEnvelopeEvent,
    next: &TaskEnvelopeEvent,
) -> Result<(), &'static str> {
    let original = &root.envelope;
    let claim = &next.envelope;
    if claim.task_id != original.task_id
        || next.channel_id != root.channel_id
        || claim.assigner != original.assigner
        || claim.assignee != original.assignee
        || claim.title != original.title
        || claim.verification.method != original.verification.method
        || claim.tracker_issue_id != original.tracker_issue_id
        || claim.dispatch_request_id != original.dispatch_request_id
        || reviewer(next) != reviewer(root)
    {
        return Err("the update changes the signed assignment scope or authority");
    }
    if claim.v < original.v {
        return Err("the update downgrades the assignment protocol");
    }
    if claim.v >= 3
        && claim
            .authority
            .as_ref()
            .and_then(|authority| authority.assignment_event_id.as_deref())
            != Some(root.event_id.as_str())
    {
        return Err("the update does not reference the signed assignment");
    }
    if claim.v >= 3
        && claim
            .authority
            .as_ref()
            .and_then(|authority| authority.previous_event_id.as_deref())
            != Some(current.event_id.as_str())
    {
        return Err("the update does not follow the accepted task state");
    }
    if claim.status == TaskStatus::Assigned {
        return Err("an existing attempt cannot be reassigned; create a new attempt");
    }
    let review_rejection =
        current.envelope.status == TaskStatus::Done && claim.status == TaskStatus::Failed;
    if claim.status == TaskStatus::Verified || review_rejection {
        if next.author_pubkey == original.assignee
            || (next.author_pubkey != original.assigner
                && Some(next.author_pubkey.as_str()) != reviewer(root))
        {
            return Err("only the assigner or delegated independent reviewer can verify");
        }
        if current.envelope.status != TaskStatus::Done {
            return Err("independent verification requires a completed attempt");
        }
        if claim
            .verification
            .evidence_ref
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("independent verification requires an evidence reference");
        }
        if review_rejection
            && claim
                .notes
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("a rejected review requires an explanation of the unmet criteria");
        }
    } else {
        if next.author_pubkey != original.assignee && next.author_pubkey != original.assigner {
            return Err("only the executor or assigner can update execution");
        }
        if matches!(
            current.envelope.status,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Verified
        ) {
            return Err("the attempt has ended; start a new attempt to resume work");
        }
        if claim.status == TaskStatus::Blocked
            && claim
                .blocked_on
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("blocked work requires a blocking reason");
        }
    }
    Ok(())
}

/// Fold complete history, preserving rejected claims as diagnostics.
///
/// Assignment collisions fail visibly rather than choosing an attacker-controlled timestamp.
/// Callers must verify signatures and enforce channel read authorization before using this fold.
pub fn fold_authorized_task_events(
    events: impl IntoIterator<Item = TaskEnvelopeEvent>,
) -> TaskAuthorityFold {
    let mut grouped = BTreeMap::<Uuid, BTreeMap<String, TaskEnvelopeEvent>>::new();
    for event in events {
        grouped
            .entry(event.envelope.task_id)
            .or_default()
            .insert(event.event_id.clone(), event);
    }
    let mut folded = TaskAuthorityFold::default();
    for (task_id, events) in grouped {
        if events.len() > 4096 {
            folded.diagnostics.push(TaskAuthorityDiagnostic {
                task_id,
                event_id: String::new(),
                reason: "attempt history exceeds the 4096-event safety limit".into(),
            });
            continue;
        }
        let mut events = events.into_values().collect::<Vec<_>>();
        events.sort_by(|a, b| (a.created_at, &a.event_id).cmp(&(b.created_at, &b.event_id)));
        let roots = events
            .iter()
            .filter(|event| {
                event.envelope.status == TaskStatus::Assigned
                    && event.author_pubkey == event.envelope.assigner
                    && event
                        .envelope
                        .authority
                        .as_ref()
                        .is_none_or(|a| a.assignment_event_id.is_none())
            })
            .collect::<Vec<_>>();
        let root_authors = roots
            .iter()
            .map(|root| &root.author_pubkey)
            .collect::<BTreeSet<_>>();
        let Some(root) = roots.first().copied().filter(|_| root_authors.len() == 1) else {
            for event in &events {
                folded.diagnostics.push(TaskAuthorityDiagnostic {
                    task_id, event_id: event.event_id.clone(),
                    reason: "missing or conflicting signed assignment; current state cannot be established".into(),
                });
            }
            continue;
        };
        let mut current = root.clone();
        let mut history = vec![root.clone()];
        folded.assignments.insert(task_id, root.clone());
        let mut pending = events
            .iter()
            .filter(|event| event.event_id != root.event_id)
            .collect::<Vec<_>>();
        // Each accepted event removes one entry. Explicit predecessor edges, not
        // event-id lexical order, establish causality for same-second updates.
        loop {
            let next = pending.iter().position(|event| {
                event.created_at >= current.created_at
                    && validate_task_transition(root, &current, event).is_ok()
            });
            let Some(index) = next else {
                break;
            };
            current = pending.remove(index).clone();
            history.push(current.clone());
        }
        for event in pending {
            let reason = if event.created_at < root.created_at {
                "the update precedes its assignment"
            } else {
                validate_task_transition(root, &current, event)
                    .err()
                    .unwrap_or("the update is on a competing or stale branch")
            };
            folded.diagnostics.push(TaskAuthorityDiagnostic {
                task_id,
                event_id: event.event_id.clone(),
                reason: reason.into(),
            });
        }
        folded.history.insert(task_id, history);
        folded.current.insert(task_id, current);
    }
    folded
}

/// Human review intent, bound to the exact reported result the reviewer opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReviewInput {
    /// Attempt reviewed.
    pub task_id: Uuid,
    /// Exact accepted result observed by the reviewer.
    pub expected_event: String,
    /// True accepts the result; false ends it with an explained failed review.
    pub accepted: bool,
    /// Review artifact or receipt; worker prose alone is insufficient.
    pub evidence: String,
    /// Review observations, including unmet criteria if rejected.
    pub notes: String,
}

/// Prepare a review using the same lifecycle validator as every task consumer.
/// The caller checks current membership and supplies admitted, complete history.
pub fn build_task_review(
    history: Vec<TaskEnvelopeEvent>,
    author: &str,
    input: &TaskReviewInput,
) -> Result<nostr::EventBuilder, String> {
    use crate::task_envelope::{build_task_message, TASK_ENVELOPE_VERSION};
    if input.evidence.trim().is_empty()
        || input.evidence.len() > 2048
        || input.notes.trim().is_empty()
        || input.notes.len() > 8192
    {
        return Err("Review requires an evidence reference and bounded review observations".into());
    }
    let folded = fold_authorized_task_events(history);
    let current = folded
        .current
        .get(&input.task_id)
        .ok_or("Accepted attempt history is unavailable")?;
    let root = folded
        .assignments
        .get(&input.task_id)
        .ok_or("Signed assignment is unavailable")?;
    if current.envelope.status != TaskStatus::Done {
        return Err("Independent review requires a reported complete result".into());
    }
    if current.event_id != input.expected_event {
        return Err("The task changed after this review was opened".into());
    }
    let mut envelope = current.envelope.clone();
    envelope.v = TASK_ENVELOPE_VERSION;
    let authority = envelope.authority.get_or_insert_with(Default::default);
    authority.assignment_event_id = Some(root.event_id.clone());
    authority.previous_event_id = Some(current.event_id.clone());
    envelope.status = if input.accepted {
        TaskStatus::Verified
    } else {
        TaskStatus::Failed
    };
    envelope.verification.evidence_ref = Some(input.evidence.trim().into());
    envelope.notes = Some(input.notes.trim().into());
    envelope.blocked_on = None;
    let candidate = TaskEnvelopeEvent {
        event_id: String::new(),
        author_pubkey: author.into(),
        channel_id: current.channel_id,
        created_at: nostr::Timestamp::now().as_secs(),
        envelope: envelope.validate().map_err(|e| e.to_string())?,
    };
    validate_task_transition(root, current, &candidate).map_err(str::to_owned)?;
    let title = if input.accepted {
        "Independent review accepted"
    } else {
        "Independent review rejected"
    };
    let thread = crate::ThreadRef {
        root_event_id: nostr::EventId::from_hex(&root.event_id).map_err(|e| e.to_string())?,
        parent_event_id: nostr::EventId::from_hex(&current.event_id).map_err(|e| e.to_string())?,
    };
    build_task_message(
        current.channel_id,
        &format!(
            "{title}: {}\n\n{}\n\nEvidence: {}",
            candidate.envelope.title,
            input.notes.trim(),
            input.evidence.trim()
        ),
        &candidate.envelope,
        Some(&thread),
        &[],
    )
    .map_err(|e| e.to_string())
}
