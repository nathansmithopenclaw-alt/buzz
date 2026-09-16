//! Explicit, exact-manifest legacy authorization baseline. No automatic startup migration.
use anyhow::Result;
use buzz_sdk::dispatch_admission::{build_dispatch_admission, AdmissionBasis, DispatchAdmission};
use nostr::Keys;
use uuid::Uuid;
pub async fn run(workspace: Uuid, apply_digest: Option<String>) -> Result<i32> {
    let db = super::connect_db().await?;
    let tenant = super::resolve_admin_tenant(&db).await?;
    let raw = std::env::var("BUZZ_RELAY_PRIVATE_KEY").map_err(|_| {
        anyhow::anyhow!("BUZZ_RELAY_PRIVATE_KEY is required; no ephemeral signer is permitted")
    })?;
    let keys = Keys::parse(&raw)?;
    let result = db
        .baseline_dispatch_admissions(
            tenant.community(),
            workspace,
            &keys.public_key().to_bytes(),
            apply_digest.as_deref(),
            Some(buzz_sdk::dispatch_admission::requires_dispatch_event_admission),
            |source, role, observed_at| {
                let role = match role {
                    Some("owner" | "admin" | "bot") => "maintainer",
                    Some(_) => "member",
                    None => "none",
                };
                build_dispatch_admission(&DispatchAdmission {
                    v: 1,
                    workspace,
                    event_id: source.id.to_hex(),
                    author: source.pubkey.to_hex(),
                    role: role.into(),
                    observed_at,
                    basis: AdmissionBasis::LegacyBaseline,
                })?
                .sign_with_keys(&keys)
                .map_err(|error| error.to_string())
            },
        )
        .await?;
    println!(
        "{}",
        serde_json::json!({"workspace":workspace,"mode":if apply_digest.is_some(){"applied"}else{"preview"},"digest":result.digest,"events":result.count,"manifest":result.manifest,"receipts":result.receipts.len(),"basis":"legacy_baseline","meaning":"Roles observed now. This does not reconstruct original historical permissions.","refresh":"After applying, refresh Dispatch history on each client."})
    );
    Ok(0)
}
