use super::*;
use buzz_sdk::{
    task_authority::fold_authorized_task_events,
    task_envelope::{
        build_task_message, TaskAuthority, TaskEnvelope, TaskEnvelopeEvent, TaskStatus,
        TaskVerification,
    },
};
impl Planner<'_> {
    pub(super) fn tasks(&self) -> buzz_sdk::task_authority::TaskAuthorityFold {
        let mut tasks = Vec::new();
        for channel in &self.snapshot.channels {
            let (_, events) = self.authorization(*channel);
            tasks.extend(
                events
                    .iter()
                    .filter_map(|e| TaskEnvelopeEvent::from_event(e).ok()),
            );
        }
        fold_authorized_task_events(tasks)
    }
    pub(super) fn assign(
        &mut self,
        workflow_channel: Uuid,
        record: &DispatchRecordProjection,
    ) -> Result<()> {
        let DispatchRecordPayload::Autopilot {
            workspace,
            project,
            agents,
            label,
            at_a_time,
            per_day,
            reviewer_required: _,
            enabled,
            activation_id,
            confirmation_required,
            confirmation_event,
        } = &record.record.payload
        else {
            return Err("autopilot record required".into());
        };
        let workspace = *workspace;
        let project = *project;
        if workspace != workflow_channel {
            return self.skip(
                workflow_channel,
                &record.event_id,
                "assign",
                "authorization scope mismatch",
            );
        }
        if !enabled {
            return self.skip(workspace, &record.event_id, "assign", "Autopilot paused");
        }
        let activation_start=self.snapshot.events.iter().filter_map(|s|serde_json::from_str::<DispatchRecord>(&s.event.content).ok().map(|r|(s,r))).filter(|(s,r)|scope(&s.event)==Some(workspace)&&record.accepted_events.contains(&s.event.id.to_hex())&&matches!(&r.payload,DispatchRecordPayload::Autopilot{activation_id:id,..} if id==activation_id)).min_by_key(|(_,r)|r.revision);
        let Some((activation_event, initial)) = activation_start else {
            return Err("Autopilot activation ancestry missing".into());
        };
        if !matches!(
            initial.payload,
            DispatchRecordPayload::Autopilot {
                confirmation_required: true,
                ..
            }
        ) {
            return self.skip(
                workspace,
                &record.event_id,
                "assign",
                "new activation requires confirmation guard",
            );
        }
        let activated_at = self
            .snapshot
            .events
            .iter()
            .filter(|stored| {
                stored.event.kind.as_u16() == 40027
                    && stored.event.pubkey == self.state.relay_keypair.public_key()
            })
            .filter_map(|stored| {
                serde_json::from_str::<DispatchAdmission>(&stored.event.content).ok()
            })
            .filter(|receipt| {
                receipt.workspace == workspace
                    && receipt.event_id == activation_event.event.id.to_hex()
            })
            .map(|receipt| receipt.observed_at)
            .min()
            .ok_or("activation has no relay admission time")?;
        let first_hour_count = self
            .snapshot
            .ledger
            .iter()
            .filter(|e| {
                e.workspace == workspace
                    && e.project == Some(project)
                    && e.activation == Some(*activation_id)
                    && e.action == "assign"
                    && (e.created_at as u64) < activated_at.saturating_add(3600)
            })
            .count();
        // Bind confirmation to the third committed action, rather than author
        // timestamps (multiple actions and edits can share a second).
        let mut challenge = self.snapshot.ledger.iter().find_map(|entry| {
            (entry.workspace == workspace
                && entry.project == Some(project)
                && entry.activation == Some(*activation_id))
            .then_some(entry.confirmation_event.clone())
            .flatten()
        });
        let confirmed = !*confirmation_required
            && first_hour_count >= 3
            && confirmation_event
                .as_ref()
                .is_some_and(|id| Some(id) == challenge.as_ref());
        let first_hour = self.now() < activated_at.saturating_add(3600);
        let fold = self.issues(workspace);
        let tasks = self.tasks();
        let mut busy = std::collections::BTreeMap::<String, usize>::new();
        for task in tasks
            .current
            .values()
            .filter(|t| !matches!(t.envelope.status, TaskStatus::Verified | TaskStatus::Failed))
        {
            *busy.entry(task.envelope.assignee.clone()).or_default() += 1;
        }
        let mut today = self
            .snapshot
            .ledger
            .iter()
            .filter(|e| {
                e.workspace == workspace
                    && e.project == Some(project)
                    && e.action == "assign"
                    && e.created_at.div_euclid(86400) == self.snapshot.now.div_euclid(86400)
            })
            .count();
        let mut first_count = first_hour_count;
        let records = self.records(workspace);
        for issue in fold.issues.values().filter(|i| {
            i.stream_id == workspace
                && i.project_id == Some(project)
                && i.status == IssueStatus::Todo
        }) {
            if self.batch.ledger.len() >= 60 {
                break;
            }
            if label
                .as_ref()
                .is_some_and(|label| !issue.labels.contains(label))
            {
                continue;
            }
            if fold.issues.values().any(|other| {
                !other.status.is_terminal()
                    && other.relations.iter().any(|r| {
                        r.kind == RelationType::Blocks && r.target_issue_id == issue.issue_id
                    })
            }) {
                continue;
            }
            // Imported work remains inert until a signed activation explicitly enables it.
            let imported = self
                .snapshot
                .events
                .iter()
                .find(|s| s.event.id.to_hex() == issue.root_event)
                .and_then(|s| serde_json::from_str::<IssueRoot>(&s.event.content).ok())
                .is_some_and(|r| r.import.is_some());
            if imported && !records.iter().any(|r|matches!(r.record.payload,DispatchRecordPayload::Activation{issue_id,active:true,..} if issue_id==issue.issue_id)) {continue;}
            if tasks.current.values().any(|t| {
                t.envelope.tracker_issue_id == Some(issue.issue_id)
                    && !matches!(t.envelope.status, TaskStatus::Verified | TaskStatus::Failed)
            }) {
                continue;
            }
            let key = format!("assign:{}:{}", issue.issue_id, issue.tip_event);
            if self.has(workspace, &key) {
                continue;
            }
            if today >= *per_day as usize {
                break;
            } // per-day-cap: negative-control call site
            if (first_hour || first_hour_count >= 3) && !confirmed && first_count >= 3 {
                self.batch.output = serde_json::json!({"confirmation_required":true,"activation_id":activation_id,"confirmation_event":challenge});
                break;
            }
            let reviewer = issue
                .assignee
                .clone()
                .or_else(|| {
                    self.snapshot
                        .events
                        .iter()
                        .find(|s| s.event.id.to_hex() == issue.root_event)
                        .map(|s| s.event.pubkey.to_hex())
                })
                .ok_or("issue owner missing")?;
            let destination = issue.home_channel_id.unwrap_or(workspace);
            if reviewer == self.state.relay_keypair.public_key().to_hex()
                || !self.member(workspace, &reviewer)
                || !self.member(destination, &reviewer)
                || !self.audience(workspace, destination)
            {
                self.skip(
                    workspace,
                    &issue.tip_event,
                    "assign",
                    "issue owner or destination not authorized",
                )?;
                continue;
            }
            let executor = agents
                .iter()
                .find(|agent| {
                    if **agent == reviewer
                        || !self.member(workspace, agent)
                        || !self.member(destination, agent)
                    {
                        return false;
                    }
                    let capacity = records
                        .iter()
                        .find_map(|r| match &r.record.payload {
                            DispatchRecordPayload::Capacity {
                                executor,
                                max_active,
                                paused,
                                ..
                            } if executor == *agent => Some(if *paused { 0 } else { *max_active }),
                            _ => None,
                        })
                        .unwrap_or(*at_a_time)
                        .min(*at_a_time);
                    busy.get(*agent).copied().unwrap_or(0) < capacity as usize
                })
                .cloned();
            let Some(executor) = executor else {
                continue;
            };
            let request = Uuid::new_v5(&DISPATCH_REQUEST_NAMESPACE, key.as_bytes());
            let task_id = Uuid::new_v4();
            let envelope = TaskEnvelope {
                v: 3,
                task_id,
                status: TaskStatus::Assigned,
                assignee: executor.clone(),
                assigner: self.state.relay_keypair.public_key().to_hex(),
                title: issue.title.clone(),
                deliverables: vec![],
                verification: TaskVerification {
                    method: "The issue owner independently reviews the result and evidence.".into(),
                    evidence_ref: None,
                },
                blocked_on: None,
                notes: Some(format!("Autopilot authorization {}", record.event_id)),
                tracker_issue_id: Some(issue.issue_id),
                dispatch_request_id: Some(request),
                authority: Some(TaskAuthority {
                    assignment_event_id: None,
                    previous_event_id: None,
                    reviewer: Some(reviewer.clone()),
                }),
            };
            let builder = build_task_message(
                destination,
                &format!(
                    "{}: {}\n{}\nReviewer: {}",
                    issue.identifier, issue.title, issue.body, reviewer
                ),
                &envelope,
                None,
                &[&executor],
            )
            .map_err(err)?
            .tags([
                tag("buzz:workflow", "true")?,
                tag("buzz:workflow-owner", &record.author)?,
                tag("buzz:workflow-mention", &executor)?,
            ]);
            let event = builder
                .custom_created_at(Timestamp::from(self.now()))
                .sign_with_keys(&self.state.relay_keypair)
                .map_err(err)?;
            // Reciprocal links and assignment are one transaction, before any wake.
            self.revision(
                workspace,
                issue,
                RevisionSet {
                    assignee: issue.assignee.is_none().then_some(reviewer),
                    ..Default::default()
                },
                vec![
                    ExternalRef {
                        kind: "task".into(),
                        reference: task_id.to_string(),
                    },
                    ExternalRef {
                        kind: "task_event".into(),
                        reference: event.id.to_hex(),
                    },
                ],
                request,
                "Assigned for execution by Autopilot",
            )?;
            self.emit(
                destination,
                EventBuilder::new(event.kind, event.content.clone())
                    .tags(event.tags.iter().cloned()),
            )?;
            let action_event = self.action_record(
                workspace,
                issue,
                request,
                DispatchActionKind::Assign,
                None,
                &format!("Assigned execution to {executor}"),
            )?;
            self.batch.ledger.push(DispatchLedgerEntry {
                key,
                workspace,
                action: "assign".into(),
                project: Some(project),
                executor: Some(executor.clone()),
                activation: Some(*activation_id),
                created_at: self.snapshot.now,
                confirmation_event: (first_hour && first_count == 2)
                    .then_some(action_event.clone()),
            });
            *busy.entry(executor).or_default() += 1;
            today += 1;
            if first_hour {
                first_count += 1;
            }
            if first_hour && first_count == 3 && !confirmed {
                challenge = Some(action_event.clone());
                self.outcome(
                    workspace,
                    &format!("confirmation:{activation_id}"),
                    &record.event_id,
                    "assign",
                    "Autopilot made 3 assignments in the first hour. Keep it on?",
                    Some(action_event.clone()),
                )?;
                self.batch.output = serde_json::json!({"confirmation_required":true,"activation_id":activation_id,"confirmation_event":action_event});
            }
            if self.batch.ledger.len() >= 60 {
                break;
            }
        }
        Ok(())
    }
}
