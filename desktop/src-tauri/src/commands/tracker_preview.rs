use super::*;

fn preview_values(
    mut entry: TrackerFoldCacheEntry,
    membership_event: Value,
    key: &TrackerFoldCacheKey,
) -> Result<Value, String> {
    let membership: Event = serde_json::from_value(membership_event.clone())
        .map_err(|error| format!("invalid membership snapshot: {error}"))?;
    membership.verify().map_err(|error| error.to_string())?;
    // The cache never grants access. Current relay membership must still admit
    // this viewer, and a changed relay signer requires a full fresh read.
    if entry.membership_event["pubkey"] != membership_event["pubkey"]
        || authorization_from_membership(&membership)
            .role(&key.managed_signer)
            .is_none()
    {
        return Err("Dispatch cache requires current workspace membership".into());
    }
    let snapshot_at = entry
        .cursor
        .ok_or("Dispatch cache has no complete snapshot")?;
    entry.merge(Vec::new(), Vec::new(), Some(membership_event));
    let mut value = fold_tracker_context_values(
        entry.events.into_values().collect(),
        entry.task_events.into_values().collect(),
        entry.membership_event,
        &key.workspace_id,
        &key.managed_signer,
        snapshot_at,
    )?;
    // A preview is useful for reading, never an authority for a new edit.
    value["viewerRole"] = json!("none");
    value["sectionErrors"] = json!({
        "comments": null, "projects": null,
        "tasks": "Refreshing linked execution history…",
    });
    Ok(value)
}

/// Read a scoped local preview using a freshly fetched membership snapshot.
/// The preview never advances cursors or overwrites accepted native history.
#[tauri::command]
pub fn read_tracker_context_cache(
    workspace_id: String,
    managed_signer: String,
    membership_event: Value,
    cache: State<'_, TrackerFoldCache>,
    state: State<'_, AppState>,
) -> Result<Option<Value>, String> {
    let key = active_cache_key(&state, &workspace_id, &managed_signer)?;
    cache
        .get(&key)?
        .map(|entry| preview_values(entry, membership_event, &key))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn membership(relay: &Keys, workspace: &str, owner: &Keys, viewer: Option<&Keys>) -> Value {
        let mut tags = vec![
            Tag::parse(["d", workspace]).unwrap(),
            Tag::parse(["p", &owner.public_key().to_hex(), "", "owner"]).unwrap(),
        ];
        if let Some(viewer) = viewer {
            tags.push(Tag::parse(["p", &viewer.public_key().to_hex()]).unwrap());
        }
        serde_json::to_value(
            EventBuilder::new(Kind::Custom(39002), "")
                .tags(tags)
                .sign_with_keys(relay)
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn preview_checks_fresh_membership_and_never_grants_write_authority() {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let viewer = Keys::generate();
        let key = cache_key(
            "wss://relay.example",
            &Uuid::new_v4().to_string(),
            &viewer.public_key().to_hex(),
        )
        .unwrap();
        let admitted = membership(&relay, &key.workspace_id, &owner, Some(&viewer));
        let entry =
            TrackerFoldCacheEntry::new(&key, Some(100), Vec::new(), Vec::new(), admitted.clone());
        let preview = preview_values(entry.clone(), admitted, &key).unwrap();
        assert_eq!(preview["viewerRole"], "none");
        assert_eq!(entry.cursor, Some(100));
        let removed = membership(&relay, &key.workspace_id, &owner, None);
        assert!(preview_values(entry.clone(), removed, &key).is_err());
        let foreign = membership(&relay, &Uuid::new_v4().to_string(), &owner, Some(&viewer));
        assert!(preview_values(entry.clone(), foreign, &key).is_err());
        let changed_signer =
            membership(&Keys::generate(), &key.workspace_id, &owner, Some(&viewer));
        assert!(preview_values(entry, changed_signer, &key).is_err());
    }
}
