use super::*;
use nostr::Keys;
fn fixture() -> (Keys, Keys, Uuid, Event, DispatchAdmission) {
    let relay = Keys::generate();
    let author = Keys::generate();
    let workspace = Uuid::new_v4();
    let source = EventBuilder::new(Kind::Custom(40020), "fixture")
        .tags([Tag::parse(["h", &workspace.to_string()]).unwrap()])
        .custom_created_at(Timestamp::from(5))
        .sign_with_keys(&author)
        .unwrap();
    let receipt = DispatchAdmission {
        v: 1,
        workspace,
        event_id: source.id.to_hex(),
        author: author.public_key().to_hex(),
        role: "maintainer".into(),
        observed_at: 10,
        basis: AdmissionBasis::Admission,
    };
    (relay, author, workspace, source, receipt)
}
#[test]
fn demotion_preserves_old_role_without_granting_new_authority() {
    let (relay, author, workspace, source, receipt) = fixture();
    let signed = build_dispatch_admission(&receipt)
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();
    let mut auth = TrackerAuthorization::new([(author.public_key().to_hex(), TrackerRole::Member)]);
    let coverage = apply_dispatch_admissions(
        &[source.clone(), signed],
        workspace,
        relay.public_key(),
        &mut auth,
        20,
    );
    assert_eq!(coverage.unsealed, 0);
    assert_eq!(
        auth.event_role(&source.id.to_hex(), &receipt.author),
        Some(TrackerRole::Maintainer)
    );
    assert_eq!(auth.role(&receipt.author), Some(TrackerRole::Member));
    assert_eq!(
        auth.event_role(&"f".repeat(64), &receipt.author),
        Some(TrackerRole::Member)
    );
}
#[test]
fn removal_preserves_receipted_history_but_foreign_receipts_do_not_grant_access() {
    let (relay, _, workspace, source, receipt) = fixture();
    let good = build_dispatch_admission(&receipt)
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();
    let bad = build_dispatch_admission(&receipt)
        .unwrap()
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let mut auth = TrackerAuthorization::default();
    let rejected = apply_dispatch_admissions(
        &[source.clone(), bad],
        workspace,
        relay.public_key(),
        &mut auth,
        20,
    );
    assert_eq!(rejected.unsealed, 1);
    assert_eq!(auth.event_role(&source.id.to_hex(), &receipt.author), None);
    apply_dispatch_admissions(
        &[source.clone(), good],
        workspace,
        relay.public_key(),
        &mut auth,
        20,
    );
    assert_eq!(auth.role(&receipt.author), None);
    assert_eq!(
        auth.event_role(&source.id.to_hex(), &receipt.author),
        Some(TrackerRole::Maintainer)
    );
}
#[test]
fn contradictory_receipts_and_future_observations_fail_closed() {
    let (relay, _, workspace, source, receipt) = fixture();
    let good = build_dispatch_admission(&receipt)
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();
    let mut changed = receipt.clone();
    changed.role = "member".into();
    let conflict = build_dispatch_admission(&changed)
        .unwrap()
        .sign_with_keys(&relay)
        .unwrap();
    let mut auth = TrackerAuthorization::new([(receipt.author.clone(), TrackerRole::Maintainer)]);
    let coverage = apply_dispatch_admissions(
        &[source.clone(), good.clone(), conflict],
        workspace,
        relay.public_key(),
        &mut auth,
        20,
    );
    assert_eq!(coverage.unsealed, 1);
    assert_eq!(auth.event_role(&source.id.to_hex(), &receipt.author), None);
    let mut auth = TrackerAuthorization::default();
    assert_eq!(
        apply_dispatch_admissions(&[source, good], workspace, relay.public_key(), &mut auth, 9)
            .unsealed,
        1
    );
}
#[test]
fn baseline_is_explicit_and_none_never_masquerades_as_admission() {
    let (_, _, _, _, mut receipt) = fixture();
    receipt.role = "none".into();
    assert!(build_dispatch_admission(&receipt).is_err());
    receipt.basis = AdmissionBasis::LegacyBaseline;
    assert!(build_dispatch_admission(&receipt).is_ok());
}
