//! Authenticated report archive operations. Archive state never grants read access.
use super::api_error;
use crate::AppState;
use axum::{http::StatusCode, Json};
use buzz_core::tenant::TenantContext;
use buzz_db::summary_archive::SummaryReportFilter;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    summary_operation: String,
    filter: Option<SummaryReportFilter>,
    event_ids: Option<Vec<String>>,
    metadata_only: Option<bool>,
}

pub(super) async fn handle(
    state: &AppState,
    tenant: &TenantContext,
    actor: &str,
    value: Value,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let denied = || api_error(StatusCode::FORBIDDEN, "Report operation is not permitted");
    let bad = || api_error(StatusCode::BAD_REQUEST, "Invalid report operation");
    let request: Request = serde_json::from_value(value).map_err(|_| bad())?;
    let reader = hex::decode(actor).map_err(|_| denied())?;
    if request.summary_operation == "reports_list" {
        if request.event_ids.is_some() {
            return Err(bad());
        }
        let mut filter = request.filter.unwrap_or_default();
        if filter
            .query
            .as_ref()
            .is_some_and(|q| q.chars().count() > 200)
            || matches!((filter.since, filter.until), (Some(s), Some(u)) if s > u)
            || filter.before_time.is_some() != filter.before_id.is_some()
            || filter
                .before_id
                .as_ref()
                .is_some_and(|id| !buzz_core::summary::valid_pubkey(id))
            || [filter.since, filter.until, filter.before_time]
                .iter()
                .flatten()
                .any(|v| *v < 0 || *v > 253402300799)
        {
            return Err(bad());
        }
        let mut events = Vec::new();
        let mut exhausted = false;
        // Do not expose cursor IDs belonging to withdrawn reports. Walk candidates
        // internally; the public cursor always belongs to an authorized result.
        for _ in 0..20 {
            let candidates = state
                .db
                .summary_report_page(tenant.community(), &reader, &filter)
                .await
                .map_err(|_| denied())?;
            exhausted = candidates.len() < 51;
            for event in candidates {
                filter.before_time = Some(event.event.created_at.as_secs() as i64);
                filter.before_id = Some(event.event.id.to_hex());
                if buzz_core::filter::reader_authorized_for_event(&event.event, actor)
                    && crate::summary::reader_allowed(
                        state,
                        tenant.community(),
                        &event.event,
                        &reader,
                    )
                    .await
                {
                    events.push(event.event);
                }
                if events.len() == 51 {
                    break;
                }
            }
            if exhausted || events.len() == 51 {
                break;
            }
        }
        if !exhausted && events.len() < 51 {
            return Err(api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Narrow the report filters to continue browsing",
            ));
        }
        let has_more = events.len() > 50;
        events.truncate(50);
        let next = if has_more {
            events
                .last()
                .map(|e| json!({"before_time":e.created_at.as_secs(),"before_id":e.id.to_hex()}))
        } else {
            None
        };
        if request.metadata_only == Some(true) {
            let reports = report_metadata(state, tenant, actor, &events).await?;
            return Ok(Json(json!({"reports":reports,"next":next})));
        }
        return Ok(Json(json!({"events":events,"next":next})));
    }
    if request.filter.is_some() || request.metadata_only.is_some() {
        return Err(bad());
    }
    let deleting = request.summary_operation == "reports_delete";
    let archived = match request.summary_operation.as_str() {
        "reports_archive" => true,
        "reports_restore" | "reports_delete" | "reports_get" => false,
        _ => return Err(bad()),
    };
    let ids = request.event_ids.ok_or_else(bad)?;
    if ids.is_empty() || ids.len() > 50 {
        return Err(bad());
    }
    if request.summary_operation == "reports_get" && ids.len() != 1 {
        return Err(bad());
    }
    // Validate the entire selection before performing any mutation.
    let mut verified = Vec::new();
    for id in ids {
        if !buzz_core::summary::valid_pubkey(&id) {
            return Err(bad());
        }
        let bytes = hex::decode(id).map_err(|_| bad())?;
        let event = state
            .db
            .get_event_by_id_for_event_write(tenant.community(), &bytes)
            .await
            .map_err(|_| denied())?
            .ok_or_else(denied)?;
        if event.event.kind.as_u16() != 40030
            || !buzz_core::filter::reader_authorized_for_event(&event.event, actor)
            || !crate::summary::reader_allowed(state, tenant.community(), &event.event, &reader)
                .await
        {
            return Err(denied());
        }
        if deleting {
            let owner = crate::summary::single_tag(&event.event, "owner");
            let owns = if let Some(owner) = owner {
                owner == actor
            } else {
                !crate::summary::is_named_report(&event.event)
                    && state
                        .db
                        .is_agent_owner(tenant.community(), &event.event.pubkey.to_bytes(), &reader)
                        .await
                        .map_err(|_| denied())?
            };
            if !owns {
                return Err(denied());
            }
        }
        if request.summary_operation == "reports_get" {
            let metadata =
                report_metadata(state, tenant, actor, std::slice::from_ref(&event.event)).await?;
            return Ok(Json(json!({"event":event.event,"report":metadata[0]})));
        }
        verified.push(bytes);
    }
    state
        .db
        .update_summary_reports(tenant.community(), &reader, &verified, archived, deleting)
        .await
        .map_err(|_| denied())?;
    Ok(Json(json!({"updated":verified.len()})))
}

async fn report_metadata(
    state: &AppState,
    tenant: &TenantContext,
    actor: &str,
    events: &[buzz_core::Event],
) -> Result<Vec<Value>, (StatusCode, Json<Value>)> {
    let denied = || api_error(StatusCode::FORBIDDEN, "Report operation is not permitted");
    let reader = hex::decode(actor).map_err(|_| denied())?;
    let mut reports = Vec::new();
    let mut names = std::collections::BTreeMap::new();
    let mut summary_names = std::collections::BTreeMap::new();
    for event in events {
        let owner = crate::summary::single_tag(event, "owner").unwrap_or(actor);
        let id = crate::summary::single_tag(event, "summary").unwrap_or("default");
        let pair = (owner.to_owned(), id.to_owned());
        if !summary_names.contains_key(&pair) {
            let name = crate::summary::definition(state, tenant.community(), owner, id)
                .await
                .map(|(_, c)| c.name)
                .unwrap_or_else(|_| "Summary".into());
            summary_names.insert(pair.clone(), name);
        }
        let reporter = event.pubkey.to_hex();
        for key in [owner, &reporter] {
            if !names.contains_key(key) {
                let bytes = hex::decode(key).map_err(|_| denied())?;
                let name = state
                    .db
                    .get_user(tenant.community(), &bytes)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|p| p.display_name)
                    .unwrap_or_else(|| format!("{}…", &key[..12]));
                names.insert(key.to_owned(), name);
            }
        }
        let content: Value = serde_json::from_str(&event.content).unwrap_or_default();
        let can_delete = if crate::summary::single_tag(event, "owner").is_some() {
            owner == actor
        } else {
            state
                .db
                .is_agent_owner(tenant.community(), &event.pubkey.to_bytes(), &reader)
                .await
                .unwrap_or(false)
        };
        reports.push(json!({"id":event.id.to_hex(),"pubkey":reporter,"created_at":event.created_at.as_secs(),
            "title":content["title"],"owner":owner,"summary_id":id,"summary_name":summary_names[&pair],
            "agent_name":names[&reporter],"owner_name":names[owner],"can_delete":can_delete,
            "since":content.get("since").or_else(||content.get("period").and_then(|p|p.get("since"))),
            "until":content.get("until").or_else(||content.get("period").and_then(|p|p.get("until")))}));
    }
    Ok(reports)
}
