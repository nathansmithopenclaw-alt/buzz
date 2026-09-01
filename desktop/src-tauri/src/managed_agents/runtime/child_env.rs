use std::{collections::BTreeMap, process::Command};

const ACP_EFFORT_KEY: &str = "BUZZ_ACP_THINKING_EFFORT";
const ACP_EFFORT_DEFAULT_KEY: &str = "BUZZ_ACP_THINKING_EFFORT_IS_DEFAULT";

/// Log directives forced onto every managed-agent child, over whatever the
/// operator set.
///
/// `EnvFilter` has no implicit global level: an event whose target matches no
/// directive is dropped, silently. buzz-acp emits most of its diagnostics on
/// *bare* targets (`acp::background`, `pool::prompt`, `canvas::fetch`, …) that
/// share no prefix with the `buzz_acp` crate directive, so a crate-only filter
/// discards every warning and error they carry — including
/// `background_task_monitoring_unconfirmed`, the one warning that exists to say
/// a real background task may have gone untracked. A termination notice then
/// tells the operator to "inspect the local agent transcript/log" about events
/// the log was configured never to contain.
///
/// Directives are matched by string prefix, so one namespace entry covers every
/// bare target beneath it. The namespaces are pinned at `warn`, which cannot be
/// chatty by construction: it admits only what is already exceptional and
/// leaves the high-volume `debug`/`info` traffic (`acp::wire`, `acp::usage`,
/// `acp::thought`) muted. The individual targets that post-incident triage
/// always needs — background-task lifecycle, session lifecycle, model
/// application — are raised to `info` on top of that floor.
const CHILD_LOG_DIRECTIVES: &[&str] = &[
    "buzz_acp=info",
    // Warn floor for buzz-acp's bare-target namespaces.
    "acp=warn",
    "pool=warn",
    "canvas=warn",
    "engram=warn",
    "observer=warn",
    // Teardown/triage evidence, raised above the floor.
    "acp::background=info",
    "acp::session=info",
    "pool::session=info",
    "pool::model=info",
];

pub(super) fn child_rust_log_filter() -> String {
    child_rust_log_filter_from(std::env::var("RUST_LOG").ok().as_deref())
}

pub(super) fn apply_thinking_effort_provenance(
    command: &mut Command,
    persona_env: &BTreeMap<String, String>,
    agent_env: &BTreeMap<String, String>,
    global_env: &BTreeMap<String, String>,
) {
    // Never inherit or accept a forged marker. Desktop sets it only when the
    // effective generic effort came from the global default layer.
    command.env_remove(ACP_EFFORT_DEFAULT_KEY);
    if super::super::env_vars::env_var_comes_from_global_default(
        ACP_EFFORT_KEY,
        persona_env,
        agent_env,
        global_env,
    ) {
        command.env(ACP_EFFORT_DEFAULT_KEY, "true");
    }
}

pub(super) fn child_rust_log_filter_from(existing: Option<&str>) -> String {
    let mut directives = existing.unwrap_or_default().trim().to_string();
    for required in CHILD_LOG_DIRECTIVES {
        let target = directive_target(required);
        // Exact target match, not a prefix test. An operator directive is
        // allowed to override its own target and nothing else: under a prefix
        // test a supplied `acp::background=debug` also starts with `acp`, so it
        // would suppress the `acp=warn` floor for every *other* target in that
        // namespace — re-muting the diagnostics this list exists to restore.
        if !directives
            .split(',')
            .any(|entry| directive_target(entry) == target)
        {
            if !directives.is_empty() {
                directives.push(',');
            }
            directives.push_str(required);
        }
    }
    directives
}

/// Target portion of one `target=level` directive (a bare `warn` global-level
/// directive has no target part and compares as itself, matching nothing here).
fn directive_target(directive: &str) -> &str {
    directive
        .trim()
        .split('=')
        .next()
        .unwrap_or_default()
        .trim()
}
