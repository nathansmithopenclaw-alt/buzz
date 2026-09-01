//! `buzz memory` — thin client for the authenticated shared-memory read plane.

use serde_json::{json, Value};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{MemoryCmd, MemoryContextArgs};

fn context_value(context: &MemoryContextArgs) -> Value {
    if let Some(id) = context.channel {
        json!({ "kind": "channel", "id": id })
    } else if let Some(id) = context.dm {
        json!({ "kind": "dm", "id": id })
    } else {
        debug_assert!(context.workspace_public);
        json!({ "kind": "workspace_public" })
    }
}

async fn query(
    client: &BuzzClient,
    context: &MemoryContextArgs,
    mode: &str,
    slot: Option<String>,
    query: Option<String>,
    limit: usize,
) -> Result<(), CliError> {
    if !(1..=100).contains(&limit) {
        return Err(CliError::Usage("--limit must be in 1..=100".into()));
    }
    let body = json!({
        "context": context_value(context),
        "mode": mode,
        "slot": slot,
        "query": query,
        "limit": limit,
    });
    let raw = client.post_authed_json("/memory/v1/query", &body).await?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("relay returned invalid memory JSON: {error}")))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| CliError::Other(format!(
            "memory output serialization failed: {error}"
        )))?
    );
    Ok(())
}

pub async fn dispatch(command: MemoryCmd, client: &BuzzClient) -> Result<(), CliError> {
    match command {
        MemoryCmd::Status { context } => {
            let body = json!({ "context": context_value(&context) });
            let raw = client.post_authed_json("/memory/v1/status", &body).await?;
            let value: Value = serde_json::from_str(&raw).map_err(|error| {
                CliError::Other(format!(
                    "relay returned invalid memory status JSON: {error}"
                ))
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(|error| {
                    CliError::Other(format!("memory output serialization failed: {error}"))
                })?
            );
            Ok(())
        }
        MemoryCmd::Current {
            context,
            slot,
            query: text,
            limit,
        } => query(client, &context, "current", slot, text, limit).await,
        MemoryCmd::Why {
            context,
            slot,
            query: text,
            limit,
        } => query(client, &context, "why", Some(slot), text, limit).await,
        MemoryCmd::History {
            context,
            slot,
            query: text,
            limit,
        } => query(client, &context, "history", slot, text, limit).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_wire_shape_has_exactly_one_realm() {
        let channel = uuid::Uuid::from_u128(7);
        assert_eq!(
            context_value(&MemoryContextArgs {
                channel: Some(channel),
                dm: None,
                workspace_public: false,
            }),
            json!({ "kind": "channel", "id": channel })
        );
    }
}
