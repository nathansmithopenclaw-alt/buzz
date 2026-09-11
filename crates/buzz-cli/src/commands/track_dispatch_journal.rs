//! Exact signed assignment recovery, scoped to relay, workspace and signer.

use super::*;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_REQUESTS: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PendingAssignment {
    pub intent: Value,
    pub task: Event,
    pub link: Event,
}

impl PendingAssignment {
    pub fn validate(&self, intent: &Value) -> Result<(), CliError> {
        let task = TaskEnvelopeEvent::from_event(&self.task)
            .map_err(|error| usage("invalid retained assignment", error))?;
        let link: IssueRevision = serde_json::from_str(&self.link.content)
            .map_err(|error| usage("invalid retained issue link", error))?;
        let envelope = &task.envelope;
        let task_scope = json!({"signer":envelope.assigner,"channel":task.channel_id,
            "assignee":envelope.assignee,"reviewer":envelope.authority.as_ref().and_then(|a| a.reviewer.as_ref()),
            "title":envelope.title,"deliverables":envelope.deliverables,"verification":envelope.verification.method,
            "issue":envelope.tracker_issue_id});
        let scope_matches = task_scope.as_object().is_some_and(|fields| {
            fields
                .iter()
                .all(|(key, value)| intent.get(key) == Some(value))
        });
        if self.intent != *intent
            || !scope_matches
            || json!(envelope.dispatch_request_id) != intent["request"]
            || intent["content"]
                .as_str()
                .and_then(|human| {
                    buzz_sdk::task_envelope::append_task_envelope(human, envelope).ok()
                })
                .as_deref()
                != Some(self.task.content.as_str())
            || envelope.status != TaskStatus::Assigned
            || self.task.pubkey.to_hex() != intent["signer"]
            || self.link.pubkey != self.task.pubkey
            || link.request_id != envelope.dispatch_request_id
            || Some(link.issue_id) != envelope.tracker_issue_id
            || !self.link.tags.iter().any(|tag| {
                let tag = tag.as_slice();
                tag.first().is_some_and(|tag| tag == "h")
                    && tag
                        .get(1)
                        .is_some_and(|value| *value == intent["workspace"])
            })
            || !link
                .external_refs
                .add
                .iter()
                .any(|r| r.kind == "task" && r.reference == envelope.task_id.to_string())
            || !link
                .external_refs
                .add
                .iter()
                .any(|r| r.kind == "task_event" && r.reference == self.task.id.to_hex())
        {
            return Err(CliError::Conflict("retained dispatch request has different scope or input; review its recovery record before starting another request".into()));
        }
        Ok(())
    }
}

pub(super) struct AssignmentJournal {
    path: PathBuf,
    // The OS releases this lock on process exit, including interrupted writes.
    _lock: fs::File,
}

fn storage_error(error: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("Dispatch recovery storage failed: {error}"))
}

fn open_private(path: &Path) -> Result<fs::File, CliError> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(storage_error)
}

impl AssignmentJournal {
    pub fn open(
        base: &Path,
        relay: &str,
        signer: &str,
        workspace: Uuid,
        issue: Uuid,
        request: Uuid,
    ) -> Result<Self, CliError> {
        let scope = json!([relay, signer, workspace]);
        let directory = base.join("dispatch-assignments-v1").join(
            Sha256::digest(scope.to_string().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
        fs::create_dir_all(&directory).map_err(storage_error)?;
        let lock = open_private(&directory.join(format!("{issue}.lock")))?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            CliError::Conflict(format!("another assignment is using this issue: {error}"))
        })?;
        Ok(Self {
            path: directory.join(format!("{request}.json")),
            _lock: lock,
        })
    }

    pub fn read(&self) -> Result<Option<PendingAssignment>, CliError> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(storage_error(error)),
        };
        if metadata.len() > MAX_BYTES {
            return Err(storage_error(
                "retained assignment exceeds the storage limit",
            ));
        }
        let bytes = fs::read(&self.path).map_err(storage_error)?;
        let entry: PendingAssignment = serde_json::from_slice(&bytes).map_err(storage_error)?;
        entry.task.verify().map_err(storage_error)?;
        entry.link.verify().map_err(storage_error)?;
        Ok(Some(entry))
    }

    pub fn save(&self, entry: &PendingAssignment) -> Result<(), CliError> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| storage_error("missing directory"))?;
        if !self.path.exists()
            && fs::read_dir(parent)
                .map_err(storage_error)?
                .take(MAX_REQUESTS * 2)
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage_error)?
                .len()
                >= MAX_REQUESTS * 2
        {
            return Err(storage_error("recovery directory is full; archive completed requests after accepted-history review"));
        }
        let bytes = serde_json::to_vec(entry).map_err(storage_error)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(storage_error(
                "assignment exceeds the recovery storage limit",
            ));
        }
        let temporary = parent.join(format!(".{}.tmp", Uuid::new_v4()));
        let mut file = open_private(&temporary)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(storage_error)?;
        fs::rename(&temporary, &self.path).map_err(storage_error)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(storage_error)?;
        Ok(())
    }
}
