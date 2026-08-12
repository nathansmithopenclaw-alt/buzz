//! The persona edit command surface: `update_persona` (best-effort enqueue),
//! the narrow delegated env apply, and the `update_persona_with` seam that
//! `update_persona_and_publish` reuses to await relay acceptance for the same
//! save.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_readiness, apply_persona_behavior, current_instance_id, effective_agent_command,
        find_managed_agent_mut, known_acp_runtime, load_global_agent_config, load_managed_agents,
        load_personas, managed_agent_avatar_url, managed_agent_runtime_keys, record_agent_command,
        resolve_effective_agent_env, save_managed_agents, save_personas,
        stop_managed_agent_process, sync_managed_agent_processes, try_regenerate_nest,
        AgentDefinition, AgentReadiness, BackendKind, LinkedInstanceBehaviorPatch,
        ManagedAgentRecord, RespondTo, UpdatePersonaRequest,
    },
    util::now_iso,
};

use super::{pending, retain_persona_pending, trim_optional, trim_required};

#[cfg(test)]
mod name_propagation_tests;

/// Return value of the `update_persona` command. Uses flatten so all
/// `AgentDefinition` fields appear at the top level of the JSON response —
/// backward-compatible with callers that already destructure a raw persona object.
#[derive(Debug, serde::Serialize)]
pub struct UpdatePersonaResult {
    #[serde(flatten)]
    persona: AgentDefinition,
}

/// Narrow payload for the standing-delegation `env_vars` capability.
///
/// The backend overlays these keys onto the latest durable persona instead of
/// accepting the frontend's full edit-form snapshot. That keeps an env-only
/// grant from overwriting unrelated owner edits made after the request was
/// received.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedPersonaEnvVarsPatch {
    pub id: String,
    pub env_vars: BTreeMap<String, String>,
}

/// Result of a delegated env apply.
///
/// `restarted_count` is for live setup-listener processes that were bounced;
/// `started_count` is for active linked instances that were already stopped.
/// A failed start is non-transactional: the env write remains durable and the
/// failure is persisted on the agent record for an honest receipt/UI state.
#[derive(Debug, Serialize)]
pub struct DelegatedPersonaEnvVarsResult {
    #[serde(flatten)]
    pub persona: AgentDefinition,
    pub restarted_count: u32,
    pub started_count: u32,
    pub failed_start_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadinessTransitionAction {
    Restart,
    Start,
}

fn readiness_transition_action(
    is_local: bool,
    is_active: bool,
    old_ready: bool,
    new_ready: bool,
    has_runtime: bool,
    every_runtime_is_setup_mode: bool,
) -> Option<ReadinessTransitionAction> {
    if !is_local || !is_active || old_ready || !new_ready {
        return None;
    }
    if has_runtime {
        every_runtime_is_setup_mode.then_some(ReadinessTransitionAction::Restart)
    } else {
        Some(ReadinessTransitionAction::Start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelegatedEnvStartOutcome {
    Restarted,
    Started,
    Failed,
    Skipped,
}

/// Propagate a persona definition's display_name rename to linked agent instances.
/// Only instances whose current `name` equals `old_display_name` are updated;
/// pool-named instances (e.g. "Birch", "Compass") keep their individualised name.
/// Updates both `record.name` (relay display name) and `record.display_name`.
/// Returns the pubkeys of the records that were renamed.
fn propagate_persona_name_rename(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    old_display_name: &str,
    new_display_name: &str,
) -> Vec<String> {
    let mut renamed = Vec::new();
    for record in records.iter_mut() {
        if record.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        if record.name != old_display_name {
            continue; // pool-named instance — keep its individualised name
        }
        record.name = new_display_name.to_string();
        record.display_name = Some(new_display_name.to_string());
        renamed.push(record.pubkey.clone());
    }
    renamed
}

/// Apply the behavior fields explicitly named by an existing-agent management
/// request to every instance currently linked to the target persona.
///
/// The definition's behavior group remains a mint-time default. Applying the
/// original narrow patch here avoids reconstructing instance intent from the
/// definition edit form, which would overwrite unrelated instance overrides.
fn apply_linked_instance_behavior_patch(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    patch: &LinkedInstanceBehaviorPatch,
) -> Result<Vec<String>, String> {
    if patch.respond_to.is_none()
        && patch.respond_to_allowlist.is_none()
        && patch.parallelism.is_none()
    {
        return Ok(Vec::new());
    }
    if let Some(parallelism) = patch.parallelism {
        if !(1..=32).contains(&parallelism) {
            return Err(format!(
                "parallelism {parallelism} is out of range (must be between 1 and 32)"
            ));
        }
    }
    let normalized_allowlist = patch
        .respond_to_allowlist
        .as_ref()
        .map(|allowlist| crate::managed_agents::validate_respond_to_allowlist(allowlist))
        .transpose()?;

    // Validate every linked record before mutating any of them. This matters
    // when the patch changes only the allowlist: instance modes can differ.
    for record in records
        .iter()
        .filter(|record| record.persona_id.as_deref() == Some(persona_id))
    {
        let mode = patch.respond_to.unwrap_or(record.respond_to);
        let allowlist = if mode != RespondTo::Allowlist {
            Vec::new()
        } else {
            normalized_allowlist
                .clone()
                .unwrap_or_else(|| record.respond_to_allowlist.clone())
        };
        if mode == RespondTo::Allowlist && allowlist.is_empty() {
            return Err(
                "respond-to mode 'allowlist' requires at least one pubkey in the allowlist"
                    .to_string(),
            );
        }
    }

    let mut changed = Vec::new();
    for record in records
        .iter_mut()
        .filter(|record| record.persona_id.as_deref() == Some(persona_id))
    {
        if let Some(mode) = patch.respond_to {
            record.respond_to = mode;
            if mode != RespondTo::Allowlist {
                record.respond_to_allowlist.clear();
            }
        }
        if let Some(allowlist) = &normalized_allowlist {
            record.respond_to_allowlist = if record.respond_to == RespondTo::Allowlist {
                allowlist.clone()
            } else {
                Vec::new()
            };
        }
        if let Some(parallelism) = patch.parallelism {
            record.parallelism = parallelism;
        }
        record.updated_at = now_iso();
        changed.push(record.pubkey.clone());
    }
    Ok(changed)
}

/// Profile sync params collected under the store lock for async relay publish.
type ProfileSyncParams = Vec<(nostr::Keys, String, String, Option<String>, Option<String>)>;

#[tauri::command]
pub async fn update_persona(
    input: UpdatePersonaRequest,
    app: AppHandle,
) -> Result<UpdatePersonaResult, String> {
    let (persona, ()) = update_persona_with(input, app, |app, state, persona| {
        retain_persona_pending(app, state, persona);
        Ok(())
    })
    .await?;
    Ok(UpdatePersonaResult { persona })
}

/// Overlay an owner-granted env patch onto a persona and immediately activate
/// linked local agents whose readiness changes from `NotReady` to `Ready`.
///
/// This is intentionally separate from `update_persona`: only the delegated
/// `env_vars` route calls it, and its typed payload cannot mutate instructions,
/// model, access policy, or any other persona field. The lifecycle work is a
/// Desktop-owned consequence of the saved readiness transition, not a lifecycle
/// operation controlled by the grantee.
#[tauri::command]
pub async fn apply_delegated_persona_env_vars(
    patch: DelegatedPersonaEnvVarsPatch,
    app: AppHandle,
) -> Result<DelegatedPersonaEnvVarsResult, String> {
    let owner_hex = super::super::agents::workspace_owner_hex(&app.state::<AppState>())?;
    let app_for_write = app.clone();
    let (persona, old_personas, candidates) = tokio::task::spawn_blocking(move || {
        let state = app_for_write.state::<AppState>();
        crate::managed_agents::validate_user_env_keys(&patch.env_vars)?;

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut personas = load_personas(&app_for_write)?;
        pending::project_active_persona_sharing(&app_for_write, &state, &mut personas);
        let old_personas = personas.clone();
        let persona = personas
            .iter_mut()
            .find(|record| record.id == patch.id)
            .ok_or_else(|| format!("agent {} not found", patch.id))?;
        persona.env_vars.extend(patch.env_vars);
        persona.updated_at = now_iso();
        let saved_persona = persona.clone();

        // Complete every fallible activation precondition before the persona
        // write. Once the env patch is durable, this phase cannot return a
        // failure that would incorrectly record the request as unapplied.
        let global = load_global_agent_config(&app_for_write)?;
        let mut records = load_managed_agents(&app_for_write)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &current_instance_id(&app_for_write),
        );
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }
        if sync_changed {
            save_managed_agents(&app_for_write, &records)?;
        }

        let candidates = records
            .iter()
            .filter(|record| record.persona_id.as_deref() == Some(saved_persona.id.as_str()))
            .filter_map(|record| {
                let old_ready = agent_is_ready(record, &old_personas, &global);
                let new_ready = agent_is_ready(record, &personas, &global);
                let runtime_keys = managed_agent_runtime_keys(&runtimes, &record.pubkey);
                let has_runtime = !runtime_keys.is_empty();
                let every_runtime_is_setup_mode = has_runtime
                    && runtime_keys
                        .iter()
                        .all(|key| runtimes.get(key).is_some_and(|runtime| runtime.setup_mode));
                readiness_transition_action(
                    record.backend == BackendKind::Local,
                    record.is_active,
                    old_ready,
                    new_ready,
                    has_runtime,
                    every_runtime_is_setup_mode,
                )
                .map(|_| record.pubkey.clone())
            })
            .collect::<Vec<_>>();

        save_personas(&app_for_write, &personas)?;
        retain_persona_pending(&app_for_write, &state, &saved_persona);
        drop(runtimes);
        try_regenerate_nest(&app_for_write);
        Ok::<_, String>((saved_persona, old_personas, candidates))
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))??;

    let mut restarted_count = 0;
    let mut started_count = 0;
    let mut failed_start_count = 0;
    for pubkey in candidates {
        match activate_after_delegated_env(&app, &pubkey, &old_personas, &owner_hex).await {
            DelegatedEnvStartOutcome::Restarted => restarted_count += 1,
            DelegatedEnvStartOutcome::Started => started_count += 1,
            DelegatedEnvStartOutcome::Failed => failed_start_count += 1,
            DelegatedEnvStartOutcome::Skipped => {}
        }
    }

    Ok(DelegatedPersonaEnvVarsResult {
        persona,
        restarted_count,
        started_count,
        failed_start_count,
    })
}

fn agent_is_ready(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> bool {
    let command = record_agent_command(record, personas);
    let effective =
        resolve_effective_agent_env(record, personas, known_acp_runtime(&command), global);
    matches!(agent_readiness(&effective), AgentReadiness::Ready)
}

async fn activate_after_delegated_env(
    app: &AppHandle,
    pubkey: &str,
    old_personas: &[AgentDefinition],
    owner_hex: &str,
) -> DelegatedEnvStartOutcome {
    let app_for_transition = app.clone();
    let pubkey_owned = pubkey.to_string();
    let old_personas_owned = old_personas.to_vec();
    let transition = tokio::task::spawn_blocking(move || {
        let state = app_for_transition.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| format!("failed to acquire store lock: {error}"))?;
        let mut records = load_managed_agents(&app_for_transition)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| format!("failed to acquire runtimes lock: {error}"))?;
        let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &current_instance_id(&app_for_transition),
        );
        for exited_pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(exited_pubkey);
        }
        if sync_changed {
            save_managed_agents(&app_for_transition, &records)?;
        }

        let current_personas = load_personas(&app_for_transition).unwrap_or_default();
        let global = load_global_agent_config(&app_for_transition).unwrap_or_default();
        let record = records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey_owned))
            .ok_or_else(|| format!("agent {pubkey_owned} not found"))?;
        let old_ready = agent_is_ready(record, &old_personas_owned, &global);
        let new_ready = agent_is_ready(record, &current_personas, &global);
        let runtime_keys = managed_agent_runtime_keys(&runtimes, &pubkey_owned);
        let has_runtime = !runtime_keys.is_empty();
        let every_runtime_is_setup_mode = has_runtime
            && runtime_keys
                .iter()
                .all(|key| runtimes.get(key).is_some_and(|runtime| runtime.setup_mode));
        let action = readiness_transition_action(
            record.backend == BackendKind::Local,
            record.is_active,
            old_ready,
            new_ready,
            has_runtime,
            every_runtime_is_setup_mode,
        )
        .ok_or_else(|| {
            format!("agent {pubkey_owned} readiness transition is no longer restart-eligible")
        })?;

        let relay_urls = if action == ReadinessTransitionAction::Restart {
            let relay_urls = runtime_keys
                .iter()
                .map(|key| key.relay_url.clone())
                .collect::<Vec<_>>();
            let record = find_managed_agent_mut(&mut records, &pubkey_owned)?;
            stop_managed_agent_process(&app_for_transition, record, &mut runtimes)?;
            save_managed_agents(&app_for_transition, &records)?;
            relay_urls
        } else {
            Vec::new()
        };

        Ok::<_, String>((action, relay_urls))
    })
    .await;

    let (action, relay_urls) = match transition {
        Ok(Ok(transition)) => transition,
        Ok(Err(error)) => {
            eprintln!(
                "buzz-desktop: delegated persona env: skipping activation of {pubkey}: {error}"
            );
            return DelegatedEnvStartOutcome::Skipped;
        }
        Err(error) => {
            eprintln!(
                "buzz-desktop: delegated persona env: activation task failed for {pubkey}: {error}"
            );
            return DelegatedEnvStartOutcome::Skipped;
        }
    };

    let state = app.state::<AppState>();
    let start_result = match action {
        ReadinessTransitionAction::Restart => {
            super::super::agents::start_local_agent_pairs_with_preflight(
                app,
                &state,
                pubkey,
                &relay_urls,
            )
            .await
        }
        ReadinessTransitionAction::Start => {
            super::super::agents::start_local_agent_with_preflight(
                app, &state, pubkey, owner_hex, false,
            )
            .await
        }
    };

    match start_result {
        Ok(_) if action == ReadinessTransitionAction::Restart => {
            eprintln!("buzz-desktop: delegated persona env: restarted setup-mode agent {pubkey}");
            DelegatedEnvStartOutcome::Restarted
        }
        Ok(_) => {
            eprintln!("buzz-desktop: delegated persona env: started active agent {pubkey}");
            DelegatedEnvStartOutcome::Started
        }
        Err(error) => {
            eprintln!("buzz-desktop: delegated persona env: failed to activate {pubkey}: {error}");
            if let Err(save_error) = persist_delegated_env_start_error(app, pubkey, &error) {
                eprintln!(
                    "buzz-desktop: delegated persona env: failed to persist last_error for {pubkey}: {save_error}"
                );
            }
            DelegatedEnvStartOutcome::Failed
        }
    }
}

fn persist_delegated_env_start_error(
    app: &AppHandle,
    pubkey: &str,
    error: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|lock_error| format!("failed to acquire store lock: {lock_error}"))?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = now_iso();
    save_managed_agents(app, &records)
}

/// Save an edited persona, hand the saved record to `retain` while the store
/// lock is still held, then sync the relay profiles of linked agent instances.
///
/// `retain` is the only difference between the two update commands:
/// [`update_persona`] enqueues best-effort, while
/// [`sharing::update_persona_and_publish`] prepares a strict publication and
/// returns the event so the caller can await relay acceptance.
pub(super) async fn update_persona_with<R: Send + 'static>(
    input: UpdatePersonaRequest,
    app: AppHandle,
    retain: impl FnOnce(&AppHandle, &AppState, &AgentDefinition) -> Result<R, String> + Send + 'static,
) -> Result<(AgentDefinition, R), String> {
    use tauri::Manager;

    // Phase 1: synchronous save (persona record + linked agent avatar updates)
    let (result, retained, profile_sync_params) = tokio::task::spawn_blocking({
        let app = app.clone();
        move || -> Result<(AgentDefinition, R, ProfileSyncParams), String> {
            let state = app.state::<AppState>();
            let display_name = trim_required(&input.display_name, "Display name")?;
            let system_prompt = input.system_prompt.clone();
            let avatar_url = trim_optional(input.avatar_url);
            let runtime = trim_optional(input.runtime);
            let model = trim_optional(input.model);
            let provider = trim_optional(input.provider);
            let linked_instance_behavior_patch = input.linked_instance_behavior_patch;

            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut personas = load_personas(&app)?;
            pending::project_active_persona_sharing(&app, &state, &mut personas);
            let persona = personas
                .iter_mut()
                .find(|record| record.id == input.id)
                .ok_or_else(|| format!("agent {} not found", input.id))?;

            // Track what changed so we can propagate to linked agent records.
            let avatar_changed = persona.avatar_url != avatar_url;
            let name_changed = persona.display_name != display_name;
            let old_display_name = persona.display_name.clone();

            persona.display_name = display_name;
            persona.avatar_url = avatar_url;
            persona.system_prompt = system_prompt;
            persona.runtime = runtime;
            persona.model = model;
            persona.provider = provider;
            persona.name_pool = input
                .name_pool
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(env_vars) = input.env_vars {
                crate::managed_agents::validate_user_env_keys(&env_vars)?;
                persona.env_vars = env_vars;
            }
            apply_persona_behavior(persona, input.behavior)?;
            persona.updated_at = now_iso();

            let result = persona.clone();
            // Validate the linked-record half before the first durable write.
            // The store lock makes the later application deterministic, so an
            // invalid instance patch cannot save the definition and then
            // report the whole request as failed.
            if let Some(patch) = &linked_instance_behavior_patch {
                let mut preview_records = load_managed_agents(&app)?;
                apply_linked_instance_behavior_patch(&mut preview_records, &result.id, patch)?;
            }
            save_personas(&app, &personas)?;

            let retained = retain(&app, &state, &result)?;
            try_regenerate_nest(&app);

            // Propagate identity fields and any explicit existing-agent
            // behavior patch to linked records, then collect relay profile sync
            // params for the async identity phase.
            let sync_params: ProfileSyncParams = if avatar_changed
                || name_changed
                || linked_instance_behavior_patch.is_some()
            {
                let mut records = load_managed_agents(&app)?;
                let mut params: ProfileSyncParams = Vec::new();
                let mut agents_modified = false;
                let workspace_relay = crate::relay::relay_ws_url_with_override(&state);

                // Propagate the display_name rename to instances that still
                // carry the old definition display_name (pool-named instances
                // keep their individualised name) in one pass; the loop below
                // only decides which records need a relay profile sync.
                let renamed: Vec<String> = if name_changed {
                    propagate_persona_name_rename(
                        &mut records,
                        &result.id,
                        &old_display_name,
                        &result.display_name,
                    )
                } else {
                    Vec::new()
                };
                let behavior_changed = linked_instance_behavior_patch
                    .as_ref()
                    .map(|patch| {
                        apply_linked_instance_behavior_patch(&mut records, &result.id, patch)
                    })
                    .transpose()?
                    .unwrap_or_default();

                for record in records.iter_mut() {
                    if record.persona_id.as_deref() != Some(&result.id) {
                        continue;
                    }
                    let mut identity_changed = renamed.contains(&record.pubkey);
                    let record_changed = behavior_changed.contains(&record.pubkey);

                    if avatar_changed {
                        // Update the persisted avatar so reconciliation on next
                        // start agrees with what we're about to publish.
                        // When the persona avatar is cleared, fall back to the
                        // command-default icon so the record never stores `None`
                        // (which reconcile_agent_profile treats as "un-migrated").
                        let effective_cmd = effective_agent_command(
                            record.persona_id.as_deref(),
                            std::slice::from_ref(&result),
                            record.agent_command_override.as_deref(),
                        );
                        record.avatar_url = result
                            .avatar_url
                            .clone()
                            .or_else(|| managed_agent_avatar_url(&effective_cmd));
                        identity_changed = true;
                    }

                    if record_changed || identity_changed {
                        agents_modified = true;
                    }
                    if identity_changed {
                        if let Ok(agent_keys) = nostr::Keys::parse(&record.private_key_nsec) {
                            let relay_url = crate::relay::effective_agent_relay_url(
                                &record.relay_url,
                                &workspace_relay,
                            );
                            params.push((
                                agent_keys,
                                relay_url,
                                record.name.clone(),
                                record.avatar_url.clone(),
                                record.auth_tag.clone(),
                            ));
                        }
                    }
                }

                if agents_modified {
                    save_managed_agents(&app, &records)?;
                    // Keep retained kind:30177 identity records in lockstep with
                    // the rename (#2423): `record.name` is part of the published
                    // identity projection, so skipping this strands the relay on
                    // the stale name→pubkey binding until the next boot reconcile.
                    // Avatar-only edits are excluded — the avatar is not in the
                    // projection, so retaining would be a guaranteed no-op.
                    for record in records.iter().filter(|r| renamed.contains(&r.pubkey)) {
                        crate::commands::agents::retain_managed_agent_pending(&app, &state, record);
                    }
                    // Access policy is part of the kind:30177 instance
                    // projection. Retain the changed instances immediately so
                    // relay observers and the next spawn read the same policy.
                    for record in records
                        .iter()
                        .filter(|r| behavior_changed.contains(&r.pubkey))
                    {
                        crate::commands::agents::retain_managed_agent_pending(&app, &state, record);
                    }
                }

                params
            } else {
                Vec::new()
            };

            Ok((result, retained, sync_params))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Phase 2: await relay profile sync for linked agents whose avatar or
    // display_name was just updated. We await (rather than fire-and-forget)
    // so the frontend cache invalidation that follows the mutation settlement
    // sees the fresh relay profile. Best-effort — failures are logged, not surfaced.
    if !profile_sync_params.is_empty() {
        let state = app.state::<AppState>();
        for (agent_keys, relay_url, display_name, avatar_url, auth_tag) in profile_sync_params {
            if let Err(e) = crate::relay::sync_managed_agent_profile(
                &state,
                &relay_url,
                &agent_keys,
                &display_name,
                avatar_url.as_deref(),
                auth_tag.as_deref(),
            )
            .await
            {
                eprintln!("buzz-desktop: relay profile sync failed after persona update: {e}");
            }
        }
    }

    Ok((result, retained))
}
