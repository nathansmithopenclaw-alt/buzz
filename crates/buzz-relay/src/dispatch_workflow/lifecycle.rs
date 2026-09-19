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
    pub(super) fn has_pending_lifecycle(&self, workspace: Uuid, only: Option<&str>) -> bool {
        let issues = self
            .events()
            .filter(|event| event.kind.as_u16() == 40020 && scope(event) == Some(workspace))
            .filter_map(|event| {
                serde_json::from_str::<IssueRoot>(&event.content)
                    .ok()
                    .map(|root| root.issue_id)
            })
            .collect::<BTreeSet<_>>();
        if issues.is_empty() {
            return false;
        }
        let claims = self
            .snapshot
            .ledger
            .iter()
            .chain(self.batch.ledger.iter())
            .filter(|entry| entry.workspace == workspace && entry.action == "lifecycle")
            .map(|entry| entry.key.as_str())
            .collect::<BTreeSet<_>>();
        let candidates = self
            .events()
            .filter(|event| event.kind.as_u16() == 9)
            .filter_map(|event| {
                buzz_sdk::task_envelope::parse_task_envelope(&event.content)
                    .ok()
                    .map(|task| (event.id.to_hex(), task))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut consumed = BTreeSet::new();
        for (id, task) in &candidates {
            if !claims.contains(format!("lifecycle:{}:{id}", task.task_id).as_str()) {
                continue;
            }
            // A committed claim authenticates this exact accepted chain. Older
            // pickup states need no replay after their completion was projected.
            let mut ancestor = Some(id.as_str());
            while let Some(id) = ancestor {
                if !consumed.insert(id.to_owned()) {
                    break;
                }
                ancestor = candidates
                    .get(id)
                    .filter(|prior| prior.task_id == task.task_id)
                    .and_then(|prior| prior.authority.as_ref()?.previous_event_id.as_deref());
            }
        }
        candidates.iter().any(|(id, task)| {
            only.is_none_or(|requested| id == requested)
                && task.tracker_issue_id.is_some_and(|id| issues.contains(&id))
                && matches!(task.status, TaskStatus::PickedUp | TaskStatus::Done)
                && !consumed.contains(id)
        })
    }
    /// Reconcile at most 32 admitted current lifecycle events per tick. Existing
    /// history is the durable retry log; ledger claims prevent later manual board
    /// changes from being overwritten by an already-consumed lifecycle event.
    pub(super) fn project_lifecycle(&mut self, workspace: Uuid, only: Option<&str>) -> Result<()> {
        // Cheap conservative gate before signature verification and authority
        // folding. False positives are harmless; a consumed current transition
        // is marked only after the full fold below proves its authority.
        if !self.has_pending_lifecycle(workspace, only) {
            return Ok(());
        }
        let tasks = self.tasks();
        let fold = self.issues(workspace);
        let times = self
            .snapshot
            .channels
            .iter()
            .flat_map(|channel| self.admission_times(*channel))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut pending = tasks
            .current
            .values()
            .filter(|current| times.contains_key(&current.event_id))
            .collect::<Vec<_>>();
        pending.sort_by_key(|current| (times[&current.event_id], &current.event_id));
        for current in pending {
            if self
                .batch
                .ledger
                .iter()
                .filter(|entry| entry.action == "lifecycle")
                .count()
                >= 32
            {
                break;
            }
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
        }
        Ok(())
    }
}
