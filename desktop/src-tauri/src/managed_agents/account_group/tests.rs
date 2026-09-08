use super::*;

fn fixture() -> Operation {
    let source = uuid::Uuid::new_v4().to_string();
    let destination = uuid::Uuid::new_v4().to_string();
    Operation {
        harness: "claude".into(),
        version: 1,
        id: "d".repeat(32),
        request: ChangeRequest::Default {
            destination: destination.clone(),
        },
        source: Some(source.clone()),
        destination: Some(destination.clone()),
        destination_minted_at: Some(123),
        prior_default: Some(source.clone()),
        change_default: true,
        remove: false,
        phase: "applying".into(),
        error: None,
        members: ['a', 'b', 'c']
            .into_iter()
            .map(|letter| Member {
                name: format!("Agent {letter}"),
                original_account_id: Some(source.clone()),
                original_generation: 7,
                inherit_default: true,
                mode_only: false,
                child: account_switch::Operation {
                    id: letter.to_string().repeat(32),
                    pubkey: letter.to_string().repeat(64),
                    account_id: destination.clone(),
                    generation: 8,
                    phase: "pausing".into(),
                    error: None,
                    runtimes: Vec::new(),
                },
                done: false,
                error: None,
            })
            .collect(),
    }
}

#[test]
fn retry_only_applies_unfinished_members_and_never_finalizes_a_partial_group() {
    let mut operation = fixture();
    let mut disk = Vec::new();
    let complete = execute::run_members(
        &mut operation,
        || false,
        |_, member| {
            if member.name == "Agent b" {
                Err("Destination refused".into())
            } else {
                Ok(())
            }
        },
        |op| {
            disk = serde_json::to_vec(op).unwrap();
            Ok(())
        },
    )
    .unwrap();
    assert!(!complete);
    let mut restored: Operation = serde_json::from_slice(&disk).unwrap();
    assert_eq!(restored.phase, "paused");
    assert!(restored.members[0].done);
    assert!(!restored.members[1].done);
    assert!(restored.members[2].done);
    let mut called = Vec::new();
    assert!(execute::run_members(
        &mut restored,
        || false,
        |_, member| {
            called.push(member.name.clone());
            Ok(())
        },
        |_| Ok(())
    )
    .unwrap());
    assert_eq!(called, ["Agent b"]);
}

#[test]
fn journal_failure_stops_before_the_next_member_and_exposes_retry() {
    let mut operation = fixture();
    let mut calls = 0;
    let result = execute::run_members(
        &mut operation,
        || false,
        |_, _| {
            calls += 1;
            Ok(())
        },
        |_| Err("disk full".into()),
    );
    assert_eq!(result.unwrap_err(), "disk full");
    assert_eq!(calls, 1);
    let status = visible_status(&operation, false);
    assert_eq!(status.phase, "paused");
    assert!(status.error.unwrap().contains("Retry"));
    assert_eq!(visible_status(&operation, true).phase, "applying");
}

#[test]
fn shutdown_during_child_keeps_intent_unfinished_for_idempotent_recovery() {
    let mut operation = fixture();
    let shutdown = std::cell::Cell::new(false);
    let mut disk = Vec::new();
    assert!(!execute::run_members(
        &mut operation,
        || shutdown.get(),
        |_, _| {
            shutdown.set(true);
            Ok(())
        },
        |op| {
            disk = serde_json::to_vec(op).unwrap();
            Ok(())
        }
    )
    .unwrap());
    let restored: Operation = serde_json::from_slice(&disk).unwrap();
    assert!(!restored.members[0].done);
    assert!(restored.error.is_none());
    assert_eq!(restored.phase, "applying");
}

#[test]
fn preview_fingerprint_ignores_random_ids_but_detects_binding_and_stop_changes() {
    let mut operation = fixture();
    let key =
        super::super::ManagedAgentRuntimeKey::new("a".repeat(64), "wss://one.example").unwrap();
    operation.members[0]
        .child
        .runtimes
        .push(account_switch::Runtime {
            key,
            nonce: "e".repeat(32),
            control_dir: PathBuf::from("/private/runtime"),
            paused: false,
            run: true,
        });
    let expected = plan::fingerprint(&operation).unwrap();
    operation.id = "f".repeat(32);
    operation.members[0].child.id = "f".repeat(32);
    assert_eq!(plan::fingerprint(&operation).unwrap(), expected);
    operation.members[0].child.runtimes[0].run = false;
    assert_ne!(plan::fingerprint(&operation).unwrap(), expected);
    operation.members[0].child.runtimes[0].run = true;
    operation.members[0].original_generation += 1;
    assert_ne!(plan::fingerprint(&operation).unwrap(), expected);
}

#[test]
fn group_journal_rejects_duplicates_and_invalid_account_identities() {
    let mut operation = fixture();
    validate(&operation).unwrap();
    operation.members.push(operation.members[0].clone());
    assert!(validate(&operation).is_err());
    operation.members.pop();
    operation.destination = Some("../outside".into());
    assert!(validate(&operation).is_err());
}

#[test]
fn retry_and_renewal_preserve_default_enrollment_but_explicit_switch_leaves_it() {
    assert!(binding_mode(None, true, Some("default"), Some("default")));
    assert!(!binding_mode(None, true, Some("default"), Some("override")));
    assert!(!binding_mode(
        Some(false),
        true,
        Some("default"),
        Some("default")
    ));
    assert!(binding_mode(
        Some(true),
        false,
        Some("override"),
        Some("default")
    ));
    assert!(!binding_mode(None, true, None, None));
}

#[test]
fn provider_journal_versions_preserve_legacy_and_fence_codex_from_old_readers() {
    let mut legacy = serde_json::to_value(fixture()).unwrap();
    legacy.as_object_mut().unwrap().remove("harness");
    let mut operation: Operation = serde_json::from_value(legacy).unwrap();
    assert_eq!(operation.harness, "claude");
    validate(&operation).unwrap();
    let claude_fingerprint = plan::fingerprint(&operation).unwrap();
    operation.harness = "codex".into();
    assert!(validate(&operation).is_err());
    operation.version = 2;
    validate(&operation).unwrap();
    assert_ne!(claude_fingerprint, plan::fingerprint(&operation).unwrap());
    operation.harness = "grok".into();
    validate(&operation).unwrap();
    operation.harness = "unknown".into();
    assert!(validate(&operation).is_err());
}
