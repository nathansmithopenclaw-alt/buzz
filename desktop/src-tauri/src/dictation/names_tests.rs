use std::collections::HashSet;

use super::*;

fn ascii() -> HashSet<char> {
    ('a'..='z')
        .chain('A'..='Z')
        .chain('0'..='9')
        .chain(['\'', '-'])
        .collect()
}

fn roster(names: &[&str]) -> RosterNames {
    let names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
    RosterNames::new(&names, &ascii())
}

#[test]
fn hotwords_keep_only_names_the_model_can_spell() {
    let names = roster(&["Bolt", "Helm  Desktop", "Zoë", "🤖", "X", "Bolt", "a/b"]);
    assert_eq!(names.hotwords(), Some("Bolt/Helm Desktop"));
    assert!(names.is_name("Helm") && names.is_name("Desktop") && names.is_name("Helm Desktop"));
    assert!(!names.is_name("Zoë"));
    assert_eq!(roster(&[]).hotwords(), None);
}

#[test]
fn a_misheard_name_is_replaced_and_title_casing_is_undone() {
    let names = roster(&["Bolt", "Theron"]);
    assert_eq!(
        splice_names(
            "Boyd, can you rerun the smoke tests on main?",
            "Bolt, can you rerun the Smoke Tests on Main?",
            &names,
        ),
        "Bolt, can you rerun the smoke tests on main?",
    );
}

#[test]
fn a_matching_name_takes_the_hotword_casing() {
    let names = roster(&["Bolt"]);
    assert_eq!(
        splice_names("bolt, can you look?", "Bolt, can you look?", &names),
        "Bolt, can you look?",
    );
}

#[test]
fn plain_punctuation_survives_a_splice() {
    let names = roster(&["Theron"]);
    assert_eq!(
        splice_names(
            "They're on, what's the status?",
            "Theron what's the status?",
            &names,
        ),
        "Theron, what's the status?",
    );
}

#[test]
fn silence_never_becomes_names() {
    let names = roster(&["Quantum", "Codex"]);
    assert_eq!(
        splice_names("", "Quantum Quantum Codex Quantum", &names),
        ""
    );
}

#[test]
fn a_name_repeated_back_to_back_is_kept_once() {
    let names = roster(&["Quantum"]);
    assert_eq!(
        splice_names(
            "hey quanta, ship it.",
            "Hey Quantum Quantum, ship it.",
            &names
        ),
        "hey Quantum, ship it.",
    );
}

#[test]
fn spans_without_a_name_keep_the_plain_words() {
    let names = roster(&["Bolt"]);
    assert_eq!(
        splice_names("Ship the fix today.", "Ship The Fix Today.", &names),
        "Ship the fix today.",
    );
}

#[test]
fn a_spliced_sentence_end_capitalizes_the_next_word() {
    let names = roster(&["Bolt"]);
    assert_eq!(
        splice_names("Boyd can you look", "Bolt. can you look", &names),
        "Bolt. Can you look",
    );
}

#[test]
fn acronyms_in_a_spliced_span_keep_their_capitals() {
    let names = roster(&["Bolt"]);
    assert_eq!(
        splice_names("Boyd a pie check", "Bolt API Check", &names),
        "Bolt API check",
    );
}

/// Compares against the eval harness's reference splice (merge_names.py) on
/// the frozen holdout transcripts:
///   BUZZ_DICTATION_SPLICE_FIXTURE=<json> cargo test --lib \
///     dictation::names::tests::splice_matches_reference -- --ignored --nocapture
/// The fixture is `{"roster": [...], "rows": [{"plain", "hot", "expected"}]}`.
#[test]
#[ignore = "needs the eval fixture on disk"]
fn splice_matches_reference() {
    let path = std::env::var("BUZZ_DICTATION_SPLICE_FIXTURE").expect("fixture path");
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read fixture")).expect("json");
    let roster: Vec<String> = serde_json::from_value(fixture["roster"].clone()).expect("roster");
    let names = RosterNames::new(&roster, &ascii());
    let rows = fixture["rows"].as_array().expect("rows");
    let mut differ = 0;
    for row in rows {
        let got = splice_names(
            row["plain"].as_str().unwrap_or_default(),
            row["hot"].as_str().unwrap_or_default(),
            &names,
        );
        if got != row["expected"].as_str().unwrap_or_default() {
            differ += 1;
            if differ <= 10 {
                println!("rust: {got}\nref:  {}\n", row["expected"]);
            }
        }
    }
    println!("{differ} of {} rows differ", rows.len());
}
