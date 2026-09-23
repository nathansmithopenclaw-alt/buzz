//! B5 effort lifecycle tests split out of `spawn_snapshot/tests.rs` to hold
//! that file under the 1500-line file-size ratchet.
//!
//! Included as `mod ext` inside `tests.rs`, so `use super::*` gives access to
//! its `record`, `snap`, and `record_with_env_effort` helpers.

use super::*;

#[test]
fn effort_set_then_cleared_round_trips_to_no_effort_projection() {
    // Persist a canonical effort, then clear it: the projection must return to
    // the exact no-effort baseline, so the badge lights on set and clears on
    // clear rather than sticking.
    let baseline = snap(&record());
    let mut set = record();
    set.effort_level = Some("high".into());
    assert_ne!(baseline, snap(&set), "setting canonical effort must badge");
    // Clear the SAME record back to None — the projection must return to the
    // exact no-effort baseline, proving the round-trip clears rather than a
    // fresh record merely matching baseline.
    set.effort_level = None;
    assert_eq!(
        baseline,
        snap(&set),
        "clearing canonical effort restores the no-effort projection"
    );
}

#[test]
fn canonical_edit_under_record_native_env_is_empty_diff() {
    // For Goose, the record-native env key `GOOSE_THINKING_EFFORT` outranks the
    // canonical column (CLEAR authority order). With a record-native `low`
    // present, editing the shadowed canonical high→medium changes nothing
    // effective, so the projections are identical and no badge lights.
    let mut high_col = record_with_env_effort("low");
    high_col.effort_level = Some("high".into());
    let mut medium_col = record_with_env_effort("low");
    medium_col.effort_level = Some("medium".into());
    assert_eq!(
        snap(&high_col),
        snap(&medium_col),
        "editing a record-native-env-shadowed canonical must not badge"
    );
}

#[test]
fn clearing_record_native_env_reveals_canonical_and_creates_a_diff() {
    // Record-native env `low` shadows canonical `high`: removing the record env
    // key drops resolution to the canonical `high`, a real change that badges.
    let mut env_over_canonical = record_with_env_effort("low");
    env_over_canonical.effort_level = Some("high".into());
    let mut canonical_only = goose_record();
    canonical_only.effort_level = Some("high".into());
    assert_ne!(
        snap(&env_over_canonical),
        snap(&canonical_only),
        "removing the record-native env must reveal the canonical and badge"
    );
}

// ── Effort: single canonical representation ──────────────────────────────
//
// `effective_effort` and the snapshot's `effort_level` field are the sole
// carrier of startup effort. Every effort key is stripped from the snapshot
// `env` so an authority handoff at an unchanged effective value raises no
// spurious restart badge, while a genuine effort change surfaces exactly once.

/// Look up the `env.GOOSE_THINKING_EFFORT` leaf of a canonical snapshot, if any
/// (the record()'s runtime is Goose, so this is its destination key).
fn effort_env_leaf(canonical: &serde_json::Value) -> Option<&serde_json::Value> {
    canonical
        .get("env")
        .and_then(|env| env.get("GOOSE_THINKING_EFFORT"))
}

/// A Goose record whose record-native env seeds `GOOSE_THINKING_EFFORT` (the
/// top authority tier for Goose: effort comes from user env_vars, no column).
/// Pins `runtime = "goose"` so the effective command resolves to Goose and
/// `GOOSE_THINKING_EFFORT` is the record-*native* key — without it the record
/// falls back to the default `buzz-agent` runtime, for which that key is a
/// foreign env alias the projection suppresses rather than an authority tier.
fn record_with_env_effort(value: &str) -> ManagedAgentRecord {
    let mut rec = record();
    rec.runtime = Some("goose".into());
    rec.env_vars
        .insert("GOOSE_THINKING_EFFORT".into(), value.into());
    rec
}

/// A Goose record with no effort env: the canonical column is the authority.
fn goose_record() -> ManagedAgentRecord {
    let mut rec = record();
    rec.runtime = Some("goose".into());
    rec
}

#[test]
fn effective_effort_reads_the_projected_key_for_the_runtime() {
    // The projection reduced the descriptor env to one effort key under the
    // runtime's destination key. `effective_effort` reads exactly that key.
    // A Goose descriptor carries `GOOSE_THINKING_EFFORT`.
    let descriptor = EffectiveHarnessDescriptor {
        command: "goose".into(),
        args: vec![],
        env: BTreeMap::from([("GOOSE_THINKING_EFFORT".to_string(), "high".to_string())]),
    };
    assert_eq!(effective_effort(&descriptor).as_deref(), Some("high"));
}

#[test]
fn effective_effort_reads_acp_sentinel_for_keyless_runtime() {
    // Claude/Codex/keyless-ACP descriptors carry the effective value under the
    // ACP-startup sentinel, which is the destination key for a runtime with no
    // native thinking-effort env var (here: the claude adapter command).
    let descriptor = EffectiveHarnessDescriptor {
        command: "claude-code-acp".into(),
        args: vec![],
        env: BTreeMap::from([("BUZZ_ACP_EFFORT_LEVEL".to_string(), "low".to_string())]),
    };
    assert_eq!(effective_effort(&descriptor).as_deref(), Some("low"));
}

#[test]
fn effective_effort_is_none_without_a_projected_key() {
    let descriptor = EffectiveHarnessDescriptor {
        command: "goose".into(),
        args: vec![],
        env: BTreeMap::new(),
    };
    assert_eq!(effective_effort(&descriptor), None);
}

#[test]
fn snapshot_carries_effort_in_field_not_env() {
    // Always-canonicalize: a record-native effort reaches the snapshot ONLY as
    // the `effort_level` field; the raw env key is stripped so effort has one
    // representation, never two.
    let canonical = snap(&record_with_env_effort("low"));
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        Some("low"),
        "effective effort must land in the effort_level field"
    );
    assert_eq!(
        effort_env_leaf(&canonical),
        None,
        "GOOSE_THINKING_EFFORT must be stripped from the snapshot env"
    );
}

#[test]
fn foreign_transport_sentinel_is_suppressed_for_goose() {
    // A user-seeded `BUZZ_ACP_EFFORT_LEVEL` is a foreign transport key for a
    // Goose descriptor: never an authority tier, and stripped from the snapshot
    // env by the suppress set. Editing it low→medium changes nothing.
    let mut low = record();
    low.env_vars
        .insert("BUZZ_ACP_EFFORT_LEVEL".into(), "low".into());
    let mut medium = record();
    medium
        .env_vars
        .insert("BUZZ_ACP_EFFORT_LEVEL".into(), "medium".into());
    assert_eq!(
        snap(&low),
        snap(&medium),
        "a foreign transport effort key must be suppressed for Goose and never badge"
    );
    let canonical = snap(&low);
    assert_eq!(
        canonical
            .get("env")
            .and_then(|env| env.get("BUZZ_ACP_EFFORT_LEVEL")),
        None,
        "the foreign sentinel must be stripped from the snapshot env"
    );
}

#[test]
fn equal_value_effort_authority_handoff_env_to_canonical_is_no_op() {
    // Record-native env `low` (no column) → canonical column `low` while the
    // record env remains: the effective effort is `low` either way (env wins,
    // but the value is identical), so a restart would change nothing.
    let env_authority = record_with_env_effort("low");
    let mut canonical_authority = record_with_env_effort("low");
    canonical_authority.effort_level = Some("low".into());
    assert_eq!(
        snap(&env_authority),
        snap(&canonical_authority),
        "an authority handoff at the same effort value must not badge"
    );
}

#[test]
fn env_only_effort_edit_changes_effort_level_not_env() {
    // A record-native env effort edit (no column) moves the single
    // `effort_level` representation and never reintroduces a
    // `env.GOOSE_THINKING_EFFORT` leaf, so the diff names `effort_level` once
    // rather than duplicating it.
    let low = snap(&record_with_env_effort("low"));
    let high = snap(&record_with_env_effort("high"));
    assert_ne!(
        low, high,
        "a record-native effort edit must change the snapshot"
    );
    assert_eq!(
        low.get("effort_level").and_then(|v| v.as_str()),
        Some("low")
    );
    assert_eq!(
        high.get("effort_level").and_then(|v| v.as_str()),
        Some("high")
    );
    assert_eq!(effort_env_leaf(&low), None);
    assert_eq!(effort_env_leaf(&high), None);
}

#[test]
fn canonical_effort_edit_changes_snapshot() {
    let mut low = record();
    low.effort_level = Some("low".into());
    let mut high = record();
    high.effort_level = Some("high".into());
    assert_ne!(
        snap(&low),
        snap(&high),
        "a canonical effort edit must trip the restart badge"
    );
}

/// A custom-command record whose runtime matches no known ACP runtime, so the
/// launch projection suppresses ONLY its own ACP sentinel (external-review-#2
/// pass-through, r5): every foreign effort key survives untouched and the child
/// receives its raw effort env.
fn custom_command_record() -> ManagedAgentRecord {
    let mut rec = record();
    rec.agent_command_override = Some("/opt/custom/my-agent".into());
    rec
}

#[test]
fn custom_runtime_effort_env_stays_in_snapshot_and_diffs() {
    // Regression (external review, Carl): for an unknown/custom runtime the
    // launch projection strips only its own ACP sentinel, so the child receives
    // the raw `GOOSE_THINKING_EFFORT` from the wrapper's env. The snapshot must
    // retain that key as ordinary env — the projection consumed nothing into
    // `effort_level` (its dest key, the ACP sentinel, is absent) — so an edit to
    // it diffs the snapshot and fires the restart badge. The prior full strip
    // erased the key from both places, producing NO restart diff on an effort
    // edit and leaving the running agent on stale effort.
    let mut high = custom_command_record();
    high.env_vars
        .insert("GOOSE_THINKING_EFFORT".into(), "high".into());
    let canonical = snap(&high);
    assert_eq!(
        canonical
            .get("env")
            .and_then(|env| env.get("GOOSE_THINKING_EFFORT"))
            .and_then(|v| v.as_str()),
        Some("high"),
        "a custom runtime's effort env must remain in the snapshot as ordinary env"
    );
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        None,
        "the custom sentinel dest key is absent, so effort_level captures nothing"
    );

    let mut low = custom_command_record();
    low.env_vars
        .insert("GOOSE_THINKING_EFFORT".into(), "low".into());
    assert_ne!(
        snap(&low),
        canonical,
        "editing a custom runtime's effort env must trip the restart badge"
    );
}

#[test]
fn known_runtime_still_strips_native_effort_env_from_snapshot() {
    // The counter-case pinning the scoping: for a KNOWN runtime the full sweep
    // still applies, so `GOOSE_THINKING_EFFORT` reaches the snapshot only as the
    // single `effort_level` field — never as a phantom `env` entry alongside it.
    let canonical = snap(&record_with_env_effort("high"));
    assert_eq!(
        effort_env_leaf(&canonical),
        None,
        "a known runtime must still strip its native effort key from the snapshot env"
    );
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        Some("high"),
        "the known runtime's effort must land solely in the effort_level field"
    );
}

#[test]
fn custom_runtime_mixed_case_sentinel_is_captured_not_lost() {
    // Regression (external review, Carl, P2): for an unknown/custom runtime a
    // user-set mixed-case `buzz_acp_effort_level` (no column) must not vanish.
    // The launch projection now reconciles it — stripping the mixed-case
    // spelling and re-emitting the pass-through value under the canonical
    // `BUZZ_ACP_EFFORT_LEVEL` (see `effort_tests::
    // unknown_runtime_collapses_mixed_case_sentinel_to_canonical_when_no_column`)
    // — so `descriptor.env` carries exactly one canonical sentinel. The snapshot
    // captures it into `effort_level` and strips it from `env`. Before the r4/r5
    // fixes the mixed-case key survived while the exact-case read missed it, so
    // the value vanished from BOTH fields and an edit produced no restart diff.
    let mut high = custom_command_record();
    high.env_vars
        .insert("buzz_acp_effort_level".into(), "high".into());
    let canonical = snap(&high);
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        Some("high"),
        "a mixed-case pass-through sentinel must be captured into effort_level"
    );
    assert_eq!(
        canonical
            .get("env")
            .and_then(|env| env.get("buzz_acp_effort_level")),
        None,
        "the sentinel is the projection's dest key and is stripped from env once represented"
    );

    // The mutation pin: editing the mixed-case sentinel must trip the badge.
    // Reverting the fix (exact-case read + case-insensitive strip, or an empty
    // unknown-runtime suppress set) makes both snapshots carry
    // `effort_level = null` with the key stripped, so they compare equal and
    // this assertion fails.
    let mut low = custom_command_record();
    low.env_vars
        .insert("buzz_acp_effort_level".into(), "low".into());
    assert_ne!(
        snap(&low),
        canonical,
        "editing a mixed-case custom-runtime sentinel must trip the restart badge"
    );
}

#[test]
fn custom_runtime_canonical_column_wins_over_mixed_case_sentinel() {
    // The with-canonical-column collision case Carl asked for, verified at the
    // SNAPSHOT here and — decisively — at the projection/descriptor seam in
    // `effort_tests::unknown_runtime_column_wins_over_mixed_case_sentinel`. The
    // projection strips every case variant of the sentinel before emitting the
    // column value, so `descriptor.env` carries exactly `BUZZ_ACP_EFFORT_LEVEL=
    // <column>` and the child receives the column value on every platform (no
    // lowercase variant survives for Windows to case-fold over the canonical
    // key). This snapshot therefore reads the same truth the child gets: the
    // column wins `effort_level` and both case variants are absent from `env`.
    let mut high_col = custom_command_record();
    high_col.effort_level = Some("high".into());
    high_col
        .env_vars
        .insert("buzz_acp_effort_level".into(), "low".into());
    let canonical = snap(&high_col);
    assert_eq!(
        canonical.get("effort_level").and_then(|v| v.as_str()),
        Some("high"),
        "the canonical column wins effort_level over the pass-through sentinel"
    );
    let env = canonical.get("env").expect("snapshot has an env object");
    assert_eq!(
        env.get("BUZZ_ACP_EFFORT_LEVEL"),
        None,
        "the projection-emitted canonical sentinel is stripped from env"
    );
    assert_eq!(
        env.get("buzz_acp_effort_level"),
        None,
        "the user's mixed-case sentinel duplicate is stripped case-insensitively"
    );

    // Editing the authority (the column) still trips the badge.
    let mut low_col = custom_command_record();
    low_col.effort_level = Some("low".into());
    low_col
        .env_vars
        .insert("buzz_acp_effort_level".into(), "low".into());
    assert_ne!(
        snap(&low_col),
        canonical,
        "editing the canonical column must trip the restart badge"
    );
}

use crate::managed_agents::spawn_snapshot::{
    eligible_restart_diff, prospective_spawn_config_snapshot, ProspectiveSpawnConfigInputs,
    RestartDiffEntry, SpawnConfigSnapshot, TrackedSpawnState,
};
use crate::managed_agents::AcpSessionPolicy;

/// Build the prospective snapshot for a linked record under one definition policy.
fn snapshot_under(policy: AcpSessionPolicy) -> SpawnConfigSnapshot {
    let mut record = record();
    record.persona_id = Some("policy-persona".into());
    let mut definition = persona("policy-persona", None, "You are a test agent.");
    definition.session_policy = policy;
    prospective_spawn_config_snapshot(ProspectiveSpawnConfigInputs {
        record: &record,
        personas: &[definition],
        teams: &[],
        workspace_relay: "wss://ws.example",
        global: &Default::default(),
        enforced_owner_only: false,

        session_residency_enabled: false,
    })
}

/// Restart-badge entries for a stamped→current session-policy transition,
/// exercising the real badge path (`eligible_restart_diff`).
fn policy_transition_diff(
    stamped: &SpawnConfigSnapshot,
    current: &SpawnConfigSnapshot,
) -> Vec<RestartDiffEntry> {
    eligible_restart_diff(
        false,
        Some(TrackedSpawnState {
            stamped,
            current,
            stamped_availability: None,
            current_availability: None,
        }),
    )
}

#[test]
fn toggling_session_policy_while_running_requires_restart() {
    // A definition edit must reach the real config-drift path. The harness
    // reads BUZZ_ACP_SESSION_POLICY only at launch, so an existing process
    // keeps its old policy until the normal restart affordance is used.
    let channel = snapshot_under(AcpSessionPolicy::Channel);
    let thread = snapshot_under(AcpSessionPolicy::Thread);

    // channel -> thread lights exactly the session_policy entry.
    let forward = policy_transition_diff(&channel, &thread);
    assert_eq!(
        forward.iter().map(|e| e.field.as_str()).collect::<Vec<_>>(),
        vec!["session_policy"],
    );

    // thread -> channel is equally visible (rollback also restarts).
    let reverse = policy_transition_diff(&thread, &channel);
    assert_eq!(
        reverse.iter().map(|e| e.field.as_str()).collect::<Vec<_>>(),
        vec!["session_policy"],
    );
}

#[test]
fn unchanged_session_policy_does_not_require_restart() {
    // An unchanged policy must not badge — the default (channel) case must stay
    // byte-for-byte inert so existing running agents don't flash a spurious
    // restart badge after this change ships.
    let channel = snapshot_under(AcpSessionPolicy::Channel);
    assert!(
        policy_transition_diff(&channel, &snapshot_under(AcpSessionPolicy::Channel)).is_empty()
    );

    let thread = snapshot_under(AcpSessionPolicy::Thread);
    assert!(policy_transition_diff(&thread, &snapshot_under(AcpSessionPolicy::Thread)).is_empty());
}

#[test]
fn canonical_effort_suppresses_legacy_restart_noise() {
    let mut configured = record();
    configured.effort_level = Some("high".into());
    let baseline = snap(&configured);
    configured
        .env_vars
        .insert("BUZZ_ACP_THINKING_EFFORT".into(), "low".into());
    assert_eq!(snap(&configured), baseline);
    configured.effort_level = None;
    assert_ne!(
        snap(&configured),
        snap(&record()),
        "legacy fallback still affects an unconfigured agent"
    );
}

#[test]
fn definitionless_instance_retains_its_stored_session_policy() {
    let mut instance = record();
    instance.session_policy = AcpSessionPolicy::Thread;

    assert_eq!(
        crate::managed_agents::effective_acp_session_policy(&instance, &[]),
        AcpSessionPolicy::Thread
    );
}

/// The stamped/current pair the runtime compares, as real snapshots rather
/// than canonical JSON, so the card-Save tests below run the exact
/// `needs_restart` computation.
fn prospective(record: &ManagedAgentRecord, personas: &[AgentDefinition]) -> SpawnConfigSnapshot {
    prospective_spawn_config_snapshot(ProspectiveSpawnConfigInputs {
        record,
        personas,
        teams: &[],
        workspace_relay: "wss://ws.example",
        global: &Default::default(),
        enforced_owner_only: false,
        session_residency_enabled: false,
    })
}

fn restart_diff(stamped: &SpawnConfigSnapshot, current: &SpawnConfigSnapshot) -> Vec<String> {
    eligible_restart_diff(
        false,
        Some(TrackedSpawnState {
            stamped,
            current,
            stamped_availability: None,
            current_availability: None,
        }),
    )
    .into_iter()
    .map(|entry| entry.field)
    .collect()
}

/// A Save from an Agents-page card is two writes: the definition save, then one
/// `update_managed_agent`, here carrying an access change. That access change
/// stops the agent's pairs and restarts them (`start_local_agent_pairs_with_preflight`,
/// which re-pins the record before it spawns). A linked agent's spawn resolves
/// the definition live, so a restart that runs after the definition save stamps
/// exactly the config the badge predicts and leaves nothing for the idle
/// auto-restart: one Save, one restart. The order is load-bearing: a restart
/// taken before the definition save would leave a second one pending.
#[test]
fn card_save_access_restart_after_definition_save_leaves_no_drift() {
    use crate::managed_agents::persona_events::{
        apply_persona_snapshot, persona_content_hash, persona_event_content,
    };
    let mut old_def = persona("pers", Some("goose"), "old prompt");
    old_def.model = Some("model-a".into());
    let mut new_def = persona("pers", Some("goose"), "new prompt");
    new_def.model = Some("model-b".into());

    // The running pair was spawned, and re-pinned, under the old definition.
    let mut rec = record();
    rec.persona_id = Some("pers".into());
    apply_persona_snapshot(&mut rec, &old_def);
    let stamped = prospective(&rec, std::slice::from_ref(&old_def));

    // Card Save: the definition saves first, then update_managed_agent writes
    // the agent's own access and thinking level.
    rec.respond_to = RespondTo::Anyone;
    rec.effort_level = Some("high".into());
    let predicted = prospective(&rec, std::slice::from_ref(&new_def));
    assert!(
        !restart_diff(&stamped, &predicted).is_empty(),
        "both halves of the Save are pending until the restart"
    );

    // Wrong order, for contrast: an access restart taken before the definition
    // save stamps the old definition, so a second restart stays pending.
    let restarted_first = prospective(&rec, std::slice::from_ref(&old_def));
    assert!(
        !restart_diff(&restarted_first, &predicted).is_empty(),
        "restarting before the definition save leaves drift behind"
    );

    // Real order: the access restart re-pins, then spawns under the saved
    // definition and stamps what it runs.
    apply_persona_snapshot(&mut rec, &new_def);
    let restamped = prospective(&rec, std::slice::from_ref(&new_def));
    assert_eq!(
        restamped.canonical(),
        predicted.canonical(),
        "the restart runs exactly the config the badge predicted"
    );
    assert_eq!(
        restart_diff(
            &restamped,
            &prospective(&rec, std::slice::from_ref(&new_def))
        ),
        Vec::<String>::new(),
        "no needs_restart is left for the idle auto-restart"
    );
    assert_eq!(
        rec.persona_source_version.as_deref(),
        Some(persona_content_hash(&persona_event_content(&new_def)).as_str()),
        "the restart consumed the new definition version"
    );
}

/// The launch backfill (`backfill_persona_snapshots`) and every local spawn
/// re-pin a definition-linked record through `apply_persona_snapshot`. The
/// definition quad (model, prompt, provider, runtime) is definition-
/// authoritative and is re-derived there by design. The agent's own access and
/// thinking level are not: a card Save writes them with `update_managed_agent`,
/// and they must survive both re-pins even when the definition's mint-time
/// access default differs.
#[test]
fn definition_repin_keeps_the_agents_access_and_thinking_level() {
    use crate::managed_agents::persona_events::apply_persona_snapshot;
    let mut def = persona("pers", Some("goose"), "prompt");
    def.model = Some("model-b".into());
    def.respond_to = Some("owner-only".into());
    let allowlist = vec!["a".repeat(64)];

    let mut rec = record();
    rec.persona_id = Some("pers".into());
    // As written by the card Save's update_managed_agent.
    rec.model = Some("model-b".into());
    rec.respond_to = RespondTo::Allowlist;
    rec.respond_to_allowlist = allowlist.clone();
    rec.effort_level = Some("high".into());

    apply_persona_snapshot(&mut rec, &def);
    // Launch backfill and a later spawn re-pin again: idempotent.
    apply_persona_snapshot(&mut rec, &def);

    assert_eq!(rec.respond_to, RespondTo::Allowlist);
    assert_eq!(rec.respond_to_allowlist, allowlist);
    assert_eq!(rec.effort_level.as_deref(), Some("high"));
    assert_eq!(rec.model.as_deref(), Some("model-b"));
}
