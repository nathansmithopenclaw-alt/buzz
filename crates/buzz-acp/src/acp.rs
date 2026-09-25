//! ACP client module — manages communication with an AI agent subprocess over stdio
//! using JSON-RPC 2.0 (newline-delimited / NDJSON).
//!
//! # Lifecycle
//! 1. [`AcpClient::spawn`] — launch agent binary as subprocess
//! 2. [`AcpClient::initialize`] — protocol version negotiation
//! 3. [`AcpClient::session_new`] — create session with MCP server config
//! 4. [`AcpClient::session_prompt_with_idle_timeout`] — send prompt with idle/hard deadline, return stop reason
//! 5. [`AcpClient::session_cancel`] / [`AcpClient::cancel_with_cleanup`] — cancel in-flight turn
//! 6. [`AcpClient::session_close_fire_and_forget`] — release an invalidated provider session

use futures_util::{FutureExt, StreamExt};
use std::collections::{HashMap, HashSet};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

#[path = "codex_model_config.rs"]
mod codex_model_config;
pub use codex_model_config::resolve_model_switch_method;
#[path = "acp_session_close.rs"]
mod session_close;
mod subscription_token;

use crate::observer::{ObserverContext, ObserverHandle};
use crate::usage::{
    PromptResponseUsage, StandardAdapterKind, StandardUsageTracker, TurnUsage, UsageTracker,
};

/// Maximum allowed size of a single NDJSON line from the agent's stdout.
/// Lines exceeding this limit are rejected to prevent OOM from rogue agents.
const MAX_LINE_SIZE: usize = 10_000_000; // 10 MB

/// Maximum number of Unicode scalar values retained from an agent's final answer.
///
/// This is deliberately small because the text may be surfaced in a failure notice.
/// Tool inputs and outputs are never retained here.
const FINAL_ANSWER_SNIPPET_CHARS: usize = 160;
const MAX_PRIVATE_CAPTURE_BYTES: usize = 16 * 1024;

/// Package name reported by the Claude ACP adapter. The adapter supports a
/// filtered raw-SDK-message stream through `_meta.claudeCode.emitRawSDKMessages`.
pub(crate) const CLAUDE_AGENT_ACP_NAME: &str = "@agentclientprotocol/claude-agent-acp";

const MAX_BACKGROUND_TASK_SESSIONS: usize = 256;
const MAX_BACKGROUND_TASKS_PER_SESSION: usize = 64;
const MAX_BACKGROUND_TASK_FIELD_CHARS: usize = 240;
/// Fallback for an id-less task in a degraded snapshot. Every such task
/// collapses to this one string, so it must never key timestamp inheritance:
/// two different id-less tasks across snapshots would otherwise swap ages.
const UNKNOWN_BACKGROUND_TASK_ID: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BackgroundTask {
    pub task_id: String,
    pub task_type: String,
    pub description: String,
    /// When THIS task id was first seen pending, independent of the
    /// session-level hold anchor. The session anchor deliberately never
    /// refreshes while the set stays non-empty (anti-pinning cap); this
    /// per-task timestamp exists so termination notices can report each
    /// task's real age instead of implying the anchor is the task's age.
    pub first_observed_at: tokio::time::Instant,
}

#[derive(Clone, Debug)]
struct SessionBackgroundTasks {
    first_observed_at: tokio::time::Instant,
    tasks: Vec<BackgroundTask>,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingBackgroundTasks {
    pub session_id: String,
    pub first_observed_at: tokio::time::Instant,
    pub tasks: Vec<BackgroundTask>,
    /// Monitoring mode label observed for this client's session
    /// (`typed-confirmed` / `typed-unconfirmed` / `raw-seam` / `unarmed`),
    /// surfaced so teardown notices state protection status directly (L2.3).
    pub monitoring_state: &'static str,
}

/// Consumer-side background-task monitoring mode (design v2.1 L2.1: four
/// branches, one enum). Selected at `initialize` from the init `_meta`
/// capability's presence, then refined by the per-session readiness object
/// from `session/new`. Readiness changes flow ONLY from adapter-published
/// snapshots — the consumer observes promotion, never performs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackgroundMode {
    /// Init `_meta.backgroundTasks` absent (stock/unforked adapter): filtered
    /// raw `_claude/sdkMessage` seam, exactly the verified 5eeeb3f8 path.
    RawSeam,
    /// Session readiness `unarmed`: loud operator notice already emitted; NO
    /// raw fallback and no listening — falling back would recreate the
    /// false-green this lane exists to kill.
    Unarmed,
    /// Session readiness `unconfirmed` (honest default for a configured CLI
    /// that has not yet emitted): typed snapshots only, protection visibly
    /// NOT confirmed.
    UnconfirmedTyped,
    /// Adapter snapshot carried `readiness: "confirmed"`: typed only, ignore
    /// any raw lifecycle duplicates; full protection claims.
    ConfirmedTyped,
}

impl BackgroundMode {
    /// True when lifecycle state may be consumed at all (typed or raw).
    pub(crate) fn is_listening(self) -> bool {
        !matches!(self, BackgroundMode::Unarmed)
    }

    /// Human-readable monitoring state for logs and notices (L2.3).
    pub(crate) fn label(self) -> &'static str {
        match self {
            BackgroundMode::RawSeam => "raw-seam",
            BackgroundMode::Unarmed => "unarmed",
            BackgroundMode::UnconfirmedTyped => "typed-unconfirmed",
            BackgroundMode::ConfirmedTyped => "typed-confirmed",
        }
    }
}

/// Prefix goose uses when the adapter, rather than the model, synthesizes a
/// terminal provider/runtime failure as an `agent_message_chunk`.
const GOOSE_RUNTIME_FAILURE_PREFIX: &str = "Ran into this error:";

/// Delivery evidence collected from ACP updates for one prompt turn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnDeliveryEvidence {
    /// Whether the agent emitted any non-whitespace final-answer text.
    pub final_answer_present: bool,
    /// Whitespace-normalized, bounded final-answer text. Never tool content.
    pub final_answer_snippet: Option<String>,
    /// Whether goose synthesized a provider/runtime failure as final output.
    pub adapter_runtime_failure_present: bool,
    /// Number of distinct `buzz messages send` tool calls observed.
    pub publish_attempt_count: usize,
    /// Number of those calls that emitted a failed/error update.
    pub publish_failure_count: usize,
    /// Candidate CLI receipts, verified against signed relay events before use.
    pub published_event_ids: HashSet<nostr::EventId>,
    publish_tool_ids: HashSet<String>,
    failed_publish_tool_ids: HashSet<String>,
}

impl TurnDeliveryEvidence {
    fn record_final_answer_chunk(&mut self, text: &str, from_goose: bool) {
        if text.trim().is_empty() {
            return;
        }
        self.final_answer_present = true;
        let combined = match self.final_answer_snippet.take() {
            Some(existing) => format!("{existing} {text}"),
            None => text.to_owned(),
        };
        // Retain the latest terminal output, not an initial adapter warning.
        // Detect goose's failure prefix before trimming the bounded excerpt.
        self.final_answer_snippet = normalized_snippet(&combined, FINAL_ANSWER_SNIPPET_CHARS);
        if from_goose
            && self
                .final_answer_snippet
                .as_deref()
                .is_some_and(|snippet| snippet.starts_with(GOOSE_RUNTIME_FAILURE_PREFIX))
        {
            self.adapter_runtime_failure_present = true;
        }
        let normalized = combined.split_whitespace().collect::<Vec<_>>().join(" ");
        let count = normalized.chars().count();
        if count > FINAL_ANSWER_SNIPPET_CHARS {
            self.final_answer_snippet = Some(format!(
                "…{}",
                normalized
                    .chars()
                    .skip(count - FINAL_ANSWER_SNIPPET_CHARS)
                    .collect::<String>()
            ));
        }
    }

    fn record_tool_call(&mut self, update: &serde_json::Value) {
        if !value_contains_buzz_message_send(update.get("rawInput")) {
            return;
        }
        let Some(tool_id) = update.get("toolCallId").and_then(|value| value.as_str()) else {
            return;
        };
        if self.publish_tool_ids.insert(tool_id.to_owned()) {
            self.publish_attempt_count += 1;
        }
    }

    fn record_tool_call_update(&mut self, update: &serde_json::Value) {
        // Claude supplies completed Bash input in an update, after the initial
        // tool_call. Process either shape before interpreting the result.
        self.record_tool_call(update);
        let Some(tool_id) = update.get("toolCallId").and_then(|value| value.as_str()) else {
            return;
        };
        if !self.publish_tool_ids.contains(tool_id) {
            return;
        }
        let failed = update.get("status").and_then(|value| value.as_str()) == Some("failed")
            || update
                .pointer("/rawOutput/isError")
                .and_then(|value| value.as_bool())
                == Some(true)
            || update.pointer("/rawOutput/error").is_some();
        if failed && self.failed_publish_tool_ids.insert(tool_id.to_owned()) {
            self.publish_failure_count += 1;
        }
        if !failed && update.get("status").and_then(|v| v.as_str()) == Some("completed") {
            if let Some(output) = update.get("rawOutput") {
                crate::publication_receipts::collect_candidates(
                    output,
                    &mut self.published_event_ids,
                );
            }
            // ACP adapters may put the actual result in content and use
            // rawOutput only for metadata (buzz-agent emits isError there).
            // These remain candidates; the relay must verify the exact signed
            // event before this turn is credited with delivery.
            if let Some(content) = update.get("content") {
                crate::publication_receipts::collect_candidates(
                    content,
                    &mut self.published_event_ids,
                );
            }
        }
    }
}

fn normalized_snippet(text: &str, max_chars: usize) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let mut chars = normalized.chars();
    let snippet: String = chars.by_ref().take(max_chars).collect();
    Some(if chars.next().is_some() {
        format!("{snippet}…")
    } else {
        snippet
    })
}

fn bounded_background_field(value: Option<&serde_json::Value>, fallback: &str) -> String {
    let value = value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    value
        .chars()
        .take(MAX_BACKGROUND_TASK_FIELD_CHARS)
        .collect()
}

fn value_contains_buzz_message_send(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::String(text)) => text.contains("buzz messages send"),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .any(|value| value_contains_buzz_message_send(Some(value))),
        Some(serde_json::Value::Object(values)) => values
            .values()
            .any(|value| value_contains_buzz_message_send(Some(value))),
        _ => false,
    }
}

/// Translate Buzz's runtime-neutral OpenAI-compatible variables into the
/// provider variables goose consumes. Explicit goose variables retain
/// precedence; inherited process values retain precedence over persona env.
fn derived_goose_provider_env(
    command: &str,
    extra_env: &[(String, String)],
    parent_env: &HashMap<String, String>,
) -> Vec<(String, String)> {
    if std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        != Some("goose")
    {
        return Vec::new();
    }

    let resolved = |key: &str| {
        parent_env.get(key).map(String::as_str).or_else(|| {
            extra_env
                .iter()
                .rev()
                .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        })
    };
    if resolved("GOOSE_PROVIDER").map(str::trim) != Some("openai") {
        return Vec::new();
    }

    [
        ("OPENAI_COMPAT_BASE_URL", "GOOSE_PROVIDER__HOST"),
        ("OPENAI_COMPAT_API_KEY", "GOOSE_PROVIDER__API_KEY"),
    ]
    .into_iter()
    .filter_map(|(source, target)| {
        let target_is_explicit = parent_env.contains_key(target)
            || extra_env.iter().any(|(candidate, _)| candidate == target);
        if target_is_explicit {
            return None;
        }
        resolved(source)
            .filter(|value| !value.trim().is_empty())
            .map(|value| (target.to_owned(), value.to_owned()))
    })
    .collect()
}
/// Package and binary name used by Buzz's Pi ACP fork.
pub(crate) const BUZZ_PI_ACP_NAME: &str = "buzz-pi-acp";

/// An MCP server configuration passed to `session/new`.
///
/// Corresponds to the `McpServerStdio` variant in the ACP schema.
/// All four fields are **required** by the schema (`args` and `env` may be empty arrays).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
}

/// A single environment variable for an MCP server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Stop reason returned by `session/prompt` when the agent finishes a turn.
///
/// Maps to the `stopReason` field in the `SessionPromptResponse`.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// Agent completed the turn normally (`"end_turn"`).
    EndTurn,
    /// Turn was cancelled via `session/cancel` (`"cancelled"`).
    Cancelled,
    /// Agent hit its token limit (`"max_tokens"`).
    MaxTokens,
    /// Agent hit its per-turn request limit (`"max_turn_requests"`).
    MaxTurnRequests,
    /// Agent refused the prompt (`"refusal"`).
    /// Note: refused turns are dropped from history by the agent.
    Refusal,
}

impl StopReason {
    /// Parse a `stopReason` string from the ACP wire format.
    ///
    /// Matching is case-insensitive so agents that send `"END_TURN"` or
    /// `"Cancelled"` are handled correctly without a protocol error.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "end_turn" => Some(Self::EndTurn),
            "cancelled" => Some(Self::Cancelled),
            "max_tokens" => Some(Self::MaxTokens),
            "max_turn_requests" => Some(Self::MaxTurnRequests),
            "refusal" => Some(Self::Refusal),
            _ => None,
        }
    }
}

/// Errors that can occur in the ACP client.
#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Agent process exited unexpectedly")]
    AgentExited,

    #[error("Idle timeout — no agent activity for {0:?}")]
    IdleTimeout(std::time::Duration),

    #[error("Hard turn timeout exceeded (silence {silence:?})")]
    HardTimeout { silence: std::time::Duration },

    #[error("Agent did not stop within {0:?} after cancellation")]
    CancelDrainTimeout(std::time::Duration),

    #[error("Request timeout — agent did not respond within {0:?}")]
    Timeout(std::time::Duration),

    #[error("Write timeout — agent stopped reading stdin (blocked for {0:?})")]
    WriteTimeout(std::time::Duration),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Agent reported error (code {code}): {message}")]
    AgentError { code: i64, message: String },

    #[error("Agent adapter returned a provider/runtime failure as terminal output")]
    AgentRuntimeFailure,
}

/// Build an [`AcpError::AgentError`] from a JSON-RPC error object,
/// preserving the numeric code. When the `message` field is missing or
/// non-string, fall back to the full JSON object so provider-specific
/// detail (e.g. a `data` field) is not lost.
fn agent_error_from_json(error: &serde_json::Value) -> AcpError {
    let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
    let message = match error.get("message").and_then(|m| m.as_str()) {
        Some(m) => m.to_string(),
        None => error.to_string(),
    };
    AcpError::AgentError { code, message }
}

fn build_initialize_params() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": 2,
        "clientCapabilities": build_client_capabilities(),
        "clientInfo": {
            "name": "buzz-acp",
            "version": env!("CARGO_PKG_VERSION")
        },
    })
}

/// ACP client that owns an agent subprocess and communicates over its stdio.
///
/// One `AcpClient` per agent process. Multiple sessions can be created on the
/// same client via repeated calls to [`session_new`](AcpClient::session_new).
pub struct AcpClient {
    /// Private MCP launch files, retained for reconnects until session teardown.
    mcp_launch_files: HashMap<String, crate::mcp_launch::LaunchFiles>,
    /// The agent child process (kept alive to prevent zombie).
    child: Child,
    /// Write end of the agent's stdin pipe.
    stdin: ChildStdin,
    /// Framed reader over the agent's stdout pipe (line-oriented, bounded).
    /// Uses `LinesCodec::new_with_max_length` to enforce MAX_LINE_SIZE at the
    /// read level — prevents OOM from rogue agents writing infinite non-newline bytes.
    reader: FramedRead<ChildStdout, LinesCodec>,
    /// Monotonically increasing JSON-RPC request id counter.
    /// Harness-generated IDs are always numeric.
    next_id: u64,
    /// The id of a `session/request_permission` request that has been received
    /// but not yet responded to. Stored as `serde_json::Value` because JSON-RPC 2.0
    /// permits both numeric and string IDs from the agent.
    /// Used by [`cancel_with_cleanup`](AcpClient::cancel_with_cleanup) to send
    /// a `cancelled` outcome before the agent returns from `session/prompt`.
    pending_permission_id: Option<serde_json::Value>,
    /// Whether we have already sent a response to the pending permission request.
    /// Guards against double-response if a timeout fires after the allow_once
    /// response was written but before `pending_permission_id` was cleared.
    permission_responded: bool,
    /// The JSON-RPC id of the most recently sent `session/prompt` request.
    /// Used by [`cancel_with_cleanup`] to drain the correct response.
    /// Set in [`session_prompt_with_idle_timeout`]; consumed in [`cancel_with_cleanup`].
    last_prompt_id: Option<u64>,
    /// Hard deadline for the current turn, set by `session_prompt_with_idle_timeout`.
    /// Inherited by `cancel_with_cleanup` so the drain loop shares the same budget
    /// rather than starting a fresh timer (prevents double-jeopardy).
    current_hard_deadline: Option<tokio::time::Instant>,
    /// Optional local observer feed used by the desktop app.
    observer: Option<ObserverHandle>,
    /// Pool slot index for this agent process.
    observer_agent_index: Option<usize>,
    /// Best-effort context attached to raw ACP wire events.
    observer_context: ObserverContext,
    /// Most recently observed `_meta.goose.activeRunId` from a
    /// `session/update` notification of kind `session_info_update`.
    ///
    /// Both goose and buzz-agent emit `session_info_update` with this field;
    /// goose emits it whenever it starts or clears an active prompt run
    /// (`crates/goose/src/acp/server.rs:2277` `send_active_run_update`).
    /// Required as `expectedRunId` when calling the non-standard
    /// `_goose/unstable/session/steer` method to inject a message into an
    /// in-flight turn without cancelling it.
    ///
    /// `None` until the first `session_info_update` arrives, or after the
    /// run clears (goose/buzz-agent emit `activeRunId: null` at end of turn).
    /// Other agents may leave this unset — readers must treat `None` as
    /// "no active run to steer into" and fall back to cancel+merge.
    active_run_id: Option<String>,
    /// Whether the agent advertised `_meta.steering.supported: true` in its
    /// `initialize` response, meaning it implements the cross-adapter
    /// [`ACP_STEER_METHOD`] extension.
    ///
    /// Set once by [`initialize`](Self::initialize); `false` for agents that
    /// omit the key. This is the **only** gate on writing an
    /// [`ACP_STEER_METHOD`] request. It must never be replaced by error-code
    /// probing: codex-acp answers unrecognized extension methods with `{}` —
    /// a JSON-RPC *success*, not `-32601` — which the main loop would read as
    /// a delivered steer and drop the user's message from the queue.
    steering_supported: bool,
    /// Per-turn channel for receiving goose-native non-cancelling steer
    /// requests from the main loop. Installed by
    /// [`install_steer_rx`](Self::install_steer_rx) at dispatch and
    /// consumed (via `take()`) by `session_prompt_with_idle_timeout` so it
    /// is dropped at scope exit alongside the turn it served. `None`
    /// outside of a goose-native turn — the read loop's steer arm is
    /// disabled in that case.
    steer_rx: Option<tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>>,
    /// Usage tracker for goose/buzz-agent's cumulative notification format.
    goose_usage: UsageTracker,
    /// Evidence used to distinguish an intentional silent completion from a
    /// failed or missing channel publication.
    turn_delivery_evidence: TurnDeliveryEvidence,
    /// Bounded text sink used only by the private sleep-summary turn. While
    /// armed, agent message chunks are collected here instead of being logged
    /// as ordinary channel-facing final output.
    private_capture: Option<String>,
    /// Per-turn prompt-response usage and Claude's optional cumulative cost.
    standard_usage: StandardUsageTracker,
    /// Known adapter identity for prompt-response usage mapping.
    standard_adapter: Option<StandardAdapterKind>,
    /// True when initialize identified claude-agent-acp, whose session/new
    /// metadata can request filtered raw SDK lifecycle events.
    claude_adapter: bool,
    /// Consumer-side monitoring mode: which lifecycle signal this client
    /// trusts (raw seam, typed snapshots, or none), per BackgroundMode docs.
    background_mode: BackgroundMode,
    /// Latest pending task set for each Claude session. Entries are removed on
    /// an empty `background_tasks_changed` update and bounded defensively.
    background_tasks: HashMap<String, SessionBackgroundTasks>,
}

/// Recursively merge `overlay` into `base`, with `overlay` winning on scalar/shape
/// collisions.  When both sides have an object for the same key, the merge recurses so
/// unrelated nested keys from `base` are preserved.
fn deep_merge(
    base: &mut serde_json::Map<String, serde_json::Value>,
    overlay: serde_json::Map<String, serde_json::Value>,
) {
    for (k, overlay_val) in overlay {
        match base.get_mut(&k) {
            Some(serde_json::Value::Object(base_obj))
                if matches!(overlay_val, serde_json::Value::Object(_)) =>
            {
                // Both sides are objects — recurse to preserve unrelated nested keys.
                if let serde_json::Value::Object(overlay_obj) = overlay_val {
                    deep_merge(base_obj, overlay_obj);
                }
            }
            _ => {
                // Scalar, array, type mismatch, or new key — overlay wins.
                base.insert(k, overlay_val);
            }
        }
    }
}

/// Build the merged `CODEX_CONFIG` environment-variable value for a Codex agent spawn.
///
/// Returns `Some(json_string)` when `has_generated_codex_config` is true (Buzz injected a
/// `CODEX_CONFIG` entry via `codex_network_env()`), `None` otherwise.
///
/// # Merge contract (when `has_generated_codex_config` is true)
///
/// 1. **Persona base** — the first `CODEX_CONFIG` value in `extra_env` is taken as
///    the base object (all keys preserved, recursively).  When there is no persona entry,
///    the generated entry serves as the base.
/// 2. **Generated overlay** — all subsequent `CODEX_CONFIG` entries are deep-merged into
///    the base so unrelated nested persona keys survive.
/// 3. **Parent-env precedence** — if `parent_codex_config` is `Some`, its keys are
///    deep-merged into the result (parent wins on colliding keys at every nesting level;
///    unrelated keys from either side survive).
/// 4. **Forced overlay** — `sandbox_workspace_write.network_access = true` is applied
///    last so relay access is guaranteed regardless of operator / persona config.
///
/// When `has_generated_codex_config` is false, the function returns `None` and the
/// caller handles any persona-supplied `CODEX_CONFIG` with ordinary operator-wins
/// semantics (no merging, no sandbox widening).
///
/// # Errors
///
/// Returns `Err(AcpError::Protocol)` when `has_generated_codex_config` is true and any
/// `CODEX_CONFIG` value is not valid JSON or is not a JSON object, or when
/// `sandbox_workspace_write` is present but not an object after all merges.
pub(crate) fn build_codex_config_env(
    extra_env: &[(String, String)],
    parent_codex_config: Option<&str>,
    has_generated_codex_config: bool,
) -> Result<Option<String>, AcpError> {
    // Without an explicit Buzz-generated overlay signal, skip the merge entirely.
    // Any persona CODEX_CONFIG is handled by the caller with operator-wins semantics.
    if !has_generated_codex_config {
        return Ok(None);
    }

    // Collect all CODEX_CONFIG entries from extra_env in order.
    let codex_entries: Vec<&str> = extra_env
        .iter()
        .filter(|(k, _)| k == "CODEX_CONFIG")
        .map(|(_, v)| v.as_str())
        .collect();

    if codex_entries.is_empty() {
        // has_generated_codex_config is true but no entry in extra_env — shouldn't
        // happen in practice, but treat as no-op rather than panic.
        return Ok(None);
    }

    // Parse all entries; first one is the persona base (or the generated entry if no
    // persona CODEX_CONFIG was set), rest are additional generated entries.
    let mut parsed_entries: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
    for (i, raw) in codex_entries.iter().enumerate() {
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(obj)) => parsed_entries.push(obj),
            Ok(_) => {
                let source = if i == 0 { "persona" } else { "generated" };
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG {source} value is valid JSON but not an object"
                )));
            }
            Err(e) => {
                let source = if i == 0 { "persona" } else { "generated" };
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG {source} value is not valid JSON: {e}"
                )));
            }
        }
    }

    // Start from first entry, deep-merge remaining entries.
    let mut base = parsed_entries.remove(0);
    for overlay in parsed_entries {
        deep_merge(&mut base, overlay);
    }

    // Deep-merge parent env (parent wins on colliding keys at every nesting level).
    if let Some(parent_raw) = parent_codex_config {
        match serde_json::from_str::<serde_json::Value>(parent_raw) {
            Ok(serde_json::Value::Object(parent_obj)) => {
                deep_merge(&mut base, parent_obj);
            }
            Ok(_) => {
                return Err(AcpError::Protocol(
                    "CODEX_CONFIG in parent environment is valid JSON but not an object".into(),
                ));
            }
            Err(e) => {
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG in parent environment is not valid JSON: {e}"
                )));
            }
        }
    }

    // Force sandbox_workspace_write.network_access = true (our invariant, always wins).
    let sws_entry = base
        .entry("sandbox_workspace_write")
        .or_insert_with(|| serde_json::json!({}));
    match sws_entry {
        serde_json::Value::Object(sws_obj) => {
            sws_obj.insert("network_access".to_string(), serde_json::Value::Bool(true));
        }
        other => {
            return Err(AcpError::Protocol(format!(
                "CODEX_CONFIG sandbox_workspace_write is not an object (got {}); \
                 cannot set network_access=true",
                other
            )));
        }
    }

    Ok(Some(serde_json::Value::Object(base).to_string()))
}

/// goose's non-standard mid-turn steer method. Requires `expectedRunId`, so it
/// is only usable once a `session_info_update` has supplied
/// `_meta.goose.activeRunId`. Emitted by goose and buzz-agent only.
const GOOSE_STEER_METHOD: &str = "_goose/unstable/session/steer";

/// The cross-adapter mid-turn steer method, shipped by claude-agent-acp
/// (`src/acp-agent.ts:200`) and codex-acp (`src/AcpExtensions.ts:11`).
/// Params are `{sessionId, prompt}` — no run id — and the result is
/// `{outcome}`. Gated on [`AcpClient::steering_supported`].
const ACP_STEER_METHOD: &str = "_session/steering";

/// `outcome` value meaning the steer was applied to the turn Buzz is waiting
/// on, which therefore keeps running.
const STEER_OUTCOME_INJECTED: &str = "injected";

/// `outcome` value meaning the turn Buzz was steering had already finished, so
/// the adapter began a fresh turn carrying the message. Still a delivery
/// success, but the awaited turn is over — see the steer-response arm for why
/// this must not renew the hard deadline.
const STEER_OUTCOME_STARTED_NEW_TURN: &str = "startedNewTurn";

/// Which wire method carried an in-flight steer request, recorded so the
/// response arm decodes the shape that method actually returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteerTransport {
    /// [`GOOSE_STEER_METHOD`] — any success result is a delivered steer.
    Goose,
    /// [`ACP_STEER_METHOD`] — success carries an `outcome` that must be
    /// positively recognized before the steer counts as delivered.
    AcpExtension,
}

fn build_client_capabilities() -> serde_json::Value {
    serde_json::json!({
        // Signal to ACP adapters that Buzz can hand users to terminal-native
        // auth flows. Adapters decide which auth methods to expose; Buzz does
        // not hardcode vendor login commands from this capability.
        "auth": {
            "terminal": true
        },
        // Signal to goose that we handle `_goose/unstable/session/update`
        // notifications. Without this the custom notification is suppressed
        // on goose's side and usage data is never emitted.
        "_meta": {
            "goose": {
                "customNotifications": true
            },
            // Non-standard extension used by claude-agent-acp to advertise the
            // exact terminal login argv for subscription auth. Unknown `_meta`
            // keys are ignored by other adapters.
            "terminal-auth": true
        }
    })
}

/// Enforce Desktop's explicit local binding at the final provider child boundary.
/// No caller-supplied extra env or credential override may outrank that binding.
fn enforce_subscription_child_env(
    child: &mut tokio::process::Command,
    command: &str,
    args: &[String],
) -> Result<(), AcpError> {
    let Some(account_id) = std::env::var_os("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID") else {
        return Ok(());
    };
    if account_id.is_empty() {
        return Err(AcpError::Protocol(
            "Empty subscription account binding".into(),
        ));
    }
    let harness = std::env::var("BUZZ_ACP_SUBSCRIPTION_HARNESS")
        .map_err(|_| AcpError::Protocol("Missing subscription harness binding".into()))?;
    let token_kind = match std::env::var("BUZZ_ACP_SUBSCRIPTION_KIND") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(kind) if kind == "login" => false,
        Ok(kind) if kind == "token" && harness == "claude" => true,
        _ => {
            return Err(AcpError::Protocol(
                "Invalid subscription account kind".into(),
            ))
        }
    };
    let home = std::env::var_os("BUZZ_ACP_SUBSCRIPTION_HOME")
        .ok_or_else(|| AcpError::Protocol("Missing subscription credential home".into()))?;
    let identity = crate::config::normalize_agent_command_identity(command);
    let matches = match harness.as_str() {
        "claude" => matches!(
            identity.as_str(),
            "claude-agent-acp" | "claude-code-acp" | "claude-code" | "claudecode"
        ),
        "codex" => matches!(identity.as_str(), "codex" | "codex-acp"),
        "grok" => identity == "grok",
        _ => false,
    };
    if !matches {
        return Err(AcpError::Protocol(
            "Subscription binding does not match the launched adapter".into(),
        ));
    }
    if args.iter().any(|arg| {
        [
            "--resume",
            "--continue",
            "--fork-session",
            "resume",
            "fork",
            "--settings",
            "--config",
            "-c",
            "--api-key",
            "--leader-socket",
        ]
        .iter()
        .any(|blocked| arg == blocked || arg.starts_with(&format!("{blocked}=")))
    }) {
        return Err(AcpError::Protocol("Subscription account cannot use authentication/config overrides or resume another provider session".into()));
    }
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_ANTHROPIC_AWS",
        "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
        "CLAUDE_CODE_USE_MANTLE",
        "ANTHROPIC_PROFILE",
        "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
        "CLAUDE_CODE_GATEWAY_TOKEN_FILE_DESCRIPTOR",
        "CLAUDE_CODE_HOST_AUTH_ENV_VAR",
        "CLAUDE_CODE_HOST_CREDS_FILE",
        "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
        "ANTHROPIC_UNIX_SOCKET",
        "ANTHROPIC_CUSTOM_HEADERS",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT_ID",
        "CODEX_API_KEY",
        "CODEX_API_KEY_FILE_DESCRIPTOR",
    ] {
        child.env_remove(name);
    }
    match harness.as_str() {
        "claude" => {
            let config_home =
                std::env::var_os("BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME").ok_or_else(|| {
                    AcpError::Protocol("Missing validated Claude configuration home".into())
                })?;
            child.env("CLAUDE_CONFIG_DIR", config_home);
            if token_kind {
                subscription_token::supply_token(child, std::path::Path::new(&home))?;
            } else {
                child.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", home);
            }
        }
        "codex" => {
            child.env("CODEX_HOME", home);
            let existing = child
                .as_std()
                .get_envs()
                .find(|(key, _)| *key == "CODEX_CONFIG")
                .and_then(|(_, v)| v.map(|s| s.to_string_lossy().into_owned()))
                .or_else(|| std::env::var("CODEX_CONFIG").ok());
            let mut config: serde_json::Value = existing
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|_| AcpError::Protocol("Invalid subscription Codex configuration".into()))?
                .unwrap_or_else(|| serde_json::json!({}));
            validate_subscription_codex_config(&config)?;
            let map = config.as_object_mut().ok_or_else(|| {
                AcpError::Protocol("Subscription Codex config must be an object".into())
            })?;
            map.insert("cli_auth_credentials_store".into(), "file".into());
            map.insert("forced_login_method".into(), "chatgpt".into());
            child.env("CODEX_CONFIG", config.to_string());
        }
        "grok" => subscription_grok::configure(child, std::path::Path::new(&home))?,
        _ => {}
    }
    Ok(())
}
fn validate_subscription_codex_config(config: &serde_json::Value) -> Result<(), AcpError> {
    match config {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if [
                    "model_providers",
                    "api_key",
                    "apikeyhelper",
                    "env_key",
                    "base_url",
                    "bearer_token",
                    "experimental_bearer_token",
                    "anthropic_api_key",
                    "anthropic_auth_token",
                    "anthropic_base_url",
                    "claude_code_oauth_token",
                    "claude_code_oauth_token_file_descriptor",
                    "claude_code_use_bedrock",
                    "claude_code_use_vertex",
                    "claude_code_use_foundry",
                    "claude_code_use_anthropic_aws",
                    "claude_code_use_anthropic_google_cloud",
                    "claude_code_use_mantle",
                    "anthropic_profile",
                    "claude_code_api_key_file_descriptor",
                    "claude_code_gateway_token_file_descriptor",
                    "claude_code_host_auth_env_var",
                    "claude_code_host_creds_file",
                    "claude_code_provider_managed_by_host",
                    "anthropic_unix_socket",
                    "anthropic_custom_headers",
                    "openai_api_key",
                    "openai_base_url",
                ]
                .contains(&key.to_ascii_lowercase().as_str())
                    || (key == "model_provider" && value.as_str() != Some("openai"))
                {
                    return Err(AcpError::Protocol(
                        "Codex configuration conflicts with subscription authentication".into(),
                    ));
                }
                validate_subscription_codex_config(value)?;
            }
        }
        serde_json::Value::Array(items) => {
            for value in items {
                validate_subscription_codex_config(value)?
            }
        }
        _ => {}
    }
    Ok(())
}

#[path = "subscription_grok.rs"]
mod subscription_grok;
#[path = "subscription_project.rs"]
mod subscription_project;
use subscription_project::validate_subscription_project_context;

impl AcpClient {
    /// Kill the agent subprocess and wait for it to exit (no zombies).
    ///
    /// `Drop` only calls `start_kill()` (sends SIGKILL but doesn't reap).
    /// Call this when you need guaranteed cleanup — e.g., in `run_models`
    /// before process exit.
    pub async fn shutdown(&mut self) {
        // Kill the entire process group when possible. The child was spawned
        // with process_group(0), so its PID == its PGID. Killing the group
        // ensures subprocesses (MCP servers, tool processes) are cleaned up
        // rather than orphaned to init.
        //
        // Falls back to start_kill() (direct child only) on non-Unix or if
        // the child has been polled to completion (id() returns None).
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Bounded wait: if the child doesn't exit within 5s after SIGKILL,
        // give up and let Drop/OS handle it. An unbounded wait here would
        // wedge the harness during respawn or shutdown if a child is stuck.
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::debug!("child wait error after kill: {e}"),
            Err(_) => tracing::warn!("child did not exit within 5s after SIGKILL — abandoning"),
        }
        self.mcp_launch_files.clear();
    }

    /// A token switch cannot acknowledge teardown when child exit is unconfirmed.
    pub async fn shutdown_for_account_switch(&mut self) -> Result<(), AcpError> {
        // Capture the group before polling: an exited adapter can still have live
        // tool descendants, and try_wait clears the child's process identity.
        let Some(pid) = self.child.id() else {
            self.mcp_launch_files.clear();
            return Ok(());
        };
        #[cfg(unix)]
        let stopped = {
            use nix::sys::signal::{killpg, Signal};
            use nix::unistd::Pid;
            matches!(
                killpg(Pid::from_raw(pid as i32), Signal::SIGKILL),
                Ok(()) | Err(nix::errno::Errno::ESRCH)
            )
        };
        #[cfg(not(unix))]
        let stopped = kill_process_group(pid);
        if !stopped {
            return Err(AcpError::Protocol(
                "Could not stop the old account's adapter process group".into(),
            ));
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait())
            .await
            .map_err(|_| AcpError::Protocol("Old account adapter exit is unconfirmed".into()))??;
        self.mcp_launch_files.clear();
        Ok(())
    }

    /// Spawn the agent binary as a subprocess and connect to its stdio pipes.
    ///
    /// `has_generated_codex_config` must be true when `codex_network_env()` successfully
    /// injected a `CODEX_CONFIG` entry into `extra_env`.  The spawn path uses it to
    /// trigger the recursive merge + forced `network_access=true` in
    /// `build_codex_config_env`.  Pass `false` for test spawns and non-Codex agents.
    ///
    /// After spawning, call [`initialize`](Self::initialize) before any other method.
    pub async fn spawn(
        command: &str,
        args: &[String],
        extra_env: &[(String, String)],
        has_generated_codex_config: bool,
    ) -> Result<Self, AcpError> {
        use std::process::Stdio;

        if has_generated_codex_config {
            // Workspace-local exact-prefix allowlist for `buzz messages send`.
            // auto_review stays on for every other command (Nathan 08-11).
            match std::env::current_dir() {
                Ok(cwd) => match crate::config::install_codex_relay_send_allowlist(&cwd) {
                    Ok(path) => tracing::info!(
                        target: "acp::codex",
                        path = %path.display(),
                        "installed Codex execpolicy allowlist for buzz messages send"
                    ),
                    Err(error) => tracing::warn!(
                        target: "acp::codex",
                        error = %error,
                        "failed to install Codex execpolicy allowlist for buzz messages send"
                    ),
                },
                Err(error) => tracing::warn!(
                    target: "acp::codex",
                    error = %error,
                    "cannot resolve cwd to install Codex execpolicy allowlist"
                ),
            }
        }

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        if crate::config::normalize_agent_command_identity(command) == BUZZ_PI_ACP_NAME {
            if !args.iter().any(|arg| arg == "--") {
                cmd.arg("--");
            }
            // Desktop launches buzz-acp in the Buzz nest; adapters inherit that
            // workspace. Keep managed skills tied to launch CWD across sessions.
            cmd.arg("--skill")
                .arg(std::env::current_dir()?.join(".agents/skills"));
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherit stderr so agent logs are visible in the harness terminal.
            .stderr(Stdio::inherit())
            // Ensure the child is killed when the AcpClient is dropped (best-effort).
            // Callers MUST still call shutdown().await for guaranteed cleanup.
            .kill_on_drop(true);

        // Per-persona env vars (e.g., GOOSE_PROVIDER, BUZZ_AGENT_PROVIDER).
        // For most keys, operator precedence wins: skip injection if already set
        // in the parent environment.
        //
        // CODEX_CONFIG is handled specially via build_codex_config_env:
        //   • has_generated_codex_config=true: merge all CODEX_CONFIG entries + parent
        //     recursively and force network_access=true.
        //   • has_generated_codex_config=false: return None; any persona-supplied
        //     CODEX_CONFIG falls through to the normal operator-wins loop below.
        let has_codex_config = extra_env.iter().any(|(k, _)| k == "CODEX_CONFIG");
        let parent_codex_config = if has_generated_codex_config && has_codex_config {
            std::env::var("CODEX_CONFIG").ok()
        } else {
            None
        };
        let codex_config_value = build_codex_config_env(
            extra_env,
            parent_codex_config.as_deref(),
            has_generated_codex_config,
        )?;
        // When the merge path was not taken (None returned), any persona CODEX_CONFIG
        // entry falls through to the standard operator-wins treatment below.
        let codex_merge_active = codex_config_value.is_some();

        // Per-runtime environment defaults (e.g. Hermes MCP-startup isolation).
        // Applied first so both persona `extra_env` (below, via `Command::env`
        // key replacement) and inherited parent env (via the parent-presence
        // check) override them.
        for &(key, value) in crate::config::default_agent_env(command) {
            if std::env::var_os(key).is_none() {
                cmd.env(key, value);
            }
        }

        for (key, value) in extra_env {
            if key == "CODEX_CONFIG" && codex_merge_active {
                // Handled by build_codex_config_env; skip here to avoid double-setting.
                continue;
            }
            if std::env::var_os(key).is_none() {
                cmd.env(key, value);
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let parent_already_set = std::env::var_os("PRIME_AGENT_CODING_AGENT_DIR").is_some();
            if let Some(dir) =
                crate::config::prime_coding_agent_dir(command, extra_env, &cwd, parent_already_set)
            {
                cmd.env("PRIME_AGENT_CODING_AGENT_DIR", dir);
            }
        }
        let goose_bridge_keys = [
            "GOOSE_PROVIDER",
            "OPENAI_COMPAT_BASE_URL",
            "OPENAI_COMPAT_API_KEY",
            "GOOSE_PROVIDER__HOST",
            "GOOSE_PROVIDER__API_KEY",
        ];
        let parent_env: HashMap<String, String> = goose_bridge_keys
            .into_iter()
            .filter_map(|key| {
                std::env::var_os(key)
                    .map(|value| (key.to_owned(), value.to_string_lossy().into_owned()))
            })
            .collect();
        for (key, value) in derived_goose_provider_env(command, extra_env, &parent_env) {
            cmd.env(key, value);
        }
        if let Some(merged) = codex_config_value {
            cmd.env("CODEX_CONFIG", merged);
        }

        enforce_subscription_child_env(&mut cmd, command, args)?;

        // Spawn the agent in its own process group so SIGKILL doesn't propagate
        // to the harness's own process group on Unix.
        // tokio::process::Command::process_group is a stable tokio API (no extra imports needed).
        #[cfg(unix)]
        cmd.process_group(0);

        // Suppress the console window that Windows otherwise allocates for every
        // console-subsystem child process spawned from a GUI/non-console parent.
        configure_no_window(&mut cmd);

        let standard_adapter =
            match crate::config::normalize_agent_command_identity(command).as_str() {
                "claude-agent-acp" | "claude-code-acp" | "claude-code" | "claudecode" => {
                    Some(StandardAdapterKind::Claude)
                }
                "codex" | "codex-acp" => Some(StandardAdapterKind::Codex),
                _ => None,
            };
        let mut child = cmd.spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdout".into()))?;

        Ok(Self {
            mcp_launch_files: HashMap::new(),
            child,
            stdin,
            reader: FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_LINE_SIZE)),
            next_id: 0,
            pending_permission_id: None,
            permission_responded: false,
            last_prompt_id: None,
            current_hard_deadline: None,
            observer: None,
            observer_agent_index: None,
            observer_context: ObserverContext::default(),
            active_run_id: None,
            steering_supported: false,
            steer_rx: None,
            goose_usage: UsageTracker::default(),
            turn_delivery_evidence: TurnDeliveryEvidence::default(),
            private_capture: None,
            standard_usage: StandardUsageTracker::default(),
            standard_adapter,
            claude_adapter: false,
            background_mode: BackgroundMode::RawSeam,
            background_tasks: HashMap::new(),
        })
    }

    /// Return a snapshot of the current turn's delivery evidence.
    pub(crate) fn turn_delivery_evidence(&self) -> TurnDeliveryEvidence {
        self.turn_delivery_evidence.clone()
    }

    /// Attach a local observer feed to this ACP client.
    pub fn set_observer(&mut self, observer: Option<ObserverHandle>, agent_index: usize) {
        self.observer = observer;
        self.observer_agent_index = Some(agent_index);
    }

    /// Update metadata that will be attached to subsequent raw wire events.
    pub fn set_observer_context(&mut self, context: ObserverContext) {
        self.observer_context = context;
    }

    /// Return a clone of the observer handle, if attached.
    pub(crate) fn observer_handle(&self) -> Option<ObserverHandle> {
        self.observer.clone()
    }

    /// Return the pool slot index for this agent process.
    pub(crate) fn observer_agent_index(&self) -> Option<usize> {
        self.observer_agent_index
    }

    /// Emit a semantic event to the local observer feed, if enabled.
    pub fn observe(&self, kind: impl Into<String>, payload: serde_json::Value) {
        if let Some(observer) = &self.observer {
            observer.emit(
                kind,
                self.observer_agent_index,
                &self.observer_context,
                payload,
            );
        }
    }

    /// Send the `initialize` request and return the agent's response result value.
    ///
    /// Must be called exactly once, before any other ACP method.
    /// The caller may inspect `agentCapabilities` in the returned value.
    ///
    /// Records `_meta.steering.supported` into
    /// [`steering_supported`](Self::steering_supported) so the read loop's steer
    /// arm can choose [`ACP_STEER_METHOD`] for adapters that implement it.
    /// Parsed here rather than at each call site so no caller can forget it.
    pub async fn initialize(&mut self) -> Result<serde_json::Value, AcpError> {
        // Requesting version 2 is an intentional temporary pin — we are squatting
        // on ACP v2 ahead of the upstream ACP RFD. Revisit when that RFD merges.
        let params = build_initialize_params();
        let result = self.send_request("initialize", params).await?;
        self.claude_adapter = result
            .get("agentInfo")
            .or_else(|| result.get("serverInfo"))
            .and_then(|info| info.get("name"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(CLAUDE_AGENT_ACP_NAME));
        // Track-A consumer selection (design v2.1 L2.1): the init capability is
        // connection-scoped static. Present ⇒ fork-aware adapter ⇒ typed
        // snapshots (session/new readiness refines below; until a snapshot
        // carries "confirmed" this stays visibly unconfirmed). Absent ⇒ stock
        // adapter ⇒ raw seam. Per-session readiness can narrow but never widen
        // what init advertised.
        self.background_mode = if result.pointer("/_meta/backgroundTasks").is_some() {
            BackgroundMode::UnconfirmedTyped
        } else {
            BackgroundMode::RawSeam
        };
        self.steering_supported = result
            .pointer("/_meta/steering/supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        tracing::debug!(target: "acp::init", "initialize response: {result}");
        Ok(result)
    }

    /// Send the ACP `authenticate` request for an adapter-advertised method.
    pub async fn authenticate(&mut self, method_id: &str) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "methodId": method_id,
        });
        self.send_request("authenticate", params).await
    }

    /// Send `session/new` and return the full response alongside the session ID.
    ///
    /// `cwd` must be an absolute path. `mcp_servers` may be empty.
    ///
    /// `system_prompt` controls how the prompt text is delivered:
    ///
    /// - `None` — no system-prompt field in the request (legacy framing).
    /// - `Some(SystemPromptTransport::Field(text))` — bare `systemPrompt` field
    ///   (ACP protocol v2, buzz-agent; goose unused).
    /// - `Some(SystemPromptTransport::PiMeta(text))` — `_meta.systemPrompt`
    ///   as a replacement string for the Buzz pi-acp fork.
    /// - `Some(SystemPromptTransport::ClaudeMeta(text))` — `_meta.systemPrompt`
    ///   as `{"append": text}`, keeping claude-agent-acp's native preset intact.
    ///
    /// `session_title` rides in `_meta.sessionTitle` when `Some`; `_meta` is
    /// omitted entirely otherwise, since adapters may distinguish an absent
    /// member from a null one. Metadata prompt transports and the title are
    /// merged into a single object.
    ///
    /// Callers use [`extract_model_config_options`] and [`extract_model_state`]
    /// to pull model info from the raw result.
    pub async fn session_new_full(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<SystemPromptTransport<'_>>,
        session_title: Option<&str>,
    ) -> Result<SessionNewResponse, AcpError> {
        if let Ok(harness) = std::env::var("BUZZ_ACP_SUBSCRIPTION_HARNESS") {
            validate_subscription_project_context(&harness, std::path::Path::new(cwd))?;
        }
        let (mcp_servers, launch_files) = crate::mcp_launch::prepare(mcp_servers)?;
        let mut params = serde_json::json!({
            "cwd": cwd,
            "mcpServers": mcp_servers,
        });
        match system_prompt {
            Some(SystemPromptTransport::Field(sp)) => {
                params["systemPrompt"] = serde_json::Value::String(sp.to_owned());
            }
            Some(SystemPromptTransport::PiMeta(sp)) => {
                params["_meta"]["systemPrompt"] = serde_json::Value::String(sp.to_owned());
            }
            Some(SystemPromptTransport::ClaudeMeta(sp)) => {
                // Merge into _meta so sessionTitle (set below) is not clobbered.
                params["_meta"]["systemPrompt"] = serde_json::json!({ "append": sp });
            }
            None => {}
        }
        if let Some(title) = session_title {
            // Merge — _meta may already carry a system prompt from an adapter extension.
            params["_meta"]["sessionTitle"] = serde_json::Value::String(title.to_owned());
        }
        if self.claude_adapter && self.background_mode == BackgroundMode::RawSeam {
            // Stock claude-agent-acp (no typed capability): request only the
            // lifecycle event buzz-acp consumes. Enabling all raw SDK traffic
            // would duplicate the normal ACP stream and create an unbounded
            // compatibility surface. A fork-aware adapter never gets this —
            // the typed path replaces it.
            params["_meta"]["claudeCode"]["emitRawSDKMessages"] = serde_json::json!([{
                "type": "system",
                "subtype": "background_tasks_changed"
            }]);
        }
        let result = self.send_request("session/new", params).await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("session/new response missing sessionId".into()))?
            .to_owned();
        if let Some(files) = launch_files {
            self.mcp_launch_files.insert(session_id.clone(), files);
        }
        // Per-session readiness (design v2.1 L1.3/L2.1): refines the
        // init-selected mode but never widens it. Unarmed fires the loud
        // operator notice here; there is no raw or typed listening on that
        // session, by contract.
        match result
            .pointer("/_meta/backgroundTasks/readiness")
            .and_then(serde_json::Value::as_str)
        {
            Some("unarmed") => {
                self.background_mode = BackgroundMode::Unarmed;
                tracing::warn!(
                    target: "acp::background",
                    session_id,
                    "UNARMED: adapter reports bundled-CLI fallback; task lifecycle events cannot be observed for this session"
                );
            }
            Some("unconfirmed") => {
                if self.background_mode != BackgroundMode::RawSeam {
                    self.background_mode = BackgroundMode::UnconfirmedTyped;
                }
                tracing::info!(
                    target: "acp::background",
                    session_id,
                    "monitoring state: {} (protection NOT confirmed until adapter observes a lifecycle emission)",
                    self.background_mode.label()
                );
            }
            Some("confirmed") => {
                if self.background_mode != BackgroundMode::RawSeam {
                    self.background_mode = BackgroundMode::ConfirmedTyped;
                }
                tracing::info!(
                    target: "acp::background",
                    session_id,
                    "monitoring state: {}",
                    self.background_mode.label()
                );
            }
            Some(other) => tracing::warn!(
                target: "acp::background",
                session_id,
                "unknown backgroundTasks readiness {other:?}; retaining conservative unconfirmed typed listening"
            ),
            None => tracing::info!(
                target: "acp::background",
                session_id,
                "monitoring state: {}",
                self.background_mode.label()
            ),
        }
        // Standard adapters omit the model on usage notifications. Retain the
        // adapter's reported selection and refresh it after native changes.
        self.record_standard_session_model(&session_id, &result);
        tracing::info!(target: "acp::session", "session created: {session_id}");
        Ok(SessionNewResponse {
            session_id,
            raw: result,
        })
    }

    /// Restore the provider session selected by a durable account handoff.
    /// Failure is terminal for this handoff; never substitute session/new.
    pub async fn session_load_full(
        &mut self,
        session_id: &str,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
    ) -> Result<SessionNewResponse, AcpError> {
        if let Ok(harness) = std::env::var("BUZZ_ACP_SUBSCRIPTION_HARNESS") {
            validate_subscription_project_context(&harness, std::path::Path::new(cwd))?;
        }
        let (mcp_servers, launch_files) = crate::mcp_launch::prepare(mcp_servers)?;
        let result = self
            .send_request(
                "session/load",
                serde_json::json!({
                    "sessionId": session_id, "cwd": cwd, "mcpServers": mcp_servers,
                }),
            )
            .await?;
        if let Some(files) = launch_files {
            self.mcp_launch_files.insert(session_id.to_owned(), files);
        }
        self.record_standard_session_model(session_id, &result);
        Ok(SessionNewResponse {
            session_id: session_id.to_owned(),
            raw: result,
        })
    }

    /// Send `session/new` and return only the `sessionId` string.
    ///
    /// Convenience wrapper around [`session_new_full`].
    #[allow(dead_code)] // Public API — callers outside the harness may use this.
    pub async fn session_new(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<SystemPromptTransport<'_>>,
        session_title: Option<&str>,
    ) -> Result<String, AcpError> {
        Ok(self
            .session_new_full(cwd, mcp_servers, system_prompt, session_title)
            .await?
            .session_id)
    }

    /// Replace Goose's native system prompt after `session/new`.
    pub async fn session_set_goose_system_prompt(
        &mut self,
        session_id: &str,
        text: &str,
    ) -> Result<serde_json::Value, AcpError> {
        self.send_request(
            "_goose/unstable/session/system-prompt/set",
            serde_json::json!({
                "sessionId": session_id,
                "mode": "set",
                "key": "buzz",
                "text": text,
            }),
        )
        .await
    }

    /// Send `session/set_config_option` (stable ACP path).
    pub async fn session_set_config_option(
        &mut self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "configId": config_id,
            "value": value,
        });
        let result = self
            .send_request("session/set_config_option", params)
            .await?;
        self.record_standard_session_model(session_id, &result);
        Ok(result)
    }

    /// Send `session/set_model` (unstable ACP path).
    pub async fn session_set_model(
        &mut self,
        session_id: &str,
        model_id: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "modelId": model_id,
        });
        let result = self.send_request("session/set_model", params).await?;
        // Re-read the adapter's post-switch model rather than assuming the
        // requested `model_id` took: an adapter may resolve an alias to a
        // different id, and NIP-AM `model` must name what actually ran.
        self.record_standard_session_model(session_id, &result);
        Ok(result)
    }

    /// Record the adapter's current model from stable controls or legacy model
    /// state into the standard-adapter usage tracker. No-op for adapters that
    /// report the model on their own usage notifications (goose, buzz-agent) and
    /// for responses that carry no model state.
    fn record_standard_session_model(&mut self, session_id: &str, result: &serde_json::Value) {
        if self.standard_adapter.is_none() {
            return;
        }
        let stable = result["configOptions"].as_array().and_then(|options| {
            options.iter().find_map(|option| {
                (option["category"].as_str() == Some("model"))
                    .then(|| option["currentValue"].as_str())
                    .flatten()
                    .filter(|value| !value.is_empty())
            })
        });
        if let Some(model) = stable.or_else(|| {
            result
                .pointer("/models/currentModelId")
                .and_then(serde_json::Value::as_str)
        }) {
            self.standard_usage.record_session_model(session_id, model);
        }
    }

    /// Send `session/prompt` with idle-based timeout instead of wall-clock.
    ///
    /// The idle deadline resets on any stdout activity from the agent. The hard
    /// deadline is an absolute wall-clock cap (safety valve).
    pub async fn session_prompt_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_text: &str,
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        self.session_prompt_blocks_with_idle_timeout(
            session_id,
            std::slice::from_ref(&prompt_text),
            idle_timeout,
            max_duration,
        )
        .await
    }

    /// Run a private prompt and return its bounded agent-message text.
    ///
    /// The response is never treated as channel delivery evidence and its text
    /// is not written to the ordinary ACP stream log. Callers must still give
    /// the model an explicit no-tools instruction; this transport suppresses
    /// output capture, not tool availability inside an existing session.
    pub(crate) async fn session_prompt_capture_text(
        &mut self,
        session_id: &str,
        prompt_text: &str,
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<(StopReason, String), AcpError> {
        self.private_capture = Some(String::new());
        let result = self
            .session_prompt_with_idle_timeout(session_id, prompt_text, idle_timeout, max_duration)
            .await;
        let captured = self.private_capture.take().unwrap_or_default();
        result.map(|stop_reason| (stop_reason, captured))
    }

    /// Like [`session_prompt_with_idle_timeout`](Self::session_prompt_with_idle_timeout),
    /// but sends each entry in `prompt_blocks` as a separate text content block.
    ///
    /// Used for slash-command pass-through: ACP connectors detect commands via
    /// the **first** block's text starting with `/`, so the harness sends
    /// `["/cmd args", "<buzz context>"]` instead of one wrapped block.
    pub async fn session_prompt_blocks_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_blocks: &[&str],
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        self.prompt_blocks(session_id, prompt_blocks, idle_timeout, max_duration, true)
            .await
    }

    /// Continue the same host turn without losing its delivery or usage evidence.
    pub(crate) async fn session_report_completion(
        &mut self,
        session_id: &str,
        text: &str,
        budget: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        self.prompt_blocks(session_id, &[text], budget, budget, false)
            .await
    }

    async fn prompt_blocks(
        &mut self,
        session_id: &str,
        prompt_blocks: &[&str],
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
        new_turn: bool,
    ) -> Result<StopReason, AcpError> {
        if new_turn {
            self.turn_delivery_evidence = TurnDeliveryEvidence::default();
        }
        let params = build_prompt_params(session_id, prompt_blocks);
        let hard_deadline = tokio::time::Instant::now() + max_duration;
        self.current_hard_deadline = Some(hard_deadline);

        // Mark the usage tracker as in-flight for this turn BEFORE sending the
        // prompt so that any setup notifications recorded earlier are not
        // misattributed to this turn.
        if new_turn {
            self.goose_usage.begin_turn(session_id);
            self.standard_usage.begin_turn(session_id);
        }

        self.last_prompt_id = Some(self.next_id);
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());
        if let Err(e) = self.write_ndjson(&msg).await {
            self.last_prompt_id = None;
            self.current_hard_deadline = None;
            return Err(e);
        }

        let result = self
            .read_until_response_with_idle_timeout(
                session_id,
                id,
                idle_timeout,
                hard_deadline,
                max_duration,
            )
            .await;

        // On timeout errors, leave current_hard_deadline set so cancel_with_cleanup
        // can inherit the remaining budget. Clear it on all other outcomes.
        match &result {
            Ok(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
            }
            Err(AcpError::IdleTimeout(_) | AcpError::HardTimeout { .. }) => {
                // Leave last_prompt_id and current_hard_deadline set —
                // caller will invoke cancel_with_cleanup.
            }
            Err(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
            }
        }
        self.parse_prompt_response(session_id, &result?)
    }

    /// Send a `session/cancel` **notification** (no `id` field, no response expected).
    ///
    /// After calling this, the agent will eventually respond to the in-flight
    /// `session/prompt` with `stopReason: "cancelled"`. Use
    /// [`cancel_with_cleanup`](Self::cancel_with_cleanup) if you need to drain
    /// that response.
    ///
    /// Note: async because writing to stdin requires async I/O.
    pub async fn session_cancel(&mut self, session_id: &str) -> Result<(), AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
        });
        self.send_notification("session/cancel", params).await
    }

    /// Send `session/close` as a JSON-RPC request without waiting for its response.
    ///
    /// Session invalidation is a local state transition: callers must be able to
    /// requeue the interrupted Buzz turn even if the adapter is wedged. The pool
    /// bounds this write; unmatched responses are ignored. Owned dsh uses acknowledged close.
    pub(crate) async fn session_close_fire_and_forget(
        &mut self,
        session_id: &str,
    ) -> Result<(), AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/close",
            "params": { "sessionId": session_id },
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());
        self.write_ndjson(&msg).await?;
        self.mcp_launch_files.remove(session_id);
        Ok(())
    }

    /// Returns `true` if a `session/prompt` request is currently in flight.
    pub fn has_in_flight_prompt(&self) -> bool {
        self.last_prompt_id.is_some()
    }

    /// Most recently observed goose `_meta.goose.activeRunId` from a
    /// `session_info_update`, if any.
    ///
    /// Both goose and buzz-agent emit `session_info_update`; other agents
    /// leave this `None` for the lifetime of the client. Read directly by
    /// `read_until_response_with_idle_timeout`'s
    /// steer arm at write time (see [`crate::pool::SteerRequest`] for
    /// why the read loop owns this); production callers do not need this
    /// accessor. Kept as `pub` so tests can introspect the field.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    /// Whether the agent advertised the [`ACP_STEER_METHOD`] extension at
    /// `initialize` time (`_meta.steering.supported`).
    ///
    /// The read loop's steer arm reads the field directly; this accessor exists
    /// for the supervisor's post-initialize log line.
    pub fn steering_supported(&self) -> bool {
        self.steering_supported
    }

    /// Consume per-turn usage for NIP-AM publishing. Goose/buzz-agent is an
    /// exclusive cumulative path; standard ACP prompt usage is used only when
    /// goose emitted nothing for this turn.
    pub fn take_turn_usage(&mut self) -> Option<TurnUsage> {
        let goose_usage = self.goose_usage.take();
        let standard_usage = self.standard_usage.take();
        goose_usage.or(standard_usage)
    }

    /// Notify the usage tracker that buzz-acp just spawned a new session.
    ///
    /// Seeds a zero baseline so the first usage notification for `session_id`
    /// produces `delta_reliable: true` (turn delta == cumulative from zero).
    /// Must be called only when buzz-acp created the session via `session/new`;
    /// never when attaching to a pre-existing session.
    pub(crate) fn notify_session_spawned(&mut self, session_id: &str) {
        self.goose_usage.seed_zero_baseline(session_id);
        self.standard_usage.seed_zero_baseline(session_id);
    }

    /// Install a per-turn steer request channel for goose-native
    /// non-cancelling mid-turn delivery.
    ///
    /// Called by the dispatch path immediately before
    /// [`session_prompt_with_idle_timeout`] for all prompt tasks.
    /// The matching `Sender` is stored in `TaskMeta.steer_tx` for the
    /// main loop's mode-gate fork to drive.
    ///
    /// Panics if a receiver is already installed — there is exactly one
    /// turn per `AcpClient` at a time, and stacking receivers would
    /// silently misroute steer requests across turns. The previous
    /// turn's receiver must have been consumed by the read loop and
    /// dropped at scope exit before the next turn dispatches.
    pub fn install_steer_rx(&mut self, rx: tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>) {
        assert!(
            self.steer_rx.is_none(),
            "install_steer_rx: previous turn's receiver was not consumed — \
             stacking receivers would misroute steer requests across turns"
        );
        self.steer_rx = Some(rx);
    }

    /// Clear any installed steer receiver without consuming it.
    ///
    /// Called by `send_prompt_result` on every exit path of `run_prompt_task`
    /// so that `install_steer_rx`'s `is_none()` invariant holds for the next
    /// dispatch even when the turn ended before the read loop ran `take()`.
    /// Idempotent — safe to call when `steer_rx` is already `None`.
    pub fn clear_steer_rx(&mut self) {
        self.steer_rx = None;
    }

    /// Returns `true` if no steer receiver is currently installed.
    ///
    /// Test-only: used by `pool` tests to assert the post-return invariant
    /// without exposing the private field directly.
    #[cfg(test)]
    pub fn steer_rx_is_none(&self) -> bool {
        self.steer_rx.is_none()
    }

    /// Cancel a turn cleanly, handling any pending permission request first.
    ///
    /// Steps:
    /// 1. If there is a pending `session/request_permission` that hasn't been
    ///    responded to yet, respond with `outcome: "cancelled"`.
    /// 2. Send `session/cancel` notification (no id).
    /// 3. Continue reading until the `session/prompt` response arrives with `stopReason: "cancelled"`.
    ///
    /// Returns the final [`StopReason`] (almost always [`StopReason::Cancelled`]).
    pub async fn cancel_with_cleanup(
        &mut self,
        session_id: &str,
        _idle_timeout: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        // Inherit the hard deadline from the timed-out turn so the drain loop
        // doesn't start a fresh timer (prevents double-jeopardy). If the original
        // deadline is already expired or near-expired, grant a 30s floor so the
        // cancel notification has time to propagate and the agent can respond.
        let stored_deadline = self.current_hard_deadline.take();
        let min_cleanup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let hard_deadline = match stored_deadline {
            Some(d) if d > min_cleanup_deadline => d,
            Some(_) => {
                tracing::debug!(
                    "original hard deadline expired or near-expired — using 30s cleanup grace"
                );
                min_cleanup_deadline
            }
            None => {
                tracing::warn!(
                    "cancel_with_cleanup called without current_hard_deadline — using 30s fallback"
                );
                min_cleanup_deadline
            }
        };

        self.cancel_with_cleanup_until(session_id, hard_deadline)
            .await
    }

    /// Cancel a user-interrupted turn with a bounded grace window.
    ///
    /// Some ACP servers currently keep streaming after `session/cancel`. For an
    /// explicit Stop button, waiting until the original turn deadline can make
    /// cancellation look broken. This variant gives the agent a short chance to
    /// acknowledge cancellation, then returns a timeout so the caller can respawn
    /// the agent process and actually stop the work.
    ///
    /// The `grace` window is a cleanup deadline, not the turn's real max-turn
    /// wall clock — a bounded drain that expires maps to
    /// [`AcpError::CancelDrainTimeout`], never [`AcpError::HardTimeout`], so
    /// callers can distinguish "agent didn't stop in time" from a genuine
    /// configured hard-cap breach.
    pub async fn cancel_with_cleanup_grace(
        &mut self,
        session_id: &str,
        grace: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        let _ = self.current_hard_deadline.take();
        let hard_deadline = tokio::time::Instant::now() + grace;
        match self
            .cancel_with_cleanup_until(session_id, hard_deadline)
            .await
        {
            Err(AcpError::HardTimeout { .. }) => Err(AcpError::CancelDrainTimeout(grace)),
            other => other,
        }
    }

    async fn cancel_with_cleanup_until(
        &mut self,
        session_id: &str,
        hard_deadline: tokio::time::Instant,
    ) -> Result<StopReason, AcpError> {
        // Validate precondition before any side effects — fail fast if there's
        // no in-flight prompt (prevents writing permission responses or cancel
        // notifications to the agent when no prompt is active).
        let prompt_id = self.last_prompt_id.take().ok_or_else(|| {
            AcpError::Protocol("cancel_with_cleanup called with no in-flight prompt".into())
        })?;

        // Step 1: respond to any pending permission request with "cancelled",
        // but only if we haven't already responded (guards against double-response race).
        if let Some(perm_id) = self.pending_permission_id.clone() {
            if !self.permission_responded {
                let response = permission_response_cancelled(&perm_id);
                self.write_ndjson(&response).await?;
                tracing::debug!(
                    target: "acp::cancel",
                    "responded cancelled to pending permission id={perm_id}"
                );
            }
            self.pending_permission_id = None;
            self.permission_responded = false;
        }

        // Step 2: send session/cancel notification (no id)
        self.session_cancel(session_id).await?;
        tracing::info!(target: "acp::cancel", "sent session/cancel for {session_id}");
        // Use a fixed 30s idle timeout during cleanup — the cancel notification
        // needs time to propagate and the agent may go silent while winding down.
        // The separate hard_deadline bounds agents that keep producing output
        // but ignore cancellation.
        let cleanup_idle = std::time::Duration::from_secs(30);
        let remaining = hard_deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or_default();
        let result = self
            .read_until_response_with_idle_timeout(
                session_id,
                prompt_id,
                cleanup_idle,
                hard_deadline,
                remaining,
            )
            .await?;
        self.parse_prompt_response(session_id, &result)
    }

    /// Serialize `value` as a single NDJSON line and flush to the agent's stdin.
    ///
    /// Bounded by a 30-second write timeout. If the agent stops reading stdin
    /// (e.g., it's stuck or dead), the write would otherwise block forever.
    async fn write_ndjson(&mut self, value: &serde_json::Value) -> Result<(), AcpError> {
        const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let line = serde_json::to_string(value)?;
        tokio::time::timeout(WRITE_TIMEOUT, async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| AcpError::WriteTimeout(WRITE_TIMEOUT))?
        .map_err(AcpError::Io)?;
        self.observe("acp_write", value.clone());
        Ok(())
    }

    /// Default timeout for non-prompt RPCs (initialize, session/new, etc.).
    const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    /// Send a JSON-RPC request and wait for the matching response.
    ///
    /// Assigns the next available id, writes the NDJSON line to stdin,
    /// then calls [`read_until_response`](Self::read_until_response).
    ///
    /// The write phase is bounded by `WRITE_TIMEOUT` (30s) and the read phase
    /// by `REQUEST_TIMEOUT` (60s), so worst-case wall clock is ~90s. Non-prompt
    /// RPCs like `initialize` and `session/new` should complete in seconds;
    /// if they don't, the agent is likely stuck and we must not block forever.
    async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());

        // Wrap write + read in a single timeout so a hung agent can't block forever.
        // We cannot use an async block that borrows `self` mutably across two awaits
        // inside timeout(), so we sequence them with early-return on timeout.
        let timeout = Self::REQUEST_TIMEOUT;
        match tokio::time::timeout(timeout, self.write_ndjson(&msg)).await {
            Ok(result) => result?,
            Err(_) => return Err(AcpError::Timeout(timeout)),
        }

        match tokio::time::timeout(timeout, self.read_until_response(id)).await {
            Ok(result) => result,
            Err(_) => Err(AcpError::Timeout(timeout)),
        }
    }

    /// Drain any buffered lines from the agent's stdout without blocking.
    ///
    /// After a [`AcpError::Timeout`] from [`send_request`], the agent may
    /// eventually send the late response. That stale message will sit in the
    /// `BufReader` buffer and be silently skipped by the next `read_until_response`
    /// call (ID mismatch). However, if the caller wants a clean slate — e.g.
    /// before retrying the same method — they can call this to consume any
    /// buffered data with a short deadline.
    ///
    /// This is a best-effort drain: it reads until the buffer is empty or
    /// `drain_timeout` elapses, whichever comes first. Errors are ignored.
    #[allow(dead_code)] // Scaffolding for future model-switch timeout cleanup; not yet wired.
    pub async fn drain_stale_responses(&mut self, drain_timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + drain_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let read_result = tokio::time::timeout(remaining, self.reader.next()).await;
            match read_result {
                // Timeout or stream ended — buffer is empty or agent exited.
                Err(_) | Ok(None) => break,
                Ok(Some(Ok(_))) => {
                    // Consumed one buffered line; loop to drain more.
                    tracing::debug!(target: "acp::wire", "drained stale buffered line");
                }
                Ok(Some(Err(_))) => break,
            }
        }
    }

    /// Send a JSON-RPC **notification** — no `id` field, no response expected.
    ///
    /// Used for `session/cancel`. The absence of `id` is the JSON-RPC 2.0
    /// distinguisher between requests and notifications.
    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), AcpError> {
        // Notifications deliberately have NO "id" field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ (notification) {}", &serde_json::to_string(&msg).unwrap_or_default());
        self.write_ndjson(&msg).await?;
        Ok(())
    }

    /// Core message loop: read NDJSON lines until we get a response matching `expected_id`.
    ///
    /// While waiting, handles:
    /// - `session/update` notifications → logged via tracing
    /// - `session/request_permission` requests → auto-approved with `allow_once`
    /// - Any other messages → debug-logged and ignored; if they carry an `id`
    ///   (i.e. they are requests, not notifications), a JSON-RPC -32601 error is sent.
    ///
    /// Compares the incoming `id` field as a `serde_json::Value` against
    /// `json!(expected_id)` so that both numeric and string IDs work correctly.
    async fn read_until_response(
        &mut self,
        expected_id: u64,
    ) -> Result<serde_json::Value, AcpError> {
        loop {
            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit, preventing
            // OOM from rogue agents writing infinite non-newline bytes.
            let line = match self.reader.next().await {
                None => {
                    self.mcp_launch_files.clear();
                    return Err(AcpError::AgentExited);
                }
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => line,
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Only log and reset idle after we have a valid non-empty line.
            tracing::debug!(target: "acp::wire", "← {trimmed}");

            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    self.observe(
                        "acp_parse_error",
                        serde_json::json!({
                            "line": trimmed,
                            "error": e.to_string(),
                        }),
                    );
                    tracing::warn!(
                        target: "acp::wire",
                        "failed to parse line as JSON: {e} — skipping"
                    );
                    continue;
                }
            };
            self.observe("acp_read", msg.clone());

            // Check if this is a response to our expected request (has matching id
            // AND no `method` field — a `method` field means it's an agent-initiated
            // request, not a response, even if the id happens to match).
            if let Some(id) = msg.get("id") {
                if *id == serde_json::json!(expected_id) && msg.get("method").is_none() {
                    if let Some(error) = msg.get("error") {
                        return Err(agent_error_from_json(error));
                    }
                    return Ok(msg["result"].clone());
                }
            }

            // Dispatch by method name (notifications and agent-initiated requests).
            if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                match method {
                    "session/update" => {
                        let _ = self.handle_session_update(&msg);
                    }
                    "_goose/unstable/session/update" => {
                        self.handle_goose_usage_update(&msg);
                    }
                    "_claude/sdkMessage" => {
                        self.handle_claude_sdk_message(&msg);
                    }
                    "session/request_permission" => {
                        self.handle_permission_request(&msg).await?;
                    }
                    other => {
                        // If the unknown message has an id, it's a request expecting a reply.
                        // Silence would cause the agent to hang waiting for a response.
                        // Send a JSON-RPC -32601 "Method not found" error.
                        if msg.get("id").is_some() {
                            let err_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": msg["id"],
                                "error": {"code": -32601, "message": format!("Method not found: {other}")}
                            });
                            // Surface write failures — a broken pipe means the
                            // agent process is dead and continuing would hang.
                            self.write_ndjson(&err_resp).await?;
                        }
                        tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                    }
                }
            }
        }
    }

    /// Idle-aware message loop: like [`read_until_response`] but resets an idle
    /// deadline on every stdout line. Fires [`AcpError::IdleTimeout`] on silence
    /// or [`AcpError::HardTimeout`] on absolute wall-clock cap.
    ///
    /// `hard_deadline` is an absolute `Instant` (pre-computed by the caller) so
    /// that `cancel_with_cleanup` can inherit the remaining budget from the
    /// original turn rather than starting a fresh timer.
    /// Read agent messages until the response with `expected_id` arrives, or
    /// either of two timeouts fires. Returns `Result<value, IdleTimeout |
    /// HardTimeout | other>`.
    ///
    /// - `idle_timeout`: silent-agent guard, **reset on every line of valid
    ///   JSON** (and explicitly on `session/update` notifications).
    /// - `hard_deadline`: absolute wall-clock cap on the whole call, passed
    ///   in so that `cancel_with_cleanup` can inherit the remaining budget
    ///   from the original turn rather than starting a fresh timer.
    ///
    /// While reading, the loop interleaves goose-native non-cancelling steer
    /// requests via `tokio::select!`. The select uses `biased` for
    /// reader-first throughput, with a pre-select deadline check at the top
    /// of every loop iteration so a continuously-ready reader arm cannot
    /// starve the hard deadline (Max's review gate). The steer arm is
    /// guarded by `pending_steer.is_none()` so at most one steer is in
    /// flight at a time; a successful steer response is routed to the
    /// caller's oneshot ack instead of being returned as the prompt result.
    ///
    /// `session_id` is threaded in lexically by callers so the goose-native
    /// steer arm can complete `sessionId` in the steer JSON-RPC params at
    /// write time without needing access to outer state. See
    /// [`crate::pool::SteerRequest`] for why params are built here and not
    /// in the main loop.
    async fn read_until_response_with_idle_timeout(
        &mut self,
        session_id: &str,
        expected_id: u64,
        idle_timeout: std::time::Duration,
        hard_deadline: tokio::time::Instant,
        max_duration: std::time::Duration,
    ) -> Result<serde_json::Value, AcpError> {
        use tokio::time::Instant;

        // Take the per-turn steer receiver into a local so it can be
        // borrowed independently of `self.reader` inside `select!`.
        // Dropped at scope exit (return paths drain `pending_steer` first
        // so the ack_tx oneshot is never leaked silently).
        let mut steer_rx = self.steer_rx.take();

        // Tracks the in-flight steer write: `(request_id, transport, ack_tx)`.
        // While `Some`, the steer arm is gated off so we don't stack writes,
        // and a response matching `id` is routed to the ack_tx instead
        // of being treated as the prompt result. `transport` records which
        // method was written so the response arm decodes the result shape
        // that method actually returns. Drained on every return path with
        // `PromptCompletedNeutral` so callers are never left hanging.
        let mut pending_steer: Option<(
            u64,
            SteerTransport,
            tokio::sync::oneshot::Sender<crate::pool::SteerAck>,
        )> = None;

        let now = Instant::now();
        let mut idle_deadline = now + idle_timeout;
        let mut hard_deadline = hard_deadline;
        let mut last_activity_at = now;

        loop {
            // Determine which deadline fires first BEFORE sleeping — this is
            // the classification we'll use on timeout, immune to scheduler jitter.
            let idle_fires_first = idle_deadline < hard_deadline;
            let next_deadline = if idle_fires_first {
                idle_deadline
            } else {
                hard_deadline
            };

            // Pre-select deadline check — required by Max's review. Under
            // `biased`, a continuously-ready reader arm wins every poll and
            // `sleep_until(next_deadline)` is never reached, silently
            // defeating the hard-deadline guarantee for agents that keep
            // producing output (see `acp.rs:608` for why the hard deadline
            // exists). Check the classified deadline here so a steady-
            // stream agent is still bounded.
            if Instant::now() >= next_deadline {
                if let Some((_, _, ack_tx)) = pending_steer.take() {
                    // Prompt is timing out — release the withheld event via
                    // PromptCompletedNeutral (no fallback signal: there is
                    // no in-flight turn to signal once we return, and
                    // normal dispatch handles redelivery).
                    let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                }
                if idle_fires_first {
                    tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                    return Err(AcpError::IdleTimeout(idle_timeout));
                } else {
                    let silence = Instant::now().saturating_duration_since(last_activity_at);
                    tracing::warn!("hard turn timeout exceeded (silence {silence:?})");
                    return Err(AcpError::HardTimeout { silence });
                }
            }

            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit.
            let read_result = tokio::select! {
                biased;
                read_result = self.reader.next() => Some(read_result),
                // Steer arm: gated off whenever a steer write is already in
                // flight so we don't stack two writes against the same
                // process. The `async { steer_rx.as_mut()?.recv().await }`
                // wrapper produces `None` when no receiver is installed,
                // which mismatches the `Some(req)` pattern and disables the
                // branch for that iteration (no busy loop). Cancel-safe:
                // `mpsc::Receiver::recv` does not lose messages on drop.
                Some(req) = async {
                    match steer_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                }, if pending_steer.is_none() => {
                    // Selected: choose the steer transport and build its
                    // params at write time using the lexical `session_id`
                    // and the freshest `active_run_id`.
                    //
                    // `active_run_id` is updated by `session/update`
                    // notifications inside this very loop; reading it here
                    // (rather than snapshotting at dispatch) guarantees the
                    // value matches what goose's run-id check will compare
                    // against.
                    //
                    // Transport precedence:
                    //   Some(run_id)              → GOOSE_STEER_METHOD. goose
                    //     wins whenever a run id exists: `expectedRunId` is
                    //     strictly more precise about *which* run is steered.
                    //   None + steering_supported → ACP_STEER_METHOD, the
                    //     cross-adapter extension (claude-agent-acp,
                    //     codex-acp), which takes no run id.
                    //   None + !steering_supported → write nothing and ack
                    //     `ExpectedRunIdMissing`; the main loop maps this to
                    //     the universal cancel+merge `Steer` fallback.
                    //
                    // The capability flag is the ONLY gate on writing
                    // ACP_STEER_METHOD. Probing an unknown method is unsafe:
                    // codex-acp answers unrecognized extension methods with
                    // `{}` — a JSON-RPC success — which would be read as a
                    // delivered steer and silently drop the user's message.
                    let prompt_block_refs: Vec<&str> =
                        req.prompt_blocks.iter().map(String::as_str).collect();
                    let selected = match (&self.active_run_id, self.steering_supported) {
                        (Some(run_id), _) => Some((
                            SteerTransport::Goose,
                            GOOSE_STEER_METHOD,
                            build_goose_steer_params(session_id, run_id, &prompt_block_refs),
                        )),
                        (None, true) => Some((
                            SteerTransport::AcpExtension,
                            ACP_STEER_METHOD,
                            build_acp_steer_params(session_id, &prompt_block_refs),
                        )),
                        (None, false) => None,
                    };
                    match selected {
                        None => {
                            tracing::warn!(
                                "steer: no active_run_id and agent did not advertise \
                                 {ACP_STEER_METHOD} — falling back to cancel+merge"
                            );
                            let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                crate::pool::SteerError::ExpectedRunIdMissing,
                            ));
                        }
                        Some((transport, method, params)) => {
                            let id = self.next_id;
                            self.next_id += 1;
                            let msg = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "method": method,
                                "params": params,
                            });
                            tracing::debug!(
                                target: "acp::wire",
                                "→ {}",
                                serde_json::to_string(&msg).unwrap_or_default()
                            );
                            match self.write_ndjson(&msg).await {
                                Ok(()) => {
                                    pending_steer = Some((id, transport, req.ack_tx));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "steer write failed ({method}): {e} — releasing withheld event"
                                    );
                                    let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                        crate::pool::SteerError::Transport(e.to_string()),
                                    ));
                                }
                            }
                        }
                    }
                    // Loop back to the next iteration without consuming a
                    // reader line; we'll wait for either the prompt
                    // response or the steer response next.
                    None
                }
                _ = tokio::time::sleep_until(next_deadline) => {
                    // The pre-select check at the top of the next iteration
                    // would catch this anyway, but firing the deadline arm
                    // here makes the wakeup immediate (no extra reader poll
                    // round-trip when stdout is idle).
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    if idle_fires_first {
                        tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                        return Err(AcpError::IdleTimeout(idle_timeout));
                    } else {
                        let silence = Instant::now().saturating_duration_since(last_activity_at);
                        tracing::warn!("hard turn timeout exceeded (silence {silence:?})");
                        return Err(AcpError::HardTimeout { silence });
                    }
                }
            };

            // Steer arm fired (or the select selected nothing read-side this
            // iteration): no reader frame to process, loop to re-evaluate
            // deadlines and arm the next select.
            let read_result = match read_result {
                Some(r) => r,
                None => continue,
            };

            match read_result {
                None => {
                    self.mcp_launch_files.clear();
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::AgentExited);
                }
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    tracing::debug!(target: "acp::wire", "← {trimmed}");

                    let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(e) => {
                            self.observe(
                                "acp_parse_error",
                                serde_json::json!({
                                    "line": trimmed,
                                    "error": e.to_string(),
                                }),
                            );
                            tracing::warn!(
                                target: "acp::wire",
                                "failed to parse line as JSON: {e} — skipping"
                            );
                            continue;
                        }
                    };
                    self.observe("acp_read", msg.clone());

                    let activity_now = Instant::now();
                    idle_deadline = activity_now + idle_timeout;
                    last_activity_at = activity_now;

                    // Steer response routing must come BEFORE the prompt
                    // response check: a steer response is a regular
                    // JSON-RPC response (id + result/error, no method),
                    // so the matcher must disambiguate by id. Both checks
                    // share the `no method` guard.
                    if let Some(id) = msg.get("id") {
                        if msg.get("method").is_none() {
                            if let Some((steer_id, _, _)) = pending_steer.as_ref() {
                                if *id == serde_json::json!(*steer_id) {
                                    // Take the ack_tx out and route the
                                    // response. We do not return — keep
                                    // reading until the prompt response
                                    // arrives.
                                    let (_, transport, ack_tx) =
                                        pending_steer.take().expect("just checked");
                                    let ack = if let Some(error) = msg.get("error") {
                                        let code = error
                                            .get("code")
                                            .and_then(|c| c.as_i64())
                                            .unwrap_or(-1);
                                        let message = error.to_string();
                                        crate::pool::SteerAck::Err(
                                            crate::pool::SteerError::AgentError { code, message },
                                        )
                                    } else {
                                        // Success result. Whether it counts as
                                        // a delivered steer — and whether the
                                        // turn Buzz awaits is still running —
                                        // depends on the transport.
                                        let outcome = match transport {
                                            // goose returns no outcome field;
                                            // a success response means the
                                            // steer landed in the live run.
                                            SteerTransport::Goose => Some(STEER_OUTCOME_INJECTED),
                                            // The outcome must be positively
                                            // recognized. An unknown or absent
                                            // value (codex-acp answers
                                            // unrecognized ext methods with a
                                            // bare `{}`) is a rejection, never
                                            // a delivery — treating it as
                                            // success would drop the event.
                                            SteerTransport::AcpExtension => msg
                                                .pointer("/result/outcome")
                                                .and_then(|v| v.as_str())
                                                .filter(|o| {
                                                    *o == STEER_OUTCOME_INJECTED
                                                        || *o == STEER_OUTCOME_STARTED_NEW_TURN
                                                }),
                                        };
                                        match outcome {
                                            Some(STEER_OUTCOME_STARTED_NEW_TURN) => {
                                                // Delivered, but into a NEW
                                                // turn: the one this read loop
                                                // is awaiting had already
                                                // finished. Renewing the hard
                                                // deadline here would extend
                                                // the clock on a settled turn,
                                                // so leave it alone and let the
                                                // prompt response land on its
                                                // original budget.
                                                tracing::info!(
                                                    "steer accepted as {STEER_OUTCOME_STARTED_NEW_TURN}: \
                                                     awaited turn had ended — hard deadline not renewed"
                                                );
                                                crate::pool::SteerAck::Success {
                                                    session_id: session_id.to_owned(),
                                                }
                                            }
                                            Some(_) => {
                                                let renew_now = Instant::now();
                                                let new_deadline = renew_now + max_duration;
                                                if new_deadline > hard_deadline {
                                                    hard_deadline = new_deadline;
                                                    self.current_hard_deadline = Some(new_deadline);
                                                    tracing::info!(
                                                        "steer success: renewed hard deadline ({max_duration:?} from now)"
                                                    );
                                                }
                                                crate::pool::SteerAck::Success {
                                                    session_id: session_id.to_owned(),
                                                }
                                            }
                                            None => {
                                                // Report the raw string when
                                                // there is one, so logs read
                                                // `failed` not `"failed"`;
                                                // fall back to the JSON for a
                                                // non-string value.
                                                let reported = match msg.pointer("/result/outcome")
                                                {
                                                    None => "<absent>".to_string(),
                                                    Some(serde_json::Value::String(s)) => s.clone(),
                                                    Some(other) => other.to_string(),
                                                };
                                                tracing::warn!(
                                                    "steer rejected: {ACP_STEER_METHOD} returned \
                                                     unrecognized outcome {reported} — releasing \
                                                     withheld event for cancel+merge"
                                                );
                                                crate::pool::SteerAck::Err(
                                                    crate::pool::SteerError::OutcomeRejected {
                                                        outcome: reported,
                                                    },
                                                )
                                            }
                                        }
                                    };
                                    let _ = ack_tx.send(ack);
                                    continue;
                                }
                            }
                            if *id == serde_json::json!(expected_id) {
                                if let Some(error) = msg.get("error") {
                                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                                        let _ = ack_tx
                                            .send(crate::pool::SteerAck::PromptCompletedNeutral);
                                    }
                                    return Err(agent_error_from_json(error));
                                }
                                if let Some((_, _, ack_tx)) = pending_steer.take() {
                                    let _ =
                                        ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                                }
                                return Ok(msg["result"].clone());
                            }
                        }
                    }

                    // Dispatch notifications and agent-initiated requests.
                    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                        match method {
                            "session/update" => {
                                if self.handle_session_update(&msg) {
                                    let activity_now = Instant::now();
                                    idle_deadline = activity_now + idle_timeout;
                                    last_activity_at = activity_now;
                                    tracing::debug!("idle clock reset: tool call started");
                                }
                            }
                            "_goose/unstable/session/update" => {
                                self.handle_goose_usage_update(&msg);
                            }
                            "_claude/sdkMessage" => {
                                self.handle_claude_sdk_message(&msg);
                            }
                            "session/request_permission" => {
                                self.handle_permission_request(&msg).await?;
                            }
                            other => {
                                // If the unknown message has an id, it's a request expecting a reply.
                                // Silence would cause the agent to hang waiting for a response.
                                // Send a JSON-RPC -32601 "Method not found" error.
                                if msg.get("id").is_some() {
                                    let err_resp = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": msg["id"],
                                        "error": {"code": -32601, "message": format!("Method not found: {other}")}
                                    });
                                    // Surface write failures — a broken pipe means the
                                    // agent process is dead and continuing would hang.
                                    self.write_ndjson(&err_resp).await?;
                                }
                                tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Log a `session/update` notification via tracing.
    ///
    /// The discriminator field is `sessionUpdate` (not `type`) per the ACP schema.
    /// Returns `true` if the update indicates a tool call started, signaling that
    /// the idle clock should be explicitly reset (the agent will be silent while
    /// the tool executes).
    ///
    /// Takes `&mut self` (not `&self`) because some updates carry agent state
    /// the client must observe — notably goose's `session_info_update` with
    /// `_meta.goose.activeRunId`, which seeds [`active_run_id`](Self::active_run_id)
    /// so the steer arm can target `_goose/unstable/session/steer` at the
    /// correct run. Agents that never emit it (claude-agent-acp, codex-acp)
    /// leave it `None` and are steered via `_session/steering` instead, which
    /// needs no run id.
    fn handle_session_update(&mut self, msg: &serde_json::Value) -> bool {
        let update = &msg["params"]["update"];
        let update_type = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match update_type {
            "agent_message_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    if let Some(capture) = self.private_capture.as_mut() {
                        append_bounded(capture, text, MAX_PRIVATE_CAPTURE_BYTES);
                        tracing::debug!(
                            target: "acp::stream",
                            captured_bytes = capture.len(),
                            "captured private agent output"
                        );
                    } else {
                        let from_goose = update.pointer("/_meta/goose").is_some();
                        self.turn_delivery_evidence
                            .record_final_answer_chunk(text, from_goose);
                        tracing::info!(target: "acp::stream", "{text}");
                    }
                }
                false
            }
            "tool_call" => {
                self.turn_delivery_evidence.record_tool_call(update);
                let title = update
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let kind = update
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                tracing::info!(target: "acp::tool", "tool_call: {title} ({kind})");
                true
            }
            "tool_call_update" => {
                self.turn_delivery_evidence.record_tool_call_update(update);
                let tool_id = update
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                tracing::info!(target: "acp::tool", "tool_call_update: {tool_id} → {status}");
                false
            }
            "plan" => {
                tracing::info!(target: "acp::plan", "plan update received");
                false
            }
            "agent_thought_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    tracing::debug!(target: "acp::thought", "{text}");
                }
                false
            }
            "available_commands_update" => {
                // Advertised slash commands (ACP slash-commands extension).
                // Logged for observability; UI surfacing is a follow-up.
                let names: Vec<&str> = update["availableCommands"]
                    .as_array()
                    .map(|cmds| cmds.iter().filter_map(|c| c["name"].as_str()).collect())
                    .unwrap_or_default();
                tracing::info!(
                    target: "acp::update",
                    "available_commands_update: {} commands [{}]",
                    names.len(),
                    names.join(", ")
                );
                false
            }
            "session_info_update" => {
                // Track-A: a fork-aware adapter publishes bounded background-task
                // snapshots on this carrier (`_meta.backgroundTasks`). Parse is
                // fail-closed: malformed input drops the whole snapshot.
                self.handle_typed_background_snapshot(
                    update,
                    msg.pointer("/params/sessionId")
                        .and_then(serde_json::Value::as_str),
                );
                // Both goose and buzz-agent emit `session_info_update` with
                // `_meta.goose.activeRunId`: the id of the currently-active
                // prompt run, or `null` when the run has cleared. Other agents
                // don't emit this field; for them `active_run_id` stays `None`
                // and steer callers will fall back to cancel+merge.
                //
                // Per the ACP `SessionInfoUpdate` schema, `_meta` is a field
                // on the update object itself — nested inside `update`, not
                // alongside it at the params level. Goose and buzz-agent both
                // emit it at `params.update._meta.goose.activeRunId`.
                let meta = msg["params"]["update"]
                    .get("_meta")
                    .and_then(|m| m.get("goose"));
                if let Some(goose_meta) = meta {
                    match goose_meta.get("activeRunId") {
                        Some(serde_json::Value::String(run_id)) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId={run_id}"
                            );
                            self.active_run_id = Some(run_id.clone());
                        }
                        Some(serde_json::Value::Null) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId cleared"
                            );
                            self.active_run_id = None;
                        }
                        // Missing or non-string/null — leave state untouched.
                        _ => {}
                    }
                }
                false
            }
            "usage_update" => {
                self.handle_standard_usage_update(msg);
                false
            }
            "keepalive" => false,
            other => {
                tracing::debug!(target: "acp::update", "session/update: {other}");
                false
            }
        }
    }

    /// Track-A typed path: consume a `session_info_update` snapshot carrying
    /// `_meta.backgroundTasks` from a fork-aware adapter. Mirrors the raw
    /// seam's bounded store exactly (same caps, same first_observed_at
    /// preservation) but keys off the adapter-normalized camelCase shape and
    /// OBSERVES readiness promotion instead of deriving any state locally.
    fn handle_typed_background_snapshot(
        &mut self,
        update: &serde_json::Value,
        outer_session_id: Option<&str>,
    ) {
        if self.background_mode == BackgroundMode::RawSeam {
            return; // stock adapter: this carrier carries no backgroundTasks
        }
        let Some(snapshot) = update.pointer("/_meta/backgroundTasks") else {
            return; // ordinary title/updatedAt session_info_update
        };
        let Some(readiness) = snapshot
            .get("readiness")
            .and_then(serde_json::Value::as_str)
        else {
            tracing::warn!(target: "acp::background", "typed background snapshot missing readiness — dropping entire snapshot (fail-closed)");
            return;
        };
        let Some(session_id) = outer_session_id
            .or_else(|| update.get("sessionId").and_then(serde_json::Value::as_str))
            .or_else(|| update.get("session_id").and_then(serde_json::Value::as_str))
        else {
            tracing::warn!(target: "acp::background", "typed background snapshot missing sessionId — dropping entire snapshot");
            return;
        };
        // Observe the adapter's promotion; never promote or demote locally.
        match (self.background_mode, readiness) {
            (BackgroundMode::UnconfirmedTyped, "confirmed") => {
                self.background_mode = BackgroundMode::ConfirmedTyped;
                tracing::info!(target: "acp::background", session_id, "adapter promoted monitoring to confirmed");
            }
            (_, other) => {
                tracing::debug!(target: "acp::background", session_id, "snapshot readiness {other:?} observed in mode {}", self.background_mode.label())
            }
        }
        if !self.background_mode.is_listening() {
            return;
        }
        let Some(raw_tasks) = snapshot.get("tasks").and_then(serde_json::Value::as_array) else {
            // Malformed input must not clear a known-pending entry.
            tracing::warn!(target: "acp::background", session_id, "typed snapshot missing tasks array — dropping entire snapshot");
            return;
        };
        if raw_tasks.is_empty() {
            self.background_tasks.remove(session_id);
            tracing::info!(target: "acp::background", session_id, "background tasks cleared (typed)");
            return;
        }
        let now = tokio::time::Instant::now();
        let tasks = raw_tasks
            .iter()
            .take(MAX_BACKGROUND_TASKS_PER_SESSION)
            .map(|task| BackgroundTask {
                task_id: bounded_background_field(task.get("taskId"), UNKNOWN_BACKGROUND_TASK_ID),
                task_type: bounded_background_field(task.get("taskType"), "unknown"),
                description: bounded_background_field(task.get("description"), "background task"),
                first_observed_at: now,
            })
            .collect::<Vec<_>>();
        self.record_background_tasks(session_id, tasks, now);
    }

    /// Shared bounded upsert for both snapshot carriers. The session-level
    /// `first_observed_at` anchor keeps its original semantics exactly: set
    /// once when the set becomes non-empty, cleared only when it empties —
    /// the absolute anti-pinning cap on the idle-sleep hold. On top of that,
    /// each task whose id survives from the previous snapshot keeps its own
    /// first-observed timestamp so teardown notices can report per-task ages.
    fn record_background_tasks(
        &mut self,
        session_id: &str,
        tasks: Vec<BackgroundTask>,
        now: tokio::time::Instant,
    ) {
        if !self.background_tasks.contains_key(session_id)
            && self.background_tasks.len() >= MAX_BACKGROUND_TASK_SESSIONS
        {
            if let Some(oldest) = self
                .background_tasks
                .iter()
                .min_by_key(|(_, state)| state.first_observed_at)
                .map(|(id, _)| id.clone())
            {
                self.background_tasks.remove(&oldest);
                tracing::warn!(target: "acp::background", evicted_session_id = oldest, "background task session bound reached; evicted oldest entry");
            }
        }
        self.background_tasks
            .entry(session_id.to_owned())
            .and_modify(|state| {
                let prev = std::mem::take(&mut state.tasks);
                state.tasks = tasks
                    .iter()
                    .map(|task| {
                        let mut task = task.clone();
                        if task.task_id != UNKNOWN_BACKGROUND_TASK_ID {
                            if let Some(prior) = prev.iter().find(|p| p.task_id == task.task_id) {
                                task.first_observed_at = prior.first_observed_at;
                            }
                        }
                        task
                    })
                    .collect();
            })
            .or_insert(SessionBackgroundTasks {
                first_observed_at: now,
                tasks,
            });
    }

    fn handle_claude_sdk_message(&mut self, msg: &serde_json::Value) {
        // Track-A: raw seam is only trusted on stock adapters. Typed-mode
        // sessions never requested it, and unarmed sessions must not fall
        // back — ignoring here keeps a stray frame from re-arming a dead path.
        if !self.background_mode.is_listening() || self.background_mode != BackgroundMode::RawSeam {
            return;
        }
        let Some(message) = msg.pointer("/params/message") else {
            tracing::warn!(target: "acp::background", "_claude/sdkMessage missing params.message");
            return;
        };
        if message.get("type").and_then(serde_json::Value::as_str) != Some("system")
            || message.get("subtype").and_then(serde_json::Value::as_str)
                != Some("background_tasks_changed")
        {
            return;
        }
        let Some(session_id) = msg
            .pointer("/params/sessionId")
            .or_else(|| message.get("session_id"))
            .and_then(serde_json::Value::as_str)
        else {
            tracing::warn!(target: "acp::background", "background_tasks_changed missing session id");
            return;
        };
        let Some(raw_tasks) = message.get("tasks").and_then(serde_json::Value::as_array) else {
            // Malformed input must not clear a known-pending entry.
            tracing::warn!(target: "acp::background", session_id, "background_tasks_changed missing tasks array");
            return;
        };
        if raw_tasks.is_empty() {
            self.background_tasks.remove(session_id);
            tracing::info!(target: "acp::background", session_id, "background tasks cleared");
            return;
        }

        let now = tokio::time::Instant::now();
        let tasks = raw_tasks
            .iter()
            .take(MAX_BACKGROUND_TASKS_PER_SESSION)
            .map(|task| BackgroundTask {
                task_id: bounded_background_field(task.get("task_id"), UNKNOWN_BACKGROUND_TASK_ID),
                task_type: bounded_background_field(task.get("task_type"), "unknown"),
                description: bounded_background_field(task.get("description"), "background task"),
                first_observed_at: now,
            })
            .collect::<Vec<_>>();
        self.record_background_tasks(session_id, tasks, now);
        tracing::info!(
            target: "acp::background",
            session_id,
            task_count = raw_tasks.len().min(MAX_BACKGROUND_TASKS_PER_SESSION),
            "background tasks pending"
        );
    }

    /// Current consumer-side monitoring label, for teardown-time surfacing.
    pub(crate) fn background_mode_label(&self) -> &'static str {
        self.background_mode.label()
    }

    pub(crate) fn pending_background_tasks(&self) -> Vec<PendingBackgroundTasks> {
        self.background_tasks
            .iter()
            .map(|(session_id, state)| PendingBackgroundTasks {
                session_id: session_id.clone(),
                first_observed_at: state.first_observed_at,
                tasks: state.tasks.clone(),
                monitoring_state: self.background_mode.label(),
            })
            .collect()
    }

    /// Drain notifications emitted while no prompt request owns the ACP client.
    /// This is intentionally non-blocking and bounded; it is called from the
    /// lazy-pool reaper before deciding whether process teardown is safe.
    pub(crate) async fn drain_idle_notifications(&mut self) -> Result<usize, AcpError> {
        const MAX_DRAINED_FRAMES: usize = 512;
        let mut drained = 0;
        while drained < MAX_DRAINED_FRAMES {
            let Some(read_result) = self.reader.next().now_or_never() else {
                break;
            };
            let line = match read_result {
                None => {
                    self.mcp_launch_files.clear();
                    return Err(AcpError::AgentExited);
                }
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(error)) => return Err(AcpError::Io(std::io::Error::other(error))),
                Some(Ok(line)) => line,
            };
            drained += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            tracing::debug!(target: "acp::wire", "← {trimmed}");
            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(error) => {
                    self.observe(
                        "acp_parse_error",
                        serde_json::json!({"line": trimmed, "error": error.to_string()}),
                    );
                    tracing::warn!(target: "acp::wire", "failed to parse idle line as JSON: {error} — skipping");
                    continue;
                }
            };
            self.observe("acp_read", msg.clone());
            match msg.get("method").and_then(serde_json::Value::as_str) {
                Some("session/update") => {
                    let _ = self.handle_session_update(&msg);
                }
                Some("_goose/unstable/session/update") => self.handle_goose_usage_update(&msg),
                Some("_claude/sdkMessage") => self.handle_claude_sdk_message(&msg),
                Some("session/request_permission") => self.handle_permission_request(&msg).await?,
                Some(other) => {
                    if msg.get("id").is_some() {
                        let response = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "error": {"code": -32601, "message": format!("Method not found: {other}")}
                        });
                        self.write_ndjson(&response).await?;
                    }
                    tracing::debug!(target: "acp::wire", "ignoring unknown idle method: {other}");
                }
                None => {
                    tracing::warn!(target: "acp::wire", "ignoring unexpected idle ACP response")
                }
            }
        }
        if drained == MAX_DRAINED_FRAMES {
            tracing::warn!(target: "acp::wire", drained, "idle notification drain hit frame bound");
        }
        Ok(drained)
    }

    /// Record the standard ACP cumulative cost notification when emitted by
    /// Claude, plus the `used`/`size` context reading that rides the same
    /// notification.
    ///
    /// `used`/`size` are context *occupancy* and window size, not token
    /// accounting — they are published as NIP-AM `contextTokens` /
    /// `contextWindow` and never folded into any token counter.
    fn handle_standard_usage_update(&mut self, msg: &serde_json::Value) {
        if self.standard_adapter != Some(StandardAdapterKind::Claude) {
            return;
        }
        let session_id = match msg
            .pointer("/params/sessionId")
            .and_then(serde_json::Value::as_str)
        {
            Some(session_id) => session_id,
            None => return,
        };
        let reading = crate::usage::ContextReading {
            tokens: crate::usage::reported_context_value(
                msg.pointer("/params/update/used")
                    .and_then(serde_json::Value::as_u64),
            ),
            window: crate::usage::reported_context_value(
                msg.pointer("/params/update/size")
                    .and_then(serde_json::Value::as_u64),
            ),
        };
        self.standard_usage.record_context(session_id, reading);
        // Cost is optional on this notification; a reading without cost must
        // still land, so the context record above happens first.
        if let Some(cost) = msg
            .pointer("/params/update/cost/amount")
            .and_then(serde_json::Value::as_f64)
        {
            self.standard_usage.record_cost(session_id, cost);
        }
    }

    /// Parse a `_goose/unstable/session/update` notification and record the
    /// usage snapshot in the per-session tracker.
    ///
    /// Silently ignores malformed or non-`usage_update` variants — the
    /// notification is best-effort observability data, not a protocol
    /// requirement. Failures are logged at debug level.
    fn handle_goose_usage_update(&mut self, msg: &serde_json::Value) {
        use crate::usage::{GooseSessionUpdateNotification, GooseSessionUpdateVariant};
        let params = match msg.get("params") {
            Some(p) => p,
            None => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: missing params"
                );
                return;
            }
        };
        match serde_json::from_value::<GooseSessionUpdateNotification>(params.clone()) {
            Ok(notif) => {
                if let GooseSessionUpdateVariant::UsageUpdate(payload) = &notif.update {
                    tracing::debug!(
                        target: "acp::usage",
                        session_id = %notif.session_id,
                        input = ?payload.accumulated_input_tokens,
                        output = ?payload.accumulated_output_tokens,
                        // A subset of `input`, logged so downstream accounting can
                        // price it at the provider's cached rate. Always emitted,
                        // including as 0, so a parser can tell "no cache hits"
                        // apart from "this build predates the field".
                        cached = payload.accumulated_cached_input_tokens,
                        "goose usage update"
                    );
                    self.goose_usage.record(&notif.session_id, payload);
                }
            }
            Err(e) => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: deserialization error: {e}"
                );
            }
        }
    }

    /// Auto-approve a `session/request_permission` request from the agent.
    ///
    /// Finds the option with `kind == "allow_once"` and responds with its `optionId`.
    /// If no `allow_once` option exists, falls back to `reject_once`.
    ///
    /// **Critical:** Never hardcode `optionId` — always find it dynamically by `kind`.
    ///
    /// The request `id` is stored as `serde_json::Value` to support both numeric
    /// and string IDs per JSON-RPC 2.0.
    async fn handle_permission_request(&mut self, msg: &serde_json::Value) -> Result<(), AcpError> {
        // Extract id as a Value — JSON-RPC 2.0 allows both numeric and string IDs.
        let id = msg
            .get("id")
            .cloned()
            .ok_or_else(|| AcpError::Protocol("permission request missing id".into()))?;

        // Store pending permission id so cancel_with_cleanup can respond to it.
        self.pending_permission_id = Some(id.clone());
        // Mark as not yet responded — guards against double-response race.
        self.permission_responded = false;

        let options = msg["params"]["options"]
            .as_array()
            .ok_or_else(|| AcpError::Protocol("permission request missing options".into()))?;

        tracing::debug!(
            target: "acp::permission",
            "session/request_permission id={id}, {} options",
            options.len()
        );

        // Find allow_once by kind — NEVER hardcode optionId.
        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        let response = if let Some(opt) = allow_once {
            let option_id = opt["optionId"]
                .as_str()
                .ok_or_else(|| AcpError::Protocol("allow_once option missing optionId".into()))?;
            tracing::info!(
                target: "acp::permission",
                "auto-approving permission id={id} with allow_once optionId={option_id:?}"
            );
            permission_response_selected(&id, option_id)
        } else {
            // No allow_once — fall back to reject_once.
            tracing::warn!(
                target: "acp::permission",
                "no allow_once option found in permission request id={id}, falling back to reject_once"
            );
            let reject = options
                .iter()
                .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("reject_once"));

            if let Some(opt) = reject {
                let option_id = opt["optionId"].as_str().unwrap_or("reject");
                permission_response_selected(&id, option_id)
            } else {
                return Err(AcpError::Protocol(
                    "no suitable permission option found (neither allow_once nor reject_once)"
                        .into(),
                ));
            }
        };

        // Write the response first, then mark as responded.
        //
        // Previous ordering (flag-before-write) was intended to guard against a
        // double-response if a timeout fires between write and flag-set. However,
        // the deadlock risk is worse: if write_ndjson fails (e.g. WriteTimeout),
        // the flag would be true but no response was actually sent. Then
        // cancel_with_cleanup would see permission_responded=true, skip sending
        // the cancelled outcome, and the agent would hang waiting for a reply
        // that never arrives — a guaranteed deadlock.
        //
        // The correct fix: set the flag AFTER a successful write. The double-
        // response window (between write completion and flag-set) is negligibly
        // small and bounded by a single memory store; the deadlock window was
        // unbounded.
        self.write_ndjson(&response).await?;
        self.permission_responded = true;
        self.pending_permission_id = None;
        Ok(())
    }

    /// Parse a completed prompt response and retain its optional per-turn usage.
    fn parse_prompt_response(
        &mut self,
        session_id: &str,
        result: &serde_json::Value,
    ) -> Result<StopReason, AcpError> {
        let stop_reason = self.parse_stop_reason(result)?;
        if let Some(adapter) = self.standard_adapter {
            match serde_json::from_value::<PromptResponseUsage>(result["usage"].clone()) {
                Ok(usage) => self
                    .standard_usage
                    .record_prompt_usage(session_id, usage, adapter),
                Err(_) if result.get("usage").is_some() => tracing::debug!(
                    target: "acp::usage",
                    "session/prompt response contained malformed standard usage"
                ),
                Err(_) => {}
            }
        }
        Ok(stop_reason)
    }

    /// Parse `stopReason` from a `session/prompt` result value.
    fn parse_stop_reason(&self, result: &serde_json::Value) -> Result<StopReason, AcpError> {
        let raw = result["stopReason"].as_str().ok_or_else(|| {
            AcpError::Protocol("session/prompt response missing stopReason".into())
        })?;
        StopReason::from_str(raw)
            .ok_or_else(|| AcpError::Protocol(format!("unknown stopReason: {raw:?}")))
    }
}

fn append_bounded(target: &mut String, value: &str, max_bytes: usize) {
    if target.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes - target.len();
    if value.len() <= remaining {
        target.push_str(value);
        return;
    }
    let mut end = remaining;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&value[..end]);
}

/// Build `session/prompt` params from one or more text content blocks.
fn build_prompt_params(session_id: &str, prompt_blocks: &[&str]) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect();
    serde_json::json!({
        "sessionId": session_id,
        "prompt": blocks,
    })
}

/// Build `_goose/unstable/session/steer` params from one or more text
/// content blocks plus the freshest `expectedRunId`.
///
/// Wire shape:
/// ```json
/// { "sessionId": "...", "expectedRunId": "...", "prompt": [{"type":"text","text":"..."}, ...] }
/// ```
///
/// Called from the read-loop steer arm at write time so `expectedRunId`
/// matches goose's *current* run (it advances on each `session/update`).
/// See [`crate::pool::SteerRequest`] for why this is the read loop's job
/// and not the main loop's.
fn build_goose_steer_params(
    session_id: &str,
    expected_run_id: &str,
    prompt_blocks: &[&str],
) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "expectedRunId": expected_run_id,
        "prompt": steer_prompt_blocks(prompt_blocks),
    })
}

/// Build the params for an [`ACP_STEER_METHOD`] request.
///
/// Wire shape:
/// ```json
/// { "sessionId": "...", "prompt": [{"type":"text","text":"..."}, ...] }
/// ```
///
/// Deliberately carries **no** `expectedRunId`: the cross-adapter method
/// steers whatever turn is currently running and neither claude-agent-acp nor
/// codex-acp emits a run id to target.
fn build_acp_steer_params(session_id: &str, prompt_blocks: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "prompt": steer_prompt_blocks(prompt_blocks),
    })
}

/// Render steer body strings as ACP `text` content blocks. Shared by both
/// steer transports so the prompt shape cannot drift between them.
fn steer_prompt_blocks(prompt_blocks: &[&str]) -> Vec<serde_json::Value> {
    prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect()
}

/// Build a JSON-RPC permission response with `outcome: "selected"`.
fn permission_response_selected(id: &serde_json::Value, option_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
    })
}

/// Build a JSON-RPC permission response with `outcome: "cancelled"`.
fn permission_response_cancelled(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "cancelled" } }
    })
}

/// Full `session/new` response — session ID plus the raw JSON result.
///
/// Callers use the extractor helpers to pull model info from `raw`.
pub struct SessionNewResponse {
    pub session_id: String,
    /// The full `result` value from the JSON-RPC response.
    pub raw: serde_json::Value,
}

/// How to deliver a system prompt on `session/new`.
///
/// - **`Field`** — bare `systemPrompt` field (ACP protocol v2, buzz-agent).
/// - **`PiMeta`** — `_meta.systemPrompt: text`, used by the Buzz pi-acp fork
///   to replace Pi's native system prompt.
/// - **`ClaudeMeta`** — `_meta.systemPrompt: {"append": text}`, used by
///   `claude-agent-acp` to append to the adapter's own native system prompt
///   while keeping its tool-use preset intact.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemPromptTransport<'a> {
    /// Deliver as a bare top-level `systemPrompt` field.
    Field(&'a str),
    /// Deliver as `_meta.systemPrompt: text`.
    PiMeta(&'a str),
    /// Deliver as `_meta.systemPrompt: {"append": text}`.
    ClaudeMeta(&'a str),
}

/// How to switch to a particular model on a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type")]
pub enum ModelSwitchMethod {
    /// Stable: use `session/set_config_option` with these exact values.
    ConfigOption {
        config_id: String,
        option_value: String,
    },
    /// Stable model selection followed by an explicitly advertised reasoning effort.
    ConfigOptionWithEffort {
        config_id: String,
        option_value: String,
        effort: String,
    },
    /// Unstable: use `session/set_model` with this model_id.
    SetModel { model_id: String },
}

/// Extract `configOptions` entries with `category == "model"` from a `session/new` result.
///
/// Returns the raw JSON array entries. Each entry has `configId` (spelled `id`
/// by some adapters, e.g. claude-agent-acp), `displayName`,
/// `options: [{ value, displayName }]`, etc.
pub fn extract_model_config_options(result: &serde_json::Value) -> Vec<serde_json::Value> {
    result["configOptions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|opt| opt.get("category").and_then(|c| c.as_str()) == Some("model"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Extract `SessionModelState` (unstable path) from a `session/new` result.
///
/// Returns the `models` object if present: `{ currentModelId, availableModels: [...] }`.
pub fn extract_model_state(result: &serde_json::Value) -> Option<serde_json::Value> {
    result.get("models").cloned()
}

/// Whether an advertised catalog value is the canonical form of a configured
/// moving-family alias.
///
/// Claude Code has changed the spelling of extended-context selectors over
/// time (for example, `opus[1m]` became `opus` when Max subscriptions began
/// receiving 1M context automatically). Treat only the four built-in family
/// aliases as compatible across that qualifier boundary. Full/pinned model
/// IDs remain exact so `claude-opus-4-7[1m]` can never degrade to moving
/// `opus`.
fn model_catalog_value_matches(configured: &str, advertised: &str) -> bool {
    if configured == advertised {
        return true;
    }

    fn qualified_family(value: &str) -> Option<&str> {
        let (family, qualifier) = value.split_once('[')?;
        (qualifier.ends_with(']') && matches!(family, "opus" | "fable" | "sonnet" | "haiku"))
            .then_some(family)
    }

    qualified_family(configured) == Some(advertised)
        || qualified_family(advertised) == Some(configured)
}

/// Extract the `configId` for the `thought_level` category option from a
/// `session/new` result, if the adapter advertised one.
///
/// Claude Code's adapter uses `category: "thought_level"` in its `configOptions`.
/// The configId is adapter-defined (e.g. `"effort"` on claude-agent-acp) and must
/// not be hardcoded in the harness — this function discovers it at session time so
/// the spawn-scoped effort application forwards the adapter's real id. Accepts both
/// `configId` (ACP spec) and `id` (claude-agent-acp), matching the model-switch path.
pub fn extract_thought_level_config_id(result: &serde_json::Value) -> Option<String> {
    let arr = result["configOptions"].as_array()?;
    for opt in arr {
        if opt.get("category").and_then(|c| c.as_str()) == Some("thought_level") {
            let config_id = opt
                .get("configId")
                .or_else(|| opt.get("id"))
                .and_then(|v| v.as_str())?;
            return Some(config_id.to_string());
        }
    }
    None
}

/// Whether `session/new` advertised a model catalog Buzz can switch against.
///
/// Prime ACP returns only `{sessionId}` — no `configOptions`, no `models`.
/// That is not "the configured model is invalid"; the agent does not participate
/// in ACP model switching. Callers must not treat a miss as `unsupported_model`
/// or a protocol kill when this is false.
pub fn session_advertises_model_catalog(session_new_result: &serde_json::Value) -> bool {
    let has_config_options = extract_model_config_options(session_new_result)
        .iter()
        .any(|opt| {
            opt.get("options")
                .and_then(|v| v.as_array())
                .is_some_and(|options| {
                    options
                        .iter()
                        .any(|entry| entry.get("value").and_then(|v| v.as_str()).is_some())
                })
        });
    if has_config_options {
        return true;
    }
    extract_model_state(session_new_result)
        .and_then(|models| {
            models
                .get("availableModels")
                .and_then(|v| v.as_array())
                .cloned()
        })
        .is_some_and(|available| {
            available
                .iter()
                .any(|model| model.get("modelId").and_then(|v| v.as_str()).is_some())
        })
}

/// Whether `desired_model` appears in pre-extracted catalog halves.
///
/// Mirrors [`resolve_model_switch_method`]'s match, but operates on the
/// already-extracted `configOptions` (model category) and `models` state that
/// [`AgentModelCapabilities`](crate::pool::AgentModelCapabilities) caches — the
/// idle-path pre-cancel guard has those halves, not the full `session/new` JSON.
pub fn model_in_catalog(
    config_options: &[serde_json::Value],
    available_models: Option<&serde_json::Value>,
    desired_model: &str,
) -> bool {
    let in_config_options = config_options.iter().any(|config_opt| {
        config_opt
            .get("options")
            .and_then(|v| v.as_array())
            .is_some_and(|options| {
                options.iter().any(|opt| {
                    opt.get("value")
                        .and_then(|v| v.as_str())
                        .is_some_and(|advertised| {
                            model_catalog_value_matches(desired_model, advertised)
                        })
                })
            })
    });
    if in_config_options {
        return true;
    }

    available_models
        .and_then(|models| models.get("availableModels"))
        .and_then(|v| v.as_array())
        .is_some_and(|available| {
            available.iter().any(|model| {
                model
                    .get("modelId")
                    .and_then(|v| v.as_str())
                    .is_some_and(|advertised| {
                        model_catalog_value_matches(desired_model, advertised)
                    })
            })
        })
}

// ─── Drop: kill child process ─────────────────────────────────────────────────

impl Drop for AcpClient {
    fn drop(&mut self) {
        // Best-effort SIGKILL + reap. We cannot `await` in Drop (sync context).
        // Kill the process group when possible so subprocesses don't leak.
        // Callers SHOULD still call `shutdown().await` for guaranteed reaping.
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Non-blocking reap attempt — prevents zombie accumulation in the
        // common case where SIGKILL takes effect before Drop returns.
        let _ = self.child.try_wait();
    }
}

/// Send SIGKILL to an entire process group. Returns `true` if the signal was sent.
///
/// The child is spawned with `process_group(0)`, so its PID equals its PGID.
/// Killing the group ensures subprocesses (MCP servers, tool processes) are
/// cleaned up rather than orphaned to init on repeated crash-recovery cycles.
///
/// Uses `nix::sys::signal::killpg` — a safe wrapper around the POSIX `killpg`
/// syscall — so the crate's `#![deny(unsafe_code)]` policy is preserved.
#[cfg(unix)]
fn kill_process_group(pid: u32) -> bool {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    // pid == pgid because the child was spawned with process_group(0).
    killpg(Pid::from_raw(pid as i32), Signal::SIGKILL).is_ok()
}

/// Fallback for non-Unix: process-group kill not available.
/// Returns `false` so the caller falls back to `child.start_kill()`.
#[cfg(not(unix))]
fn kill_process_group(_pid: u32) -> bool {
    false
}

/// Suppress the console window that Windows otherwise allocates for every
/// console-subsystem child process spawned from a GUI (non-console) parent.
/// No-op on non-Windows platforms.
fn configure_no_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
#[path = "acp_model_usage_tests.rs"]
mod model_usage_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_parses_all_known_values() {
        assert_eq!(StopReason::from_str("end_turn"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("cancelled"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("max_tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("max_turn_requests"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("refusal"), Some(StopReason::Refusal));
    }

    #[test]
    fn stop_reason_returns_none_for_unknown() {
        assert_eq!(StopReason::from_str("unknown_value"), None);
        assert_eq!(StopReason::from_str(""), None);
        assert_eq!(StopReason::from_str("endturn"), None); // no camelCase — still unknown
    }

    #[test]
    fn stop_reason_is_case_insensitive() {
        // Agents may send uppercase or mixed-case variants — all should parse correctly.
        assert_eq!(StopReason::from_str("END_TURN"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("CANCELLED"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("Max_Tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("MAX_TURN_REQUESTS"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("Refusal"), Some(StopReason::Refusal));
    }

    #[test]
    fn find_allow_once_by_kind_not_by_option_id() {
        // optionId values are intentionally non-obvious to prove we don't hardcode them.
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"optionId": "opt-reject-42",  "name": "Reject",       "kind": "reject_once"},
            {"optionId": "opt-allow-99",   "name": "Allow once",   "kind": "allow_once"},
            {"optionId": "opt-always-7",   "name": "Always allow", "kind": "allow_always"}
        ]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        assert!(allow_once.is_some(), "should find allow_once option");
        let opt = allow_once.unwrap();
        // Found by kind, not by hardcoded optionId
        assert_eq!(opt["kind"].as_str(), Some("allow_once"));
        assert_eq!(opt["optionId"].as_str(), Some("opt-allow-99"));
    }

    #[test]
    fn find_allow_once_returns_none_when_absent() {
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"optionId": "reject-1",      "name": "Reject",        "kind": "reject_once"},
            {"optionId": "reject-always", "name": "Always reject", "kind": "reject_always"}
        ]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));

        assert!(allow_once.is_none());
    }

    #[test]
    fn find_reject_once_fallback_when_no_allow_once() {
        let options: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"optionId": "rej-x", "name": "Reject", "kind": "reject_once"}]"#,
        )
        .unwrap();

        let allow_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"));
        assert!(allow_once.is_none());

        let reject_once = options
            .iter()
            .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("reject_once"));
        assert!(reject_once.is_some());
        assert_eq!(reject_once.unwrap()["optionId"].as_str(), Some("rej-x"));
    }

    #[test]
    fn request_has_id_field() {
        let id: u64 = 42;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {}
        });
        assert!(msg.get("id").is_some(), "request must have id field");
        assert_eq!(msg["id"].as_u64(), Some(42));
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("initialize"));
    }

    #[test]
    fn notification_has_no_id_field() {
        // session/cancel is a notification — must NOT have an id field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": "sess_abc123"
            }
        });
        assert!(
            msg.get("id").is_none(),
            "notification must NOT have id field"
        );
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("session/cancel"));
    }

    #[test]
    fn initialize_request_format() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0u64,
            "method": "initialize",
            "params": {
                "protocolVersion": 2,
                "clientCapabilities": build_client_capabilities(),
                "clientInfo": {
                    "name": "buzz-acp",
                    "version": "0.1.0"
                }
            }
        });
        assert_eq!(msg["params"]["protocolVersion"].as_u64(), Some(2));
        assert_eq!(
            msg["params"]["clientInfo"]["name"].as_str(),
            Some("buzz-acp")
        );
        assert!(msg["params"]["clientCapabilities"].is_object());
        assert_eq!(
            msg["params"]["clientCapabilities"]["auth"]["terminal"].as_bool(),
            Some(true),
            "terminal auth capability must be advertised so adapters can expose terminal login methods"
        );
        assert_eq!(
            msg["params"]["clientCapabilities"]["_meta"]["goose"]["customNotifications"].as_bool(),
            Some(true),
            "goose customNotifications capability must be advertised"
        );
    }

    #[test]
    fn session_new_mcp_server_has_required_fields() {
        // Schema requires name, command, args, env — all present, args/env may be empty.
        let server = McpServer {
            name: "test-mcp".into(),
            command: "/usr/local/bin/test-mcp-server".into(),
            args: vec![],
            env: vec![
                EnvVar {
                    name: "BUZZ_RELAY_URL".into(),
                    value: "ws://localhost:3000".into(),
                },
                EnvVar {
                    name: "BUZZ_PRIVATE_KEY".into(),
                    value: "nsec1abc".into(),
                },
            ],
        };
        let serialized = serde_json::to_value(&server).unwrap();
        assert_eq!(serialized["name"].as_str(), Some("test-mcp"));
        assert_eq!(
            serialized["command"].as_str(),
            Some("/usr/local/bin/test-mcp-server")
        );
        assert!(serialized["args"].is_array());
        assert_eq!(serialized["args"].as_array().unwrap().len(), 0);
        assert!(serialized["env"].is_array());
        assert_eq!(serialized["env"].as_array().unwrap().len(), 2);
        assert_eq!(
            serialized["env"][0]["name"].as_str(),
            Some("BUZZ_RELAY_URL")
        );
    }

    #[test]
    fn session_prompt_request_format() {
        let prompt_text = "[Buzz @mention]\nChannel: test\nFrom: npub1...\nMessage: hello";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2u64,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess_abc123",
                "prompt": [
                    { "type": "text", "text": prompt_text }
                ]
            }
        });
        assert_eq!(msg["method"].as_str(), Some("session/prompt"));
        let prompt = msg["params"]["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 1);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some(prompt_text));
    }

    #[test]
    fn session_prompt_slash_command_two_block_format() {
        // Slash-command pass-through: bare command first, wrapped context second.
        let params = build_prompt_params(
            "sess_abc123",
            &[
                "/goal ship it",
                "[Buzz event: @mention]\nContent: @Eva /goal ship it",
            ],
        );
        let prompt = params["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 2);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some("/goal ship it"));
        assert!(prompt[0]["text"].as_str().unwrap().starts_with('/'));
        assert_eq!(prompt[1]["type"].as_str(), Some("text"));
    }

    #[test]
    fn permission_response_selected_format() {
        let id: u64 = 5;
        let option_id = "opt-allow-99";
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "selected",
                    "optionId": option_id
                }
            }
        });
        assert_eq!(response["id"].as_u64(), Some(5));
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("selected")
        );
        assert_eq!(
            response["result"]["outcome"]["optionId"].as_str(),
            Some("opt-allow-99")
        );
    }

    #[test]
    fn permission_response_cancelled_format() {
        let id: u64 = 5;
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "cancelled"
                }
            }
        });
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("cancelled")
        );
        // cancelled outcome has no optionId
        assert!(response["result"]["outcome"].get("optionId").is_none());
    }

    #[test]
    fn session_cancel_notification_has_session_id_in_params() {
        let session_id = "sess_xyz789";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": session_id
            }
        });
        // Must have no id (notification)
        assert!(msg.get("id").is_none());
        // Must have sessionId in params
        assert_eq!(msg["params"]["sessionId"].as_str(), Some("sess_xyz789"));
    }

    #[test]
    fn permission_request_with_string_id() {
        // Verify that permission response uses the same ID type as the request.
        // JSON-RPC 2.0 permits string IDs from the agent.
        let string_id = serde_json::json!("perm-req-001");
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id,
            "result": {
                "outcome": { "outcome": "selected", "optionId": "allow-once" }
            }
        });
        assert_eq!(response["id"], "perm-req-001");
        assert!(response["id"].is_string());
    }

    #[test]
    fn id_comparison_works_for_numeric_and_string() {
        // Verify json!(expected_id) comparison logic used in read_until_response.
        let expected_id: u64 = 3;
        let numeric_response_id = serde_json::json!(3u64);
        let string_response_id = serde_json::json!("3");

        // Numeric matches
        assert_eq!(numeric_response_id, serde_json::json!(expected_id));
        // String does NOT match numeric (correct — different types)
        assert_ne!(string_response_id, serde_json::json!(expected_id));
    }

    #[test]
    fn permission_cancelled_response_preserves_id_type() {
        // String ID from agent should be echoed back as string in cancelled response.
        let string_id = serde_json::json!("req-abc");
        let cancelled = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled["id"], string_id);
        assert!(cancelled["id"].is_string());

        // Numeric ID from agent should be echoed back as numeric.
        let numeric_id = serde_json::json!(42u64);
        let cancelled_numeric = serde_json::json!({
            "jsonrpc": "2.0",
            "id": numeric_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled_numeric["id"], numeric_id);
        assert!(cancelled_numeric["id"].is_number());
    }

    #[test]
    fn extract_model_config_options_finds_model_category() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [
                {
                    "configId": "model",
                    "category": "model",
                    "displayName": "Model",
                    "options": [
                        { "value": "claude-sonnet-4-20250514", "displayName": "Claude Sonnet 4" },
                        { "value": "claude-opus-4-20250514", "displayName": "Claude Opus 4" }
                    ]
                },
                {
                    "configId": "theme",
                    "category": "appearance",
                    "displayName": "Theme",
                    "options": [{ "value": "dark", "displayName": "Dark" }]
                }
            ]
        });
        let opts = super::extract_model_config_options(&result);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0]["configId"].as_str(), Some("model"));
    }

    #[test]
    fn extract_model_config_options_empty_when_no_config_options() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_config_options_empty_when_no_model_category() {
        let result = serde_json::json!({
            "configOptions": [
                { "configId": "theme", "category": "appearance" }
            ]
        });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_state_returns_models_object() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let ms = super::extract_model_state(&result).expect("should have models");
        assert_eq!(ms["currentModelId"].as_str(), Some("gpt-5"));
        assert_eq!(ms["availableModels"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extract_model_state_none_when_absent() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_state(&result).is_none());
    }

    #[test]
    fn session_advertises_model_catalog_false_for_session_id_only() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(
            !super::session_advertises_model_catalog(&result),
            "Prime-style session/new must not count as a catalog"
        );
    }

    #[test]
    fn session_advertises_model_catalog_true_for_config_options() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [{ "value": "model-a" }]
            }]
        });
        assert!(super::session_advertises_model_catalog(&result));
    }

    #[test]
    fn session_advertises_model_catalog_true_for_available_models() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "models": {
                "currentModelId": "local-v4",
                "availableModels": [{ "modelId": "local-v4" }]
            }
        });
        assert!(super::session_advertises_model_catalog(&result));
    }

    #[test]
    fn session_advertises_model_catalog_false_for_empty_available_models() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "models": { "currentModelId": "local-v4", "availableModels": [] }
        });
        assert!(!super::session_advertises_model_catalog(&result));
    }

    #[test]
    fn extract_thought_level_config_id_finds_config_id() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [
                { "configId": "model", "category": "model" },
                {
                    "configId": "effort",
                    "category": "thought_level",
                    "options": [{ "value": "high" }, { "value": "low" }]
                }
            ]
        });
        assert_eq!(
            super::extract_thought_level_config_id(&result).as_deref(),
            Some("effort")
        );
    }

    #[test]
    fn extract_thought_level_config_id_falls_back_to_id_key() {
        let result = serde_json::json!({
            "configOptions": [
                { "id": "effort", "category": "thought_level" }
            ]
        });
        assert_eq!(
            super::extract_thought_level_config_id(&result).as_deref(),
            Some("effort")
        );
    }

    #[test]
    fn extract_thought_level_config_id_none_without_category() {
        let result = serde_json::json!({
            "configOptions": [
                { "configId": "model", "category": "model" }
            ]
        });
        assert!(super::extract_thought_level_config_id(&result).is_none());
    }

    #[test]
    fn extract_thought_level_config_id_none_without_config_options() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_thought_level_config_id(&result).is_none());
    }

    #[test]
    fn resolve_prefers_stable_over_unstable() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [
                    { "value": "claude-sonnet-4-20250514", "displayName": "Sonnet 4" }
                ]
            }],
            "models": {
                "currentModelId": "claude-sonnet-4-20250514",
                "availableModels": [
                    { "modelId": "claude-sonnet-4-20250514", "name": "Sonnet 4" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "claude-sonnet-4-20250514");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "claude-sonnet-4-20250514".to_string(),
            })
        );
    }

    #[test]
    fn resolve_accepts_id_keyed_config_options() {
        // claude-agent-acp (observed on v0.61.0) keys config options with
        // `id` instead of the spec's `configId`. Payload mirrors its real
        // `session/new` response.
        let result = serde_json::json!({
            "configOptions": [{
                "id": "model",
                "name": "Model",
                "category": "model",
                "type": "select",
                "currentValue": "default",
                "options": [
                    { "value": "default", "name": "Default" },
                    { "value": "opus[1m]", "name": "Opus" },
                    { "value": "sonnet", "name": "Sonnet" }
                ]
            }],
            "models": null
        });
        let method = super::resolve_model_switch_method(&result, "opus[1m]");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "opus[1m]".to_string(),
            })
        );
    }

    #[test]
    fn resolve_canonicalizes_qualified_builtin_alias_to_live_catalog() {
        let result = serde_json::json!({
            "configOptions": [{
                "id": "model",
                "category": "model",
                "options": [
                    { "value": "sonnet", "name": "Sonnet" },
                    { "value": "opus", "name": "Opus" },
                    { "value": "haiku", "name": "Haiku" }
                ]
            }]
        });

        assert_eq!(
            super::resolve_model_switch_method(&result, "opus[1m]"),
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "opus".to_string(),
            })
        );
    }

    #[test]
    fn resolve_does_not_canonicalize_pinned_model_to_moving_alias() {
        let result = serde_json::json!({
            "configOptions": [{
                "id": "model",
                "category": "model",
                "options": [{ "value": "opus", "name": "Opus" }]
            }]
        });

        assert!(super::resolve_model_switch_method(&result, "claude-opus-4-7[1m]").is_none());
    }

    #[test]
    fn resolve_falls_back_to_unstable() {
        let result = serde_json::json!({
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "o3-pro");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::SetModel {
                model_id: "o3-pro".to_string(),
            })
        );
    }

    #[test]
    fn resolve_returns_none_when_no_match() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [{ "value": "claude-sonnet-4-20250514" }]
            }],
            "models": {
                "availableModels": [{ "modelId": "gpt-5" }]
            }
        });
        assert!(super::resolve_model_switch_method(&result, "nonexistent-model").is_none());
    }

    #[test]
    fn resolve_returns_none_when_no_model_info() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::resolve_model_switch_method(&result, "anything").is_none());
    }

    #[test]
    fn resolve_handles_multiple_config_options() {
        // Agent could have multiple configOptions with category "model"
        // (unlikely but defensive).
        let result = serde_json::json!({
            "configOptions": [
                {
                    "configId": "primary-model",
                    "category": "model",
                    "options": [{ "value": "model-a" }]
                },
                {
                    "configId": "fallback-model",
                    "category": "model",
                    "options": [{ "value": "model-b" }]
                }
            ]
        });
        let method = super::resolve_model_switch_method(&result, "model-b");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "fallback-model".to_string(),
                option_value: "model-b".to_string(),
            })
        );
    }

    // ── model_in_catalog tests ────────────────────────────────────────────

    #[test]
    fn model_in_catalog_true_when_in_config_options() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "category": "model",
            "options": [
                { "value": "claude-sonnet-4-20250514" },
                { "value": "claude-opus-4-20250514" }
            ]
        })];
        assert!(super::model_in_catalog(
            &config_options,
            None,
            "claude-opus-4-20250514"
        ));
    }

    #[test]
    fn model_in_catalog_true_when_in_available_models() {
        let available = serde_json::json!({
            "currentModelId": "gpt-5",
            "availableModels": [
                { "modelId": "gpt-5" },
                { "modelId": "o3-pro" }
            ]
        });
        assert!(super::model_in_catalog(&[], Some(&available), "o3-pro"));
    }

    #[test]
    fn model_in_catalog_accepts_qualified_builtin_alias_canonicalization() {
        let config_options = vec![serde_json::json!({
            "id": "model",
            "category": "model",
            "options": [{ "value": "opus" }]
        })];

        assert!(super::model_in_catalog(&config_options, None, "opus[1m]"));
    }

    #[test]
    fn model_in_catalog_false_when_absent_from_both_halves() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "options": [{ "value": "claude-sonnet-4-20250514" }]
        })];
        let available = serde_json::json!({
            "availableModels": [{ "modelId": "gpt-5" }]
        });
        assert!(!super::model_in_catalog(
            &config_options,
            Some(&available),
            "nonexistent-model"
        ));
    }

    #[test]
    fn model_in_catalog_false_when_both_halves_empty() {
        assert!(!super::model_in_catalog(&[], None, "anything"));
    }

    // ── Error variant display ─────────────────────────────────────────────

    #[test]
    fn idle_timeout_error_includes_duration() {
        let err = AcpError::IdleTimeout(std::time::Duration::from_secs(320));
        let msg = err.to_string();
        assert!(
            msg.contains("320"),
            "IdleTimeout display should include duration: {msg}"
        );
    }

    #[test]
    fn hard_timeout_error_display() {
        let err = AcpError::HardTimeout {
            silence: std::time::Duration::from_secs(120),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("Hard turn timeout"),
            "HardTimeout display: {msg}"
        );
    }

    async fn spawn_script(script: &str) -> AcpClient {
        AcpClient::spawn("bash", &["-c".into(), script.into()], &[], false)
            .await
            .expect("failed to spawn test script")
    }

    #[cfg(unix)]
    async fn spawn_named_script(name: &str, script: &str) -> (AcpClient, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "buzz-acp-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp adapter dir");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/usr/bin/env bash\n{script}\n"))
            .expect("write fake adapter");
        let mut permissions = std::fs::metadata(&path)
            .expect("adapter metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod fake adapter");
        let client = AcpClient::spawn(path.to_str().expect("utf8 path"), &[], &[], false)
            .await
            .expect("spawn named fake adapter");
        (client, dir)
    }

    /// Spawn a probe script whose file name carries a runtime identity (e.g.
    /// `hermes-acp`) and return the value of `var` as the child observed it.
    /// `<unset>` means the child did not receive the var.
    #[cfg(unix)]
    async fn spawn_named_and_read_child_env(
        file_name: &str,
        var: &str,
        extra_env: &[(String, String)],
    ) -> String {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("buzz-acp-env-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create env probe dir");
        let path = dir.join(file_name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' \"${{{var}:-<unset>}}\"\n"),
        )
        .expect("write env probe script");
        let mut permissions = std::fs::metadata(&path).expect("stat probe").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("chmod probe");

        let mut client = AcpClient::spawn(
            path.to_str().expect("probe path is UTF-8"),
            &[],
            extra_env,
            false,
        )
        .await
        .expect("spawn env probe script");
        let observed = client
            .reader
            .next()
            .await
            .unwrap_or_else(|| panic!("child produced no output for {var}"))
            .expect("child stdout was not readable");
        client.shutdown().await;
        std::fs::remove_dir_all(&dir).expect("remove env probe dir");
        observed
    }

    /// Buzz-owned Hermes processes get the configured-MCP isolation default,
    /// and an explicit persona entry still overrides it (defaults are applied
    /// before `extra_env`, so the later `Command::env` write wins).
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_applies_runtime_env_defaults_with_extra_env_precedence() {
        const VAR: &str = "HERMES_ACP_SKIP_CONFIGURED_MCP";
        if std::env::var_os(VAR).is_some() {
            // Inherited parent values win over both layers; the default and
            // override behavior below is unobservable in such an environment.
            return;
        }

        assert_eq!(
            spawn_named_and_read_child_env("hermes-acp", VAR, &[]).await,
            "1",
            "Hermes spawns must default {VAR}=1"
        );
        assert_eq!(
            spawn_named_and_read_child_env("hermes-acp", VAR, &[(VAR.into(), "0".into())]).await,
            "0",
            "an explicit extra_env entry must override the runtime default"
        );
        assert_eq!(
            spawn_named_and_read_child_env("other-agent", VAR, &[]).await,
            "<unset>",
            "non-Hermes spawns must not receive Hermes defaults"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_bridges_compatible_openai_env_into_goose_process() {
        const RELEVANT_PARENT_KEYS: [&str; 5] = [
            "GOOSE_PROVIDER",
            "OPENAI_COMPAT_BASE_URL",
            "OPENAI_COMPAT_API_KEY",
            "GOOSE_PROVIDER__HOST",
            "GOOSE_PROVIDER__API_KEY",
        ];
        if RELEVANT_PARENT_KEYS
            .iter()
            .any(|key| std::env::var_os(key).is_some())
        {
            return;
        }
        let extra = env(&[
            ("GOOSE_PROVIDER", "openai"),
            ("OPENAI_COMPAT_BASE_URL", "http://spark:4000/v1"),
            ("OPENAI_COMPAT_API_KEY", "dummy"),
        ]);

        assert_eq!(
            spawn_named_and_read_child_env("goose", "GOOSE_PROVIDER__HOST", &extra).await,
            "http://spark:4000/v1"
        );
        assert_eq!(
            spawn_named_and_read_child_env("goose", "GOOSE_PROVIDER__API_KEY", &extra).await,
            "dummy"
        );
    }

    #[tokio::test]
    async fn idle_timeout_fires_on_silent_process() {
        let mut client = spawn_script("sleep 10").await;
        let max_dur = std::time::Duration::from_secs(30);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn hard_timeout_fires_when_deadline_is_immediate() {
        let mut client = spawn_script("while true; do echo 'noise'; sleep 0.01; done").await;
        let max_dur = std::time::Duration::from_millis(1);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(60),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "expected HardTimeout, got {result:?}"
        );
    }

    /// `cancel_with_cleanup_grace`'s bounded drain deadline must map to
    /// [`AcpError::CancelDrainTimeout`], never [`AcpError::HardTimeout`] —
    /// the two share an underlying deadline mechanism but must not share
    /// classification, since callers dead-letter a real `HardTimeout` and
    /// must not dead-letter a drain that simply ran past its grace window.
    #[tokio::test]
    async fn cancel_with_cleanup_grace_maps_expiry_to_cancel_drain_timeout() {
        // Agent ignores `session/cancel` on stdin and keeps producing noise
        // forever — never drains within the grace window.
        let mut client = spawn_script("while true; do echo 'noise'; sleep 0.01; done").await;
        client.last_prompt_id = Some(999);
        let grace = std::time::Duration::from_millis(200);
        let result = client
            .cancel_with_cleanup_grace("test-session", grace)
            .await;
        assert!(
            matches!(result, Err(AcpError::CancelDrainTimeout(g)) if g == grace),
            "expected CancelDrainTimeout({grace:?}), got {result:?}"
        );
    }

    #[tokio::test]
    async fn idle_resets_on_stdout_activity() {
        // Send valid JSON (session/update notifications) to reset the idle timer.
        // Non-JSON lines no longer reset idle — only valid JSON notifications do.
        let mut client = spawn_script(
            r#"for i in $(seq 1 10); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"thinking"}}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // 10 messages × 50ms = ~500ms of activity, then idle timeout fires after 200ms more
        assert!(elapsed >= std::time::Duration::from_millis(400));
        assert!(elapsed < std::time::Duration::from_secs(3));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn response_returned_when_matching_id_arrives() {
        let mut client =
            spawn_script(r#"echo '{"jsonrpc":"2.0","id":42,"result":{"stopReason":"end_turn"}}'"#)
                .await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                42,
                std::time::Duration::from_secs(2),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["stopReason"].as_str(), Some("end_turn"));
    }

    #[tokio::test]
    async fn agent_exit_detected_as_eof() {
        let mut client = spawn_script("exit 0").await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(2),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(matches!(result, Err(AcpError::AgentExited)));
    }

    /// A message with both `id` and `method` is an agent-initiated request,
    /// not a response. The response matcher must not consume it even if the
    /// id happens to match the expected value.
    #[tokio::test]
    async fn agent_request_with_matching_id_not_consumed_as_response() {
        // The script sends an agent-initiated request (has both id and method)
        // whose id matches what we're waiting for (0), then sends the real
        // response. The request should be dispatched (triggering -32601 since
        // "test/method" is unknown), and the real response should be returned.
        let script = r#"
            echo '{"jsonrpc":"2.0","id":0,"method":"test/method","params":{}}'
            read -t 2 _reply
            echo '{"jsonrpc":"2.0","id":0,"result":{"ok":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                0,
                std::time::Duration::from_secs(3),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(result.is_ok(), "expected Ok response, got {result:?}");
        assert_eq!(result.unwrap()["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn idle_fires_before_hard_when_idle_is_shorter() {
        let mut client = spawn_script("sleep 10").await;
        let idle = std::time::Duration::from_millis(100);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline, max_dur)
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "idle should fire before hard when idle << hard, got {result:?}"
        );
    }

    /// Hard-deadline starvation regression (Max's review gate, Eva's required test).
    ///
    /// When the read-loop became a `tokio::select!` with `biased; reader →
    /// steer → sleep_until`, a continuously-ready reader arm could win every
    /// poll and starve the timer arm — silently defeating the hard-deadline
    /// guarantee. The fix is a pre-select deadline check at the top of every
    /// loop iteration; this test pins that behavior.
    ///
    /// Setup: agent emits a **gapless** stream of valid JSON `session/update`
    /// notifications (no `sleep` between lines) so the reader arm is
    /// continuously ready. Each line is valid JSON, so it resets the idle
    /// clock — and we set idle ≫ hard so idle cannot fire first. With
    /// `biased; reader → steer → sleep_until`, the reader arm would win
    /// every poll and `sleep_until` would never be reached. Only the
    /// pre-select deadline check at the top of the loop can stop us.
    ///
    /// Without the pre-select check, this test hangs against the infinite
    /// bash subprocess until the test harness's own outer timeout, and the
    /// returned error would never be `HardTimeout`.
    #[tokio::test]
    async fn hard_deadline_fires_under_continuous_valid_json_stream() {
        // Truly infinite, gapless stream of valid JSON. No `sleep` between
        // echoes — the reader arm is continuously ready, which is the
        // exact starvation scenario the pre-select check guards against.
        // `while :; do echo ...; done` (not a fixed-count `for`) so the
        // subprocess never naturally exits before the hard deadline,
        // regardless of how fast the host drains bash output. Without
        // this, fast hardware drains a bounded loop in < hard_deadline
        // and the reader hits EOF (`AgentExited`) before the timer fires,
        // masking whether the pre-select check actually works.
        let mut client = spawn_script(
            r#"while :; do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"x"}}}}'; done"#,
        )
        .await;
        let hard = std::time::Duration::from_millis(300);
        let hard_deadline = tokio::time::Instant::now() + hard;
        let idle = std::time::Duration::from_secs(60); // idle ≫ hard
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline, hard)
            .await;
        let elapsed = start.elapsed();
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "expected HardTimeout under gapless valid-JSON stream, got {result:?} (elapsed {elapsed:?})"
        );
        // Must fire close to the hard deadline, not late. Without the
        // pre-select check the reader arm starves sleep_until and elapsed
        // tracks the bash subprocess lifetime instead.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "HardTimeout fired late ({elapsed:?}); reader arm may be starving sleep_until"
        );
    }

    /// Same as `agent_request_with_matching_id_not_consumed_as_response` but
    /// exercises the non-idle `read_until_response` path (via `send_request`).
    #[tokio::test]
    async fn agent_request_not_consumed_via_send_request() {
        // Script: wait for the initialize request, reply, then send an
        // agent-initiated request with id=1 (matching the next send_request id),
        // wait for the -32601 error reply, then send the real response.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 _req
            echo '{"jsonrpc":"2.0","id":1,"method":"test/unknown","params":{}}'
            read -t 2 _err_reply
            echo '{"jsonrpc":"2.0","id":1,"result":{"worked":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        // initialize consumes id=0
        let _init = client
            .initialize()
            .await
            .expect("initialize should succeed");
        // send_request uses id=1 — the agent's request with id=1 and method
        // must not be consumed as the response.
        let result = client
            .send_request("test/echo", serde_json::json!({}))
            .await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert_eq!(result.unwrap()["worked"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn keepalive_resets_idle_past_deadline() {
        // Keepalive session/update lines every 50ms against a 100ms idle deadline.
        // The turn should survive well past the 100ms deadline (proves the fix).
        let mut client = spawn_script(
            r#"for i in $(seq 1 20); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"keepalive"}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // 20 keepalives × 50ms = ~1000ms of activity, then idle fires after 100ms more.
        // Must survive well past the 100ms deadline.
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "keepalive should reset idle past the deadline; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn tool_call_resets_idle_then_silence_times_out() {
        // A tool_call session/update resets the idle timer (belt-and-suspenders path),
        // then silence causes idle timeout. This proves the reset works for tool_call
        // specifically — not just via the general valid-JSON reset at line 839.
        //
        // The script emits a tool_call, waits 80ms (under the 200ms idle), then goes
        // silent. If the tool_call reset didn't fire, idle would fire at 200ms from
        // start. With the reset, idle fires at 80ms + 200ms = ~280ms from start.
        let mut client = spawn_script(
            r#"echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","title":"long_running","kind":"shell"}}}'; sleep 0.08; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // The tool_call arrives near-instantly and resets idle.
        // Then 80ms of silence, then idle fires at ~280ms from start.
        // Must be > 200ms (proves the reset happened after the tool_call).
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "tool_call should reset idle; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(2));
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout after silence, got {result:?}"
        );
    }

    #[tokio::test]
    async fn session_new_full_includes_system_prompt_when_some() {
        // Script: respond to initialize, then echo back the session/new request.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full(
                "/tmp",
                vec![],
                Some(SystemPromptTransport::Field("Custom system prompt")),
                None,
            )
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["systemPrompt"].as_str(),
            Some("Custom system prompt"),
            "systemPrompt should be included in params when Some"
        );
    }

    #[tokio::test]
    async fn session_close_fire_and_forget_writes_request_and_stale_ack_is_ignored() {
        let script = r#"
            read -t 2 CLOSE
            echo '{"jsonrpc":"2.0","id":0,"result":{}}'
            read -t 2 NEXT
            echo '{"jsonrpc":"2.0","id":1,"result":{"close":'"$CLOSE"',"next":'"$NEXT"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;

        client
            .session_close_fire_and_forget("ses_old")
            .await
            .expect("close request write should succeed");
        let response = client
            .send_request("test/next", serde_json::json!({ "value": 7 }))
            .await
            .expect("the next request should ignore the stale close response");

        assert_eq!(response["close"]["method"], "session/close");
        assert_eq!(response["close"]["params"]["sessionId"], "ses_old");
        assert_eq!(response["next"]["method"], "test/next");
        assert_eq!(response["next"]["params"]["value"], 7);
    }

    #[tokio::test]
    async fn goose_system_prompt_request_uses_set_contract() {
        let script = r#"
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":0,"result":{"_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        let result = client
            .session_set_goose_system_prompt("ses_goose", "Be terse")
            .await
            .expect("custom request succeeds");
        let received = &result["_receivedRequest"];
        assert_eq!(
            received["method"],
            "_goose/unstable/session/system-prompt/set"
        );
        assert_eq!(received["params"]["sessionId"], "ses_goose");
        assert_eq!(received["params"]["mode"], "set");
        assert_eq!(received["params"]["key"], "buzz");
        assert_eq!(received["params"]["text"], "Be terse");
    }

    #[tokio::test]
    async fn goose_system_prompt_preserves_method_not_found_for_fallback() {
        let script = r#"
            read -t 2 _REQ
            echo '{"jsonrpc":"2.0","id":0,"error":{"code":-32601,"message":"Method not found"}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        assert!(matches!(
            client
                .session_set_goose_system_prompt("ses_goose", "Be terse")
                .await,
            Err(AcpError::AgentError { code: -32601, .. })
        ));
    }

    #[tokio::test]
    async fn goose_system_prompt_preserves_invalid_params_as_error() {
        let script = r#"
            read -t 2 _REQ
            echo '{"jsonrpc":"2.0","id":0,"error":{"code":-32602,"message":"Invalid params"}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        assert!(matches!(
            client
                .session_set_goose_system_prompt("ses_goose", "Be terse")
                .await,
            Err(AcpError::AgentError { code: -32602, .. })
        ));
    }

    #[tokio::test]
    async fn session_new_full_omits_system_prompt_when_none() {
        // When system_prompt is None, the field should not appear in params.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, None)
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"]["systemPrompt"].is_null(),
            "systemPrompt should NOT be in params when value is None"
        );
    }

    #[tokio::test]
    async fn session_new_full_sends_session_title_in_meta_when_some() {
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, Some("Fizz · #buzz-dev"))
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["_meta"]["sessionTitle"].as_str(),
            Some("Fizz · #buzz-dev"),
            "title should ride in _meta.sessionTitle, out of band from the prompt"
        );
        let meta = received["params"]["_meta"]
            .as_object()
            .expect("_meta must be an object");
        assert_eq!(
            meta.len(),
            1,
            "session/new must not inject adapter-specific settings that override Claude's managed project catalog"
        );
        assert!(
            !meta.contains_key("claudeCode"),
            "future _meta.claudeCode.options.settings would silently shadow Claude's managed project catalog"
        );
    }

    #[tokio::test]
    async fn claude_session_requests_only_background_task_sdk_events() {
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{},"agentInfo":{"name":"@agentclientprotocol/claude-agent-acp"}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_claude","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, Some("Claude lane"))
            .await
            .expect("session_new_full should succeed");
        let filters =
            &resp.raw["_receivedRequest"]["params"]["_meta"]["claudeCode"]["emitRawSDKMessages"];
        assert_eq!(
            filters,
            &serde_json::json!([{
                "type": "system",
                "subtype": "background_tasks_changed"
            }])
        );
        assert!(
            resp.raw["_receivedRequest"]["params"]["_meta"]["claudeCode"]
                .get("options")
                .is_none(),
            "lifecycle opt-in must not inject Claude settings/options"
        );
    }

    #[tokio::test]
    async fn session_new_full_omits_meta_when_session_title_none() {
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, None)
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"].get("_meta").is_none(),
            "_meta should be absent entirely, not an empty object or null"
        );
    }

    // ── claude-agent-acp _meta.systemPrompt transport ─────────────────────

    include!("acp/system_prompt_tests.rs");
    #[tokio::test]
    async fn account_switch_teardown_reaps_live_adapter_and_is_idempotent() {
        let mut client = spawn_inert_client().await;
        client.shutdown_for_account_switch().await.unwrap();
        assert!(client.child.try_wait().unwrap().is_some());
        client.shutdown_for_account_switch().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn account_switch_teardown_stops_tools_after_adapter_exits() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("late-write");
        let mut client = AcpClient::spawn(
            "/bin/sh",
            &[
                "-c".into(),
                "(sleep 0.4; echo late > \"$1\") & echo spawned; exit 0".into(),
                "fixture".into(),
                marker.to_string_lossy().into_owned(),
            ],
            &[],
            false,
        )
        .await
        .unwrap();
        assert_eq!(client.reader.next().await.unwrap().unwrap(), "spawned");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        client.shutdown_for_account_switch().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        assert!(
            !marker.exists(),
            "old account tool survived adapter teardown"
        );
        assert!(client.child.try_wait().unwrap().is_some());
    }

    fn background_tasks_message(session_id: &str, tasks: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_claude/sdkMessage",
            "params": {
                "sessionId": session_id,
                "message": {
                    "type": "system",
                    "subtype": "background_tasks_changed",
                    "session_id": session_id,
                    "tasks": tasks
                }
            }
        })
    }

    /// Track-A: full session/update frame carrying a fork adapter's
    /// `_meta.backgroundTasks` snapshot on the session_info_update carrier.
    fn typed_session_update(session_id: &str, snapshot: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "session_info_update",
                    "_meta": { "backgroundTasks": snapshot }
                }
            }
        })
    }

    #[tokio::test]
    async fn typed_snapshot_stores_tasks_and_observes_promotion() {
        let mut client = spawn_inert_client().await;
        client.background_mode = BackgroundMode::UnconfirmedTyped;
        client.handle_session_update(&typed_session_update(
            "sess-t",
            serde_json::json!({
                "readiness": "unconfirmed",
                "cli": {"source": "configured", "path": "/usr/bin/claude"},
                "tasks": [{"taskId": "b1", "taskType": "local_bash", "kind": "shell", "description": "Run build"}]
            }),
        ));
        let pending = client.pending_background_tasks();
        assert_eq!(pending.len(), 1, "typed task must be stored");
        assert_eq!(pending[0].tasks[0].task_id, "b1");
        assert_eq!(
            pending[0].monitoring_state, "typed-unconfirmed",
            "notice label must reflect unconfirmed state before promotion"
        );
        // Adapter publishes promotion; consumer observes only.
        client.handle_session_update(&typed_session_update(
            "sess-t",
            serde_json::json!({"readiness": "confirmed", "tasks": []}),
        ));
        assert_eq!(client.background_mode, BackgroundMode::ConfirmedTyped);
        assert!(
            client.pending_background_tasks().is_empty(),
            "empty tasks array is the only clear signal"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_updates_preserve_per_task_first_observed() {
        let mut client = spawn_inert_client().await;
        client.background_mode = BackgroundMode::ConfirmedTyped;
        let t0 = tokio::time::Instant::now();
        client.handle_session_update(&typed_session_update(
            "sess-age",
            serde_json::json!({"readiness": "confirmed", "tasks": [
                {"taskId": "old-1", "taskType": "local_bash", "description": "Original watch"}
            ]}),
        ));
        tokio::time::advance(std::time::Duration::from_secs(600)).await;
        client.handle_session_update(&typed_session_update(
            "sess-age",
            serde_json::json!({"readiness": "confirmed", "tasks": [
                {"taskId": "old-1", "taskType": "local_bash", "description": "Original watch"},
                {"taskId": "new-1", "taskType": "local_bash", "description": "Fresh poll"}
            ]}),
        ));
        let pending = client.pending_background_tasks();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].first_observed_at, t0,
            "session hold anchor must NOT refresh while the set stays non-empty"
        );
        let old = pending[0]
            .tasks
            .iter()
            .find(|t| t.task_id == "old-1")
            .expect("surviving task present");
        let new = pending[0]
            .tasks
            .iter()
            .find(|t| t.task_id == "new-1")
            .expect("new task present");
        assert_eq!(
            old.first_observed_at, t0,
            "surviving task keeps its original first-observed timestamp"
        );
        assert_eq!(
            new.first_observed_at,
            t0 + std::time::Duration::from_secs(600),
            "new task in a rolling chain gets its own timestamp, not the anchor"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idless_tasks_never_inherit_each_others_age() {
        // Two DIFFERENT id-less tasks across snapshots both collapse to the
        // "unknown" sentinel; the second must not inherit the first's age.
        let mut client = spawn_inert_client().await;
        client.background_mode = BackgroundMode::ConfirmedTyped;
        let t0 = tokio::time::Instant::now();
        client.handle_session_update(&typed_session_update(
            "sess-idless",
            serde_json::json!({"readiness": "confirmed", "tasks": [
                {"taskType": "local_bash", "description": "Watch A"}
            ]}),
        ));
        tokio::time::advance(std::time::Duration::from_secs(600)).await;
        client.handle_session_update(&typed_session_update(
            "sess-idless",
            serde_json::json!({"readiness": "confirmed", "tasks": [
                {"taskType": "local_bash", "description": "Watch B"}
            ]}),
        ));
        let pending = client.pending_background_tasks();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].tasks.len(), 1);
        assert_eq!(pending[0].tasks[0].task_id, "unknown");
        assert_eq!(
            pending[0].tasks[0].first_observed_at,
            t0 + std::time::Duration::from_secs(600),
            "a fresh id-less task must carry its own timestamp, not Watch A's"
        );
        assert_eq!(
            pending[0].first_observed_at, t0,
            "session anchor still never refreshes while the set is non-empty"
        );
    }

    #[tokio::test]
    async fn typed_snapshot_malformed_drops_whole_snapshot_not_state() {
        let mut client = spawn_inert_client().await;
        client.background_mode = BackgroundMode::UnconfirmedTyped;
        client.handle_session_update(&typed_session_update(
            "sess-m",
            serde_json::json!({"readiness": "unconfirmed", "tasks": [
                {"taskId": "keep-1", "taskType": "local_bash", "description": "Keep me"}
            ]}),
        ));
        assert_eq!(client.pending_background_tasks().len(), 1);
        // Missing tasks array: fail-closed — never normalizes to a clear.
        client.handle_session_update(&typed_session_update(
            "sess-m",
            serde_json::json!({"readiness": "unconfirmed"}),
        ));
        // Missing readiness entirely: also dropped.
        client.handle_session_update(&typed_session_update(
            "sess-m",
            serde_json::json!({"tasks": []}),
        ));
        assert_eq!(
            client.pending_background_tasks().len(),
            1,
            "malformed snapshots must not clear known-pending work"
        );
    }

    #[tokio::test]
    async fn unarmed_and_typed_modes_ignore_raw_seam_frames() {
        let mut client = spawn_inert_client().await;
        for mode in [BackgroundMode::Unarmed, BackgroundMode::ConfirmedTyped] {
            client.background_mode = mode;
            client.handle_claude_sdk_message(&background_tasks_message(
                "sess-r",
                serde_json::json!([{"task_id": "raw-1", "task_type": "local_bash"}]),
            ));
        }
        assert!(
            client.pending_background_tasks().is_empty(),
            "raw seam must not feed typed/unarmed sessions (no fallback)"
        );
    }

    #[tokio::test]
    async fn unarmed_mode_ignores_even_typed_snapshots() {
        let mut client = spawn_inert_client().await;
        client.background_mode = BackgroundMode::Unarmed;
        client.handle_session_update(&typed_session_update(
            "sess-u",
            serde_json::json!({"readiness": "unarmed", "tasks": [
                {"taskId": "ghost", "taskType": "local_bash"}
            ]}),
        ));
        assert!(
            client.pending_background_tasks().is_empty(),
            "unarmed sessions listen to nothing by contract"
        );
    }

    // ── M3 live matrix (ignored by default; run explicitly) ─────────────
    // Drives the REAL forked adapter (node dist/index.js) end-to-end through
    // this consumer: initialize → session/new readiness → prompt with a
    // background bash task → typed snapshot consumption + observe-only
    // promotion. Env: ACP_FORK_DIST=<path to dist dir>, CLAUDE_CODE_EXECUTABLE
    // set for the armed case, unset for the disarm case.

    fn fork_adapter_command() -> Option<(String, Vec<String>)> {
        let dist = std::env::var("ACP_FORK_DIST").ok()?;
        Some(("node".to_string(), vec![format!("{dist}/index.js")]))
    }

    #[tokio::test]
    #[ignore = "live M3 matrix: needs real claude CLI; run with --ignored"]
    async fn m3_live_typed_configured_end_to_end() {
        use tokio::time::{timeout, Duration};
        let Some((cmd, args)) = fork_adapter_command() else {
            panic!("set ACP_FORK_DIST to the forked adapter's dist dir");
        };
        let claude_cli =
            std::env::var("CLAUDE_CODE_EXECUTABLE").expect("set CLAUDE_CODE_EXECUTABLE (armed)");
        let mut env: Vec<(String, String)> = vec![
            ("CLAUDE_CODE_EXECUTABLE".into(), claude_cli),
            ("HOME".into(), std::env::var("HOME").unwrap_or_default()),
        ];
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            env.push(("ANTHROPIC_API_KEY".into(), key));
        }
        let mut client = AcpClient::spawn(&cmd, &args, &env, false)
            .await
            .expect("spawn forked adapter");
        timeout(Duration::from_secs(30), client.initialize())
            .await
            .expect("initialize")
            .expect("init ok");
        assert_eq!(
            client.background_mode_label(),
            "typed-unconfirmed",
            "fork adapter must advertise init cap -> typed mode"
        );
        let resp = timeout(
            Duration::from_secs(60),
            client.session_new_full("/tmp", vec![], None, None),
        )
        .await
        .expect("session/new in time")
        .expect("session/new ok");
        eprintln!("M3EVIDENCE session/new _meta={:?}", resp.raw.get("_meta"));
        let session_id = resp.session_id.clone();
        let prompt = "Run exactly one background Bash task that echoes m3-probe then sleeps 2 seconds, then finish your turn.";
        let outcome = timeout(
            Duration::from_secs(240),
            client.session_prompt_with_idle_timeout(
                &session_id,
                prompt,
                Duration::from_secs(120),
                Duration::from_secs(240),
            ),
        )
        .await;
        // The turn may outlive the quiet window while the bg task runs — that
        // is fine; what matters is the typed path fired.
        eprintln!("M3EVIDENCE prompt_outcome={outcome:?}");
        eprintln!(
            "M3EVIDENCE final_mode={} pending={:?}",
            client.background_mode_label(),
            client.pending_background_tasks()
        );
        assert_eq!(
            client.background_mode_label(),
            "typed-confirmed",
            "adapter snapshot must have promoted monitoring to confirmed"
        );
        let _ = client
            .cancel_with_cleanup(&session_id, Duration::from_secs(10))
            .await;
    }

    #[tokio::test]
    #[ignore = "live M3 matrix: disarm variant"]
    async fn m3_live_disarmed_bundled_end_to_end() {
        use tokio::time::{timeout, Duration};
        let Some((cmd, args)) = fork_adapter_command() else {
            panic!("set ACP_FORK_DIST to the forked adapter's dist dir");
        };
        let mut env: Vec<(String, String)> =
            vec![("HOME".into(), std::env::var("HOME").unwrap_or_default())];
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            env.push(("ANTHROPIC_API_KEY".into(), key));
        }
        let mut client = AcpClient::spawn(&cmd, &args, &env, false)
            .await
            .expect("spawn forked adapter");
        timeout(Duration::from_secs(30), client.initialize())
            .await
            .expect("initialize")
            .expect("init ok");
        // Init cap stays advertised even when unarmed (static contract L1.3).
        assert_eq!(
            client.background_mode_label(),
            "typed-unconfirmed",
            "init capability is static; mode refines only at session/new"
        );
        let resp = timeout(
            Duration::from_secs(60),
            client.session_new_full("/tmp", vec![], None, None),
        )
        .await
        .expect("session/new in time")
        .expect("session/new ok");
        eprintln!(
            "M3EVIDENCE disarmed session/new _meta={:?}",
            resp.raw.get("_meta")
        );
        assert_eq!(
            client.background_mode_label(),
            "unarmed",
            "bundled-CLI readiness must narrow to unarmed"
        );
        let session_id = resp.session_id.clone();
        let _ = client
            .cancel_with_cleanup(&session_id, Duration::from_secs(10))
            .await;
    }

    #[tokio::test]
    async fn background_tasks_are_tracked_per_session_and_clear_fail_closed() {
        let mut client = spawn_inert_client().await;
        client.handle_claude_sdk_message(&background_tasks_message(
            "sess-a",
            serde_json::json!([{
                "task_id": "build-1",
                "task_type": "local_bash",
                "description": "Run the build"
            }]),
        ));
        client.handle_claude_sdk_message(&background_tasks_message(
            "sess-b",
            serde_json::json!([{
                "task_id": "sign-1",
                "task_type": "local_bash",
                "description": "Sign the bundle"
            }]),
        ));
        assert_eq!(client.pending_background_tasks().len(), 2);

        let malformed = serde_json::json!({
            "method": "_claude/sdkMessage",
            "params": {"sessionId": "sess-a", "message": {
                "type": "system", "subtype": "background_tasks_changed"
            }}
        });
        client.handle_claude_sdk_message(&malformed);
        assert_eq!(
            client.pending_background_tasks().len(),
            2,
            "malformed update must not clear pending work"
        );

        client
            .handle_claude_sdk_message(&background_tasks_message("sess-a", serde_json::json!([])));
        let pending = client.pending_background_tasks();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].session_id, "sess-b");
    }

    #[tokio::test]
    async fn idle_notification_drain_observes_task_start_and_completion() {
        let pending = background_tasks_message(
            "sess-idle",
            serde_json::json!([{
                "task_id": "long-1",
                "task_type": "local_bash",
                "description": "Long background task"
            }]),
        );
        let cleared = background_tasks_message("sess-idle", serde_json::json!([]));
        let script = format!(
            "printf '%s\\n' '{}'; read -t 2 _resume; printf '%s\\n' '{}'; sleep 1",
            pending, cleared
        );
        let mut client = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .expect("spawn scripted ACP");

        let mut first_drain = 0;
        for _ in 0..50 {
            first_drain += client
                .drain_idle_notifications()
                .await
                .expect("first idle drain");
            if first_drain > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(first_drain, 1);
        assert_eq!(client.pending_background_tasks().len(), 1);

        client
            .write_ndjson(&serde_json::json!({"resume": true}))
            .await
            .expect("release scripted adapter");
        let mut second_drain = 0;
        for _ in 0..50 {
            second_drain += client
                .drain_idle_notifications()
                .await
                .expect("second idle drain");
            if second_drain > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(second_drain, 1);
        assert!(client.pending_background_tasks().is_empty());
    }

    #[tokio::test]
    async fn delivery_evidence_records_only_final_answer_text() {
        let mut client = spawn_inert_client().await;
        for text in ["No further", "\n action   required."] {
            let message = serde_json::json!({
                "params": { "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "text": text }
                }}
            });
            let _ = client.handle_session_update(&message);
        }

        let evidence = client.turn_delivery_evidence();
        assert!(evidence.final_answer_present);
        assert_eq!(
            evidence.final_answer_snippet.as_deref(),
            Some("No further action required.")
        );
        assert_eq!(evidence.publish_attempt_count, 0);
        assert!(!evidence.adapter_runtime_failure_present);
    }

    #[tokio::test]
    async fn private_prompt_capture_returns_text_without_channel_delivery_evidence() {
        let script = r#"
read -r _request
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"Goal: resume the bounded lane"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"stopReason":"end_turn"}}'
"#;
        let mut client = spawn_script(script).await;
        let (stop_reason, text) = client
            .session_prompt_capture_text(
                "summary-session",
                "summarize",
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("private prompt succeeds");
        assert_eq!(stop_reason, StopReason::EndTurn);
        assert_eq!(text, "Goal: resume the bounded lane");
        assert!(!client.turn_delivery_evidence().final_answer_present);
        client.shutdown().await;
    }

    #[tokio::test]
    async fn delivery_evidence_marks_goose_synthetic_runtime_failure() {
        let mut client = spawn_inert_client().await;
        for text in ["Ran into this ", "error: Authentication error: missing key"] {
            let message = serde_json::json!({
                "params": { "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text },
                    "_meta": { "goose": { "messageId": "msg_failure" } }
                }}
            });
            let _ = client.handle_session_update(&message);
        }

        let evidence = client.turn_delivery_evidence();
        assert!(evidence.final_answer_present);
        assert!(evidence.adapter_runtime_failure_present);
    }

    #[tokio::test]
    async fn delivery_evidence_does_not_infer_runtime_failure_from_model_prose() {
        let mut client = spawn_inert_client().await;
        for (text, goose_meta) in [
            ("Ran into this error: quoted documentation", false),
            ("Ordinary goose answer", true),
        ] {
            let mut update = serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text }
            });
            if goose_meta {
                update["_meta"] = serde_json::json!({ "goose": { "messageId": "msg_ok" } });
            }
            let message = serde_json::json!({ "params": { "update": update } });
            let _ = client.handle_session_update(&message);
            assert!(
                !client
                    .turn_delivery_evidence()
                    .adapter_runtime_failure_present
            );
            client.turn_delivery_evidence = TurnDeliveryEvidence::default();
        }
    }

    #[tokio::test]
    async fn delivery_evidence_tracks_nested_publish_attempt_and_failure() {
        let mut client = spawn_inert_client().await;
        let call = serde_json::json!({
            "params": { "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "publish-1",
                "rawInput": { "cmd": ["buzz", "messages", "send"], "shell": "buzz messages send --content -" }
            }}
        });
        let failed = serde_json::json!({
            "params": { "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "publish-1",
                "status": "completed",
                "rawOutput": { "isError": true, "error": "approval rejected" }
            }}
        });
        let _ = client.handle_session_update(&call);
        let _ = client.handle_session_update(&failed);
        // Duplicate updates must not inflate the counters.
        let _ = client.handle_session_update(&call);
        let _ = client.handle_session_update(&failed);

        let evidence = client.turn_delivery_evidence();
        assert_eq!(evidence.publish_attempt_count, 1);
        assert_eq!(evidence.publish_failure_count, 1);
        assert!(!evidence.final_answer_present);
        assert!(evidence.final_answer_snippet.is_none());
    }

    #[test]
    fn terminal_excerpt_retains_the_failure_after_startup_warnings() {
        let mut evidence = TurnDeliveryEvidence::default();
        evidence.record_final_answer_chunk(
            &"Warning: Skill descriptions were shortened. ".repeat(10),
            false,
        );
        evidence.record_final_answer_chunk(
            "Automatic approval review rejected Summary publication. No cursors advanced.",
            false,
        );
        let text = evidence.final_answer_snippet.unwrap();
        assert!(text.ends_with("No cursors advanced."));
        assert!(text.contains("rejected Summary publication"));
        assert!(text.chars().count() <= FINAL_ANSWER_SNIPPET_CHARS + 1);
    }

    #[test]
    fn final_answer_snippet_is_unicode_safe_and_bounded() {
        let text = "🪿".repeat(FINAL_ANSWER_SNIPPET_CHARS + 5);
        let snippet = normalized_snippet(&text, FINAL_ANSWER_SNIPPET_CHARS).unwrap();
        assert_eq!(snippet.chars().count(), FINAL_ANSWER_SNIPPET_CHARS + 1);
        assert!(snippet.ends_with('…'));
    }

    #[tokio::test]
    async fn delivery_evidence_accepts_claude_late_bash_input_and_cli_receipt() {
        let mut client = spawn_inert_client().await;
        let event_id = "a".repeat(64);
        for update in [
            serde_json::json!({"sessionUpdate":"tool_call", "toolCallId":"bash-1", "title":"Bash"}),
            serde_json::json!({"sessionUpdate":"tool_call_update", "toolCallId":"bash-1",
                "rawInput":{"command":"buzz messages send --content result"}}),
            serde_json::json!({"sessionUpdate":"tool_call_update", "toolCallId":"bash-1", "status":"completed",
                "rawOutput":[{"type":"text", "text":serde_json::json!({"accepted":true,"event_id":event_id}).to_string()}]}),
        ] {
            let message = serde_json::json!({"params":{"update":update}});
            let _ = client.handle_session_update(&message);
            let _ = client.handle_session_update(&message);
        }
        let evidence = client.turn_delivery_evidence();
        assert_eq!(evidence.publish_attempt_count, 1);
        assert_eq!(evidence.publish_failure_count, 0);
        assert_eq!(
            evidence.published_event_ids,
            HashSet::from([nostr::EventId::from_hex(&event_id).unwrap()])
        );
    }

    #[tokio::test]
    async fn delivery_evidence_reads_buzz_agent_content_receipt_only_for_completed_publish() {
        for (publish, status, is_error, expected) in [
            (true, "completed", false, true),
            (false, "completed", false, false),
            (true, "in_progress", false, false),
            (true, "failed", false, false),
            (true, "completed", true, false),
        ] {
            let mut client = spawn_inert_client().await;
            let id = "b".repeat(64);
            let call = serde_json::json!({"params":{"update":{
                "sessionUpdate":"tool_call", "toolCallId":"shell-1",
                "rawInput":{"command":if publish {"buzz messages send --channel test --content done"} else {"cat unrelated.json"}}
            }}});
            let _ = client.handle_session_update(&call);
            // Exact emit_completed shape: receipt in ACP text, metadata in rawOutput.
            let update = serde_json::json!({"params":{"update":{
                "sessionUpdate":"tool_call_update", "toolCallId":"shell-1", "status":status,
                "rawOutput":{"isError":is_error},
                "content":[{"type":"content","content":{"type":"text","text":
                    serde_json::json!({"accepted":true,"event_id":id}).to_string()}}]
            }}});
            let _ = client.handle_session_update(&update);
            assert_eq!(
                client
                    .turn_delivery_evidence()
                    .published_event_ids
                    .contains(&nostr::EventId::from_hex(&id).unwrap()),
                expected
            );
        }
    }

    /// Build a `session/update` JSON-RPC notification carrying a
    /// `session_info_update` with the given `_meta.goose.activeRunId` value.
    /// Pass `None` to omit the `activeRunId` field entirely.
    ///
    /// `_meta` is nested inside the `update` object (per the ACP
    /// `SessionInfoUpdate` schema), matching what goose and buzz-agent
    /// emit on the wire.

    #[tokio::test]
    async fn active_run_id_sets_on_string() {
        let mut client = spawn_inert_client().await;
        assert!(client.active_run_id().is_none(), "starts as None");

        let msg = session_info_update_msg(Some(serde_json::json!("run-abc-123")));
        let _ = client.handle_session_update(&msg);

        assert_eq!(client.active_run_id(), Some("run-abc-123"));
    }

    #[tokio::test]
    async fn active_run_id_clears_on_null() {
        let mut client = spawn_inert_client().await;
        // Set it first
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-xyz")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-xyz"));

        // Then clear with explicit null
        let clear_msg = session_info_update_msg(Some(serde_json::Value::Null));
        let _ = client.handle_session_update(&clear_msg);
        assert!(
            client.active_run_id().is_none(),
            "explicit null must clear active_run_id"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_when_missing() {
        // Field absent entirely — must NOT clear existing state (only an
        // explicit null clears; missing means "no new info this update").
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        // session_info_update with no activeRunId field — leave state alone.
        let missing_msg = session_info_update_msg(None);
        let _ = client.handle_session_update(&missing_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "missing activeRunId must leave state untouched"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_on_wrong_type() {
        // A number or object in activeRunId is malformed — neither set nor clear.
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        let wrong_type_msg = session_info_update_msg(Some(serde_json::json!(42)));
        let _ = client.handle_session_update(&wrong_type_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "non-string/non-null activeRunId must leave state untouched"
        );
    }

    // ── Goose-native steer arm tests ──────────────────────────────────────
    //
    // These exercise the seam between `install_steer_rx` and the read
    // loop's steer arm, isolated from `AgentPool` / `EventQueue` /
    // dispatch. They prove the locked Option-X contract at the read-loop
    // boundary:
    //   1. With `active_run_id == None`, the steer arm acks
    //      `Err(ExpectedRunIdMissing)` and writes nothing — the main
    //      loop's "Err-before-pending" fallback path is reachable.
    //   2. With `active_run_id` set, the steer arm writes the JSON-RPC
    //      request with the matching `expectedRunId` and routes the
    //      response to the ack oneshot as `Success`.
    //
    // We don't test the full mode-gate fork here — that lives in lib.rs
    // and is covered by goose e2e (Eva's lane).

    /// Steer with no `active_run_id` set acks `ExpectedRunIdMissing`
    /// without writing anything. The read loop continues normally and
    /// eventually hits the idle timeout (which is fine — we just need to
    /// observe the ack).
    #[tokio::test]
    async fn native_steer_with_no_active_run_id_acks_expected_run_id_missing() {
        // Quiet process: never emits anything, so the read loop has only
        // the steer arm and the idle timeout to consider.
        let mut client = spawn_script("sleep 10").await;
        assert!(
            client.active_run_id().is_none(),
            "precondition: active_run_id starts as None"
        );

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        // Fire-and-forget: send a SteerRequest from a separate task so
        // the read loop picks it up via the select! arm.
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop with short idle timeout so the test
        // doesn't hang. The expected_id is intentionally never going to
        // be matched (the script writes nothing); the read loop will
        // exit via IdleTimeout shortly after the steer arm fires.
        let idle = std::time::Duration::from_millis(500);
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit shape: IdleTimeout (no agent activity).
        assert!(
            matches!(read_result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout once steer was acked + script stayed silent, got {read_result:?}"
        );

        // Ack must be ExpectedRunIdMissing — the steer arm bailed out
        // without writing because active_run_id was None at write time.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::ExpectedRunIdMissing) => {}
            other => panic!("expected SteerAck::Err(ExpectedRunIdMissing), got {other:?}"),
        }
    }

    /// Steer with `active_run_id` set writes the JSON-RPC request and
    /// routes the matching response to the ack oneshot as `Success`.
    /// Verifies the wire shape (`sessionId` + `expectedRunId` + `prompt`)
    /// indirectly: the bash script emits a response keyed by the steer
    /// id (0), and `Success` only fires if the read loop matched that
    /// id to its `pending_steer` entry.
    #[tokio::test]
    async fn native_steer_with_active_run_id_routes_response_to_ack() {
        // Script: pause briefly so the test task can install the steer
        // and we can be sure the response doesn't race ahead of the
        // write — then emit the steer response (id=0 because next_id
        // starts at 0 and the steer is the first request the read loop
        // writes), then idle. This is a JSON-RPC success response with
        // a `stopReason` payload (matching the shape goose uses for
        // steer responses in fake_llm.rs).
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"stopReason\":\"end_turn\"}}'; \
                      sleep 10";
        let mut client = spawn_script(script).await;

        // Set active_run_id via a synthesized session_info_update so the
        // steer arm has a non-None value to read at write time.
        let update = session_info_update_msg(Some(serde_json::json!("run-42")));
        let _ = client.handle_session_update(&update);
        assert_eq!(client.active_run_id(), Some("run-42"));

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop. Expected_id 999 will never be emitted by
        // the script so the read loop exits via idle timeout after the
        // steer response is routed to ack.
        let idle = std::time::Duration::from_secs(2);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit: IdleTimeout (no further activity after the
        // routed steer response). AgentExited would also be a valid
        // exit if the bash script terminated early; either is fine —
        // what matters is the ack.
        assert!(
            matches!(
                read_result,
                Err(AcpError::IdleTimeout(_)) | Err(AcpError::AgentExited)
            ),
            "expected IdleTimeout or AgentExited after steer ack, got {read_result:?}"
        );

        // Ack must be Success: the steer response (id=0) was routed to
        // pending_steer.ack_tx.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Success { .. } => {}
            other => panic!("expected SteerAck::Success, got {other:?}"),
        }
    }

    /// Steer-success renewal keeps the turn alive past the original hard
    /// deadline. This is the red-on-old/green-on-new test for the core bug
    /// fix (acp.rs:1440-1444): without renewal, the read loop returns
    /// `HardTimeout` before the prompt response arrives.
    ///
    /// Timeline:
    ///   t≈0:    read loop starts, `hard_deadline = now + 1s`
    ///   t≈0.5s: script emits steer response (id=0) → Success renewal
    ///           moves `hard_deadline` to `now + 3s` (≈3.5s from start)
    ///   t≈1.5s: script emits prompt response (id=999) → `Ok`
    ///
    /// Old code: `HardTimeout` at t≈1s (before prompt response).
    /// New code: deadline renewed at t≈0.5s → prompt response at t≈1.5s → `Ok`.
    #[tokio::test]
    async fn steer_success_renews_hard_deadline_and_survives_past_original() {
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"stopReason\":\"end_turn\"}}'; \
                      sleep 1; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;

        let update = session_info_update_msg(Some(serde_json::json!("run-99")));
        let _ = client.handle_session_update(&update);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        assert!(
            result.is_ok(),
            "expected Ok (prompt response after renewed deadline), got {result:?}"
        );
        assert_eq!(result.unwrap()["done"], serde_json::json!(true));

        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Success { .. } => {}
            other => panic!("expected SteerAck::Success, got {other:?}"),
        }
    }

    // ── Cross-harness steer transport tests ───────────────────────────────
    //
    // These cover the `_session/steering` transport added alongside the
    // goose-native method: capability capture at `initialize`, write-time
    // transport selection, and outcome decoding. Wire-shape assertions read
    // the actual serialized request bytes via `capture_steer_request` rather
    // than inferring the shape from response-id routing.

    /// Spawn a client whose script captures the first line written to its
    /// stdin into `capture_path`, then emits `response` (already-serialized
    /// JSON-RPC) and idles.
    ///
    /// The steer request is the first thing this read loop writes, so the
    /// captured line IS the steer request bytes.
    async fn spawn_steer_capture_script(
        capture_path: &std::path::Path,
        response: &str,
    ) -> AcpClient {
        let script = format!(
            "read -r line; printf '%s' \"$line\" > {capture}; \
             printf '%s\\n' '{response}'; sleep 10",
            capture = capture_path.display(),
            response = response,
        );
        spawn_script(&script).await
    }

    /// Drive one steer through the read loop and return
    /// `(captured_request_bytes, ack)`.
    ///
    /// `capture_path` may be absent afterwards when the arm wrote nothing —
    /// callers assert on that. The read loop is expected to exit via a
    /// timeout or EOF; the ack is what these tests care about.
    async fn run_one_steer(
        client: &mut AcpClient,
        capture_path: &std::path::Path,
    ) -> (Option<String>, crate::pool::SteerAck) {
        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_millis(800);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let _ = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        (std::fs::read_to_string(capture_path).ok(), ack)
    }

    /// Unique temp path for one test's captured request bytes.
    fn capture_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("buzz-acp-steer-capture");
        std::fs::create_dir_all(&dir).expect("create capture dir");
        let path = dir.join(format!("{name}.json"));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Mark a client as having advertised `_meta.steering.supported` without
    /// running a real `initialize` handshake. The capability-parsing tests
    /// cover the handshake itself.
    fn set_steering_supported(client: &mut AcpClient) {
        client.steering_supported = true;
    }

    /// Run `initialize` against a script that replies with `init_result` as
    /// the JSON-RPC result, and return the resulting `steering_supported`.
    async fn steering_supported_after_initialize(init_result: &str) -> bool {
        let script = format!(
            "read -r _init; printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{result}}}'; \
             sleep 5",
            result = init_result,
        );
        let mut client = spawn_script(&script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");
        client.steering_supported()
    }

    /// Test 1a: an adapter advertising `_meta.steering.supported: true`
    /// (claude-agent-acp `src/acp-agent.ts:1444`, codex-acp
    /// `src/CodexAcpServer.ts:247`) is recorded as steering-capable.
    #[tokio::test]
    async fn initialize_records_steering_supported_when_advertised() {
        let supported = steering_supported_after_initialize(
            r#"{"protocolVersion":2,"agentCapabilities":{},"_meta":{"steering":{"supported":true}}}"#,
        )
        .await;
        assert!(
            supported,
            "_meta.steering.supported: true must set steering_supported"
        );
    }

    /// Test 1b: no `_meta` at all (goose, buzz-agent, any older adapter) must
    /// leave the capability off — this is what keeps a steer off the wire for
    /// agents that never implemented it.
    #[tokio::test]
    async fn initialize_leaves_steering_unsupported_when_meta_absent() {
        let supported =
            steering_supported_after_initialize(r#"{"protocolVersion":2,"agentCapabilities":{}}"#)
                .await;
        assert!(
            !supported,
            "absent _meta must leave steering_supported false"
        );
    }

    /// Test 1c: an explicit `supported: false` is respected, not treated as
    /// "the key exists so it must work".
    #[tokio::test]
    async fn initialize_leaves_steering_unsupported_when_explicitly_false() {
        let supported = steering_supported_after_initialize(
            r#"{"protocolVersion":2,"_meta":{"steering":{"supported":false}}}"#,
        )
        .await;
        assert!(
            !supported,
            "_meta.steering.supported: false must leave steering_supported false"
        );
    }

    /// Test 2: no `active_run_id` + capability advertised → the bytes on the
    /// wire are an `_session/steering` request carrying `sessionId` and
    /// `prompt`, and carrying **no** `expectedRunId` (the adapters reject
    /// unknown required fields, and there is no run id to report anyway).
    #[tokio::test]
    async fn acp_steer_request_omits_expected_run_id_and_carries_session_and_prompt() {
        let capture = capture_path("acp_shape");
        let mut client = spawn_steer_capture_script(
            &capture,
            r#"{"jsonrpc":"2.0","id":0,"result":{"outcome":"injected"}}"#,
        )
        .await;
        set_steering_supported(&mut client);
        assert!(
            client.active_run_id().is_none(),
            "precondition: no active_run_id"
        );

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        let written = written.expect("steer request must have been written");
        let msg: serde_json::Value =
            serde_json::from_str(&written).expect("written line must be valid JSON");
        assert_eq!(
            msg["method"].as_str(),
            Some(ACP_STEER_METHOD),
            "must use the cross-adapter steer method; wrote: {written}"
        );
        assert_eq!(msg["params"]["sessionId"].as_str(), Some("sess-test"));
        assert_eq!(
            msg["params"]["prompt"][0]["text"].as_str(),
            Some("steer body"),
            "prompt must carry the steer body as a text block"
        );
        assert!(
            msg["params"].get("expectedRunId").is_none(),
            "_session/steering must not carry expectedRunId; wrote: {written}"
        );
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "injected outcome must ack Success, got {ack:?}"
        );
    }

    /// Test 3: goose keeps priority. With both an `active_run_id` and the
    /// advertised capability, the goose method wins — `expectedRunId` is
    /// strictly more precise about which run is being steered.
    #[tokio::test]
    async fn goose_transport_wins_when_both_run_id_and_capability_present() {
        let capture = capture_path("goose_priority");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        set_steering_supported(&mut client);
        let update = session_info_update_msg(Some(serde_json::json!("run-77")));
        let _ = client.handle_session_update(&update);

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        let written = written.expect("steer request must have been written");
        let msg: serde_json::Value =
            serde_json::from_str(&written).expect("written line must be valid JSON");
        assert_eq!(
            msg["method"].as_str(),
            Some(GOOSE_STEER_METHOD),
            "goose method must win when a run id exists; wrote: {written}"
        );
        assert_eq!(msg["params"]["expectedRunId"].as_str(), Some("run-77"));
        // A bare `{}` result is a success on the goose transport (goose sends
        // no `outcome`) — the OutcomeRejected guard applies only to
        // `_session/steering`.
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "goose success result must ack Success, got {ack:?}"
        );
    }

    /// Test 7: codex-acp's third outcome, `failed`
    /// (`src/AcpExtensions.ts:92`), is a delivery rejection despite being a
    /// JSON-RPC success — release the event and fall back.
    #[tokio::test]
    async fn acp_steer_failed_outcome_acks_outcome_rejected() {
        let capture = capture_path("outcome_failed");
        let mut client = spawn_steer_capture_script(
            &capture,
            r#"{"jsonrpc":"2.0","id":0,"result":{"outcome":"failed"}}"#,
        )
        .await;
        set_steering_supported(&mut client);

        let (_written, ack) = run_one_steer(&mut client, &capture).await;

        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::OutcomeRejected { outcome }) => {
                assert_eq!(
                    outcome, "failed",
                    "rejected outcome must report what the agent said, unquoted"
                );
            }
            other => panic!("expected Err(OutcomeRejected), got {other:?}"),
        }
    }

    /// Test 8: **codex `extMethod` silent-loss regression guard.** codex-acp's
    /// ext dispatcher answers unrecognized methods with a bare `{}` — a
    /// JSON-RPC *success*, not `-32601` (`src/CodexAcpServer.ts:255-258`).
    /// Buzz maps `SteerAck::Success` to `queue.remove_event`, so decoding
    /// `{}` as success would delete the user's message with no error, no
    /// fallback, and no log. An absent `outcome` must therefore be a
    /// rejection, which releases the event and fires cancel+merge.
    #[tokio::test]
    async fn acp_steer_missing_outcome_acks_outcome_rejected_and_never_drops_event() {
        let capture = capture_path("outcome_absent");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        set_steering_supported(&mut client);

        let (_written, ack) = run_one_steer(&mut client, &capture).await;

        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::OutcomeRejected { outcome }) => {
                assert_eq!(
                    outcome, "<absent>",
                    "a result with no outcome field must be reported as absent"
                );
            }
            other => panic!(
                "expected Err(OutcomeRejected) for a bare {{}} success — \
                 anything else risks dropping the event, got {other:?}"
            ),
        }
    }

    /// Test 5: `injected` renews the hard deadline, so the turn survives past
    /// its original one. Mirrors
    /// `steer_success_renews_hard_deadline_and_survives_past_original` for
    /// the `_session/steering` transport.
    ///
    /// Timeline: original hard deadline at t≈1s; steer response at t≈0.5s
    /// renews it to t≈3.5s; prompt response at t≈1.5s lands inside it.
    #[tokio::test]
    async fn acp_steer_injected_renews_hard_deadline_and_survives_past_original() {
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":\"injected\"}}'; \
                      sleep 1; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;
        set_steering_supported(&mut client);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        assert!(
            result.is_ok(),
            "injected must renew the deadline so the prompt response still lands, got {result:?}"
        );
        assert_eq!(result.unwrap()["done"], serde_json::json!(true));
        let ack = ack_rx.await.expect("ack must be received");
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "injected must ack Success, got {ack:?}"
        );
    }

    /// Test 6: **red/green for the no-renewal rule.** `startedNewTurn` means
    /// the turn Buzz was steering had already ended and the adapter began a
    /// fresh, detached one. It acks `Success` (the message WAS delivered, so
    /// the event must not be redelivered) but must NOT renew the hard
    /// deadline — that clock belongs to a turn which is already settled.
    ///
    /// Same timeline as the `injected` test, so the only difference is the
    /// outcome string: original hard deadline at t≈1s, steer response at
    /// t≈0.5s, prompt response at t≈1.5s. With renewal the prompt response
    /// would land and this returns `Ok`; without renewal the original
    /// deadline fires first and we get `HardTimeout`.
    #[tokio::test]
    async fn acp_steer_started_new_turn_acks_success_without_renewing_hard_deadline() {
        let script = "sleep 0.5; \
             echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":\"startedNewTurn\"}}'; \
             sleep 1; \
             echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;
        set_steering_supported(&mut client);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // The original deadline must still fire — renewal here would extend
        // the clock on a turn the adapter has already finished.
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "startedNewTurn must NOT renew the hard deadline, so the original \
             one must still fire; got {result:?}"
        );
        // Delivery still succeeded, so the withheld event must be dropped
        // rather than released — hence Success, not an Err.
        let ack = ack_rx.await.expect("ack must be received");
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "startedNewTurn is a delivery success, got {ack:?}"
        );
    }

    /// Test 4 (companion to the existing
    /// `native_steer_with_no_active_run_id_acks_expected_run_id_missing`):
    /// no run id AND no advertised capability means nothing is written at
    /// all. This is the gate that keeps a steer off the wire for adapters
    /// that never implemented either method.
    #[tokio::test]
    async fn steer_writes_nothing_when_no_run_id_and_capability_absent() {
        let capture = capture_path("no_transport");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        assert!(!client.steering_supported(), "precondition: not advertised");
        assert!(
            client.active_run_id().is_none(),
            "precondition: no active_run_id"
        );

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        assert!(
            written.is_none(),
            "no transport available must write nothing; wrote: {written:?}"
        );
        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::ExpectedRunIdMissing) => {}
            other => panic!("expected Err(ExpectedRunIdMissing), got {other:?}"),
        }
    }

    // ── Standard ACP prompt-response usage ─────────────────────────────────

    fn prompt_response_usage(
        input: u64,
        output: u64,
        total: u64,
        cached_read: Option<u64>,
        cached_write: Option<u64>,
    ) -> serde_json::Value {
        let mut usage = serde_json::json!({
            "inputTokens": input,
            "outputTokens": output,
            "totalTokens": total,
        });
        if let Some(cached_read) = cached_read {
            usage["cachedReadTokens"] = serde_json::json!(cached_read);
        }
        if let Some(cached_write) = cached_write {
            usage["cachedWriteTokens"] = serde_json::json!(cached_write);
        }
        serde_json::json!({"stopReason": "end_turn", "usage": usage})
    }

    fn standard_cost_update(session_id: &str, cost: f64) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "usage_update",
                    "cost": {"amount": cost, "currency": "USD"}
                }
            }
        })
    }

    #[tokio::test]
    async fn claude_prompt_response_usage_merges_with_cumulative_cost() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("claude-session");
        client.standard_usage.begin_turn("claude-session");
        client.handle_session_update(&standard_cost_update("claude-session", 0.042));
        assert_eq!(
            client
                .parse_prompt_response(
                    "claude-session",
                    &prompt_response_usage(100, 20, 175, Some(30), Some(25)),
                )
                .unwrap(),
            StopReason::EndTurn
        );

        let usage = client.take_turn_usage().expect("prompt usage");
        assert!(usage.delta_reliable, "response tokens need no baseline");
        assert_eq!(usage.turn_input_tokens, Some(155));
        assert_eq!(usage.turn_output_tokens, Some(20));
        assert_eq!(
            usage.turn_total_tokens, None,
            "Claude total is adapter-derived"
        );
        assert_eq!(usage.turn_cache_read_tokens, Some(30));
        assert_eq!(usage.turn_cache_write_tokens, Some(25));
        assert_eq!(usage.turn_cost_usd, Some(0.042));
        assert_eq!(usage.cumulative_cost_usd, Some(0.042));
        assert_eq!(usage.cumulative_input_tokens, None);
        assert_eq!(usage.cumulative_output_tokens, None);
    }

    #[tokio::test]
    async fn codex_prompt_response_usage_preserves_provider_total_without_cost() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Codex);
        client.standard_usage.begin_turn("codex-session");
        client.handle_session_update(&standard_cost_update("codex-session", 0.042));
        client
            .parse_prompt_response(
                "codex-session",
                &prompt_response_usage(90, 10, 140, Some(40), None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("prompt usage");
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(130));
        assert_eq!(usage.turn_output_tokens, Some(10));
        assert_eq!(usage.turn_total_tokens, Some(140));
        assert_eq!(usage.turn_cache_read_tokens, Some(40));
        assert_eq!(usage.turn_cache_write_tokens, None);
        assert_eq!(
            usage.cumulative_cost_usd, None,
            "Codex cost update is ignored"
        );
        assert_eq!(usage.cumulative_input_tokens, None);
        assert_eq!(usage.cumulative_output_tokens, None);
    }

    #[tokio::test]
    async fn standard_prompt_input_overflow_fails_closed() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.standard_usage.begin_turn("overflow-session");
        client
            .parse_prompt_response(
                "overflow-session",
                &prompt_response_usage(u64::MAX, 10, u64::MAX, Some(1), None),
            )
            .unwrap();

        assert!(
            client.take_turn_usage().is_none(),
            "overflow without another valid signal must not emit all-null usage"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_named_adapter_wire_lifecycle_records_prompt_and_cost() {
        let script = r#"
            read -r REQ
            ID=$(printf '%s' "$REQ" | sed -E 's/.*"id":([0-9]+).*/\1/')
            echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"wire-session","update":{"sessionUpdate":"usage_update","cost":{"amount":0.5,"currency":"USD"}}}}'
            echo '{"jsonrpc":"2.0","id":'"$ID"',"result":{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":3,"totalTokens":10,"cachedReadTokens":2}}}'
            sleep 1
        "#;
        let (mut client, dir) = spawn_named_script("claude-code", script).await;
        assert_eq!(client.standard_adapter, Some(StandardAdapterKind::Claude));
        client.notify_session_spawned("wire-session");

        let stop = client
            .session_prompt_with_idle_timeout(
                "wire-session",
                "hello",
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("wire prompt");
        assert_eq!(stop, StopReason::EndTurn);

        let usage = client.take_turn_usage().expect("wire usage");
        assert_eq!(usage.turn_seq, 1);
        assert_eq!(usage.turn_input_tokens, Some(9));
        assert_eq!(usage.turn_output_tokens, Some(3));
        assert_eq!(usage.turn_cost_usd, Some(0.5));
        assert_eq!(usage.cumulative_cost_usd, Some(0.5));
        drop(client);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn claude_cost_only_record_survives_missing_prompt_usage() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("cost-only-session");
        client.standard_usage.begin_turn("cost-only-session");
        client.handle_session_update(&standard_cost_update("cost-only-session", 0.125));

        let usage = client.take_turn_usage().expect("cost-only usage");
        assert_eq!(usage.turn_seq, 1);
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, None);
        assert_eq!(usage.turn_cost_usd, Some(0.125));
        assert_eq!(usage.cumulative_cost_usd, Some(0.125));
    }

    #[tokio::test]
    async fn attached_claude_session_does_not_invent_first_cost_delta() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.standard_usage.begin_turn("attached-session");
        client.handle_session_update(&standard_cost_update("attached-session", 1.25));
        client
            .parse_prompt_response(
                "attached-session",
                &prompt_response_usage(10, 2, 12, None, None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("attached usage");
        assert_eq!(usage.turn_cost_usd, None);
        assert_eq!(usage.cumulative_cost_usd, Some(1.25));
    }

    #[tokio::test]
    async fn standard_usage_two_prompts_preserve_both_monotonic_sequences() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("two-prompt-session");

        client.standard_usage.begin_turn("two-prompt-session");
        client.handle_session_update(&standard_cost_update("two-prompt-session", 0.1));
        client
            .parse_prompt_response(
                "two-prompt-session",
                &prompt_response_usage(10, 2, 12, None, None),
            )
            .unwrap();
        let initial = client.take_turn_usage().expect("initial prompt usage");

        client.standard_usage.begin_turn("two-prompt-session");
        client.handle_session_update(&standard_cost_update("two-prompt-session", 0.25));
        client
            .parse_prompt_response(
                "two-prompt-session",
                &prompt_response_usage(20, 3, 23, None, None),
            )
            .unwrap();
        let user = client.take_turn_usage().expect("user prompt usage");

        assert_eq!((initial.turn_seq, user.turn_seq), (1, 2));
        assert_eq!(
            (initial.turn_input_tokens, user.turn_input_tokens),
            (Some(10), Some(20))
        );
        assert_eq!(
            (initial.turn_cost_usd, user.turn_cost_usd),
            (Some(0.1), Some(0.15))
        );
    }

    #[tokio::test]
    async fn goose_usage_stays_exclusive_and_drains_standard_usage() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.goose_usage.begin_turn("goose-session");
        client.standard_usage.begin_turn("goose-session");
        client.handle_goose_usage_update(&goose_usage_update_msg("goose-session", 1000, 200, None));
        client
            .parse_prompt_response(
                "goose-session",
                &prompt_response_usage(100, 20, 120, None, None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("goose usage");
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(
            usage.turn_input_tokens, None,
            "goose first delta remains exclusive"
        );
        assert!(
            client.take_turn_usage().is_none(),
            "standard usage was drained"
        );
    }

    // ── Goose usage notification integration ──────────────────────────────

    /// Build a `_goose/unstable/session/update` JSON-RPC notification.
    fn goose_usage_update_msg(
        session_id: &str,
        input: u64,
        output: u64,
        cost: Option<f64>,
    ) -> serde_json::Value {
        let mut update = serde_json::json!({
            "sessionUpdate": "usage_update",
            "used": input + output,
            "contextLimit": 200000u64,
            "accumulatedInputTokens": input,
            "accumulatedOutputTokens": output,
        });
        if let Some(c) = cost {
            update["accumulatedCost"] = serde_json::json!(c);
        }
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": {
                "sessionId": session_id,
                "update": update
            }
        })
    }

    #[tokio::test]
    async fn goose_usage_notification_recorded_and_take_returns_usage() {
        let mut client = spawn_inert_client().await;
        assert!(client.take_turn_usage().is_none(), "starts empty");

        // begin_turn before sending the prompt — mirrors the real call flow.
        client.goose_usage.begin_turn("s1");
        let msg = goose_usage_update_msg("s1", 1000, 200, Some(0.01));
        client.handle_goose_usage_update(&msg);

        let usage = client
            .take_turn_usage()
            .expect("usage should be present after notification");
        assert_eq!(usage.session_id, "s1");
        assert_eq!(usage.turn_seq, 1);
        assert!(!usage.delta_reliable, "first turn must be unreliable");
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(usage.cumulative_output_tokens, Some(200));
        assert_eq!(usage.cumulative_cost_usd, Some(0.01));

        // Second take must be None.
        assert!(
            client.take_turn_usage().is_none(),
            "take after drain is None"
        );
    }

    #[tokio::test]
    async fn goose_usage_second_turn_delta_reliable() {
        let mut client = spawn_inert_client().await;
        // Turn 1.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1000, 200, None));
        let _ = client.take_turn_usage();
        // Turn 2.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1800, 450, None));
        let usage = client.take_turn_usage().expect("turn 2 usage");
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(800));
        assert_eq!(usage.turn_output_tokens, Some(250));
    }

    #[tokio::test]
    async fn goose_usage_malformed_notification_does_not_panic() {
        let mut client = spawn_inert_client().await;
        // Missing params entirely.
        let bad = serde_json::json!({"jsonrpc":"2.0","method":"_goose/unstable/session/update"});
        client.handle_goose_usage_update(&bad);
        assert!(client.take_turn_usage().is_none());

        // params present but wrong shape.
        let bad2 = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": { "oops": true }
        });
        client.handle_goose_usage_update(&bad2);
        assert!(client.take_turn_usage().is_none());
    }

    #[test]
    fn agent_error_from_json_falls_back_to_full_json_when_message_missing() {
        // Errors without a string `message` field (e.g. only a `data` field) must
        // not be silently truncated to "unknown error" — the full JSON is preserved.
        let error = serde_json::json!({"code": -32000, "data": "quota exceeded"});
        match super::agent_error_from_json(&error) {
            AcpError::AgentError { code, message } => {
                assert_eq!(code, -32000);
                assert!(
                    message.contains("quota exceeded"),
                    "expected full JSON in message, got: {message}"
                );
            }
            other => panic!("expected AgentError, got {other:?}"),
        }
    }

    #[test]
    fn agent_error_from_json_uses_message_field_when_present() {
        let error = serde_json::json!({"code": -32001, "message": "auth denied"});
        match super::agent_error_from_json(&error) {
            AcpError::AgentError { code, message } => {
                assert_eq!(code, -32001);
                assert_eq!(message, "auth denied");
            }
            other => panic!("expected AgentError, got {other:?}"),
        }
    }

    // ── build_codex_config_env ────────────────────────────────────────────────

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const GENERATED: &str = r#"{"sandbox_workspace_write":{"network_access":true}}"#;

    #[test]
    fn build_codex_config_env_returns_none_when_no_codex_config_in_extra_env() {
        // Non-Codex agents: extra_env has no CODEX_CONFIG → None regardless of signal.
        let extra = env(&[("GOOSE_PROVIDER", "openai")]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "no CODEX_CONFIG in extra_env must return None"
        );
    }

    #[test]
    fn goose_openai_spawn_derives_provider_env_from_compatible_contract() {
        let extra = env(&[
            ("GOOSE_PROVIDER", "openai"),
            ("OPENAI_COMPAT_BASE_URL", "http://spark:4000/v1"),
            ("OPENAI_COMPAT_API_KEY", "dummy"),
        ]);

        assert_eq!(
            derived_goose_provider_env("/opt/bin/goose", &extra, &HashMap::new()),
            env(&[
                ("GOOSE_PROVIDER__HOST", "http://spark:4000/v1"),
                ("GOOSE_PROVIDER__API_KEY", "dummy"),
            ])
        );
    }

    #[test]
    fn goose_provider_specific_env_is_never_overwritten_by_compat_bridge() {
        let extra = env(&[
            ("GOOSE_PROVIDER", "openai"),
            ("OPENAI_COMPAT_BASE_URL", "http://compat"),
            ("OPENAI_COMPAT_API_KEY", "compat-key"),
            ("GOOSE_PROVIDER__HOST", "http://explicit"),
        ]);
        let parent = HashMap::from([(
            "GOOSE_PROVIDER__API_KEY".to_string(),
            "operator-key".to_string(),
        )]);

        assert!(derived_goose_provider_env("goose", &extra, &parent).is_empty());
    }

    #[test]
    fn compatible_env_bridge_is_goose_openai_only_and_parent_source_wins() {
        let extra = env(&[
            ("GOOSE_PROVIDER", "openai"),
            ("OPENAI_COMPAT_BASE_URL", "http://persona"),
        ]);
        let parent = HashMap::from([(
            "OPENAI_COMPAT_BASE_URL".to_string(),
            "http://operator".to_string(),
        )]);
        assert_eq!(
            derived_goose_provider_env("goose", &extra, &parent),
            env(&[("GOOSE_PROVIDER__HOST", "http://operator")])
        );
        assert!(derived_goose_provider_env("codex", &extra, &parent).is_empty());

        let anthropic = env(&[("GOOSE_PROVIDER", "anthropic")]);
        assert!(derived_goose_provider_env("goose", &anthropic, &parent).is_empty());
    }

    #[test]
    fn build_codex_config_env_generated_only_single_entry_with_signal_true_merges_with_parent() {
        // No persona: Buzz injects one CODEX_CONFIG; signal=true.
        // Parent may have its own CODEX_CONFIG — deep_merge applies, network_access forced.
        let extra = env(&[("CODEX_CONFIG", GENERATED)]);
        let parent =
            r#"{"some_operator_key":"val","sandbox_workspace_write":{"operator_key":"keep"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // network_access forced true even though only one entry in extra_env.
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true with signal=true"
        );
        // Operator key preserved via deep_merge.
        assert_eq!(
            v["sandbox_workspace_write"]["operator_key"], "keep",
            "operator nested key must survive"
        );
        assert_eq!(
            v["some_operator_key"], "val",
            "operator top-level key must survive"
        );
    }

    #[test]
    fn build_codex_config_env_persona_only_signal_false_returns_none() {
        // Persona set CODEX_CONFIG; Buzz did not inject a generated overlay (signal=false).
        // Must return None — no merging, no sandbox widening.
        let persona = r#"{"some_feature":"on"}"#;
        let extra = env(&[("CODEX_CONFIG", persona)]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "persona-only CODEX_CONFIG with signal=false must return None"
        );
    }

    #[test]
    fn build_codex_config_env_returns_none_for_persona_only_no_generated_overlay() {
        // Alias: same scenario as above, confirms the old count-based path no longer exists.
        let persona = r#"{"some_feature":"on"}"#;
        let extra = env(&[("CODEX_CONFIG", persona)]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "persona-only CODEX_CONFIG with signal=false must return None"
        );
    }

    #[test]
    fn build_codex_config_env_sets_network_access_from_scratch() {
        // Persona + generated overlay, signal=true: network_access is forced true.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let merged = build_codex_config_env(&extra, None, true).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
    }

    #[test]
    fn build_codex_config_env_persona_keys_survive_merge() {
        // Persona has CODEX_CONFIG with unrelated keys; generated overlay must
        // force network_access=true without erasing persona keys.
        let persona_cfg = r#"{"some_feature":{"enabled":true}}"#;
        // Config::from_args appends generated AFTER persona env vars.
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let merged = build_codex_config_env(&extra, None, true).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            v["some_feature"]["enabled"], true,
            "persona key must survive merge"
        );
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true"
        );
    }

    #[test]
    fn build_codex_config_env_nested_persona_keys_survive_when_parent_has_same_top_level_key() {
        // Persona has sandbox_workspace_write.persona_only; parent has
        // sandbox_workspace_write.parent_only.  A flat top-level spread would drop
        // persona_only.  deep_merge must preserve both nested keys, and
        // network_access must be forced true last.
        let persona_cfg = r#"{"sandbox_workspace_write":{"persona_only":"keep_me"}}"#;
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let parent = r#"{"sandbox_workspace_write":{"parent_only":"also_here"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Both nested keys survive — no flat-spread drop.
        assert_eq!(
            v["sandbox_workspace_write"]["persona_only"], "keep_me",
            "nested persona key must survive when parent has the same top-level key"
        );
        assert_eq!(
            v["sandbox_workspace_write"]["parent_only"], "also_here",
            "nested parent key must be present"
        );
        // Forced last.
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true"
        );
    }

    #[test]
    fn build_codex_config_env_parent_env_wins_on_collisions_persona_keys_survive() {
        // Parent env has CODEX_CONFIG with some keys; persona has different keys.
        // Parent wins on collision; unrelated persona keys survive.
        // network_access is always forced true.
        let persona_cfg = r#"{"persona_key":"persona_val","shared_key":"persona_version"}"#;
        // Config::from_args appends generated AFTER persona env vars.
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let parent = r#"{"parent_key":"parent_val","shared_key":"parent_version"}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Parent-only key present
        assert_eq!(
            v["parent_key"], "parent_val",
            "parent-only key must be present"
        );
        // Unrelated persona key survives (no collision with parent)
        assert_eq!(
            v["persona_key"], "persona_val",
            "unrelated persona key must survive"
        );
        // Collision: parent wins
        assert_eq!(
            v["shared_key"], "parent_version",
            "parent must win on colliding key"
        );
        // network_access always true (forced last)
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
    }

    #[test]
    fn build_codex_config_env_parent_has_existing_sandbox_other_keys_survive() {
        // Parent env has sandbox_workspace_write with extra keys; after merge
        // those extra keys survive alongside network_access=true.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let parent =
            r#"{"sandbox_workspace_write":{"network_access":false,"other_sandbox_key":"val"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // network_access forced true even though parent set false
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
        // other_sandbox_key survives (parent's sws merged, then network_access forced)
        assert_eq!(v["sandbox_workspace_write"]["other_sandbox_key"], "val");
    }

    #[test]
    fn build_codex_config_env_errors_on_invalid_persona_json() {
        // Bad persona JSON + generated overlay, signal=true → parse error before merging.
        let extra = env(&[("CODEX_CONFIG", "not-json"), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, None, true);
        assert!(result.is_err(), "invalid persona JSON must return Err");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("CODEX_CONFIG"),
            "error must mention CODEX_CONFIG"
        );
    }

    #[test]
    fn build_codex_config_env_errors_on_non_object_persona_json() {
        // Non-object persona JSON + generated overlay, signal=true → parse error.
        let extra = env(&[("CODEX_CONFIG", "[1,2,3]"), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, None, true);
        assert!(result.is_err(), "non-object persona JSON must return Err");
    }

    #[test]
    fn build_codex_config_env_errors_on_invalid_parent_json() {
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, Some("bad-json"), true);
        assert!(result.is_err(), "invalid parent env JSON must return Err");
    }

    #[test]
    fn build_codex_config_env_errors_on_non_object_sandbox_workspace_write() {
        // sandbox_workspace_write must be an object for network_access forcing.
        // If the parent env sets it to a non-object scalar, deep_merge replaces
        // our object with the scalar, and the force step must fail clearly.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        // Parent replaces the object with a scalar — deep_merge: scalar overlay wins.
        let parent = r#"{"sandbox_workspace_write": 42}"#;
        let result = build_codex_config_env(&extra, Some(parent), true);
        assert!(
            result.is_err(),
            "non-object sandbox_workspace_write must return Err"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandbox_workspace_write"),
            "error must mention sandbox_workspace_write"
        );
    }
}

#[cfg(all(test, unix))]
mod subscription_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn claude_subscription_rejects_cloud_profile_and_host_authentication() {
        const MARKER: &str = "BUZZ_TEST_CLAUDE_SUBSCRIPTION_CHILD";
        const KEYS: &[&str] = &[
            "CLAUDE_CODE_USE_ANTHROPIC_AWS",
            "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
            "CLAUDE_CODE_USE_MANTLE",
            "ANTHROPIC_PROFILE",
            "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
            "CLAUDE_CODE_GATEWAY_TOKEN_FILE_DESCRIPTOR",
            "CLAUDE_CODE_HOST_AUTH_ENV_VAR",
            "CLAUDE_CODE_HOST_CREDS_FILE",
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
            "ANTHROPIC_UNIX_SOCKET",
            "ANTHROPIC_CUSTOM_HEADERS",
        ];
        if std::env::var_os(MARKER).is_none() {
            let root = tempfile::tempdir().unwrap();
            let adapter = root.path().join("claude-agent-acp");
            std::fs::write(
                &adapter,
                format!(
                    r#"#!/usr/bin/python3
import json, os, sys
keys = {}
for line in sys.stdin:
    req = json.loads(line)
    print(json.dumps({{'jsonrpc':'2.0','id':req['id'],'result':{{
      'sessionId':'fixture', 'auth_keys':[key for key in keys if key in os.environ],
      'home':os.environ.get('CLAUDE_SECURESTORAGE_CONFIG_DIR'),
      'buzz':os.environ.get('BUZZ_AUTH_TAG')}}}}), flush=True)
"#,
                    serde_json::to_string(KEYS).unwrap()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut test = std::process::Command::new(std::env::current_exe().unwrap());
            test.args(["--exact", "acp::subscription_tests::claude_subscription_rejects_cloud_profile_and_host_authentication", "--nocapture"])
                .env(MARKER, &adapter)
                .env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID", "fixture-id")
                .env("BUZZ_ACP_SUBSCRIPTION_HARNESS", "claude")
                .env("BUZZ_ACP_SUBSCRIPTION_HOME", root.path())
                .env("BUZZ_ACP_SUBSCRIPTION_CONFIG_HOME", root.path())
                .env("BUZZ_AUTH_TAG", "fixture-communication");
            for late_override in [false, true] {
                test.env(
                    "BUZZ_TEST_CLAUDE_LATE_OVERRIDE",
                    if late_override { "1" } else { "0" },
                );
                for key in KEYS {
                    if late_override {
                        test.env_remove(key);
                    } else {
                        test.env(key, "fixture-inherited");
                    }
                }
                let output = test.output().unwrap();
                assert!(
                    output.status.success(),
                    "late_override={late_override}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        }
        let adapter = std::env::var(MARKER).unwrap();
        {
            let late_override = std::env::var("BUZZ_TEST_CLAUDE_LATE_OVERRIDE").unwrap() == "1";
            let env = if late_override {
                KEYS.iter()
                    .map(|key| ((*key).into(), "fixture-late".into()))
                    .collect()
            } else {
                vec![]
            };
            let mut client = AcpClient::spawn(&adapter, &[], &env, false).await.unwrap();
            let result = client
                .send_request("fixture", serde_json::json!({}))
                .await
                .unwrap();
            assert_eq!(result["auth_keys"], serde_json::json!([]));
            assert_eq!(
                result["home"].as_str(),
                std::env::var("BUZZ_ACP_SUBSCRIPTION_HOME").ok().as_deref()
            );
            assert_eq!(result["buzz"], "fixture-communication");
            let project = tempfile::tempdir().unwrap();
            std::fs::create_dir(project.path().join(".claude")).unwrap();
            // Exercise the actual session admission call after adapter spawn.
            for key in KEYS {
                std::fs::write(
                    project.path().join(".claude/settings.json"),
                    serde_json::json!({"env": {*key: "fixture"}}).to_string(),
                )
                .unwrap();
                let error = client
                    .session_new_full(project.path().to_str().unwrap(), vec![], None, None)
                    .await
                    .err()
                    .unwrap_or_else(|| panic!("{key} reached a session from settings"));
                assert!(
                    error
                        .to_string()
                        .contains("conflicts with subscription authentication"),
                    "{error}"
                );
            }
            client.shutdown().await;
        }
    }

    /// The parent invokes this exact test in a child test process so trusted
    /// Desktop env can be exercised without mutating the parallel test runner.
    #[tokio::test]
    async fn subscription_child_environment_is_authoritative() {
        const MARKER: &str = "BUZZ_TEST_SUBSCRIPTION_CHILD";
        if std::env::var_os(MARKER).is_none() {
            let root = tempfile::tempdir().unwrap();
            let adapter = root.path().join("codex-acp");
            std::fs::write(&adapter,r#"#!/usr/bin/python3
import json, os, sys
for line in sys.stdin:
    req=json.loads(line)
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':{
      'home':os.environ.get('CODEX_HOME'), 'key':os.environ.get('OPENAI_API_KEY'),
      'buzz':os.environ.get('BUZZ_AUTH_TAG'), 'config':json.loads(os.environ['CODEX_CONFIG'])}}),flush=True)
"#).unwrap();
            std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "acp::subscription_tests::subscription_child_environment_is_authoritative",
                    "--nocapture",
                ])
                .env(MARKER, &adapter)
                .env("BUZZ_ACP_SUBSCRIPTION_ACCOUNT_ID", "fixture-id")
                .env("BUZZ_ACP_SUBSCRIPTION_HARNESS", "codex")
                .env("BUZZ_ACP_SUBSCRIPTION_HOME", root.path())
                .env("OPENAI_API_KEY", "parent-fixture-competing")
                .env("BUZZ_AUTH_TAG", "fixture-communication")
                .env_remove("CODEX_CONFIG")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let adapter = std::env::var(MARKER).unwrap();
        let mut client=AcpClient::spawn(&adapter,&[],&[("OPENAI_API_KEY".into(),"late-fixture-competing".into()),("CODEX_HOME".into(),"wrong-home".into()),("CODEX_CONFIG".into(),r#"{"model":"fixture-model","cli_auth_credentials_store":"keyring","forced_login_method":"api"}"#.into())],false).await.unwrap();
        let result = client
            .send_request("fixture", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            result["home"].as_str(),
            std::env::var("BUZZ_ACP_SUBSCRIPTION_HOME").ok().as_deref()
        );
        assert!(result["key"].is_null());
        assert_eq!(result["buzz"], "fixture-communication");
        assert_eq!(result["config"]["model"], "fixture-model");
        assert_eq!(result["config"]["cli_auth_credentials_store"], "file");
        assert_eq!(result["config"]["forced_login_method"], "chatgpt");
        client.shutdown().await;
        assert!(AcpClient::spawn(
            &adapter,
            &["--resume".into(), "other-account-session".into()],
            &[],
            false
        )
        .await
        .is_err());
        assert!(AcpClient::spawn(
            &adapter,
            &[],
            &[(
                "CODEX_CONFIG".into(),
                r#"{"model_provider":"other"}"#.into()
            )],
            false
        )
        .await
        .is_err());
    }

    #[test]
    fn subscription_session_rejects_new_project_auth_override() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".claude");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("settings.json"),
            r#"{"apiKeyHelper":"competing-helper"}"#,
        )
        .unwrap();
        assert!(validate_subscription_project_context("claude", root.path()).is_err());
    }
}
