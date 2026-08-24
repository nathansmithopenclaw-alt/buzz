//! Owned DeepSeek Harness (`dsh`) composition provisioning.
//!
//! The `deepseek` runtime runs `dsh --profile buzz-acp`. The profile tree —
//! package manifest, Cordis patch, and the owned buzz-publish plugin — ships
//! embedded in this module and is materialized into the nest
//! (`<agent workdir>/.dsh-home/profiles/buzz-acp/`) on every dsh spawn,
//! write-if-changed. This keeps the harness's sessions, compaction state, and
//! composition inside the Buzz workspace instead of a second home-level source
//! of truth (mirrors the Prime `.prime/agent` redirect), and makes the owned
//! publish seam a pinned, reviewed artifact rather than operator setup.
//!
//! Credentials never pass through here: the plugin reads them from the dsh
//! process environment at execute time; nothing in the provisioned tree holds
//! or fetches secret values.

use std::fs;
use std::path::Path;

/// Embedded files, as `(path relative to the profile root, contents)`.
const PROFILE_FILES: &[(&str, &str)] = &[
    (
        "package.json",
        include_str!("dsh_profile_resources/package.json"),
    ),
    (
        "cordis.patch.yml",
        include_str!("dsh_profile_resources/cordis.patch.yml"),
    ),
    (
        "packages/dsh-buzz-publish/package.json",
        include_str!("dsh_profile_resources/packages/dsh-buzz-publish/package.json"),
    ),
    (
        "packages/dsh-buzz-publish/index.js",
        include_str!("dsh_profile_resources/packages/dsh-buzz-publish/index.js"),
    ),
];

/// Write every embedded profile file under `<dsh_home>/profiles/buzz-acp/`,
/// skipping files whose on-disk bytes already match. Idempotent; safe to run
/// on every spawn.
///
/// Returns the profile directory on success so callers can derive `DSH_HOME`.
pub(crate) fn provision(dsh_home: &Path) -> std::io::Result<std::path::PathBuf> {
    let profile_dir = dsh_home.join("profiles").join("buzz-acp");
    for (relative, contents) in PROFILE_FILES {
        let path = profile_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_if_changed(&path, contents)?;
    }
    Ok(profile_dir)
}

/// Write only when the existing file differs, so spawns never touch mtimes of
/// unchanged content (keeps backup/rsync noise down and keeps the operation
/// cheap on the hot spawn path).
fn write_if_changed(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == contents {
            return Ok(());
        }
    }
    fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisions_all_files_and_is_idempotent() {
        let root =
            std::env::temp_dir().join(format!("buzz-dsh-profile-test-{}", std::process::id()));
        let dsh_home = root.join(".dsh-home");
        let profile = provision(&dsh_home).expect("provision succeeds");
        assert_eq!(profile, dsh_home.join("profiles").join("buzz-acp"));
        for (relative, contents) in PROFILE_FILES {
            let written = fs::read_to_string(profile.join(relative)).expect("file written");
            assert_eq!(written, *contents, "content mismatch for {relative}");
        }

        // Second run is a no-op that leaves identical content.
        provision(&dsh_home).expect("re-provision succeeds");
        for (relative, contents) in PROFILE_FILES {
            let written = fs::read_to_string(profile.join(relative)).expect("file still present");
            assert_eq!(written, *contents);
        }

        // Diverged on-disk content is repaired to the embedded truth.
        fs::write(profile.join("cordis.patch.yml"), "tampered").unwrap();
        provision(&dsh_home).expect("repair succeeds");
        let repaired = fs::read_to_string(profile.join("cordis.patch.yml")).unwrap();
        assert_eq!(repaired, PROFILE_FILES[1].1);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn plugin_resource_never_contains_credential_names() {
        // Guard the no-creds-on-disk contract at the resource level: the owned
        // plugin may NAME the env keys it forwards but must not embed any
        // credential VALUE, key material pattern, or file-based fallback.
        let plugin = PROFILE_FILES[3].1;
        assert!(
            plugin.contains("process.env.BUZZ_PRIVATE_KEY"),
            "plugin reads creds from process env"
        );
        assert!(
            !plugin.contains("readFileSync"),
            "plugin must not read files"
        );
        assert!(
            !plugin.contains("writeFileSync"),
            "plugin must not write files"
        );
        assert!(
            !plugin.contains("sk-"),
            "no embedded API-key-looking literal"
        );
    }
}
