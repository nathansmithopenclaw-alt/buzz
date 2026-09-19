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
        let accepted = self.issues(workspace);
        let existing = self.snapshot.events.iter().find_map(|e| {
            if e.channel_id != Some(workspace) || !self.member(workspace, &e.event.pubkey.to_hex())
            {
                return None;
            }
            let parsed = parse_tracker_event(&e.event, self.now()).ok()?;
            let TrackerPayload::Root(root) = parsed.payload else {
                return None;
            };
            (root.source_event_id.as_deref() == Some(&source_hex)
                && accepted
                    .issues
                    .get(&(workspace, root.issue_id))
                    .is_some_and(|issue| issue.root_event == e.event.id.to_hex()))
            .then_some(root.issue_id)
        });
        if let Some(existing) = existing {
            return self.skip(
                channel,
                &source_hex,
                "create_issue",
                &format!("existing issue {existing} already captures source"),
            );
        }
        if !self.claim(channel, key.clone(), "create_issue") {
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
        let times = self.admission_times(workspace);
        // A compatibility fold is diagnostic only: never grant legacy history
        // unattended authority. It exposes issues hidden by missing receipts.
        let (auth, _) = self.authorization(workspace);
        let visible = fold_tracker_events(
            self.events()
                .filter(|e| scope(e) == Some(workspace))
                .filter_map(|e| parse_tracker_event(e, self.now()).ok()),
            &auth,
        );
        let mut digest = BTreeMap::<Uuid, Vec<String>>::new();
        let threshold = format!("unowned-{after_hours}h");
        let mut missing = BTreeSet::new();
        for issue in visible
            .issues
            .values()
            .filter(|i| i.stream_id == workspace && !i.status.is_terminal() && i.assignee.is_none())
        {
            if status_observed(&visible, issue, &times).is_none() {
                missing.insert(issue.issue_id);
                if self.batch.ledger.len() < 100 {
                    self.skip(
                        workspace,
                        &format!("{}:{threshold}", issue.issue_id),
                        "escalate",
                        "no admission receipt",
                    )?;
                }
            }
        }
        for issue in fold
            .issues
            .values()
            .filter(|i| i.stream_id == workspace && !i.status.is_terminal() && i.assignee.is_none())
        {
            if self.batch.ledger.len() >= 100 {
                break;
            }
            if missing.contains(&issue.issue_id) {
                continue;
            }
            let Some(observed) = status_observed(&fold, issue, &times) else {
                self.skip(
                    workspace,
                    &format!("{}:{threshold}", issue.issue_id),
                    "escalate",
                    "no admission receipt",
                )?;
                continue;
            };
            let age = self.now().saturating_sub(observed);
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
                e.event.pubkey == self.state.relay_keypair.public_key()
                    && parse_dispatch_action(&e.event, self.now())
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

// Canonical root or last actual status change, never a title-only edit.
fn status_observed(
    fold: &TrackerFold,
    issue: &IssueProjection,
    times: &BTreeMap<String, u64>,
) -> Option<u64> {
    let entered = fold
        .activity
        .get(&(issue.stream_id, issue.issue_id))?
        .iter()
        .rev()
        .find(|entry| {
            entry
                .before
                .as_ref()
                .is_none_or(|before| before.status != entry.after.status)
        })?;
    times.get(&entered.event_id).copied()
}
