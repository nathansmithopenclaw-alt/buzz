//! Authenticated Summary control operations, dispatched by the query bridge.
use super::api_error;
use crate::AppState;
use axum::{http::StatusCode, Json};
use buzz_core::tenant::TenantContext;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    summary_operation: String,
    owner: Option<String>,
    summary_id: Option<String>,
    run_id: Option<Uuid>,
    state: Option<String>,
    message: Option<String>,
    before_time: Option<i64>,
    before_id: Option<String>,
    source_report: Option<String>,
}

pub(super) async fn handle(
    state: &AppState,
    tenant: &TenantContext,
    actor: &str,
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let denied = || api_error(StatusCode::FORBIDDEN, "Summary operation is not permitted");
    if filters.len() != 1 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Summary operations require one request",
        ));
    }
    if filters[0]
        .get("summary_operation")
        .and_then(Value::as_str)
        .is_some_and(|op| op.starts_with("reports_"))
    {
        return super::summary_reports::handle(state, tenant, actor, filters[0].clone()).await;
    }
    let request: Request = serde_json::from_value(filters[0].clone())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid Summary operation"))?;
    if request.summary_operation != "progress"
        && (request.state.is_some() || request.message.is_some())
    {
        return Err(denied());
    }
    if request.summary_operation == "capabilities" {
        return Ok(Json(
            json!({"summary_protocol":2,"summary_report_archive":1,"summary_hardening":1,"complete_directory":state.config.require_relay_membership}),
        ));
    }
    if request.summary_operation == "people" {
        if request.owner.is_some() || request.summary_id.is_some() || request.run_id.is_some() {
            return Err(denied());
        }
        let people = state
            .db
            .summary_people(tenant.community())
            .await
            .map_err(|_| denied())?;
        // A usable roster includes the requesting human. Agent identities cannot enumerate it here.
        if !people.contains(actor) {
            return Err(denied());
        }
        return Ok(Json(
            json!({"people":people,"complete":state.config.require_relay_membership}),
        ));
    }
    if request.source_report.is_some() && request.summary_operation != "dispatch" {
        return Err(denied());
    }
    let owner = request.owner.as_deref().unwrap_or(actor);
    if request.summary_operation == "overview" {
        if owner != actor
            || request.run_id.is_some()
            || request.summary_id.is_some()
            || request.before_time.is_some()
            || request.before_id.is_some()
        {
            return Err(denied());
        }
        let mut summaries = state
            .db
            .summary_overview(
                tenant.community(),
                &hex::decode(owner).map_err(|_| denied())?,
            )
            .await
            .map_err(|_| denied())?;
        for item in &mut summaries {
            if let Some(object) = item.as_object_mut() {
                let config = object.remove("config").and_then(|v| {
                    serde_json::from_value::<buzz_core::summary::SummaryDefinition>(v).ok()
                });
                let anchor = object
                    .remove("last_scheduled")
                    .and_then(|v| {
                        v.as_str()
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    })
                    .map(|v| v.with_timezone(&chrono::Utc));
                let next = config.as_ref().and_then(|config| {
                    buzz_workflow::next_summary_run(config, anchor, chrono::Utc::now())
                });
                object.insert("next_run_at".into(), json!(next));
            }
        }
        return Ok(Json(json!({"summaries":summaries})));
    }
    if request.summary_operation == "definitions" {
        if owner != actor
            || request.run_id.is_some()
            || request.summary_id.is_some()
            || request.before_time.is_some() != request.before_id.is_some()
        {
            return Err(denied());
        }
        let before = request
            .before_time
            .map(|t| chrono::DateTime::from_timestamp(t, 0).ok_or_else(denied))
            .transpose()?;
        let before_id = request
            .before_id
            .as_ref()
            .map(|id| {
                hex::decode(id)
                    .ok()
                    .filter(|b| b.len() == 32)
                    .ok_or_else(denied)
            })
            .transpose()?;
        let mut events = state
            .db
            .query_events_for_event_write(&buzz_db::EventQuery {
                pubkey: Some(hex::decode(owner).map_err(|_| denied())?),
                kinds: Some(vec![30623]),
                global_only: true,
                until: before,
                before_id,
                limit: Some(201),
                ..buzz_db::EventQuery::for_community(tenant.community())
            })
            .await
            .map_err(|_| denied())?
            .into_iter()
            .map(|e| e.event)
            .collect::<Vec<_>>();
        let more = events.len() > 200;
        events.truncate(200);
        let next = if more {
            events
                .last()
                .map(|e| json!({"before_time":e.created_at.as_secs(),"before_id":e.id.to_hex()}))
        } else {
            None
        };
        events.retain(|e| {
            crate::summary::single_tag(e, "d")
                .is_some_and(|d| d == "summary" || d.starts_with("summary:"))
        });
        return Ok(Json(json!({"events":events,"next":next})));
    }
    if request.before_time.is_some() || request.before_id.is_some() {
        return Err(denied());
    }
    let id = request.summary_id.as_deref().ok_or_else(denied)?;
    let (event, config) = crate::summary::definition(state, tenant.community(), owner, id)
        .await
        .map_err(|_| denied())?;
    if actor != owner && actor != config.reporter_pubkey {
        return Err(denied());
    }
    let owner_bytes = hex::decode(owner).map_err(|_| denied())?;
    match request.summary_operation.as_str() {
        "dispatch" => {
            if actor != owner
                || request.run_id.is_some()
                || config.archived
                || !config.read_delegation
            {
                return Err(denied());
            }
            let replay_period = if let Some(source) = request.source_report.as_deref() {
                if !buzz_core::summary::valid_pubkey(source) {
                    return Err(denied());
                }
                let original = state
                    .db
                    .get_event_by_id_for_event_write(
                        tenant.community(),
                        &hex::decode(source).map_err(|_| denied())?,
                    )
                    .await
                    .map_err(|_| denied())?
                    .ok_or_else(denied)?;
                if original.event.kind.as_u16() != 40030
                    || !crate::summary::reader_allowed(
                        state,
                        tenant.community(),
                        &original.event,
                        &owner_bytes,
                    )
                    .await
                {
                    return Err(denied());
                }
                let report: buzz_core::summary::SummaryReport =
                    serde_json::from_str(&original.event.content).map_err(|_| denied())?;
                if report.owner_pubkey != owner || report.summary_id != id {
                    return Err(denied());
                }
                Some(json!({"source_report":source,"since":report.since,"until":report.until}))
            } else {
                None
            };
            let reporter = hex::decode(&config.reporter_pubkey).map_err(|_| denied())?;
            let channel = state
                .db
                .create_dm(tenant.community(), &[&owner_bytes, &reporter], &owner_bytes)
                .await
                .map_err(|_| denied())?;
            let result = state
                .workflow_engine
                .dispatch_summary(tenant.community(), channel.id, owner, id, replay_period)
                .await
                .map_err(|e| api_error(StatusCode::CONFLICT, &e.to_string()))?;
            Ok(Json(result))
        }
        "start" => {
            if actor != owner
                || request.run_id.is_some()
                || config.archived
                || !config.read_delegation
            {
                return Err(denied());
            }
            let audience = crate::summary::prepare_attempt(
                state,
                tenant.community(),
                owner,
                &event.event,
                &config,
            )
            .await
            .map_err(|e| api_error(StatusCode::CONFLICT, &e))?;
            let (run, created) = state
                .db
                .claim_summary_run(
                    tenant.community(),
                    &owner_bytes,
                    id,
                    event.event.id.as_bytes(),
                    &hex::decode(&config.reporter_pubkey).map_err(|_| denied())?,
                    &audience,
                )
                .await
                .map_err(|_| denied())?;
            Ok(Json(json!({"run":run,"created":created})))
        }
        "progress" => {
            let run_id = request.run_id.ok_or_else(denied)?;
            let run = state
                .db
                .summary_run(tenant.community(), &owner_bytes, id, run_id)
                .await
                .map_err(|_| denied())?;
            let next = request.state.as_deref().ok_or_else(denied)?;
            if (actor != owner && !run.matches_definition(&config))
                || (actor != config.reporter_pubkey && !(actor == owner && next == "failed"))
            {
                return Err(denied());
            }
            state
                .db
                .update_summary_run_state(
                    tenant.community(),
                    &owner_bytes,
                    id,
                    run_id,
                    next,
                    request.message.as_deref(),
                )
                .await
                .map_err(|_| denied())?;
            let updated = state
                .db
                .summary_run(tenant.community(), &owner_bytes, id, run_id)
                .await
                .map_err(|_| denied())?;
            Ok(Json(json!({"run":updated})))
        }
        "history" => {
            if actor != owner || request.run_id.is_some() {
                return Err(denied());
            }
            let runs = state
                .db
                .summary_runs(tenant.community(), &owner_bytes, id)
                .await
                .map_err(|_| denied())?;
            Ok(Json(json!({"runs":runs})))
        }
        "status" => {
            let run = state
                .db
                .summary_run(
                    tenant.community(),
                    &owner_bytes,
                    id,
                    request.run_id.ok_or_else(denied)?,
                )
                .await
                .map_err(|_| denied())?;
            if actor != owner && run.reporter_pubkey != hex::decode(actor).map_err(|_| denied())? {
                return Err(denied());
            }
            let cursors = state
                .db
                .summary_cursors(tenant.community(), &owner_bytes, id)
                .await
                .map_err(|_| denied())?;
            let cursors = if run.replay_period.is_some() {
                Vec::new()
            } else {
                cursors
            };
            let observed = state
                .db
                .summary_observed_since(tenant.community(), &owner_bytes, id, run.run_id)
                .await
                .map_err(|_| denied())?;
            let frozen: buzz_core::summary::SummaryDefinition =
                serde_json::from_value(run.config_snapshot.clone()).map_err(|_| denied())?;
            let coverage_since = run.replay_since().unwrap_or_else(|| {
                frozen
                    .channels
                    .iter()
                    .chain(&frozen.dms)
                    .map(|source| {
                        cursors
                            .iter()
                            .find(|c| &c.channel_id == source)
                            .map(|c| c.created_at)
                            .unwrap_or(run.started_at.timestamp() - 86400)
                    })
                    .chain(observed)
                    .min()
                    .unwrap_or(run.started_at.timestamp())
            });
            Ok(Json(
                json!({"run":run,"config":config,"cursors":cursors,"coverage_since":coverage_since}),
            ))
        }
        "cursors" => {
            if request.run_id.is_some() {
                return Err(denied());
            }
            let cursors = state
                .db
                .summary_cursors(tenant.community(), &owner_bytes, id)
                .await
                .map_err(|_| denied())?;
            Ok(Json(json!({"cursors":cursors})))
        }
        _ => Err(api_error(
            StatusCode::BAD_REQUEST,
            "Unknown Summary operation",
        )),
    }
}
