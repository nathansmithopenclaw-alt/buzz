use nostr::{Event, Kind, PublicKey};

use crate::{client::BuzzClient, error::CliError, validate::validate_uuid};

/// Desktop only accepts management requests from a channel shared with the
/// owner. Check before publishing an ephemeral request that it would discard.
/// This is a usability preflight; Desktop's ownership and delegation gates
/// remain authoritative, including when membership changes after this query.
pub(super) async fn require_shared_origin(
    client: &BuzzClient,
    owner: &PublicKey,
    channel: &str,
) -> Result<(), CliError> {
    validate_uuid(channel)?;
    let raw = client
        .query(&serde_json::json!({"kinds": [39002], "#d": [channel], "limit": 1}))
        .await?;
    let events: Vec<Event> = serde_json::from_str(&raw).map_err(|error| {
        CliError::Other(format!("invalid management origin membership: {error}"))
    })?;
    let requester = client.keys().public_key().to_hex();
    let owner = owner.to_hex();
    let shared = events.iter().any(|event| {
        event.kind == Kind::Custom(39002)
            && event.verify().is_ok()
            && event.tags.iter().any(|tag| {
                let fields = tag.as_slice();
                fields.first().map(String::as_str) == Some("d")
                    && fields.get(1).map(String::as_str) == Some(channel)
            })
            && [&requester, &owner].iter().all(|member| {
                event.tags.iter().any(|tag| {
                    let fields = tag.as_slice();
                    fields.first().map(String::as_str) == Some("p")
                        && fields
                            .get(1)
                            .is_some_and(|key| key.eq_ignore_ascii_case(member))
                })
            })
    });
    if !shared {
        return Err(CliError::Auth(format!(
            "agent management requires --channel to name a channel shared by the requesting agent and its owner; {channel} does not establish that membership. Use a shared channel. No management request was published."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentsCmd;
    use axum::{routing::post, Router};
    use nostr::{EventBuilder, JsonUtil, Keys, Tag};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::net::TcpListener;

    const CHANNEL: &str = "42481632-7210-4347-9c63-2bd9f1d7e7d4";

    async fn client_with_membership(
        body: String,
        keys: Keys,
        owner: &Keys,
    ) -> (BuzzClient, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let count = Arc::new(AtomicUsize::new(0));
        let hits = count.clone();
        let app = Router::new().route(
            "/query",
            post(move || {
                hits.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                async move { body }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let auth = Tag::parse(["auth", &owner.public_key().to_hex(), "", ""]).unwrap();
        (
            BuzzClient::new(url, keys, Some(auth), None).unwrap(),
            count,
            task,
        )
    }

    fn membership(
        keys: &Keys,
        owner: &Keys,
        include_requester: bool,
        include_owner: bool,
    ) -> Event {
        let mut tags = vec![Tag::parse(["d", CHANNEL]).unwrap()];
        if include_requester {
            tags.push(Tag::parse(["p", &keys.public_key().to_hex()]).unwrap());
        }
        if include_owner {
            tags.push(Tag::parse(["p", &owner.public_key().to_hex()]).unwrap());
        }
        EventBuilder::new(Kind::Custom(39002), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[tokio::test]
    async fn shared_origin_requires_both_members_and_valid_scoped_event() {
        for case in 0..8 {
            let keys = Keys::generate();
            let owner = Keys::generate();
            let mut event = membership(&keys, &owner, case != 1, case != 2);
            if case == 3 {
                event.content = "tampered".into();
            }
            if case == 4 {
                event = EventBuilder::new(Kind::Custom(39002), "")
                    .tags([
                        Tag::parse(["d", "wrong-channel"]).unwrap(),
                        Tag::parse(["p", &keys.public_key().to_hex()]).unwrap(),
                        Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
                    ])
                    .sign_with_keys(&Keys::generate())
                    .unwrap();
            }
            if case == 7 {
                event = EventBuilder::new(Kind::Custom(39000), "")
                    .tags(event.tags.clone())
                    .sign_with_keys(&Keys::generate())
                    .unwrap();
            }
            let body = match case {
                5 => "[]".into(),
                6 => "malformed".into(),
                _ => format!("[{}]", event.as_json()),
            };
            let (client, count, task) = client_with_membership(body, keys, &owner).await;
            let result = require_shared_origin(&client, &owner.public_key(), CHANNEL).await;
            assert_eq!(result.is_ok(), case == 0, "case {case}: {result:?}");
            assert_eq!(count.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn apply_dispatch_rejects_unshared_origin_before_ephemeral_publish() {
        let commands = [
            AgentsCmd::ApplyCreate {
                request_id: None,
                channel: CHANNEL.into(),
                display_name: "Probe".into(),
                system_prompt: "probe".into(),
                runtime: None,
                provider: None,
                model: None,
                respond_to: None,
                parallelism: None,
                env: vec![],
            },
            AgentsCmd::ApplyUpdate {
                request_id: Some("6a5318fb-558a-428f-9ba1-176cf0b5afce".into()),
                channel: CHANNEL.into(),
                agent_name: "Probe".into(),
                display_name: None,
                system_prompt: None,
                runtime: None,
                provider: None,
                model: Some("default[]".into()),
                effort: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                parallelism: None,
                instance_name_pool: None,
                env: vec![],
            },
            AgentsCmd::ApplyGlobalConfig {
                request_id: None,
                channel: CHANNEL.into(),
                provider: None,
                model: Some("default[]".into()),
                preferred_runtime: None,
                effort: None,
                env: vec![],
            },
        ];
        for command in commands {
            let keys = Keys::generate();
            let owner = Keys::generate();
            let body = format!("[{}]", membership(&keys, &owner, true, false).as_json());
            let (client, count, task) = client_with_membership(body, keys, &owner).await;
            // This HTTP fixture has no WebSocket endpoint. Skipping the production
            // preflight attempts publication and fails with a different error.
            let error = super::super::dispatch(command, &client).await.unwrap_err();
            assert!(matches!(error, CliError::Auth(_)), "{error:?}");
            assert!(error
                .to_string()
                .contains("No management request was published"));
            assert_eq!(count.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }
}
