//! Workflow approval command event builders.

use nostr::{EventBuilder, Kind, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn build(token: &str, note: Option<&str>, kind: u16) -> Result<EventBuilder, String> {
    Uuid::parse_str(token).map_err(|e| format!("invalid approval token: {e}"))?;
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    let tag = Tag::parse(["d", token_hash.as_str()])
        .map_err(|e| format!("invalid approval token tag: {e}"))?;
    Ok(EventBuilder::new(Kind::Custom(kind), note.unwrap_or("")).tags([tag]))
}

/// Kind 46030 — grant an approval token (with optional note).
pub fn build_approval_grant(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    build(token, note, 46030)
}

/// Kind 46031 — deny an approval token (with optional note).
pub fn build_approval_deny(token: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    build(token, note, 46031)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn builders_hash_raw_token_into_d_tag() {
        let token = "123e4567-e89b-12d3-a456-426614174000";
        let expected = hex::encode(Sha256::digest(token.as_bytes()));
        for builder in [
            build_approval_grant(token, None).unwrap(),
            build_approval_deny(token, Some("no")).unwrap(),
        ] {
            let event = builder.sign_with_keys(&Keys::generate()).unwrap();
            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect();
            assert_eq!(tags, vec![vec!["d".to_owned(), expected.clone()]]);
        }
        assert!(build_approval_grant("not-a-uuid", None).is_err());
    }
}
