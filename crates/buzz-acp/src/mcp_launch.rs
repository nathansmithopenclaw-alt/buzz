//! Keep MCP credentials out of adapter-owned argv, wire logs, and observers.
//!
//! All ACP runtimes support stdio MCP commands. The adapter sees only this
//! launcher's path and a private file path; the final child receives the exact
//! environment through Command::envs. Files live with the provider session so
//! adapters can restart MCP children, and are removed on session/adapter teardown.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{ensure, Context, Result};
use tempfile::{NamedTempFile, TempDir};

use crate::acp::{AcpError, McpServer};

const MAX_CONFIG_BYTES: u64 = 10_000_000;

pub(crate) struct LaunchFiles {
    // Drop files before their private parent directory.
    files: Vec<NamedTempFile>,
    _directory: TempDir,
}

/// The common boundary must run before serializing either new or load requests.
pub(crate) fn prepare(
    servers: Vec<McpServer>,
) -> Result<(Vec<McpServer>, Option<LaunchFiles>), AcpError> {
    prepare_inner(servers)
        .map_err(|_| AcpError::Protocol("Cannot prepare private MCP launch configuration".into()))
}

fn prepare_inner(mut servers: Vec<McpServer>) -> Result<(Vec<McpServer>, Option<LaunchFiles>)> {
    let sensitive = |server: &McpServer| {
        server.env.iter().any(|entry| {
            ["BUZZ_PRIVATE_KEY", "BUZZ_AUTH_TAG", "NOSTR_PRIVATE_KEY"]
                .iter()
                .any(|name| entry.name.eq_ignore_ascii_case(name))
        })
    };
    if !servers.iter().any(sensitive) {
        return Ok((servers, None));
    }
    let launcher = crate::execution_evidence::reporter_command()?;
    let directory = tempfile::Builder::new().prefix("buzz-mcp-").tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    let mut files = LaunchFiles {
        files: Vec::new(),
        _directory: directory,
    };
    for server in &mut servers {
        if !sensitive(server) {
            continue;
        }
        let mut file = NamedTempFile::new_in(files._directory.path())?;
        // NamedTempFile creates exclusively with mode 0600 on Unix, before
        // writing any bytes. Never follow an existing path or chmod afterward.
        let bytes = serde_json::to_vec(server)?;
        ensure!(
            bytes.len() as u64 <= MAX_CONFIG_BYTES,
            "MCP launch configuration too large"
        );
        file.write_all(&bytes)?;
        file.flush()?;
        *server = McpServer {
            name: server.name.clone(),
            command: launcher.to_string_lossy().into_owned(),
            args: vec![
                "mcp-launch".into(),
                file.path().to_string_lossy().into_owned(),
            ],
            env: Vec::new(),
        };
        files.files.push(file);
    }
    Ok((servers, Some(files)))
}

fn command_from_file(path: &Path) -> Result<Command> {
    // A managed file is regular and private. Refuse symlinks or permissive
    // files; the private parent directory prevents other users replacing it.
    let metadata = std::fs::symlink_metadata(path).context("Cannot inspect MCP launch file")?;
    ensure!(metadata.file_type().is_file(), "Invalid MCP launch file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "MCP launch file is not private"
        );
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "MCP launch configuration too large"
    );
    let server: McpServer = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid MCP launch configuration"))?;
    let mut command = Command::new(server.command);
    command.args(server.args);
    for entry in server.env {
        command.env(entry.name, entry.value);
    }
    Ok(command)
}

pub(crate) fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(2);
    let path = args.next().context("Missing MCP launch file")?;
    ensure!(args.next().is_none(), "Unexpected MCP launcher argument");
    let mut command = command_from_file(Path::new(&path))?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Preserve PID, process group, cwd and all three stdio handles. No
        // shell, interpolation, additional process, or secret argv is involved.
        Err(command.exec()).context("Cannot execute MCP server")
    }
    #[cfg(not(unix))]
    {
        let status = command.status().context("Cannot execute MCP server")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests;
