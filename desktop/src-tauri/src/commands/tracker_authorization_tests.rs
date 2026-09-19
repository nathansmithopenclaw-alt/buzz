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
