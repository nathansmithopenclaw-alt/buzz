//! Nostr command for bounded SDK projections and unsigned mobile edits.
//! This handler is read-only: no event, proposal or execution is published.
use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;
use buzz_core::tenant::TenantContext;
use buzz_sdk::{
    dispatch_client::{dispatch_client_response, DispatchClientRequest},
    tracker::{TrackerAuthorization, TrackerRole},
};
use nostr::{Event, Timestamp};
use uuid::Uuid;

pub(super) async fn handle(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    auth: &IngestAuth,
) -> Result<IngestResult, IngestError> {
    let denied = || {
        IngestError::AuthFailed("restricted: current Dispatch workspace access is required".into())
    };
    let invalid = |message: &str| IngestError::Rejected(format!("invalid: {message}"));
    if event.content.len() > 16384 {
        return Err(invalid("Dispatch client request exceeds 16 KiB"));
    }
    let channels = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
        .collect::<Vec<_>>();
    if channels.len() != 1 || channels[0].as_slice().len() != 2 {
        return Err(invalid("one exact workspace tag is required"));
    }
    let workspace = channels[0].as_slice()[1]
        .parse::<Uuid>()
        .map_err(|_| invalid("invalid workspace UUID"))?;
    if auth
        .channel_ids()
        .is_some_and(|ids| !ids.contains(&workspace))
    {
        return Err(denied());
    }
    let actor = auth.pubkey();
    let role = state
        .db
        .get_member_role(tenant.community(), workspace, &actor.to_bytes())
        .await
        .map_err(|_| denied())?;
    if role.is_none() {
        return Err(denied());
    }
    let request: DispatchClientRequest = serde_json::from_str(&event.content)
        .map_err(|_| invalid("malformed Dispatch client request"))?;
    let members = state
        .db
        .get_members_for_event_write(tenant.community(), workspace)
        .await
        .map_err(|_| denied())?;
    let mut roles = vec![(
        state.relay_keypair.public_key().to_hex(),
        TrackerRole::Maintainer,
    )];
    roles.extend(members.into_iter().map(|member| {
        (
            hex::encode(member.pubkey),
            if matches!(member.role.as_str(), "owner" | "admin" | "bot") {
                TrackerRole::Maintainer
            } else {
                TrackerRole::Member
            },
        )
    }));
    let events = state
        .db
        .dispatch_client_history(tenant.community(), workspace)
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(message) => invalid(&message),
            other => IngestError::Internal(format!("Dispatch history read failed: {other}")),
        })?;
    let events = events
        .into_iter()
        .map(|value| value.event)
        .collect::<Vec<_>>();
    let response = dispatch_client_response(
        &events,
        workspace,
        state.relay_keypair.public_key(),
        *actor,
        TrackerAuthorization::new(roles),
        request,
        Timestamp::now().as_secs(),
    )
    .map_err(|message| invalid(&message))?;
    // Role/access changes during the read require a new snapshot rather than a
    // response that grants current controls using an earlier member list.
    let latest = state
        .db
        .get_member_role(tenant.community(), workspace, &actor.to_bytes())
        .await
        .map_err(|_| denied())?;
    if latest != role {
        return Err(denied());
    }
    let encoded = serde_json::to_string(&response)
        .map_err(|error| IngestError::Internal(error.to_string()))?;
    if encoded.len() > 1024 * 1024 {
        return Err(invalid(
            "Dispatch response is too large; open a specific issue",
        ));
    }
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: format!("response:{encoded}"),
    })
}

#[cfg(test)]
mod postgres_tests;
