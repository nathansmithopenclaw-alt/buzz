use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

#[test]
fn automation_controls_require_owner_or_admin_even_when_bots_are_maintainers() {
    let relay = Keys::generate();
    let viewer = Keys::generate();
    let owner = Keys::generate();
    let workspace = Uuid::new_v4();
    for (role, expected) in [
        ("owner", true),
        ("admin", true),
        ("bot", false),
        ("member", false),
    ] {
        let membership = EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", &workspace.to_string()]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex(), "", "owner"]).unwrap(),
                Tag::parse(["p", &viewer.public_key().to_hex(), "", role]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&relay)
            .unwrap();
        let value = fold_tracker_context_values(
            vec![],
            vec![],
            json!(membership),
            &workspace.to_string(),
            &viewer.public_key().to_hex(),
            100,
        )
        .unwrap();
        assert_eq!(value["canAuthorizeAutomation"], expected, "role {role}");
        if role == "bot" {
            assert_eq!(value["viewerRole"], "maintainer");
        }
    }
}

#[test]
fn desktop_builder_accepts_the_settled_relay_authorization_payloads_verbatim() {
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let project = Uuid::new_v4();
    let activation = Uuid::new_v4();
    for payload in [
        json!({"type":"capture","channel":workspace,"workspace":workspace,"project":null,"enabled":true}),
        json!({"type":"unowned_alert","workspace":workspace,"channel":null,"after_hours":24,"enabled":true}),
        json!({"type":"autopilot","workspace":workspace,"project":project,"agents":[author.public_key().to_hex()],"label":null,"at_a_time":2,"per_day":10,"reviewer_required":true,"enabled":true,"activation_id":activation,"confirmation_required":true,"confirmation_event":null}),
    ] {
        let record = json!({"v":1,"record_id":Uuid::new_v4(),"revision":0,"parent_event":null,"request_id":Uuid::new_v4(),"archived":false,"payload":payload});
        let parsed = serde_json::from_value(record.clone())
            .expect("settled payload must deserialize in Desktop's SDK");
        let shape = super::super::tracker_mutations::build_dispatch_record_event(
            workspace,
            parsed,
            author.public_key().to_hex(),
        )
        .expect("Desktop record builder must accept the payload");
        assert_eq!(shape.kind, 40026);
        assert_eq!(
            serde_json::from_str::<Value>(&shape.content).unwrap(),
            record
        );
        assert!(shape
            .tags
            .iter()
            .any(|tag| tag == &vec!["h".to_string(), workspace.to_string()]));
    }
}
