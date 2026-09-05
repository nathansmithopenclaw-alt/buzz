//! Fork-local setup-token accounts. Only non-secret lifecycle metadata crosses IPC.
use super::{adapters, attempts, lifecycle_lock, store, SubscriptionAccount};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::AppHandle;
use zeroize::Zeroizing;

pub(super) const LIFETIME_SECONDS: i64 = 365 * 86_400;
const TOKEN_FILE: &str = "oauth-token";

/// Read only the selected private credential; every failure is fail-closed.
pub(crate) fn read_token(home: &Path) -> Result<Zeroizing<String>, String> {
    store::private_dir(home, false)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(home.join(TOKEN_FILE))
        .map_err(|_| "Token file unavailable; renew the account")?;
    let metadata = file.metadata().map_err(|_| "Token file unavailable")?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("Invalid token file; renew the account".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o7777 != 0o600 {
            return Err("Token file permissions must be 0600; renew the account".into());
        }
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(4097));
    Read::by_ref(&mut file)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read token file; renew the account")?;
    let raw = std::str::from_utf8(&bytes).map_err(|_| "Invalid token file")?;
    let token = Zeroizing::new(
        crate::managed_agents::normalize_claude_token(raw).map_err(|_| "Invalid token file")?,
    );
    if bytes.len() > 4096
        || !token.starts_with("sk-ant-oat01-")
        || token.as_bytes() != bytes.as_slice()
    {
        return Err("Invalid token file; renew the account".into());
    }
    Ok(token)
}

/// Rename the fixed account entry directly; never canonicalize a credential target.
fn write_token(home: &Path, bytes: &[u8]) -> Result<(), String> {
    store::private_dir(home, false)?;
    let target = home.join(TOKEN_FILE);
    match fs::symlink_metadata(&target) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("Invalid token file; remove the unsafe entry before renewal".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("Token file unavailable".into())
        }
        _ => {}
    }
    let temporary = home.join(format!(".token-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &target)?;
        fs::File::open(home)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| "Could not save private token file".into())
}

/// Extract a setup-token transcript without ever exposing it to logs or IPC.
fn extract_token(output: &[u8]) -> Result<Zeroizing<String>, String> {
    let mut clean = Zeroizing::new(Vec::with_capacity(output.len()));
    let mut escape = false;
    let mut csi = false;
    for &byte in output {
        if escape {
            if !csi && byte == b'[' {
                csi = true;
                continue;
            }
            if !csi || (0x40..=0x7e).contains(&byte) {
                escape = false;
                csi = false;
            }
        } else if byte == 0x1b {
            escape = true;
        } else {
            clean.push(byte);
        }
    }
    let text = std::str::from_utf8(&clean).map_err(|_| "Provider returned invalid token output")?;
    let mut found: Option<Zeroizing<String>> = None;
    for (start, _) in text.match_indices("sk-ant-oat01-") {
        let mut candidate = Zeroizing::new(String::with_capacity(109));
        let mut chars = text[start..].chars();
        for c in chars.by_ref() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                candidate.push(c);
            } else if !c.is_whitespace() {
                break;
            }
            if candidate.len() == 108 {
                break;
            }
        }
        if candidate.len() != 108
            || chars
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        let normalized = Zeroizing::new(
            crate::managed_agents::normalize_claude_token(&candidate)
                .map_err(|_| "Provider returned invalid token")?,
        );
        if found
            .as_ref()
            .is_some_and(|old| old.as_str() != normalized.as_str())
        {
            return Err("Provider returned ambiguous token output; retry minting".into());
        }
        found = Some(normalized);
    }
    found.ok_or_else(|| {
        "Provider did not return a long-lived token; retry browser authorization".into()
    })
}

#[tauri::command]
pub(crate) async fn begin_subscription_account_token(
    account_id: Option<String>,
    app: AppHandle,
) -> Result<SubscriptionAccount, String> {
    tauri::async_runtime::spawn_blocking(move || begin_token(account_id, app))
        .await
        .map_err(|_| "Token worker failed".to_string())?
}

fn begin_token(account_id: Option<String>, app: AppHandle) -> Result<SubscriptionAccount, String> {
    if !cfg!(unix) {
        return Err("Token mint containment unavailable on this platform".into());
    }
    let executable = crate::managed_agents::resolve_command("claude")
        .ok_or("Install Claude before adding its account")?;
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    let mut active = attempts()
        .lock()
        .map_err(|_| "Account login state unavailable")?;
    if !active.is_empty() {
        return Err("Finish or cancel the current browser login first".into());
    }
    let renew = account_id.is_some();
    if !renew && !registry.enabled {
        return Err("Enable Subscription Accounts before adding an account".into());
    }
    let id = account_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if renew {
        let account = store::account_mut(&mut registry, &id)?;
        if account.kind != "token" || account.state == "removal_pending" {
            return Err("This account cannot be renewed as a token account".into());
        }
        if account
            .attempt_started_at
            .is_some_and(|at| chrono::Utc::now().timestamp() < at.saturating_add(302))
        {
            return Err(
                "Previous browser login is still stopping; retry after five minutes".into(),
            );
        }
        store::home_at(&root, &id)?;
    } else {
        if registry.accounts.len() >= 100 {
            return Err("Subscription account limit reached (100)".into());
        }
        let home = root.join(&id);
        if home.exists() {
            return Err("Account path already exists".into());
        }
        store::private_dir(&home, true)?;
        registry.accounts.push(SubscriptionAccount {
            id: id.clone(),
            kind: "token".into(),
            minted_at: None,
            expires_at: None,
            provider: "anthropic".into(),
            harness: "claude".into(),
            nickname: None,
            state: "needs_login".into(),
            last_checked_at: None,
            last_error: None,
            attempt_id: None,
            attempt_started_at: None,
            logout_completed: false,
        });
    }
    registry.schema_version = 2;
    let home = store::home_at(&root, &id)?;
    let attempt = uuid::Uuid::new_v4().to_string();
    let account = store::account_mut(&mut registry, &id)?;
    account.state = "connecting".into();
    account.attempt_id = Some(attempt.clone());
    account.attempt_started_at = Some(chrono::Utc::now().timestamp());
    account.last_error = None;
    let view = account.clone();
    store::save(&root, &registry)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    active.insert(attempt.clone(), cancelled.clone());
    std::thread::spawn(move || {
        let timeout = Duration::from_secs(300);
        let command =
            adapters::isolated_command(&executable, "claude", &home, &["setup-token"], timeout);
        let result = super::token_process::run(command, timeout, &cancelled)
            .and_then(|bytes| extract_token(&bytes))
            .and_then(|token| {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .build()
                    .map_err(|_| "Token verification unavailable".to_string())?;
                let status = tauri::async_runtime::block_on(
                    crate::managed_agents::probe_inference(&client, &token),
                )?;
                Ok((token, status))
            });
        match finish_token(&root, &id, &attempt, result, &cancelled) {
            Ok(Some(minted)) if renew => restart_account(&app, &id, minted),
            Ok(_) => {}
            Err(_) => eprintln!("buzz-desktop: token account result needs recovery"),
        }
    });
    Ok(view)
}

fn finish_token(
    root: &Path,
    id: &str,
    attempt: &str,
    result: Result<(Zeroizing<String>, Option<u16>), String>,
    cancelled: &AtomicBool,
) -> Result<Option<i64>, String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    attempts()
        .lock()
        .map_err(|_| "Account login state unavailable")?
        .remove(attempt);
    let mut registry = store::load(root)?;
    let account = store::account_mut(&mut registry, id)?;
    if account.attempt_id.as_deref() != Some(attempt) || cancelled.load(Ordering::SeqCst) {
        if account.attempt_id.is_none() {
            account.attempt_started_at = None;
            store::save(root, &registry)?;
        }
        return Ok(None);
    }
    let minted = chrono::Utc::now().timestamp();
    let result = result.and_then(|(token, status)| {
        let home = store::home_at(root, id)?;
        write_token(&home, token.as_bytes())?;
        read_token(&home)?;
        Ok(status)
    });
    account.attempt_id = None;
    account.attempt_started_at = None;
    account.last_checked_at = Some(crate::util::now_iso());
    let succeeded = result.is_ok();
    match result {
        Ok(status) => {
            account.minted_at = Some(minted);
            account.expires_at = Some(
                minted
                    .checked_add(LIFETIME_SECONDS)
                    .ok_or("Token expiry overflow")?,
            );
            account.state = "ready".into();
            account.last_error = match status {
                Some(200) => None,
                Some(code) => Some(format!("Token saved; verification returned HTTP {code}. A successful model turn is not confirmed")),
                None => Some("Token saved; verification could not reach the provider. A successful model turn is not confirmed".into()),
            };
        }
        Err(error) => {
            account.state = "unavailable".into();
            account.last_error = Some(error);
        }
    }
    store::save(root, &registry)?;
    Ok(succeeded.then_some(minted))
}

fn restart_account(app: &AppHandle, id: &str, minted: i64) {
    let result = crate::managed_agents::load_managed_agents(app).and_then(|records| {
        let mut failed = 0;
        for record in records.iter().filter(|record| record.subscription_account_id.as_deref() == Some(id)) {
            match crate::managed_agents::subscription_binding::restart_agent_token_account(record.pubkey.clone(), id.into(), minted, app.clone()) {
                Ok(result) => failed += result.failed_restart_count,
                Err(_) => failed += 1,
            }
        }
        if failed == 0 { Ok(()) } else { Err("Token renewed; some agents could not restart. Select this account again in Runtime after their turns finish".into()) }
    });
    if let Err(error) = result {
        let save = || -> Result<(), String> {
            let _guard = lifecycle_lock()
                .lock()
                .map_err(|_| "Account lifecycle unavailable")?;
            let root = store::root(app)?;
            let mut registry = store::load(&root)?;
            let account = store::account_mut(&mut registry, id)?;
            if account.minted_at != Some(minted) {
                return Ok(());
            }
            account.last_error = Some(error);
            store::save(&root, &registry)
        };
        if save().is_err() {
            eprintln!("buzz-desktop: token renewal restart status needs recovery");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> String {
        format!("sk-ant-oat01-{}", "x".repeat(95))
    }

    #[test]
    fn mint_capture_handles_terminal_wrapping_and_rejects_ambiguity() {
        let token = fixture();
        let transcript = format!(
            "Approved\r\n\x1b[32m{}\r\n{}\x1b[0m\r\nKeep safe",
            &token[..60],
            &token[60..]
        );
        assert!(extract_token(transcript.as_bytes()).unwrap().as_str() == token);
        assert!(extract_token(b"Authorization failed").is_err());
        assert!(extract_token(format!("{token}x").as_bytes()).is_err());
        let other = format!("sk-ant-oat01-{}", "y".repeat(95));
        assert!(extract_token(format!("{token}\n{other}").as_bytes()).is_err());
    }

    #[test]
    fn selected_token_file_is_private_bounded_and_never_falls_back() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(read_token(root.path()).is_err());
        let path = root.path().join(TOKEN_FILE);
        fs::write(&path, fixture()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_token(root.path()).is_ok());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(read_token(root.path()).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "x".repeat(4097)).unwrap();
        assert!(read_token(root.path()).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("absent", &path).unwrap();
        assert!(read_token(root.path()).is_err());
        let outside = root.path().join("outside");
        fs::remove_file(&path).unwrap();
        fs::write(&outside, "unchanged").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(write_token(root.path(), fixture().as_bytes()).is_err());
        assert!(fs::read_to_string(&outside).unwrap() == "unchanged");
    }

    #[test]
    fn token_completion_persists_exact_expiry_and_fences_late_mints() {
        let (_temp, root, id) = super::super::tests::registry();
        let mut registry = store::load(&root).unwrap();
        registry.accounts[0].kind = "token".into();
        registry.schema_version = 2;
        store::save(&root, &registry).unwrap();
        assert!(finish_token(
            &root,
            &id,
            "stale",
            Ok((Zeroizing::new(fixture()), Some(200))),
            &AtomicBool::new(false),
        )
        .unwrap()
        .is_none());
        assert!(!root.join(&id).join(TOKEN_FILE).exists());
        assert!(finish_token(
            &root,
            &id,
            "attempt-one",
            Ok((Zeroizing::new(fixture()), Some(200))),
            &AtomicBool::new(false),
        )
        .unwrap()
        .is_some());
        let account = store::load(&root).unwrap().accounts.remove(0);
        assert_eq!(
            account.expires_at.unwrap() - account.minted_at.unwrap(),
            31_536_000
        );
        assert_eq!(account.nickname.as_deref(), Some("Work"));
        let dto = serde_json::to_string(&account).unwrap();
        assert!(!dto.contains("sk-ant-"));
        assert!(read_token(&root.join(id)).is_ok());
    }

    #[test]
    fn minted_but_unverified_token_keeps_a_classified_warning() {
        for status in [None, Some(403), Some(429), Some(500)] {
            let (_temp, root, id) = super::super::tests::registry();
            let mut registry = store::load(&root).unwrap();
            registry.accounts[0].kind = "token".into();
            registry.schema_version = 2;
            store::save(&root, &registry).unwrap();
            finish_token(
                &root,
                &id,
                "attempt-one",
                Ok((Zeroizing::new(fixture()), status)),
                &AtomicBool::new(false),
            )
            .unwrap();
            let account = store::load(&root).unwrap().accounts.remove(0);
            assert_eq!(account.state, "ready");
            assert!(account
                .last_error
                .as_deref()
                .unwrap()
                .contains("not confirmed"));
            assert!(!serde_json::to_string(&account).unwrap().contains("sk-ant-"));
        }
    }

    #[test]
    fn terminal_mint_runner_captures_and_cancels_without_echoing_output() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("provider");
        fs::write(
            &executable,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", fixture()),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let timeout = Duration::from_secs(2);
        let command = adapters::isolated_command(
            &executable,
            "claude",
            root.path(),
            &["setup-token"],
            timeout,
        );
        let output =
            super::super::token_process::run(command, timeout, &Arc::new(AtomicBool::new(false)))
                .unwrap();
        assert!(extract_token(&output).is_ok());
        fs::write(&executable, "#!/bin/sh\nsleep 30\n").unwrap();
        let command = adapters::isolated_command(
            &executable,
            "claude",
            root.path(),
            &["setup-token"],
            timeout,
        );
        assert!(super::super::token_process::run(
            command,
            timeout,
            &Arc::new(AtomicBool::new(true))
        )
        .unwrap_err()
        .contains("cancelled"));
    }
}
