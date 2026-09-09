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
    let request: Request = serde_json::from_value(filters[0].clone())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid Summary operation"))?;
    if request.summary_operation != "progress"
        && (request.state.is_some() || request.message.is_some())
    {
        return Err(denied());
    }
    if request.summary_operation == "capabilities" {
        return Ok(Json(
            json!({"summary_protocol":2,"complete_directory":state.config.require_relay_membership}),
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
    let owner = request.owner.as_deref().unwrap_or(actor);
    let id = request.summary_id.as_deref().ok_or_else(denied)?;
    let (event, config) = crate::summary::definition(state, tenant.community(), owner, id)
        .await
        .map_err(|_| denied())?;
    if actor != owner && actor != config.reporter_pubkey {
        return Err(denied());
    }
    let owner_bytes = hex::decode(owner).map_err(|_| denied())?;
    match request.summary_operation.as_str() {
        "start" => {
            if actor != owner
                || request.run_id.is_some()
                || config.archived
                || !config.read_delegation
            {
                return Err(denied());
            }
            let audience = crate::summary::audience(state, tenant.community(), owner, &config)
                .await
                .map_err(|_| denied())?;
            crate::summary::source_access(state, tenant.community(), owner, &config, &audience)
                .await
                .map_err(|_| denied())?;
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
            if run.config_event_id != event.event.id.as_bytes().as_slice()
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
            Ok(Json(json!({"run":run,"config":config,"cursors":cursors})))
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
