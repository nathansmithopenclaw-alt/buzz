use super::*;
use crate::{app_state::AppState, subscription_accounts};

fn apply_member(app: &AppHandle, group: &Operation, member: &Member) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        if member.mode_only {
            let mut records = super::super::load_managed_agents(app)?;
            let record = super::super::find_managed_agent_mut(&mut records, &member.child.pubkey)?;
            if record.subscription_account_id != group.destination
                || record.subscription_account_generation != member.original_generation
            {
                return Err("The selected account changed; review this operation again".into());
            }
            record.subscription_account_inherit_default = member.inherit_default;
            record.updated_at = crate::util::now_iso();
            super::super::save_managed_agents(app, &records)?;
            return Ok(());
        }
        ensure_child(app, &member.child.pubkey)?;
        // Crash after the child committed, before the parent marked it done.
        // Do not start another switch or erase an explicit Stop in that gap.
        let child = account_switch::read(app, &member.child.pubkey)?
            .ok_or("Account change child missing")?;
        if group.destination.is_some() && child.phase == "complete" && child.error.is_none() {
            let records = super::super::load_managed_agents(app)?;
            if records.iter().any(|record| {
                record.pubkey == child.pubkey
                    && record.subscription_account_id == group.destination
                    && record.subscription_account_generation == child.generation
                    && record.subscription_account_inherit_default == member.inherit_default
            }) {
                return Ok(());
            }
        }
    }
    if group.destination.is_none() {
        super::super::subscription_binding::detach_account(app, &member.child.pubkey, &group.id)
    } else {
        let result = super::super::subscription_binding::switch_account_scoped(
            member.child.pubkey.clone(),
            group.destination.clone(),
            app.clone(),
            None,
            Some(&group.id),
            Some(member.inherit_default),
        )?;
        if result.failed_restart_count > 0 || !result.errors.is_empty() {
            return Err(result.errors.join("; "));
        }
        Ok(())
    }
}

pub(super) fn run(app: &AppHandle, mut operation: Operation) -> Result<Status, String> {
    let state = app.state::<AppState>();
    if !run_members(
        &mut operation,
        || {
            state
                .shutdown_started
                .load(std::sync::atomic::Ordering::Acquire)
        },
        |operation, member| apply_member(app, operation, member),
        |operation| save(app, operation),
    )? {
        return Ok(operation.status());
    }
    let finalization = (|| -> Result<(), String> {
        if operation.change_default {
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;
            subscription_accounts::defaults::commit(
                app,
                &operation.harness,
                operation.prior_default.as_deref(),
                operation.destination.as_deref(),
            )?;
        }
        if operation.remove {
            operation.phase = "removing".into();
            save(app, &operation)?;
            let id = operation
                .source
                .as_deref()
                .ok_or("Missing account to remove")?;
            if subscription_accounts::defaults::account_exists(app, id)? {
                subscription_accounts::remove_account_scoped(id, app, Some(&operation.id))?;
            }
        }
        Ok(())
    })();
    match finalization {
        Ok(()) => {
            operation.phase = "complete".into();
            operation.error = None;
        }
        Err(error) => {
            operation.phase = "paused".into();
            operation.error = Some(error);
        }
    }
    save(app, &operation)?;
    Ok(operation.status())
}

/// A saved prefix is the recovery boundary. Persistence failures propagate;
/// finalization is unreachable while any member is unfinished.
pub(super) fn run_members(
    operation: &mut Operation,
    shutdown: impl Fn() -> bool,
    mut apply: impl FnMut(&Operation, &Member) -> Result<(), String>,
    mut persist: impl FnMut(&Operation) -> Result<(), String>,
) -> Result<bool, String> {
    for index in 0..operation.members.len() {
        if operation.members[index].done {
            continue;
        }
        if shutdown() {
            persist(operation)?;
            return Ok(false);
        }
        let result = apply(operation, &operation.members[index]);
        if shutdown() {
            operation.phase = "applying".into();
            operation.error = None;
            persist(operation)?;
            return Ok(false);
        }
        match result {
            Ok(()) => {
                operation.members[index].done = true;
                operation.members[index].error = None;
            }
            Err(error) => {
                operation.members[index].error = Some(error.chars().take(1000).collect());
            }
        }
        persist(operation)?;
    }
    if operation.members.iter().any(|member| !member.done) {
        operation.phase = "paused".into();
        operation.error = Some("Some agents could not finish changing accounts. Their work is retained; retry or choose another account.".into());
        persist(operation)?;
        return Ok(false);
    }
    Ok(true)
}
