//! Serialize kind:0 writes and source their picture from durable avatar intent.
use super::{avatar_selection::AvatarSelection, load_managed_agents, save_managed_agents};
use crate::app_state::AppState;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
};

type PublishLock = tokio::sync::Mutex<()>;
static LOCKS: LazyLock<Mutex<HashMap<String, Weak<PublishLock>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-agent/per-relay ordering, with unused lock entries reclaimed on access.
pub fn publication_lock(relay: &str, pubkey: &str) -> Result<Arc<PublishLock>, String> {
    let mut locks = LOCKS.lock().map_err(|e| e.to_string())?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    let key = format!("{}:{pubkey}", crate::relay::relay_http_base_url(relay));
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(PublishLock::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

/// Re-read under the store lock after all asynchronous profile reads complete.
pub fn current_selection(
    state: &AppState,
    pubkey: &str,
) -> Result<Option<AvatarSelection>, String> {
    let app = state.app_handle.lock().map_err(|e| e.to_string())?.clone();
    let Some(app) = app else {
        return Ok(None);
    };
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(load_managed_agents(&app)?
        .into_iter()
        .find(|r| r.pubkey == pubkey)
        .and_then(|r| r.avatar_selection))
}

/// Initialize a genuinely unset legacy identity once, without replacing an edit
/// that landed while the relay query was pending.
pub fn initialize_selection(
    state: &AppState,
    pubkey: &str,
    fallback: Option<String>,
) -> Result<Option<AvatarSelection>, String> {
    let app = state.app_handle.lock().map_err(|e| e.to_string())?.clone();
    let Some(app) = app else {
        return Ok(None);
    };
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let Some(record) = records.iter_mut().find(|r| r.pubkey == pubkey) else {
        return Ok(None);
    };
    if record.avatar_selection.is_some() {
        return Ok(record.avatar_selection.clone());
    }
    record.avatar_url = fallback;
    super::avatar_selection::bootstrap(record);
    let selection = record.avatar_selection.clone();
    save_managed_agents(&app, &records)?;
    Ok(selection)
}

/// Acknowledge only the exact selection that was published. A newer edit remains pending.
pub fn mark_synced(
    state: &AppState,
    relay: &str,
    pubkey: &str,
    selection: &AvatarSelection,
) -> Result<(), String> {
    let app = state.app_handle.lock().map_err(|e| e.to_string())?.clone();
    let Some(app) = app else {
        return Ok(());
    };
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    if let Some(record) = records.iter_mut().find(|r| r.pubkey == pubkey) {
        if record.avatar_selection.as_ref() == Some(selection) {
            record.avatar_synced.insert(
                crate::relay::relay_http_base_url(relay),
                selection.edit_id.clone(),
            );
            save_managed_agents(&app, &records)?;
        }
    }
    Ok(())
}

/// Retry durable selections on the existing thirty-second owner flush loop.
/// Failed sends keep the selection unacknowledged across process restart.
pub async fn flush_pending(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    if super::require_fleet_host(state).is_err() {
        return Ok(());
    }
    let records = {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        load_managed_agents(app)?
    };
    let workspace = crate::relay::relay_ws_url_with_override(state);
    let mut failures = Vec::new();
    for record in records {
        let Some(selection) = &record.avatar_selection else {
            continue;
        };
        let relay = crate::relay::effective_agent_relay_url(&record.relay_url, &workspace);
        if record
            .avatar_synced
            .get(&crate::relay::relay_http_base_url(&relay))
            == Some(&selection.edit_id)
        {
            continue;
        }
        let keys = match nostr::Keys::parse(&record.private_key_nsec) {
            Ok(keys) => keys,
            Err(_) => {
                failures.push(format!(
                    "avatar publication pending for {}: signing key unavailable",
                    record.pubkey
                ));
                continue;
            }
        };
        // Avatar-only retry must preserve agent-managed name/about/extra fields.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::relay::sync_agent_managed_profile_avatar(
                state,
                &relay,
                &keys,
                &record.name,
                selection.url.as_deref().unwrap_or(""),
                record.auth_tag.as_deref(),
            ),
        )
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!(
                "avatar publication pending for {}: {e}",
                record.pubkey
            )),
            Err(_) => failures.push(format!(
                "avatar publication pending for {}: timeout",
                record.pubkey
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Production ordering seam: perform the asynchronous read first, then capture
/// the durable edit under the per-agent publisher lock and publish that edit.
/// Callers retain pending state until this returns successfully.
pub async fn publish_current<Q, QF, R, S, SF>(
    relay: &str,
    pubkey: &str,
    query: Q,
    read: R,
    send: S,
) -> Result<Option<AvatarSelection>, String>
where
    Q: FnOnce() -> QF,
    QF: std::future::Future<Output = Result<Option<crate::relay::AgentProfileInfo>, String>>,
    R: FnOnce(Option<&crate::relay::AgentProfileInfo>) -> Result<Option<AvatarSelection>, String>,
    S: FnOnce(Option<crate::relay::AgentProfileInfo>, Option<AvatarSelection>) -> SF,
    SF: std::future::Future<Output = Result<(), String>>,
{
    let lock = publication_lock(relay, pubkey)?;
    let _guard = lock.lock().await;
    let latest = query().await?;
    let selection = read(latest.as_ref())?;
    send(latest, selection.clone()).await?;
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    fn selection(id: &str) -> AvatarSelection {
        AvatarSelection {
            revision: 1,
            edit_id: id.into(),
            url: Some(format!("https://example.com/{id}.png")),
        }
    }

    #[tokio::test]
    async fn paused_old_query_publishes_the_new_direct_edit() {
        let stored = Arc::new(Mutex::new(selection("a")));
        let querying = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let observed = Arc::new(Mutex::new(None));
        let task = tokio::spawn({
            let stored = stored.clone();
            let querying = querying.clone();
            let resume = resume.clone();
            let observed = observed.clone();
            async move {
                publish_current(
                    "https://race.example",
                    "agent",
                    || async {
                        querying.notify_one();
                        resume.notified().await;
                        Ok(None)
                    },
                    |_| Ok(Some(stored.lock().unwrap().clone())),
                    |_, avatar| async move {
                        *observed.lock().unwrap() = avatar;
                        Ok(())
                    },
                )
                .await
            }
        });
        querying.notified().await;
        *stored.lock().unwrap() = selection("b");
        resume.notify_one();
        assert_eq!(task.await.unwrap().unwrap(), Some(selection("b")));
        assert_eq!(*observed.lock().unwrap(), Some(selection("b")));
    }

    #[tokio::test]
    async fn failed_send_is_not_an_acknowledgement_and_does_not_block_retry() {
        let result = publish_current(
            "https://failure.example",
            "agent",
            || async { Ok(None) },
            |_| Ok(Some(selection("b"))),
            |_, _| async { Err("offline".into()) },
        )
        .await;
        assert_eq!(result, Err("offline".into()));
        let result = publish_current(
            "https://failure.example",
            "agent",
            || async { Ok(None) },
            |_| Ok(Some(selection("b"))),
            |_, _| async { Ok(()) },
        )
        .await;
        assert_eq!(result.unwrap(), Some(selection("b")));
    }

    #[tokio::test]
    async fn publication_order_is_per_identity_and_per_relay() {
        let lock = publication_lock("https://order.example", "agent").unwrap();
        let _held = lock.lock().await;
        assert!(publication_lock("https://order.example/", "agent")
            .unwrap()
            .try_lock()
            .is_err());
        assert!(publication_lock("https://other.example", "agent")
            .unwrap()
            .try_lock()
            .is_ok());
        assert!(publication_lock("https://order.example", "other")
            .unwrap()
            .try_lock()
            .is_ok());
    }
}
