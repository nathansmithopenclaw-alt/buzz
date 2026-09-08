//! NIP-BR briefing events: config, watermarks, and published reports.

use buzz_core::kind::{KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT, KIND_BRIEF_WATERMARK};
use nostr::{Event, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SdkError;

/// Discovery tag value shared by every NIP-BR event.
pub const BRIEF_DISCOVERY_TAG: &str = "buzz-brief";
/// Addressable `d` tag for the owner's live briefing config.
pub const BRIEF_CONFIG_D_TAG: &str = "summary";
/// Prefix for reporter watermark `d` tags (`wm:<channel-uuid>`).
pub const BRIEF_WATERMARK_D_PREFIX: &str = "wm:";

const MAX_INCLUDE: usize = 256;
const MAX_TITLE: usize = 200;
const MAX_MARKDOWN: usize = 64 * 1024;

fn tag(parts: &[&str]) -> Result<Tag, SdkError> {
    Tag::parse(parts.iter().copied()).map_err(|e| SdkError::InvalidTag(e.to_string()))
}

fn check_pubkey_hex(s: &str, field: &str) -> Result<String, SdkError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a 64-character hex pubkey"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

fn check_event_hex(s: &str, field: &str) -> Result<String, SdkError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{field} must be a 64-character hex event id"
        )));
    }
    Ok(s.to_ascii_lowercase())
}

fn check_uuid(s: &str, field: &str) -> Result<Uuid, SdkError> {
    Uuid::parse_str(s).map_err(|_| SdkError::InvalidInput(format!("{field} must be a UUID")))
}

fn check_timezone(tz: &str) -> Result<(), SdkError> {
    if tz.is_empty() || tz.len() > 64 || tz.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(SdkError::InvalidInput(
            "timezone must be an IANA name or UTC".into(),
        ));
    }
    if tz != "UTC" && !tz.contains('/') {
        return Err(SdkError::InvalidInput(
            "timezone must be an IANA name or UTC".into(),
        ));
    }
    Ok(())
}

fn unique_uuids(values: &[String], field: &str) -> Result<Vec<String>, SdkError> {
    if values.len() > MAX_INCLUDE {
        return Err(SdkError::InvalidInput(format!(
            "{field} exceeds maximum of {MAX_INCLUDE}"
        )));
    }
    let mut out = Vec::with_capacity(values.len());
    let mut seen = std::collections::HashSet::new();
    for raw in values {
        let id = check_uuid(raw, field)?;
        let canonical = id.to_string();
        if !seen.insert(canonical.clone()) {
            return Err(SdkError::InvalidInput(format!(
                "{field} contains a duplicate UUID"
            )));
        }
        out.push(canonical);
    }
    Ok(out)
}

/// Owner-authored briefing include-set and local schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefConfig {
    /// Schema version. Currently 1.
    pub v: u32,
    /// Reporter pubkey, when one has been selected.
    #[serde(default)]
    pub reporter_pubkey: Option<String>,
    /// Local hour (0–23).
    pub hour: u8,
    /// Local minute (0–59).
    pub minute: u8,
    /// IANA timezone or `UTC`.
    pub timezone: String,
    /// Newly joined member conversations are included until opted out.
    pub auto_include: bool,
    /// Explicit consent for snapshot-only transcript reads by the selected reporter.
    #[serde(default)]
    pub read_delegation: bool,
    /// Included live channel UUIDs.
    #[serde(default)]
    pub channels: Vec<String>,
    /// Included DM conversation UUIDs.
    #[serde(default)]
    pub dms: Vec<String>,
    /// Explicit opt-outs used with `auto_include`.
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl BriefConfig {
    /// Validate field ranges and identifier shapes.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.v != 1 {
            return Err(SdkError::InvalidInput("brief config v must be 1".into()));
        }
        if self.hour > 23 {
            return Err(SdkError::InvalidInput("hour must be 0–23".into()));
        }
        if self.minute > 59 {
            return Err(SdkError::InvalidInput("minute must be 0–59".into()));
        }
        check_timezone(&self.timezone)?;
        if let Some(ref reporter) = self.reporter_pubkey {
            check_pubkey_hex(reporter, "reporter_pubkey")?;
        }
        unique_uuids(&self.channels, "channels")?;
        unique_uuids(&self.dms, "dms")?;
        unique_uuids(&self.exclude, "exclude")?;
        Ok(())
    }
}

/// Reporter-authored per-conversation watermark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefWatermark {
    /// Schema version. Currently 1.
    pub v: u32,
    /// Conversation UUID.
    pub channel_id: String,
    /// Last processed event id.
    pub last_event_id: String,
    /// Last processed `created_at`.
    pub last_created_at: u64,
}

impl BriefWatermark {
    /// Validate identifier shapes.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.v != 1 {
            return Err(SdkError::InvalidInput("brief watermark v must be 1".into()));
        }
        check_uuid(&self.channel_id, "channel_id")?;
        check_event_hex(&self.last_event_id, "last_event_id")?;
        Ok(())
    }

    /// Addressable `d` tag for this watermark.
    pub fn d_tag(&self) -> String {
        format!("{BRIEF_WATERMARK_D_PREFIX}{}", self.channel_id)
    }
}

/// Structured status block preserved for agent consumers.
pub type BriefStatus = serde_json::Value;

/// Immutable published briefing report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefReport {
    /// Schema version. Currently 1.
    pub v: u32,
    /// Run identity; matches the `d` tag.
    pub run_id: String,
    /// Human title.
    pub title: String,
    /// Human markdown body.
    pub markdown: String,
    /// Machine-queryable status object.
    #[serde(default)]
    pub status: BriefStatus,
    /// Inclusive period covered by this run.
    pub period: BriefPeriod,
    /// Conversations this run actually read.
    #[serde(default)]
    pub sources: BriefSources,
}

/// Unix-second window a report covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefPeriod {
    /// Inclusive start.
    pub since: u64,
    /// Inclusive end.
    pub until: u64,
}

/// Channel and DM UUIDs actually consulted for a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefSources {
    /// Live channels.
    #[serde(default)]
    pub channels: Vec<String>,
    /// Direct messages.
    #[serde(default)]
    pub dms: Vec<String>,
}

impl BriefReport {
    /// Validate identifier shapes and size limits.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.v != 1 {
            return Err(SdkError::InvalidInput("brief report v must be 1".into()));
        }
        check_uuid(&self.run_id, "run_id")?;
        if self.title.is_empty() || self.title.len() > MAX_TITLE {
            return Err(SdkError::InvalidInput(format!(
                "title must be 1–{MAX_TITLE} bytes"
            )));
        }
        if self.markdown.len() > MAX_MARKDOWN {
            return Err(SdkError::ContentTooLarge {
                max: MAX_MARKDOWN,
                got: self.markdown.len(),
            });
        }
        if self.period.until < self.period.since {
            return Err(SdkError::InvalidInput(
                "period.until must be >= period.since".into(),
            ));
        }
        unique_uuids(&self.sources.channels, "sources.channels")?;
        unique_uuids(&self.sources.dms, "sources.dms")?;
        Ok(())
    }
}

fn tag_first<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) == Some(name) {
            parts.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

fn parse_json_event<T>(event: &Event, kind: u32, label: &str) -> Result<T, SdkError>
where
    T: for<'de> Deserialize<'de>,
{
    if event.kind.as_u16() as u32 != kind {
        return Err(SdkError::InvalidInput(format!(
            "expected kind:{kind} {label}, got {}",
            event.kind.as_u16()
        )));
    }
    serde_json::from_str(&event.content)
        .map_err(|e| SdkError::InvalidInput(format!("invalid {label} JSON: {e}")))
}

/// Parse a kind 30623 config event.
pub fn parse_brief_config(event: &Event) -> Result<BriefConfig, SdkError> {
    let config: BriefConfig = parse_json_event(event, KIND_BRIEF_CONFIG, "brief config")?;
    config.validate()?;
    let d = tag_first(event, "d").ok_or_else(|| {
        SdkError::InvalidInput("brief config is missing the required `d` tag".into())
    })?;
    if d != BRIEF_CONFIG_D_TAG {
        return Err(SdkError::InvalidInput(format!(
            "brief config d-tag must be {BRIEF_CONFIG_D_TAG:?}"
        )));
    }
    Ok(config)
}

/// Parse a kind 30624 watermark event.
pub fn parse_brief_watermark(event: &Event) -> Result<BriefWatermark, SdkError> {
    let watermark: BriefWatermark =
        parse_json_event(event, KIND_BRIEF_WATERMARK, "brief watermark")?;
    watermark.validate()?;
    Ok(watermark)
}

/// Parse a kind 40030 report event.
pub fn parse_brief_report(event: &Event) -> Result<BriefReport, SdkError> {
    let report: BriefReport = parse_json_event(event, KIND_BRIEF_REPORT, "brief report")?;
    report.validate()?;
    Ok(report)
}

/// Build the owner's live briefing config event (kind 30623).
pub fn build_brief_config(
    owner_pubkey: &str,
    config: &BriefConfig,
) -> Result<EventBuilder, SdkError> {
    config.validate()?;
    let owner = check_pubkey_hex(owner_pubkey, "owner_pubkey")?;
    let channels = unique_uuids(&config.channels, "channels")?;
    let dms = unique_uuids(&config.dms, "dms")?;
    let exclude = unique_uuids(&config.exclude, "exclude")?;
    let mut tags = vec![
        tag(&["d", BRIEF_CONFIG_D_TAG])?,
        tag(&["t", BRIEF_DISCOVERY_TAG])?,
        tag(&["p", &owner])?,
        tag(&[
            "auto-include",
            if config.auto_include { "true" } else { "false" },
        ])?,
        tag(&["tz", &config.timezone])?,
        tag(&["hour", &config.hour.to_string()])?,
        tag(&["minute", &config.minute.to_string()])?,
    ];
    if let Some(ref reporter) = config.reporter_pubkey {
        let reporter = check_pubkey_hex(reporter, "reporter_pubkey")?;
        if reporter != owner {
            tags.push(tag(&["p", &reporter])?);
        }
    }
    for id in &channels {
        tags.push(tag(&["include", "channel", id])?);
    }
    for id in &dms {
        tags.push(tag(&["include", "dm", id])?);
    }
    for id in &exclude {
        tags.push(tag(&["exclude", id])?);
    }
    let content = serde_json::to_string(config)
        .map_err(|e| SdkError::InvalidInput(format!("serialize brief config: {e}")))?;
    // NIP-BR private reads require the author's own recipient tag after signing.
    Ok(
        EventBuilder::new(Kind::Custom(KIND_BRIEF_CONFIG as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

/// Build a reporter watermark event (kind 30624).
pub fn build_brief_watermark(
    reporter_pubkey: &str,
    owner_pubkey: Option<&str>,
    watermark: &BriefWatermark,
) -> Result<EventBuilder, SdkError> {
    watermark.validate()?;
    let reporter = check_pubkey_hex(reporter_pubkey, "reporter_pubkey")?;
    let mut tags = vec![
        tag(&["d", &watermark.d_tag()])?,
        tag(&["t", BRIEF_DISCOVERY_TAG])?,
        tag(&["p", &reporter])?,
    ];
    if let Some(owner) = owner_pubkey {
        let owner = check_pubkey_hex(owner, "owner_pubkey")?;
        if owner != reporter {
            tags.push(tag(&["p", &owner])?);
        }
    }
    let content = serde_json::to_string(watermark)
        .map_err(|e| SdkError::InvalidInput(format!("serialize brief watermark: {e}")))?;
    // NIP-BR private reads require the author's own recipient tag after signing.
    Ok(
        EventBuilder::new(Kind::Custom(KIND_BRIEF_WATERMARK as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

/// Build an immutable briefing report (kind 40030).
pub fn build_brief_report(
    owner_pubkey: &str,
    reporter_pubkey: Option<&str>,
    report: &BriefReport,
) -> Result<EventBuilder, SdkError> {
    report.validate()?;
    let owner = check_pubkey_hex(owner_pubkey, "owner_pubkey")?;
    let mut tags = vec![
        tag(&["d", &report.run_id])?,
        tag(&["t", BRIEF_DISCOVERY_TAG])?,
        tag(&["p", &owner])?,
        tag(&["title", &report.title])?,
        tag(&["published_at", &report.period.until.to_string()])?,
    ];
    if let Some(reporter) = reporter_pubkey {
        let reporter = check_pubkey_hex(reporter, "reporter_pubkey")?;
        if reporter != owner {
            tags.push(tag(&["p", &reporter])?);
        }
    }
    let content = serde_json::to_string(report)
        .map_err(|e| SdkError::InvalidInput(format!("serialize brief report: {e}")))?;
    // NIP-BR private reads require the author's own recipient tag after signing.
    Ok(
        EventBuilder::new(Kind::Custom(KIND_BRIEF_REPORT as u16), content)
            .tags(tags)
            .allow_self_tagging(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const REPORTER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CHANNEL: &str = "53f97470-aab9-43f7-87c0-8506c7208d73";
    const EVENT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn sign(builder: EventBuilder) -> Event {
        builder.sign_with_keys(&Keys::generate()).expect("sign")
    }

    fn sample_config() -> BriefConfig {
        BriefConfig {
            v: 1,
            reporter_pubkey: Some(REPORTER.to_string()),
            hour: 9,
            minute: 0,
            timezone: "America/New_York".into(),
            auto_include: true,
            read_delegation: false,
            channels: vec![CHANNEL.to_string()],
            dms: vec![],
            exclude: vec![],
        }
    }

    #[test]
    fn signed_briefs_preserve_author_recipient_for_private_reads() {
        let owner = Keys::generate();
        let reporter = Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let reporter_hex = reporter.public_key().to_hex();
        let mut config = sample_config();
        config.reporter_pubkey = Some(reporter_hex.clone());
        let watermark = BriefWatermark {
            v: 1,
            channel_id: CHANNEL.into(),
            last_event_id: EVENT.into(),
            last_created_at: 10,
        };
        let report = BriefReport {
            v: 1,
            run_id: "806e3534-99d8-4664-90a4-303ab1f28aea".into(),
            title: "Summary".into(),
            markdown: "Read one conversation.".into(),
            status: serde_json::json!({}),
            period: BriefPeriod {
                since: 0,
                until: 10,
            },
            sources: BriefSources::default(),
        };
        for (builder, signer, recipients) in [
            (
                build_brief_config(&owner_hex, &config).unwrap(),
                &owner,
                vec![owner_hex.as_str(), reporter_hex.as_str()],
            ),
            (
                build_brief_watermark(&reporter_hex, Some(&owner_hex), &watermark).unwrap(),
                &reporter,
                vec![reporter_hex.as_str(), owner_hex.as_str()],
            ),
            (
                build_brief_watermark(&reporter_hex, None, &watermark).unwrap(),
                &reporter,
                vec![reporter_hex.as_str()],
            ),
            (
                build_brief_report(&owner_hex, Some(&reporter_hex), &report).unwrap(),
                &reporter,
                vec![owner_hex.as_str(), reporter_hex.as_str()],
            ),
            (
                build_brief_report(&owner_hex, Some(&owner_hex), &report).unwrap(),
                &owner,
                vec![owner_hex.as_str()],
            ),
        ] {
            let event = builder.sign_with_keys(signer).unwrap();
            event.verify().unwrap();
            let actual: Vec<&str> = event
                .tags
                .iter()
                .filter_map(|tag| {
                    let parts = tag.as_slice();
                    (parts.first().map(String::as_str) == Some("p")).then(|| parts[1].as_str())
                })
                .collect();
            assert_eq!(
                actual, recipients,
                "signed kind {} lost a private-read recipient",
                event.kind
            );
        }
    }

    #[test]
    fn legacy_config_does_not_grant_transcript_reads() {
        let mut json = serde_json::to_value(sample_config()).unwrap();
        json.as_object_mut().unwrap().remove("read_delegation");
        assert!(
            !serde_json::from_value::<BriefConfig>(json)
                .unwrap()
                .read_delegation
        );
        let mut config = sample_config();
        config.read_delegation = true;
        let event = sign(build_brief_config(OWNER, &config).unwrap());
        assert!(parse_brief_config(&event).unwrap().read_delegation);
    }

    #[test]
    fn config_round_trip() {
        let config = sample_config();
        let event = sign(build_brief_config(OWNER, &config).unwrap());
        assert_eq!(event.kind.as_u16() as u32, KIND_BRIEF_CONFIG);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", BRIEF_CONFIG_D_TAG]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["t", BRIEF_DISCOVERY_TAG]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["include", "channel", CHANNEL]));
        assert_eq!(parse_brief_config(&event).unwrap(), config);
    }

    #[test]
    fn config_rejects_utc_hour_overflow() {
        let mut config = sample_config();
        config.hour = 24;
        let err = build_brief_config(OWNER, &config).unwrap_err();
        assert!(err.to_string().contains("hour"));
    }

    #[test]
    fn config_rejects_non_iana_timezone() {
        let mut config = sample_config();
        config.timezone = "Eastern".into();
        let err = build_brief_config(OWNER, &config).unwrap_err();
        assert!(err.to_string().contains("timezone"));
    }

    #[test]
    fn watermark_round_trip() {
        let watermark = BriefWatermark {
            v: 1,
            channel_id: CHANNEL.to_string(),
            last_event_id: EVENT.to_string(),
            last_created_at: 1_720_000_000,
        };
        let event = sign(build_brief_watermark(REPORTER, Some(OWNER), &watermark).unwrap());
        assert_eq!(event.kind.as_u16() as u32, KIND_BRIEF_WATERMARK);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", &format!("{BRIEF_WATERMARK_D_PREFIX}{CHANNEL}")]));
        assert_eq!(parse_brief_watermark(&event).unwrap(), watermark);
    }

    #[test]
    fn report_round_trip() {
        let report = BriefReport {
            v: 1,
            run_id: "806e3534-99d8-4664-90a4-303ab1f28aea".into(),
            title: "Wednesday, 27 August".into(),
            markdown: "Shipped the Summary tab.".into(),
            status: serde_json::json!({"projects": []}),
            period: BriefPeriod {
                since: 1_720_000_000,
                until: 1_720_086_400,
            },
            sources: BriefSources {
                channels: vec![CHANNEL.to_string()],
                dms: vec![],
            },
        };
        let event = sign(build_brief_report(OWNER, Some(REPORTER), &report).unwrap());
        assert_eq!(event.kind.as_u16() as u32, KIND_BRIEF_REPORT);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", &report.run_id]));
        assert_eq!(parse_brief_report(&event).unwrap(), report);
    }

    #[test]
    fn report_rejects_inverted_period() {
        let report = BriefReport {
            v: 1,
            run_id: "806e3534-99d8-4664-90a4-303ab1f28aea".into(),
            title: "Bad".into(),
            markdown: "x".into(),
            status: serde_json::json!({}),
            period: BriefPeriod {
                since: 20,
                until: 10,
            },
            sources: BriefSources::default(),
        };
        let err = build_brief_report(OWNER, None, &report).unwrap_err();
        assert!(err.to_string().contains("period"));
    }
}
