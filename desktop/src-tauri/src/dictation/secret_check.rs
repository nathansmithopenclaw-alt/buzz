//! Outbound secret check for dictation cleanup requests.
//!
//! Dictation sends only the transcript and roster display names to a cleanup
//! model. This check runs on that exact payload before any network call; a
//! match keeps the dictation on-device. It is deliberately broader than
//! `egress_guard` (which only blocks encrypted key backups because relay
//! pairing must carry raw `nsec`): no credential of any kind belongs in a
//! cleanup request.

use std::sync::OnceLock;

use regex::Regex;

/// A credential-shaped substring found in outbound text. Carries only the kind,
/// never the matched value, so callers can log or display it safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SecretMatch {
    pub kind: &'static str,
}

/// Number of credential patterns; a failed compile would silently drop one.
const PATTERN_COUNT: usize = 9;

struct Pattern {
    kind: &'static str,
    regex: Regex,
}

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let table: &[(&str, &str)] = &[
            ("Nostr private key", r"(?i)\bnsec1[02-9ac-hj-np-z]{20,}"),
            (
                "encrypted Nostr key",
                r"(?i)\bncryptsec1[02-9ac-hj-np-z]{20,}",
            ),
            ("Anthropic key", r"\bsk-ant-[A-Za-z0-9_\-]{16,}"),
            (
                "API key",
                r"\bsk-(?:proj-|or-v1-|live-|test-)?[A-Za-z0-9_\-]{20,}",
            ),
            (
                "GitHub token",
                r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})",
            ),
            ("AWS access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("Slack token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}"),
            ("Google API key", r"\bAIza[0-9A-Za-z_\-]{35}"),
            ("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ];
        debug_assert_eq!(table.len(), PATTERN_COUNT);
        // Static patterns; `every_pattern_compiles` pins that none is dropped.
        table
            .iter()
            .filter_map(|(kind, source)| {
                Regex::new(source).ok().map(|regex| Pattern { kind, regex })
            })
            .collect()
    })
}

/// Minimum length for a bare token to be judged key-like by entropy alone.
const KEY_LIKE_MIN_LEN: usize = 32;
/// Shannon entropy (bits per character) above which a long mixed token is
/// treated as a key. English words and hyphenated phrases sit well below.
const KEY_LIKE_MIN_ENTROPY: f64 = 3.5;

fn shannon_entropy(token: &str) -> f64 {
    let mut counts = [0usize; 256];
    let mut total = 0usize;
    for byte in token.bytes() {
        counts[byte as usize] += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}

fn is_key_like(token: &str) -> bool {
    if token.len() < KEY_LIKE_MIN_LEN {
        return false;
    }
    let charset_ok = token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-'));
    if !charset_ok {
        return false;
    }
    let has_digit = token.bytes().any(|b| b.is_ascii_digit());
    let has_alpha = token.bytes().any(|b| b.is_ascii_alphabetic());
    has_digit && has_alpha && shannon_entropy(token) >= KEY_LIKE_MIN_ENTROPY
}

/// Returns the first credential-shaped substring in `text`, if any.
pub(crate) fn find_secret(text: &str) -> Option<SecretMatch> {
    if let Some(pattern) = patterns().iter().find(|p| p.regex.is_match(text)) {
        return Some(SecretMatch { kind: pattern.kind });
    }
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\'' | '(' | ')'))
        .map(|token| token.trim_end_matches(['.', ':', '!', '?']))
        .find(|token| is_key_like(token))
        .map(|_| SecretMatch {
            kind: "key-like string",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert_eq!(patterns().len(), PATTERN_COUNT);
    }

    #[test]
    fn detects_each_credential_shape() {
        let cases = [
            ("my key is nsec1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0jqfj5v5x", "Nostr private key"),
            ("ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p", "encrypted Nostr key"),
            ("use sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123 for this", "Anthropic key"),
            ("openrouter sk-or-v1-0123456789abcdef0123456789abcdef", "API key"),
            ("token ghp_abcdefghijklmnopqrstuvwxyz0123456789", "GitHub token"),
            ("aws AKIAIOSFODNN7EXAMPLE here", "AWS access key"),
            ("slack xoxb-1234567890-abcdefghij", "Slack token"),
            ("google AIzaSyA-1234567890abcdefghijklmnopqrstu", "Google API key"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "private key"),
            ("secret is 8f3kQ9zLw2xR7vN4bT6yH1mJ5cP0dS8aE3gU", "key-like string"),
        ];
        for (text, kind) in cases {
            assert_eq!(find_secret(text), Some(SecretMatch { kind }), "{text}");
        }
    }

    #[test]
    fn ordinary_dictation_passes() {
        for text in [
            "@[Anvil], can you review the patch Codex sent this morning?",
            "Let's ship the internationalization-accessibility-improvements branch today.",
            "The commit is abc123 and the ticket is SMI-310.",
            "Skip the sk- prefix discussion, it's not relevant.",
            "Ask Helm to check whether Titus finished the B2BD deploy on the DGX Spark.",
            "Antidisestablishmentarianism counterrevolutionaries notwithstanding.",
        ] {
            assert_eq!(find_secret(text), None, "{text}");
        }
    }

    #[test]
    fn long_tokens_need_digits_letters_and_entropy() {
        assert!(!is_key_like("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1"));
        assert!(!is_key_like("abcdefghijklmnopqrstuvwxyzabcdefghij"));
        assert!(is_key_like("Zx8Qp2Lr9Tk4Wm7Nb3Vc6Hy1Fg5Dj0Ks8Ae2"));
    }
}
