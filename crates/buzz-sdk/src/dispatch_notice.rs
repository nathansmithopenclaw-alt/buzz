//! Explicit human-authored conversation readback. These messages carry links,
//! never authority to change issue status or establish independent verification.
use nostr::{Event, EventBuilder, EventId, Tag};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// Identity of one explicitly shared change, independent of message prose.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DispatchNotice {
    /// Dispatch workspace.
    pub workspace: Uuid,
    /// Linked issue.
    pub issue: Uuid,
    /// Original message, when sharing a reply to the source conversation.
    pub source_event: Option<String>,
    /// Exact issue, task or decision event this update describes.
    pub change_event: String,
    /// Human-selected reason for notifying the conversation.
    pub purpose: String,
}
impl DispatchNotice {
    /// Stable across authors, devices, retries and process restarts.
    pub fn key(&self, destination: Uuid) -> Result<String, String> {
        if self.workspace.is_nil()
            || self.issue.is_nil()
            || destination.is_nil()
            || !matches!(
                self.purpose.as_str(),
                "tracking"
                    | "status"
                    | "blocked"
                    | "completion"
                    | "decision_requested"
                    | "decision_resolved"
            )
        {
            return Err("Invalid Dispatch update scope or purpose".into());
        }
        EventId::from_hex(&self.change_event).map_err(|e| e.to_string())?;
        if let Some(source) = &self.source_event {
            EventId::from_hex(source).map_err(|e| e.to_string())?;
        }
        use nostr::hashes::{sha256, Hash};
        Ok(sha256::Hash::hash(
            format!(
                "dispatch-notice-v1:{}:{}:{}:{}:{}:{}",
                self.workspace,
                self.issue,
                destination,
                self.source_event.as_deref().unwrap_or(""),
                self.change_event,
                self.purpose
            )
            .as_bytes(),
        )
        .to_string())
    }
}
/// Whether the reserved message marker is present (including malformed claims).
pub fn is_dispatch_notice(event: &Event) -> bool {
    event.kind.as_u16() == 9
        && event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", "dispatch-update"])
}
/// Build one visible message for one meaningful change; no autonomous wake.
pub fn build_dispatch_notice(
    destination: Uuid,
    notice: &DispatchNotice,
    body: &str,
    thread: Option<&crate::ThreadRef>,
    mentions: &[&str],
) -> Result<EventBuilder, String> {
    if body.trim().is_empty() || body.len() > 16384 || mentions.len() > 1 {
        return Err(
            "A Dispatch update requires a bounded message and at most one explicit recipient"
                .into(),
        );
    }
    let key = notice.key(destination)?;
    let mut tags = vec![
        Tag::parse(["t", "dispatch-update"]).map_err(|error| error.to_string())?,
        Tag::parse(["d", &key]).map_err(|error| error.to_string())?,
        Tag::parse([
            "dispatch",
            &notice.workspace.to_string(),
            &notice.issue.to_string(),
            &notice.change_event,
            &notice.purpose,
        ])
        .map_err(|error| error.to_string())?,
    ];
    if let Some(source) = &notice.source_event {
        tags.push(Tag::parse(["q", source]).map_err(|error| error.to_string())?);
    }
    crate::build_message(destination, body, thread, mentions, false, &[], &[])
        .map(|builder| builder.tags(tags))
        .map_err(|e| e.to_string())
}
/// Validate the idempotency key and metadata, without treating message prose as
/// authoritative task or issue state. Readers resolve the linked current record.
pub fn parse_dispatch_notice(event: &Event) -> Result<(Uuid, DispatchNotice, String), String> {
    if event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["t", "buzz-task"])
        || !is_dispatch_notice(event)
        || event.content.len() > 16384
        || event.content.trim().is_empty()
    {
        return Err("Invalid Dispatch update message".into());
    }
    let single = |name: &str| -> Result<Option<Vec<String>>, String> {
        let tags = event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
            .collect::<Vec<_>>();
        if tags.len() > 1 {
            return Err(format!("Duplicate {name} tag"));
        }
        Ok(tags.first().map(|tag| tag.as_slice().to_vec()))
    };
    let h = single("h")?.ok_or("Missing destination")?;
    let descriptor = single("dispatch")?.ok_or("Missing Dispatch reference")?;
    let d = single("d")?.ok_or("Missing notification identity")?;
    let source = single("q")?;
    if h.len() != 2
        || descriptor.len() != 5
        || d.len() != 2
        || source.as_ref().is_some_and(|tag| tag.len() != 2)
        || event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("p"))
            .count()
            > 1
    {
        return Err("Invalid Dispatch update tags".into());
    }
    let channel = h[1].parse::<Uuid>().map_err(|e| e.to_string())?;
    let notice = DispatchNotice {
        workspace: descriptor[1].parse::<Uuid>().map_err(|e| e.to_string())?,
        issue: descriptor[2].parse::<Uuid>().map_err(|e| e.to_string())?,
        change_event: descriptor[3].clone(),
        purpose: descriptor[4].clone(),
        source_event: source.map(|tag| tag[1].clone()),
    };
    let key = notice.key(channel)?;
    if key != d[1] {
        return Err("Dispatch update identity does not match its scope".into());
    }
    Ok((channel, notice, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notice_identity_is_stable_but_changes_with_destination_or_observation() {
        let channel = Uuid::new_v4();
        let mut notice = DispatchNotice {
            workspace: Uuid::new_v4(),
            issue: Uuid::new_v4(),
            source_event: Some("ab".repeat(32)),
            change_event: "cd".repeat(32),
            purpose: "blocked".into(),
        };
        let a = build_dispatch_notice(channel, &notice, "Waiting for a decision", None, &[])
            .unwrap()
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        let b = build_dispatch_notice(
            channel,
            &notice,
            "The same change, from a second client",
            None,
            &[],
        )
        .unwrap()
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap();
        let key = parse_dispatch_notice(&a).unwrap().2;
        assert_eq!(parse_dispatch_notice(&b).unwrap().2, key);
        assert_ne!(notice.key(Uuid::new_v4()).unwrap(), key);
        notice.change_event = "ef".repeat(32);
        assert_ne!(notice.key(channel).unwrap(), key);
        let mut raw = serde_json::to_value(a).unwrap();
        raw["tags"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(["q", "11".repeat(32)]));
        let malformed: Event = serde_json::from_value(raw).unwrap();
        assert!(parse_dispatch_notice(&malformed).is_err());
    }
}
