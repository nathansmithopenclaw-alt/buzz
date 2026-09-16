use super::*;

fn definition() -> SummaryDefinition {
    SummaryDefinition {
        v: 2,
        summary_id: Uuid::new_v4().to_string(),
        name: "Engineering".into(),
        reporter_pubkey: "d".repeat(64),
        audience: SummaryAudience::OnlyMe,
        read_delegation: true,
        channels: vec![Uuid::new_v4()],
        dms: vec![],
        exclude: vec![],
        auto_include: false,
        schedule: None,
        timezone: "America/New_York".into(),
        paused: false,
        archived: false,
    }
}

#[test]
fn three_people_and_two_agents_have_distinct_audience_rights() {
    let owner = "a".repeat(64);
    let humans: BTreeSet<_> = [owner.clone(), "b".repeat(64), "c".repeat(64)].into();
    let mut config = definition();
    assert_eq!(
        config.recipients(&owner, &humans).unwrap(),
        [owner.clone()].into()
    );
    config.audience = SummaryAudience::Selected {
        people: vec!["b".repeat(64)],
    };
    let selected = config.recipients(&owner, &humans).unwrap();
    assert!(selected.contains(&owner));
    assert!(selected.contains(&"b".repeat(64)));
    for other in ["c", "d", "e"] {
        assert!(!selected.contains(&other.repeat(64)));
    }
    config.audience = SummaryAudience::Selected {
        people: vec!["e".repeat(64)],
    };
    assert!(config.recipients(&owner, &humans).is_err());
    config.audience = SummaryAudience::Community;
    assert_eq!(config.recipients(&owner, &humans).unwrap(), humans);
}

#[test]
fn audience_changes_do_not_share_history_or_preserve_revoked_access() {
    let snapshot = ["a".repeat(64), "b".repeat(64)].into();
    let current = ["a".repeat(64), "c".repeat(64)].into();
    assert!(reader_in_audience(&"a".repeat(64), &snapshot, &current));
    assert!(!reader_in_audience(&"b".repeat(64), &snapshot, &current));
    assert!(!reader_in_audience(&"c".repeat(64), &snapshot, &current));
}

#[test]
fn cursor_identity_is_scoped_to_owner_summary_and_source() {
    let room = Uuid::new_v4();
    let first = Uuid::new_v4().to_string();
    let second = Uuid::new_v4().to_string();
    let base = watermark_key(&"a".repeat(64), &first, room).unwrap();
    assert_ne!(base, watermark_key(&"a".repeat(64), &second, room).unwrap());
    assert_ne!(base, watermark_key(&"b".repeat(64), &first, room).unwrap());
    assert_ne!(
        base,
        watermark_key(&"a".repeat(64), &first, Uuid::new_v4()).unwrap()
    );
    assert_eq!(config_d_tag(LEGACY_SUMMARY_ID).unwrap(), "summary");
    assert!(config_d_tag("summary:someone-else").is_err());
}

#[test]
fn malformed_definitions_fail_closed() {
    let original = definition();
    original.validate().unwrap();
    let mut invalid = original.clone();
    invalid.dms = invalid.channels.clone();
    assert!(invalid.validate().is_err());
    invalid = original.clone();
    invalid.exclude = invalid.channels.clone();
    assert!(invalid.validate().is_err());
    invalid = original.clone();
    invalid.audience = SummaryAudience::Selected { people: vec![] };
    assert!(invalid.validate().is_err());
    invalid = original;
    invalid.schedule = Some(SummarySchedule {
        daily_times: None,
        cron: Some("0 9 * * *".into()),
        interval: Some("1h".into()),
    });
    assert!(invalid.validate().is_err());
}

#[test]
fn unix_summary_weekdays_match_human_calendar_days() {
    for (expression, expected) in [
        ("0 9 * * 1", "0 9 * * MON"),
        ("0 9 * * 0,7", "0 9 * * SUN"),
        ("0 9 * * 1-5", "0 9 * * MON,TUE,WED,THU,FRI"),
        ("0 9 * * */2", "0 9 * * SUN,TUE,THU,SAT"),
        ("0 9 * * 1-7/2", "0 9 * * MON,WED,FRI,SUN"),
        ("0 9 * * mon-fri", "0 9 * * MON-FRI"),
        ("0 0 9 * * 1", "0 0 9 * * 1"),
    ] {
        assert_eq!(workflow_cron(expression).unwrap(), expected);
    }
    for invalid in ["0 9 * * 8", "0 9 * * 1/0", "0 9 * * 7-1"] {
        assert!(workflow_cron(invalid).is_err());
    }
}

#[test]
fn summary_names_count_characters_and_exclusions_do_not_consume_source_slots() {
    let mut config = definition();
    config.name = "界".repeat(200);
    config.exclude = (0..300).map(|_| Uuid::new_v4()).collect();
    config.validate().unwrap();
    config.name.push('界');
    assert!(config.validate().is_err());
}
