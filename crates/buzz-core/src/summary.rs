//! Versioned Summary contracts shared by clients and relay authorization.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The stable identifier assigned to the pre-existing singleton Summary.
pub const LEGACY_SUMMARY_ID: &str = "default";

/// An owner-selected report audience, independent of agent control permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SummaryAudience {
    /// Owner and the executing reporter only.
    OnlyMe,
    /// Current human community members, snapshotted per run.
    Community,
    /// Explicit human community members, in addition to the owner.
    Selected {
        /// Normalized public keys; empty selections are invalid.
        people: Vec<String>,
    },
}

/// Recurrence uses the same expressions as Workflow schedules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummarySchedule {
    /// Explicit daily wall-clock times, expanded into exact Workflow triggers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "dailyTimes"
    )]
    pub daily_times: Option<Vec<String>>,
    /// Five-field cron, mutually exclusive with interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// Workflow duration, mutually exclusive with cron.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
}

/// Owner-signed version-two Summary definition. Private to owner and reporter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryDefinition {
    /// Schema version, exactly two.
    pub v: u32,
    /// Stable UUID, or `default` for the migrated singleton.
    pub summary_id: String,
    /// Display name, independent of the agent's name.
    pub name: String,
    /// Exact agent identity, never a persona name.
    pub reporter_pubkey: String,
    /// Who may view published reports.
    pub audience: SummaryAudience,
    /// Snapshot-only access to selected conversations.
    pub read_delegation: bool,
    /// Included channel IDs.
    pub channels: Vec<Uuid>,
    /// Included DM IDs.
    pub dms: Vec<Uuid>,
    /// Explicitly excluded IDs.
    #[serde(default)]
    pub exclude: Vec<Uuid>,
    /// Preselect new sources in the editor; never silently grant access.
    #[serde(default)]
    pub auto_include: bool,
    /// Null means manual-only.
    pub schedule: Option<SummarySchedule>,
    /// IANA timezone; validated against the runtime timezone database.
    pub timezone: String,
    /// Pause automatic scheduling without deleting configuration or history.
    #[serde(default)]
    pub paused: bool,
    /// Archived definitions cannot start new runs.
    #[serde(default)]
    pub archived: bool,
}

/// Validate an identifier without allowing cross-definition tag ambiguity.
pub fn valid_summary_id(id: &str) -> bool {
    id == LEGACY_SUMMARY_ID || Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

/// Explicit config address. Legacy identity retains its original address.
pub fn config_d_tag(id: &str) -> Result<String, String> {
    if !valid_summary_id(id) {
        return Err("invalid Summary identifier".into());
    }
    Ok(if id == LEGACY_SUMMARY_ID {
        "summary".into()
    } else {
        format!("summary:{id}")
    })
}

/// Validate canonical identity keys. Never infer a human from a malformed key.
pub fn valid_pubkey(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl SummaryDefinition {
    /// Presentation and recurrence edits do not revoke an already claimed read
    /// grant. Audience, source, reporter and archive changes still do.
    pub fn same_execution_authority(&self, other: &Self) -> bool {
        self.summary_id == other.summary_id
            && self.reporter_pubkey == other.reporter_pubkey
            && self.audience == other.audience
            && self.read_delegation == other.read_delegation
            && self.archived == other.archived
            && self.channels.iter().collect::<BTreeSet<_>>() == other.channels.iter().collect()
            && self.dms.iter().collect::<BTreeSet<_>>() == other.dms.iter().collect()
    }
    /// Validate structural invariants before signing or applying a policy.
    pub fn validate(&self) -> Result<(), String> {
        if self.v != 2 || !valid_summary_id(&self.summary_id) {
            return Err("invalid Summary version or identifier".into());
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > 200
            || !valid_pubkey(&self.reporter_pubkey)
        {
            return Err("invalid Summary name or reporter".into());
        }
        let sources: BTreeSet<_> = self.channels.iter().chain(&self.dms).collect();
        if sources.len() > 256 || sources.len() != self.channels.len() + self.dms.len() {
            return Err("Summary sources must be unique and bounded".into());
        }
        if self.exclude.len() > 4096 || self.exclude.iter().any(|id| sources.contains(id)) {
            return Err("excluded conversations cannot be Summary sources".into());
        }
        if let SummaryAudience::Selected { people } = &self.audience {
            if people.is_empty()
                || people.len() > 256
                || people.iter().any(|key| !valid_pubkey(key))
                || people.iter().collect::<BTreeSet<_>>().len() != people.len()
            {
                return Err("select unique people for this Summary".into());
            }
        }
        if self.timezone.is_empty()
            || self.timezone.len() > 64
            || self.timezone.chars().any(char::is_whitespace)
        {
            return Err("invalid Summary timezone".into());
        }
        if let Some(schedule) = &self.schedule {
            if let Some(times) = &schedule.daily_times {
                if times.is_empty()
                    || times.len() > 24
                    || times.iter().collect::<BTreeSet<_>>().len() != times.len()
                    || times.iter().any(|time| {
                        let bytes = time.as_bytes();
                        bytes.len() != 5
                            || bytes[2] != b':'
                            || time
                                .get(..2)
                                .and_then(|v| v.parse::<u8>().ok())
                                .is_none_or(|h| h > 23)
                            || time
                                .get(3..)
                                .and_then(|v| v.parse::<u8>().ok())
                                .is_none_or(|m| m > 59)
                    })
                    || schedule.cron.is_some()
                    || schedule.interval.is_some()
                {
                    return Err("Choose unique daily times".into());
                }
                return Ok(());
            }
            if schedule.cron.as_ref().is_some_and(|s| s.trim().is_empty())
                || schedule
                    .interval
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty())
                || schedule.cron.is_some() == schedule.interval.is_some()
            {
                return Err("choose a cron schedule or an interval".into());
            }
        }
        Ok(())
    }

    /// Eligible human recipients. The relay supplies the authoritative roster.
    pub fn recipients(
        &self,
        owner: &str,
        humans: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, String> {
        self.validate()?;
        if !valid_pubkey(owner) {
            return Err("invalid Summary owner".into());
        }
        let mut recipients = match &self.audience {
            SummaryAudience::OnlyMe => BTreeSet::new(),
            SummaryAudience::Community => humans.clone(),
            SummaryAudience::Selected { people } => {
                if people.iter().any(|key| !humans.contains(key)) {
                    return Err(
                        "Summary recipients must be current people in this community".into(),
                    );
                }
                people.iter().cloned().collect()
            }
        };
        recipients.insert(owner.to_owned());
        Ok(recipients)
    }
}

/// A frozen run audience cannot be widened by subsequent configuration edits.
pub fn reader_in_audience(
    reader: &str,
    snapshot: &BTreeSet<String>,
    current: &BTreeSet<String>,
) -> bool {
    snapshot.contains(reader) && current.contains(reader)
}

/// Reporter-independent cursor address; one Summary cannot consume another's delta.
pub fn watermark_key(owner: &str, summary_id: &str, channel: Uuid) -> Result<String, String> {
    if !valid_pubkey(owner) || !valid_summary_id(summary_id) {
        return Err("invalid Summary cursor identity".into());
    }
    Ok(format!("wm:{owner}:{summary_id}:{channel}"))
}

/// Cursor requested for commit only after a Summary report is durably stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryCursor {
    /// Source actually read in this run.
    pub channel_id: Uuid,
    /// Last included source event.
    pub event_id: String,
    /// Source timestamp, validated against the observed event.
    pub created_at: i64,
}

/// Dedicated Summary publication input, distinct from generic conversation reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryReport {
    /// Exactly two.
    pub v: u32,
    /// Explicit owner; recipient ordering has no ownership meaning.
    pub owner_pubkey: String,
    /// Named definition.
    pub summary_id: String,
    /// Durable single-flight run identifier.
    pub run_id: Uuid,
    /// Frozen owner-signed configuration event.
    pub config_event_id: String,
    /// Human title.
    pub title: String,
    /// Agent-composed content.
    pub markdown: String,
    /// Structured output for consumers.
    pub status: serde_json::Value,
    /// Exact sources returned by the bounded read operation.
    pub sources: Vec<Uuid>,
    /// Included window start.
    pub since: i64,
    /// Included window end.
    pub until: i64,
    /// Cursors committed in the same transaction as the report.
    pub watermarks: Vec<SummaryCursor>,
}

impl SummaryReport {
    /// Reject malformed, unbounded, or internally inconsistent publication input.
    pub fn validate(&self) -> Result<(), String> {
        if self.v != 2
            || !valid_summary_id(&self.summary_id)
            || !valid_pubkey(&self.owner_pubkey)
            || !valid_pubkey(&self.config_event_id)
            || self.title.trim().is_empty()
            || self.title.len() > 200
            || self.markdown.trim().is_empty()
            || self.markdown.len() > 65_536
            || !self.status.is_object()
            || self.since < 0
            || self.until < self.since
        {
            return Err("invalid Summary report".into());
        }
        let sources: BTreeSet<_> = self.sources.iter().collect();
        if sources.len() > 256
            || sources.len() != self.sources.len()
            || self
                .watermarks
                .iter()
                .map(|cursor| cursor.channel_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.watermarks.len()
            || self.watermarks.iter().any(|cursor| {
                !sources.contains(&cursor.channel_id)
                    || !valid_pubkey(&cursor.event_id)
                    || cursor.created_at < self.since
                    || cursor.created_at > self.until
            })
        {
            return Err("invalid Summary source cursor".into());
        }
        Ok(())
    }
}

/// Translate five-field Unix weekday numbers to names before handing a Summary
/// schedule to the workflow cron library (which numbers Sunday as 1).
/// Six/seven-field engine-native expressions retain their existing semantics.
pub fn workflow_cron(expression: &str) -> Result<String, String> {
    let mut fields: Vec<String> = expression.split_whitespace().map(str::to_owned).collect();
    if fields.len() != 5 || fields[4] == "*" {
        return Ok(expression.to_owned());
    }
    let names = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    let mut output = Vec::new();
    for part in fields[4].split(',') {
        if part.chars().any(char::is_alphabetic) {
            output.push(part.to_ascii_uppercase());
            continue;
        }
        let (base, step) = part.split_once('/').map_or((part, 1), |(base, step)| {
            (base, step.parse::<usize>().unwrap_or(0))
        });
        if step == 0 {
            return Err("Invalid Summary weekday step".into());
        }
        let (start, end) = if base == "*" {
            (0, 6)
        } else if let Some((start, end)) = base.split_once('-') {
            (
                start
                    .parse::<usize>()
                    .map_err(|_| "Invalid Summary weekday")?,
                end.parse::<usize>()
                    .map_err(|_| "Invalid Summary weekday")?,
            )
        } else {
            let start = base
                .parse::<usize>()
                .map_err(|_| "Invalid Summary weekday")?;
            (start, if part.contains('/') { 7 } else { start })
        };
        if start > 7 || end > 7 || start > end {
            return Err("Invalid Summary weekday range".into());
        }
        for day in (start..=end).step_by(step) {
            let name = names[day % 7].to_owned();
            if !output.contains(&name) {
                output.push(name);
            }
        }
    }
    fields[4] = output.join(",");
    Ok(fields.join(" "))
}

#[cfg(test)]
mod tests;
