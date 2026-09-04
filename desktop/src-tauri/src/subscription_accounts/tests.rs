use super::*;

fn registry() -> (tempfile::TempDir, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("accounts");
    store::private_dir(&root, true).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    store::private_dir(&root.join(&id), true).unwrap();
    let account = SubscriptionAccount {
        id: id.clone(),
        provider: "anthropic".into(),
        harness: "claude".into(),
        nickname: Some("Work".into()),
        state: "connecting".into(),
        last_checked_at: None,
        last_error: None,
        attempt_id: Some("attempt-one".into()),
        attempt_started_at: None,
        logout_completed: false,
    };
    store::save(
        &root,
        &store::Registry {
            schema_version: 1,
            enabled: true,
            accounts: vec![account],
        },
    )
    .unwrap();
    (temp, root, id)
}

#[test]
fn stale_attempt_cannot_mark_reconnected_account_ready() {
    let (_temp, root, id) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].attempt_id = Some("attempt-two".into());
    store::save(&root, &registry).unwrap();
    finish_attempt(&root, &id, "attempt-one", Ok(())).unwrap();
    let account = store::load(&root).unwrap().accounts.remove(0);
    assert_eq!(account.state, "connecting");
    assert_eq!(account.attempt_id.as_deref(), Some("attempt-two"));
}

#[test]
fn cancelled_completion_preserves_cancellation_and_nickname() {
    let (_temp, root, id) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].state = "needs_login".into();
    registry.accounts[0].attempt_id = None;
    store::save(&root, &registry).unwrap();
    finish_attempt(&root, &id, "attempt-one", Ok(())).unwrap();
    let account = store::load(&root).unwrap().accounts.remove(0);
    assert_eq!(account.state, "needs_login");
    assert_eq!(account.nickname.as_deref(), Some("Work"));
}

#[test]
fn subscription_status_rejects_generic_or_api_key_authentication() {
    for value in [
        r#"{"loggedIn":true}"#,
        r#"{"loggedIn":true,"authMethod":"api_key"}"#,
        r#"{"loggedIn":false,"authMethod":"claude.ai"}"#,
    ] {
        assert!(adapters::verify_status("claude", value.as_bytes()).is_err());
    }
    assert!(
        adapters::verify_status("claude", br#"{"loggedIn":true,"authMethod":"claude.ai"}"#).is_ok()
    );
    assert!(adapters::verify_status("codex", b"Logged in using ChatGPT\n").is_ok());
    assert!(adapters::verify_status("codex", b"Logged in using an API key\n").is_err());
    assert!(adapters::verify_status("grok", b"Logged in using ChatGPT\n").is_err());
}

#[cfg(unix)]
#[test]
fn registry_and_home_reject_symlinks_and_traversal() {
    use std::os::unix::fs::symlink;
    let (temp, root, id) = registry();
    assert!(store::home_at(&root, "../elsewhere").is_err());
    std::fs::remove_dir(root.join(&id)).unwrap();
    symlink(temp.path(), root.join(&id)).unwrap();
    assert!(store::home_at(&root, &id).is_err());
    std::fs::remove_file(root.join("accounts.json")).unwrap();
    let target = temp.path().join("target");
    std::fs::write(&target, b"untouched").unwrap();
    symlink(&target, root.join("accounts.json")).unwrap();
    assert!(store::load(&root).is_err());
    assert!(store::save(
        &root,
        &store::Registry {
            schema_version: 1,
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"untouched");
}

#[cfg(unix)]
#[test]
fn dangling_registry_link_never_becomes_an_empty_store() {
    let (temp, root, _) = registry();
    std::fs::remove_file(root.join("accounts.json")).unwrap();
    std::os::unix::fs::symlink(temp.path().join("missing"), root.join("accounts.json")).unwrap();
    assert!(store::load(&root).is_err());
    assert!(store::save(
        &root,
        &store::Registry {
            schema_version: 1,
            ..Default::default()
        }
    )
    .is_err());
    assert!(!temp.path().join("missing").exists());
}

#[test]
fn malformed_registry_and_duplicate_ids_fail_closed() {
    let (_temp, root, _) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts.push(registry.accounts[0].clone());
    assert!(store::save(&root, &registry).is_err());
    std::fs::write(root.join("accounts.json"), b"not json").unwrap();
    assert!(store::load(&root).is_err());
    assert!(store::save(
        &root,
        &store::Registry {
            schema_version: 1,
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(
        std::fs::read(root.join("accounts.json")).unwrap(),
        b"not json"
    );
}

#[cfg(unix)]
#[test]
fn metadata_and_homes_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, root, id) = registry();
    for path in [&root, &root.join(&id)] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        std::fs::metadata(root.join("accounts.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::set_permissions(root.join(&id), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(store::home_at(&root, &id).is_err());
}

#[cfg(unix)]
fn fake_command(script: &str, timeout: std::time::Duration) -> std::process::Command {
    process::command(std::path::Path::new("/bin/sh"), &["-c", script], timeout)
}

#[cfg(unix)]
#[test]
fn process_success_captures_status_without_leaving_descendants() {
    let timeout = std::time::Duration::from_secs(2);
    let output = process::run(
        fake_command(
            "printf 'Logged in using ChatGPT\\n' >&2; sleep 30 &",
            timeout,
        ),
        timeout,
        &Arc::new(AtomicBool::new(false)),
        true,
    )
    .unwrap();
    assert_eq!(output, b"Logged in using ChatGPT\n");
}

#[cfg(unix)]
#[test]
fn process_timeout_and_cancel_are_bounded() {
    let timeout = std::time::Duration::from_millis(100);
    let start = std::time::Instant::now();
    let result = process::run(
        fake_command("trap '' TERM; sleep 30", timeout),
        timeout,
        &Arc::new(AtomicBool::new(false)),
        false,
    );
    assert!(result.unwrap_err().contains("timed out"));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    let result = process::run(
        fake_command("sleep 30", timeout),
        timeout,
        &Arc::new(AtomicBool::new(true)),
        false,
    );
    assert!(result.unwrap_err().contains("cancelled"));
}

#[cfg(unix)]
#[test]
fn oversized_or_failed_status_never_becomes_ready() {
    let timeout = std::time::Duration::from_secs(2);
    let result = process::run(
        fake_command("head -c 20000 /dev/zero", timeout),
        timeout,
        &Arc::new(AtomicBool::new(false)),
        true,
    );
    assert!(result.is_err());
    assert!(process::run(
        fake_command("echo Logged in using ChatGPT; exit 1", timeout),
        timeout,
        &Arc::new(AtomicBool::new(false)),
        true
    )
    .is_err());
}

#[test]
fn successful_verification_retains_identity_and_labels_check_kind_honestly() {
    let (_temp, root, id) = registry();
    finish_attempt(&root, &id, "attempt-one", Ok(())).unwrap();
    let account = store::load(&root).unwrap().accounts.remove(0);
    assert_eq!(account.id, id);
    assert_eq!(account.nickname.as_deref(), Some("Work"));
    assert_eq!(account.state, "ready");
    assert!(account.last_checked_at.is_some());
    assert!(account.attempt_id.is_none());
}

#[test]
fn failed_login_retains_reconnectable_record() {
    let (_temp, root, id) = registry();
    finish_attempt(
        &root,
        &id,
        "attempt-one",
        Err("Provider command failed".into()),
    )
    .unwrap();
    let account = store::load(&root).unwrap().accounts.remove(0);
    assert_eq!(account.state, "unavailable");
    assert_eq!(account.id, id);
    assert_eq!(
        account.last_error.as_deref(),
        Some("Provider command failed")
    );
}

#[test]
fn logout_failure_preserves_tombstone_and_retry_deletes_only_owned_home() {
    let (_temp, root, id) = registry();
    let home = root.join(&id);
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].state = "removal_pending".into();
    registry.accounts[0].attempt_id = None;
    store::save(&root, &registry).unwrap();
    assert!(finish_removal(&root, &id, &home, Err("Provider logout failed".into())).is_err());
    let pending = store::load(&root).unwrap();
    assert_eq!(pending.accounts[0].state, "removal_pending");
    assert!(!pending.accounts[0].logout_completed);
    assert!(home.is_dir());
    finish_removal(&root, &id, &home, Ok(())).unwrap();
    assert!(!home.exists());
    assert!(store::load(&root).unwrap().accounts.is_empty());
}

#[test]
fn logout_completed_tombstone_survives_missing_home_and_finishes_removal() {
    let (_temp, root, id) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].state = "removal_pending".into();
    registry.accounts[0].logout_completed = true;
    registry.accounts[0].attempt_id = None;
    store::save(&root, &registry).unwrap();
    std::fs::remove_dir(root.join(&id)).unwrap();
    finish_removal(&root, &id, &root.join(&id), Ok(())).unwrap();
    assert!(store::load(&root).unwrap().accounts.is_empty());
}

#[test]
fn live_lease_blocks_removal_until_runtime_drops_it() {
    let (_temp, root, id) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].state = "removal_pending".into();
    registry.accounts[0].attempt_id = None;
    store::save(&root, &registry).unwrap();
    leases().lock().unwrap().insert(
        id.clone(),
        LeaseEntry {
            token: "test".into(),
            pid: None,
        },
    );
    let lease = SubscriptionLease {
        id: id.clone(),
        pid: None,
        token: "test".into(),
        reservation: None,
    };
    assert!(ensure_unused(&id).is_err());
    assert!(finish_removal(&root, &id, &root.join(&id), Ok(())).is_err());
    assert!(root.join(&id).is_dir());
    drop(lease);
    finish_removal(&root, &id, &root.join(&id), Ok(())).unwrap();
    assert!(store::load(&root).unwrap().accounts.is_empty());
}

#[cfg(unix)]
#[test]
fn dropped_live_process_lease_remains_busy_until_process_exits() {
    let id = uuid::Uuid::new_v4().to_string();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("20")
        .spawn()
        .unwrap();
    leases().lock().unwrap().insert(
        id.clone(),
        LeaseEntry {
            token: "test".into(),
            pid: None,
        },
    );
    let mut lease = SubscriptionLease {
        id: id.clone(),
        pid: None,
        token: "test".into(),
        reservation: None,
    };
    lease.attach_pid(child.id()).unwrap();
    drop(lease);
    assert!(ensure_unused(&id).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ensure_unused(&id).is_ok());
}

#[test]
fn old_lease_drop_cannot_release_replacement_generation() {
    let id = uuid::Uuid::new_v4().to_string();
    let lease = SubscriptionLease {
        id: id.clone(),
        pid: None,
        token: "old".into(),
        reservation: None,
    };
    leases().lock().unwrap().insert(
        id.clone(),
        LeaseEntry {
            token: "new".into(),
            pid: None,
        },
    );
    drop(lease);
    assert!(ensure_unused(&id).is_err());
    leases().lock().unwrap().remove(&id);
}

#[cfg(unix)]
#[test]
fn durable_live_receipt_refuses_reconnect_even_without_memory_lease() {
    let temp = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let receipt = serde_json::json!({"key":{"pubkey":"a".repeat(64),"relayUrl":"wss://example.test"},"pid":std::process::id(),"desktopInstanceId":"test","startedAt":"2026-09-04T00:00:00Z","subscriptionAccountId":id,"subscriptionAccountGeneration":1});
    std::fs::write(
        temp.path().join("receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(ensure_no_live_receipts_at(temp.path(), &id).is_err());
    assert!(ensure_no_live_receipts_at(temp.path(), &uuid::Uuid::new_v4().to_string()).is_ok());
    std::fs::write(temp.path().join("receipt.json"), b"broken").unwrap();
    assert!(ensure_no_live_receipts_at(temp.path(), &id).is_err());
}

#[test]
fn removal_attempt_is_persistently_fenced_until_cleanup_finishes() {
    let (_temp, root, id) = registry();
    let mut registry = store::load(&root).unwrap();
    registry.accounts[0].attempt_id = None;
    start_removal(&mut registry.accounts[0]).unwrap();
    store::save(&root, &registry).unwrap();
    let mut recovered = store::load(&root).unwrap();
    assert!(start_removal(&mut recovered.accounts[0]).is_err());
    assert!(finish_removal(&root, &id, &root.join(&id), Err("Logout failed".into())).is_err());
    let mut recovered = store::load(&root).unwrap();
    start_removal(&mut recovered.accounts[0]).unwrap();
}

#[test]
fn unknown_pre_spawn_reservation_blocks_after_in_memory_state_is_lost() {
    let (_temp, root, id) = registry();
    let handle = reservation::create(&root, &id, &uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(reservation::ensure_clear(&root, &id).is_err());
    drop(handle); // No runtime owner survived to prove whether spawn happened.
    assert!(reservation::ensure_clear(&root, &id).is_err());
    assert!(reservation::create(&root, &id, &uuid::Uuid::new_v4().to_string()).is_err());
}

#[cfg(unix)]
#[test]
fn recorded_reservation_releases_only_after_process_exit() {
    let (_temp, root, id) = registry();
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("20")
        .spawn()
        .unwrap();
    let handle = reservation::create(&root, &id, &uuid::Uuid::new_v4().to_string()).unwrap();
    handle.attach_pid(child.id()).unwrap();
    assert!(reservation::ensure_clear(&root, &id).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    reservation::ensure_clear(&root, &id).unwrap();
    let next = reservation::create(&root, &id, &uuid::Uuid::new_v4().to_string()).unwrap();
    handle.release().unwrap(); // An old token cannot erase the new reservation.
    assert!(reservation::ensure_clear(&root, &id).is_err());
    next.release().unwrap();
    reservation::ensure_clear(&root, &id).unwrap();
}

#[cfg(unix)]
#[test]
fn reservation_rejects_symlink_and_unparseable_crash_record() {
    let (temp, root, id) = registry();
    let path = root.join(format!("{id}.lease.json"));
    std::os::unix::fs::symlink(temp.path().join("missing"), &path).unwrap();
    assert!(reservation::ensure_clear(&root, &id).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"partial").unwrap();
    assert!(reservation::ensure_clear(&root, &id).is_err());
}
