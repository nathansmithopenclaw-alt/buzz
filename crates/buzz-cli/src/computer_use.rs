//! Host-local capability entry point. Computer control never goes through a relay.
use crate::error::CliError;
use clap::Subcommand;
use std::{path::Path, process::Command};

#[derive(Subcommand)]
pub(crate) enum ComputerCmd {
    /// Report host-supplied configuration (does not claim live permission grants)
    Status,
    /// Serve the configured computer-use MCP protocol on stdin/stdout
    Mcp,
}

pub(crate) fn dispatch(command: &ComputerCmd) -> Result<(), CliError> {
    let node = std::env::var("BUZZ_COMPUTER_NODE").ok();
    let script = std::env::var("BUZZ_COMPUTER_SCRIPT").ok();
    let registry = std::env::var("BUZZ_COMPUTER_REGISTRY").ok();
    let owner = std::env::var("BUZZ_COMPUTER_OWNER").ok();
    let configured = [&node, &script, &registry].iter().all(|value| {
        value
            .as_deref()
            .is_some_and(|path| Path::new(path).is_absolute())
    }) && owner
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let dependencies_present = [&node, &script].iter().all(|value| {
        value
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file())
    });
    if matches!(command, ComputerCmd::Status) {
        println!(
            "{}",
            serde_json::json!({
                "configured": configured,
                "dependencies_present": dependencies_present,
                "native_configured": std::env::var_os("BUZZ_NATIVE_SOCKET").is_some(),
                "live_acceptance": "not_checked",
                "instructions": "Enable computer use in Buzz, then start a new managed agent session. The host supplies configuration automatically. Use the MCP computer_status and native_check_permissions tools for runtime checks."
            })
        );
        return Ok(());
    }
    if !configured || !dependencies_present {
        return Err(CliError::Usage("Computer use is not configured by this execution host; enable it in Buzz and start a new agent session".into()));
    }
    let node = node.ok_or_else(|| CliError::Usage("Missing host Node runtime".into()))?;
    let script = script.ok_or_else(|| CliError::Usage("Missing host computer service".into()))?;
    let mut process = Command::new(node);
    process.arg(script);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(CliError::Other(format!(
            "start computer-use MCP: {}",
            process.exec()
        )))
    }
    #[cfg(not(unix))]
    {
        let status = process
            .status()
            .map_err(|e| CliError::Other(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(CliError::Other(format!(
                "Computer-use MCP exited: {status}"
            )))
        }
    }
}
