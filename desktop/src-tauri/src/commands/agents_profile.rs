//! Agent kind:0 profile reconciliation — split from `agents.rs` (file-size
//! guard). Reconciles profile metadata while preserving durable picture edits.

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileReconcileOutcome {
    AlreadyCurrent,
    Published,
    SkippedAgentManagedProfiles,
}

fn profile_reconcile_enabled(flag: &std::sync::atomic::AtomicBool) -> bool {
    flag.load(std::sync::atomic::Ordering::Acquire)
}

pub(crate) struct ProfileReconcileData {
    pub(crate) private_key_nsec: String,
    pub(crate) name: String,
    pub(crate) relay_url: String,
    /// Exact relay pinned by the caller for the deferred task — captured
    /// while the authorizing workspace/spawn was active (UI start, boot
    /// restore, migration queue). When set it wins unconditionally; a task
    /// left unpinned (no tenant boundary) resolves the current workspace at
    /// execution time. See `resolve_reconcile_relay`.
    pub(crate) target_relay_url: Option<String>,
    /// Saved avatar source, localized to the target community before publishing.
    /// `None` for legacy records
    /// that predate the `avatar_url` field — these will be backfilled from the
    /// relay's existing kind:0 profile on first reconciliation.
    pub(crate) avatar_url: Option<String>,
    pub(crate) auth_tag: Option<String>,
    /// Expected kind:0 `about` — the agent's effective public description
    /// (owner-authored when present; see
    /// `managed_agents::record_effective_description`). `None` publishes an
    /// about-less profile.
    pub(crate) about: Option<String>,
}

/// Resolve the relay a reconciliation task will query and publish on. The
/// pure core of the `reconcile_agent_profile` relay choice, extracted so the
/// pinning contract is unit-testable: a caller-pinned `target_relay_url`
/// (captured while the authorizing workspace was active) wins UNCONDITIONALLY
/// over the execution-time workspace read — otherwise a community switch
/// landing between spawn and execution would retarget the kind:0
/// query/publish to a tenant the caller never authorized. Only an unpinned
/// task (no tenant boundary) resolves the live workspace.
pub(super) fn resolve_reconcile_relay(
    target_relay_url: Option<&str>,
    record_relay_url: &str,
    workspace_relay_at_execution: &str,
) -> String {
    match target_relay_url {
        Some(pinned) => pinned.to_string(),
        None => {
            crate::relay::effective_agent_relay_url(record_relay_url, workspace_relay_at_execution)
        }
    }
}

pub(crate) fn profile_reconcile_data(
    record: &crate::managed_agents::ManagedAgentRecord,
    personas: &[crate::managed_agents::AgentDefinition],
) -> ProfileReconcileData {
    ProfileReconcileData {
        private_key_nsec: record.private_key_nsec.clone(),
        name: record.name.clone(),
        relay_url: record.relay_url.clone(),
        target_relay_url: None,
        avatar_url: record.avatar_url.clone(),
        auth_tag: record.auth_tag.clone(),
        about: crate::managed_agents::record_effective_description(record, personas),
    }
}

pub(crate) fn load_pending_profile_reconciliations(
    app: &AppHandle,
    workspace_relay: &str,
) -> Result<Vec<(String, ProfileReconcileData)>, String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let store_path = crate::managed_agents::managed_agents_store_path(app)?;
    let queue_path = crate::migration::profile_reconcile_queue_path(&store_path);
    if !queue_path.exists() {
        return Ok(Vec::new());
    }

    let relay_key = crate::migration::profile_reconcile_relay_key(workspace_relay)?;
    let pending = crate::migration::read_profile_reconcile_queue(&queue_path)?;
    let records = crate::managed_agents::load_managed_agents(app)?;
    let personas = crate::managed_agents::load_personas(app).unwrap_or_default();
    Ok(records
        .iter()
        // A queue write deliberately precedes the migrated agent-store write.
        // If the process dies between them, retain (but do not execute) the
        // stale item until the next boot finishes renaming the record.
        .filter(|record| {
            pending.iter().any(|entry| {
                entry.pubkey == record.pubkey
                    && entry.expected_name == record.name
                    && !entry
                        .reconciled_relays
                        .iter()
                        .any(|relay| relay == &relay_key)
            })
        })
        .map(|record| {
            let mut data = profile_reconcile_data(record, &personas);
            // Pin the relay captured by the caller. Otherwise a fast community
            // switch could make a queued task for A run on B.
            data.target_relay_url = Some(workspace_relay.to_string());
            (record.pubkey.clone(), data)
        })
        .collect())
}

pub(crate) fn mark_profile_reconciled(
    app: &AppHandle,
    pubkey: &str,
    relay_url: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let store_path = crate::managed_agents::managed_agents_store_path(app)?;
    let queue_path = crate::migration::profile_reconcile_queue_path(&store_path);
    if !queue_path.exists() {
        return Ok(());
    }
    let relay_key = crate::migration::profile_reconcile_relay_key(relay_url)?;
    let mut pending = crate::migration::read_profile_reconcile_queue(&queue_path)?;
    crate::migration::record_profile_reconciled(&mut pending, pubkey, relay_key);
    crate::migration::write_profile_reconcile_queue(&queue_path, &pending)
}

/// Reconcile an agent's kind:0 profile on the relay.
///
/// Queries the relay for the agent's existing profile and re-publishes if missing
/// or stale (display_name or picture mismatch). This is fire-and-forget — errors
/// are returned to the caller for logging but never block agent startup.
///
/// Picture selection is independent of runtime defaults. The publisher re-reads
/// durable intent after asynchronous profile reads, initializes genuinely unset
/// records conservatively, and refuses unresolved legacy conflicts.
///
/// Query and publish target the caller-pinned `target_relay_url` when set
/// (UI start, boot restore, migration queue — captured while the authorizing
/// workspace was active); an unpinned task falls back to
/// `effective_agent_relay_url` against the workspace at execution time. This
/// keeps deferred reconciliation from following a community switch it was
/// never authorized for while honoring a deliberate per-agent pin wherever
/// it points.
pub(crate) async fn reconcile_agent_profile(
    state: &AppState,
    _app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<ProfileReconcileOutcome, String> {
    use crate::relay::query_agent_profile_with_keys;

    // Resolved ONCE and used for both the read and the write-back. A pinned
    // `target_relay_url` wins unconditionally — see `resolve_reconcile_relay`.
    let relay_url = resolve_reconcile_relay(
        data.target_relay_url.as_deref(),
        &data.relay_url,
        &relay_ws_url_with_override(state),
    );

    if !profile_reconcile_enabled(&state.managed_agent_profile_reconcile_enabled) {
        return Ok(ProfileReconcileOutcome::SkippedAgentManagedProfiles);
    }

    // Query the relay for the agent's existing kind:0 profile.
    let keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;
    let existing =
        query_agent_profile_with_keys(state, &relay_url, &keys, data.auth_tag.as_deref()).await?;

    // Resolve from the latest durable selection after the relay read. A stale
    // startup snapshot must never re-adopt a picture over an explicit removal.
    let selection =
        crate::managed_agents::avatar_publication::current_selection(state, agent_pubkey)?;
    let expected_avatar = selection
        .as_ref()
        .map(|s| s.url.clone().unwrap_or_default())
        .or_else(|| data.avatar_url.clone());

    reconcile_profile_at(
        state,
        &relay_url,
        data,
        expected_avatar.as_deref(),
        existing.as_ref(),
    )
    .await
}

/// Network half of startup/restore reconciliation, after legacy source backfill.
/// Kept separate from the disk migration so the production compare-and-publish
/// seam can be exercised against two communities without a GUI runtime.
pub(crate) async fn reconcile_profile_at(
    state: &AppState,
    relay_url: &str,
    data: &ProfileReconcileData,
    expected_avatar: Option<&str>,
    existing: Option<&crate::relay::AgentProfileInfo>,
) -> Result<ProfileReconcileOutcome, String> {
    if !state
        .managed_agent_profile_reconcile_enabled
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(ProfileReconcileOutcome::SkippedAgentManagedProfiles);
    }
    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;
    // Capture intent before localization awaits; only this edit may be acknowledged.
    let agent_pubkey = agent_keys.public_key().to_hex();
    let selection =
        crate::managed_agents::avatar_publication::current_selection(state, &agent_pubkey)?;
    let expected_avatar = selection
        .as_ref()
        .map(|selected| selected.url.as_deref().unwrap_or(""))
        .or(expected_avatar);
    let expected_avatar = crate::relay::profile_avatar::localize_avatar(
        state,
        relay_url,
        &agent_keys,
        expected_avatar,
        data.auth_tag.as_deref(),
    )
    .await?;

    if !profile_needs_sync(
        existing,
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
    ) {
        if let Some(selection) = &selection {
            crate::managed_agents::avatar_publication::mark_synced(
                state,
                relay_url,
                &agent_pubkey,
                selection,
            )?;
        }
        return Ok(ProfileReconcileOutcome::AlreadyCurrent);
    }

    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

    if !profile_reconcile_enabled(&state.managed_agent_profile_reconcile_enabled) {
        return Ok(ProfileReconcileOutcome::SkippedAgentManagedProfiles);
    }

    crate::relay::sync_managed_agent_profile_from_snapshot(
        state,
        relay_url,
        &agent_keys,
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
        data.auth_tag.as_deref(),
        existing,
    )
    .await?;
    Ok(ProfileReconcileOutcome::Published)
}

/// Publish a managed agent's complete kind:0 profile and return a best-effort
/// error for create/import callers to surface without rolling back creation.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_agent_profile_with_about(
    state: &AppState,
    record_relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    about: Option<&str>,
    auth_tag: Option<&str>,
) -> Option<String> {
    let relay_url = crate::relay::effective_agent_relay_url(
        record_relay_url,
        &relay_ws_url_with_override(state),
    );
    crate::relay::sync_managed_agent_profile(
        state,
        &relay_url,
        agent_keys,
        display_name,
        avatar_url,
        about,
        auth_tag,
    )
    .await
    .err()
}

pub(crate) async fn publish_persona_profile(
    state: &AppState,
    record_relay_url: &str,
    agent_keys: &nostr::Keys,
    display_name: &str,
    avatar_url: Option<&str>,
    persona: &crate::managed_agents::AgentDefinition,
    auth_tag: Option<&str>,
) -> Option<String> {
    let about = crate::managed_agents::effective_agent_description(persona.description.as_deref());
    publish_agent_profile_with_about(
        state,
        record_relay_url,
        agent_keys,
        display_name,
        avatar_url,
        about.as_deref(),
        auth_tag,
    )
    .await
}

/// Decide whether a published profile is missing or stale relative to the
/// expected name, avatar, and about. A missing profile always needs sync; a
/// present one is stale when the display name, picture, or about diverges.
/// For about, `None` and the empty string are treated as equal so an
/// about-less profile never triggers a pointless republish loop.
pub(super) fn profile_needs_sync(
    existing: Option<&crate::relay::AgentProfileInfo>,
    expected_name: &str,
    expected_avatar: Option<&str>,
    expected_about: Option<&str>,
) -> bool {
    match existing {
        None => true,
        Some(info) => {
            let name_matches = info.display_name.as_deref() == Some(expected_name);
            let picture_matches = info.picture.as_deref() == expected_avatar;
            let about_matches = info.about.as_deref().unwrap_or("") == expected_about.unwrap_or("");
            !name_matches || !picture_matches || !about_matches
        }
    }
}

#[cfg(test)]
mod tests {
    use super::profile_reconcile_enabled;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn reconciliation_guard_tracks_agent_managed_profiles_both_ways() {
        let reconcile_enabled = AtomicBool::new(true);
        assert!(profile_reconcile_enabled(&reconcile_enabled));

        reconcile_enabled.store(false, Ordering::Release);
        assert!(!profile_reconcile_enabled(&reconcile_enabled));

        reconcile_enabled.store(true, Ordering::Release);
        assert!(profile_reconcile_enabled(&reconcile_enabled));
    }
}

// Async so the blocking body (disk reads/writes + process termination) runs off
// the main UI thread via spawn_blocking. State is re-derived from the owned
// AppHandle inside the closure (`State<'_, _>` is borrowed, MutexGuard is !Send).
