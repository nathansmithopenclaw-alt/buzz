//! Channel-member names for dictation: sherpa-onnx hotwords and the splice
//! that moves those names into the plain transcript.
//!
//! Each chunk is decoded twice from the same audio. The plain decode keeps
//! clean wording and casing. The hotword decode spells roster names far more
//! reliably but also title-cases ordinary words near them ("rerun the Smoke
//! Tests on Main"). [`splice_names`] aligns the two word sequences and takes
//! the hotword words only where they carry a roster name.

use std::collections::HashSet;

/// Most names passed as hotwords (the channel roster is usually far smaller).
const MAX_HOTWORDS: usize = 300;
const MAX_NAME_CHARS: usize = 40;

/// The roster names the recognizer can bias toward, and the name tokens the
/// splice recognizes.
#[derive(Debug, Default, Clone)]
pub(crate) struct RosterNames {
    /// sherpa-onnx per-stream hotwords: names separated by `/`.
    hotwords: String,
    /// Full names and each of their words, case-sensitive.
    tokens: HashSet<String>,
}

impl RosterNames {
    /// Keeps names the model vocabulary can spell. `encodable` holds every
    /// character that appears in the model's tokens; a name with any other
    /// character would make sherpa-onnx drop the whole hotword list.
    pub(crate) fn new(roster: &[String], encodable: &HashSet<char>) -> Self {
        let mut names: Vec<String> = Vec::new();
        for name in roster {
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            let usable = (2..=MAX_NAME_CHARS).contains(&name.chars().count())
                && name.chars().any(char::is_alphabetic)
                && name.chars().all(|c| c == ' ' || encodable.contains(&c));
            if usable && !names.contains(&name) {
                names.push(name);
            }
            if names.len() == MAX_HOTWORDS {
                break;
            }
        }
        let tokens = names
            .iter()
            .flat_map(|name| name.split(' ').map(str::to_string))
            .chain(names.iter().cloned())
            .collect();
        Self {
            hotwords: names.join("/"),
            tokens,
        }
    }

    /// `None` when no name survived, so the caller skips the hotword pass.
    pub(crate) fn hotwords(&self) -> Option<&str> {
        (!self.hotwords.is_empty()).then_some(self.hotwords.as_str())
    }

    fn is_name(&self, word: &str) -> bool {
        self.tokens.contains(word)
    }
}

/// Letters, digits and apostrophes of a word (punctuation stripped).
fn bare(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .collect()
}

fn ends_sentence(word: &str) -> bool {
    word.ends_with(['.', '!', '?'])
}

fn lowercase_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn uppercase_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// One aligned region: `a[a_from..a_to]` against `b[b_from..b_to]`.
#[derive(Debug, PartialEq, Eq)]
struct Region {
    equal: bool,
    a_from: usize,
    a_to: usize,
    b_from: usize,
    b_to: usize,
}

/// Aligns two word sequences by longest common subsequence and groups the
/// result into alternating equal and differing regions.
fn align(a: &[String], b: &[String]) -> Vec<Region> {
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = LCS length of a[i..] and b[j..].
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut regions: Vec<Region> = Vec::new();
    let mut push = |equal: bool, i: usize, j: usize, di: usize, dj: usize| match regions.last_mut()
    {
        Some(last) if last.equal == equal => {
            last.a_to += di;
            last.b_to += dj;
        }
        _ => regions.push(Region {
            equal,
            a_from: i,
            a_to: i + di,
            b_from: j,
            b_to: j + dj,
        }),
    };
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            push(true, i, j, 1, 1);
            i += 1;
            j += 1;
        } else if j == m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1]) {
            push(false, i, j, 1, 0);
            i += 1;
        } else {
            push(false, i, j, 0, 1);
            j += 1;
        }
    }
    regions
}

/// The plain transcript with roster names taken from the hotword transcript.
///
/// - Where the two agree up to case, a roster name keeps the hotword casing.
/// - Where they differ and the hotword side holds a roster name, the hotword
///   words replace the plain ones; ordinary words it title-cased mid-sentence
///   are lowercased again, and a name repeated back to back is kept once.
/// - An empty plain decode stays empty: on silence or hum the hotword decode
///   can emit nothing but roster names.
pub(crate) fn splice_names(plain: &str, hot: &str, names: &RosterNames) -> String {
    let a: Vec<&str> = plain.split_whitespace().collect();
    let b: Vec<&str> = hot.split_whitespace().collect();
    if a.is_empty() || b.is_empty() {
        return a.join(" ");
    }
    let key =
        |words: &[&str]| -> Vec<String> { words.iter().map(|w| bare(w).to_lowercase()).collect() };
    let mut out: Vec<String> = Vec::with_capacity(a.len());
    for region in align(&key(&a), &key(&b)) {
        let plain_span = &a[region.a_from..region.a_to];
        let hot_span = &b[region.b_from..region.b_to];
        if region.equal {
            for (word, hot_word) in plain_span.iter().zip(hot_span) {
                let name = bare(hot_word);
                if names.is_name(&name) {
                    out.push(word.replacen(&bare(word), &name, 1));
                } else {
                    out.push((*word).to_string());
                }
            }
            continue;
        }
        if !hot_span.iter().any(|w| names.is_name(&bare(w))) {
            out.extend(plain_span.iter().map(|w| (*w).to_string()));
            continue;
        }
        let mut previous_name: Option<String> = None;
        for word in hot_span {
            let word_bare = bare(word);
            let is_name = names.is_name(&word_bare);
            if is_name && previous_name.as_deref() == Some(word_bare.as_str()) {
                continue;
            }
            previous_name = is_name.then(|| word_bare.clone());
            let sentence_start = out.last().is_none_or(|w| ends_sentence(w));
            // "Smoke" but not "API": acronyms keep their capitals.
            let title_cased = word_bare.chars().next().is_some_and(char::is_uppercase)
                && word_bare.chars().any(char::is_lowercase);
            if !is_name && !sentence_start && title_cased {
                out.push(lowercase_first(word));
            } else {
                out.push((*word).to_string());
            }
        }
        // Keep the plain side's closing punctuation if the hotword side lost it.
        if let (Some(last_plain), Some(last)) = (plain_span.last(), out.last_mut()) {
            if let Some(mark) = last_plain.chars().last().filter(|c| ",.!?:".contains(*c)) {
                if !last.ends_with([',', '.', '!', '?', ':']) {
                    last.push(mark);
                }
            }
        }
    }
    // A spliced span can end a sentence ("Bolt." for "Bolt,"): capitalize
    // what follows.
    for k in 1..out.len() {
        if ends_sentence(&out[k - 1]) && out[k].chars().next().is_some_and(char::is_lowercase) {
            out[k] = uppercase_first(&out[k]);
        }
    }
    out.join(" ")
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
