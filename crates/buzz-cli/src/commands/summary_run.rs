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
    sources: Vec<Uuid>,
    watermarks: Vec<SummaryCursor>,
}

fn read_composition(input: &str) -> Result<Composition, CliError> {
    serde_json::from_str(&read_file_or_stdin(input)?)
        .map_err(|e| CliError::Usage(format!("Invalid Summary composition: {e}")))
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
            let until = receipt["run"]["started_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or_else(|| CliError::Other("Invalid Summary run boundary".into()))?
                .timestamp();
            let cursors: Vec<SummaryCursor> = serde_json::from_value(receipt["cursors"].clone())
                .map_err(|e| CliError::Other(e.to_string()))?;
            let since = cursors
                .iter()
                .filter(|c| composition.sources.contains(&c.channel_id))
                .map(|c| c.created_at)
                .chain(composition.watermarks.iter().map(|c| c.created_at))
                .min()
                .unwrap_or(until);
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
}
