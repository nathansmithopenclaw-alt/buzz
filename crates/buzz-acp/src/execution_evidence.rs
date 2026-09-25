//! Host-recorded computer-use facts and scoped, durable turn routing.
//! Tool content and provider prose are deliberately excluded from this journal.
use crate::{
    acp::{EnvVar, McpServer},
    pool::PromptContext,
    queue::FlushBatch,
    scope::SessionScope,
};
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const MAX_FILE: u64 = 128 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Turn {
    pub version: u32,
    pub session: String,
    pub turn_id: String,
    pub channel_id: uuid::Uuid,
    pub root_event_id: String,
    pub reply_event_id: String,
    pub event_ids: Vec<String>,
    pub previous_turns: Vec<String>,
    pub recovery: bool,
    #[serde(default)]
    pub report_only: bool,
    pub active: bool,
}

pub(crate) fn private_dir(path: &Path) -> anyhow::Result<()> {
    ensure!(path.is_absolute(), "Execution directory must be absolute");
    for parent in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(parent) {
            ensure!(
                !meta.file_type().is_symlink(),
                "Execution directory contains a symlink"
            );
        }
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    for parent in path.ancestors() {
        ensure!(
            !fs::symlink_metadata(parent)?.file_type().is_symlink(),
            "Execution record contains a symlink"
        );
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= MAX_FILE,
        "Invalid execution record size"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "Execution record grew beyond limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

/// Atomically persist complete JSON. `exclusive` never replaces an existing receipt.
pub(crate) fn write(path: &Path, value: &impl Serialize, exclusive: bool) -> anyhow::Result<()> {
    let parent = path.parent().context("Missing execution directory")?;
    private_dir(parent)?;
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "Execution record exceeds limit"
    );
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        if exclusive {
            fs::hard_link(&temp, path)?;
        } else {
            fs::rename(&temp, path)?;
        }
        fs::File::open(parent)?.sync_all()?;
        anyhow::Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

pub(crate) fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok() && !id.contains(['/', '\\'])
}

fn scope_root(ctx: &PromptContext, scope: &SessionScope) -> Option<PathBuf> {
    let server = ctx
        .mcp_servers
        .iter()
        .find(|server| server.name == "buzz-computer-use")?;
    let registry = server
        .env
        .iter()
        .find(|e| e.name == "BUZZ_COMPUTER_REGISTRY")?;
    let identity = format!(
        "{}\n{}\n{}\n{}",
        ctx.rest_client.base_url,
        ctx.rest_client.keys.public_key(),
        scope.channel_id(),
        scope.root_event_id().unwrap_or("")
    );
    Some(
        PathBuf::from(&registry.value)
            .join("execution")
            .join(hex::encode(Sha256::digest(identity))),
    )
}

// Preserve a multicall personality symlink (Sprig dispatches from argv[0]).
pub(crate) fn reporter_command() -> anyhow::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let invocation = PathBuf::from(
        std::env::args_os()
            .next()
            .context("Missing harness invocation")?,
    );
    let candidates = std::iter::once(std::env::current_dir()?.join(&invocation)).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(&invocation))
            .collect::<Vec<_>>(),
    );
    Ok(candidates
        .into_iter()
        .find(|path| path.canonicalize().is_ok_and(|target| target == executable))
        .unwrap_or(executable))
}

/// New provider sessions get a new grant; old sessions cannot publish or start input.
pub(crate) fn session_servers(
    ctx: &PromptContext,
    scope: Option<&SessionScope>,
    mut servers: Vec<McpServer>,
) -> anyhow::Result<Vec<McpServer>> {
    let Some(root) = scope.and_then(|scope| scope_root(ctx, scope)) else {
        return Ok(servers);
    };
    private_dir(&root)?;
    let session = uuid::Uuid::new_v4().to_string();
    write(&root.join("session.json"), &session, false)?;
    let grant = vec![
        EnvVar {
            name: "BUZZ_EXECUTION_ROOT".into(),
            value: root.to_string_lossy().into_owned(),
        },
        EnvVar {
            name: "BUZZ_EXECUTION_SESSION".into(),
            value: session,
        },
    ];
    for server in &mut servers {
        if server.name == "buzz-computer-use" {
            server.env.extend(grant.clone());
        }
    }
    let mut env = grant;
    env.push(EnvVar {
        name: "BUZZ_RELAY_URL".into(),
        value: ctx.rest_client.base_url.clone(),
    });
    env.push(EnvVar {
        name: "BUZZ_PRIVATE_KEY".into(),
        value: ctx.rest_client.keys.secret_key().to_secret_hex(),
    });
    if let Some(auth) = &ctx.rest_client.auth_tag_json {
        env.push(EnvVar {
            name: "BUZZ_AUTH_TAG".into(),
            value: auth.clone(),
        });
    }
    servers.push(McpServer {
        name: "buzz-report".into(),
        command: reporter_command()?.to_string_lossy().into_owned(),
        args: vec!["report-mcp".into()],
        env,
    });
    Ok(servers)
}

pub(crate) struct ActiveTurn {
    pub root: PathBuf,
    pub turn: Turn,
}
impl ActiveTurn {
    pub(crate) fn revoke(&self) -> anyhow::Result<()> {
        let mut active = read::<Turn>(&self.root.join("active.json"))?;
        if active.turn_id == self.turn.turn_id {
            active.active = false;
            let path = self.root.join("active.json");
            if let Err(error) = write(&path, &active, false) {
                // Disk-full may prevent an atomic replacement; removing the grant
                // still makes every subsequent call fail before dispatch.
                fs::remove_file(&path)
                    .with_context(|| format!("Cannot revoke execution grant after {error}"))?;
            }
        }
        Ok(())
    }
}
impl Drop for ActiveTurn {
    fn drop(&mut self) {
        if let Err(error) = self.revoke() {
            tracing::error!(%error, "could not retire execution grant");
        }
    }
}

pub(crate) fn begin(
    ctx: &PromptContext,
    batch: Option<&FlushBatch>,
    turn_id: &str,
) -> anyhow::Result<Option<ActiveTurn>> {
    let Some(batch) = batch else {
        return Ok(None);
    };
    let Some(root) = scope_root(ctx, &batch.scope) else {
        return Ok(None);
    };
    let session: String = read(&root.join("session.json"))?;
    ensure!(
        valid_id(&session) && valid_id(turn_id),
        "Invalid execution identity"
    );
    let events: Vec<String> = batch
        .events
        .iter()
        .chain(&batch.cancelled_events)
        .map(|be| be.event.id.to_hex())
        .collect();
    let mut previous_turns = Vec::new();
    let turns = root.join("turns");
    private_dir(&turns)?;
    let entries: Vec<_> = fs::read_dir(&turns)?.take(4097).collect::<Result<_, _>>()?;
    ensure!(
        entries.len() <= 4096,
        "Execution journal retention limit reached; archive reviewed records before continuing"
    );
    for entry in entries {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) {
            continue;
        }
        let prior: Turn = read(&entry.path().join("turn.json"))?;
        if prior.event_ids.iter().any(|id| events.contains(id)) {
            previous_turns.push(id);
        }
    }
    ensure!(previous_turns.len() <= 32, "Too many prior attempts for a bounded recovery; inspect the execution journal before continuing");
    let last = batch
        .events
        .last()
        .context("Execution turn has no triggering event")?;
    let thread = crate::queue::parse_thread_tags(&last.event);
    let reply_event_id = last.event.id.to_hex();
    let recovery =
        !batch.cancelled_events.is_empty() || batch.attempt > 1 || !previous_turns.is_empty();
    let turn = Turn {
        version: 1,
        session,
        turn_id: turn_id.into(),
        channel_id: batch.channel_id,
        root_event_id: thread
            .root_event_id
            .unwrap_or_else(|| reply_event_id.clone()),
        reply_event_id,
        event_ids: events,
        previous_turns,
        recovery,
        report_only: false,
        active: true,
    };
    private_dir(&turns.join(turn_id).join("calls"))?;
    write(&turns.join(turn_id).join("turn.json"), &turn, true)?;
    write(&root.join("active.json"), &turn, false)?;
    Ok(Some(ActiveTurn { root, turn }))
}

pub(crate) fn active(root: &Path, session: &str) -> anyhow::Result<Turn> {
    let current: String = read(&root.join("session.json"))?;
    let turn: Turn = read(&root.join("active.json"))?;
    ensure!(
        turn.version == 1
            && turn.active
            && turn.session == session
            && current == session
            && valid_id(&turn.turn_id),
        "Execution grant is inactive or replaced"
    );
    Ok(turn)
}

/// Facts only: a tool response is not proof of downstream external completion.
pub(crate) fn summary(root: &Path, turn: &Turn) -> anyhow::Result<serde_json::Value> {
    let mut groups = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let mut pending_mutations = 0;
    let mut reports = Vec::new();
    for id in turn
        .previous_turns
        .iter()
        .chain(std::iter::once(&turn.turn_id))
    {
        ensure!(valid_id(id), "Invalid prior execution reference");
        let directory = root.join("turns").join(id);
        let report_path = directory.join("report.json");
        if report_path.try_exists()? {
            let pending: serde_json::Value = read(&report_path)?;
            let event: nostr::Event = serde_json::from_value(pending["event"].clone())?;
            event.verify()?;
            let receipt_path = directory.join("report-receipt.json");
            let verified = if receipt_path.try_exists()? {
                let receipt: serde_json::Value = read(&receipt_path)?;
                receipt["event_id"] == event.id.to_hex() && receipt["readback_verified"] == true
            } else {
                false
            };
            reports.push(serde_json::json!({"turn_id":id,"event_id":event.id,"readback_verified_at_publication":verified,
                "retry_policy":"Reuse this signed event; do not create a replacement for an unresolved delivery"}));
        }
        let calls = directory.join("calls");
        let records: Vec<_> = fs::read_dir(calls)?.take(1025).collect::<Result<_, _>>()?;
        ensure!(
            records.len() <= 1024,
            "Execution evidence exceeds bounded read limit"
        );
        for record in records {
            if record.path().extension().is_none_or(|e| e != "json") {
                continue;
            }
            let fact: serde_json::Value = read(&record.path())?;
            let tool = fact["tool"]
                .as_str()
                .context("Execution record lacks tool")?;
            let state = fact["state"]
                .as_str()
                .context("Execution record lacks state")?;
            ensure!(
                tool.len() <= 100
                    && ["started", "succeeded", "refused", "unknown"].contains(&state),
                "Invalid execution fact"
            );
            *groups
                .entry(tool.into())
                .or_default()
                .entry(state.into())
                .or_default() += 1;
            if ["started", "unknown"].contains(&state) && fact["mutation"] == true {
                pending_mutations += 1;
            }
        }
    }
    Ok(
        serde_json::json!({"turn_id":turn.turn_id,"recovery":turn.recovery,"report_only":turn.report_only,"tools":groups,
        "unknown_mutation_outcomes":pending_mutations,"reports":reports,
        "coverage":"Host computer-use calls only. Succeeded means a tool returned success, not independent external acceptance. Started or unknown actions may have taken effect. Provider built-in tools are not covered.",
        "mutation_policy":if turn.report_only {"Report completion only: new mutations are fenced; publish the existing result without repeating work."} else if turn.recovery {"Recovery turn: inspect, close/release and report; new mutations are fenced to prevent replay. A new explicit user task is required to resume mutations."} else {"normal"}}),
    )
}

pub(crate) fn prompt(active: &ActiveTurn) -> anyhow::Result<String> {
    Ok(crate::prompt_framing::semantic_section("host-execution-evidence", &format!(
        "Host-recorded execution facts survive cancelled provider sessions. Current empty resource lists do not erase these facts. Report with buzz-report report_publish using its structured content argument; never place report text in shell commands. Call execution_status for current facts.\n{}", serde_json::to_string(&summary(&active.root, &active.turn)?)?)))
}
