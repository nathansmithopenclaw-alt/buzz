//! Durable, issue-first CLI assignment.

use super::*;

#[path = "track_dispatch_journal.rs"]
mod journal;

#[path = "track_dispatch_support.rs"]
mod support;
use support::task_for_dispatch_request;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_dispatch(
    client: &BuzzClient,
    workspace: Option<String>,
    issue_key: String,
    assignee: String,
    channel: Option<String>,
    title: Option<String>,
    deliverables: Vec<String>,
    verification: Option<String>,
    reviewer: Option<String>,
    content: Option<String>,
    allow_active: bool,
    set_status: String,
    mutation: TrackMutationArgs,
) -> Result<(), CliError> {
    let workspace_id = parse_workspace(workspace)?;
    let import = parse_import_json(mutation.import_json.as_deref())?;
    let reason = read_optional_stdin(mutation.reason.clone())?;
    let (context, _) = load_context(client, workspace_id).await?;
    let issue_id = context.issue(&issue_key)?.issue_id;
    let request_id = parse_request_id(mutation.request_id.as_deref())?;
    let signer = client.keys().public_key().to_hex();
    let base = cache_path(workspace_id)?;
    let journal = journal::AssignmentJournal::open(
        base.parent()
            .ok_or_else(|| CliError::Other("missing Tracker cache directory".into()))?,
        client.relay_url(),
        &signer,
        workspace_id,
        issue_id,
        request_id,
    )?;
    let pending = journal.read()?;
    let (mut context, authorization) = load_context(client, workspace_id).await?;
    let issue = context.issue(&issue_key)?.clone();
    if authorization.role(&signer).is_none() {
        return Err(CliError::Conflict(
            "current workspace membership is required for assignment".into(),
        ));
    }
    if issue.status.is_terminal() {
        return Err(CliError::Conflict(
            "terminal issues must be reopened before dispatch".into(),
        ));
    }
    if mutation.if_revision.as_ref().is_some_and(|expected| {
        expected != &issue.tip_event
            && !pending.as_ref().is_some_and(|saved| {
                saved.link.id.to_hex() == issue.tip_event
                    && serde_json::from_str::<IssueRevision>(&saved.link.content)
                        .is_ok_and(|revision| revision.parent_event == *expected)
            })
    }) {
        return Err(CliError::Conflict(format!(
            "current revision is {}; expected {}",
            issue.tip_event,
            mutation.if_revision.as_deref().unwrap_or_default()
        )));
    }
    let channel_id = match channel {
        Some(value) => Uuid::parse_str(&value)
            .map_err(|_| CliError::Usage(format!("invalid channel UUID: {value}")))?,
        None => issue.home_channel_id.ok_or_else(|| {
            CliError::Usage("--channel is required when the issue has no home channel".into())
        })?,
    };
    let assignee = resolve_issue_assignee(client, channel_id, &assignee).await?;
    let reviewer = match reviewer {
        Some(value) => Some(resolve_issue_assignee(client, channel_id, &value).await?),
        None => None,
    };
    let set_status = parse_status(&set_status)?;
    if set_status.is_terminal() {
        return Err(CliError::Usage(
            "dispatch --set-status cannot set a terminal state".into(),
        ));
    }
    let project_default = issue
        .project_id
        .and_then(|id| context.projects.projects.get(&id))
        .and_then(|project| project.default_verify.clone());
    let verification = verification.or(project_default).ok_or_else(|| {
        CliError::Usage("--verify is required unless the project supplies --default-verify".into())
    })?;
    let deliverables = parse_deliverables(&deliverables)?;
    let task_title = title
        .clone()
        .unwrap_or_else(|| format!("{}: {}", issue.identifier, issue.title));
    let human = match content {
        Some(value) => read_or_stdin(&value)?,
        None => format!("Dispatch {} for execution.", issue.identifier),
    };
    let intent = json!({"workspace":workspace_id,"issue":issue.issue_id,"request":request_id,
        "signer":signer,"channel":channel_id,"assignee":assignee,"reviewer":reviewer,
        "title":task_title,"deliverables":deliverables,"verification":verification,
        "content":human,"status":set_status,"reason":reason,"import":import});
    if let Some(saved) = &pending {
        saved.validate(&intent)?;
    }
    let existing_task = task_for_dispatch_request(client, request_id).await?;
    if let (Some(saved), Some(existing)) = (&pending, &existing_task) {
        if saved.task.id.to_hex() != existing.event_id {
            return Err(CliError::Conflict(
                "accepted assignment differs from the retained signed request".into(),
            ));
        }
    }
    let current_tasks = fold_task_events(query_task_events(client, None).await?);
    let retained_task_id = pending
        .as_ref()
        .map(|saved| TaskEnvelopeEvent::from_event(&saved.task))
        .transpose()
        .map_err(|error| usage("invalid retained assignment", error))?
        .map(|task| task.envelope.task_id);
    for reference in issue
        .external_refs
        .iter()
        .filter(|value| value.kind == "task")
    {
        if !current_tasks
            .values()
            .any(|task| task.envelope.task_id.to_string() == reference.reference)
            && retained_task_id.is_none_or(|task| task.to_string() != reference.reference)
        {
            return Err(CliError::Conflict("issue has an unresolved task link; recover that assignment before creating another".into()));
        }
    }
    let active: Vec<_> = current_tasks
        .values()
        .filter(|event| event.envelope.tracker_issue_id == Some(issue.issue_id))
        .filter(|event| event.envelope.dispatch_request_id != Some(request_id))
        .filter(|event| {
            !matches!(
                event.envelope.status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Verified
            )
        })
        .collect();
    if !active.is_empty() && !allow_active {
        return Err(CliError::Conflict(format!(
            "issue has {} active linked task(s); pass --allow-active to create another without superseding them",
            active.len()
        )));
    }
    if !active.is_empty() {
        context.warnings.push(format!(
            "{} existing active linked task(s) remain active; --allow-active does not supersede them",
            active.len()
        ));
    }

    let (task_event, task_id, task_replayed) = if let Some(existing) = existing_task {
        if existing.envelope.assigner != signer
            || existing.envelope.tracker_issue_id != Some(issue.issue_id)
            || existing.envelope.assignee != assignee
            || existing.envelope.verification.method != verification
            || existing.envelope.deliverables != deliverables
            || existing.envelope.title != task_title
            || existing
                .envelope
                .authority
                .as_ref()
                .and_then(|authority| authority.reviewer.as_ref())
                != reviewer.as_ref()
            || existing.channel_id != channel_id
        {
            return Err(CliError::Conflict(format!(
                "dispatch request {request_id} was already used with different semantic input"
            )));
        }
        let event = client
            .query_all(json!({"ids":[existing.event_id]}))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| CliError::Conflict("existing assignment is unavailable".into()))?;
        let event: Event =
            serde_json::from_value(event).map_err(|error| usage("invalid assignment", error))?;
        (event, existing.envelope.task_id, true)
    } else {
        let task_id = retained_task_id.unwrap_or_else(Uuid::new_v4);
        let envelope = TaskEnvelope {
            v: TASK_ENVELOPE_VERSION,
            authority: Some(buzz_sdk::task_envelope::TaskAuthority {
                reviewer: reviewer.clone(),
                ..Default::default()
            }),
            task_id,
            status: TaskStatus::Assigned,
            assignee: assignee.clone(),
            assigner: client.keys().public_key().to_hex(),
            title: task_title.clone(),
            deliverables: deliverables.clone(),
            verification: TaskVerification {
                method: verification.clone(),
                evidence_ref: None,
            },
            blocked_on: None,
            notes: None,
            tracker_issue_id: Some(issue.issue_id),
            dispatch_request_id: Some(request_id),
        }
        .validate()
        .map_err(|error| usage("invalid dispatch task", error))?;
        let builder = buzz_sdk::task_envelope::build_task_message(
            channel_id,
            &human,
            &envelope,
            None,
            &[assignee.as_str()],
        )
        .map_err(|error| usage("build task assignment", error))?;
        let event = match &pending {
            Some(saved) => saved.task.clone(),
            None => client.sign_event(builder)?,
        };
        (event, task_id, false)
    };
    let task_event_id = task_event.id.to_hex();

    let already_linked = issue
        .external_refs
        .iter()
        .any(|value| value.kind == "task" && value.reference == task_id.to_string());
    let (link_event_id, link_replayed) = if already_linked {
        let accepted_link = existing_request_event(client, workspace_id, request_id)
            .await?
            .ok_or_else(|| {
                CliError::Conflict(
                    "task link has no accepted request record; reconcile before retrying".into(),
                )
            })?;
        if accepted_link.pubkey.to_hex() != signer
            || pending
                .as_ref()
                .is_some_and(|saved| saved.link.id != accepted_link.id)
            || !issue
                .external_refs
                .iter()
                .any(|value| value.kind == "task_event" && value.reference == task_event_id)
        {
            return Err(CliError::Conflict(
                "accepted task link differs from this dispatch request".into(),
            ));
        }
        (accepted_link.id.to_hex(), true)
    } else {
        let mut set = RevisionSet {
            // Delegating execution must not remove the accountable person.
            assignee: issue
                .assignee
                .is_none()
                .then(|| client.keys().public_key().to_hex()),
            ..RevisionSet::default()
        };
        if set_status != issue.status {
            set.status = Some(set_status);
        }
        let revision = IssueRevision {
            v: TRACKER_VERSION,
            op: "revise".into(),
            issue_id: issue.issue_id,
            root_event: issue.root_event.clone(),
            parent_event: issue.tip_event.clone(),
            revision: context.revision_number(&issue) + 1,
            set,
            clear: vec![],
            relations: SetOperations::default(),
            external_refs: SetOperations {
                add: vec![
                    ExternalRef {
                        kind: "task".into(),
                        reference: task_id.to_string(),
                    },
                    ExternalRef {
                        kind: "task_event".into(),
                        reference: task_event_id.clone(),
                    },
                ],
                remove: vec![],
            },
            reason: if set_status != issue.status {
                Some(reason.unwrap_or_else(|| "dispatched for execution".into()))
            } else {
                reason
            },
            request_id: Some(request_id),
            import,
        };
        let event = match &pending {
            Some(saved) => saved.link.clone(),
            None => client.sign_event(
                build_tracker_revision(workspace_id, &revision)
                    .map_err(|error| usage("build assignment link", error))?,
            )?,
        };
        validate_candidate(&context, &authorization, &event, issue.issue_id)?;
        journal.save(&journal::PendingAssignment {
            intent: intent.clone(),
            task: task_event.clone(),
            link: event.clone(),
        })?;
        let result = async {
            let result = submit_idempotent(client, workspace_id, request_id, event.clone()).await?;
            context.add_event(event.clone(), &authorization);
            context.persist()?;
            Ok::<_, CliError>(result)
        }
        .await;
        result.inspect_err(|_| {
            support::report_pending(request_id, issue.issue_id, task_id, &event.id.to_hex());
        })?
    };
    if !task_replayed {
        if let Err(error) = support::publish_retained_task(
            client,
            workspace_id,
            issue.issue_id,
            request_id,
            &link_event_id,
            task_event,
            mutation.if_revision.as_deref(),
        )
        .await
        {
            support::report_pending(request_id, issue.issue_id, task_id, &link_event_id);
            return Err(error);
        }
    }

    println!(
        "{}",
        json!({
            "accepted": true,
            "operation": "track.dispatch",
            "request_id": request_id,
            "event_ids": [link_event_id, task_event_id],
            "issue_id": issue.issue_id,
            "task_id": task_id,
            "task_event_id": task_event_id,
            "issue_link_event_id": link_event_id,
            "assignee_pubkey": assignee,
            "issue_revision_event_id": link_event_id,
            "replayed": task_replayed && link_replayed,
            "warnings": context.warnings,
        })
    );
    Ok(())
}
