use super::*;
use buzz_sdk::task_envelope::{TaskEnvelopeEvent, TaskStatus};
/// The source lifecycle event is already durable. Ingestion logs errors without
/// failing the ACK; scheduled Dispatch ticks retry from admitted task history.
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
                p.project_lifecycle(workspace, Some(&event.id.to_hex()))?;
                Ok(p.batch)
            })
            .await
            .map_err(err)?;
        publish(state, community, events).await?;
    }
    Ok(())
}

impl Planner<'_> {
    /// Reconcile at most 32 admitted current lifecycle events per tick. Existing
    /// history is the durable retry log; ledger claims prevent later manual board
    /// changes from being overwritten by an already-consumed lifecycle event.
    pub(super) fn project_lifecycle(&mut self, workspace: Uuid, only: Option<&str>) -> Result<()> {
        let tasks = self.tasks();
        let fold = self.issues(workspace);
        let mut attempted = 0;
        for current in tasks.current.values() {
            if only.is_some_and(|id| current.event_id != id)
                || !matches!(
                    current.envelope.status,
                    TaskStatus::PickedUp | TaskStatus::Done
                )
            {
                continue;
            }
            let Some(issue_id) = current.envelope.tracker_issue_id else {
                continue;
            };
            let Some(assignment) = tasks.assignments.get(&current.envelope.task_id) else {
                continue;
            };
            let Some(issue) = fold.issues.get(&(workspace, issue_id)) else {
                continue;
            };
            if !issue
                .external_refs
                .iter()
                .any(|r| r.kind == "task_event" && r.reference == assignment.event_id)
            {
                continue;
            }
            // Only the most recently linked execution may change the board state.
            let newer = tasks.assignments.values().any(|other| {
                other.envelope.task_id != current.envelope.task_id
                    && other.envelope.tracker_issue_id == Some(issue.issue_id)
                    && other.created_at >= assignment.created_at
                    && issue
                        .external_refs
                        .iter()
                        .any(|r| r.kind == "task_event" && r.reference == other.event_id)
            });
            let key = format!(
                "lifecycle:{}:{}",
                current.envelope.task_id, current.event_id
            );
            if !self.claim(workspace, key.clone(), "lifecycle") {
                continue;
            }
            let status = if current.envelope.status == TaskStatus::PickedUp {
                IssueStatus::InProgress
            } else {
                IssueStatus::InReview
            };
            if !newer && !issue.status.is_terminal() && issue.status != status {
                self.revision(
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
            attempted += 1;
            if attempted >= 32 {
                break;
            }
        }
        Ok(())
    }
}
