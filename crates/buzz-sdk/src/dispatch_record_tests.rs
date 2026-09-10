use crate::dispatch_record::*;
use crate::tracker::{TrackerAuthorization, TrackerRole};
use nostr::{Event, Keys, Timestamp};
use uuid::Uuid;

fn record(decider: &Keys) -> DispatchRecord {
    DispatchRecord {
        v: 1,
        record_id: Uuid::new_v4(),
        revision: 0,
        parent_event: None,
        request_id: Uuid::new_v4(),
        archived: false,
        payload: DispatchRecordPayload::Decision {
            issue_id: Uuid::new_v4(),
            decider: decider.public_key().to_hex(),
            question: "Ship this change?".into(),
            context: "Review the evidence first.".into(),
            options: vec![
                DecisionOption {
                    id: "ship".into(),
                    label: "Ship".into(),
                    consequence: "Install the reviewed build".into(),
                },
                DecisionOption {
                    id: "hold".into(),
                    label: "Hold".into(),
                    consequence: "Keep the current release".into(),
                },
            ],
            evidence: vec![],
            selected_option: None,
            rationale: None,
        },
    }
}
fn signed(workspace: Uuid, record: &DispatchRecord, keys: &Keys, at: u64) -> Event {
    build_dispatch_record(workspace, record)
        .expect("builder")
        .custom_created_at(Timestamp::from_secs(at))
        .sign_with_keys(keys)
        .expect("sign")
}
fn next(record: &DispatchRecord, parent: &Event) -> DispatchRecord {
    DispatchRecord {
        revision: record.revision + 1,
        parent_event: Some(parent.id.to_hex()),
        request_id: Uuid::new_v4(),
        ..record.clone()
    }
}
#[test]
fn only_designated_decider_can_resolve_a_shared_decision() {
    let owner = Keys::generate();
    let decider = Keys::generate();
    let stranger = Keys::generate();
    let workspace = Uuid::new_v4();
    let auth = TrackerAuthorization::new(
        [
            (&owner, TrackerRole::Maintainer),
            (&decider, TrackerRole::Member),
            (&stranger, TrackerRole::Maintainer),
        ]
        .map(|(key, role)| (key.public_key().to_hex(), role)),
    );
    let root = record(&decider);
    let event = signed(workspace, &root, &owner, 100);
    let mut decision = next(&root, &event);
    if let DispatchRecordPayload::Decision {
        selected_option,
        rationale,
        ..
    } = &mut decision.payload
    {
        *selected_option = Some("ship".into());
        *rationale = Some("Reviewed the behavior".into());
    }
    let invalid = signed(workspace, &decision, &stranger, 101);
    let rejected = fold_dispatch_records([event.clone(), invalid], workspace, &auth, 200);
    assert_eq!(rejected.records[&root.record_id].record.revision, 0);
    assert_eq!(rejected.diagnostics.len(), 1);
    let accepted = fold_dispatch_records(
        [event, signed(workspace, &decision, &decider, 101)],
        workspace,
        &auth,
        200,
    );
    assert_eq!(accepted.records[&root.record_id].record, decision);
    assert!(accepted.diagnostics.is_empty());
}
#[test]
fn records_require_current_scope_signatures_and_exact_predecessors() {
    let owner = Keys::generate();
    let workspace = Uuid::new_v4();
    let auth = TrackerAuthorization::new([(owner.public_key().to_hex(), TrackerRole::Maintainer)]);
    let root = record(&owner);
    let event = signed(workspace, &root, &owner, 100);
    let update = next(&root, &event);
    let changed = signed(workspace, &update, &owner, 100);
    let mut stale = next(&update, &changed);
    stale.parent_event = Some(event.id.to_hex());
    let folded = fold_dispatch_records(
        [
            signed(workspace, &stale, &owner, 101),
            changed,
            event.clone(),
        ],
        workspace,
        &auth,
        200,
    );
    assert_eq!(folded.records[&root.record_id].record.revision, 1);
    assert_eq!(folded.diagnostics.len(), 1);
    assert!(
        fold_dispatch_records([event.clone()], Uuid::new_v4(), &auth, 200)
            .records
            .is_empty()
    );
    assert!(fold_dispatch_records(
        [event.clone()],
        workspace,
        &TrackerAuthorization::default(),
        200
    )
    .records
    .is_empty());
    let mut corrupt = event;
    corrupt.content.push(' ');
    assert!(fold_dispatch_records([corrupt], workspace, &auth, 200)
        .records
        .is_empty());
}
#[test]
fn import_activation_and_capacity_remain_maintainer_controlled() {
    let owner = Keys::generate();
    let member = Keys::generate();
    let workspace = Uuid::new_v4();
    let auth = TrackerAuthorization::new([
        (owner.public_key().to_hex(), TrackerRole::Maintainer),
        (member.public_key().to_hex(), TrackerRole::Member),
    ]);
    let mut activation = record(&owner);
    activation.payload = DispatchRecordPayload::Activation {
        issue_id: Uuid::new_v4(),
        active: true,
        reason: "Accepted into current work".into(),
    };
    assert!(fold_dispatch_records(
        [signed(workspace, &activation, &member, 100)],
        workspace,
        &auth,
        200
    )
    .records
    .is_empty());
    let root = signed(workspace, &activation, &owner, 100);
    let mut update = next(&activation, &root);
    update.archived = true;
    let result = fold_dispatch_records(
        [root, signed(workspace, &update, &member, 101)],
        workspace,
        &auth,
        200,
    );
    assert!(!result.records[&activation.record_id].record.archived);
    let mut oversized = activation;
    oversized.payload = DispatchRecordPayload::Capacity {
        executor: member.public_key().to_hex(),
        max_active: 100,
        capabilities: vec![],
        paused: false,
    };
    assert!(build_dispatch_record(workspace, &oversized).is_err());
}
#[test]
fn conflicting_root_authors_cannot_take_over_an_existing_record() {
    let a = Keys::generate();
    let b = Keys::generate();
    let workspace = Uuid::new_v4();
    let root = record(&a);
    let auth = TrackerAuthorization::new(
        [a.public_key().to_hex(), b.public_key().to_hex()].map(|key| (key, TrackerRole::Member)),
    );
    let folded = fold_dispatch_records(
        [
            signed(workspace, &root, &a, 100),
            signed(workspace, &root, &b, 99),
        ],
        workspace,
        &auth,
        200,
    );
    assert!(folded.records.is_empty());
    assert!(!folded.diagnostics.is_empty());
}

#[test]
fn decision_creator_cannot_rewrite_a_completed_decision_or_its_consequences() {
    let owner = Keys::generate();
    let decider = Keys::generate();
    let workspace = Uuid::new_v4();
    let auth = TrackerAuthorization::new([
        (owner.public_key().to_hex(), TrackerRole::Maintainer),
        (decider.public_key().to_hex(), TrackerRole::Member),
    ]);
    let root = record(&decider);
    let root_event = signed(workspace, &root, &owner, 100);
    let mut decision = next(&root, &root_event);
    if let DispatchRecordPayload::Decision {
        selected_option,
        rationale,
        ..
    } = &mut decision.payload
    {
        *selected_option = Some("hold".into());
        *rationale = Some("Needs another test".into());
    }
    let decision_event = signed(workspace, &decision, &decider, 101);
    let mut rewrite = next(&decision, &decision_event);
    if let DispatchRecordPayload::Decision { options, .. } = &mut rewrite.payload {
        options[1].consequence = "Install anyway".into();
    }
    let result = fold_dispatch_records(
        [
            root_event.clone(),
            decision_event.clone(),
            signed(workspace, &rewrite, &owner, 102),
        ],
        workspace,
        &auth,
        200,
    );
    assert_eq!(result.records[&root.record_id].record, decision);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.records[&root.record_id].accepted_events,
        vec![root_event.id.to_hex(), decision_event.id.to_hex()]
    );
    let revised = fold_dispatch_records(
        [
            root_event,
            decision_event,
            signed(workspace, &rewrite, &decider, 102),
        ],
        workspace,
        &auth,
        200,
    );
    assert_eq!(revised.records[&root.record_id].record, rewrite);
}

#[test]
fn planning_records_allow_one_bound_auth_tag_but_reject_extra_semantic_tags() {
    let owner = Keys::generate();
    let workspace = Uuid::new_v4();
    let auth = TrackerAuthorization::new([(owner.public_key().to_hex(), TrackerRole::Member)]);
    let root = record(&owner);
    let event = build_dispatch_record(workspace, &root)
        .unwrap()
        .tag(nostr::Tag::parse(["auth", "opaque-token"]).unwrap())
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&owner)
        .unwrap();
    assert_eq!(
        fold_dispatch_records([event], workspace, &auth, 200)
            .records
            .len(),
        1
    );
    let event = build_dispatch_record(workspace, &root)
        .unwrap()
        .tag(nostr::Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap())
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(&owner)
        .unwrap();
    assert!(fold_dispatch_records([event], workspace, &auth, 200)
        .records
        .is_empty());
}

#[test]
fn project_milestones_reject_ambiguous_identity_and_nil_issue_links() {
    let author = Keys::generate();
    let mut value = record(&author);
    let milestone = Milestone {
        id: Uuid::new_v4(),
        title: "Acceptance".into(),
        target_date: None,
        issue_ids: vec![Uuid::new_v4()],
    };
    value.payload = DispatchRecordPayload::ProjectPlan {
        project_id: Uuid::new_v4(),
        owner: None,
        purpose: "Scope".into(),
        outcome: "Observed delivery".into(),
        target_date: None,
        milestones: vec![milestone.clone()],
        links: vec![],
    };
    assert!(validate_dispatch_record(&value).is_ok());
    if let DispatchRecordPayload::ProjectPlan { milestones, .. } = &mut value.payload {
        milestones.push(milestone);
    }
    assert!(validate_dispatch_record(&value).is_err());
    if let DispatchRecordPayload::ProjectPlan { milestones, .. } = &mut value.payload {
        milestones.pop();
        milestones[0].issue_ids = vec![Uuid::nil()];
    }
    assert!(validate_dispatch_record(&value).is_err());
}
