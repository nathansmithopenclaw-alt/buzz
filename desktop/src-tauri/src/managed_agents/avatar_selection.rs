//! Durable avatar intent. Whole-record saves and background reconciliation may
//! carry a selection forward, but only a direct edit creates a newer revision.

use super::ManagedAgentRecord;
use serde::{Deserialize, Serialize};

/// An owner-selected picture, including a durable explicit removal (`url: None`).
/// Revision zero is a conservative bootstrap of an existing saved picture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct AvatarSelection {
    pub revision: u64,
    pub edit_id: String,
    pub url: Option<String>,
}

/// Omission means Keep; clearing a form field must explicitly request Remove.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AvatarEdit {
    Keep,
    Set { url: String },
    Remove,
}

impl AvatarSelection {
    /// Create a monotonically ordered direct edit, even within one clock tick.
    pub fn edit(previous: Option<&Self>, edit: AvatarEdit) -> Result<Option<Self>, String> {
        let url = match edit {
            AvatarEdit::Keep => return Ok(previous.cloned()),
            AvatarEdit::Set { url } => {
                let url = url.trim();
                if url.is_empty() {
                    return Err("avatar URL must not be empty".into());
                }
                let parsed =
                    url::Url::parse(url).map_err(|e| format!("invalid avatar URL: {e}"))?;
                if (parsed.scheme() == "data" && !url.starts_with("data:image/"))
                    || !matches!(parsed.scheme(), "https" | "data")
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err("avatar must be an HTTPS image or an embedded image".into());
                }
                Some(url.to_string())
            }
            AvatarEdit::Remove => None,
        };
        let clock =
            u64::try_from(chrono::Utc::now().timestamp_millis()).map_err(|e| e.to_string())?;
        let successor = previous
            .map(|v| v.revision.checked_add(1).ok_or("avatar revision exhausted"))
            .transpose()?
            .unwrap_or(0);
        let revision = clock.max(successor);
        Ok(Some(Self {
            revision,
            edit_id: uuid::Uuid::new_v4().to_string(),
            url,
        }))
    }

    /// Empty string is the legacy-compatible tombstone; it blocks fallback to
    /// a runtime icon without confusing explicit removal with never initialized.
    pub fn stored_url(&self) -> Option<String> {
        Some(self.url.clone().unwrap_or_default())
    }
}

/// Preserve existing bytes on first upgrade; stock images are choices too.
pub fn bootstrap(record: &mut ManagedAgentRecord) {
    if record.avatar_selection.is_none() {
        if let Some(url) = &record.avatar_url {
            record.avatar_selection = Some(AvatarSelection {
                revision: 0,
                edit_id: "legacy".into(),
                url: (!url.is_empty()).then(|| url.clone()),
            });
        }
    }
    if let Some(selection) = &record.avatar_selection {
        record.avatar_url = selection.stored_url();
    }
}

/// Merge just the avatar edit, independent of a whole record's update time.
pub fn preserve_selection(incoming: &mut ManagedAgentRecord, current: &ManagedAgentRecord) {
    let mut current = current.clone();
    bootstrap(&mut current);
    if let Some(existing) = &current.avatar_selection {
        if incoming.avatar_selection.as_ref().is_none_or(|next| {
            (next.revision, &next.edit_id) <= (existing.revision, &existing.edit_id)
        }) {
            incoming.avatar_selection = Some(existing.clone());
        }
    }
    bootstrap(incoming);
    for (relay, edit) in &current.avatar_synced {
        let incoming_ack_current = incoming
            .avatar_selection
            .as_ref()
            .is_some_and(|s| incoming.avatar_synced.get(relay) == Some(&s.edit_id));
        if !incoming_ack_current {
            incoming.avatar_synced.insert(relay.clone(), edit.clone());
        }
    }
}

/// Apply storage protection and newer definition-scoped edits atomically.
pub fn prepare_store(records: &mut [ManagedAgentRecord], previous: &[ManagedAgentRecord]) {
    for record in records.iter_mut() {
        if let Some(old) = previous.iter().find(|old| {
            if record.pubkey.is_empty() {
                old.pubkey.is_empty() && old.slug == record.slug
            } else {
                old.pubkey == record.pubkey
            }
        }) {
            preserve_selection(record, old);
        } else {
            bootstrap(record);
        }
    }
    let definitions: Vec<_> = records
        .iter()
        .filter(|r| r.pubkey.is_empty())
        .cloned()
        .collect();
    for record in records.iter_mut().filter(|r| !r.pubkey.is_empty()) {
        if let Some(definition) = definitions
            .iter()
            .find(|d| record.persona_id.as_deref() == d.slug.as_deref())
        {
            // Bootstrap does not assert a definition-wide edit or resolve a
            // legacy conflict by guessing. Only an actual newer edit fans out.
            if definition
                .avatar_selection
                .as_ref()
                .is_some_and(|s| s.revision > 0)
            {
                preserve_selection(record, definition);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition(id: &str, url: &str) -> ManagedAgentRecord {
        let d: super::super::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": id, "display_name": id, "avatar_url": url, "system_prompt": "test",
            "name_pool": [], "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z"
        })).unwrap();
        d.into_agent_record()
    }
    fn set(record: &mut ManagedAgentRecord, url: Option<&str>) {
        bootstrap(record);
        record.avatar_selection = AvatarSelection::edit(
            record.avatar_selection.as_ref(),
            url.map_or(AvatarEdit::Remove, |url| AvatarEdit::Set {
                url: url.into(),
            }),
        )
        .unwrap();
        record.avatar_url = record.avatar_selection.as_ref().unwrap().stored_url();
    }
    #[test]
    fn stale_settings_save_cannot_replace_a_newer_avatar() {
        let mut old = definition("p", "https://example.com/a.png");
        bootstrap(&mut old);
        let mut current = old.clone();
        set(&mut current, Some("https://example.com/b.png"));
        old.name = "Renamed".into();
        old.agent_command = "new-runtime".into();
        let mut incoming = vec![old];
        prepare_store(&mut incoming, &[current.clone()]);
        assert_eq!(incoming[0].name, "Renamed");
        assert_eq!(incoming[0].avatar_selection, current.avatar_selection);
        assert_eq!(incoming[0].avatar_url, current.avatar_url);
    }
    #[test]
    fn explicit_removal_survives_serialization_restart_and_old_snapshot() {
        let old = definition("p", "https://example.com/a.png");
        let mut current = old.clone();
        set(&mut current, None);
        let reloaded: ManagedAgentRecord =
            serde_json::from_slice(&serde_json::to_vec(&current).unwrap()).unwrap();
        let mut incoming = vec![old];
        prepare_store(&mut incoming, &[reloaded]);
        assert_eq!(incoming[0].avatar_url.as_deref(), Some(""));
        assert_eq!(incoming[0].avatar_selection.as_ref().unwrap().url, None);
    }
    #[test]
    fn stock_artwork_is_preserved_without_url_classification() {
        let old = definition("p", "data:image/png;base64,AA==");
        let mut incoming = vec![old.clone()];
        incoming[0].avatar_url = Some("https://example.com/new-catalog.png".into());
        prepare_store(&mut incoming, &[old]);
        assert_eq!(
            incoming[0].avatar_url.as_deref(),
            Some("data:image/png;base64,AA==")
        );
    }
    #[test]
    fn definition_edit_fans_out_but_instance_edit_does_not_mutate_siblings() {
        let mut d = definition("p", "https://example.com/a.png");
        let mut one = d.clone();
        one.pubkey = "one".into();
        one.persona_id = Some("p".into());
        let mut two = one.clone();
        two.pubkey = "two".into();
        set(&mut d, Some("https://example.com/b.png"));
        let mut records = vec![d, one, two];
        prepare_store(&mut records, &[]);
        assert!(records
            .iter()
            .all(|r| r.avatar_url.as_deref() == Some("https://example.com/b.png")));
        let previous = records.clone();
        set(&mut records[1], Some("https://example.com/c.png"));
        prepare_store(&mut records, &previous);
        assert_eq!(
            records[1].avatar_url.as_deref(),
            Some("https://example.com/c.png")
        );
        assert_eq!(
            records[2].avatar_url.as_deref(),
            Some("https://example.com/b.png")
        );
        assert_eq!(records[0].avatar_url, records[2].avatar_url);
    }
    #[test]
    fn legacy_conflicting_instance_and_definition_are_not_silently_unified() {
        let d = definition("p", "https://example.com/definition.png");
        let mut instance = d.clone();
        instance.pubkey = "one".into();
        instance.persona_id = Some("p".into());
        instance.avatar_url = Some("https://example.com/instance.png".into());
        let mut records = vec![d, instance];
        prepare_store(&mut records, &[]);
        assert_ne!(records[0].avatar_url, records[1].avatar_url);
    }
    #[test]
    fn keep_does_not_create_an_edit_and_clear_does() {
        let mut d = definition("p", "https://example.com/a.png");
        bootstrap(&mut d);
        let current = d.avatar_selection.clone();
        assert_eq!(
            AvatarSelection::edit(current.as_ref(), AvatarEdit::Keep).unwrap(),
            current
        );
        let removed = AvatarSelection::edit(current.as_ref(), AvatarEdit::Remove)
            .unwrap()
            .unwrap();
        assert!(removed.revision > current.unwrap().revision);
        assert_eq!(removed.url, None);
    }
    #[test]
    fn new_selection_stays_pending_when_an_older_publish_is_acknowledged() {
        let mut d = definition("p", "https://example.com/a.png");
        set(&mut d, Some("https://example.com/b.png"));
        let mut stale = d.clone();
        set(&mut d, Some("https://example.com/c.png"));
        stale.avatar_synced.insert(
            "https://relay.example".into(),
            stale.avatar_selection.as_ref().unwrap().edit_id.clone(),
        );
        let mut incoming = vec![stale];
        prepare_store(&mut incoming, &[d.clone()]);
        assert_eq!(incoming[0].avatar_selection, d.avatar_selection);
        assert_ne!(
            incoming[0].avatar_synced.get("https://relay.example"),
            Some(&d.avatar_selection.unwrap().edit_id)
        );
    }
}
