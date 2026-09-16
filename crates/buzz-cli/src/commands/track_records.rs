use super::*;
use buzz_sdk::dispatch_record::{build_dispatch_record, fold_dispatch_records, DispatchRecord};
use std::io::Read;

pub(super) async fn dispatch(
    client: &BuzzClient,
    command: crate::TrackRecordCmd,
) -> Result<(), CliError> {
    let (workspace, input) = match command {
        crate::TrackRecordCmd::List { workspace } => (workspace, None),
        crate::TrackRecordCmd::Put { workspace, input } => (workspace, Some(input)),
    };
    let workspace = parse_workspace(workspace)?;
    let (context, authorization) = load_context(client, workspace).await?;
    let now = Utc::now().timestamp().max(0) as u64;
    let Some(input) = input else {
        println!(
            "{}",
            serde_json::to_string(&fold_dispatch_records(
                context.cache.events,
                workspace,
                &authorization,
                now
            ))
            .map_err(|e| CliError::Other(e.to_string()))?
        );
        return Ok(());
    };
    let mut raw = String::new();
    let source: Box<dyn Read> = if input == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(fs::File::open(input).map_err(|e| CliError::Usage(e.to_string()))?)
    };
    source
        .take(65537)
        .read_to_string(&mut raw)
        .map_err(|e| CliError::Usage(e.to_string()))?;
    if raw.len() > 65536 {
        return Err(CliError::Usage("planning record exceeds 64 KiB".into()));
    }
    let record: DispatchRecord =
        serde_json::from_str(&raw).map_err(|e| CliError::Usage(e.to_string()))?;
    let candidate =
        client.sign_event(build_dispatch_record(workspace, &record).map_err(CliError::Usage)?)?;
    if let Some(existing) = existing_request_event(client, workspace, record.request_id).await? {
        if !same_semantic_request(&existing, &candidate) {
            return Err(CliError::Conflict(
                "request ID already names different planning input".into(),
            ));
        }
        let fold = fold_dispatch_records(context.cache.events, workspace, &authorization, now);
        let accepted = fold
            .records
            .get(&record.record_id)
            .is_some_and(|value| value.accepted_events.contains(&existing.id.to_hex()));
        if !accepted {
            return Err(CliError::Conflict("the prior request is preserved but is no longer the accepted record tip; inspect current records".into()));
        }
        println!(
            "{}",
            json!({"record":fold.records.get(&record.record_id),"replayed":true})
        );
        return Ok(());
    }
    let mut events = context.cache.events;
    events.push(candidate.clone());
    let preview = fold_dispatch_records(
        events,
        workspace,
        &authorization,
        now.max(candidate.created_at.as_secs()),
    );
    if !preview
        .records
        .get(&record.record_id)
        .is_some_and(|value| value.accepted_events.contains(&candidate.id.to_hex()))
    {
        return Err(CliError::Conflict(format!(
            "planning record rejected: {}",
            preview.diagnostics.join("; ")
        )));
    }
    submit_idempotent(client, workspace, record.request_id, candidate.clone()).await?;
    let (readback, authorization) = load_context(client, workspace).await?;
    let folded = fold_dispatch_records(
        readback.cache.events,
        workspace,
        &authorization,
        Utc::now().timestamp().max(0) as u64,
    );
    if !folded
        .records
        .get(&record.record_id)
        .is_some_and(|value| value.accepted_events.contains(&candidate.id.to_hex()))
    {
        return Err(CliError::Conflict("record was published but its current state was not confirmed; retry the same request ID".into()));
    }
    println!(
        "{}",
        json!({"record":folded.records.get(&record.record_id),"replayed":false})
    );
    Ok(())
}
