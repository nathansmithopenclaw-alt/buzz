//! Presentation of the next configured Summary trigger; execution remains scheduler-owned.
use buzz_core::summary::SummaryDefinition;
use chrono::{DateTime, Utc};

/// The next planned trigger. Intervals need a durable scheduler anchor; a manual
/// run is never used as that anchor. Missed triggers are shown as due now.
pub fn next_summary_run(
    config: &SummaryDefinition,
    last_scheduled: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if config.archived || config.paused || !config.read_delegation {
        return None;
    }
    let schedule = config.schedule.as_ref()?;
    if let Some(interval) = &schedule.interval {
        let seconds = crate::executor::parse_duration_secs(interval).ok()?;
        return Some(
            (last_scheduled? + chrono::Duration::seconds(i64::try_from(seconds).ok()?)).max(now),
        );
    }
    let timezone: chrono_tz::Tz = config.timezone.parse().ok()?;
    let crons: Vec<String> = if let Some(times) = &schedule.daily_times {
        times
            .iter()
            .filter_map(|time| {
                let (hour, minute) = time.split_once(':')?;
                Some(format!("{minute} {hour} * * *"))
            })
            .collect()
    } else {
        vec![schedule.cron.clone()?]
    };
    crons
        .iter()
        .filter_map(|expression| {
            let cron: cron::Schedule =
                crate::schema::normalize_cron(&buzz_core::summary::workflow_cron(expression).ok()?)
                    .parse()
                    .ok()?;
            cron.after(&now.with_timezone(&timezone))
                .next()
                .map(|date| date.with_timezone(&Utc))
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::summary::{SummaryAudience, SummarySchedule};
    fn config(cron: &str) -> SummaryDefinition {
        SummaryDefinition {
            v: 2,
            summary_id: "default".into(),
            name: "Daily".into(),
            reporter_pubkey: "ab".repeat(32),
            audience: SummaryAudience::OnlyMe,
            read_delegation: true,
            channels: vec![],
            dms: vec![],
            exclude: vec![],
            auto_include: false,
            schedule: Some(SummarySchedule {
                cron: Some(cron.into()),
                interval: None,
                daily_times: None,
            }),
            timezone: "America/New_York".into(),
            paused: false,
            archived: false,
        }
    }
    fn time(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    #[test]
    fn next_summary_run_respects_weekday_timezone_and_archiving() {
        let mut c = config("0 9 * * 1");
        let now = time("2026-09-09T12:00:00Z");
        assert_eq!(
            next_summary_run(&c, None, now),
            Some(time("2026-09-14T13:00:00Z"))
        );
        c.archived = true;
        assert_eq!(next_summary_run(&c, None, now), None);
    }
    #[test]
    fn next_summary_run_handles_dst_and_interval_anchors() {
        let mut c = config("0 9 * * *");
        assert_eq!(
            next_summary_run(&c, None, time("2026-03-07T15:00:00Z")),
            Some(time("2026-03-08T13:00:00Z"))
        );
        assert_eq!(
            next_summary_run(&c, None, time("2026-10-31T15:00:00Z")),
            Some(time("2026-11-01T14:00:00Z"))
        );
        c.schedule = Some(SummarySchedule {
            cron: None,
            interval: Some("1h".into()),
            daily_times: None,
        });
        assert_eq!(
            next_summary_run(&c, None, time("2026-09-09T12:30:00Z")),
            None
        );
        assert_eq!(
            next_summary_run(
                &c,
                Some(time("2026-09-09T12:00:00Z")),
                time("2026-09-09T12:30:00Z")
            ),
            Some(time("2026-09-09T13:00:00Z"))
        );
    }
}
