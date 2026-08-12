use buzz_sdk_pkg::tracker::{
    fold_tracker_events, fold_tracker_projects_at, parse_tracker_event, RelationType,
    TrackerAuthorization, TrackerRole,
};
use chrono::{DateTime, Utc};
use nostr::{Event, PublicKey};
use serde_json::{json, Value};
use uuid::Uuid;

fn authorization_from_membership(event: &Event, managed_signer: &str) -> TrackerAuthorization {
    let mut roles = vec![
        (event.pubkey.to_hex(), TrackerRole::Maintainer),
        (managed_signer.to_string(), TrackerRole::Maintainer),
    ];
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        if tag.first().map(String::as_str) != Some("p") {
            continue;
        }
        let Some(pubkey) = tag.get(1).filter(|value| PublicKey::parse(value).is_ok()) else {
            continue;
        };
        let role = match tag.get(3).map(String::as_str).unwrap_or("member") {
            "owner" | "admin" | "bot" => TrackerRole::Maintainer,
            _ => TrackerRole::Member,
        };
        roles.push((pubkey.clone(), role));
    }
    TrackerAuthorization::new(roles)
}

fn fold_tracker_context_values(
    raw_events: Vec<Value>,
    membership_event: Value,
    workspace_id: &str,
    managed_signer: &str,
    snapshot_at: u64,
) -> Result<Value, String> {
    let workspace_id = workspace_id
        .parse::<Uuid>()
        .map_err(|error| format!("invalid Tracker workspace UUID: {error}"))?;
    let membership_event: Event = serde_json::from_value(membership_event)
        .map_err(|error| format!("invalid membership snapshot: {error}"))?;
    membership_event
        .verify()
        .map_err(|error| format!("invalid membership snapshot signature: {error}"))?;
    let workspace_tag = workspace_id.to_string();
    if membership_event.kind.as_u16() != 39002
        || !membership_event.tags.iter().any(|tag| {
            let tag = tag.as_slice();
            tag.first().map(String::as_str) == Some("d")
                && tag.get(1).map(String::as_str) == Some(workspace_tag.as_str())
        })
    {
        return Err("membership snapshot does not identify the Tracker workspace".to_string());
    }
    let managed_signer = PublicKey::parse(managed_signer)
        .map_err(|error| format!("invalid Tracker managed signer: {error}"))?
        .to_hex();
    let authorization = authorization_from_membership(&membership_event, &managed_signer);
    let mut signed_events = Vec::new();
    let mut parsed_tracker_events = Vec::new();
    let mut diagnostics = Vec::new();

    for raw in raw_events {
        let event: Event = match serde_json::from_value(raw) {
            Ok(event) => event,
            Err(error) => {
                diagnostics.push(json!({"kind": "InvalidEvent", "message": error.to_string()}));
                continue;
            }
        };
        if matches!(event.kind.as_u16() as u32, 40020..=40022) {
            match parse_tracker_event(&event, snapshot_at) {
                Ok(event) if event.stream_id == workspace_id => parsed_tracker_events.push(event),
                Ok(_) => {}
                Err(error) => diagnostics.push(json!({
                    "kind": "InvalidEvent",
                    "message": format!("{}: {error}", event.id.to_hex()),
                })),
            }
        }
        signed_events.push(event);
    }

    let folded = fold_tracker_events(parsed_tracker_events, &authorization);
    let projects =
        fold_tracker_projects_at(signed_events, workspace_id, &managed_signer, snapshot_at);
    diagnostics.extend(folded.diagnostics.iter().map(|diagnostic| {
        json!({"kind": format!("{diagnostic:?}").split_whitespace().next().unwrap_or("FoldDiagnostic"), "message": format!("{diagnostic:?}")})
    }));
    diagnostics.extend(projects.diagnostics.iter().map(
        |diagnostic| json!({"kind": "ProjectDiagnostic", "message": format!("{diagnostic:?}")}),
    ));

    let issues = folded
        .issues
        .values()
        .map(|issue| {
            let relations = issue
                .relations
                .iter()
                .map(|relation| {
                    json!({
                        "kind": match relation.kind {
                            RelationType::Blocks => "blocks",
                            RelationType::DuplicateOf => "duplicate_of",
                            RelationType::Parent => "parent",
                        },
                        "targetIssueId": relation.target_issue_id,
                        "targetRootEvent": relation.target_root_event,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "streamId": issue.stream_id,
                "issueId": issue.issue_id,
                "rootEvent": issue.root_event,
                "rootCreatedAt": issue.root_created_at,
                "tipEvent": issue.tip_event,
                "identifier": issue.identifier,
                "title": issue.title,
                "body": issue.body,
                "status": issue.status.to_string(),
                "priority": issue.priority.map(|value| value.to_string()),
                "assignee": issue.assignee,
                "projectId": issue.project_id,
                "due": issue.due,
                "estimateMinutes": issue.estimate_minutes,
                "homeChannelId": issue.home_channel_id,
                "workflow": issue.workflow,
                "labels": issue.labels,
                "externalRefs": issue.external_refs,
                "relations": relations,
            })
        })
        .collect::<Vec<_>>();
    let projects = projects
        .projects
        .values()
        .map(|project| {
            json!({
                "projectId": project.project_id,
                "slug": project.slug,
                "name": project.name,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "issues": issues,
        "projects": projects,
        "diagnostics": diagnostics,
        "snapshotAt": DateTime::<Utc>::from_timestamp(snapshot_at as i64, 0)
            .ok_or_else(|| "Tracker snapshot timestamp is out of range".to_string())?
            .to_rfc3339(),
    }))
}

#[tauri::command]
pub fn fold_tracker_context(
    events: Vec<Value>,
    membership_event: Value,
    workspace_id: String,
    managed_signer: String,
    snapshot_at: u64,
) -> Result<Value, String> {
    fold_tracker_context_values(
        events,
        membership_event,
        &workspace_id,
        &managed_signer,
        snapshot_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk_pkg::build_project_with_tags;
    use buzz_sdk_pkg::tracker::{build_tracker_issue, IssueRoot, IssueStatus, Priority};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn membership(workspace_id: Uuid, member: &Keys, signer: &Keys) -> Event {
        EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", workspace_id.to_string().as_str()]).expect("workspace tag"),
                Tag::parse(["p", member.public_key().to_hex().as_str(), "", "member"])
                    .expect("member tag"),
            ])
            .sign_with_keys(signer)
            .expect("signed membership")
    }

    #[test]
    fn fold_tracker_context_matches_the_sdk_issue_and_project_projections() {
        let workspace_id = Uuid::new_v4();
        let issue_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();
        let member = Keys::generate();
        let membership = membership(workspace_id, &member, &Keys::generate());
        let root = IssueRoot {
            v: 1,
            op: "create".to_string(),
            issue_id,
            identifier: "DSP-123".to_string(),
            title: "Canonical fold fixture".to_string(),
            body: "Fixture body".to_string(),
            status: IssueStatus::Todo,
            priority: Some(Priority::P1),
            assignee: Some(member.public_key().to_hex()),
            project_id: None,
            due: None,
            estimate_minutes: Some(30),
            home_channel_id: None,
            source_event_id: None,
            workflow: "default".to_string(),
            labels: vec!["desktop".to_string()],
            external_refs: Vec::new(),
            request_id: None,
            import: None,
        };
        let event = build_tracker_issue(workspace_id, &root)
            .expect("root builder")
            .sign_with_keys(&member)
            .expect("signed root");
        let project = build_project_with_tags(
            "",
            vec![
                Tag::parse(["d", "dispatch"]).expect("project slug"),
                Tag::parse(["name", "Dispatch"]).expect("project name"),
                Tag::parse(["buzz-project-id", project_id.to_string().as_str()])
                    .expect("project id"),
                Tag::parse(["buzz-workspace", workspace_id.to_string().as_str()])
                    .expect("project workspace"),
            ],
        )
        .expect("project builder")
        .sign_with_keys(&member)
        .expect("signed project");
        let snapshot_at = event.created_at.as_secs().max(project.created_at.as_secs()) + 1;
        let managed_signer = member.public_key().to_hex();

        let actual = fold_tracker_context_values(
            vec![
                serde_json::to_value(&event).expect("root JSON"),
                serde_json::to_value(&project).expect("project JSON"),
            ],
            serde_json::to_value(&membership).expect("membership JSON"),
            &workspace_id.to_string(),
            &managed_signer,
            snapshot_at,
        )
        .expect("fold context");
        let parsed = parse_tracker_event(&event, snapshot_at).expect("SDK parse");
        let expected = fold_tracker_events(
            [parsed],
            &authorization_from_membership(&membership, &managed_signer),
        );
        let expected_projects =
            fold_tracker_projects_at([project], workspace_id, &managed_signer, snapshot_at);
        let expected_issue = expected
            .issues
            .get(&(workspace_id, issue_id))
            .expect("SDK projection");
        let actual_issue = actual["issues"]
            .as_array()
            .and_then(|issues| issues.first())
            .expect("command projection");

        assert_eq!(actual_issue["issueId"], issue_id.to_string());
        assert_eq!(actual_issue["tipEvent"], expected_issue.tip_event);
        assert_eq!(actual_issue["identifier"], expected_issue.identifier);
        assert_eq!(actual_issue["title"], expected_issue.title);
        assert_eq!(actual_issue["status"], expected_issue.status.to_string());
        assert_eq!(actual_issue["priority"], "P1");
        assert_eq!(actual_issue["labels"], json!(["desktop"]));
        assert_eq!(actual["projects"][0]["projectId"], project_id.to_string());
        assert_eq!(actual["projects"][0]["slug"], "dispatch");
        assert_eq!(
            actual["projects"][0]["name"],
            expected_projects.projects[&project_id].name
        );
        assert_eq!(actual["diagnostics"], json!([]));
    }

    #[test]
    fn fold_tracker_context_rejects_a_membership_snapshot_for_another_workspace() {
        let requested = Uuid::new_v4();
        let membership = membership(Uuid::new_v4(), &Keys::generate(), &Keys::generate());
        let error = fold_tracker_context_values(
            Vec::new(),
            serde_json::to_value(membership).expect("membership JSON"),
            &requested.to_string(),
            &Keys::generate().public_key().to_hex(),
            1,
        )
        .expect_err("workspace mismatch must fail");

        assert_eq!(
            error,
            "membership snapshot does not identify the Tracker workspace"
        );
    }
}
