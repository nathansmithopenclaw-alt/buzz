//! Live authorization for named Summary definitions and their immutable reports.
use crate::AppState;
use buzz_core::kind::{KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT};
use buzz_core::{
    summary::{config_d_tag, valid_pubkey, SummaryAudience, SummaryDefinition},
    CommunityId, Event, StoredEvent,
};
use buzz_db::EventQuery;
use std::collections::BTreeSet;

/// Return a tag only when it has exactly one unambiguous value.
pub fn single_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|key| key == name));
    let value = tags.next()?.as_slice().get(1)?;
    if tags.next().is_some() {
        return None;
    }
    Some(value)
}

/// Whether an event uses the named-Summary envelope.
pub fn is_named_report(event: &Event) -> bool {
    u32::from(event.kind.as_u16()) == KIND_BRIEF_REPORT
        && (event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().is_some_and(|key| key == "summary"))
            || serde_json::from_str::<serde_json::Value>(&event.content)
                .ok()
                .is_some_and(|value| value.get("v").and_then(|v| v.as_u64()) == Some(2)))
}

/// Read the current definition from the writer; replica lag cannot revive access.
pub async fn definition(
    state: &AppState,
    community: CommunityId,
    owner: &str,
    id: &str,
) -> Result<(StoredEvent, SummaryDefinition), String> {
    if !valid_pubkey(owner) {
        return Err("invalid Summary owner".into());
    }
    let events = state
        .db
        .query_events_for_event_write(&EventQuery {
            kinds: Some(vec![KIND_BRIEF_CONFIG as i32]),
            pubkey: Some(hex::decode(owner).map_err(|e| e.to_string())?),
            d_tag: Some(config_d_tag(id)?),
            global_only: true,
            limit: Some(1),
            ..EventQuery::for_community(community)
        })
        .await
        .map_err(|e| e.to_string())?;
    let event = events.into_iter().next().ok_or("Summary does not exist")?;
    let config: SummaryDefinition =
        serde_json::from_str(&event.event.content).map_err(|e| e.to_string())?;
    config.validate()?;
    if config.summary_id != id {
        return Err("Summary identity mismatch".into());
    }
    Ok((event, config))
}

/// Validate source membership for the complete audience before any delegated read or publish.
pub async fn source_access(
    state: &AppState,
    community: CommunityId,
    owner: &str,
    config: &SummaryDefinition,
    people: &BTreeSet<String>,
) -> Result<(), String> {
    let owner_bytes = hex::decode(owner).map_err(|e| e.to_string())?;
    if !state
        .db
        .is_agent_owner(
            community,
            &hex::decode(&config.reporter_pubkey).map_err(|e| e.to_string())?,
            &owner_bytes,
        )
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("Summary agent ownership is no longer current".into());
    }
    for channel in config.channels.iter().chain(&config.dms) {
        let room = state
            .db
            .get_channel_for_event_write(community, *channel)
            .await
            .map_err(|e| e.to_string())?;
        if room.deleted_at.is_some()
            || room.archived_at.is_some()
            || !state
                .db
                .is_member(community, *channel, &owner_bytes)
                .await
                .map_err(|e| e.to_string())?
        {
            return Err("Summary source is unavailable".into());
        }
        if matches!(config.audience, SummaryAudience::Community)
            && (room.channel_type == "dm" || room.visibility != "open")
        {
            return Err("Community Summaries require community-readable sources".into());
        }
        if room.channel_type == "dm" || room.visibility != "open" {
            for person in people {
                if !state
                    .db
                    .is_member(
                        community,
                        *channel,
                        &hex::decode(person).map_err(|e| e.to_string())?,
                    )
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Err("A Summary recipient cannot read a selected source".into());
                }
            }
        }
    }
    Ok(())
}

/// Current audience from authoritative membership, excluding registered agents.
pub async fn audience(
    state: &AppState,
    community: CommunityId,
    owner: &str,
    config: &SummaryDefinition,
) -> Result<BTreeSet<String>, String> {
    if matches!(config.audience, SummaryAudience::Community)
        && !state.config.require_relay_membership
    {
        return Err("Community-wide sharing requires a complete membership directory".into());
    }
    let humans = state
        .db
        .summary_people(community)
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(config.audience, SummaryAudience::OnlyMe) && !humans.contains(owner) {
        return Err("Sharing requires an available community member directory".into());
    }
    config.recipients(owner, &humans)
}

/// Live report authorization in addition to immutable recipient tags.
/// False on every lookup error; identifiers alone never grant access.
pub async fn reader_allowed(
    state: &AppState,
    community: CommunityId,
    event: &Event,
    reader: &[u8],
) -> bool {
    if !is_named_report(event) {
        return true;
    }
    if !buzz_core::filter::reader_authorized_for_event(event, &hex::encode(reader)) {
        return false;
    }
    async {
        let owner = single_tag(event, "owner").ok_or("missing owner")?;
        let id = single_tag(event, "summary").ok_or("missing Summary")?;
        let (_, mut config) = definition(state, community, owner, id).await?;
        let reader = hex::encode(reader);
        if state.config.require_relay_membership
            && !state
                .db
                .is_relay_member(community, &reader)
                .await
                .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
        // Owner can always review their history. Only the actual authoring reporter
        // receives operational readback; unrelated owner-owned agents do not.
        if reader == owner {
            return Ok(true);
        }
        if reader == event.pubkey.to_hex() && config.reporter_pubkey == reader {
            return Ok(true);
        }
        let recipients = audience(state, community, owner, &config).await?;
        if !recipients.contains(&reader) {
            return Ok(false);
        }
        let report: buzz_core::summary::SummaryReport =
            serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
        report.validate()?;
        let run = state
            .db
            .summary_run(
                community,
                &hex::decode(owner).map_err(|e| e.to_string())?,
                id,
                report.run_id,
            )
            .await
            .map_err(|e| e.to_string())?;
        let frozen: SummaryDefinition =
            serde_json::from_value(run.config_snapshot).map_err(|e| e.to_string())?;
        // The model cannot omit a private source from its claimed provenance to
        // avoid later revocation. Reauthorize the complete frozen read grant.
        config.channels = frozen.channels;
        config.dms = frozen.dms;
        source_access(state, community, owner, &config, &[reader].into()).await?;
        Ok::<_, String>(true)
    }
    .await
    .unwrap_or(false)
}

/// Reject malformed definitions and unauthorized report audiences during ingestion.
pub async fn validate_event(
    state: &AppState,
    community: CommunityId,
    event: &Event,
) -> Result<(), String> {
    let version = serde_json::from_str::<serde_json::Value>(&event.content)
        .ok()
        .and_then(|v| v.get("v").and_then(|v| v.as_u64()));
    if u32::from(event.kind.as_u16()) == KIND_BRIEF_CONFIG && version == Some(2) {
        let config: SummaryDefinition =
            serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
        config.validate()?;
        let schedule = config
            .schedule
            .clone()
            .unwrap_or(buzz_core::summary::SummarySchedule {
                daily_times: None,
                cron: Some("0 9 * * *".into()),
                interval: None,
            });
        let workflow: buzz_workflow::schema::WorkflowDef = serde_json::from_value(serde_json::json!({
            "name":config.name, "trigger":{"on":"schedule","cron":if schedule.daily_times.is_some() {Some("0 9 * * *".to_string())} else {schedule.cron},"interval":schedule.interval,"timezone":config.timezone},
            "steps":[{"id":"run","action":"run_summary","summary_id":config.summary_id}]
        })).map_err(|e|e.to_string())?;
        workflow.validate().map_err(|e| e.to_string())?;
        if single_tag(event, "d") != Some(config_d_tag(&config.summary_id)?.as_str()) {
            return Err("Summary config address mismatch".into());
        }
        let owner = event.pubkey.to_hex();
        if !state
            .db
            .is_agent_owner(
                community,
                &hex::decode(&config.reporter_pubkey).map_err(|e| e.to_string())?,
                &event.pubkey.to_bytes(),
            )
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("Choose one of your registered Summary agents".into());
        }
        let actual: BTreeSet<_> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().is_some_and(|key| key == "p"))
                    .then(|| parts.get(1).cloned())
                    .flatten()
            })
            .collect();
        let expected = [owner.clone(), config.reporter_pubkey.clone()].into();
        if actual != expected {
            return Err("Summary configuration must remain private to owner and reporter".into());
        }
        if !config.paused && !config.archived {
            let people = audience(state, community, &owner, &config).await?;
            source_access(state, community, &owner, &config, &people).await?;
        }
    }
    if is_named_report(event) {
        let report: buzz_core::summary::SummaryReport =
            serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
        report.validate()?;
        let owner = single_tag(event, "owner").ok_or("Summary report requires an owner")?;
        let id = single_tag(event, "summary").ok_or("Summary report requires an identifier")?;
        if owner != report.owner_pubkey
            || id != report.summary_id
            || single_tag(event, "config") != Some(report.config_event_id.as_str())
            || single_tag(event, "d") != Some(report.run_id.to_string().as_str())
        {
            return Err("Summary report envelope mismatch".into());
        }
        let (current, config) = definition(state, community, owner, id).await?;
        if config.archived
            || !config.read_delegation
            || config.reporter_pubkey != event.pubkey.to_hex()
            || single_tag(event, "config") != Some(current.event.id.to_hex().as_str())
        {
            return Err("Summary run configuration is no longer current".into());
        }
        let people = audience(state, community, owner, &config).await?;
        source_access(state, community, owner, &config, &people).await?;
        let run = state
            .db
            .summary_run(
                community,
                &hex::decode(owner).map_err(|e| e.to_string())?,
                id,
                report.run_id,
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut expected: BTreeSet<String> =
            serde_json::from_value(run.audience).map_err(|e| e.to_string())?;
        if !expected.is_subset(&people) {
            return Err("Summary audience changed during run".into());
        }
        expected.insert(config.reporter_pubkey.clone());
        let actual: BTreeSet<_> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().is_some_and(|key| key == "p"))
                    .then(|| parts.get(1).cloned())
                    .flatten()
            })
            .collect();
        if actual != expected {
            return Err("Summary report recipients do not match owner policy".into());
        }
    }
    Ok(())
}
