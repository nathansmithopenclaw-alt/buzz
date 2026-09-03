//! Tests for `propagate_persona_name_rename` — the helper that propagates a
//! persona definition's display_name change to linked agent instances.

use super::*;

fn agent(persona_id: &str, name: &str, display_name: Option<&str>) -> ManagedAgentRecord {
    ManagedAgentRecord {
        description: None,
        pubkey: format!("pubkey-{name}"),
        name: name.to_string(),
        persona_id: Some(persona_id.to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        idle_pool_sleep_seconds: None,
        max_turns_per_session: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        claude_account_id: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: display_name.map(str::to_string),
        slug: None,
        runtime: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: vec![],
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
    }
}

#[test]
fn test_rename_propagates_to_matching_instance() {
    // An instance whose `name` equals the OLD persona display_name must get
    // both `name` and `display_name` updated to the new value.
    let mut records = vec![agent("persona-1", "Paul", Some("Paul"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string()],
        "must report the renamed record's pubkey"
    );
    assert_eq!(records[0].name, "Paul Atreides", "name must be updated");
    assert_eq!(
        records[0].display_name,
        Some("Paul Atreides".to_string()),
        "display_name must be updated"
    );
    // The relay-profile sync params use `record.name`; after rename it carries
    // the new display_name, so the relay profile will be published with the correct name.
    assert_eq!(records[0].name, "Paul Atreides");
}

#[test]
fn test_rename_skips_pool_named_instance() {
    // A pool-named instance (e.g. "Birch") has a name DIFFERENT from the
    // persona display_name. It must keep its individualised name.
    let mut records = vec![agent("persona-1", "Birch", Some("Birch"))];

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert!(
        renamed.is_empty(),
        "pool-named instance must not be reported as renamed"
    );
    assert_eq!(records[0].name, "Birch", "pool name must be preserved");
    assert_eq!(
        records[0].display_name,
        Some("Birch".to_string()),
        "pool display_name must be preserved"
    );
}

#[test]
fn test_rename_propagates_both_name_and_display_name() {
    // Explicit dual-field check: BOTH `name` and `display_name` must be
    // updated so the relay profile and the local UI are consistent.
    let mut records = vec![agent("persona-1", "OldName", None)];

    propagate_persona_name_rename(&mut records, "persona-1", "OldName", "NewName");

    assert_eq!(records[0].name, "NewName");
    assert_eq!(records[0].display_name, Some("NewName".to_string()));
}

#[test]
fn test_rename_only_affects_linked_persona() {
    // An instance linked to a DIFFERENT persona must not be touched, even
    // if it happens to carry the same display_name.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-2", "Paul", Some("Paul")),
    ];

    propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Paul Atreides");

    assert_eq!(records[0].name, "Paul Atreides", "linked instance renamed");
    assert_eq!(
        records[1].name, "Paul",
        "unrelated persona's instance untouched"
    );
}

#[test]
fn description_only_update_syncs_without_mutating_record_and_preserves_legacy_persona_avatar() {
    let mut record = agent("persona-1", "Paul", Some("Paul"));
    record.avatar_url = None;
    record.slug = Some("persona-1".to_string());
    let before = record.clone();
    let mut persona = record
        .clone()
        .to_definition_view()
        .expect("test record projects to a definition");
    persona.id = "persona-1".to_string();
    persona.avatar_url = Some("https://example.com/paul.png".to_string());

    let update = prepare_linked_profile_update(&mut record, &persona, false, false, true);

    assert!(update.profile_sync_required, "about-only edits must sync");
    assert!(
        !update.record_changed,
        "about-only edits must not write the agent store"
    );
    assert_eq!(
        record, before,
        "description-only edits leave instance bytes untouched"
    );
    assert_eq!(
        update.profile_avatar.as_deref(),
        Some("https://example.com/paul.png"),
        "complete kind:0 replacement must not clear a legacy agent avatar"
    );
}

#[test]
fn unchanged_identity_needs_neither_store_write_nor_profile_sync() {
    let mut record = agent("persona-1", "Paul", Some("Paul"));
    record.slug = Some("persona-1".to_string());
    let persona = record
        .clone()
        .to_definition_view()
        .expect("test record projects to a definition");

    let update = prepare_linked_profile_update(&mut record, &persona, false, false, false);

    assert!(!update.record_changed);
    assert!(!update.profile_sync_required);
}

#[test]
fn test_rename_renames_all_matching_instances_in_one_pass() {
    // Several instances may carry the definition name (multi-instance deploys
    // without a name pool): one call renames every match and reports each
    // pubkey, which is what the relay profile sync collection keys on.
    let mut records = vec![
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Paul", Some("Paul")),
        agent("persona-1", "Birch", Some("Birch")),
    ];
    records[1].pubkey = "pubkey-Paul-2".to_string();

    let renamed = propagate_persona_name_rename(&mut records, "persona-1", "Paul", "Duncan Idaho");

    assert_eq!(
        renamed,
        vec!["pubkey-Paul".to_string(), "pubkey-Paul-2".to_string()],
        "every matching instance's pubkey must be reported"
    );
    assert_eq!(records[0].name, "Duncan Idaho");
    assert_eq!(records[1].name, "Duncan Idaho");
    assert_eq!(records[2].name, "Birch", "pool-named instance untouched");
}

#[test]
fn delegated_env_unblocks_running_setup_mode_agent_for_restart() {
    assert_eq!(
        readiness_transition_action(true, true, false, true, true, true),
        Some(ReadinessTransitionAction::Restart)
    );
}

#[test]
fn delegated_env_unblocks_stopped_active_agent_for_start() {
    assert_eq!(
        readiness_transition_action(true, true, false, true, false, false),
        Some(ReadinessTransitionAction::Start),
        "an active linked agent must not stay silently stopped after its missing env arrives"
    );
}

#[test]
fn delegated_env_does_not_resurrect_archived_agent() {
    assert_eq!(
        readiness_transition_action(true, false, false, true, false, false),
        None
    );
}

#[test]
fn delegated_env_does_not_bounce_non_setup_runtime() {
    assert_eq!(
        readiness_transition_action(true, true, false, true, true, false),
        None,
        "a live non-setup runtime is outside readiness-transition candidacy"
    );
}

#[test]
fn delegated_env_requires_an_actual_not_ready_to_ready_transition() {
    assert_eq!(
        readiness_transition_action(true, true, true, true, true, true),
        None
    );
    assert_eq!(
        readiness_transition_action(true, true, false, false, false, false),
        None
    );
}

#[test]
fn delegated_allowlist_patch_reaches_the_real_spawn_env_builder() {
    let allowed = "ab".repeat(32);
    let mut records = vec![agent("persona-1", "Emberproof", Some("Emberproof"))];
    records[0].pubkey = "cd".repeat(32);
    records[0].respond_to = RespondTo::OwnerOnly;
    records[0].respond_to_allowlist = Vec::new();
    records[0].parallelism = 7;

    let changed = apply_linked_instance_behavior_patch(
        &mut records,
        "persona-1",
        &LinkedInstanceBehaviorPatch {
            respond_to: Some(RespondTo::Allowlist),
            respond_to_allowlist: Some(vec![allowed.to_uppercase()]),
            parallelism: None,
        },
    )
    .unwrap();

    assert_eq!(changed, vec!["cd".repeat(32)]);
    assert_eq!(records[0].parallelism, 7, "unrequested field preserved");
    let (set, remove) = crate::managed_agents::access_policy::build_respond_to_env_with_policy(
        &records[0],
        Some("owner"),
        false,
    )
    .unwrap();
    assert_eq!(
        set,
        vec![
            ("BUZZ_ACP_RESPOND_TO", "allowlist".to_string()),
            ("BUZZ_ACP_RESPOND_TO_ALLOWLIST", allowed),
            ("BUZZ_ACP_AGENT_OWNER", "owner".to_string()),
        ]
    );
    assert_eq!(remove, vec!["BUZZ_ACP_ALLOWED_RESPOND_TO"]);
}

#[test]
fn delegated_access_patch_updates_only_linked_instances() {
    let mut records = vec![
        agent("persona-1", "One", Some("One")),
        agent("persona-1", "Two", Some("Two")),
        agent("persona-2", "Other", Some("Other")),
    ];
    records[0].respond_to_allowlist = vec!["aa".repeat(32)];
    records[1].respond_to_allowlist = vec!["bb".repeat(32)];
    records[2].respond_to = RespondTo::Allowlist;
    records[2].respond_to_allowlist = vec!["cc".repeat(32)];

    let changed = apply_linked_instance_behavior_patch(
        &mut records,
        "persona-1",
        &LinkedInstanceBehaviorPatch {
            respond_to: Some(RespondTo::Anyone),
            respond_to_allowlist: None,
            parallelism: None,
        },
    )
    .unwrap();

    assert_eq!(changed.len(), 2);
    assert!(records[..2]
        .iter()
        .all(|record| record.respond_to == RespondTo::Anyone));
    assert!(records[..2]
        .iter()
        .all(|record| record.respond_to_allowlist.is_empty()));
    assert_eq!(records[2].respond_to, RespondTo::Allowlist);
    assert_eq!(records[2].respond_to_allowlist, vec!["cc".repeat(32)]);
}
