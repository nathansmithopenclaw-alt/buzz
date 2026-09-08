//! Structured report publication: no shell, a fixed destination, replay-safe signed events.
use crate::{execution_evidence as evidence, relay::RestClient};
use anyhow::{ensure, Context};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};

#[derive(Deserialize, schemars::JsonSchema)]
struct Report {
    /// Copy the current turn_id from host-execution-evidence or execution_status.
    turn_id: String,
    /// Exact Markdown/text to publish. Do not quote it as a shell command.
    content: String,
}

#[derive(Serialize, Deserialize)]
struct PendingReport {
    content_hash: String,
    event: nostr::Event,
}

#[derive(Clone)]
struct Reports {
    root: PathBuf,
    session: String,
    rest: RestClient,
    gate: Arc<tokio::sync::Mutex<()>>,
    tool_router: ToolRouter<Self>,
}

fn response(result: anyhow::Result<serde_json::Value>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::success(vec![Content::text(value.to_string())]),
        Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
    }
}

#[tool_router]
impl Reports {
    fn new(root: PathBuf, session: String, rest: RestClient) -> Self {
        Self {
            root,
            session,
            rest,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Read host-recorded computer-use execution facts, including cancelled turns. Empty live resource lists never erase prior actions; pending actions have unknown external outcomes."
    )]
    async fn execution_status(&self) -> CallToolResult {
        response(
            evidence::active(&self.root, &self.session)
                .and_then(|turn| evidence::summary(&self.root, &turn)),
        )
    }

    #[tool(
        description = "Publish the exact report content to this turn's authorized Buzz conversation and reply target using structured data. Use this for all reports instead of shell/CLI commands. Backticks, dollar signs, quotes and newlines are literal. A retry with identical content reuses the same signed event; changing a pending report is refused. Returns a verified publication receipt, or an explicit unresolved delivery error."
    )]
    async fn report_publish(&self, Parameters(report): Parameters<Report>) -> CallToolResult {
        response(self.publish(&report.turn_id, report.content).await)
    }
}

impl Reports {
    async fn publish(&self, turn_id: &str, content: String) -> anyhow::Result<serde_json::Value> {
        ensure!(
            !content.trim().is_empty() && content.len() <= 24000,
            "Report content must be 1–24000 bytes"
        );
        let _guard = self.gate.lock().await;
        let turn = evidence::active(&self.root, &self.session)?;
        ensure!(
            turn.turn_id == turn_id,
            "Stale report turn_id; obtain execution_status for the current turn"
        );
        let path = self
            .root
            .join("turns")
            .join(&turn.turn_id)
            .join("report.json");
        let content_hash = hex::encode(Sha256::digest(content.as_bytes()));
        let pending = if path.try_exists()? {
            evidence::read::<PendingReport>(&path)?
        } else {
            let summary = evidence::summary(&self.root, &turn)?;
            let content = if turn.recovery {
                format!(
                    "{content}\n\nHost execution record (survives cancellation):\n{}",
                    serde_json::to_string_pretty(&summary)?
                )
            } else {
                content
            };
            let thread = buzz_sdk::ThreadRef {
                root_event_id: nostr::EventId::from_hex(&turn.root_event_id)?,
                parent_event_id: nostr::EventId::from_hex(&turn.reply_event_id)?,
            };
            let mut builder = buzz_sdk::build_message(
                turn.channel_id,
                &content,
                Some(&thread),
                &[],
                false,
                &[],
                &[],
            )?;
            if let Some(auth) = &self.rest.auth_tag_json {
                builder = builder.tag(nostr::Tag::parse(serde_json::from_str::<Vec<String>>(
                    auth,
                )?)?);
            }
            let pending = PendingReport {
                content_hash: content_hash.clone(),
                event: builder.sign_with_keys(&self.rest.keys)?,
            };
            if let Err(error) = evidence::write(&path, &pending, true) {
                if !path.try_exists()? {
                    return Err(error);
                }
            }
            // Re-read the exclusive winner; concurrent callers cannot create two events.
            evidence::read::<PendingReport>(&path)?
        };
        ensure!(pending.content_hash == content_hash, "A different report already exists for this turn; retry the original content to reconcile its receipt");
        pending.event.verify()?;
        ensure!(
            pending.event.pubkey == self.rest.keys.public_key(),
            "Report signer does not match this session"
        );
        let channels: Vec<_> = pending
            .event
            .tags
            .iter()
            .filter_map(|tag| {
                let values = tag.as_slice();
                (values.first().is_some_and(|name| name == "h"))
                    .then(|| values.get(1))
                    .flatten()
            })
            .collect();
        ensure!(
            channels == vec![&turn.channel_id.to_string()],
            "Stored report has a different channel"
        );
        let routing = crate::queue::parse_thread_tags(&pending.event);
        ensure!(
            routing.root_event_id.as_deref() == Some(&turn.root_event_id)
                && routing.parent_event_id.as_deref() == Some(&turn.reply_event_id),
            "Stored report has stale routing"
        );
        ensure!(
            evidence::active(&self.root, &self.session)?.turn_id == turn.turn_id,
            "Turn changed before report publication"
        );
        let result = self.rest.submit_event(&pending.event).await?;
        ensure!(
            result.get("accepted").and_then(|v| v.as_bool()) != Some(false),
            "Relay refused the report"
        );
        let events = self
            .rest
            .query(&[nostr::Filter::new()
                .id(pending.event.id)
                .kind(nostr::Kind::Custom(9))
                .limit(1)])
            .await?;
        let verified = events.as_array().is_some_and(|events| {
            events.iter().any(|raw| {
                serde_json::from_value::<nostr::Event>(raw.clone())
                    .is_ok_and(|event| event.id == pending.event.id && event.verify().is_ok())
            })
        });
        ensure!(verified, "Report delivery is unresolved: signed event {} is retained; retry identical content to verify, not a new report", pending.event.id);
        let receipt = serde_json::json!({"accepted":true,"event_id":pending.event.id,"readback_verified":true,"content_sha256":content_hash});
        evidence::write(&path.with_file_name("report-receipt.json"), &receipt, false)?;
        Ok(receipt)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Reports {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new("buzz-report", env!("CARGO_PKG_VERSION")))
            .with_instructions("Publish reports with report_publish, never shell interpolation. Consult execution_status before describing prior cancelled work. Unknown outcomes must remain unknown; do not repeat prior mutations.")
    }
}

pub(crate) async fn run() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var("BUZZ_EXECUTION_ROOT").context("Missing report grant")?);
    evidence::private_dir(&root)?;
    let session = std::env::var("BUZZ_EXECUTION_SESSION")?;
    ensure!(evidence::valid_id(&session), "Invalid report session");
    let keys =
        nostr::Keys::parse(&std::env::var("BUZZ_PRIVATE_KEY")?).context("Invalid report signer")?;
    let base_url = std::env::var("BUZZ_RELAY_URL")?;
    let rest = RestClient {
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?,
        base_url,
        keys,
        auth_tag_json: std::env::var("BUZZ_AUTH_TAG").ok(),
    };
    Reports::new(root, session, rest)
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "report_mcp_tests.rs"]
mod tests;
