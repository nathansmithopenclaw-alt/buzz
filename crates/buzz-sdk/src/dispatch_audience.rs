//! Permission-aware transfer of conversation context into a Dispatch workspace.
use nostr::Event;
use std::collections::BTreeSet;
use uuid::Uuid;

fn members(event: &Event, channel: Uuid) -> Result<BTreeSet<String>, String> {
    event
        .verify()
        .map_err(|_| "membership signature is invalid")?;
    if event.kind.as_u16() != 39002 {
        return Err("expected channel membership".into());
    }
    let ids = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
        .collect::<Vec<_>>();
    if ids.len() != 1 || ids[0].as_slice().get(1) != Some(&channel.to_string()) {
        return Err("membership identifies another channel".into());
    }
    Ok(event
        .tags
        .iter()
        .filter_map(|tag| {
            let tag = tag.as_slice();
            (tag.first().map(String::as_str) == Some("p"))
                .then(|| tag.get(1).cloned())
                .flatten()
        })
        .collect())
}
/// Check current relay-issued membership snapshots before copying source content.
/// The caller must fetch the latest snapshots from its authenticated relay and
/// fence that relay and signer through publication. A destination may only narrow
/// the source audience, and every required reader must belong to both channels.
pub fn validate_dispatch_audience(
    source: &Event,
    destination: &Event,
    source_channel: Uuid,
    destination_channel: Uuid,
    viewer: &str,
    required_readers: &[String],
) -> Result<(), String> {
    if source.pubkey != destination.pubkey {
        return Err("membership snapshots have different issuers".into());
    }
    let source = members(source, source_channel)?;
    let destination = members(destination, destination_channel)?;
    if !source.contains(viewer) || !destination.contains(viewer) {
        return Err("the current identity must belong to both conversations".into());
    }
    if !destination.is_subset(&source) {
        return Err("the destination includes people who cannot read the source; share a link and your own summary instead".into());
    }
    if required_readers
        .iter()
        .any(|reader| !source.contains(reader) || !destination.contains(reader))
    {
        return Err(
            "the executor and reviewer must be able to read the work and its destination".into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    fn membership(channel: Uuid, relay: &Keys, people: &[&str]) -> Event {
        let mut tags = vec![Tag::parse(["d", &channel.to_string()]).unwrap()];
        tags.extend(
            people
                .iter()
                .map(|person| Tag::parse(["p", *person]).unwrap()),
        );
        EventBuilder::new(Kind::Custom(39002), "")
            .tags(tags)
            .sign_with_keys(relay)
            .unwrap()
    }
    #[test]
    fn context_transfer_requires_signed_current_audience_containment() {
        let relay = Keys::generate();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let owner = Keys::generate().public_key().to_hex();
        let worker = Keys::generate().public_key().to_hex();
        let guest = Keys::generate().public_key().to_hex();
        let source = membership(a, &relay, &[&owner, &worker]);
        let narrow = membership(b, &relay, &[&owner]);
        assert!(validate_dispatch_audience(&source, &narrow, a, b, &owner, &[]).is_ok());
        assert!(validate_dispatch_audience(
            &source,
            &narrow,
            a,
            b,
            &owner,
            std::slice::from_ref(&worker)
        )
        .is_err());
        let broad = membership(b, &relay, &[&owner, &guest]);
        assert!(validate_dispatch_audience(&source, &broad, a, b, &owner, &[]).is_err());
        let forged = membership(b, &Keys::generate(), &[&owner]);
        assert!(validate_dispatch_audience(&source, &forged, a, b, &owner, &[]).is_err());
        let revoked = membership(a, &relay, &[&worker]);
        assert!(validate_dispatch_audience(&revoked, &narrow, a, b, &owner, &[]).is_err());
    }
}
