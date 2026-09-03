//! Live-evidence harness: run the real usage-series accounting over a real
//! archive file and print the result as JSON.
//!
//! Deliberately `#[ignore]`d — it needs a real `archive.db` and is not a unit
//! test. It exists because the accounting ladder's correctness on *synthetic*
//! rows and its correctness on *the shapes a live fleet actually produces* are
//! different claims, and only the second one closes a "show me the real
//! numbers" request. Same rationale as the explicitly-excluded terminal
//! latency gate.
//!
//! It runs the production path — `open_archive_db` (so every migration fires,
//! including the M5 rebuild) and `agent_usage_series` (the exact function the
//! Tauri command calls) — over a **copy** of the archive. Never point it at a
//! live file: it writes.
//!
//! ```sh
//! sqlite3 <app-data>/archive/archive.db ".backup '/tmp/copy.db'"
//! BUZZ_LIVE_ARCHIVE=/tmp/copy.db \
//! BUZZ_LIVE_IDENTITY=<64-hex> \
//! BUZZ_LIVE_RELAY=wss://relay.example \
//! BUZZ_LIVE_DAYS=1 \
//!   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
//!     archive::agent_usage_live_evidence -- --ignored --nocapture
//! ```

use super::agent_usage::AgentUsageSeriesRequest;

/// Local-midnight boundaries for the last `days` days, ending after today.
///
/// Mirrors the frontend's `lastNDaysBoundaries`: real calendar arithmetic, not
/// a fixed 86_400 stride, so a DST transition inside the range cannot slide the
/// edges off local midnight.
fn local_day_boundaries(days: i64) -> Vec<i64> {
    use chrono::{Duration, Local, TimeZone};
    let today = Local::now().date_naive();
    let mut out = Vec::new();
    for i in (0..=days).rev() {
        let day = today + Duration::days(1) - Duration::days(i);
        let midnight = day
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always a valid time");
        out.push(
            Local
                .from_local_datetime(&midnight)
                .earliest()
                .expect("a DST-skipped midnight has an earliest mapping")
                .timestamp(),
        );
    }
    out
}

#[test]
#[ignore = "requires a real archive.db copy via BUZZ_LIVE_ARCHIVE"]
fn print_live_agent_usage_series() {
    let Ok(path) = std::env::var("BUZZ_LIVE_ARCHIVE") else {
        panic!("set BUZZ_LIVE_ARCHIVE to a COPY of an archive.db — this test writes");
    };
    let identity = std::env::var("BUZZ_LIVE_IDENTITY")
        .expect("set BUZZ_LIVE_IDENTITY to the archive's identity pubkey (64 hex)");
    let relay =
        std::env::var("BUZZ_LIVE_RELAY").expect("set BUZZ_LIVE_RELAY to the archive's relay URL");
    let days: i64 = std::env::var("BUZZ_LIVE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    // The real open path, so every migration runs exactly as it would on a
    // user's machine — including M5's index rebuild.
    let conn = super::store::open_archive_db(std::path::Path::new(&path))
        .expect("open_archive_db on the archive copy");

    let request = AgentUsageSeriesRequest {
        bucket_boundaries: local_day_boundaries(days),
        agent_pubkey: None,
    };
    let series = super::agent_usage_series(&conn, &identity, &relay, &request)
        .expect("agent_usage_series over the live archive");

    println!(
        "BUZZ_LIVE_SERIES_JSON {}",
        serde_json::to_string(&series).expect("serialize series")
    );
}
