//! Runtime identity telemetry for detecting mixed Desktop/sidecar builds.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub(crate) fn file_sha256(path: &Path) -> Option<String> {
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

pub(crate) fn cached_file_sha256(path: &Path) -> Option<String> {
    static HASHES: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();
    let hashes = HASHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = hashes.lock().ok()?;
    guard
        .entry(path.to_path_buf())
        .or_insert_with(|| file_sha256(path))
        .clone()
}

pub(crate) fn secret_fingerprint(value: &str) -> String {
    let digest = hex::encode(Sha256::digest(value.as_bytes()));
    digest[..16].to_string()
}

pub(crate) fn desktop_identity() -> Option<&'static (PathBuf, String)> {
    static IDENTITY: OnceLock<Option<(PathBuf, String)>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let path = std::env::current_exe().ok()?;
            let hash = file_sha256(&path)?;
            Some((path, hash))
        })
        .as_ref()
}

pub(crate) fn log_desktop_identity() {
    match desktop_identity() {
        Some((path, hash)) => {
            eprintln!(
                "buzz-desktop: runtime_identity path={} sha256={} version={}",
                path.display(),
                hash,
                env!("CARGO_PKG_VERSION")
            );
        }
        None => eprintln!("buzz-desktop: runtime_identity unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::{file_sha256, secret_fingerprint};

    #[test]
    fn fingerprints_are_stable_and_redacted() {
        assert_eq!(secret_fingerprint("token-value"), "e6c02a5742ea9d4d");
        assert!(!secret_fingerprint("token-value").contains("token"));
    }

    #[test]
    fn hashes_file_bytes() {
        let temp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(temp.path(), b"buzz").expect("write");
        assert_eq!(
            file_sha256(temp.path()).as_deref(),
            Some("9fff3bcb10ca5e87b8109ccde9e9452012d634a005942afc46cf2b7fa307526a")
        );
    }
}
