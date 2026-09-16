use std::collections::{BTreeMap, BTreeSet};

use buzz_sdk_pkg::task_envelope::{
    build_task_message, TaskEnvelope, TaskStatus, TaskVerification, TASK_ENVELOPE_VERSION,
};
use buzz_sdk_pkg::tracker::{ExternalRef, IssueProjection, IssueStatus};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;
use uuid::Uuid;

use super::tracker_tasks::fold_linked_tasks;

fn membership(relay: &Keys, channel: Uuid, members: &[&Keys], timestamp: u64) -> Event {
    let mut tags = vec![Tag::parse(["d", &channel.to_string()]).unwrap()];
    for key in members {
        tags.push(Tag::parse(["p", &key.public_key().to_hex()]).unwrap());
    }
    EventBuilder::new(Kind::Custom(39002), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(timestamp))
        .sign_with_keys(relay)
        .unwrap()
}

#[test]
fn linked_foreign_channel_requires_explicit_reference_and_current_read_membership() {
    let relay = Keys::generate();
    let owner = Keys::generate();
    let executor = Keys::generate();
    let workspace = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let issue_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let workspace_members = membership(&relay, workspace, &[&owner, &executor], 100);
    let channel_members = membership(&relay, channel, &[&owner, &executor], 100);
    let assignment = build_task_message(
        channel,
        "Implement the issue",
        &TaskEnvelope {
            v: TASK_ENVELOPE_VERSION,
            authority: Some(Default::default()),
            task_id,
            status: TaskStatus::Assigned,
            assignee: executor.public_key().to_hex(),
            assigner: owner.public_key().to_hex(),
            title: "Execution in its home conversation".into(),
            deliverables: vec![],
            verification: TaskVerification {
                method: "Independent review".into(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: Some(issue_id),
            dispatch_request_id: Some(Uuid::new_v4()),
        },
        None,
        &[],
    )
    .unwrap()
    .custom_created_at(Timestamp::from(101))
    .sign_with_keys(&owner)
    .unwrap();
    let issue = IssueProjection {
        stream_id: workspace,
        issue_id,
        root_event: "aa".repeat(32),
        root_created_at: 100,
        tip_event: "aa".repeat(32),
        tip_created_at: 100,
        revision: 0,
        entered_status_at: 100,
        identifier: "DSP-1".into(),
        title: "Example".into(),
        body: String::new(),
        status: IssueStatus::Todo,
        priority: None,
        assignee: Some(owner.public_key().to_hex()),
        project_id: None,
        due: None,
        estimate_minutes: None,
        home_channel_id: Some(channel),
        workflow: "default-v1".into(),
        labels: BTreeSet::new(),
        external_refs: BTreeSet::from([ExternalRef {
            kind: "task".into(),
            reference: task_id.to_string(),
        }]),
        relations: BTreeSet::new(),
    };
    let raw = vec![serde_json::to_value(&assignment).unwrap()];
    let issues = BTreeMap::from([((workspace, issue_id), issue.clone())]);
    let (tasks, diagnostics) = fold_linked_tasks(
        raw.clone(),
        std::slice::from_ref(&channel_members),
        &workspace_members,
        &owner.public_key().to_hex(),
        &issues,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["channelId"], json!(channel));
    assert_eq!(tasks[0]["assigner"], owner.public_key().to_hex());
    // Removing the original author does not erase their exact admitted task,
    // but a receipt must never restore a removed viewer's read access.
    let receipt = buzz_sdk_pkg::dispatch_admission::build_dispatch_admission(
        &buzz_sdk_pkg::dispatch_admission::DispatchAdmission {
            v: 1,
            workspace: channel,
            event_id: assignment.id.to_hex(),
            author: owner.public_key().to_hex(),
            role: "member".into(),
            observed_at: 101,
            basis: buzz_sdk_pkg::dispatch_admission::AdmissionBasis::Admission,
        },
    )
    .unwrap()
    .sign_with_keys(&relay)
    .unwrap();
    let without_author = membership(&relay, channel, &[&executor], 104);
    let admitted_raw = vec![json!(assignment), json!(receipt)];
    let (preserved, diagnostics) = fold_linked_tasks(
        admitted_raw.clone(),
        std::slice::from_ref(&without_author),
        &workspace_members,
        &executor.public_key().to_hex(),
        &issues,
    );
    assert_eq!(preserved.len(), 1);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let (denied, _) = fold_linked_tasks(
        admitted_raw,
        std::slice::from_ref(&without_author),
        &workspace_members,
        &owner.public_key().to_hex(),
        &issues,
    );
    assert!(denied.is_empty());
    let (unsealed, _) = fold_linked_tasks(
        raw.clone(),
        std::slice::from_ref(&without_author),
        &workspace_members,
        &executor.public_key().to_hex(),
        &issues,
    );
    assert!(unsealed.is_empty());
    let mut unlinked = issue;
    unlinked.external_refs.clear();
    let (tasks, diagnostics) = fold_linked_tasks(
        raw.clone(),
        std::slice::from_ref(&channel_members),
        &workspace_members,
        &owner.public_key().to_hex(),
        &BTreeMap::from([((workspace, issue_id), unlinked)]),
    );
    assert!(tasks.is_empty());
    assert!(diagnostics[0].contains("explicit link"));
    let removed = membership(&relay, channel, &[&executor], 102);
    let (tasks, diagnostics) = fold_linked_tasks(
        raw.clone(),
        &[channel_members, removed],
        &workspace_members,
        &owner.public_key().to_hex(),
        &issues,
    );
    assert!(tasks.is_empty());
    assert!(diagnostics[0].contains("does not authorize"));
    let forged = membership(&Keys::generate(), channel, &[&owner, &executor], 103);
    let (tasks, diagnostics) = fold_linked_tasks(
        raw,
        &[forged],
        &workspace_members,
        &owner.public_key().to_hex(),
        &issues,
    );
    assert!(tasks.is_empty());
    assert!(diagnostics[0].contains("does not authorize"));
}
