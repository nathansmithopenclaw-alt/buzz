use super::*;
use crate::dispatch_record::{DecisionOption, DispatchRecord};
use nostr::Keys;
fn root(workspace: Uuid, id: Uuid, author: &Keys) -> Event {
    build_tracker_issue(
        workspace,
        &IssueRoot {
            v: TRACKER_VERSION,
            op: "create".into(),
            issue_id: id,
            identifier: format!("BT-{}", id.simple()),
            title: "Review release".into(),
            body: "Scope and acceptance".into(),
            status: IssueStatus::Todo,
            priority: None,
            assignee: Some(author.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: None,
            home_channel_id: None,
            source_event_id: None,
            workflow: "default-v1".into(),
            labels: vec![],
            external_refs: vec![],
            request_id: None,
            import: None,
        },
    )
    .unwrap()
    .custom_created_at(Timestamp::from(10))
    .sign_with_keys(author)
    .unwrap()
}
fn request(value: DispatchClientOperation) -> DispatchClientRequest {
    DispatchClientRequest {
        v: 1,
        request: value,
    }
}
#[test]
fn mobile_prepares_the_standard_issue_event_and_preserves_owner() {
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let id = Uuid::new_v4();
    let source = root(workspace, id, &author);
    let roles = TrackerAuthorization::new([(author.public_key().to_hex(), TrackerRole::Member)]);
    let result = dispatch_client_response(
        std::slice::from_ref(&source),
        workspace,
        Keys::generate().public_key(),
        author.public_key(),
        roles.clone(),
        request(DispatchClientOperation::PrepareIssue {
            issue_id: id,
            expected_tip: source.id.to_hex(),
            set: MobileIssueSet {
                status: Some(IssueStatus::InProgress),
                ..Default::default()
            },
            clear_assignee: false,
            clear_priority: false,
            reason: "Ready to begin".into(),
            request_id: Uuid::new_v4(),
        }),
        20,
    )
    .unwrap();
    let value = &result["unsigned"];
    let signed = EventBuilder::new(
        Kind::Custom(value["kind"].as_u64().unwrap() as u16),
        value["content"].as_str().unwrap(),
    )
    .tags(serde_json::from_value::<Vec<Tag>>(value["tags"].clone()).unwrap())
    .custom_created_at(Timestamp::from(20))
    .sign_with_keys(&author)
    .unwrap();
    let fold = fold_tracker_events(
        [
            parse_tracker_event(&source, 20).unwrap(),
            parse_tracker_event(&signed, 20).unwrap(),
        ],
        &roles,
    );
    assert_eq!(
        fold.issues[&(workspace, id)].status,
        IssueStatus::InProgress
    );
    assert_eq!(
        fold.issues[&(workspace, id)].assignee,
        Some(author.public_key().to_hex())
    );
}
#[test]
fn mobile_does_not_bypass_terminal_authority_or_frozen_base() {
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let id = Uuid::new_v4();
    let source = root(workspace, id, &author);
    let roles = TrackerAuthorization::new([(author.public_key().to_hex(), TrackerRole::Member)]);
    for (tip, status, expected) in [
        (source.id.to_hex(), IssueStatus::Done, "maintainer"),
        ("f".repeat(64), IssueStatus::InProgress, "changed"),
    ] {
        let result = dispatch_client_response(
            std::slice::from_ref(&source),
            workspace,
            Keys::generate().public_key(),
            author.public_key(),
            roles.clone(),
            request(DispatchClientOperation::PrepareIssue {
                issue_id: id,
                expected_tip: tip,
                set: MobileIssueSet {
                    status: Some(status),
                    ..Default::default()
                },
                clear_assignee: false,
                clear_priority: false,
                reason: "Reviewed".into(),
                request_id: Uuid::new_v4(),
            }),
            20,
        );
        assert!(result.unwrap_err().contains(expected));
    }
}
#[test]
fn named_decider_is_required_even_for_a_maintainer() {
    let owner = Keys::generate();
    let decider = Keys::generate();
    let workspace = Uuid::new_v4();
    let id = Uuid::new_v4();
    let record = DispatchRecord {
        v: 1,
        record_id: Uuid::new_v4(),
        revision: 0,
        parent_event: None,
        request_id: Uuid::new_v4(),
        archived: false,
        payload: DispatchRecordPayload::Decision {
            issue_id: id,
            question: "Ship today?".into(),
            context: "Passing tests".into(),
            decider: decider.public_key().to_hex(),
            options: vec![
                DecisionOption {
                    id: "wait".into(),
                    label: "Wait".into(),
                    consequence: "More review time".into(),
                },
                DecisionOption {
                    id: "ship".into(),
                    label: "Ship".into(),
                    consequence: "Available sooner".into(),
                },
            ],
            evidence: vec![],
            selected_option: None,
            rationale: None,
        },
    };
    let event = build_dispatch_record(workspace, &record)
        .unwrap()
        .custom_created_at(Timestamp::from(10))
        .sign_with_keys(&owner)
        .unwrap();
    let roles = TrackerAuthorization::new([
        (owner.public_key().to_hex(), TrackerRole::Maintainer),
        (decider.public_key().to_hex(), TrackerRole::Member),
    ]);
    let action = DispatchClientOperation::PrepareDecision {
        record_id: record.record_id,
        expected_event: event.id.to_hex(),
        option: "wait".into(),
        rationale: "Independent review is pending".into(),
        request_id: Uuid::new_v4(),
    };
    assert!(dispatch_client_response(
        std::slice::from_ref(&event),
        workspace,
        Keys::generate().public_key(),
        owner.public_key(),
        roles.clone(),
        request(action.clone()),
        20
    )
    .unwrap_err()
    .contains("named decision maker"));
    let prepared = dispatch_client_response(
        &[event],
        workspace,
        Keys::generate().public_key(),
        decider.public_key(),
        roles,
        request(action),
        20,
    )
    .unwrap();
    assert_eq!(prepared["unsigned"]["kind"], 40026);
    let payload: DispatchRecord =
        serde_json::from_str(prepared["unsigned"]["content"].as_str().unwrap()).unwrap();
    assert!(
        matches!(payload.payload,DispatchRecordPayload::Decision{selected_option:Some(ref selected),..} if selected=="wait")
    );
}
#[test]
fn mobile_snapshot_pages_have_stable_ids_and_never_invent_historical_proof() {
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let issuer = Keys::generate().public_key();
    let events = (0..101)
        .map(|_| root(workspace, Uuid::new_v4(), &author))
        .collect::<Vec<_>>();
    let roles = TrackerAuthorization::new([(author.public_key().to_hex(), TrackerRole::Member)]);
    let page = dispatch_client_response(
        &events,
        workspace,
        issuer,
        author.public_key(),
        roles.clone(),
        request(DispatchClientOperation::Snapshot {
            issue_id: None,
            after: None,
            include_closed: false,
        }),
        20,
    )
    .unwrap();
    assert_eq!(page["issues"].as_array().unwrap().len(), 100);
    assert_eq!(page["admission"]["unsealed"], 101);
    let after = page["nextAfter"].as_str().unwrap().parse().unwrap();
    let next = dispatch_client_response(
        &events,
        workspace,
        issuer,
        author.public_key(),
        roles,
        request(DispatchClientOperation::Snapshot {
            issue_id: None,
            after: Some(after),
            include_closed: false,
        }),
        20,
    )
    .unwrap();
    assert_eq!(next["issues"].as_array().unwrap().len(), 1);
    assert!(next["nextAfter"].is_null());
    assert!(!page["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["issueId"] == next["issues"][0]["issueId"]));
}

#[test]
fn mobile_aliases_match_desktop_and_preserve_imported_identifiers() {
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let issuer = Keys::generate().public_key();
    let id = Uuid::new_v4();
    let closed_id = Uuid::new_v4();
    let imported_id = Uuid::new_v4();
    let events = [
        (
            id,
            "smi-aac99c4d66694089ada46b611e4096c2",
            IssueStatus::Todo,
        ),
        (
            closed_id,
            "SMI-aac99c5a66694089ada46b611e4096c2",
            IssueStatus::Done,
        ),
        (imported_id, "BUG-42", IssueStatus::Todo),
    ]
    .into_iter()
    .map(|(id, identifier, status)| {
        let source = root(workspace, id, &author);
        let mut payload: IssueRoot = serde_json::from_str(&source.content).unwrap();
        payload.identifier = identifier.into();
        payload.status = status;
        build_tracker_issue(workspace, &payload)
            .unwrap()
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&author)
            .unwrap()
    })
    .collect::<Vec<_>>();
    let roles =
        TrackerAuthorization::new([(author.public_key().to_hex(), TrackerRole::Maintainer)]);
    for detail in [None, Some(id), Some(imported_id)] {
        let response = dispatch_client_response(
            &events,
            workspace,
            issuer,
            author.public_key(),
            roles.clone(),
            request(DispatchClientOperation::Snapshot {
                issue_id: detail,
                after: None,
                include_closed: false,
            }),
            20,
        )
        .unwrap();
        let issues = response["issues"].as_array().unwrap();
        assert_eq!(issues.len(), if detail.is_some() { 1 } else { 2 });
        for issue in issues {
            if issue["issueId"] == id.to_string() {
                assert_eq!(issue["identifier"], "SMI-AAC99C4D");
            } else {
                assert_eq!(issue["issueId"], imported_id.to_string());
                assert_eq!(issue["identifier"], "BUG-42");
            }
        }
    }
}
