use super::*;
use crate::managed_agents::avatar_selection::AvatarSelection;

fn latest() -> AgentProfileInfo {
    AgentProfileInfo {
        display_name: Some("Relay name".into()), picture: Some("https://example.com/a.png".into()),
        about: Some("New biography".into()), tags: vec![], created_at: nostr::Timestamp::now().as_secs() as i64,
        metadata: serde_json::json!({"display_name":"Relay name","about":"New biography","picture":"https://example.com/a.png","nip05":"agent@example.com"}).as_object().unwrap().clone(),
    }
}
#[test]
fn selected_avatar_overrides_stale_event_and_advances_signed_timestamp() {
    let keys = nostr::Keys::generate();
    let previous = latest();
    let stale = build_agent_managed_avatar_event(
        &keys,
        Some(&previous),
        "fallback",
        "https://example.com/a.png",
        None,
    )
    .unwrap();
    for url in [Some("https://example.com/b.png".into()), None] {
        let selection = AvatarSelection {
            revision: 1,
            edit_id: "edit".into(),
            url: url.clone(),
        };
        let event =
            build_selected_profile_event(&keys, &stale, Some(&previous), Some(&selection)).unwrap();
        assert!(event.verify().is_ok());
        assert!(event.created_at.as_secs() > previous.created_at as u64);
        let metadata: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(metadata["picture"], url.unwrap_or_default());
        assert_eq!(metadata["about"], "New biography");
        assert_eq!(metadata["nip05"], "agent@example.com");
    }
}
#[test]
fn conflicting_legacy_avatar_requires_an_explicit_edit() {
    let keys = nostr::Keys::generate();
    let previous = latest();
    let event = build_agent_managed_avatar_event(
        &keys,
        Some(&previous),
        "fallback",
        "https://example.com/a.png",
        None,
    )
    .unwrap();
    let selection = AvatarSelection {
        revision: 0,
        edit_id: "legacy".into(),
        url: Some("https://example.com/b.png".into()),
    };
    assert!(
        build_selected_profile_event(&keys, &event, Some(&previous), Some(&selection))
            .unwrap_err()
            .contains("avatar conflict")
    );
}
#[test]
fn older_device_cannot_publish_over_newer_published_avatar_edit() {
    let keys = nostr::Keys::generate();
    let mut previous = latest();
    previous.metadata.insert(
        "buzz_avatar_edit".into(),
        serde_json::json!({"revision": 20, "edit_id": "new"}),
    );
    let event = build_agent_managed_avatar_event(
        &keys,
        Some(&previous),
        "fallback",
        "https://example.com/old.png",
        None,
    )
    .unwrap();
    let selection = AvatarSelection {
        revision: 19,
        edit_id: "old".into(),
        url: Some("https://example.com/old.png".into()),
    };
    assert!(
        build_selected_profile_event(&keys, &event, Some(&previous), Some(&selection))
            .unwrap_err()
            .contains("newer picture edit")
    );
}
#[test]
fn profile_acknowledgement_requires_acceptance_of_the_exact_event() {
    for (accepted, id) in [(false, "expected"), (true, "other")] {
        let receipt = SubmitEventResponse {
            event_id: id.into(),
            accepted,
            message: String::new(),
        };
        assert!(validate_profile_receipt(&receipt, "expected").is_err());
    }
    assert!(validate_profile_receipt(
        &SubmitEventResponse {
            event_id: "expected".into(),
            accepted: true,
            message: String::new()
        },
        "expected"
    )
    .is_ok());
}
