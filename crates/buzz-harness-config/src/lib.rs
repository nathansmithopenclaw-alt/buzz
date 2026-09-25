//! Native harness configuration boundaries shared by Desktop and the ACP broker.

pub mod cli_version;
pub mod codex;
pub mod codex_project;

/// Explicit native isolation contract; absent for hosted agents.
pub const CODEX_SESSION_ONLY_ENV: &str = "BUZZ_ACP_CODEX_SESSION_ONLY";
