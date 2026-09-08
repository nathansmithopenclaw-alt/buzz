//! One bounded, input-fenced completion attempt when OpenCode ends without a report.
use crate::{
    acp::{AcpClient, AcpError, StopReason},
    execution_evidence as evidence,
    pool::PromptContext,
};
use anyhow::ensure;
use std::time::Duration;

fn published(turn: &evidence::ActiveTurn, ctx: &PromptContext) -> anyhow::Result<bool> {
    let directory = turn.root.join("turns").join(&turn.turn.turn_id);
    let path = directory.join("report-receipt.json");
    if !path.try_exists()? {
        return Ok(false);
    }
    let receipt: serde_json::Value = evidence::read(&path)?;
    let pending: serde_json::Value = evidence::read(&directory.join("report.json"))?;
    let event: nostr::Event = serde_json::from_value(pending["event"].clone())?;
    event.verify()?;
    ensure!(
        receipt["event_id"] == event.id.to_hex() && receipt["readback_verified"] == true,
        "Invalid report publication receipt"
    );
    ensure!(
        event.pubkey == ctx.rest_client.keys.public_key(),
        "Report receipt has a different signer"
    );
    ensure!(
        event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["h", &turn.turn.channel_id.to_string()]),
        "Report receipt has a different channel"
    );
    let thread = crate::queue::parse_thread_tags(&event);
    ensure!(
        thread.parent_event_id.as_deref() == Some(&turn.turn.reply_event_id)
            && thread.root_event_id.as_deref() == Some(&turn.turn.root_event_id),
        "Report receipt has a different reply target"
    );
    Ok(true)
}

pub(crate) async fn prompt(
    client: &mut AcpClient,
    session: &str,
    blocks: &[&str],
    turn: Option<&evidence::ActiveTurn>,
    ctx: &PromptContext,
) -> Result<StopReason, AcpError> {
    let started = tokio::time::Instant::now();
    let result = client
        .session_prompt_blocks_with_idle_timeout(
            session,
            blocks,
            ctx.idle_timeout,
            ctx.max_turn_duration,
        )
        .await?;
    let Some(turn) = turn.filter(|_| ctx.harness_name == "opencode") else {
        return Ok(result);
    };
    let delivery = client.turn_delivery_evidence();
    if result != StopReason::EndTurn
        || !delivery.final_answer_present
        || delivery.adapter_runtime_failure_present
        || delivery.publish_attempt_count > 0
        || published(turn, ctx).map_err(|e| AcpError::Protocol(e.to_string()))?
    {
        return Ok(result);
    }
    let remaining = ctx
        .max_turn_duration
        .saturating_sub(started.elapsed())
        .min(Duration::from_secs(60));
    if remaining < Duration::from_secs(5) {
        return Ok(result);
    }
    let mut active = evidence::active(&turn.root, &turn.turn.session)
        .map_err(|e| AcpError::Protocol(e.to_string()))?;
    if active.turn_id != turn.turn.turn_id {
        return Err(AcpError::Protocol(
            "Report completion turn was replaced".into(),
        ));
    }
    active.report_only = true;
    evidence::write(&turn.root.join("active.json"), &active, false)
        .map_err(|e| AcpError::Protocol(e.to_string()))?;
    // The caller's existing cancellation select covers both RPCs. No input is
    // re-enabled, no model prose is auto-published, and no original work is replayed.
    let instruction = format!(
        "This addressed turn is not delivered: your terminal text was not published to Buzz, and no verified report receipt exists. Complete this same turn now by calling buzz-report report_publish with turn_id {} and your full report as its literal content argument. Only reporting, observation and cleanup are permitted; host input is now fenced. Preserve exact document contents and host execution facts; do not redo any work. Do not use shell or CLI publication. Text alone does not deliver a report. This is the single bounded publication-completion attempt.", turn.turn.turn_id);
    tracing::info!(turn_id=%turn.turn.turn_id, "starting input-fenced report completion");
    let completion = client
        .session_report_completion(session, &instruction, remaining)
        .await?;
    if !published(turn, ctx).map_err(|e| AcpError::Protocol(e.to_string()))? {
        tracing::warn!(turn_id=%turn.turn.turn_id, "report completion ended without a verified receipt; publication watchdog retains failure");
    }
    Ok(completion)
}
