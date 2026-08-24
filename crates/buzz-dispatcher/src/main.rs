//! `buzz-dispatcher` entry point.
//!
//! Subcommands:
//!
//! - `check-config` — load + validate configuration and print a redacted summary.
//! - `run` — resident capture/route/escalate/heartbeat loop (lands in a later
//!   checkpoint of this lane).
//! - `watchdog` — external liveness watcher over kind-40024 heartbeats (later
//!   checkpoint).

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

/// Resident Buzz Tracker triage dispatcher.
#[derive(Debug, Parser)]
#[command(name = "buzz-dispatcher", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Load and validate the configuration file, printing a redacted summary.
    CheckConfig {
        /// Path to the dispatcher config JSON.
        #[arg(long)]
        config: PathBuf,
    },
    /// Run the resident dispatcher loop.
    Run {
        /// Path to the dispatcher config JSON.
        #[arg(long)]
        config: PathBuf,
    },
    /// Run the external heartbeat watchdog.
    Watchdog {
        /// Path to the dispatcher config JSON.
        #[arg(long)]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::CheckConfig { config } => check_config(&config),
        Command::Run { config } => {
            let (summary, _) = load_validated(&config)?;
            println!("{summary}");
            anyhow::bail!(
                "run mode is not implemented yet in this checkpoint of the P2 dispatcher lane"
            )
        }
        Command::Watchdog { config } => {
            let (summary, _) = load_validated(&config)?;
            println!("{summary}");
            anyhow::bail!(
                "watchdog mode is not implemented yet in this checkpoint of the P2 dispatcher lane"
            )
        }
    }
}

fn load_validated(
    path: &Path,
) -> anyhow::Result<(String, buzz_dispatcher::config::DispatcherConfig)> {
    let config = buzz_dispatcher::config::DispatcherConfig::load(path)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    // Identity must also load for run/watchdog modes; never print it.
    let (keys, auth_tag) =
        buzz_dispatcher::config::identity_from_env().map_err(|error| anyhow::anyhow!("{error}"))?;
    tracing::info!(
        pubkey = %keys.public_key().to_hex(),
        auth_tag_present = auth_tag.is_some(),
        "dispatcher identity loaded"
    );
    Ok((redacted_summary(&config), config))
}

fn check_config(path: &Path) -> anyhow::Result<()> {
    let config = buzz_dispatcher::config::DispatcherConfig::load(path)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{}", redacted_summary(&config));
    Ok(())
}

/// One-line-per-field summary with no secret material.
fn redacted_summary(config: &buzz_dispatcher::config::DispatcherConfig) -> String {
    let routing_labels: Vec<&str> = config
        .routing
        .iter()
        .map(|rule| rule.label.as_str())
        .collect();
    let threshold_ids: Vec<&str> = config
        .thresholds
        .iter()
        .map(|threshold| threshold.id.as_str())
        .collect();
    format!(
        "config OK\n  relay_url: {}\n  service_id: {}\n  stream_id: {}\n  poll_interval_secs: {}\n  pin_emoji: {}\n  monitored_channels: {}\n  default_channel: {}\n  routing labels: {:?}\n  thresholds: {:?}\n  decision_surface channel: {}\n  liveness channel: {} (stale after {}s)\n  state_dir: {}",
        config.relay_url(),
        config.service_id,
        config.stream_id,
        config.poll_interval_secs,
        config.pin_emoji,
        config.monitored_channels.len(),
        config
            .default_channel
            .map(|channel| channel.to_string())
            .unwrap_or_else(|| "(none)".into()),
        routing_labels,
        threshold_ids,
        config.decision_surface.channel,
        config.liveness.channel,
        config.liveness.stale_after_secs,
        config.state_dir.display(),
    )
}
