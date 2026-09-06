use super::*;
use crate::{app_state::AppState, subscription_accounts};
use sha2::{Digest, Sha256};

pub(super) fn fingerprint(operation: &Operation) -> Result<String, String> {
    // Random journal identities are deliberately absent. Scope, saved binding
    // generations, current token mint and actual runtime nonces are compared.
    let members: Vec<_> = operation
        .members
        .iter()
        .map(|member| {
            let mut runtimes = member.child.runtimes.clone();
            runtimes.sort_by_key(|runtime| runtime.key.runtime_id());
            serde_json::json!({
                "pubkey":member.child.pubkey, "account":member.original_account_id,
                "generation":member.original_generation, "inherit":member.inherit_default,
                "mode_only":member.mode_only, "runtimes":runtimes,
            })
        })
        .collect();
    let value = serde_json::json!({"request":operation.request, "source":operation.source,
        "destination":operation.destination, "mint":operation.destination_minted_at,
        "default":operation.prior_default, "change_default":operation.change_default,
        "members":members});
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )))
}

fn ready_local_claude(
    record: &super::super::ManagedAgentRecord,
    personas: &[super::super::AgentDefinition],
) -> bool {
    let command = super::super::record_agent_command(record, personas);
    record.backend == super::super::BackendKind::Local
        && super::super::known_acp_runtime(&command).is_some_and(|runtime| runtime.id == "claude")
}

pub(super) fn capture(
    app: &AppHandle,
    request: ChangeRequest,
    expected: Option<&str>,
) -> Result<Operation, String> {
    let state = app.state::<AppState>();
    super::super::require_fleet_host(&state)?;
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    if read(app)?.is_some_and(|operation| operation.phase != "complete") {
        return Err("Finish or retry the current account change first".into());
    }
    let records = super::super::load_managed_agents(app)?;
    let personas = super::super::load_personas(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let _accounts = subscription_accounts::lifecycle_lock()
        .lock()
        .map_err(|e| e.to_string())?;
    let prior_default = subscription_accounts::defaults::selected(app)?;
    let (source, destination, change_default, remove) = match &request {
        ChangeRequest::Agent {
            pubkey,
            destination,
            inherit_default,
            all_on_current,
        } => {
            let record = records
                .iter()
                .find(|record| &record.pubkey == pubkey)
                .ok_or("Agent not found")?;
            if !ready_local_claude(record, &personas) {
                return Err("Choose a local Claude agent".into());
            }
            if *inherit_default && prior_default.as_deref() != Some(destination) {
                return Err("The default account changed. Choose Use default again.".into());
            }
            if *all_on_current && record.subscription_account_id.is_none() {
                return Err("All-agent switching requires a named current account".into());
            }
            let source = record.subscription_account_id.clone();
            let move_default = *all_on_current && source.is_some() && source == prior_default;
            (source, Some(destination.clone()), move_default, false)
        }
        ChangeRequest::Default { destination } => (
            prior_default.clone(),
            Some(destination.clone()),
            true,
            false,
        ),
        ChangeRequest::Remove {
            account_id,
            replacement,
        } => {
            let account = subscription_accounts::load_account(app, account_id)?;
            if account.harness != "claude" {
                return Err("This removal flow is for Claude accounts".into());
            }
            if account.attempt_id.is_some() || account.state == "connecting" {
                return Err("Finish or cancel account setup before removing it".into());
            }
            if replacement.as_deref() == Some(account_id) {
                return Err("Choose a different replacement account".into());
            }
            (
                Some(account_id.clone()),
                replacement.clone(),
                prior_default.as_deref() == Some(account_id),
                true,
            )
        }
    };
    let destination_minted_at = if let Some(id) = &destination {
        subscription_accounts::defaults::validate_destination(app, id)?;
        if !subscription_accounts::accounts_enabled(app)? {
            return Err("Enable Subscription Accounts to select a new account".into());
        }
        subscription_accounts::load_account(app, id)?.minted_at
    } else {
        None
    };
    let mut members = Vec::new();
    for record in &records {
        let included = match &request {
            ChangeRequest::Agent {
                pubkey,
                all_on_current,
                ..
            } => {
                &record.pubkey == pubkey
                    || (*all_on_current && record.subscription_account_id == source)
            }
            ChangeRequest::Default { .. } => record.subscription_account_inherit_default,
            ChangeRequest::Remove { account_id, .. } => {
                record.subscription_account_id.as_deref() == Some(account_id)
            }
        };
        if !included {
            continue;
        }
        if !ready_local_claude(record, &personas) {
            return Err(format!(
                "{} has an incompatible account binding; repair it before this change",
                record.name
            ));
        }
        let inherit_default = match &request {
            ChangeRequest::Agent {
                pubkey,
                inherit_default,
                ..
            } if &record.pubkey == pubkey => *inherit_default,
            ChangeRequest::Default { .. } => true,
            ChangeRequest::Remove {
                replacement: None, ..
            } => false,
            _ => record.subscription_account_inherit_default,
        };
        let (mode_only, child) = capture_child(
            app,
            record,
            destination
                .as_ref()
                .or(source.as_ref())
                .ok_or("Missing account")?,
            remove,
            &mut runtimes,
        )?;
        members.push(Member {
            name: record.name.clone(),
            original_account_id: record.subscription_account_id.clone(),
            original_generation: record.subscription_account_generation,
            inherit_default,
            mode_only,
            child,
            done: false,
            error: None,
        });
    }
    members.sort_by(|left, right| left.child.pubkey.cmp(&right.child.pubkey));
    let operation = Operation {
        version: 1,
        id: uuid::Uuid::new_v4().simple().to_string(),
        request,
        source,
        destination,
        destination_minted_at,
        prior_default,
        change_default,
        remove,
        phase: "applying".into(),
        error: None,
        members,
    };
    validate(&operation)?;
    if let Some(expected) = expected {
        if fingerprint(&operation)? != expected {
            return Err("Account state changed. Review the affected agents again.".into());
        }
        save(app, &operation)?;
    }
    Ok(operation)
}

/// Retry keeps the original membership even after some agents have moved. A new
/// destination captures fresh child operations while preserving explicit Stops.
pub(super) fn retarget(
    app: &AppHandle,
    operation: &mut Operation,
    destination: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = super::super::load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let _accounts = subscription_accounts::lifecycle_lock()
        .lock()
        .map_err(|e| e.to_string())?;
    subscription_accounts::defaults::validate_destination(app, destination)?;
    if operation.remove && operation.source.as_deref() == Some(destination) {
        return Err("Choose a different replacement account".into());
    }
    operation.destination = Some(destination.into());
    if operation.change_default {
        operation.prior_default = subscription_accounts::defaults::selected(app)?;
    }
    operation.destination_minted_at =
        subscription_accounts::load_account(app, destination)?.minted_at;
    for member in &mut operation.members {
        let record = records
            .iter()
            .find(|record| record.pubkey == member.child.pubkey)
            .ok_or("Group member no longer exists")?;
        member.original_account_id = record.subscription_account_id.clone();
        member.original_generation = record.subscription_account_generation;
        // A replacement chosen for one follower becomes an explicit override
        // unless this operation also changes the Mac's default.
        if !operation.change_default {
            member.inherit_default = false;
        }
        (member.mode_only, member.child) =
            capture_child(app, record, destination, false, &mut runtimes)?;
        member.done = false;
        member.error = None;
    }
    operation.phase = "applying".into();
    operation.error = None;
    save(app, operation)
}

fn capture_child(
    app: &AppHandle,
    record: &super::super::ManagedAgentRecord,
    destination: &str,
    force_switch: bool,
    runtimes: &mut std::collections::HashMap<
        super::super::ManagedAgentRuntimeKey,
        super::super::ManagedAgentPairRuntime,
    >,
) -> Result<(bool, account_switch::Operation), String> {
    let previous = account_switch::read(app, &record.pubkey)?;
    let mode_only = !force_switch
        && record.subscription_account_id.as_deref() == Some(destination)
        && record.last_error.is_none()
        && runtimes
            .iter()
            .filter(|(key, _)| key.pubkey == record.pubkey)
            .all(|(_, runtime)| {
                runtime.error.is_none()
                    && runtime.lifecycle != super::super::ManagedAgentRuntimeLifecycle::Failed
            })
        && previous
            .as_ref()
            .is_none_or(|op| op.phase == "complete" && op.error.is_none());
    let child = if mode_only {
        account_switch::Operation {
            id: uuid::Uuid::new_v4().simple().to_string(),
            pubkey: record.pubkey.clone(),
            account_id: destination.into(),
            generation: record.subscription_account_generation,
            phase: "complete".into(),
            error: None,
            runtimes: Vec::new(),
        }
    } else {
        super::super::subscription_binding::capture_operation(
            record,
            &Some(destination.into()),
            previous,
            runtimes,
        )?
    };
    Ok((mode_only, child))
}
