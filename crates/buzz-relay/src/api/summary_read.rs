//! Bounded read-only Summary delegation through the authenticated Nostr query bridge.
//!
//! `summary_owner` opts a single explicit-channel query into the owner's live,
//! signed Summary configuration. It never alters membership or subscriptions.
use axum::{http::StatusCode, Json};
use buzz_core::{kind::KIND_BRIEF_CONFIG, tenant::TenantContext};
use buzz_db::EventQuery;
use serde_json::Value;
use uuid::Uuid;

use super::{api_error, internal_error};
use crate::state::AppState;

type ReadError = (StatusCode, Json<Value>);

fn denied() -> ReadError {
    api_error(
        StatusCode::FORBIDDEN,
        "restricted: no active Summary read grant",
    )
}

fn grants_channel(config: &Value, reporter: &str, channel: Uuid) -> bool {
    config.get("v").and_then(Value::as_u64) == Some(1)
        && config.get("read_delegation").and_then(Value::as_bool) == Some(true)
        && config
            .get("reporter_pubkey")
            .and_then(Value::as_str)
            .is_some_and(|key| key.eq_ignore_ascii_case(reporter))
        && ["channels", "dms"].iter().any(|key| {
            config
                .get(key)
                .and_then(Value::as_array)
                .is_some_and(|ids| {
                    ids.iter().any(|id| {
                        id.as_str().and_then(|id| Uuid::parse_str(id).ok()) == Some(channel)
                    })
                })
        })
        && !config
            .get("exclude")
            .and_then(Value::as_array)
            .is_some_and(|ids| {
                ids.iter()
                    .any(|id| id.as_str().and_then(|id| Uuid::parse_str(id).ok()) == Some(channel))
            })
}

async fn authorize(
    state: &AppState,
    tenant: &TenantContext,
    owner: &[u8],
    reporter: &str,
    channel: Uuid,
    named: Option<(&str, Uuid)>,
) -> Result<Vec<u8>, ReadError> {
    if let Some((id, run_id)) = named {
        let owner_hex = hex::encode(owner);
        let (event, config) = crate::summary::definition(state, tenant.community(), &owner_hex, id)
            .await
            .map_err(|_| denied())?;
        if config.archived
            || !config.read_delegation
            || config.reporter_pubkey != reporter
            || !config
                .channels
                .iter()
                .chain(&config.dms)
                .any(|id| *id == channel)
        {
            return Err(denied());
        }
        let run = state
            .db
            .summary_run(tenant.community(), owner, id, run_id)
            .await
            .map_err(|_| denied())?;
        if run.state != "running"
            || run.expires_at <= chrono::Utc::now()
            || !run.matches_definition(&config)
            || hex::encode(run.reporter_pubkey) != reporter
        {
            return Err(denied());
        }
        let frozen: std::collections::BTreeSet<String> =
            serde_json::from_value(run.audience).map_err(|_| denied())?;
        let current = crate::summary::audience(state, tenant.community(), &owner_hex, &config)
            .await
            .map_err(|_| denied())?;
        if !frozen.is_subset(&current) {
            return Err(denied());
        }
        crate::summary::source_access(state, tenant.community(), &owner_hex, &config, &frozen)
            .await
            .map_err(|_| denied())?;
        return Ok(event.event.id.to_bytes().to_vec());
    }
    // Writer reads prevent replica lag from reviving revoked grants.
    let configs = state
        .db
        .query_events_for_event_write(&EventQuery {
            kinds: Some(vec![KIND_BRIEF_CONFIG as i32]),
            pubkey: Some(owner.to_vec()),
            d_tag: Some("summary".into()),
            global_only: true,
            limit: Some(1),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .map_err(|e| internal_error(&format!("Summary grant: {e}")))?;
    let config = configs.first().ok_or_else(denied)?;
    let value: Value = serde_json::from_str(&config.event.content).map_err(|_| denied())?;
    if !grants_channel(&value, reporter, channel) {
        return Err(denied());
    }
    if !state
        .db
        .is_member(tenant.community(), channel, owner)
        .await
        .map_err(|e| internal_error(&format!("Summary owner membership: {e}")))?
    {
        return Err(denied());
    }
    let room = state
        .db
        .get_channel_for_event_write(tenant.community(), channel)
        .await
        .map_err(|e| internal_error(&format!("Summary channel: {e}")))?;
    if room.archived_at.is_some() || room.deleted_at.is_some() {
        return Err(denied());
    }
    Ok(config.event.id.to_bytes().to_vec())
}

/// Handle one snapshot query. Unknown fields and mixed filters fail closed.
pub(super) async fn read(
    state: &AppState,
    tenant: &TenantContext,
    reporter: &str,
    raw: &[Value],
) -> Result<Json<Value>, ReadError> {
    if raw.len() != 1 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Summary reads require one filter",
        ));
    }
    let filter = raw[0].as_object().ok_or_else(denied)?;
    if filter.keys().any(|key| {
        ![
            "summary_owner",
            "summary_id",
            "summary_run",
            "#h",
            "since",
            "until",
            "before_id",
            "limit",
        ]
        .contains(&key.as_str())
    }) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "unsupported Summary read filter",
        ));
    }
    let owner = filter
        .get("summary_owner")
        .and_then(Value::as_str)
        .and_then(|key| hex::decode(key).ok())
        .filter(|key| key.len() == 32)
        .ok_or_else(denied)?;
    let named = match (filter.get("summary_id"), filter.get("summary_run")) {
        (None, None) => None,
        (Some(id), Some(run)) => Some((
            id.as_str().ok_or_else(denied)?,
            run.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(denied)?,
        )),
        _ => return Err(denied()),
    };
    let channels = filter
        .get("#h")
        .and_then(Value::as_array)
        .filter(|ids| ids.len() == 1)
        .ok_or_else(denied)?;
    let channel = channels[0]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(denied)?;
    let timestamp = |key: &str| -> Result<Option<chrono::DateTime<chrono::Utc>>, ReadError> {
        filter
            .get(key)
            .map(|value| {
                value
                    .as_i64()
                    .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                    .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid Summary timestamp"))
            })
            .transpose()
    };
    let since = timestamp("since")?;
    let mut until = timestamp("until")?;
    if let Some((id, run_id)) = named {
        let run = state
            .db
            .summary_run(tenant.community(), &owner, id, run_id)
            .await
            .map_err(|_| denied())?;
        let upper = chrono::DateTime::from_timestamp(run.window_until(), 0).ok_or_else(denied)?;
        until = Some(until.map_or(upper, |requested| requested.min(upper)));
    }
    let before_id = filter
        .get("before_id")
        .map(|value| {
            value
                .as_str()
                .and_then(|id| hex::decode(id).ok())
                .filter(|id| id.len() == 32)
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "invalid Summary cursor"))
        })
        .transpose()?;
    if before_id.is_some() && until.is_none() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "before_id requires until",
        ));
    }
    let limit = match filter.get("limit") {
        Some(value) => value
            .as_i64()
            .filter(|n| (1..=200).contains(n))
            .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "Summary limit must be 1–200"))?,
        None => 100,
    };
    let grant = authorize(state, tenant, &owner, reporter, channel, named).await?;
    let mut rows = if let Some((id, run_id)) = named {
        state
            .db
            .summary_source_page(
                tenant.community(),
                &owner,
                id,
                run_id,
                channel,
                since,
                until.ok_or_else(denied)?,
                before_id,
                limit + 1,
            )
            .await
    } else {
        state
            .db
            .query_events_for_event_write(&EventQuery {
                channel_id: Some(channel),
                kinds: Some(vec![
                    buzz_core::kind::KIND_STREAM_MESSAGE as i32,
                    buzz_core::kind::KIND_STREAM_MESSAGE_V2 as i32,
                    buzz_core::kind::KIND_FORUM_POST as i32,
                    buzz_core::kind::KIND_FORUM_COMMENT as i32,
                ]),
                since,
                until,
                before_id,
                limit: Some(limit + 1),
                ..EventQuery::for_community(tenant.community())
            })
            .await
    }
    .map_err(|e| internal_error(&format!("Summary transcript: {e}")))?;
    // A revoke/reassignment during the read invalidates the response as well.
    if authorize(state, tenant, &owner, reporter, channel, named).await? != grant {
        return Err(denied());
    }
    // Named reads apply consumed-message filtering before the page limit.
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if has_more {
        rows.last().map(|row| {
            serde_json::json!({
                "until":row.event.created_at.as_secs(), "before_id":row.event.id.to_hex()
            })
        })
    } else {
        None
    };
    let events: Vec<_> = rows
        .into_iter()
        .filter(|row| row.channel_id == Some(channel))
        .map(|row| row.event)
        .collect();
    if let Some((id, run_id)) = named {
        state
            .db
            .record_summary_reads(tenant.community(), &owner, id, run_id, channel, &events)
            .await
            .map_err(|_| denied())?;
        if authorize(state, tenant, &owner, reporter, channel, named).await? != grant {
            return Err(denied());
        }
    }
    Ok(Json(
        serde_json::json!({"summary_read": if named.is_some() {2} else {1}, "events": events,
            "has_more":has_more,"next":next}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grant_requires_explicit_consent_exact_reporter_and_selected_room() {
        let room = Uuid::new_v4();
        let mut config = serde_json::json!({"v":1,"read_delegation":true,"reporter_pubkey":"abcd","dms":[room.to_string()],"exclude":[]});
        assert!(grants_channel(&config, "ABCD", room));
        assert!(!grants_channel(&config, "other", room));
        assert!(!grants_channel(&config, "abcd", Uuid::new_v4()));
        config["read_delegation"] = Value::Bool(false);
        assert!(!grants_channel(&config, "abcd", room));
        config["read_delegation"] = Value::Bool(true);
        config["exclude"] = serde_json::json!([room.to_string()]);
        assert!(!grants_channel(&config, "abcd", room));
    }
}
