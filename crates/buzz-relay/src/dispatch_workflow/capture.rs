use super::*;
use std::collections::BTreeMap;
impl Planner<'_> {
    pub(super) fn capture(
        &mut self,
        workflow_channel: Uuid,
        record: &DispatchRecordProjection,
        ids: &[Vec<u8>],
    ) -> Result<()> {
        let DispatchRecordPayload::Capture {
            channel,
            workspace,
            project,
            enabled,
        } = record.record.payload
        else {
            return Err("capture record required".into());
        };
        if channel != workflow_channel {
            return self.skip(
                workflow_channel,
                &record.event_id,
                "create_issue",
                "authorization scope mismatch",
            );
        }
        let source_id = ids.get(1).ok_or("capture source missing")?;
        let source_hex = hex::encode(source_id);
        if !enabled {
            return self.skip(channel, &source_hex, "create_issue", "capture paused");
        }
        if !self.member(workspace, &record.author) || !self.audience(channel, workspace) {
            return self.skip(
                channel,
                &source_hex,
                "create_issue",
                "destination audience not authorized",
            );
        }
        let source = self
            .snapshot
            .events
            .iter()
            .find(|e| e.event.id.as_bytes().as_slice() == source_id)
            .ok_or("capture source missing")?;
        if source.channel_id != Some(channel)
            || !matches!(source.event.kind.as_u16(), 9 | 11 | 45001 | 45003)
        {
            return Err("pin target is not a message in the authorized channel".into());
        }
        let first_line = source.event.content.lines().next().unwrap_or("").trim();
        let mut end = first_line.len().min(256);
        while !first_line.is_char_boundary(end) {
            end -= 1;
        }
        let title = first_line[..end].to_owned();
        let title = if title.is_empty() {
            "Tracked message".into()
        } else {
            title
        };
        let request = pin_capture_request_id(&source_hex);
        let key = format!("capture:{source_hex}");
        // Cross-writer source pre-check preserves captures made by the desktop or sidecar.
        let existing = self.snapshot.events.iter().any(|e| {
            e.channel_id == Some(workspace)
                && serde_json::from_str::<IssueRoot>(&e.event.content)
                    .is_ok_and(|root| root.source_event_id.as_deref() == Some(&source_hex))
        });
        if existing || !self.claim(channel, key.clone(), "create_issue") {
            return Ok(());
        }
        let root = IssueRoot {
            v: 1,
            op: "create".into(),
            issue_id: request,
            identifier: format!("D-{}", request.simple()),
            title,
            body: format!("[Source message](buzz://message?channel={channel}&id={source_hex})"),
            status: IssueStatus::Triage,
            priority: None,
            assignee: None,
            project_id: project,
            due: None,
            estimate_minutes: None,
            home_channel_id: Some(channel),
            source_event_id: Some(source_hex.clone()),
            workflow: "default-v1".into(),
            labels: vec![],
            external_refs: vec![ExternalRef {
                kind: "origin-message".into(),
                reference: source_hex.clone(),
            }],
            request_id: Some(request),
            import: None,
        };
        self.emit(
            workspace,
            build_tracker_issue(workspace, &root).map_err(err)?,
        )?;
        self.outcome(
            workspace,
            &key,
            &source_hex,
            "create_issue",
            &format!("Created {}", root.identifier),
            None,
        )?;
        Ok(())
    }

    pub(super) fn escalate(
        &mut self,
        workflow_channel: Uuid,
        record: &DispatchRecordProjection,
        requested_hours: u32,
    ) -> Result<()> {
        let DispatchRecordPayload::UnownedAlert {
            workspace,
            channel,
            after_hours,
            enabled,
        } = record.record.payload
        else {
            return Err("unowned alert record required".into());
        };
        if workspace != workflow_channel || after_hours != requested_hours {
            return self.skip(
                workflow_channel,
                &record.event_id,
                "escalate",
                "authorization scope mismatch",
            );
        }
        if !enabled {
            return self.skip(workspace, &record.event_id, "escalate", "alerts paused");
        }
        let fold = self.issues(workspace);
        let mut digest = BTreeMap::<Uuid, Vec<String>>::new();
        let threshold = format!("unowned-{after_hours}h");
        for issue in fold
            .issues
            .values()
            .filter(|i| i.stream_id == workspace && !i.status.is_terminal() && i.assignee.is_none())
        {
            if self.batch.ledger.len() >= 100 {
                break;
            }
            let age = self.now().saturating_sub(issue.entered_status_at);
            if age < u64::from(after_hours) * 3600 {
                continue;
            }
            let destination = channel.or(issue.home_channel_id).unwrap_or(workspace);
            if !self.member(destination, &record.author) || !self.audience(workspace, destination) {
                self.skip(
                    workspace,
                    &issue.tip_event,
                    "escalate",
                    "destination audience not authorized",
                )?;
                continue;
            }
            let key = format!("escalate:{}:{threshold}", issue.issue_id);
            let request = threshold_request_id(issue.issue_id, &threshold, 0);
            let prior = self.snapshot.events.iter().any(|e| {
                parse_dispatch_action(&e.event, self.now())
                    .is_ok_and(|a| a.stream_id == workspace && a.action.request_id == request)
            });
            if prior || !self.claim(workspace, key, "escalate") {
                continue;
            }
            let title = issue.title.replace(['\n', '\r'], " ");
            digest.entry(destination).or_default().push(format!(
                "{} — {} — {}h",
                issue.identifier,
                title,
                age / 3600
            ));
            self.action_record(
                workspace,
                issue,
                request,
                DispatchActionKind::Escalate,
                Some(threshold.clone()),
                "Unowned work reminder",
            )?;
        }
        for (destination, lines) in digest {
            self.emit(
                destination,
                EventBuilder::new(
                    Kind::Custom(9),
                    format!("Work that still needs an owner:\n{}", lines.join("\n")),
                )
                .tags([
                    tag("h", &destination.to_string())?,
                    tag("buzz:workflow", "true")?,
                ]),
            )?;
        }
        Ok(())
    }
}
