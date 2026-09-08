//! `buzz briefs` — NIP-BR personal briefing reports (kinds 30623/30624/40030).

use buzz_core::kind::{KIND_BRIEF_CONFIG, KIND_BRIEF_REPORT, KIND_BRIEF_WATERMARK};
use buzz_sdk::{
    build_brief_config, build_brief_report, build_brief_watermark, parse_brief_config,
    parse_brief_report, parse_brief_watermark, BriefConfig, BriefPeriod, BriefReport, BriefSources,
    BriefWatermark, BRIEF_CONFIG_D_TAG, BRIEF_DISCOVERY_TAG, BRIEF_WATERMARK_D_PREFIX,
};
use nostr::Event;
use uuid::Uuid;

use crate::client::BuzzClient;
use crate::commands::parse_write_response;
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err, validate_hex64, validate_uuid};

fn parse_events(json: &str) -> Result<Vec<Event>, CliError> {
    serde_json::from_str::<Vec<Event>>(json)
        .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))
}

fn newest_first(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    events
}

pub(crate) fn config_read_filter(author: &str, reader: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_BRIEF_CONFIG],
        "authors": [author],
        "#d": [BRIEF_CONFIG_D_TAG],
        "#p": [reader],
        "limit": 1,
    })
}

pub(crate) fn watermarks_list_filter(reader: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_BRIEF_WATERMARK],
        "authors": [reader],
        "#p": [reader],
        "#t": [BRIEF_DISCOVERY_TAG],
        "limit": 200,
    })
}

pub(crate) fn reports_list_filter(owner: &str, reporter: &str, limit: u32) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_BRIEF_REPORT],
        "authors": [reporter],
        "#p": [owner],
        "#t": [BRIEF_DISCOVERY_TAG],
        "limit": limit.min(100),
    })
}

async fn fetch_config(client: &BuzzClient, owner: Option<&str>) -> Result<Option<Event>, CliError> {
    let me = client.keys().public_key().to_hex();
    let author = match owner {
        Some(value) => {
            validate_hex64(value)?;
            value.to_ascii_lowercase()
        }
        None => me.clone(),
    };
    let raw = client.query(&config_read_filter(&author, &me)).await?;
    Ok(newest_first(parse_events(&raw)?).into_iter().next())
}

async fn fetch_own_config(client: &BuzzClient) -> Result<Option<Event>, CliError> {
    fetch_config(client, None).await
}

async fn expected_brief_reporter(client: &BuzzClient) -> Result<Option<String>, CliError> {
    let Some(event) = fetch_own_config(client).await? else {
        return Ok(None);
    };
    Ok(parse_brief_config(&event)
        .ok()
        .and_then(|config| config.reporter_pubkey)
        .map(|pubkey| pubkey.to_ascii_lowercase()))
}

/// List published briefing reports recency-first.
pub async fn cmd_list(client: &BuzzClient, limit: u32) -> Result<(), CliError> {
    let me = client.keys().public_key().to_hex();
    let reporter = match fetch_own_config(client).await? {
        Some(event) => parse_brief_config(&event)
            .ok()
            .and_then(|config| config.reporter_pubkey)
            .unwrap_or_else(|| me.clone()),
        None => me.clone(),
    };
    let filter = reports_list_filter(&me, &reporter, limit);
    let raw = client.query(&filter).await?;
    let mut rows = Vec::new();
    for event in newest_first(parse_events(&raw)?) {
        match parse_brief_report(&event) {
            Ok(report) => rows.push(serde_json::json!({
                "id": event.id.to_hex(),
                "pubkey": event.pubkey.to_hex(),
                "created_at": event.created_at.as_secs(),
                "run_id": report.run_id,
                "title": report.title,
            })),
            Err(_) => continue,
        }
    }
    println!(
        "{}",
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    );
    Ok(())
}

/// Get one report by event id or run id.
pub async fn cmd_get(
    client: &BuzzClient,
    event_id: Option<&str>,
    run_id: Option<&str>,
) -> Result<(), CliError> {
    let filter = if let Some(id) = event_id {
        validate_hex64(id)?;
        serde_json::json!({ "ids": [id.to_ascii_lowercase()], "limit": 1 })
    } else if let Some(run) = run_id {
        let uuid = parse_uuid(run)?;
        let me = client.keys().public_key().to_hex();
        let mut filter = serde_json::json!({
            "kinds": [KIND_BRIEF_REPORT],
            "#d": [uuid.to_string()],
            "#p": [me],
            "limit": 5,
        });
        if let Some(reporter) = expected_brief_reporter(client).await? {
            filter["authors"] = serde_json::json!([reporter]);
        }
        filter
    } else {
        return Err(CliError::Usage("briefs get requires --id or --run".into()));
    };
    let raw = client.query(&filter).await?;
    let Some(event) = newest_first(parse_events(&raw)?).into_iter().next() else {
        println!("null");
        return Ok(());
    };
    if let Some(reporter) = expected_brief_reporter(client).await? {
        if event.pubkey.to_hex() != reporter {
            println!("null");
            return Ok(());
        }
    }
    let report = parse_brief_report(&event).map_err(sdk_err)?;
    let output = serde_json::json!({
        "id": event.id.to_hex(),
        "pubkey": event.pubkey.to_hex(),
        "created_at": event.created_at.as_secs(),
        "report": report,
    });
    println!("{output}");
    Ok(())
}

/// Publish an immutable briefing report.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_publish(
    client: &BuzzClient,
    markdown: &str,
    title: Option<&str>,
    status_json: Option<&str>,
    owner: Option<&str>,
    since: Option<u64>,
    until: Option<u64>,
    channels: &[String],
    dms: &[String],
) -> Result<(), CliError> {
    let markdown = read_or_stdin(markdown)?;
    let owner = match owner {
        Some(value) => {
            validate_hex64(value)?;
            value.to_ascii_lowercase()
        }
        None => client.keys().public_key().to_hex(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| CliError::Other(format!("system clock: {e}")))?
        .as_secs();
    let report = BriefReport {
        v: 1,
        run_id: Uuid::new_v4().to_string(),
        title: title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Summary")
            .to_string(),
        markdown,
        status: status_json
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| CliError::Usage(format!("invalid --status-json: {e}")))?
            .unwrap_or_else(|| serde_json::json!({})),
        period: BriefPeriod {
            since: since.unwrap_or(now.saturating_sub(86_400)),
            until: until.unwrap_or(now),
        },
        sources: BriefSources {
            channels: channels.to_vec(),
            dms: dms.to_vec(),
        },
    };
    let reporter = client.keys().public_key().to_hex();
    let builder = build_brief_report(&owner, Some(&reporter), &report).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event.clone()).await?;
    let mut output: serde_json::Value = serde_json::from_str(&resp)
        .unwrap_or_else(|_| serde_json::json!({ "accepted": true, "event_id": event.id.to_hex() }));
    if let Some(object) = output.as_object_mut() {
        object.insert("run_id".into(), report.run_id.clone().into());
        object.insert("event_id".into(), event.id.to_hex().into());
    }
    println!("{output}");
    Ok(())
}

/// Print a live briefing config, or null.
///
/// Defaults to the signer's own config. Pass `owner` so a reporter can read
/// the include-set they are assigned to (`#p` = signer, `authors` = owner).
pub async fn cmd_config_get(client: &BuzzClient, owner: Option<&str>) -> Result<(), CliError> {
    match fetch_config(client, owner).await? {
        Some(event) => {
            let config = parse_brief_config(&event).map_err(sdk_err)?;
            println!(
                "{}",
                serde_json::json!({
                    "id": event.id.to_hex(),
                    "created_at": event.created_at.as_secs(),
                    "config": config,
                })
            );
        }
        None => println!("null"),
    }
    Ok(())
}

/// Create or replace the caller's live briefing config.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_config_set(
    client: &BuzzClient,
    hour: Option<u8>,
    minute: Option<u8>,
    timezone: Option<String>,
    auto_include: Option<bool>,
    reporter: Option<String>,
    channels: Vec<String>,
    dms: Vec<String>,
    exclude: Vec<String>,
) -> Result<(), CliError> {
    let existing = match fetch_own_config(client).await? {
        Some(event) => Some(parse_brief_config(&event).map_err(sdk_err)?),
        None => None,
    };
    let mut config = existing.unwrap_or(BriefConfig {
        v: 1,
        reporter_pubkey: None,
        hour: 9,
        minute: 0,
        timezone: "UTC".into(),
        auto_include: true,
        read_delegation: false,
        channels: Vec::new(),
        dms: Vec::new(),
        exclude: Vec::new(),
    });
    if let Some(hour) = hour {
        config.hour = hour;
    }
    if let Some(minute) = minute {
        config.minute = minute;
    }
    if let Some(timezone) = timezone {
        config.timezone = timezone;
    }
    if let Some(auto_include) = auto_include {
        config.auto_include = auto_include;
    }
    if let Some(reporter) = reporter {
        if reporter.is_empty() {
            config.reporter_pubkey = None;
        } else {
            validate_hex64(&reporter)?;
            config.reporter_pubkey = Some(reporter.to_ascii_lowercase());
        }
    }
    if !channels.is_empty() {
        config.channels = channels;
    }
    if !dms.is_empty() {
        config.dms = dms;
    }
    if !exclude.is_empty() {
        config.exclude = exclude;
    }
    let owner = client.keys().public_key().to_hex();
    let builder = build_brief_config(&owner, &config).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!(
        "{}",
        parse_write_response(&resp, "brief config was superseded")?
    );
    Ok(())
}

/// List the caller's watermark events.
pub async fn cmd_watermarks_list(client: &BuzzClient) -> Result<(), CliError> {
    let me = client.keys().public_key().to_hex();
    let filter = watermarks_list_filter(&me);
    let raw = client.query(&filter).await?;
    let mut rows = Vec::new();
    for event in newest_first(parse_events(&raw)?) {
        if let Ok(watermark) = parse_brief_watermark(&event) {
            rows.push(watermark);
        }
    }
    println!(
        "{}",
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    );
    Ok(())
}

/// Write a single conversation watermark.
pub async fn cmd_watermark_set(
    client: &BuzzClient,
    channel: &str,
    last_event_id: &str,
    last_created_at: u64,
    owner: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(channel)?;
    validate_hex64(last_event_id)?;
    let watermark = BriefWatermark {
        v: 1,
        channel_id: parse_uuid(channel)?.to_string(),
        last_event_id: last_event_id.to_ascii_lowercase(),
        last_created_at,
    };
    let reporter = client.keys().public_key().to_hex();
    if let Some(owner) = owner {
        validate_hex64(owner)?;
    }
    let builder = build_brief_watermark(&reporter, owner, &watermark).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!(
        "{}",
        parse_write_response(
            &resp,
            &format!("watermark {BRIEF_WATERMARK_D_PREFIX}{channel} was superseded")
        )?
    );
    Ok(())
}

pub async fn dispatch(cmd: crate::BriefsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::BriefsCmd;
    match cmd {
        BriefsCmd::Read {
            owner,
            channel,
            since,
            until,
            before_id,
            limit,
        } => {
            validate_hex64(&owner)?;
            validate_uuid(&channel)?;
            if let Some(id) = before_id.as_deref() {
                validate_hex64(id)?;
            }
            if !(1..=200).contains(&limit) {
                return Err(CliError::Usage("limit must be 1–200".into()));
            }
            let mut filter =
                serde_json::json!({"summary_owner": owner, "#h": [channel], "limit": limit});
            if let Some(value) = since {
                filter["since"] = value.into();
            }
            if let Some(value) = until {
                filter["until"] = value.into();
            }
            if let Some(value) = before_id {
                filter["before_id"] = value.into();
            }
            let raw = client.query(&filter).await?;
            let response: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| CliError::Other(e.to_string()))?;
            if response
                .get("summary_read")
                .and_then(serde_json::Value::as_u64)
                != Some(1)
            {
                return Err(CliError::Other(
                    "This relay does not support read-only Summary delegation".into(),
                ));
            }
            let events = response
                .get("events")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| CliError::Other("Invalid Summary read response".into()))?;
            println!(
                "{}",
                serde_json::to_string(events).map_err(|e| CliError::Other(e.to_string()))?
            );
            Ok(())
        }

        BriefsCmd::List { limit } => cmd_list(client, limit).await,
        BriefsCmd::Get { id, run } => cmd_get(client, id.as_deref(), run.as_deref()).await,
        BriefsCmd::Publish {
            markdown,
            title,
            status_json,
            owner,
            since,
            until,
            channels,
            dms,
        } => {
            cmd_publish(
                client,
                &markdown,
                title.as_deref(),
                status_json.as_deref(),
                owner.as_deref(),
                since,
                until,
                &channels,
                &dms,
            )
            .await
        }
        BriefsCmd::Config(cmd) => match cmd {
            crate::BriefsConfigCmd::Get { owner } => cmd_config_get(client, owner.as_deref()).await,
            crate::BriefsConfigCmd::Set {
                hour,
                minute,
                timezone,
                auto_include,
                reporter,
                channels,
                dms,
                exclude,
            } => {
                cmd_config_set(
                    client,
                    hour,
                    minute,
                    timezone,
                    auto_include,
                    reporter,
                    channels,
                    dms,
                    exclude,
                )
                .await
            }
        },
        BriefsCmd::Watermarks(cmd) => match cmd {
            crate::BriefsWatermarksCmd::List => cmd_watermarks_list(client).await,
            crate::BriefsWatermarksCmd::Set {
                channel,
                last_event,
                last_created_at,
                owner,
            } => {
                cmd_watermark_set(
                    client,
                    &channel,
                    &last_event,
                    last_created_at,
                    owner.as_deref(),
                )
                .await
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const REPORTER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn p_values(filter: &serde_json::Value) -> Vec<&str> {
        filter["#p"]
            .as_array()
            .expect("#p must be present")
            .iter()
            .map(|value| value.as_str().expect("#p values are strings"))
            .collect()
    }

    #[test]
    fn config_read_filter_pins_reader_p_tag() {
        let own = config_read_filter(OWNER, OWNER);
        assert_eq!(own["kinds"], serde_json::json!([KIND_BRIEF_CONFIG]));
        assert_eq!(own["authors"], serde_json::json!([OWNER]));
        assert_eq!(own["#d"], serde_json::json!([BRIEF_CONFIG_D_TAG]));
        assert_eq!(p_values(&own), vec![OWNER]);

        let reporter_view = config_read_filter(OWNER, REPORTER);
        assert_eq!(reporter_view["authors"], serde_json::json!([OWNER]));
        assert_eq!(p_values(&reporter_view), vec![REPORTER]);
    }

    #[test]
    fn watermarks_list_filter_pins_reader_p_tag() {
        let filter = watermarks_list_filter(REPORTER);
        assert_eq!(filter["kinds"], serde_json::json!([KIND_BRIEF_WATERMARK]));
        assert_eq!(filter["authors"], serde_json::json!([REPORTER]));
        assert_eq!(p_values(&filter), vec![REPORTER]);
    }

    #[test]
    fn reports_list_filter_pins_owner_p_tag() {
        let filter = reports_list_filter(OWNER, REPORTER, 20);
        assert_eq!(filter["kinds"], serde_json::json!([KIND_BRIEF_REPORT]));
        assert_eq!(filter["authors"], serde_json::json!([REPORTER]));
        assert_eq!(p_values(&filter), vec![OWNER]);
    }
}
