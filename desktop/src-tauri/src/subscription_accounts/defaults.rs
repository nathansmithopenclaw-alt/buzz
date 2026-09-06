//! The local Claude default. Existing agents retain explicit bindings on upgrade.
use tauri::AppHandle;

pub(crate) fn selected(app: &AppHandle) -> Result<Option<String>, String> {
    Ok(super::store::load(&super::store::root(app)?)?.claude_default_account_id)
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
    expected: Option<&str>,
    destination: Option<&str>,
) -> Result<(), String> {
    let _guard = super::lifecycle_lock()
        .lock()
        .map_err(|_| "Account lifecycle unavailable")?;
    let root = super::store::root(app)?;
    let mut registry = super::store::load(&root)?;
    if registry.claude_default_account_id.as_deref() != expected
        && registry.claude_default_account_id.as_deref() != destination
    {
        return Err("The default account changed; review the operation again".into());
    }
    if let Some(id) = destination {
        validate_destination(app, id)?;
    }
    registry.schema_version = 3;
    registry.claude_default_account_id = destination.map(str::to_owned);
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
    if crate::managed_agents::known_acp_runtime(&command)
        .is_none_or(|runtime| runtime.id != "claude")
    {
        return Ok(());
    }
    let Some(id) = selected(app)? else {
        return Ok(());
    };
    crate::managed_agents::account_group::guard_new_default(app, &id)?;
    validate_destination(app, &id)?;
    record.subscription_account_id = Some(id);
    record.subscription_account_inherit_default = true;
    record.subscription_account_generation = 1;
    record.claude_account_id = Some(crate::managed_agents::CREDENTIALS_FILE_ACCOUNT_ID.into());
    Ok(())
}
