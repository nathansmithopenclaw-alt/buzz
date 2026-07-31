use nostr::ToBech32;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, ManagedAgentRecord},
};

pub(crate) fn managed_agent_nsec_from_records(
    records: &[ManagedAgentRecord],
    requested_pubkey: &str,
) -> Result<String, String> {
    let pubkey = nostr::PublicKey::parse(requested_pubkey.trim())
        .map_err(|error| format!("invalid agent pubkey: {error}"))?;
    let pubkey_hex = pubkey.to_hex();
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey_hex))
        .ok_or_else(|| format!("managed agent not found: {pubkey_hex}"))?;
    if record.private_key_nsec.trim().is_empty() {
        return Err(format!("managed agent key unavailable: {pubkey_hex}"));
    }
    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("stored managed-agent key is invalid: {error}"))?;
    if keys.public_key() != pubkey {
        return Err("stored managed-agent key does not match the requested pubkey".to_string());
    }
    keys.secret_key()
        .to_bech32()
        .map_err(|error| format!("encode managed-agent nsec: {error}"))
}

/// Reveal a managed agent's persisted nsec to the local owner UI. This is a
/// direct Tauri request/response and must never be routed through relay events.
#[tauri::command]
pub async fn get_managed_agent_nsec(pubkey: String, app: AppHandle) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        managed_agent_nsec_from_records(&records, &pubkey)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}
