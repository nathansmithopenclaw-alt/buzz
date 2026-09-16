//! Named Summary operations. The host constructs the authority-bearing envelope.
use crate::{client::BuzzClient, error::CliError, validate::read_file_or_stdin, BriefsRunCmd};
use buzz_core::summary::{
    valid_pubkey, valid_summary_id, SummaryCursor, SummaryDefinition, SummaryReport,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Composition {
    title: String,
    markdown: String,
    #[serde(default)]
    status: serde_json::Map<String, Value>,
    #[serde(default)]
    sources: Vec<Uuid>,
    #[serde(default)]
    watermarks: Vec<SummaryCursor>,
}

fn read_composition(input: &str) -> Result<Composition, CliError> {
    serde_json::from_str(&read_file_or_stdin(input)?)
        .map_err(|e| CliError::Usage(format!("Invalid Summary composition: {e}")))
}

fn observed_composition(
    mut composition: Composition,
    config: &SummaryDefinition,
    receipt: &Value,
) -> Result<Composition, CliError> {
    let sources: Vec<Uuid> = serde_json::from_value(receipt["coverage"]["sources"].clone())
        .map_err(|_| {
            CliError::Other(
                "Relay does not supply observed Summary coverage; update the relay".into(),
            )
        })?;
    let configured: std::collections::BTreeSet<_> =
        config.channels.iter().chain(&config.dms).copied().collect();
    if sources
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        != configured
    {
        return Err(CliError::Usage(
            "Read every configured conversation before publishing, including empty sources".into(),
        ));
    }
    let watermarks: Vec<SummaryCursor> =
        serde_json::from_value(receipt["coverage"]["watermarks"].clone())
            .map_err(|e| CliError::Other(e.to_string()))?;
    let messages = receipt["coverage"]["messages_observed"]
        .as_u64()
        .ok_or_else(|| CliError::Other("Missing observed message count".into()))?;
    composition.sources = configured.into_iter().collect();
    composition
        .status
        .insert("conversations_read".into(), json!(sources.len()));
    composition.status.insert(
        "conversations_with_messages".into(),
        json!(watermarks.len()),
    );
    composition.status.insert(
        "conversations_without_messages".into(),
        json!(sources.len().saturating_sub(watermarks.len())),
    );
    composition
        .status
        .insert("messages_observed".into(), json!(messages));
    composition
        .status
        .insert("sources".into(), json!(composition.sources));
    composition
        .status
        .insert("run_id".into(), receipt["run"]["run_id"].clone());
    composition.watermarks = watermarks;
    Ok(composition)
}

async fn status(
    client: &BuzzClient,
    owner: &str,
    summary: &str,
    run: &str,
) -> Result<Value, CliError> {
    if !valid_pubkey(owner) || !valid_summary_id(summary) || Uuid::parse_str(run).is_err() {
        return Err(CliError::Usage("Invalid Summary identity".into()));
    }
    serde_json::from_str(&client.query(&json!({"summary_operation":"status","owner":owner,"summary_id":summary,"run_id":run})).await?)
        .map_err(|e|CliError::Other(e.to_string()))
}

pub async fn dispatch(cmd: BriefsRunCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        BriefsRunCmd::Start { summary } => {
            if !valid_summary_id(&summary) {
                return Err(CliError::Usage("Invalid Summary identity".into()));
            }
            println!(
                "{}",
                client
                    .query(&json!({"summary_operation":"start","summary_id":summary}))
                    .await?
            );
        }
        BriefsRunCmd::Status {
            owner,
            summary,
            run,
        } => println!("{}", status(client, &owner, &summary, &run).await?),
        BriefsRunCmd::Progress {
            owner,
            summary,
            run,
            state,
            message,
        } => {
            status(client, &owner, &summary, &run).await?;
            println!("{}", client.query(&json!({"summary_operation":"progress","owner":owner,"summary_id":summary,"run_id":run,"state":state,"message":message})).await?);
        }
        BriefsRunCmd::Publish {
            owner,
            summary,
            run,
            input,
        } => {
            let receipt = status(client, &owner, &summary, &run).await?;
            if receipt["run"]["state"] == "published" {
                println!(
                    "{}",
                    json!({"published":true,"existing":true,"run":receipt["run"]})
                );
                return Ok(());
            }
            let composition = read_composition(&input)?;
            let config: SummaryDefinition = serde_json::from_value(receipt["config"].clone())
                .map_err(|e| CliError::Other(e.to_string()))?;
            if config.reporter_pubkey != client.keys().public_key().to_hex() {
                return Err(CliError::Usage(
                    "Only the assigned Summary agent may publish".into(),
                ));
            }
            let composition = observed_composition(composition, &config, &receipt)?;
            let normal_until = receipt["run"]["started_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or_else(|| CliError::Other("Invalid Summary run boundary".into()))?
                .timestamp();
            let until = receipt["run"]["replay_period"]["until"]
                .as_i64()
                .unwrap_or(normal_until);
            let cursors: Vec<SummaryCursor> = serde_json::from_value(receipt["cursors"].clone())
                .map_err(|e| CliError::Other(e.to_string()))?;
            let since = cursors
                .iter()
                .filter(|c| composition.sources.contains(&c.channel_id))
                .map(|c| c.created_at)
                .chain(composition.watermarks.iter().map(|c| c.created_at))
                .min()
                .unwrap_or(until);
            let since = receipt["coverage_since"]
                .as_i64()
                .or_else(|| receipt["run"]["replay_period"]["since"].as_i64())
                .unwrap_or(since);
            let report = SummaryReport {
                v: 2,
                owner_pubkey: owner.clone(),
                summary_id: summary.clone(),
                run_id: Uuid::parse_str(&run).map_err(|e| CliError::Usage(e.to_string()))?,
                config_event_id: receipt["run"]["config_event_id"]
                    .as_str()
                    .ok_or_else(|| CliError::Other("Invalid Summary revision".into()))?
                    .into(),
                title: composition.title,
                markdown: composition.markdown,
                status: Value::Object(composition.status),
                sources: composition.sources,
                since,
                until,
                watermarks: composition.watermarks,
            };
            let audience = serde_json::from_value(receipt["run"]["audience"].clone())
                .map_err(|e| CliError::Other(e.to_string()))?;
            let event = client.sign_event(
                buzz_sdk::build_summary_report(&report, &config.reporter_pubkey, &audience)
                    .map_err(crate::validate::sdk_err)?,
            )?;
            let submission = client.submit_event(event.clone()).await;
            // Reconcile the durable receipt even when the HTTP acknowledgement was lost.
            let confirmed = status(client, &owner, &summary, &run).await;
            match confirmed {
                Ok(value) if value["run"]["state"]=="published" && value["run"]["report_event_id"]==event.id.to_hex()=>
                    println!("{}",json!({"published":true,"event_id":event.id.to_hex(),"run_id":run,"kind":40030})),
                _=>return Err(CliError::Other(format!("Summary publication not confirmed for run {run}, event {}. Inspect run status before retrying. Submission: {}",event.id,
                    submission.map(|_|"acknowledged".to_owned()).unwrap_or_else(|e|e.to_string())))),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_input_reads_json_file_and_rejects_authority_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("composition.json");
        let mut body = json!({"title":"Summary", "markdown":"No new activity.",
            "status":{}, "sources":[], "watermarks":[]});
        std::fs::write(&path, body.to_string()).unwrap();
        assert_eq!(
            read_composition(path.to_str().unwrap()).unwrap().title,
            "Summary"
        );
        body["audience"] = json!(["unauthorized"]);
        std::fs::write(&path, body.to_string()).unwrap();
        assert!(read_composition(path.to_str().unwrap()).is_err());
    }
    #[test]
    fn publication_metadata_uses_observed_sources_and_unique_counts() {
        let full = Uuid::new_v4();
        let empty = Uuid::new_v4();
        let config: SummaryDefinition = serde_json::from_value(json!({"v":2,"summary_id":"default","name":"Daily","reporter_pubkey":"a".repeat(64),"audience":{"mode":"only_me"},"read_delegation":true,"channels":[full,empty],"dms":[],"schedule":null,"timezone":"UTC"})).unwrap();
        let composition = Composition {
            title: "Daily".into(),
            markdown: "Observed work.".into(),
            status: serde_json::from_value(json!({"messages_observed":12})).unwrap(),
            sources: vec![full],
            watermarks: vec![],
        };
        let receipt = json!({"run":{"run_id":Uuid::new_v4()},"coverage":{"sources":[full,empty],"messages_observed":11,"watermarks":[{"channel_id":full,"event_id":"b".repeat(64),"created_at":100}]}});
        let result = observed_composition(composition, &config, &receipt).unwrap();
        assert_eq!(result.sources.len(), 2);
        assert!(result.sources.contains(&empty));
        assert_eq!(result.status["messages_observed"], 11);
        assert_eq!(result.status["conversations_without_messages"], 1);
        assert_eq!(result.watermarks.len(), 1);
        let mut incomplete = receipt.clone();
        incomplete["coverage"]["sources"] = json!([full]);
        assert!(observed_composition(result, &config, &incomplete).is_err());
    }
}
