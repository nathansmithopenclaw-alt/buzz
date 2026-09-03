//! Agent kind:0 profile reconciliation — split from `agents.rs` (file-size
//! guard). Owns the reconcile data carrier, the legacy-avatar backfill, the
//! avatar precedence rule, and the needs-sync predicate.

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::managed_agents::{is_managed_agent_default_avatar, managed_agent_avatar_url};

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
    /// Expected avatar URL for the published profile. `None` for legacy records
    /// that predate the `avatar_url` field — these will be backfilled from the
    /// relay's existing kind:0 profile on first reconciliation.
    pub(crate) avatar_url: Option<String>,
    pub(crate) auth_tag: Option<String>,
    /// The agent's pubkey (hex). Needed to update the persisted record during
    /// avatar backfill migration.
    pub(crate) pubkey: String,
    /// The agent's command (e.g. "goose"). Used as fallback when no profile
    /// exists on the relay during avatar backfill.
    pub(crate) agent_command: String,
    /// Persona ID if this agent was created from a persona. Used during avatar
    /// backfill to recover the correct avatar from the persona record when the
    /// relay profile has been corrupted.
    pub(crate) persona_id: Option<String>,
    /// Expected kind:0 `about` — the agent's effective public description
    /// (owner-authored when present; see
    /// `managed_agents::record_effective_description`). `None` publishes an
    /// about-less profile.
    pub(crate) about: Option<String>,
}

/// Resolve the avatar to backfill for a legacy agent record (pre-PR-921, no
/// stored `avatar_url`).
///
/// Priority: the persona's avatar wins, because the old reconciliation code
/// could have overwritten the relay's kind:0 `picture` with the command default
/// — making the relay an unreliable source for persona-backed agents. Only fall
/// back to the relay's `picture`, then the command icon, for agents with no
/// persona avatar to recover from.
pub(super) fn resolve_legacy_avatar(
    persona_avatar: Option<String>,
    relay_picture: Option<String>,
    agent_command: &str,
) -> String {
    persona_avatar
        .or(relay_picture)
        .or_else(|| managed_agent_avatar_url(agent_command))
        .unwrap_or_default()
}

/// Which picture wins when the record and the relay disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AvatarPrecedence {
    /// Keep the record's avatar as the expected picture, republishing it when
    /// the relay's copy diverges.
    KeepRecord,
    /// The record carries a runtime default while the relay already publishes
    /// a chosen picture — adopt the relay's picture into the record.
    AdoptRelay(String),
}

/// Resolve the kind:0 `picture` precedence for one reconciliation.
///
/// Precedence: an explicit picture on the record > a custom picture already
/// published on the relay > the runtime default icon.
///
/// A record whose `avatar_url` is a runtime default holds no owner intent — it
/// is whatever the harness table supplied when the record was written. When
/// the relay's kind:0 already carries a picture that is *not* a runtime
/// default, that picture is the more specific fact, so reconciliation adopts
/// it rather than republishing the default over it every launch. Every other
/// combination keeps the record: an explicit record picture is a deliberate
/// choice and wins outright; a relay with no profile (or no picture) has
/// nothing to adopt; and a relay picture that is itself a default says no more
/// than the record already does.
///
/// The relay picture is adopted verbatim rather than trimmed or otherwise
/// normalized, so the adopted value equals the published bytes and the next
/// reconciliation settles on `AlreadyCurrent` instead of republishing a
/// normalized variant over the very picture it just adopted.
pub(super) fn resolve_avatar_precedence(
    record_avatar: Option<&str>,
    relay_picture: Option<&str>,
) -> AvatarPrecedence {
    let Some(record_avatar) = record_avatar else {
        return AvatarPrecedence::KeepRecord;
    };
    if !is_managed_agent_default_avatar(record_avatar) {
        return AvatarPrecedence::KeepRecord;
    }
    match relay_picture {
        Some(picture)
            if !picture.trim().is_empty() && !is_managed_agent_default_avatar(picture) =>
        {
            AvatarPrecedence::AdoptRelay(picture.to_string())
        }
        _ => AvatarPrecedence::KeepRecord,
    }
}

/// Persist `avatar_url` onto the managed-agent record under the store lock.
/// Shared by the legacy backfill and relay adoption so both writes take the
/// same lock/load/save path. A record that has since disappeared is not an
/// error — there is simply nothing left to write.
fn persist_record_avatar(
    state: &AppState,
    app: &AppHandle,
    pubkey: &str,
    avatar_url: &str,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    if let Some(record) = records.iter_mut().find(|r| r.pubkey == pubkey) {
        record.avatar_url = Some(avatar_url.to_string());
        save_managed_agents(app, &records)?;
    }
    Ok(())
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
        pubkey: record.pubkey.clone(),
        agent_command: crate::managed_agents::record_agent_command(record, personas),
        persona_id: record.persona_id.clone(),
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
/// For legacy records (pre-PR-921) where `avatar_url` is `None`, this function
/// backfills via `resolve_legacy_avatar` — preferring the persona record's avatar
/// over the relay's `picture`, since the old code may have corrupted the relay
/// profile — and persists the updated record. After backfill, normal
/// reconciliation proceeds.
///
/// The resolved avatar then passes through `resolve_avatar_precedence`: a
/// record still holding a runtime default yields to a picture already chosen
/// on the relay, which is adopted into the record so it survives the next
/// launch. See that function for the full precedence rule.
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
    app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<ProfileReconcileOutcome, String> {
    use crate::relay::{query_agent_profile, sync_managed_agent_profile_from_snapshot};

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
    let existing = query_agent_profile(state, &relay_url, agent_pubkey).await?;

    // Resolve the expected avatar — backfilling for legacy records that have no
    // stored avatar_url yet.
    let expected_avatar = match data.avatar_url.as_deref() {
        Some(url) => url.to_string(),
        None => {
            // Legacy record: the relay profile may have been corrupted by the
            // old reconciliation code (it overwrote the persona avatar with the
            // command default), so the persona record is the authoritative source.
            let persona_avatar = data.persona_id.as_ref().and_then(|pid| {
                load_personas(app)
                    .ok()?
                    .into_iter()
                    .find(|p| p.id == *pid)?
                    .avatar_url
            });

            let backfilled = resolve_legacy_avatar(
                persona_avatar,
                existing.as_ref().and_then(|info| info.picture.clone()),
                &data.agent_command,
            );

            // Persist the backfilled avatar so this migration only runs once.
            if !backfilled.is_empty() {
                persist_record_avatar(state, app, &data.pubkey, &backfilled)?;
            }

            backfilled
        }
    };

    let expected_avatar = if expected_avatar.is_empty() {
        None
    } else {
        Some(expected_avatar)
    };

    // A record still holding the runtime default carries no owner intent, so a
    // picture already chosen on the relay outranks it. Adopt and persist that
    // picture instead of republishing the default over it on every launch.
    // Name and `about` reconciliation are unaffected: when either of those is
    // stale the republish below now carries the adopted picture forward.
    let expected_avatar = match resolve_avatar_precedence(
        expected_avatar.as_deref(),
        existing.as_ref().and_then(|info| info.picture.as_deref()),
    ) {
        AvatarPrecedence::KeepRecord => expected_avatar,
        AvatarPrecedence::AdoptRelay(picture) => {
            persist_record_avatar(state, app, &data.pubkey, &picture)?;
            Some(picture)
        }
    };

    if !profile_needs_sync(
        existing.as_ref(),
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
    ) {
        return Ok(ProfileReconcileOutcome::AlreadyCurrent);
    }

    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

    if !profile_reconcile_enabled(&state.managed_agent_profile_reconcile_enabled) {
        return Ok(ProfileReconcileOutcome::SkippedAgentManagedProfiles);
    }

    sync_managed_agent_profile_from_snapshot(
        state,
        &relay_url,
        &agent_keys,
        &data.name,
        expected_avatar.as_deref(),
        data.about.as_deref(),
        data.auth_tag.as_deref(),
        existing.as_ref(),
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
