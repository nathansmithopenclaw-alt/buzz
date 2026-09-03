//! Workflow approval command event builders.

use nostr::{EventBuilder, Kind, Tag};
fn build(approval_ref: &str, note: Option<&str>, kind: u16) -> Result<EventBuilder, String> {
    if approval_ref.len() != 64 || !approval_ref.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("invalid approval reference: expected 64 hex characters".into());
    }
    let tag = Tag::parse(["d", approval_ref.to_ascii_lowercase().as_str()])
        .map_err(|e| format!("invalid approval reference tag: {e}"))?;
    Ok(EventBuilder::new(Kind::Custom(kind), note.unwrap_or("")).tags([tag]))
}

/// Kind 46030 — grant an approval reference (with optional note).
pub fn build_approval_grant(
    approval_ref: &str,
    note: Option<&str>,
) -> Result<EventBuilder, String> {
    build(approval_ref, note, 46030)
}

/// Kind 46031 — deny an approval reference (with optional note).
pub fn build_approval_deny(approval_ref: &str, note: Option<&str>) -> Result<EventBuilder, String> {
    build(approval_ref, note, 46031)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn builders_put_opaque_reference_in_d_tag() {
        let approval_ref = "ab".repeat(32);
        for builder in [
            build_approval_grant(&approval_ref, None).unwrap(),
            build_approval_deny(&approval_ref, Some("no")).unwrap(),
        ] {
            let event = builder.sign_with_keys(&Keys::generate()).unwrap();
            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect();
            assert_eq!(tags, vec![vec!["d".to_owned(), approval_ref.clone()]]);
        }
        assert!(build_approval_grant("not-hex", None).is_err());
    }
}
