use buzz_sdk_pkg::tracker::{
    build_tracker_comment, build_tracker_revision, IssueComment, IssueRevision,
};
use nostr::PublicKey;
use serde::Serialize;
use uuid::Uuid;

/// Validated unsigned Tracker event fields returned to the existing Desktop signer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerUnsignedEvent {
    pub kind: u16,
    pub content: String,
    pub created_at: u64,
    pub tags: Vec<Vec<String>>,
}

fn unsigned_shape(
    builder: nostr::EventBuilder,
    author_pubkey: &str,
) -> Result<TrackerUnsignedEvent, String> {
    let author = PublicKey::from_hex(author_pubkey)
        .map_err(|error| format!("invalid Tracker author pubkey: {error}"))?;
    let unsigned = builder.build(author);
    let value = serde_json::to_value(unsigned)
        .map_err(|error| format!("serialize unsigned Tracker event: {error}"))?;
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| "unsigned Tracker event has invalid kind".to_string())?;
    let content = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "unsigned Tracker event has invalid content".to_string())?
        .to_string();
    let created_at = value
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "unsigned Tracker event has invalid created_at".to_string())?;
    let tags = serde_json::from_value(
        value
            .get("tags")
            .cloned()
            .ok_or_else(|| "unsigned Tracker event has no tags".to_string())?,
    )
    .map_err(|error| format!("unsigned Tracker event has invalid tags: {error}"))?;
    Ok(TrackerUnsignedEvent {
        kind,
        content,
        created_at,
        tags,
    })
}

/// Build and validate a Tracker revision without signing or publishing it.
#[tauri::command]
pub fn build_tracker_revision_event(
    stream_id: Uuid,
    revision: IssueRevision,
    author_pubkey: String,
) -> Result<TrackerUnsignedEvent, String> {
    let builder =
        build_tracker_revision(stream_id, &revision).map_err(|error| error.to_string())?;
    unsigned_shape(builder, &author_pubkey)
}

/// Build and validate a Tracker comment without signing or publishing it.
#[tauri::command]
pub fn build_tracker_comment_event(
    stream_id: Uuid,
    root_event: String,
    parent_event: Option<String>,
    comment: IssueComment,
    mentions: Vec<String>,
    author_pubkey: String,
) -> Result<TrackerUnsignedEvent, String> {
    let builder = build_tracker_comment(
        stream_id,
        &root_event,
        parent_event.as_deref(),
        &comment,
        &mentions,
    )
    .map_err(|error| error.to_string())?;
    unsigned_shape(builder, &author_pubkey)
}

#[cfg(test)]
mod tests {
    use buzz_sdk_pkg::tracker::{
        IssueStatus, Priority, RevisionSet, SetOperations, TRACKER_VERSION,
    };
    use nostr::{Event, JsonUtil, Keys};

    use super::*;

    fn event_id(value: char) -> String {
        std::iter::repeat_n(value, 64).collect()
    }

    #[test]
    fn revision_command_uses_sdk_content_and_preserves_self_assignment_tag() {
        let keys = Keys::generate();
        let author = keys.public_key().to_hex();
        let issue_id = Uuid::new_v4();
        let shape = build_tracker_revision_event(
            Uuid::new_v4(),
            IssueRevision {
                v: TRACKER_VERSION,
                op: "revise".into(),
                issue_id,
                root_event: event_id('a'),
                parent_event: event_id('b'),
                revision: 2,
                set: RevisionSet {
                    status: Some(IssueStatus::InProgress),
                    priority: Some(Priority::P0),
                    assignee: Some(author.clone()),
                    ..RevisionSet::default()
                },
                clear: Vec::new(),
                relations: SetOperations::default(),
                external_refs: SetOperations::default(),
                reason: Some("Starting work".into()),
                request_id: Some(Uuid::new_v4()),
                import: None,
            },
            author.clone(),
        )
        .expect("build revision");

        assert_eq!(shape.kind, 40021);
        assert!(shape
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["p", author.as_str(), "", "assignee"] }));
        let signed =
            nostr::EventBuilder::new(nostr::Kind::Custom(shape.kind), shape.content.clone())
                .tags(
                    shape
                        .tags
                        .iter()
                        .map(nostr::Tag::parse)
                        .collect::<Result<Vec<_>, _>>()
                        .expect("parse tags"),
                )
                .custom_created_at(nostr::Timestamp::from(shape.created_at))
                .allow_self_tagging()
                .sign_with_keys(&keys)
                .expect("sign");
        assert!(Event::from_json(signed.as_json()).is_ok());
    }

    #[test]
    fn comment_command_rejects_invalid_root_before_signing() {
        let keys = Keys::generate();
        let error = build_tracker_comment_event(
            Uuid::new_v4(),
            "not-an-event".into(),
            None,
            IssueComment {
                v: TRACKER_VERSION,
                issue_id: Uuid::new_v4(),
                body: "Comment".into(),
                request_id: None,
                import: None,
            },
            Vec::new(),
            keys.public_key().to_hex(),
        )
        .expect_err("invalid root");
        assert!(error.contains("root_event"));
    }
}
