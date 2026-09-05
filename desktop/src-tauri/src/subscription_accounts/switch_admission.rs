//! Check exclusive account admission before a switch disturbs working runtimes.
use std::path::Path;

use tauri::AppHandle;

use super::{
    ensure_no_live_receipts_at, ensure_no_other_live_receipts_at, ensure_unused, leases,
    reservation, store, SubscriptionLease,
};

/// An owned admission snapshot keeps the check adjacent to switch effects while
/// allowing the tracked runtime map to be mutably borrowed by those effects.
pub(crate) struct SwitchAdmission {
    root: std::path::PathBuf,
    id: Option<String>,
    runtime_count: usize,
    owned_lease: Option<LeaseProof>,
}

struct LeaseProof {
    id: String,
    pid: Option<u32>,
    tracked_pid: u32,
    token: String,
    reservation: Option<reservation::ReservationHandle>,
}

impl SwitchAdmission {
    /// Caller retains the account lifecycle lock through teardown and persist;
    /// restart reacquires it while reserving the next runtime generation.
    pub(crate) fn execute<T>(self, apply: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        if let Some(id) = self.id.as_deref() {
            preflight_switch_at(
                &self.root,
                id,
                self.runtime_count,
                self.owned_lease.as_ref(),
            )?;
        }
        apply()
    }
}

/// Snapshot non-secret ownership proof before borrowing runtimes for teardown.
pub(crate) fn prepare_switch(
    app: &AppHandle,
    id: Option<&str>,
    runtime_count: usize,
    owned_lease: Option<(&SubscriptionLease, u32)>,
) -> Result<SwitchAdmission, String> {
    let id = match id {
        Some(id) if super::load_account(app, id)?.kind == "token" => None,
        other => other,
    };
    Ok(SwitchAdmission {
        // Ambient is the recovery path even if account storage is unavailable.
        root: id
            .map(|_| store::root(app))
            .transpose()?
            .unwrap_or_default(),
        id: id.map(str::to_owned),
        runtime_count,
        owned_lease: owned_lease.map(|(lease, tracked_pid)| LeaseProof {
            id: lease.id.clone(),
            pid: lease.pid,
            tracked_pid,
            token: lease.token.clone(),
            reservation: lease.reservation.clone(),
        }),
    })
}

fn preflight_switch_at(
    root: &Path,
    id: &str,
    runtime_count: usize,
    owned_lease: Option<&LeaseProof>,
) -> Result<(), String> {
    let data = root.parent().ok_or("Account root unavailable")?;
    let receipts = data.join("agents/agent-pids");
    if runtime_count > 1 {
        return Err("This agent is running in multiple communities. Subscription Accounts currently supports one running session per account; the current account is unchanged".into());
    }
    // Re-selecting the current account may release its own lease during this
    // switch. A matching account id alone is insufficient: both memory and
    // durable reservation must prove this exact generation and tracked PID.
    if let Some(lease) = owned_lease.filter(|lease| lease.id == id) {
        let pid = lease.tracked_pid;
        let owns_memory = leases()
            .lock()
            .map_err(|_| "Account lease state unavailable")?
            .get(id)
            .is_some_and(|entry| entry.token == lease.token && entry.pid == Some(pid));
        if owns_memory && lease.pid == Some(pid) {
            if let Some(reservation) = &lease.reservation {
                if reservation.owns_running_process(pid)? {
                    ensure_no_other_live_receipts_at(&receipts, id, Some(pid))
                        .map_err(switch_error)?;
                    return Ok(());
                }
            }
        }
    }
    // This is the same availability boundary as runtime acquisition. Never
    // clear a live/unknown reservation to make a switch appear successful.
    reservation::ensure_clear(root, id).map_err(switch_error)?;
    ensure_no_live_receipts_at(&receipts, id).map_err(switch_error)?;
    ensure_unused(id).map_err(switch_error)
}

fn switch_error(error: String) -> String {
    format!("Cannot switch subscription account: {error}. The current account and running agent are unchanged")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::subscription_accounts::LeaseEntry;

    fn switch<T>(
        root: &Path,
        id: &str,
        runtime_count: usize,
        owned_lease: Option<(&SubscriptionLease, u32)>,
        apply: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        SwitchAdmission {
            root: root.to_owned(),
            id: Some(id.to_owned()),
            runtime_count,
            owned_lease: owned_lease.map(|(lease, tracked_pid)| LeaseProof {
                id: lease.id.clone(),
                pid: lease.pid,
                tracked_pid,
                token: lease.token.clone(),
                reservation: lease.reservation.clone(),
            }),
        }
        .execute(apply)
    }

    #[test]
    fn live_destination_is_refused_before_switch_and_own_generation_can_restart() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("accounts");
        store::private_dir(&root, true).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let reservation = reservation::create(&root, &id, &token).unwrap();
        let mut lease = SubscriptionLease {
            id: id.clone(),
            pid: None,
            token: token.clone(),
            reservation: Some(reservation),
        };
        leases()
            .lock()
            .unwrap()
            .insert(id.clone(), LeaseEntry { token, pid: None });
        lease.attach_pid(child.id()).unwrap();
        let saved = std::fs::read(root.join(format!("{id}.lease.json"))).unwrap();
        let mut previous_runtime = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let selection_path = directory.path().join("selected-account.json");
        std::fs::write(&selection_path, "ambient").unwrap();
        // The real switch uses this same execution seam around quiesce,
        // teardown, selection persistence and spawn. A rejected account must
        // never invoke those effects, even if the destination still has a PID.
        let error = switch(&root, &id, 1, None, || {
            previous_runtime.kill().unwrap();
            previous_runtime.wait().unwrap();
            std::fs::write(&selection_path, &id).unwrap();
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.contains("current account and running agent are unchanged"),
            "{error}"
        );
        assert!(child.try_wait().unwrap().is_none());
        assert!(previous_runtime.try_wait().unwrap().is_none());
        assert_eq!(std::fs::read_to_string(selection_path).unwrap(), "ambient");
        previous_runtime.kill().unwrap();
        previous_runtime.wait().unwrap();
        assert_eq!(
            std::fs::read(root.join(format!("{id}.lease.json"))).unwrap(),
            saved
        );
        switch(&root, &id, 1, Some((&lease, child.id())), || Ok(())).unwrap();
        let receipts = directory.path().join("agents/agent-pids");
        std::fs::create_dir_all(&receipts).unwrap();
        let mut receipt = serde_json::json!({"key":{"pubkey":"a".repeat(64),"relayUrl":"wss://example.test"},"pid":child.id(),"desktopInstanceId":"test","startedAt":"2026-09-05T00:00:00Z","subscriptionAccountId":id,"subscriptionAccountGeneration":1});
        let receipt_path = receipts.join("receipt.json");
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        switch(&root, &id, 1, Some((&lease, child.id())), || Ok(())).unwrap();
        receipt["pid"] = serde_json::json!(std::process::id());
        std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(switch(&root, &id, 1, Some((&lease, child.id())), || Ok(())).is_err());
        std::fs::remove_file(receipt_path).unwrap();
        assert!(switch(&root, &id, 1, Some((&lease, std::process::id())), || Ok(())).is_err());
        assert!(switch(&root, &id, 2, Some((&lease, child.id())), || Ok(()))
            .unwrap_err()
            .contains("multiple communities"));
        child.kill().unwrap();
        child.wait().unwrap();
        drop(lease);
        switch(&root, &id, 1, None, || Ok(())).unwrap();
    }

    #[test]
    fn unknown_reservation_and_multiple_runtimes_refuse_admission() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("accounts");
        store::private_dir(&root, true).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        switch(&root, &id, 0, None, || Ok(())).unwrap();
        switch(&root, &id, 1, None, || Ok(())).unwrap();
        assert!(switch(&root, &id, 2, None, || Ok(())).is_err());
        let reservation =
            reservation::create(&root, &id, &uuid::Uuid::new_v4().to_string()).unwrap();
        assert!(switch(&root, &id, 1, None, || Ok(()))
            .unwrap_err()
            .contains("Operator verification"));
        reservation.release().unwrap();
        SwitchAdmission {
            root: directory.path().join("missing-account-root"),
            id: None,
            runtime_count: 2,
            owned_lease: None,
        }
        .execute(|| Ok(()))
        .unwrap();
    }
}
