//! Local OS token reminders, only while Desktop runs; no credentials or relay IO.
//! macOS uses the existing native permission + delivery helpers because the
//! notification plugin's desktop permission methods always report Granted.
//! Other platforms use the existing notification plugin.
use std::{fs, future::Future, path::Path, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::{store, SubscriptionAccount};

const DAY: i64 = 86_400;
const RETRY_SECONDS: i64 = 3_600;
static STATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Deserialize, Serialize)]
struct ReminderState {
    account_id: String,
    minted_at: i64,
    delivered: Option<i64>,
    retry_at: i64,
}

fn load(root: &Path) -> Result<Vec<ReminderState>, String> {
    store::private_dir(root, false)?;
    let path = root.join("token-reminders.json");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Token reminder state unavailable".into()),
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 131_072 => {
            return Err("Invalid token reminder state".into());
        }
        Ok(_) => {}
    }
    let rows: Vec<ReminderState> =
        serde_json::from_slice(&fs::read(path).map_err(|_| "Token reminder state unavailable")?)
            .map_err(|_| "Invalid token reminder state")?;
    if rows.len() > 100 {
        return Err("Invalid token reminder state".into());
    }
    Ok(rows)
}

fn save(root: &Path, rows: &[ReminderState]) -> Result<(), String> {
    store::private_dir(root, false)?;
    let bytes = serde_json::to_vec(rows).map_err(|_| "Could not encode token reminders")?;
    crate::managed_agents::atomic_write_json_restricted(&root.join("token-reminders.json"), &bytes)
        .map_err(|_| "Could not persist token reminders".into())
}

/// Choose only the most urgent crossed threshold after time away. A clock
/// rollback cannot repeat a delivered threshold or a less urgent one.
fn due_threshold(expires_at: i64, now: i64, delivered: Option<i64>) -> Option<i64> {
    let remaining = expires_at.saturating_sub(now);
    [0, 1, 7, 14, 30]
        .into_iter()
        .find(|days| remaining <= days * DAY && delivered.is_none_or(|previous| *days < previous))
}

fn message(account: &SubscriptionAccount, now: i64) -> String {
    let name = account
        .nickname
        .as_deref()
        .unwrap_or("Claude token account");
    let expires_at = account.expires_at.unwrap_or(now);
    let expiry = chrono::DateTime::from_timestamp(expires_at, 0)
        .map(|date| date.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| format!("Unix time {expires_at}"));
    let remaining = expires_at.saturating_sub(now);
    let status = if remaining <= 0 {
        "has expired".into()
    } else {
        let days = remaining / DAY + i64::from(remaining % DAY != 0);
        let unit = if days == 1 { "day" } else { "days" };
        format!("expires in {days} {unit}")
    };
    format!("Subscription Account “{name}” {status}. Expiry: {expiry}. Renew it in Settings → AI → Subscription Accounts.")
}

fn prepare_rows(
    root: &Path,
    accounts: &[SubscriptionAccount],
) -> Result<Vec<ReminderState>, String> {
    let _guard = STATE_LOCK
        .lock()
        .map_err(|_| "Token reminders unavailable")?;
    let mut rows = load(root)?;
    rows.retain(|row| {
        accounts.iter().any(|account| {
            account.id == row.account_id
                && account.minted_at == Some(row.minted_at)
                && account.kind == "token"
                && account.state != "removal_pending"
        })
    });
    for account in accounts {
        let Some(minted_at) = account.minted_at else {
            continue;
        };
        if account.kind != "token"
            || account.expires_at.is_none()
            || account.state == "removal_pending"
            || rows.iter().any(|row| row.account_id == account.id)
        {
            continue;
        }
        rows.push(ReminderState {
            account_id: account.id.clone(),
            minted_at,
            delivered: None,
            retry_at: 0,
        });
    }
    if rows.len() > 100 {
        return Err("Token reminder account limit reached".into());
    }
    save(root, &rows)?;
    Ok(rows)
}

fn update_row(
    root: &Path,
    row: &ReminderState,
    update: impl FnOnce(&mut ReminderState),
) -> Result<bool, String> {
    let _guard = STATE_LOCK
        .lock()
        .map_err(|_| "Token reminders unavailable")?;
    let mut rows = load(root)?;
    let Some(current) = rows
        .iter_mut()
        .find(|current| current.account_id == row.account_id && current.minted_at == row.minted_at)
    else {
        return Ok(false);
    };
    update(current);
    save(root, &rows)?;
    Ok(true)
}

/// Shared production/fake-notifier seam. Failed permission or delivery keeps
/// the durable hourly retry; success suppresses that threshold across relaunch.
async fn run_due_with<F, Fut>(
    root: &Path,
    accounts: &[SubscriptionAccount],
    now: i64,
    mut notify: F,
) -> Result<usize, String>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let rows = prepare_rows(root, accounts)?;
    let mut delivered = 0;
    let mut deferred = false;
    for row in rows {
        let Some(account) = accounts.iter().find(|account| account.id == row.account_id) else {
            continue;
        };
        let Some(threshold) = due_threshold(account.expires_at.unwrap_or(now), now, row.delivered)
        else {
            continue;
        };
        if row.retry_at > now {
            continue;
        }
        // Persist before attempting the OS call, including permission requests.
        if !update_row(root, &row, |current| {
            current.retry_at = now.saturating_add(RETRY_SECONDS)
        })? {
            continue;
        }
        if notify(message(account, now)).await.is_err() {
            deferred = true;
            continue;
        }
        if update_row(root, &row, |current| current.delivered = Some(threshold))? {
            delivered += 1;
        }
    }
    if deferred {
        Err("Token reminder deferred; retry remains scheduled".into())
    } else {
        Ok(delivered)
    }
}

async fn notify(app: AppHandle, body: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use crate::macos_notifications::{self, NotificationPermissionState};
        let mut permission = macos_notifications::notification_permission_state().await?;
        if permission == NotificationPermissionState::Default {
            permission = macos_notifications::request_notification_access().await?;
        }
        if permission != NotificationPermissionState::Granted {
            return Err("Notification permission is not granted".into());
        }
        crate::commands::show_native_notification(
            app,
            "Subscription Account expiry".into(),
            Some(body),
            None,
        )
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_notification::{NotificationExt, PermissionState};
        tokio::task::spawn_blocking(move || {
            let notifications = app.notification();
            let mut permission = notifications
                .permission_state()
                .map_err(|_| "Notification permission unavailable")?;
            if matches!(
                permission,
                PermissionState::Prompt | PermissionState::PromptWithRationale
            ) {
                permission = notifications
                    .request_permission()
                    .map_err(|_| "Notification permission request failed")?;
            }
            if permission != PermissionState::Granted {
                return Err("Notification permission is not granted".into());
            }
            notifications
                .builder()
                .title("Subscription Account expiry")
                .body(body)
                .show()
                .map_err(|_| "Notification delivery failed".into())
        })
        .await
        .map_err(|_| "Notification task failed")?
    }
}

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(RETRY_SECONDS as u64));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let snapshot = (|| {
                let root = store::root(&app)?;
                let _guard = super::lifecycle_lock()
                    .lock()
                    .map_err(|_| "Accounts unavailable")?;
                let registry = store::load(&root)?;
                Ok::<_, String>((root, registry.accounts))
            })();
            let result = match snapshot {
                Ok((root, accounts)) => {
                    run_due_with(&root, &accounts, chrono::Utc::now().timestamp(), |body| {
                        let app = app.clone();
                        async move {
                            tokio::time::timeout(Duration::from_secs(60), notify(app, body))
                                .await
                                .map_err(|_| "Notification timed out")?
                        }
                    })
                    .await
                }
                Err(error) => Err(error),
            };
            if result.is_err() {
                tracing::warn!("Local subscription token reminder deferred; will retry");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn fixture(expires_at: i64) -> (tempfile::TempDir, SubscriptionAccount) {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let account = serde_json::from_value(serde_json::json!({
            "id": "fixture", "kind": "token", "harness": "claude", "provider": "anthropic",
            "state": "ready", "nickname": "Work", "minted_at": 1, "expires_at": expires_at,
        }))
        .unwrap();
        (dir, account)
    }

    #[test]
    fn thresholds_use_seconds_and_collapse_missed_windows() {
        let now = 1_800_000_000;
        for (seconds, expected) in [
            (31 * DAY, None),
            (30 * DAY + 1, None),
            (30 * DAY, Some(30)),
            (14 * DAY + 1, Some(30)),
            (14 * DAY, Some(14)),
            (7 * DAY, Some(7)),
            (2 * DAY, Some(7)),
            (DAY, Some(1)),
            (1, Some(1)),
            (0, Some(0)),
            (-DAY, Some(0)),
        ] {
            assert_eq!(due_threshold(now + seconds, now, None), expected);
        }
        assert_eq!(due_threshold(now + 20 * DAY, now, Some(7)), None);
        assert_eq!(due_threshold(now, now, Some(7)), Some(0));
        assert_eq!(due_threshold(now - DAY, now, Some(0)), None);
    }

    #[tokio::test]
    async fn first_due_only_and_relaunch_dedup_use_fake_notifier() {
        let expires = 1_800_000_000;
        let (dir, account) = fixture(expires);
        let accounts = [account];
        let calls = RefCell::new(Vec::new());
        let send = |body| {
            calls.borrow_mut().push(body);
            std::future::ready(Ok(()))
        };
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires - 31 * DAY, send)
                .await
                .unwrap(),
            0
        );
        assert!(calls.borrow().is_empty()); // no permission or notification before due
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires - 30 * DAY, send)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires - 29 * DAY, send)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires - 14 * DAY, send)
                .await
                .unwrap(),
            1
        );
        assert_eq!(calls.borrow().len(), 2);
        assert_eq!(load(dir.path()).unwrap()[0].delivered, Some(14));
    }

    #[tokio::test]
    async fn denied_or_failed_delivery_retries_hourly_with_current_expiry_wording() {
        let expires = 1_800_000_000;
        let (dir, account) = fixture(expires);
        let accounts = [account];
        let denied = |_| std::future::ready(Err("fixture permission denied".into()));
        assert!(
            run_due_with(dir.path(), &accounts, expires - 30 * DAY, denied)
                .await
                .is_err()
        );
        let calls = RefCell::new(Vec::new());
        let send = |body| {
            calls.borrow_mut().push(body);
            std::future::ready(Ok(()))
        };
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires - 30 * DAY + 1, send)
                .await
                .unwrap(),
            0
        );
        assert_eq!(load(dir.path()).unwrap()[0].delivered, None);
        // Restart much later: only the current expiry notice, never stale 30-day text.
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires, send)
                .await
                .unwrap(),
            1
        );
        assert!(calls.borrow()[0].contains("has expired"));
        assert_eq!(
            run_due_with(dir.path(), &accounts, expires + RETRY_SECONDS, send)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn renewal_resets_threshold_and_old_ack_cannot_overwrite_new_mint() {
        let expires = 1_800_000_000;
        let (dir, mut account) = fixture(expires);
        let send = |_| std::future::ready(Ok(()));
        run_due_with(dir.path(), &[account.clone()], expires, send)
            .await
            .unwrap();
        let old = load(dir.path()).unwrap().remove(0);
        account.minted_at = Some(expires);
        account.expires_at = Some(expires + 365 * DAY);
        run_due_with(dir.path(), &[account], expires, send)
            .await
            .unwrap();
        assert!(!update_row(dir.path(), &old, |row| row.delivered = Some(0)).unwrap());
        assert_eq!(load(dir.path()).unwrap()[0].delivered, None);
    }

    #[tokio::test]
    async fn corrupt_state_blocks_delivery_instead_of_resetting_dedup() {
        let (dir, account) = fixture(100_000);
        fs::write(dir.path().join("token-reminders.json"), b"bad json").unwrap();
        let calls = RefCell::new(0);
        assert!(run_due_with(dir.path(), &[account], 100_000, |_| {
            *calls.borrow_mut() += 1;
            std::future::ready(Ok(()))
        })
        .await
        .is_err());
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn reminder_days_round_up_and_expiry_is_exact() {
        let (_dir, account) = fixture(100_000);
        assert!(message(&account, 100_000 - DAY - 1).contains("expires in 2 days"));
        assert!(message(&account, 99_999).contains("expires in 1 day."));
        assert!(message(&account, 100_000).contains("has expired"));
        assert!(message(&account, 99_999).contains("Expiry: 1970-01-02T03:46:40Z."));
    }
}
