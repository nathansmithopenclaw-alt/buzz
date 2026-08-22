//! End-to-end regression for restoring Tracker events from a retained cache.
//!
//! Start an isolated relay, then run:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_tracker_restore -- --ignored
//! ```
//!
//! Run once against a normal relay (expects the aged replay to be rejected),
//! then restart it with `BUZZ_RESTORE_INGEST_MAX_AGE_SECS=259200` and run with
//! `BUZZ_E2E_EXPECT_RESTORE_INGEST=true` (expects restore-then-revise to pass).

use std::time::Duration;

use buzz_sdk::tracker::{
    build_tracker_issue, build_tracker_revision, IssueRevision, IssueRoot, IssueStatus,
    RevisionSet, SetOperations, TRACKER_VERSION,
};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

async fn create_test_channel(keys: &Keys) -> Uuid {
    let channel_id = Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", &format!("tracker-restore-e2e-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let response = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-channel event");
    assert!(
        response.status().is_success(),
        "channel creation request failed: {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await.expect("parse channel response");
    assert_eq!(body["accepted"], true, "channel creation rejected: {body}");
    channel_id
}

#[tokio::test]
#[ignore]
async fn retained_root_replay_restores_revision_writes() {
    let keys = Keys::generate();
    let workspace_id = create_test_channel(&keys).await;
    let issue_id = Uuid::new_v4();
    let now = Timestamp::now().as_secs();
    let aged_created_at = now.saturating_sub(172_800);
    let expect_restore_ingest = std::env::var("BUZZ_E2E_EXPECT_RESTORE_INGEST")
        .is_ok_and(|value| value == "true" || value == "1");
    let root = IssueRoot {
        v: TRACKER_VERSION,
        op: "create".into(),
        issue_id,
        identifier: format!("RESTORE-{}", &issue_id.simple().to_string()[..8]),
        title: "Retained cache restore fixture".into(),
        body: "Signed before the database snapshot was restored.".into(),
        status: IssueStatus::Todo,
        priority: None,
        assignee: None,
        project_id: None,
        due: None,
        estimate_minutes: None,
        home_channel_id: Some(workspace_id),
        source_event_id: None,
        workflow: "default-v1".into(),
        labels: vec!["restore-regression".into()],
        external_refs: vec![],
        request_id: Some(Uuid::new_v4()),
        import: None,
    };
    let root_event = build_tracker_issue(workspace_id, &root)
        .expect("build root")
        .custom_created_at(Timestamp::from(aged_created_at))
        .sign_with_keys(&keys)
        .expect("sign root");
    let revision = IssueRevision {
        v: TRACKER_VERSION,
        op: "revise".into(),
        issue_id,
        root_event: root_event.id.to_hex(),
        parent_event: root_event.id.to_hex(),
        revision: 1,
        set: RevisionSet {
            body: Some("Revision accepted after retained root replay.".into()),
            ..RevisionSet::default()
        },
        clear: vec![],
        relations: SetOperations::default(),
        external_refs: SetOperations::default(),
        reason: None,
        request_id: Some(Uuid::new_v4()),
        import: None,
    };
    let revision_event = build_tracker_revision(workspace_id, &revision)
        .expect("build revision")
        .custom_created_at(Timestamp::from(aged_created_at.saturating_add(1)))
        .sign_with_keys(&keys)
        .expect("sign revision");

    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect to isolated relay");

    let before_restore = client
        .send_event(revision_event.clone())
        .await
        .expect("relay response before root replay");
    assert!(!before_restore.accepted, "orphan revision must be rejected");
    if !expect_restore_ingest {
        assert!(
            before_restore
                .message
                .contains("event timestamp too far from server time"),
            "normal relay must reject aged replay at the timestamp guard: {}",
            before_restore.message
        );

        let root_without_window = client
            .send_event(root_event)
            .await
            .expect("relay response for aged root without restore window");
        assert!(!root_without_window.accepted);
        assert!(
            root_without_window
                .message
                .contains("event timestamp too far from server time"),
            "unexpected aged root rejection: {}",
            root_without_window.message
        );
        client.disconnect().await.expect("disconnect cleanly");
        return;
    }
    assert!(
        before_restore.message.contains("reply parent not found"),
        "unexpected orphan rejection: {}",
        before_restore.message
    );

    let restored = client
        .send_event(root_event.clone())
        .await
        .expect("restore retained signed root");
    assert!(
        restored.accepted,
        "root replay rejected: {}",
        restored.message
    );

    let after_restore = client
        .send_event(revision_event.clone())
        .await
        .expect("submit revision after root replay");
    assert!(
        after_restore.accepted,
        "revision after restore rejected: {}",
        after_restore.message
    );

    let subscription = format!("tracker-restore-{issue_id}");
    client
        .subscribe(
            &subscription,
            vec![Filter::new().ids([root_event.id, revision_event.id])],
        )
        .await
        .expect("query restored events");
    let events = client
        .collect_until_eose(&subscription, Duration::from_secs(5))
        .await
        .expect("collect restored events");
    assert_eq!(events.len(), 2, "root and revision must both persist");
    client.disconnect().await.expect("disconnect cleanly");
}
