//! Local provider defaults. Existing agents retain explicit bindings on upgrade.
use tauri::AppHandle;

pub(crate) fn selected(app: &AppHandle) -> Result<Option<String>, String> {
    selected_for(app, "claude")
}

pub(crate) fn selected_for(app: &AppHandle, harness: &str) -> Result<Option<String>, String> {
    let registry = super::store::load(&super::store::root(app)?)?;
    match harness {
        "claude" => Ok(registry.claude_default_account_id),
        "codex" => Ok(registry.codex_default_account_id),
        "grok" => Ok(None),
        _ => Err("This harness does not support subscription defaults".into()),
    }
}

#[tauri::command]
pub(crate) fn get_subscription_default_account(
    app: AppHandle,
    harness: String,
) -> Result<Option<String>, String> {
    selected_for(&app, &harness)
}

pub(crate) fn validate_for(app: &AppHandle, id: &str, harness: &str) -> Result<(), String> {
    let account = super::load_account(app, id)?;
    if account.harness != harness || !matches!(harness, "claude" | "codex") {
        return Err("Choose an account for this provider".into());
    }
    if harness == "claude" {
        return validate_destination(app, id);
    }
    if account.provider != "openai"
        || account.state != "ready"
        || account
            .nickname
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
    {
        return Err("Choose a ready, named OpenAI account".into());
    }
    super::account_home(app, id)?;
    Ok(())
}

pub(crate) fn account_exists(app: &AppHandle, id: &str) -> Result<bool, String> {
    Ok(super::store::load(&super::store::root(app)?)?
        .accounts
        .iter()
        .any(|account| account.id == id))
}

#[tauri::command]
pub(crate) fn get_claude_default_account(app: AppHandle) -> Result<Option<String>, String> {
    selected(&app)
}

pub(crate) fn validate_destination(app: &AppHandle, id: &str) -> Result<(), String> {
    let account = super::load_account(app, id)?;
    if account.harness != "claude"
        || account.kind != "token"
        || account.state != "ready"
        || account
            .nickname
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
        || account
            .expires_at
            .is_none_or(|at| at <= chrono::Utc::now().timestamp())
    {
        return Err("Choose a ready, unexpired Claude token account".into());
    }
    super::account_home(app, id)?;
    Ok(())
}

/// Only a completed durable group operation commits a changed default.
pub(crate) fn commit(
    app: &AppHandle,
    harness: &str,
    expected: Option<&str>,
    destination: Option<&str>,
) -> Result<(), String> {
    let _guard = super::lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = super::store::root(app)?;
    let mut registry = super::store::load(&root)?;
    let current = match harness {
        "claude" => &mut registry.claude_default_account_id,
        "codex" => &mut registry.codex_default_account_id,
        _ => return Err("This provider does not support account defaults yet".into()),
    };
    if current.as_deref() != expected && current.as_deref() != destination {
        return Err("The default account changed; review the operation again".into());
    }
    if let Some(id) = destination {
        validate_for(app, id, harness)?;
    }
    *current = destination.map(str::to_owned);
    registry.schema_version = registry
        .schema_version
        .max(if harness == "codex" { 4 } else { 3 });
    super::store::save(&root, &registry)
}

/// Called inside the new agent's existing store transaction before first save.
pub(crate) fn initialize_agent(
    app: &AppHandle,
    record: &mut crate::managed_agents::ManagedAgentRecord,
    personas: &[crate::managed_agents::AgentDefinition],
) -> Result<(), String> {
    if record.backend != crate::managed_agents::BackendKind::Local {
        return Ok(());
    }
    let command = crate::managed_agents::record_agent_command(record, personas);
    let harness = crate::managed_agents::known_acp_runtime(&command).map(|runtime| runtime.id);
    let Some(harness) = harness.filter(|id| matches!(*id, "claude" | "codex")) else {
        return Ok(());
    };
    let Some(id) = selected_for(app, harness)? else {
        return Ok(());
    };
    crate::managed_agents::account_group::guard_new_default(app, &id)?;
    validate_for(app, &id, harness)?;
    record.subscription_account_id = Some(id);
    record.subscription_account_inherit_default = true;
    record.subscription_account_generation = 1;
    record.claude_account_id = Some(crate::managed_agents::CREDENTIALS_FILE_ACCOUNT_ID.into());
    Ok(())
}
