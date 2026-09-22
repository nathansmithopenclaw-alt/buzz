use std::{collections::HashMap, sync::atomic::Ordering};

use tauri::{AppHandle, Manager};

use super::agent_env::idle_pool_sleep_env;

use crate::{
    managed_agents::{
        append_log_marker, known_acp_runtime, login_shell_path, managed_agent_continuity_dir,
        managed_agent_log_path, missing_command_message, normalize_agent_args, open_log_file,
        resolve_command, spawn_key_refusal, KnownAcpRuntime, ManagedAgentPairRuntime,
        ManagedAgentRecord, ManagedAgentRuntimeKey, ManagedAgentSummary,
    },
    util::now_iso,
};

use super::claude_config::apply_claude_model_env;
mod path;
pub(in crate::managed_agents) use path::build_augmented_path;
pub(crate) use path::{compose_path_entries, should_skip_claude_executable, should_use_inherited};

pub(crate) use super::access_policy::{build_respond_to_env_with_policy, RespondToEnv};

mod metadata;
pub(crate) use metadata::{
    apply_agent_display_env, apply_replay_floor_env, persona_drift_state, resolve_session_title,
    runtime_metadata_env_vars, DISPLAY_NAME_ENV_VAR, REPLAY_FLOOR_ENV_VAR, SESSION_TITLE_ENV_VAR,
};

mod child_env;
#[cfg(test)]
use child_env::child_rust_log_filter_from;
use child_env::{apply_thinking_effort_provenance, child_rust_log_filter};

mod setup_payload;
use setup_payload::apply_setup_payload_env;
mod stop;
pub(crate) use stop::managed_agent_runtime_keys;
pub use stop::{
    stop_managed_agent_process, stop_managed_agent_processes, stop_managed_agent_workspace_pair,
};

mod sweep;
pub(crate) use sweep::sweep_untracked_bundle_harnesses;

mod process;
#[cfg(test)]
use process::{
    buzz_marker_entry, name_matches_interpreter, name_matches_known_binary,
    terminate_runtime_receipt_with, valid_agent_runtime_receipt_with,
};
pub(crate) use process::{
    current_instance_id, process_belongs_to_us, process_has_buzz_marker, process_is_running,
    terminate_process, terminate_processes_with_shared_grace, terminate_untracked_pair_runtime,
    valid_agent_runtime_receipt,
};

mod orphan_sweep;
#[cfg(target_os = "macos")]
use orphan_sweep::proc_pidinfo;
pub(crate) use orphan_sweep::{
    sweep_orphaned_agent_processes, sweep_system_agent_processes,
    sweep_system_agent_processes_with_grace,
};
#[cfg(target_os = "macos")]
use orphan_sweep::{BSDInfo, PROC_PIDTBSDINFO};
#[cfg(unix)]
use process::resolve_pgids_and_kill;

mod instance_reaper;
#[cfg(test)]
use instance_reaper::{buffer_contains_identifier, is_desktop_binary};
pub(crate) use instance_reaper::{emit_dead_instance_reap_reports, reap_dead_instance_agents};

mod lifecycle;
#[cfg(test)]
use lifecycle::kill_stale_tracked_processes_with;
pub use lifecycle::{kill_stale_tracked_processes, sync_managed_agent_processes};
mod spawn_key; // production spawn-key derivation + its regressions
pub(crate) use spawn_key::bound_runtime_key;

/// Resolve the runtime-pair key this record maps to for the active
/// workspace: always the active workspace relay (the legacy per-record relay
/// pin is ignored — see `effective_agent_relay_url`). Returns `None` for
/// records that cannot form a valid pair key yet (e.g. key-less agents that
/// mint keys on first start).
pub(crate) fn workspace_pair_key(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Option<ManagedAgentRuntimeKey> {
    let state = app.state::<crate::app_state::AppState>();
    resolve_workspace_pair_key(
        &record.pubkey,
        &record.relay_url,
        &crate::relay::relay_ws_url_with_override(&state),
    )
}

/// Pure core of [`workspace_pair_key`]: workspace-relay resolution (legacy
/// record pins ignored) plus canonical key construction, kept `AppHandle`-free
/// so summary/stop scoping semantics are unit-testable.
pub(crate) fn resolve_workspace_pair_key(
    pubkey: &str,
    record_relay_url: &str,
    workspace_relay_url: &str,
) -> Option<ManagedAgentRuntimeKey> {
    let effective_relay =
        crate::relay::effective_agent_relay_url(record_relay_url, workspace_relay_url);
    ManagedAgentRuntimeKey::new(pubkey.to_string(), &effective_relay).ok()
}

pub fn build_managed_agent_summary(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    teams: &[crate::managed_agents::TeamRecord],
    global_config: &crate::managed_agents::GlobalAgentConfig,
) -> Result<ManagedAgentSummary, String> {
    use crate::managed_agents::BackendKind;

    // Community-scoped truth: this summary describes the pair for the active
    // workspace relay. An agent running only in another community must read
    // as stopped here — matching by pubkey alone would show every community a
    // green light as long as any pair anywhere is alive.
    let pair_key = workspace_pair_key(app, record);
    let pair_runtime = pair_key.as_ref().and_then(|key| runtimes.get(key));

    let (status, pid, log_path) = if record.backend != BackendKind::Local {
        // Two-axis status model for remote agents:
        //
        //   Control-plane (this field): "deployed" = provider has been invoked and
        //   returned a backend_agent_id. "not_deployed" = no deploy call yet (or it
        //   failed). This axis tracks whether infrastructure *exists*, not whether
        //   the process is currently running.
        //
        //   Live axis (relay presence, polled by frontend): online/away/offline.
        //   Shown as a PresenceDot next to the agent name. This is the real-time
        //   signal for whether the harness is connected.
        //
        // After !shutdown the agent goes offline (presence) but stays "deployed"
        // (infrastructure still exists). This is intentional — the provider may
        // have allocated a VM/container that persists across process restarts.
        // A future provider `undeploy` operation (v2) will handle teardown.
        let status = if record.backend_agent_id.is_some() {
            "deployed".to_string()
        } else {
            "not_deployed".to_string()
        };
        (status, None, String::new())
    } else {
        let persisted_pid = record.runtime_pid.filter(|pid| process_is_running(*pid));
        if let Some(runtime) = pair_runtime {
            (
                "running".to_string(),
                Some(runtime.child.id()),
                runtime.log_path.display().to_string(),
            )
        } else if let Some(pid) = persisted_pid {
            (
                "running".to_string(),
                Some(pid),
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
            )
        } else {
            (
                "stopped".to_string(),
                None,
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
            )
        }
    };

    let (persona_out_of_date, persona_orphaned) = persona_drift_state(record, personas);

    let effective_cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record,
        personas,
        global_config,
    );
    let (effective_model, effective_provider, effective_prompt, model_source) = match effective_cfg
    {
        crate::managed_agents::effective_config::EffectiveConfigResult::Resolved(cfg) => {
            let source = cfg.model.source.clone();
            (
                cfg.model.value,
                cfg.provider.value,
                cfg.system_prompt.value,
                Some(source),
            )
        }
        crate::managed_agents::effective_config::EffectiveConfigResult::OrphanedInstance {
            record_pubkey,
            missing_persona_id,
        } => {
            eprintln!(
                "orphaned agent instance: pubkey={record_pubkey}, missing_persona_id={missing_persona_id}"
            );
            (None, None, None, None)
        }
    };

    // Restart badge: the running process stamped its effective spawn config;
    // recompute a prospective one from current disk state and report every
    // differing field. Only the tracked live pair for THIS workspace can drift
    // (stopped agents spawn fresh; adopted processes have no stamp; other-
    // community pairs are judged in their own community). Adapter drift
    // (codex only) contributes a synthetic entry for out-of-band npm changes.
    // Global config drives both snapshot and descriptor env layering; the
    // caller loads it once so list callers pay one disk read per call.

    // The prospective side is computed only for a tracked pair: an unstamped
    // agent has nothing to compare against.
    let tracked_spawn = pair_key.as_ref().zip(pair_runtime).map(|(key, runtime)| {
        let current = crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            crate::managed_agents::spawn_snapshot::ProspectiveSpawnConfigInputs {
                record,
                personas,
                teams,
                workspace_relay: &key.relay_url,
                global: global_config,
                enforced_owner_only: super::owner_only_access_build(),

                session_residency_enabled: app
                    .state::<crate::app_state::AppState>()
                    .session_residency_enabled()
                    .load(Ordering::Acquire),
            },
        );
        (runtime, current)
    });
    let restart_diff = crate::managed_agents::spawn_snapshot::eligible_restart_diff(
        persona_orphaned,
        tracked_spawn.as_ref().map(|(runtime, current)| {
            crate::managed_agents::spawn_snapshot::TrackedSpawnState {
                stamped: &runtime.spawn_config,
                current,
                stamped_availability: runtime.adapter_availability.as_ref(),
                current_availability: super::adapter_availability_cached(),
            }
        }),
    );
    // One vector is the whole truth: badge on ⟺ there is a diff to show.
    let needs_restart = !restart_diff.is_empty();

    // Resolve the effective harness via the single typed descriptor — same resolver
    // as spawn, so the UI reflects the persona's current harness (or explicit pin).
    let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
        record,
        personas,
        global_config,
    )
    .unwrap_or_else(|e| {
        // Dangling harness — surface the missing id so the UI tells the same
        // story as spawn (which refuses with a sentence), rather than silently
        // showing the default-command fallback as if the agent were healthy.
        let cmd = match crate::managed_agents::dangling_harness_id(&e) {
            Some(id) => crate::managed_agents::dangling_harness_display(id),
            None => crate::managed_agents::record_agent_command(record, personas),
        };
        let args = normalize_agent_args(&cmd, record.agent_args.clone());
        crate::managed_agents::readiness::EffectiveHarnessDescriptor {
            command: cmd,
            args,
            env: Default::default(),
        }
    });
    let effective_mcp_command = known_acp_runtime(&descriptor.command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("")
        .to_string();
    let claude =
        super::runtime_claude_account_summary(record, personas, global_config, pair_runtime);
    let supports_claude_accounts =
        super::record_supports_claude_accounts(record, &descriptor.command);

    let subscription_account_harness = super::subscription_binding::account_harness(
        known_acp_runtime(&descriptor.command).map(|r| r.id),
    )
    .map(str::to_owned);
    Ok(ManagedAgentSummary {
        external_execution: pair_key
            .as_ref()
            .map(|key| super::external_execution::load(app, key))
            .transpose()?
            .flatten(),
        effort_level: record.effort_level.clone(),
        pubkey: record.pubkey.clone(),
        name: record.name.clone(),
        persona_id: record.persona_id.clone(),
        runtime: record.runtime.clone(),
        team_id: record.team_id.clone(),
        relay_url: record.relay_url.clone(),
        acp_command: record.acp_command.clone(),
        agent_command: descriptor.command,
        agent_command_override: record.agent_command_override.clone(),
        agent_args: descriptor.args,
        mcp_command: effective_mcp_command,
        turn_timeout_seconds: record.turn_timeout_seconds,
        idle_timeout_seconds: record.idle_timeout_seconds,
        max_turn_duration_seconds: record.max_turn_duration_seconds,
        idle_pool_sleep_seconds: record.idle_pool_sleep_seconds,
        max_turns_per_session: record.max_turns_per_session,
        parallelism: record.parallelism,
        session_policy: super::effective_acp_session_policy(record, personas),
        system_prompt: effective_prompt,
        avatar_url: record.avatar_url.clone(),
        avatar_sync_pending: record.avatar_selection.as_ref().is_some_and(|selection| {
            let relay = crate::relay::effective_agent_relay_url(
                &record.relay_url,
                &crate::relay::relay_ws_url_with_override(
                    app.state::<crate::app_state::AppState>().inner(),
                ),
            );
            record
                .avatar_synced
                .get(&crate::relay::relay_http_base_url(&relay))
                != Some(&selection.edit_id)
        }),
        model: effective_model,
        model_source,
        provider: effective_provider,
        claude_account_id: claude.account_id,
        claude_account_source: claude.source,
        claude_account_shadowed: claude.shadowed,
        supports_claude_accounts,
        subscription_account_id: record.subscription_account_id.clone(),
        subscription_account_generation: record.subscription_account_generation,
        subscription_account_inherit_default: record.subscription_account_inherit_default,
        subscription_account_harness,
        running_subscription_account_id: super::subscription_binding::ready_binding(pair_runtime)
            .map(|b| b.account_id.clone()),
        running_subscription_account_generation: super::subscription_binding::ready_binding(
            pair_runtime,
        )
        .map(|b| b.generation),
        subscription_account_error: pair_runtime
            .and_then(|r| r.error.clone())
            .or_else(|| record.last_error.clone()),
        persona_out_of_date,
        persona_orphaned,
        needs_restart,
        restart_diff,
        env_vars: record.env_vars.clone(),
        backend: record.backend.clone(),
        backend_agent_id: record.backend_agent_id.clone(),
        status,
        pid,
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        last_started_at: record.last_started_at.clone(),
        last_stopped_at: record.last_stopped_at.clone(),
        last_exit_code: record.last_exit_code,
        last_error: record.last_error.clone(),
        last_error_code: record.last_error_code,
        start_on_app_launch: record.start_on_app_launch,
        auto_restart_on_config_change: record.auto_restart_on_config_change,
        log_path,
        respond_to: record.respond_to,
        respond_to_allowlist: record.respond_to_allowlist.clone(),
    })
}

pub fn find_managed_agent_mut<'a>(
    records: &'a mut [ManagedAgentRecord],
    pubkey: &str,
) -> Result<&'a mut ManagedAgentRecord, String> {
    records
        .iter_mut()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))
}

/// Pure decision function for the inbound author gate env vars.
///
/// Returns the env vars to **set** and the env vars to **remove**. Removal is
/// belt-and-suspenders: an inherited parent env var must not leak into a
/// child agent and silently change its security posture.
///
/// The `owner_hex` argument is the current workspace owner pubkey. It's used
/// as a fallback for legacy records (`auth_tag.is_none()`) — without it, the
/// harness's owner cache stays empty and `owner-only` / `allowlist` modes
/// drop everything.
///
/// Returns `Err(...)` if the record's allowlist fails validation. The harness
/// validates too, but doing it here means we never spawn a doomed process.
pub(crate) fn build_respond_to_env(
    record: &ManagedAgentRecord,
    owner_hex: Option<&str>,
) -> Result<RespondToEnv, String> {
    build_respond_to_env_with_policy(record, owner_hex, super::owner_only())
}

pub(crate) fn configure_runtime_cli(
    command: &mut std::process::Command,
    runtime: Option<&KnownAcpRuntime>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.id != "claude" {
        return;
    }
    if let Some(cli_path) = super::claude_cli::resolved_claude_cli() {
        // On Windows, `.cmd` and `.bat` files are batch shims — they cannot be
        // passed directly to `CreateProcess` and cause EINVAL when the Claude
        // adapter tries to spawn them (issue #2397). Skip setting
        // `CLAUDE_CODE_EXECUTABLE` for shim paths so the adapter falls back to
        // its own PATH lookup and finds the real binary instead.
        // Non-Windows: `.cmd`/`.bat` are valid executables and must be assigned.
        if should_skip_claude_executable(&cli_path, cfg!(windows)) {
            return;
        }
        command.env("CLAUDE_CODE_EXECUTABLE", cli_path);
    }
}

/// Proof token for the effort-application outer binding. `#[must_use]`;
/// makes `let effort = apply_effort_to_spawn_command(…)` a compile-time
/// requirement — deleting the binding is a compile error because
/// `spawn_with_effort_proof` consumes it by value.
///
/// The private field prevents any crate-local code from constructing
/// `EffortApplied` directly (same shape as `RecordFieldsApplied(())`), so
/// the only way to obtain a token is to call `apply_effort_to_spawn_command`.
#[must_use]
pub(crate) struct EffortApplied(());

/// Apply effort env to an agent spawn command. Called by `spawn_agent_child`
/// (production) and `effort_cmd_tests` (test seam). Inner-seam: removing
/// `apply_spawn_effort_env` below turns the production-sequence tests RED.
/// Outer-seam: the returned token is consumed by `spawn_with_effort_proof`;
/// deleting this call leaves `effort` undefined at the spawn site.
pub(crate) fn apply_effort_to_spawn_command(
    cmd: &mut std::process::Command,
    record: &crate::managed_agents::types::ManagedAgentRecord,
    runtime: Option<&crate::managed_agents::discovery::KnownAcpRuntime>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    persona_id: Option<&str>,
    global_env: &std::collections::BTreeMap<String, String>,
    baked_env: &std::collections::BTreeMap<String, String>,
) -> EffortApplied {
    super::config_bridge::effort::apply_spawn_effort_env(
        cmd, record, runtime, personas, persona_id, global_env, baked_env,
    );
    EffortApplied(())
}

/// Spawn the agent command, consuming the `EffortApplied` proof token.
/// Deleting `apply_effort_to_spawn_command` from `spawn_agent_child` leaves
/// `effort` undefined here — a compile error CI catches before any test runs.
pub(crate) fn spawn_with_effort_proof(
    cmd: &mut std::process::Command,
    _effort: EffortApplied,
) -> std::io::Result<std::process::Child> {
    cmd.spawn()
}

/// Spawn an agent process without holding any locks on records or runtimes.
/// Returns the child process and log path on success. The caller is responsible
/// for updating `ManagedAgentRecord` fields and inserting into the runtimes map.
///
/// `owner_hex`: the workspace owner's pubkey, used as a fallback for legacy
/// records that have no NIP-OA `auth_tag`. See `build_respond_to_env`.
///
/// `replay_floor_unix`: optional unix-seconds replay floor for the harness's
/// startup watermark (`BUZZ_ACP_REPLAY_FLOOR`). A publish-first mention send
/// publishes the triggering message before this spawn and passes its send
/// timestamp here so the harness's first REQ replays past that message no
/// matter how long the spawn takes. buzz-acp clamps stale floors to ~15 min.
pub fn spawn_agent_child(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    relay_url: &str,
    lazy: bool,
    owner_hex: Option<&str>,
    replay_floor_unix: Option<u64>,
) -> Result<crate::managed_agents::ManagedAgentProcess, String> {
    let state = app.state::<crate::app_state::AppState>();
    crate::managed_agents::require_fleet_host(&state)?;
    let session_residency_enabled = state.session_residency_enabled().load(Ordering::Acquire);
    if let Some(error) = spawn_key_refusal(record) {
        return Err(error);
    }
    let runtime_key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), relay_url)?;
    super::external_execution::require_local(app, &runtime_key)?;
    super::account_switch::check_spawn(app, record, &runtime_key)?;
    // Resolve the effective harness (agent command) from the linked persona, so
    // persona harness edits propagate on the next spawn; an explicit per-agent
    // override wins. `agent_args` and `mcp_command` are pure derivations of the
    // command, so we recompute them from the effective value rather than the
    // frozen record snapshot. Mirrors the model resolution below.
    let personas = super::load_personas(app).unwrap_or_default();
    let teams = super::load_teams(app).unwrap_or_default();
    // Load global config once; used for runtime_metadata_env_vars (model/provider fallback)
    // and for the env-var merge at spawn time.
    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();

    // Resolve model/provider/prompt ONCE, here, at the shared spawn boundary —
    // the single source both the env writes below and the spawn-config snapshot
    // read from. Previously prompt was read from the record's own (possibly
    // stale, Phase-A-snapshot) bytes while model/provider were resolved live
    // from `personas`; a definition edit landing between a caller's snapshot
    // apply and this spawn could hand a fresh model/provider to a stale
    // prompt. This also folds in orphan refusal via `require_resolved`: every
    // caller (interactive start, launch restore, `start_managed_agent_process`)
    // inherits it — no caller can bypass this by reaching `spawn_agent_child`
    // directly. Checked before any side effect (log marker, log file, process
    // spawn) so a refused spawn leaves no trace.
    let effective_cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record, &personas, &global,
    )
    .require_resolved()?;

    // Single typed resolver: validates runtime id (dangling harness → Err), resolves
    // command, args (instance wins over definition default), and the full env layer stack.
    // This is the sole path for harness-definition lookup — spawn, snapshot,
    // summary, and model probes all consume this descriptor rather than
    // assembling values inline.
    // Like the orphan refusal above, this runs before any side effect so a refused
    // spawn leaves no trace.
    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, &personas, &global)
            .map_err(|e| {
                format!(
                    "cannot spawn agent {}: {}",
                    record.pubkey,
                    crate::managed_agents::user_facing_harness_error(&e)
                )
            })?;
    let effective_command = &descriptor.command;
    let agent_args = &descriptor.args;
    if known_acp_runtime(effective_command).is_some_and(|runtime| runtime.id == "claude") {
        if let Some(blocker) =
            super::claude_cli::blocker_for_model(effective_cfg.model.value.as_deref())
        {
            return Err(blocker.message);
        }
    }

    let log_path = super::managed_agent_runtime_log_path(app, &runtime_key)?;
    append_log_marker(
        &log_path,
        &format!(
            "\n=== starting {} ({}) at {} ===",
            record.name,
            record.pubkey,
            now_iso()
        ),
    )?;

    let stdout = open_log_file(&log_path)?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone log handle: {error}"))?;
    let resolved_acp_command = resolve_command(&record.acp_command)
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let effective_mcp_command = known_acp_runtime(effective_command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("");
    let resolved_mcp_command: Option<std::path::PathBuf> = if effective_mcp_command.is_empty() {
        None
    } else {
        match resolve_command(effective_mcp_command) {
            Some(path) => Some(path),
            None => {
                eprintln!(
                    "buzz-desktop: mcp_command {effective_mcp_command:?} not found, skipping"
                );
                None
            }
        }
    };
    // Resolve agent command to a full path (DMG launches have minimal PATH).
    let resolved_agent_command = resolve_command(effective_command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| effective_command.clone());

    // The caller supplies the explicit canonical pair relay. This is the only
    // relay this child may connect to, regardless of the record/workspace default.
    let effective_relay_url = runtime_key.relay_url.clone();
    // Augment PATH for DMG launches so child processes can find:
    //   - bundled CLI via ~/.local/bin symlink
    //   - nvm-managed node/npm (nvm initializes only in interactive shells)
    //   - bundled sidecars (buzz, buzz-acp, etc.) via exe parent (Contents/MacOS/)
    //   - runtimes (node, python, etc.) via login shell PATH
    let nvm_bin = dirs::home_dir()
        .as_deref()
        .and_then(super::find_nvm_default_bin);
    let augmented_path = build_augmented_path(
        dirs::home_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        login_shell_path(),
        nvm_bin,
    );

    let mut command = std::process::Command::new(&resolved_acp_command);
    if let Some(home) = super::default_agent_workdir() {
        command.current_dir(home);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::from(stdout));
    command.stderr(std::process::Stdio::from(stderr));
    if let Some(ref path) = augmented_path {
        command.env("PATH", path);
    }
    command.env("RUST_LOG", child_rust_log_filter());
    command.env("BUZZ_PRIVATE_KEY", &record.private_key_nsec);
    command.env("BUZZ_RELAY_URL", &effective_relay_url);
    command.env("BUZZ_ACP_LAZY_POOL", if lazy { "true" } else { "false" });
    command.env(
        "BUZZ_ACP_IDLE_POOL_SLEEP",
        idle_pool_sleep_env(
            lazy,
            session_residency_enabled,
            record.idle_pool_sleep_seconds,
        ),
    );
    command.env(
        "BUZZ_ACP_MAX_TURNS_PER_SESSION",
        if session_residency_enabled {
            record.max_turns_per_session.unwrap_or(0)
        } else {
            0
        }
        .to_string(),
    );
    command.env(
        "BUZZ_ACP_SLEEP_WITH_SUMMARY",
        if session_residency_enabled {
            "true"
        } else {
            "false"
        },
    );
    if session_residency_enabled {
        command.env(
            "BUZZ_ACP_SLEEP_SUMMARY_DIR",
            managed_agent_continuity_dir(app, &runtime_key)?,
        );
        command.env("BUZZ_ACP_PREFIX_BUDGET_WARNINGS", "true");
        command.env("BUZZ_ACP_PREFIX_CORE_WARNING_BYTES", "12288");
        command.env("BUZZ_ACP_PREFIX_TOTAL_WARNING_BYTES", "204800");
    } else {
        command.env_remove("BUZZ_ACP_SLEEP_SUMMARY_DIR");
        command.env("BUZZ_ACP_PREFIX_BUDGET_WARNINGS", "false");
        command.env_remove("BUZZ_ACP_PREFIX_CORE_WARNING_BYTES");
        command.env_remove("BUZZ_ACP_PREFIX_TOTAL_WARNING_BYTES");
    }
    // Publish-first mention sends hand the harness the send timestamp as a
    // startup replay floor. Strip any ambient value here — before the
    // `descriptor.env` loop — so a floor from the parent environment can never
    // leak into an unrelated spawn; the caller's floor is asserted AFTER that
    // loop by `apply_replay_floor_env` so saved user env cannot shadow it.
    command.env_remove(REPLAY_FLOOR_ENV_VAR);
    command.env("BUZZ_ACP_AGENT_COMMAND", &resolved_agent_command);
    command.env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
    match &resolved_mcp_command {
        Some(mcp_cmd) => {
            command.env("BUZZ_ACP_MCP_COMMAND", mcp_cmd);
        }
        None => {
            command.env("BUZZ_ACP_MCP_COMMAND", "");
        }
    }
    // Enable MCP hook tools (_Stop, _PostCompact) for agents that need them.
    // Uses "*" because build_mcp_servers() hard-codes the server name to "buzz-mcp".
    let runtime_meta = known_acp_runtime(effective_command);
    if runtime_meta.is_some_and(|r| r.mcp_hooks) {
        command.env("MCP_HOOK_SERVERS", "*");
    }

    // ── Readiness check: set setup-payload if agent is not ready ─────────────
    // `spawned_setup_mode` is stamped on `ManagedAgentProcess` below.
    let spawned_setup_mode =
        apply_setup_payload_env(&mut command, record, &descriptor, runtime_meta);
    // Emit BUZZ_ACP_IDLE_TIMEOUT only when explicitly set; the harness
    // DEFAULT_IDLE_TIMEOUT_SECS is the single source of truth. The deprecated
    // BUZZ_ACP_TURN_TIMEOUT pinned agents to a stale default (320s).
    if let Some(idle) = record.idle_timeout_seconds {
        command.env("BUZZ_ACP_IDLE_TIMEOUT", idle.to_string());
    }

    if let Some(max_dur) = record.max_turn_duration_seconds {
        command.env("BUZZ_ACP_MAX_TURN_DURATION", max_dur.to_string());
    }
    // DeepSeek Harness composition: provision the owned buzz-acp profile into
    // the nest and derive its environment (DSH_HOME redirect + idle-timeout
    // default). Runs only for the dsh identity; every entry yields to an
    // operator- or record-set value.
    let dsh_home_dir = dsh_provision_profile(
        super::discovery::normalize_command_identity(effective_command).as_str(),
    );
    for (key, value) in dsh_derived_env(
        dsh_home_dir.as_deref(),
        std::env::var_os("DSH_HOME").is_some(),
        std::env::var_os("BUZZ_ACP_IDLE_TIMEOUT").is_some(),
        record.idle_timeout_seconds,
    ) {
        command.env(key, value);
    }
    let acp_n = super::acp_agents_value(effective_command, record.parallelism);
    command.env("BUZZ_ACP_AGENTS", acp_n);
    command.env("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer");
    command.env("BUZZ_ACP_DEDUP", "queue");
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                command.env(key, value);
            }
        }
    }
    let team_instructions = super::spawn_snapshot::effective_team_instructions(record, &teams);
    if let Some(instructions) = &team_instructions {
        command.env("BUZZ_ACP_TEAM_INSTRUCTIONS", instructions);
    } else {
        command.env_remove("BUZZ_ACP_TEAM_INSTRUCTIONS");
    }

    // Prompt, model, and provider all come from the single `effective_cfg`
    // resolved at the top of this function — the SAME resolve the spawn-config
    // snapshot reads, so env write and restart badge cannot disagree. Linked
    // instances never consult the record's own model/provider/prompt bytes;
    // definition-less instances fall back to their own fields, then global.
    //
    // Derive the mesh decision BEFORE moving fields out — `relay_mesh_model_id`
    // is the single authoritative gate; the mesh-llm block below MUST use it
    // rather than re-deriving from `effective_provider` to keep preflight and
    // spawn semantics in lock-step (see `EffectiveAgentConfig::relay_mesh_model_id`).
    #[cfg(feature = "mesh-llm")]
    let mesh_model_id = effective_cfg.relay_mesh_model_id();
    let effective_prompt = effective_cfg.system_prompt.value;
    let effective_model = effective_cfg.model.value;
    let effective_provider = effective_cfg.provider.value;

    if let Some(prompt) = &effective_prompt {
        command.env("BUZZ_ACP_SYSTEM_PROMPT", prompt);
    } else {
        command.env_remove("BUZZ_ACP_SYSTEM_PROMPT");
    }
    // Shared compute stores `auto`, but the wire name is MeshLLM's virtual
    // `mesh` model. Translate here too, so the harness and the LLM client are
    // told the same thing: `BUZZ_ACP_MODEL=auto` would name a model the mesh
    // never advertises, leaving buzz-acp to warn and fall back on every new
    // session while `BUZZ_AGENT_MODEL` said `mesh`.
    #[cfg(feature = "mesh-llm")]
    let acp_model = match (&mesh_model_id, effective_model.as_deref()) {
        (Some(mesh_model_id), _) => Some(super::relay_mesh_wire_model(mesh_model_id).to_string()),
        (None, model) => model.map(str::to_owned),
    };
    #[cfg(not(feature = "mesh-llm"))]
    let acp_model = effective_model.as_deref().map(str::to_owned);
    if let Some(model) = acp_model.as_deref() {
        command.env("BUZZ_ACP_MODEL", model);
    } else {
        command.env_remove("BUZZ_ACP_MODEL");
    }
    // Session title for the harness to pass out-of-band on `session/new`. The
    // adapter names the session after it; it never reaches the prompt, so this
    // is display metadata only. The spawn-config snapshot records the same
    // resolve, so a rename raises the restart badge instead of leaving the
    // process stale.
    apply_agent_display_env(
        &mut command,
        resolve_session_title(record.display_name.as_deref(), &record.name),
    );
    if let Some(meta) = runtime_meta {
        for (key, value) in runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            effective_model.as_deref(),
            effective_provider.as_deref(),
        ) {
            command.env(key, value);
        }
    }
    command.env_remove("BUZZ_ACP_PRIVATE_KEY");
    command.env_remove("BUZZ_ACP_API_TOKEN");
    command.env_remove("BUZZ_API_TOKEN");

    if let Some(ref auth_tag) = record.auth_tag {
        command.env("BUZZ_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("BUZZ_AUTH_TAG");
    }

    // Inbound author gate: who is this agent allowed to respond to?
    // Validation is strict here — a malformed allowlist on disk fails before
    // we spawn anything (the harness would also reject it, but we'd rather
    // fail with a clear error than crash-loop the child).
    let (gate_set, gate_remove) = build_respond_to_env(record, owner_hex)?;
    for (key, value) in &gate_set {
        command.env(key, value);
    }
    for key in &gate_remove {
        command.env_remove(key);
    }

    command.env("BUZZ_ACP_RELAY_OBSERVER", "true");

    // Git credential helper: NIP-98 auth for Buzz relay git via git-credential-nostr.
    // Ephemeral GIT_CONFIG_COUNT env vars scoped to relay HTTP URL; NOSTR_PRIVATE_KEY mirrors BUZZ_PRIVATE_KEY.
    if let Some(cred_helper) = resolve_command("git-credential-nostr") {
        let relay_http_url = crate::relay::relay_http_base_url(&effective_relay_url);

        command.env("NOSTR_PRIVATE_KEY", &record.private_key_nsec);
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.env("GIT_CONFIG_COUNT", "2");
        command.env(
            "GIT_CONFIG_KEY_0",
            format!("credential.{relay_http_url}/git.helper"),
        );
        let helper = cred_helper.to_string_lossy().replace('\\', "/");
        command.env("GIT_CONFIG_VALUE_0", helper);
        command.env(
            "GIT_CONFIG_KEY_1",
            format!("credential.{relay_http_url}/git.useHttpPath"),
        );
        command.env("GIT_CONFIG_VALUE_1", "true");
    } else {
        eprintln!(
            "buzz-desktop: git-credential-nostr not found — agent {} will not have automatic Buzz git auth",
            record.name,
        );
    }

    // Fully layered, reserved-key-filtered user env; structured fields below remain authoritative.
    for (key, value) in &descriptor.env {
        command.env(key, value);
    }
    // dsh's adapter requires a token even for servers with no authentication.
    // Supply a non-secret placeholder only on the explicitly compatible route.
    if runtime_meta.is_some_and(|meta| meta.id == "deepseek") {
        let value = |key: &str| {
            descriptor
                .env
                .get(key)
                .cloned()
                .or_else(|| std::env::var(key).ok())
                .map(|value| value.trim().to_string())
        };
        for key in [
            "DSH_PROVIDER",
            "DSH_LLM_BASE_URL",
            "DSH_API_KEY",
            "DEEPSEEK_API_KEY",
        ] {
            if let Some(value) = value(key) {
                command.env(key, value);
            }
        }
        if value("DSH_PROVIDER").as_deref() == Some("deepseek-compatible")
            && value("DSH_API_KEY").is_none_or(|v| v.trim().is_empty())
        {
            command.env("DSH_API_KEY", "local-no-auth");
        }
    }

    crate::model_connections::runtime::apply(
        app,
        &mut command,
        effective_command,
        &descriptor.env,
        effective_model.as_deref(),
        &record.pubkey,
    )?;

    // Strip all known effort keys and emit exactly one projected key. Command
    // inherits the parent env — the returned EffortApplied token is consumed
    // by spawn_with_effort_proof below; deleting this call is a compile error.
    let effort = apply_effort_to_spawn_command(
        &mut command,
        record,
        runtime_meta,
        &personas,
        record.persona_id.as_deref(),
        &global.env_vars,
        &super::agent_env::baked_build_env(),
    );
    // Carry trusted provenance for generic ACP effort. A Desktop-global value
    // is a harness default and may be ignored when the active model exposes no
    // effort option; persona/agent values remain explicit and fail loud. Clear
    // any ambient marker first, then set only the Desktop-derived result.
    let persona_env = super::env_vars::live_persona_env(&personas, record.persona_id.as_deref());
    apply_thinking_effort_provenance(
        &mut command,
        &persona_env,
        &record.env_vars,
        &global.env_vars,
    );
    if record.effort_level.is_some() {
        command.env_remove("BUZZ_ACP_THINKING_EFFORT_IS_DEFAULT");
    }
    // Resolve once and stamp the same value onto the snapshot below.
    // Resolve once and stamp the same value onto the environment and snapshot.
    let acp_session_policy = super::effective_acp_session_policy(record, &personas);
    super::apply_acp_session_policy_env(&mut command, acp_session_policy);
    super::apply_app_answered_event_watermark_env(app, &mut command);

    crate::build_identity::apply_demo_config_home(&mut command)?;
    // Structured model state wins over stale free-form BUZZ_ACP_MODEL values.
    if let Some(model) = effective_model.as_deref() {
        command.env("BUZZ_ACP_MODEL", model);
    } else {
        command.env_remove("BUZZ_ACP_MODEL");
    }
    // Claude has a single startup model authority. Generic ACP runtimes keep
    // BUZZ_ACP_MODEL; local Claude receives ANTHROPIC_MODEL instead.
    // Publish-first replay floor: written AFTER the `descriptor.env` loop, the
    // same post-loop authority ordering the A1 model write uses. This send's
    // floor is invocation state and must win over a saved
    // BUZZ_ACP_REPLAY_FLOOR — the shadow `apply_replay_floor` strips from the
    // provider payload's `launch.env` tier for the same reason.
    apply_replay_floor_env(&mut command, replay_floor_unix);

    if record.backend == super::BackendKind::Local && runtime_meta.is_some_and(|r| r.id == "claude")
    {
        apply_claude_model_env(&mut command, effective_model.as_deref());
    }
    // Claude OAuth is derived at spawn and must never come from persisted env_vars.
    let (
        spawned_claude_account_id,
        spawned_claude_token_fingerprint,
        spawned_claude_account_source,
    ) = if record.subscription_account_id.is_some() {
        (None, None, "subscription_account".into())
    } else {
        super::configure_spawned_claude_account(
            &mut command,
            runtime_meta.is_some_and(|runtime| runtime.id == "claude"),
            record,
            &personas,
            &global,
        )?
    };

    if let Some((desktop_path, desktop_hash)) = crate::runtime_identity::desktop_identity() {
        command.env("BUZZ_ACP_DESKTOP_PATH", desktop_path);
        command.env("BUZZ_ACP_DESKTOP_SHA256", desktop_hash);
    }
    let broker_hash = crate::runtime_identity::cached_file_sha256(&resolved_acp_command)
        .unwrap_or_else(|| "unavailable".to_string());
    eprintln!(
        "buzz-desktop: agent_runtime_identity agent={} desktop_path={} broker_path={} broker_sha256={} requested_model={} token_source={} token_fingerprint={} token_length={} token_has_whitespace=false",
        record.name,
        crate::runtime_identity::desktop_identity()
            .map(|(path, _)| path.display().to_string())
            .unwrap_or_else(|| "unavailable".to_string()),
        resolved_acp_command.display(),
        broker_hash,
        effective_model.as_deref().unwrap_or("<default>"),
        spawned_claude_account_source,
        spawned_claude_token_fingerprint
            .as_deref()
            .unwrap_or("unavailable"),
        spawned_claude_account_id.as_ref().map_or(0, |_| 108),
    );
    configure_runtime_cli(&mut command, runtime_meta);

    // Buzz shared compute is stored as a native provider; derive the OpenAI-compatible
    // transport at spawn time and scrub any unrelated ambient OpenAI key.
    // Gate on `mesh_model_id` (derived from `effective_cfg.relay_mesh_model_id()`
    // above) — not on `effective_provider` directly — so the mesh gate here
    // uses the same trim semantics as the preflight callers.
    #[cfg(feature = "mesh-llm")]
    if let Some(ref mesh_model_id) = mesh_model_id {
        let mesh_env = super::relay_mesh_process_env(&descriptor.env, mesh_model_id);
        command.env_remove("OPENAI_API_KEY");
        for (key, value) in mesh_env {
            command.env(key, value);
        }
    }

    let mut subscription_binding = super::subscription_binding::configure(
        app,
        record,
        super::subscription_binding::account_harness(runtime_meta.map(|r| r.id)),
        effective_provider.as_deref(),
        &mut command,
    )?;

    // Stamp desktop ownership and an unpredictable harness-generation identity.
    let start_nonce = uuid::Uuid::new_v4().simple().to_string();
    command
        .env("BUZZ_MANAGED_AGENT", current_instance_id(app))
        .env("BUZZ_MANAGED_AGENT_START_NONCE", &start_nonce);
    let account_control_dir = Some(super::subscription_binding::configure_control(
        app,
        &runtime_key,
        &start_nonce,
        &mut command,
    )?);

    // Stamp spawn config from values above, BEFORE spawning — a post-spawn
    // re-resolve races config edits and would stamp the wrong values.
    let spawn_config = super::spawn_snapshot::SpawnConfigSnapshot::from_inputs(
        super::spawn_snapshot::SpawnConfigInputs {
            record,
            descriptor: &descriptor,
            relay_url: &effective_relay_url,
            team_instructions: team_instructions.as_deref(),
            system_prompt: effective_prompt.as_deref(),
            model: effective_model.as_deref(),
            provider: effective_provider.as_deref(),
            enforced_owner_only: super::owner_only_access_build(),
            session_policy: acp_session_policy,
            session_residency_enabled,
        },
    );

    // Spawn in its own process group (Unix) or with CREATE_NO_WINDOW (Windows).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // Windows: suppress the harness console window. Without this a bare
    // terminal pops for buzz-acp.exe and lingers (the app itself sets
    // windows_subsystem="windows", but the spawned child does not inherit it).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    crate::computer_use::configure_runtime(
        app,
        &mut command,
        &format!("{} | {}", effective_relay_url, record.pubkey),
        &record.name,
    )?;
    let mut child = spawn_with_effort_proof(&mut command, effort).map_err(|error| {
        format!(
            "failed to spawn `{}` for agent {}: {error}",
            resolved_acp_command.display(),
            record.name
        )
    })?;

    if let Some(lease) = subscription_binding
        .as_mut()
        .and_then(|binding| binding._lease.as_mut())
    {
        if let Err(error) = lease.attach_pid(child.id()) {
            // The reservation was durable before spawn. If stamping its PID
            // fails, terminate this exact child; unresolved cleanup keeps the
            // reservation busy rather than admitting another account consumer.
            if let Err(cleanup) = terminate_process(child.id()) {
                return Err(format!(
                    "{error}; spawned runtime cleanup failed: {cleanup}"
                ));
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return Err(error),
                    Err(cleanup) => {
                        return Err(format!(
                            "{error}; spawned runtime cleanup failed: {cleanup}"
                        ))
                    }
                    Ok(None) if std::time::Instant::now() >= deadline => {
                        return Err(format!("{error}; spawned runtime exit remains unconfirmed"));
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
                }
            }
        }
    }

    // Stamp the adapter availability for runtimes with a version gate (codex
    // only). The summary builder compares this against the current cached value
    // to detect out-of-band adapter changes after spawn (Phase-2 badge fallback).
    // Non-codex runtimes get `None` — nothing changes for them.
    // When the cache is cold (e.g. Doctor just installed and cleared the cache),
    // `adapter_availability_cached()` returns `None`, so the stamp is `None` and
    // the drift check is skipped until discovery warms the cache — preventing a
    // false restart badge immediately after auto-restart.
    let spawned_adapter_availability = if runtime_meta.is_some_and(|r| r.id == "codex") {
        super::adapter_availability_cached()
    } else {
        None
    };

    // Receipt persistence belongs to the caller's atomic register transition.

    // Windows: assign the harness to a Job Object so its whole tree dies with
    // the handle. The Unix process-group equivalent is set above.
    #[cfg(windows)]
    return Ok(super::process_lifecycle::finish_spawn(
        child,
        log_path,
        spawn_config,
        spawned_claude_account_id,
        spawned_claude_token_fingerprint,
        spawned_claude_account_source,
        spawned_setup_mode,
        spawned_adapter_availability,
        start_nonce,
        subscription_binding,
        account_control_dir,
        &record.name,
    ));
    #[cfg(not(windows))]
    Ok(crate::managed_agents::ManagedAgentProcess {
        child,
        log_path,
        spawn_config,
        claude_account_id: spawned_claude_account_id,
        claude_token_fingerprint: spawned_claude_token_fingerprint,
        claude_account_source: spawned_claude_account_source,
        setup_mode: spawned_setup_mode,
        adapter_availability: spawned_adapter_availability,
        start_nonce,
        subscription_binding,
        account_control_dir,
    })
}

/// Spawn (or adopt) the runtime pair for `record` on the caller's bound
/// workspace relay. `workspace_relay` can only be produced by
/// `bind_expected_relay_scope`, so this spawn consumes — by construction — the
/// exact workspace-relay read the caller's scope assertion passed on; it never
/// re-reads the mutable override (see `relay::scope`). The key comes from
/// [`bound_runtime_key`] — the seam the spawn-key regressions exercise.
/// Derived environment entries for the DeepSeek Harness (`dsh`) identity.
///
/// `dsh_profile_dir` is `Some` only for dsh identities whose owned composition
/// provisioned successfully (see [`dsh_provision_profile`]); the redirect then
/// yields to an operator-set `DSH_HOME`. The idle-timeout default exists
/// because dsh's ACP leaf emits committed text only — silence equals whole-turn
/// latency, and high-reasoning turns measured in the D0 spike can outlast the
/// harness's 900 s default — and yields to a record- or operator-set value.
pub(crate) fn dsh_derived_env(
    dsh_profile_dir: Option<&std::path::Path>,
    parent_has_dsh_home: bool,
    parent_has_idle_timeout: bool,
    record_idle_timeout: Option<u64>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(dir) = dsh_profile_dir {
        if !parent_has_dsh_home {
            env.push(("DSH_HOME".to_string(), dir.display().to_string()));
        }
        if record_idle_timeout.is_none() && !parent_has_idle_timeout {
            env.push((
                "BUZZ_ACP_IDLE_TIMEOUT".to_string(),
                DSH_DEFAULT_IDLE_TIMEOUT_SECS.to_string(),
            ));
        }
    }
    env
}

const DSH_DEFAULT_IDLE_TIMEOUT_SECS: u64 = 1800;

/// Provision the owned buzz-acp profile for a dsh agent into the nest's
/// `.dsh-home`, returning the profile home on success. Non-dsh identities skip
/// entirely; provisioning failure is logged and reported as `None` so the
/// spawn proceeds under whatever operator-provided setup exists (the dsh boot
/// will fail loud there if the profile is genuinely missing).
fn dsh_provision_profile(command_identity: &str) -> Option<std::path::PathBuf> {
    if command_identity != "dsh" {
        return None;
    }
    let workdir = super::default_agent_workdir()?;
    dsh_provision_profile_in(&workdir)
}

/// Provision into `<workdir>/.dsh-home` and return the DSH home ROOT — never
/// the profile directory itself: dsh resolves `--profile <name>` at
/// `$DSH_HOME/profiles/<name>` (dsh-app-boot contract), so a DSH_HOME pointing
/// at the profile dir makes dsh look for
/// `.dsh-home/profiles/buzz-acp/profiles/buzz-acp` and fail boot.
fn dsh_provision_profile_in(workdir: &std::path::Path) -> Option<std::path::PathBuf> {
    let dsh_home = workdir.join(".dsh-home");
    match super::dsh_profile::provision(&dsh_home) {
        Ok(_) => Some(dsh_home),
        Err(error) => {
            eprintln!(
                "buzz-desktop: dsh profile provisioning failed ({error}); leaving DSH_HOME to operator setup"
            );
            None
        }
    }
}

pub fn start_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    owner_hex: Option<&str>,
    workspace_relay: &crate::relay::ScopedWorkspaceRelay,
    replay_floor_unix: Option<u64>,
) -> Result<(), String> {
    let personas = super::load_personas(app).unwrap_or_default();
    let global = super::load_global_agent_config(app).unwrap_or_default();
    if let Some(blocker) = super::claude_cli::blocker_for_record(record, &personas, &global)? {
        record.last_error = Some(blocker.message.clone());
        record.last_error_code = Some(blocker.code);
        record.updated_at = now_iso();
        return Err(blocker.message);
    }
    let key = bound_runtime_key(record, workspace_relay)?;
    if let Some(runtime) = runtimes.get_mut(&key) {
        if runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect running process: {error}"))?
            .is_none()
        {
            return Ok(());
        }

        runtimes.remove(&key);
        super::remove_agent_runtime_receipt(app, &key);
    }

    // Scalar PIDs are migration-only and never establish pair liveness.
    record.runtime_pid = None;

    let mut process = spawn_agent_child(
        app,
        record,
        &key.relay_url,
        false,
        owner_hex,
        replay_floor_unix,
    )?;
    let now = now_iso();
    let receipt = super::ManagedAgentRuntimeReceipt {
        subscription_account_id: process
            .subscription_binding
            .as_ref()
            .map(|b| b.account_id.clone()),
        subscription_account_generation: process
            .subscription_binding
            .as_ref()
            .map_or(0, |b| b.generation),
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(app),
        started_at: now.clone(),
    };
    if let Err(error) = super::write_agent_runtime_receipt(app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }

    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;
    record.last_error_code = None;

    let mut runtime = ManagedAgentPairRuntime::starting(process);
    // The profile Start command runs this on its blocking worker. Its first
    // lifecycle frame can precede the observer subscription; confirm the exact
    // broker locally without releasing authentication holds.
    if let Err(error) = super::subscription_binding::probe_runtime_binding(&mut runtime) {
        record.last_error = Some(error.clone());
        runtime.error = Some(error);
    }
    runtimes.insert(key, runtime);
    Ok(())
}

#[cfg(test)]
mod test_fixtures;

#[cfg(test)]
mod tests;
