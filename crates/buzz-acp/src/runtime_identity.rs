//! Redacted boot telemetry for executable, credential, and model provenance.

use sha2::{Digest, Sha256};
use std::io::Read;

fn file_sha256(path: &std::path::Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hex::encode(hasher.finalize()))
}

fn secret_fingerprint(value: &str) -> String {
    let digest = hex::encode(Sha256::digest(value.as_bytes()));
    digest[..16].to_string()
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct RuntimeIdentity {
    pub broker_path: String,
    pub broker_sha256: String,
    pub desktop_path: String,
    pub desktop_sha256: String,
    pub token_source: &'static str,
    pub token_fingerprint: String,
    pub token_length: usize,
    pub token_has_whitespace: bool,
}

pub(crate) fn capture(agent_command: &str) -> RuntimeIdentity {
    let broker = std::env::current_exe().ok();
    let token = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok();
    let is_claude = agent_command.to_ascii_lowercase().contains("claude");
    let token_source = if token.is_some() {
        "environment"
    } else if is_claude {
        "credentials_file_fallback"
    } else {
        "not_applicable"
    };

    RuntimeIdentity {
        broker_path: broker
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unavailable".to_string()),
        broker_sha256: broker
            .as_deref()
            .and_then(file_sha256)
            .unwrap_or_else(|| "unavailable".to_string()),
        desktop_path: std::env::var("BUZZ_ACP_DESKTOP_PATH")
            .unwrap_or_else(|_| "unavailable".to_string()),
        desktop_sha256: std::env::var("BUZZ_ACP_DESKTOP_SHA256")
            .unwrap_or_else(|_| "unavailable".to_string()),
        token_source,
        token_fingerprint: token
            .as_deref()
            .map(secret_fingerprint)
            .unwrap_or_else(|| "unavailable".to_string()),
        token_length: token.as_deref().map(str::len).unwrap_or_default(),
        token_has_whitespace: token
            .as_deref()
            .is_some_and(|value| value.chars().any(char::is_whitespace)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fingerprint_never_contains_secret() {
        let fingerprint = super::secret_fingerprint("token-value");
        assert_eq!(fingerprint, "e6c02a5742ea9d4d");
        assert!(!fingerprint.contains("token"));
    }
}
