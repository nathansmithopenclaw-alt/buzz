use super::*;
use buzz_sdk::task_envelope::{TaskEnvelopeEvent, TaskStatus};
/// The source lifecycle event is already durable. Failure propagates before ACK;
/// an identical retry repeats this idempotent projection before the duplicate ACK.
pub(crate) async fn transition(
    state: &Arc<AppState>,
    community: CommunityId,
    event: &Event,
) -> Result<()> {
    let Ok(task) = TaskEnvelopeEvent::from_event(event) else {
        return Ok(());
    };
    if !matches!(
        task.envelope.status,
        TaskStatus::PickedUp | TaskStatus::Done
    ) {
        return Ok(());
    }
    let Some(issue) = task.envelope.tracker_issue_id else {
        return Ok(());
    };
    for workspace in state
        .db
        .dispatch_issue_workspaces(community, issue)
        .await
        .map_err(err)?
    {
        let channels = discover_scopes(state, community, workspace).await?;
        let (events, _) = state
            .db
            .dispatch_workflow_transaction(community, &channels, &[], None, |snapshot| {
                let mut p = Planner::new(state, snapshot);
                let tasks = p.tasks();
                let Some(current) = tasks
                    .current
                    .get(&task.envelope.task_id)
                    .filter(|t| t.event_id == event.id.to_hex())
                else {
                    return Ok(p.batch);
                };
                let Some(assignment) = tasks.assignments.get(&task.envelope.task_id) else {
                    return Ok(p.batch);
                };
                let fold = p.issues(workspace);
                let Some(issue) = fold.issues.get(&(workspace, issue)) else {
                    return Ok(p.batch);
                };
                if issue.status.is_terminal()
                    || !issue
                        .external_refs
                        .iter()
                        .any(|r| r.kind == "task_event" && r.reference == assignment.event_id)
                {
                    return Ok(p.batch);
                }
                // Only the most recently linked execution may change the board state.
                let newer = tasks.assignments.values().any(|other| {
                    other.envelope.task_id != task.envelope.task_id
                        && other.envelope.tracker_issue_id == Some(issue.issue_id)
                        && other.created_at >= assignment.created_at
                        && issue
                            .external_refs
                            .iter()
                            .any(|r| r.kind == "task_event" && r.reference == other.event_id)
                });
                if newer {
                    return Ok(p.batch);
                }
                let status = if current.envelope.status == TaskStatus::PickedUp {
                    IssueStatus::InProgress
                } else {
                    IssueStatus::InReview
                };
                let key = format!("lifecycle:{}:{}", task.envelope.task_id, event.id.to_hex());
                if issue.status != status && p.claim(workspace, key.clone(), "lifecycle") {
                    p.revision(
                        workspace,
                        issue,
                        RevisionSet {
                            status: Some(status),
                            ..Default::default()
                        },
                        vec![],
                        Uuid::new_v5(&DISPATCH_REQUEST_NAMESPACE, key.as_bytes()),
                        if status == IssueStatus::InProgress {
                            "Executor accepted the task"
                        } else {
                            "Executor reported completion; independent review required"
                        },
                    )?;
                }
                Ok(p.batch)
            })
            .await
            .map_err(err)?;
        publish(state, community, events).await?;
    }
    Ok(())
}
