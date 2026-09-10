use nostr::{Keys, Timestamp};
use uuid::Uuid;

use crate::task_authority::fold_authorized_task_events;
use crate::task_envelope::{
    build_task_message, TaskAuthority, TaskEnvelope, TaskEnvelopeEvent, TaskStatus,
    TaskVerification, TASK_ENVELOPE_VERSION,
};

fn signed(keys: &Keys, task: &TaskEnvelope, time: u64) -> TaskEnvelopeEvent {
    let event = build_task_message(Uuid::nil(), "Task update", task, None, &[])
        .unwrap()
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap();
    TaskEnvelopeEvent::from_event(&event).unwrap()
}

fn assignment(owner: &Keys, executor: &Keys, reviewer: Option<&Keys>) -> TaskEnvelopeEvent {
    signed(
        owner,
        &TaskEnvelope {
            v: TASK_ENVELOPE_VERSION,
            task_id: Uuid::new_v4(),
            status: TaskStatus::Assigned,
            assignee: executor.public_key().to_hex(),
            assigner: owner.public_key().to_hex(),
            title: "Ship the accepted scope".into(),
            deliverables: vec![],
            verification: TaskVerification {
                method: "Review the exact artifact".into(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: Some(Uuid::new_v4()),
            dispatch_request_id: Some(Uuid::new_v4()),
            authority: Some(TaskAuthority {
                assignment_event_id: None,
                previous_event_id: None,
                reviewer: reviewer.map(|key| key.public_key().to_hex()),
            }),
        },
        100,
    )
}

fn transition(
    root: &TaskEnvelopeEvent,
    previous: &TaskEnvelopeEvent,
    signer: &Keys,
    status: TaskStatus,
) -> TaskEnvelopeEvent {
    let mut task = previous.envelope.clone();
    task.status = status;
    task.authority.as_mut().unwrap().assignment_event_id = Some(root.event_id.clone());
    task.authority.as_mut().unwrap().previous_event_id = Some(previous.event_id.clone());
    if status == TaskStatus::Verified {
        task.verification.evidence_ref = Some("https://example.test/receipt".into());
    }
    if status == TaskStatus::Blocked {
        task.blocked_on = Some("Waiting for a decision".into());
    }
    signed(signer, &task, 100)
}

#[test]
fn unrelated_and_self_review_cannot_promote_completed_work() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let outsider = Keys::generate();
    let root = assignment(&owner, &executor, None);
    let done = transition(&root, &root, &executor, TaskStatus::Done);
    for attacker in [&outsider, &executor] {
        let forged = transition(&root, &done, attacker, TaskStatus::Verified);
        let folded = fold_authorized_task_events([forged, done.clone(), root.clone()]);
        assert_eq!(
            folded.current[&root.envelope.task_id].envelope.status,
            TaskStatus::Done
        );
        assert_eq!(folded.diagnostics.len(), 1);
    }
    let verified = transition(&root, &done, &owner, TaskStatus::Verified);
    let folded = fold_authorized_task_events([verified.clone(), done, root.clone()]);
    assert_eq!(
        folded.current[&root.envelope.task_id].event_id,
        verified.event_id
    );
    assert!(folded.diagnostics.is_empty());
}

#[test]
fn delegated_review_requires_completion_and_evidence() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let reviewer = Keys::generate();
    let root = assignment(&owner, &executor, Some(&reviewer));
    let premature = transition(&root, &root, &reviewer, TaskStatus::Verified);
    assert_eq!(
        fold_authorized_task_events([premature, root.clone()]).current[&root.envelope.task_id]
            .envelope
            .status,
        TaskStatus::Assigned
    );
    let done = transition(&root, &root, &executor, TaskStatus::Done);
    let verified = transition(&root, &done, &reviewer, TaskStatus::Verified);
    let mut missing = verified.envelope.clone();
    missing.verification.evidence_ref = None;
    let missing = signed(&reviewer, &missing, 101);
    let result = fold_authorized_task_events([missing, done.clone(), root.clone()]);
    assert_eq!(
        result.current[&root.envelope.task_id].envelope.status,
        TaskStatus::Done
    );
    assert!(result.diagnostics[0].reason.contains("evidence"));
    let result = fold_authorized_task_events([verified.clone(), root.clone(), done]);
    assert_eq!(
        result.current[&root.envelope.task_id].event_id,
        verified.event_id
    );
}

#[test]
fn scope_rebinding_and_stale_predecessor_are_rejected() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let root = assignment(&owner, &executor, None);
    let picked = transition(&root, &root, &executor, TaskStatus::PickedUp);
    let mut changed = transition(&root, &picked, &executor, TaskStatus::Done).envelope;
    changed.title = "Different unapproved task".into();
    let changed = signed(&executor, &changed, 101);
    let result = fold_authorized_task_events([changed, root.clone(), picked.clone()]);
    assert_eq!(
        result.current[&root.envelope.task_id].event_id,
        picked.event_id
    );
    assert!(result.diagnostics[0].reason.contains("scope"));
    let mut stale = transition(&root, &picked, &executor, TaskStatus::Done).envelope;
    stale.authority.as_mut().unwrap().previous_event_id = Some("aa".repeat(32));
    let result =
        fold_authorized_task_events([signed(&executor, &stale, 101), picked.clone(), root.clone()]);
    assert_eq!(
        result.current[&root.envelope.task_id].event_id,
        picked.event_id
    );
}

#[test]
fn conflicting_assignment_cannot_steal_task_by_backdating() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let attacker = Keys::generate();
    let root = assignment(&owner, &executor, None);
    let mut collision = root.envelope.clone();
    collision.assigner = attacker.public_key().to_hex();
    let collision = signed(&attacker, &collision, 99);
    let result = fold_authorized_task_events([root, collision]);
    assert!(result.current.is_empty());
    assert_eq!(result.diagnostics.len(), 2);
}

#[test]
fn altered_signature_is_rejected_by_the_shared_parser() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let root = assignment(&owner, &executor, None);
    let event = build_task_message(Uuid::nil(), "Task", &root.envelope, None, &[])
        .unwrap()
        .sign_with_keys(&owner)
        .unwrap();
    let mut raw = serde_json::to_value(event).unwrap();
    raw["content"] = serde_json::json!("tampered");
    let altered = serde_json::from_value(raw).unwrap();
    assert!(TaskEnvelopeEvent::from_event(&altered).is_err());
}

#[test]
fn human_review_binds_result_and_requires_independent_evidence_for_accept_or_reject() {
    let owner = Keys::generate();
    let executor = Keys::generate();
    let reviewer = Keys::generate();
    let outsider = Keys::generate();
    let root = assignment(&owner, &executor, Some(&reviewer));
    let done = transition(&root, &root, &executor, TaskStatus::Done);
    for accepted in [true, false] {
        let input = crate::task_authority::TaskReviewInput {
            task_id: root.envelope.task_id,
            expected_event: done.event_id.clone(),
            accepted,
            evidence: "https://example.test/review/42".into(),
            notes: "Checked the exact deliverable against the acceptance criteria".into(),
        };
        for unauthorized in [&executor, &outsider] {
            assert!(crate::task_authority::build_task_review(
                vec![root.clone(), done.clone()],
                &unauthorized.public_key().to_hex(),
                &input
            )
            .is_err());
        }
        let event = crate::task_authority::build_task_review(
            vec![done.clone(), root.clone()],
            &reviewer.public_key().to_hex(),
            &input,
        )
        .unwrap()
        .sign_with_keys(&reviewer)
        .unwrap();
        let reviewed = TaskEnvelopeEvent::from_event(&event).unwrap();
        let folded = fold_authorized_task_events([root.clone(), done.clone(), reviewed.clone()]);
        assert_eq!(
            folded.current[&root.envelope.task_id].event_id,
            reviewed.event_id
        );
        assert!(folded.diagnostics.is_empty());
        let mut stale = input.clone();
        stale.expected_event = root.event_id.clone();
        assert!(crate::task_authority::build_task_review(
            vec![root.clone(), done.clone()],
            &reviewer.public_key().to_hex(),
            &stale
        )
        .is_err());
        let mut absent = input;
        absent.evidence.clear();
        assert!(crate::task_authority::build_task_review(
            vec![root.clone(), done.clone()],
            &reviewer.public_key().to_hex(),
            &absent
        )
        .is_err());
    }
}
