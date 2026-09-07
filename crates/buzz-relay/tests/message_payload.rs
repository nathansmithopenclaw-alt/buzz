//! New-message payload contract through real HTTP and WebSocket relay routes.
//! Run with --ignored against an isolated migrated Postgres DB, Redis and S3.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use buzz_core::{
    channel::{ChannelType, ChannelVisibility},
    tenant::TenantContext,
};
use buzz_db::{CreateCommunityWithOwnerResult, Db};
use buzz_relay::{config::Config, state::AppState};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

struct Relay {
    state: Arc<AppState>,
    tenant: TenantContext,
    keys: Keys,
    stream: Uuid,
    forum: Uuid,
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Relay {
    async fn start() -> Self {
        let mut config = Config::from_env().expect("test config");
        config.database_url =
            std::env::var("BUZZ_TEST_DATABASE_URL").expect("explicit isolated test DB");
        config.require_auth_token = true;
        config.require_relay_membership = false;
        let host = format!("payload-{}.test", Uuid::new_v4().simple());
        config.relay_url = format!("ws://{host}");
        config.media.public_base_url = format!("http://{host}/media");
        let pool = sqlx::PgPool::connect(&config.database_url)
            .await
            .expect("Postgres");
        let db = Db::from_pool(pool.clone());
        db.migrate().await.expect("migrations");
        let keys = Keys::generate();
        let community = match db
            .create_community_with_owner(&host, &keys.public_key().to_hex())
            .await
            .expect("community")
        {
            CreateCommunityWithOwnerResult::Created(record) => record.id,
            other => panic!("fresh community: {other:?}"),
        };
        db.ensure_user(community, &keys.public_key().to_bytes())
            .await
            .expect("user");
        let stream = db
            .create_channel(
                community,
                "payload-stream",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &keys.public_key().to_bytes(),
                None,
            )
            .await
            .expect("stream")
            .id;
        let forum = db
            .create_channel(
                community,
                "payload-forum",
                ChannelType::Forum,
                ChannelVisibility::Open,
                None,
                &keys.public_key().to_bytes(),
                None,
            )
            .await
            .expect("forum")
            .id;
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("Redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool);
        let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow,
            Keys::generate(),
            storage,
        );
        let state = Arc::new(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listen");
        let addr = listener.local_addr().expect("addr");
        let router = buzz_relay::router::build_router(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("serve")
        });
        Self {
            state,
            tenant: TenantContext::resolved(community, host),
            keys,
            stream,
            forum,
            addr,
            task,
        }
    }

    fn event(&self, kind: u16, content: &str, parent: Option<&Event>, media: &[Tag]) -> Event {
        let channel = if matches!(kind, 45001 | 45003) {
            self.forum
        } else {
            self.stream
        };
        let mut tags = vec![
            Tag::parse(["h", &channel.to_string()]).expect("channel"),
            Tag::parse(["t", &Uuid::new_v4().to_string()]).expect("unique fixture"),
        ];
        if let Some(parent) = parent {
            tags.push(Tag::parse(["e", &parent.id.to_hex(), "", "reply"]).expect("parent"));
        }
        tags.extend_from_slice(media);
        // Deliberately bypass the SDK: only the relay can reject these clients.
        EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .sign_with_keys(&self.keys)
            .expect("sign event")
    }

    async fn http(&self, event: &Event) -> (bool, String) {
        let body = event.as_json();
        let url = format!("http://{}/events", self.tenant.host());
        let hash = hex::encode(Sha256::digest(body.as_bytes()));
        let auth = EventBuilder::new(Kind::Custom(27235), "")
            .tags([
                Tag::parse(["u", &url]).expect("url"),
                Tag::parse(["method", "POST"]).expect("method"),
                Tag::parse(["payload", &hash]).expect("hash"),
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).expect("nonce"),
            ])
            .sign_with_keys(&self.keys)
            .expect("NIP-98 sign");
        let response = reqwest::Client::new()
            .post(format!("http://{}/events", self.addr))
            .header("host", self.tenant.host())
            .header(
                "authorization",
                format!("Nostr {}", STANDARD.encode(auth.as_json())),
            )
            .body(body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .expect("HTTP response");
        let status = response.status();
        let body = response.text().await.expect("HTTP body");
        if status.is_success() {
            assert_eq!(
                serde_json::from_str::<Value>(&body).expect("JSON")["accepted"],
                true,
                "{body}"
            );
        } else {
            assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{body}");
        }
        (status.is_success(), body)
    }

    async fn websocket(&self, event: &Event) -> (bool, String) {
        let mut request = format!("ws://{}/", self.addr)
            .into_client_request()
            .expect("WS request");
        request
            .headers_mut()
            .insert("host", self.tenant.host().parse().expect("host"));
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("WS connect");
        let challenge = loop {
            let frame = socket.next().await.expect("AUTH frame").expect("frame");
            if let Message::Text(text) = frame {
                let value: Value = serde_json::from_str(&text).expect("AUTH JSON");
                if value[0] == "AUTH" {
                    break value[1].as_str().expect("challenge").to_owned();
                }
            }
        };
        let auth = EventBuilder::new(Kind::Custom(22242), "")
            .tags([
                Tag::parse(["relay", &format!("ws://{}", self.tenant.host())]).expect("relay"),
                Tag::parse(["challenge", &challenge]).expect("challenge"),
            ])
            .sign_with_keys(&self.keys)
            .expect("NIP-42 sign");
        socket
            .send(Message::Text(json!(["AUTH", auth]).to_string().into()))
            .await
            .expect("AUTH send");
        loop {
            let frame = socket.next().await.expect("AUTH ack").expect("frame");
            if let Message::Text(text) = frame {
                let value: Value = serde_json::from_str(&text).expect("AUTH ack JSON");
                if value[0] == "OK" {
                    assert_eq!(value[2], true, "{value}");
                    break;
                }
            }
        }
        socket
            .send(Message::Text(json!(["EVENT", event]).to_string().into()))
            .await
            .expect("EVENT send");
        let result = loop {
            let frame = socket.next().await.expect("EVENT ack").expect("frame");
            if let Message::Text(text) = frame {
                let value: Value = serde_json::from_str(&text).expect("EVENT JSON");
                if value[0] == "OK" && value[1] == event.id.to_hex() {
                    break (
                        value[2].as_bool().expect("accepted"),
                        value[3].as_str().expect("message").to_owned(),
                    );
                }
            }
        };
        socket.close(None).await.expect("WS close");
        result
    }

    async fn send(&self, ws: bool, event: &Event, accepted: bool, reason: Option<&str>) {
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            if ws {
                self.websocket(event).await
            } else {
                self.http(event).await
            }
        })
        .await
        .expect("transport deadline");
        assert_eq!(
            result.0, accepted,
            "ws={ws} kind={} response={}",
            event.kind, result.1
        );
        if let Some(reason) = reason {
            assert!(result.1.contains(reason), "{}", result.1);
        }
        let stored = self
            .state
            .db
            .get_event_by_id(self.tenant.community(), event.id.as_bytes())
            .await
            .expect("readback");
        assert_eq!(
            stored.is_some(),
            accepted,
            "durable event presence must match acceptance"
        );
        if let Some(stored) = stored {
            assert_eq!(stored.event.content, event.content);
        }
    }
}

mod external_infra {
    use super::*;

    #[tokio::test]
    #[ignore = "requires isolated Postgres, Redis and S3; set BUZZ_TEST_DATABASE_URL"]
    async fn payload_contract_over_http_and_websocket() {
        let relay = Relay::start().await;
        let parent = relay.event(45001, "forum parent", None, &[]);
        relay.send(false, &parent, true, None).await;
        // Real stored attachment and tenant sidecar, as produced by an upload.
        let bytes = Uuid::new_v4().to_string().into_bytes();
        let hash = hex::encode(Sha256::digest(&bytes));
        let key = format!("{hash}.txt");
        relay
            .state
            .media_storage
            .put(&key, &bytes, "text/plain")
            .await
            .expect("store fixture blob");
        relay
            .state
            .media_storage
            .put_sidecar(
                &relay.tenant,
                &hash,
                &buzz_media::BlobMeta {
                    ext: "txt".into(),
                    mime_type: "text/plain".into(),
                    size: bytes.len() as u64,
                    ..Default::default()
                },
            )
            .await
            .expect("store tenant sidecar");
        let media = Tag::parse(vec![
            "imeta".to_owned(),
            format!("url http://{}/media/{key}", relay.tenant.host()),
            "m text/plain".into(),
            format!("x {hash}"),
            format!("size {}", bytes.len()),
        ])
        .expect("media");
        for ws in [false, true] {
            for kind in [9, 40002, 45001, 45003] {
                let reply = (kind == 45003).then_some(&parent);
                for content in ["", " \t\n", "\u{00a0}\u{2003}\u{3000}"] {
                    relay
                        .send(
                            ws,
                            &relay.event(kind, content, reply, &[]),
                            false,
                            Some("empty or whitespace-only"),
                        )
                        .await;
                }
                relay
                    .send(
                        ws,
                        &relay.event(kind, "  Markdown  \nnext 🐝", reply, &[]),
                        true,
                        None,
                    )
                    .await;
                relay
                    .send(
                        ws,
                        &relay.event(kind, "", reply, std::slice::from_ref(&media)),
                        true,
                        None,
                    )
                    .await;
            }
            let malformed = Tag::parse(["imeta"]).expect("malformed fixture tag");
            relay
                .send(
                    ws,
                    &relay.event(9, "", None, &[malformed]),
                    false,
                    Some("imeta"),
                )
                .await;
            let missing_hash = "0".repeat(64);
            let missing = Tag::parse(vec![
                "imeta".into(),
                format!(
                    "url http://{}/media/{missing_hash}.txt",
                    relay.tenant.host()
                ),
                "m text/plain".into(),
                format!("x {missing_hash}"),
                "size 1".into(),
            ])
            .expect("missing blob tag");
            relay
                .send(
                    ws,
                    &relay.event(9, "", None, &[missing]),
                    false,
                    Some("nonexistent blob"),
                )
                .await;
            let original = relay.event(9, "to clear", None, &[]);
            relay.send(ws, &original, true, None).await;
            relay
                .send(
                    ws,
                    &relay.event(40003, "", Some(&original), &[]),
                    true,
                    None,
                )
                .await;
        }
        relay
            .state
            .media_storage
            .delete(&key)
            .await
            .expect("remove fixture blob");
        relay
            .state
            .media_storage
            .delete(&buzz_media::MediaStorage::ctx_sidecar_key(
                &relay.tenant,
                &hash,
            ))
            .await
            .expect("remove fixture sidecar");
    }
}
