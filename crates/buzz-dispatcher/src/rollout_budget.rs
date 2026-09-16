//! Durable reservations prevent restart or uncertain outcomes from resetting a rollout budget.
use crate::error::DispatchError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::Path;
use uuid::Uuid;
#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    approvals: BTreeMap<Uuid, BTreeSet<Uuid>>,
}
fn read(path: &Path) -> Result<Ledger, DispatchError> {
    if !path.exists() {
        return Ok(Ledger::default());
    }
    serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|error| DispatchError::Other(format!("rollout budget is unreadable: {error}")))
}
/// Reserve an action before publishing. Returning false leaves the action unstarted.
/// Callers hold the dispatcher's exclusive state-directory instance lock.
pub fn reserve(
    state_dir: &Path,
    approval: Uuid,
    request: Uuid,
    maximum: usize,
) -> Result<bool, DispatchError> {
    let path = state_dir.join("rollout_budget.json");
    let mut ledger = read(&path)?;
    let requests = ledger.approvals.entry(approval).or_default();
    if requests.contains(&request) {
        return Ok(true);
    }
    if requests.len() >= maximum {
        return Ok(false);
    }
    requests.insert(request);
    let temporary = state_dir.join("rollout_budget.json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(&ledger)?)?;
    file.sync_all()?;
    std::fs::rename(&temporary, &path)?;
    std::fs::File::open(state_dir)?.sync_all()?;
    Ok(true)
}
/// Requests previously reserved by this exact approval may resume with the same bytes.
pub fn requests(state_dir: &Path, approval: Uuid) -> Result<BTreeSet<Uuid>, DispatchError> {
    Ok(read(&state_dir.join("rollout_budget.json"))?
        .approvals
        .remove(&approval)
        .unwrap_or_default())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_survives_reopen_and_never_spends_twice_for_one_request() {
        let directory = tempfile::tempdir().unwrap();
        let approval = Uuid::new_v4();
        let request = Uuid::new_v4();
        assert!(reserve(directory.path(), approval, request, 1).unwrap());
        assert!(reserve(directory.path(), approval, request, 1).unwrap());
        assert!(!reserve(directory.path(), approval, Uuid::new_v4(), 1).unwrap());
        assert_eq!(requests(directory.path(), approval).unwrap().len(), 1);
        assert!(reserve(directory.path(), Uuid::new_v4(), Uuid::new_v4(), 1).unwrap());
    }
    #[test]
    fn corrupted_budget_cannot_reset_allowance() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("rollout_budget.json"), "broken").unwrap();
        assert!(reserve(directory.path(), Uuid::new_v4(), Uuid::new_v4(), 3).is_err());
    }
}
