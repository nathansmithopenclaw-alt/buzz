#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-dispatcher` — resident Buzz Tracker triage dispatcher.
//!
//! Converts tracker stream state into deterministic automation:
//!
//! - **Capture:** pin reactions (config emoji) in monitored channels become
//!   `triage` issues carrying `source_event_id`, with deterministic UUIDv5
//!   request IDs so duplicate/re-pin deliveries collapse to one create.
//! - **Routing:** label-table auto-routing via task-envelope v2 + kind-40023
//!   sharing one `dispatch_request_id`.
//! - **Escalation:** threshold ladder with at-most-once firing per
//!   `(issue, threshold, interval epoch)`.
//! - **Liveness:** kind-40024 heartbeats every tick; an external watchdog
//!   surfaces stale services via channel message only.
//!
//! All identity/location inputs come from config + env; no fleet keys,
//! channel UUIDs, or state paths are hardcoded (binding plan amendments).

pub mod config;
pub mod error;
pub mod projection;
pub mod relay_http;
pub mod rules;

pub use error::DispatchError;
