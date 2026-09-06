//! Private provider-session handoff for an explicit Desktop account switch.
//! An unacknowledged cancellation can never be restored as a safe continuation.
use crate::{queue::FlushBatch, scope::SessionScope};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Session {
    version: u32,
    session_id: String,
    safe: bool,
    #[serde(default)]
    unfinished: bool,
}

fn token_valid(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|c| c.is_ascii_hexdigit())
}

fn directory(token: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(token_valid(token), "Invalid account handoff identity");
    let root = std::env::var_os("BUZZ_ACP_AUTH_RECOVERY_DIR")
        .ok_or_else(|| anyhow::anyhow!("Account handoff storage unavailable"))?;
    let root = PathBuf::from(root).join(format!("switch-{token}"));
    for path in root.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(path) {
            anyhow::ensure!(!meta.file_type().is_symlink(), "Unsafe handoff directory");
        }
    }
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    Ok(root)
}

fn path(root: &Path, scope: &SessionScope) -> PathBuf {
    use sha2::{Digest, Sha256};
    let key = format!(
        "{}:{}",
        scope.channel_id(),
        scope.root_event_id().unwrap_or("")
    );
    root.join(format!("{}.json", hex::encode(Sha256::digest(key))))
}

fn save_at(root: &Path, scope: &SessionScope, session: &Session) -> anyhow::Result<()> {
    anyhow::ensure!(
        session.session_id.len() <= 512,
        "Invalid provider session reference"
    );
    let target = path(root, scope);
    let tmp = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&serde_json::to_vec(session)?)?;
    file.sync_all()?;
    fs::rename(tmp, target)?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

pub(crate) fn save(token: &str, scope: &SessionScope, id: &str, safe: bool) -> anyhow::Result<()> {
    save_at(
        &directory(token)?,
        scope,
        &Session {
            version: 1,
            session_id: id.into(),
            safe,
            unfinished: true,
        },
    )
}

fn record_at(root: &Path, scope: &SessionScope) -> anyhow::Result<Option<Session>> {
    let path = path(root, scope);
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= 2048,
        "Invalid account handoff file"
    );
    let mut bytes = Vec::new();
    file.take(2049).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 2048, "Invalid account handoff file");
    let session: Session = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        session.version == 1 && session.safe && session.session_id.len() <= 512,
        "The interrupted tool outcome is unconfirmed; work remains paused"
    );
    Ok(Some(session))
}
fn load_at(root: &Path, scope: &SessionScope) -> anyhow::Result<Option<String>> {
    Ok(record_at(root, scope)?.map(|record| record.session_id))
}

pub(crate) fn load(scope: &SessionScope) -> anyhow::Result<Option<String>> {
    let Ok(token) = std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_ID") else {
        return Ok(None);
    };
    Ok(load_at(&directory(&token)?, scope)?.filter(|id| !id.is_empty()))
}

/// Capture an idle session without turning an unconfirmed cancellation into success.
pub(crate) fn save_idle(token: &str, scope: &SessionScope, id: &str) -> anyhow::Result<()> {
    let root = directory(token)?;
    let absent = !path(&root, scope).try_exists()?;
    let empty =
        !id.is_empty() && matches!(load_at(&root, scope), Ok(Some(saved)) if saved.is_empty());
    if absent || empty {
        save_at(
            &root,
            scope,
            &Session {
                version: 1,
                session_id: id.into(),
                safe: true,
                unfinished: false,
            },
        )?;
    }
    Ok(())
}

pub(crate) fn unfinished(scope: &SessionScope) -> anyhow::Result<bool> {
    let Ok(token) = std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_ID") else {
        return Ok(false);
    };
    Ok(record_at(&directory(&token)?, scope)?.is_some_and(|record| record.unfinished))
}

pub(crate) fn inherit_if_missing(token: &str, scope: &SessionScope) -> anyhow::Result<()> {
    let root = directory(token)?;
    match load_at(&root, scope) {
        Ok(Some(id)) if id.is_empty() => {}
        Ok(None) => {}
        _ => return Ok(()), // Never overwrite an unconfirmed cancellation.
    }
    let Ok(previous) = std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_ID") else {
        return Ok(());
    };
    if let Some(record) = record_at(&directory(&previous)?, scope)? {
        save_at(&root, scope, &record)?;
    }
    Ok(())
}

pub(crate) fn completed_during_pause(
    token: &str,
    scope: &SessionScope,
    id: &str,
) -> anyhow::Result<()> {
    save_at(
        &directory(token)?,
        scope,
        &Session {
            version: 1,
            session_id: id.into(),
            safe: true,
            unfinished: false,
        },
    )
}

pub(crate) fn validate(token: &str, batches: &[FlushBatch]) -> anyhow::Result<()> {
    let root = directory(token)?;
    for batch in batches {
        anyhow::ensure!(
            load_at(&root, &batch.scope)?.is_some(),
            "The interrupted session was not saved; work remains paused"
        );
    }
    Ok(())
}

pub(crate) fn complete(scope: &SessionScope) -> anyhow::Result<()> {
    let Ok(token) = std::env::var("BUZZ_ACP_ACCOUNT_SWITCH_ID") else {
        return Ok(());
    };
    let root = directory(&token)?;
    match fs::remove_file(path(&root, scope)) {
        Ok(()) => {
            fs::File::open(root)?.sync_all()?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_confirmed_exact_scope_can_resume() {
        let root = tempfile::tempdir().unwrap();
        let scope = SessionScope::Conversation {
            channel_id: uuid::Uuid::new_v4(),
        };
        let mut saved = Session {
            version: 1,
            session_id: "provider-session".into(),
            safe: false,
            unfinished: true,
        };
        save_at(root.path(), &scope, &saved).unwrap();
        assert!(load_at(root.path(), &scope).is_err());
        saved.safe = true;
        save_at(root.path(), &scope, &saved).unwrap();
        assert_eq!(
            load_at(root.path(), &scope).unwrap().as_deref(),
            Some("provider-session")
        );
        let sibling = SessionScope::Thread {
            channel_id: scope.channel_id(),
            root_event_id: "a".repeat(64),
        };
        assert!(load_at(root.path(), &sibling).unwrap().is_none());
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[test]
    fn corrupt_and_oversized_references_never_become_fresh_sessions() {
        let root = tempfile::tempdir().unwrap();
        let scope = SessionScope::Conversation {
            channel_id: uuid::Uuid::new_v4(),
        };
        let file = path(root.path(), &scope);
        for bytes in [b"broken".to_vec(), vec![b'x'; 2049]] {
            fs::write(&file, bytes).unwrap();
            assert!(load_at(root.path(), &scope).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn session_reference_cannot_follow_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let scope = SessionScope::Conversation {
            channel_id: uuid::Uuid::new_v4(),
        };
        let other = root.path().join("other");
        fs::write(&other, r#"{"version":1,"session_id":"other","safe":true}"#).unwrap();
        std::os::unix::fs::symlink(&other, path(root.path(), &scope)).unwrap();
        assert!(load_at(root.path(), &scope).is_err());
    }
}
