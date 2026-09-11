use super::*;

pub(super) async fn task_for_dispatch_request(
    client: &BuzzClient,
    request_id: Uuid,
) -> Result<Option<TaskEnvelopeEvent>, CliError> {
    let history = query_task_events(client, None).await?;
    let has_request = history
        .iter()
        .any(|event| event.envelope.dispatch_request_id == Some(request_id));
    let current = fold_task_events(history.clone());
    let mut matches = current
        .into_values()
        .filter(|event| event.envelope.dispatch_request_id == Some(request_id));
    let found = matches.next();
    if matches.next().is_some() {
        return Err(CliError::Conflict(format!(
            "dispatch request {request_id} names multiple tasks"
        )));
    }
    let Some(found) = found else {
        if has_request {
            return Err(CliError::Conflict("dispatch request has incomplete or conflicting assignment history; reconcile it before retrying".into()));
        }
        return Ok(None);
    };
    // Idempotency compares the immutable assignment, not a later transition
    // whose deliverables and evidence legitimately changed during execution.
    Ok(history
        .into_iter()
        .filter(|event| {
            event.envelope.task_id == found.envelope.task_id
                && event.envelope.status == TaskStatus::Assigned
                && event.author_pubkey == found.envelope.assigner
        })
        .min_by(|a, b| (a.created_at, &a.event_id).cmp(&(b.created_at, &b.event_id))))
}

pub(super) fn report_pending(request: Uuid, issue: Uuid, task: Uuid, link: &str) {
    println!(
        "{}",
        json!({"accepted":false,"partial":true,"operation":"track.dispatch",
        "request_id":request,"issue_id":issue,"task_id":task,"issue_link_event_id":link,
        "pending":["confirm_issue_link","task_assignment"],"retryable":true,
        "message":"assignment retained; retry the same --request-id and input to check accepted history and resume the exact signed events"})
    );
}

pub(super) async fn publish_retained_task(
    client: &BuzzClient,
    workspace_id: Uuid,
    issue_id: Uuid,
    request_id: Uuid,
    link_event_id: &str,
    task_event: Event,
    reviewed_tip: Option<&str>,
) -> Result<(), CliError> {
    let task = TaskEnvelopeEvent::from_event(&task_event)
        .map_err(|error| usage("invalid retained assignment", error))?;
    let signer = client.keys().public_key().to_hex();
    let task_event_id = task_event.id.to_hex();
    // A receipt alone is insufficient: the link must be the current accepted
    // issue state before the executor can observe a task message.
    if !client
        .query_all(json!({"ids":[link_event_id],"kinds":[40021],"#h":[workspace_id.to_string()]}))
        .await?
        .into_iter()
        .any(|event| event["id"] == link_event_id && event["pubkey"] == signer)
    {
        return Err(CliError::Conflict(
            "assignment link was not read back from the relay; retained task was not published"
                .into(),
        ));
    }
    let (fresh, fresh_authorization) = load_context(client, workspace_id).await?;
    let current = fresh.issue(&issue_id.to_string())?;
    if fresh_authorization.role(&signer).is_none()
        || (current.tip_event != link_event_id && reviewed_tip != Some(current.tip_event.as_str()))
        || current.status.is_terminal()
        || !current
            .external_refs
            .iter()
            .any(|value| value.kind == "task_event" && value.reference == task_event_id)
    {
        return Err(CliError::Conflict("issue changed after its assignment link; inspect the latest issue and retry the same request with --if-revision set to its reviewed current event; retained task was not published".into()));
    }
    // Recheck the original destination immediately before replaying the
    // exact signed event; a recovered write never borrows new authority.
    resolve_issue_assignee(client, task.channel_id, &task.envelope.assignee).await?;
    if let Some(reviewer) = task
        .envelope
        .authority
        .as_ref()
        .and_then(|a| a.reviewer.as_ref())
    {
        resolve_issue_assignee(client, task.channel_id, reviewer).await?;
    }
    let raw = client.submit_event(task_event).await?;
    let receipt: Value = serde_json::from_str(&normalize_write_response(&raw))
        .map_err(|error| usage("invalid task receipt", error))?;
    if receipt["accepted"] != true {
        return Err(CliError::Other(format!(
            "relay rejected assignment: {}",
            receipt["message"]
        )));
    }
    if task_for_dispatch_request(client, request_id)
        .await?
        .is_none_or(|event| event.event_id != task_event_id)
    {
        return Err(CliError::Other("assignment acknowledgement has no matching accepted task history; retry the retained request".into()));
    }
    Ok::<_, CliError>(())
}
