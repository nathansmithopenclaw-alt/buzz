//! Bounded CLI receipt candidates. Tool output alone never proves publication.
use std::collections::HashSet;

pub(crate) const MAX_CANDIDATES: usize = 32;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

pub(crate) fn collect_candidates(value: &serde_json::Value, ids: &mut HashSet<nostr::EventId>) {
    let mut budget = MAX_OUTPUT_BYTES;
    collect(value, ids, 0, &mut budget);
}

fn collect(
    value: &serde_json::Value,
    ids: &mut HashSet<nostr::EventId>,
    depth: usize,
    budget: &mut usize,
) {
    if depth > 8 || ids.len() >= MAX_CANDIDATES || *budget == 0 {
        return;
    }
    *budget -= 1;
    match value {
        serde_json::Value::Object(object) => {
            if object.get("accepted").and_then(|v| v.as_bool()) == Some(true) {
                if let Some(id) = object
                    .get("event_id")
                    .and_then(|v| v.as_str())
                    .and_then(|v| nostr::EventId::from_hex(v).ok())
                {
                    ids.insert(id);
                }
            }
            for child in object.values().take(128) {
                collect(child, ids, depth + 1, budget);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values.iter().take(128) {
                collect(child, ids, depth + 1, budget);
            }
        }
        serde_json::Value::String(text) if text.len() <= *budget => {
            *budget -= text.len();
            // CLI stdout is JSON, sometimes wrapped in MCP text blocks. Accept
            // whole JSON or JSON lines; never scrape an arbitrary hex string.
            if let Ok(parsed) = serde_json::from_str(text) {
                collect(&parsed, ids, depth + 1, budget);
            } else {
                for line in text.lines().take(128) {
                    if let Ok(parsed) = serde_json::from_str(line) {
                        collect(&parsed, ids, depth + 1, budget);
                    }
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn verified_candidate(
    event: &nostr::Event,
    ids: &HashSet<nostr::EventId>,
    author: nostr::PublicKey,
    channel: uuid::Uuid,
    since: nostr::Timestamp,
) -> bool {
    ids.contains(&event.id)
        && event.pubkey == author
        && event.created_at >= since
        && crate::pool::is_channel_report_kind(event.kind.as_u16() as u32)
        && !crate::is_delivery_failure_report(event)
        && event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.first().map(String::as_str) == Some("h")
                && parts
                    .get(1)
                    .is_some_and(|value| value == &channel.to_string())
        })
        && event.verify().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_dm_receipt_requires_exact_signed_author_channel_and_time() {
        let keys = nostr::Keys::generate();
        let channel = uuid::Uuid::new_v4();
        let event = buzz_sdk::build_message(channel, "completed", None, &[], false, &[], &[])
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert!(buzz_core::nip10::parse_thread_markers(&event.tags)
            .reply
            .is_none());
        let ids = HashSet::from([event.id]);
        let verify =
            |event: &nostr::Event, ids: &HashSet<nostr::EventId>, author, channel, since| {
                verified_candidate(event, ids, author, channel, since)
            };
        assert!(verify(
            &event,
            &ids,
            keys.public_key(),
            channel,
            event.created_at
        ));
        assert!(!verify(
            &event,
            &HashSet::new(),
            keys.public_key(),
            channel,
            event.created_at
        ));
        assert!(!verify(
            &event,
            &ids,
            nostr::Keys::generate().public_key(),
            channel,
            event.created_at
        ));
        assert!(!verify(
            &event,
            &ids,
            keys.public_key(),
            uuid::Uuid::new_v4(),
            event.created_at
        ));
        assert!(!verify(
            &event,
            &ids,
            keys.public_key(),
            channel,
            nostr::Timestamp::from(event.created_at.as_secs() + 1)
        ));
        let mut forged = serde_json::to_value(&event).unwrap();
        forged["content"] = serde_json::json!("forged");
        let forged = serde_json::from_value(forged).unwrap();
        assert!(!verify(
            &forged,
            &ids,
            keys.public_key(),
            channel,
            event.created_at
        ));
    }

    #[test]
    fn candidates_require_accepted_json_and_bound_output_and_count() {
        let mut ids = HashSet::new();
        for value in [
            serde_json::json!({"accepted":false,"event_id":"a".repeat(64)}),
            serde_json::json!({"event_id":"b".repeat(64)}),
            serde_json::json!("c".repeat(64)),
            serde_json::json!("x".repeat(MAX_OUTPUT_BYTES + 1)),
        ] {
            collect_candidates(&value, &mut ids);
        }
        assert!(ids.is_empty());
        for n in 0..100 {
            collect_candidates(
                &serde_json::json!({"accepted":true,"event_id":format!("{n:064x}")}),
                &mut ids,
            );
        }
        assert_eq!(ids.len(), MAX_CANDIDATES);
    }
}
