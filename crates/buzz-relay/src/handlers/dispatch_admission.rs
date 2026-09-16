//! Relay role evidence captured in the original Dispatch event transaction.
use super::ingest::IngestError;
use crate::state::AppState;
use buzz_core::{CommunityId, StoredEvent};
use nostr::Event;
use uuid::Uuid;
pub(super) async fn insert(
    state: &AppState,
    community: CommunityId,
    event: &Event,
    workspace: Uuid,
    thread_meta: Option<buzz_db::event::ThreadMetadataParams<'_>>,
) -> Result<(StoredEvent, bool, Option<StoredEvent>), IngestError> {
    let capture_id = if event.kind.as_u16() == 40020 {
        serde_json::from_str::<buzz_sdk::tracker::IssueRoot>(&event.content)
            .ok()
            .and_then(|root| {
                let source = root.source_event_id.as_deref()?;
                let request = buzz_sdk::tracker::pin_capture_request_id(source);
                (root.import.is_none()
                    && root.issue_id == request
                    && root.request_id == Some(request))
                .then_some(request)
            })
    } else {
        None
    };
    state
        .db
        .insert_dispatch_with_admission(
            community,
            event,
            workspace,
            thread_meta,
            capture_id,
            |role, observed_at| {
                use buzz_sdk::dispatch_admission::{
                    build_dispatch_admission, AdmissionBasis, DispatchAdmission,
                };
                let role = if event.pubkey == state.relay_keypair.public_key() {
                    "maintainer"
                } else {
                    match role {
                        Some("owner" | "admin" | "bot") => "maintainer",
                        Some(_) => "member",
                        None => {
                            return Err("Dispatch author is no longer a workspace member".into())
                        }
                    }
                };
                build_dispatch_admission(&DispatchAdmission {
                    v: 1,
                    workspace,
                    event_id: event.id.to_hex(),
                    author: event.pubkey.to_hex(),
                    role: role.into(),
                    observed_at,
                    basis: AdmissionBasis::Admission,
                })?
                .sign_with_keys(&state.relay_keypair)
                .map_err(|error| error.to_string())
            },
        )
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: Dispatch admission failed: {error}"))
        })
}
