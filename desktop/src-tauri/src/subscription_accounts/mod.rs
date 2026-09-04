//! Local subscription accounts. The harness owns all provider credentials.
mod adapters;
mod process;
mod reservation;
mod store;

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{AppHandle, Manager};

pub(crate) use adapters::SubscriptionAccountCapability;

/// Non-secret local account state. Paths and provider output never enter DTOs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SubscriptionAccount {
    pub id: String,
    pub provider: String,
    pub harness: String,
    pub nickname: Option<String>,
    pub state: String,
    pub last_checked_at: Option<String>,
    pub last_error: Option<String>,
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub attempt_started_at: Option<i64>,
    #[serde(default)]
    pub logout_completed: bool,
}

/// Serializes account lifecycle with spawn and saved binding changes.
pub(crate) fn lifecycle_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn attempts() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone)]
struct LeaseEntry {
    token: String,
    pid: Option<u32>,
}

fn leases() -> &'static Mutex<HashMap<String, LeaseEntry>> {
    static LEASES: OnceLock<Mutex<HashMap<String, LeaseEntry>>> = OnceLock::new();
    LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Exclusive live consumer until authenticated concurrent refresh is proven.
#[derive(Debug)]
pub(crate) struct SubscriptionLease {
    id: String,
    pid: Option<u32>,
    token: String,
    reservation: Option<reservation::ReservationHandle>,
}

impl SubscriptionLease {
    /// Attach immediately after spawn, before any fallible runtime setup.
    pub(crate) fn attach_pid(&mut self, pid: u32) -> Result<(), String> {
        self.pid = Some(pid);
        if let Ok(mut leases) = leases().lock() {
            if let Some(entry) = leases.get_mut(&self.id) {
                if entry.token == self.token {
                    entry.pid = Some(pid);
                }
            }
        }
        if let Some(reservation) = &self.reservation {
            reservation.attach_pid(pid)?;
        }
        Ok(())
    }
}

impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        if self
            .pid
            .is_some_and(crate::managed_agents::process_is_running)
        {
            // Cleanup failed or ownership was lost. Retain a busy lease while
            // that PID lives; reconnect must never race its credential refresh.
            return;
        }
        // A poisoned map remains conservatively busy for future admissions.
        if let Ok(mut leases) = leases().lock() {
            if leases
                .get(&self.id)
                .is_some_and(|entry| entry.token == self.token)
            {
                leases.remove(&self.id);
            }
        }
        if let Some(reservation) = &self.reservation {
            if reservation.release().is_err() {
                eprintln!("buzz-desktop: stopped account runtime reservation needs cleanup");
            }
        }
    }
}

fn ensure_unused(id: &str) -> Result<(), String> {
    let mut leases = leases()
        .lock()
        .map_err(|_| "Account lease state unavailable")?;
    if leases
        .get(id)
        .and_then(|entry| entry.pid)
        .is_some_and(|pid| !crate::managed_agents::process_is_running(pid))
    {
        leases.remove(id);
    }
    if leases.contains_key(id) {
        return Err(
            "Account is in use by a running agent; stop it before reconnecting or removing".into(),
        );
    }
    Ok(())
}

fn ensure_no_live_receipts(app: &AppHandle, id: &str) -> Result<(), String> {
    let root = store::root(app)?;
    reservation::ensure_clear(&root, id)?;
    let data = root.parent().ok_or("Account root unavailable")?;
    let directory = data.join("agents/agent-pids");
    ensure_no_live_receipts_at(&directory, id)
}

fn ensure_no_live_receipts_at(directory: &std::path::Path, id: &str) -> Result<(), String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Could not inspect saved runtime leases".into()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| "Could not inspect saved runtime lease")?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| "Could not inspect saved runtime lease")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 65536 {
            return Err("Invalid saved runtime lease".into());
        }
        let bytes = std::fs::read(path).map_err(|_| "Could not read saved runtime lease")?;
        let receipt: crate::managed_agents::ManagedAgentRuntimeReceipt =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid saved runtime lease")?;
        if receipt.subscription_account_id.as_deref() == Some(id)
            && crate::managed_agents::process_is_running(receipt.pid)
        {
            return Err(
                "Account is still used by a surviving runtime; stop that runtime before continuing"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Read under [`lifecycle_lock`]. Missing or invalid metadata fails closed.
pub(crate) fn load_account(app: &AppHandle, id: &str) -> Result<SubscriptionAccount, String> {
    store::load(&store::root(app)?)?
        .accounts
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| "Subscription account unavailable".into())
}

/// Resolve an existing private opaque home. Never recreate a missing home.
pub(crate) fn account_home(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    store::home_at(&store::root(app)?, id)
}

/// Caller holds [`lifecycle_lock`] until the lease has been attached to runtime.
pub(crate) fn acquire_runtime_lease(
    app: &AppHandle,
    id: &str,
    harness: &str,
) -> Result<SubscriptionLease, String> {
    let account = load_account(app, id)?;
    if account.state != "ready" || account.nickname.is_none() {
        return Err("Subscription account needs login and a nickname before use".into());
    }
    if account.harness != harness {
        return Err("Subscription account does not match agent harness".into());
    }
    adapters::provider(harness)?;
    account_home(app, id)?;
    ensure_no_live_receipts(app, id)?;
    ensure_unused(id)?;
    let mut leases = leases()
        .lock()
        .map_err(|_| "Account lease state unavailable")?;
    if leases.contains_key(id) {
        return Err("Subscription account is busy; one live consumer is supported until shared refresh is validated".into());
    }
    let token = uuid::Uuid::new_v4().to_string();
    let reservation = reservation::create(&store::root(app)?, id, &token)?;
    leases.insert(
        id.to_string(),
        LeaseEntry {
            token: token.clone(),
            pid: None,
        },
    );
    Ok(SubscriptionLease {
        id: id.to_string(),
        pid: None,
        token,
        reservation: Some(reservation),
    })
}

/// Read creation/editing flag; existing bindings must not consult this flag.
pub(crate) fn accounts_enabled(app: &AppHandle) -> Result<bool, String> {
    Ok(store::load(&store::root(app)?)?.enabled)
}

/// Record a definitive runtime authentication failure; quota and network
/// errors must never call this. Caller holds [`lifecycle_lock`].
pub(crate) fn mark_needs_login(app: &AppHandle, id: &str) -> Result<(), String> {
    let root = store::root(app)?;
    let mut registry = store::load(&root)?;
    let account = store::account_mut(&mut registry, id)?;
    if account.state == "ready" {
        account.state = "needs_login".into();
        account.last_error = Some("Provider authentication expired; stop the agent and reconnect, then resume its pending work".into());
        store::save(&root, &registry)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_subscription_accounts_enabled(app: AppHandle) -> Result<bool, String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    accounts_enabled(&app)
}

#[tauri::command]
pub(crate) fn set_subscription_accounts_enabled(
    enabled: bool,
    app: AppHandle,
) -> Result<(), String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    registry.enabled = enabled;
    store::save(&root, &registry)
}

#[tauri::command]
pub(crate) fn subscription_account_capabilities() -> Vec<SubscriptionAccountCapability> {
    adapters::capabilities()
}

#[tauri::command]
pub(crate) fn list_subscription_accounts(
    app: AppHandle,
) -> Result<Vec<SubscriptionAccount>, String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    let attempts = attempts()
        .lock()
        .map_err(|_| "Account login state unavailable")?;
    let mut changed = false;
    for account in &mut registry.accounts {
        if account.state == "connecting"
            && !account
                .attempt_id
                .as_ref()
                .is_some_and(|id| attempts.contains_key(id))
        {
            // The subprocess watchdog bounds an interrupted login. Do not
            // automatically probe or re-use its readiness after restart.
            account.state = "needs_login".into();
            account.attempt_id = None;
            account.last_error = Some(
                "Login interrupted by Desktop restart; reconnect to check authentication".into(),
            );
            changed = true;
        }
        if account.state == "ready" && store::home_at(&root, &account.id).is_err() {
            account.state = "unavailable".into();
            account.last_error =
                Some("Account directory unavailable; restore it or remove this account".into());
            changed = true;
        }
    }
    if changed {
        store::save(&root, &registry)?;
    }
    Ok(registry.accounts)
}

#[tauri::command]
pub(crate) async fn begin_subscription_account_login(
    harness: String,
    account_id: Option<String>,
    app: AppHandle,
) -> Result<SubscriptionAccount, String> {
    tauri::async_runtime::spawn_blocking(move || begin_account_login(harness, account_id, app))
        .await
        .map_err(|_| "Account login worker failed".to_string())?
}

fn begin_account_login(
    harness: String,
    account_id: Option<String>,
    app: AppHandle,
) -> Result<SubscriptionAccount, String> {
    let provider = adapters::provider(&harness)?;
    let executable = crate::managed_agents::resolve_command(&harness)
        .ok_or("Install the provider CLI before adding its account")?;
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    if account_id.is_none() && !registry.enabled {
        return Err("Enable the Subscription Accounts experiment before adding an account".into());
    }
    let mut active = attempts()
        .lock()
        .map_err(|_| "Account login state unavailable")?;
    if !active.is_empty() {
        return Err("Finish or cancel the current browser login first".into());
    }
    let reconnect = account_id.is_some();
    let id = account_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    ensure_unused(&id)?;
    ensure_no_live_receipts(&app, &id)?;
    let home = if registry.accounts.iter().any(|account| account.id == id) {
        let account = store::account_mut(&mut registry, &id)?;
        if account.harness != harness {
            return Err("Reconnect must use the original harness".into());
        }
        if account.state == "removal_pending" {
            return Err("Account removal is pending; retry removal".into());
        }
        if account
            .attempt_started_at
            .is_some_and(|started| chrono::Utc::now().timestamp() < started.saturating_add(302))
        {
            return Err(
                "Interrupted login is still being contained; retry after its five minute deadline"
                    .into(),
            );
        }
        store::home_at(&root, &id)?
    } else {
        // Caller-provided unknown ids must not become new profile paths.
        if reconnect {
            return Err("Subscription account not found".into());
        }
        if registry.accounts.len() >= 100 {
            return Err("Subscription account limit reached (100)".into());
        }
        let home = root.join(&id);
        if home.exists() {
            return Err("Account path already exists; paths are never adopted or reused".into());
        }
        store::private_dir(&home, true)?;
        registry.accounts.push(SubscriptionAccount {
            id: id.clone(),
            provider: provider.into(),
            harness: harness.clone(),
            nickname: None,
            state: "needs_login".into(),
            last_checked_at: None,
            last_error: None,
            attempt_id: None,
            attempt_started_at: None,
            logout_completed: false,
        });
        home
    };
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let account = store::account_mut(&mut registry, &id)?;
    account.state = "connecting".into();
    account.last_error = None;
    account.attempt_id = Some(attempt_id.clone());
    account.attempt_started_at = Some(chrono::Utc::now().timestamp());
    let view = account.clone();
    store::save(&root, &registry)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    active.insert(attempt_id.clone(), cancelled.clone());
    std::thread::spawn(move || {
        let result = adapters::operation(&executable, &harness, &home, "login", &cancelled)
            .and_then(|()| adapters::operation(&executable, &harness, &home, "status", &cancelled));
        let finish = finish_attempt(&root, &id, &attempt_id, result);
        if finish.is_err() {
            // Persisted Connecting remains a recovery record. No provider
            // output or credential path is logged.
            eprintln!("buzz-desktop: could not persist subscription login result");
        }
    });
    Ok(view)
}

fn finish_attempt(
    root: &std::path::Path,
    id: &str,
    attempt_id: &str,
    result: Result<(), String>,
) -> Result<(), String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let mut active = attempts()
        .lock()
        .map_err(|_| "Account login state unavailable")?;
    active.remove(attempt_id);
    let mut registry = store::load(root)?;
    let account = store::account_mut(&mut registry, id)?;
    if account.attempt_id.as_deref() != Some(attempt_id) {
        // Cancellation invalidated the result but cleanup has now completed.
        if account.state == "needs_login" && account.attempt_id.is_none() {
            account.attempt_started_at = None;
            store::save(root, &registry)?;
        }
        return Ok(());
    }
    account.attempt_id = None;
    account.attempt_started_at = None;
    account.last_checked_at = Some(crate::util::now_iso());
    match result {
        Ok(()) => {
            account.state = "ready".into();
            account.last_error = None;
        }
        Err(error) => {
            account.state =
                if error.contains("cancelled") || error.contains("subscription authentication") {
                    "needs_login"
                } else {
                    "unavailable"
                }
                .into();
            account.last_error = Some(error);
        }
    }
    store::save(root, &registry)
}

#[tauri::command]
pub(crate) fn cancel_subscription_account_login(
    id: String,
    app: AppHandle,
) -> Result<SubscriptionAccount, String> {
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    let account = store::account_mut(&mut registry, &id)?;
    if let Some(attempt_id) = account.attempt_id.take() {
        if let Some(cancelled) = attempts()
            .lock()
            .map_err(|_| "Account login state unavailable")?
            .get(&attempt_id)
        {
            cancelled.store(true, Ordering::SeqCst);
        }
    }
    if account.state == "connecting" {
        account.state = "needs_login".into();
        account.last_error = Some("Login cancelled".into());
    }
    let view = account.clone();
    store::save(&root, &registry)?;
    Ok(view)
}

#[tauri::command]
pub(crate) fn rename_subscription_account(
    id: String,
    nickname: String,
    app: AppHandle,
) -> Result<SubscriptionAccount, String> {
    let nickname = nickname.trim();
    if nickname.is_empty()
        || nickname.chars().count() > 80
        || nickname.chars().any(char::is_control)
    {
        return Err("Nickname must contain 1 to 80 visible characters".into());
    }
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = store::root(&app)?;
    let mut registry = store::load(&root)?;
    let account = store::account_mut(&mut registry, &id)?;
    if account.state != "ready" && account.nickname.is_none() {
        return Err("Complete subscription login before naming this account".into());
    }
    account.nickname = Some(nickname.into());
    let view = account.clone();
    store::save(&root, &registry)?;
    Ok(view)
}

#[tauri::command]
pub(crate) async fn remove_subscription_account(id: String, app: AppHandle) -> Result<(), String> {
    // Do not block Tauri's UI thread during bounded provider logout.
    tauri::async_runtime::spawn_blocking(move || remove_account(&id, &app))
        .await
        .map_err(|_| "Account removal worker failed".to_string())?
}

fn remove_account(id: &str, app: &AppHandle) -> Result<(), String> {
    let (root, home, harness, logout_completed) = {
        let state = app.state::<crate::app_state::AppState>();
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|_| "Agent store unavailable")?;
        let _guard = lifecycle_lock()
            .lock()
            .map_err(|_| "Account lifecycle unavailable")?;
        ensure_unused(id)?;
        ensure_no_live_receipts(app, id)?;
        if crate::managed_agents::load_managed_agents(app)?
            .iter()
            .any(|record| record.subscription_account_id.as_deref() == Some(id))
        {
            return Err("Account is selected by an agent; switch that agent before removal".into());
        }
        let root = store::root(app)?;
        let mut registry = store::load(&root)?;
        let account = store::account_mut(&mut registry, id)?;
        if account.attempt_id.is_some()
            || !attempts()
                .lock()
                .map_err(|_| "Account login state unavailable")?
                .is_empty()
        {
            return Err("Finish or cancel browser login before removal".into());
        }
        if account
            .attempt_started_at
            .is_some_and(|started| chrono::Utc::now().timestamp() < started.saturating_add(302))
        {
            return Err(
                "Interrupted login is still being contained; retry after its five minute deadline"
                    .into(),
            );
        }
        let harness = account.harness.clone();
        let logout_completed = account.logout_completed;
        let home = if logout_completed && !root.join(id).exists() {
            root.join(id)
        } else {
            store::home_at(&root, id)?
        };
        start_removal(account)?;
        store::save(&root, &registry)?;
        (root, home, harness, logout_completed)
    };
    let result = if logout_completed {
        Ok(())
    } else {
        crate::managed_agents::resolve_command(&harness)
            .ok_or("Install the provider CLI to complete logout".to_string())
            .and_then(|executable| {
                adapters::operation(
                    &executable,
                    &harness,
                    &home,
                    "logout",
                    &Arc::new(AtomicBool::new(false)),
                )
            })
    };
    let _guard = lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    finish_removal(&root, id, &home, result)
}

fn finish_removal(
    root: &std::path::Path,
    id: &str,
    home: &std::path::Path,
    result: Result<(), String>,
) -> Result<(), String> {
    let mut registry = store::load(root)?;
    store::account_mut(&mut registry, id)?.attempt_started_at = None;
    if let Err(error) = result {
        store::account_mut(&mut registry, id)?.last_error = Some(error.clone());
        store::save(root, &registry)?;
        return Err(error);
    }
    ensure_unused(id)?;
    store::account_mut(&mut registry, id)?.logout_completed = true;
    store::save(root, &registry)?;
    // Keep the tombstone until both logout and deletion are durable successes.
    match std::fs::symlink_metadata(home) {
        Ok(_) => {
            store::home_at(root, id)?;
            std::fs::remove_dir_all(home)
                .map_err(|_| "Logout succeeded but profile deletion failed; retry removal")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err("Could not inspect logged-out account directory; retry removal".into())
        }
    }
    registry.accounts.retain(|account| account.id != id);
    store::save(root, &registry)
}

fn start_removal(account: &mut SubscriptionAccount) -> Result<(), String> {
    if account
        .attempt_started_at
        .is_some_and(|started| chrono::Utc::now().timestamp() < started.saturating_add(302))
    {
        return Err(
            "Account cleanup is still running; retry after its five minute deadline".into(),
        );
    }
    account.state = "removal_pending".into();
    account.last_error = None;
    account.attempt_started_at = Some(chrono::Utc::now().timestamp());
    Ok(())
}

#[cfg(test)]
mod tests;
