use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub(crate) const TOO_OLD_ERROR_CODE: i64 = -32550;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(value: &str) -> Option<Self> {
        let token = value.trim().split_whitespace().next()?;
        let mut parts = token.split('.');
        let version = Self(
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    path: PathBuf,
    modified_nanos: u128,
    size: u64,
}

fn version_cache() -> &'static Mutex<HashMap<CacheKey, Option<Version>>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Option<Version>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn resolved_claude_cli() -> Option<PathBuf> {
    super::known_acp_runtime("claude-agent-acp")
        .and_then(|runtime| runtime.underlying_cli)
        .and_then(super::resolve_command)
}

fn cache_key(path: &Path) -> Result<CacheKey, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok(CacheKey {
        path: path.to_path_buf(),
        modified_nanos,
        size: metadata.len(),
    })
}

fn installed_version(path: &Path) -> Result<Option<Version>, String> {
    let key = cache_key(path)?;
    if let Some(cached) = version_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .copied()
    {
        return Ok(cached);
    }
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("run {} --version: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version exited {}",
            path.display(),
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = Version::parse(&stdout);
    if parsed.is_none() {
        eprintln!(
            "buzz-desktop: warning: could not parse Claude Code version from {:?}; allowing launch",
            stdout.trim()
        );
    }
    version_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, parsed);
    Ok(parsed)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCliBlocker {
    pub code: i64,
    pub model: String,
    pub installed: String,
    pub required: String,
    pub message: String,
    pub binary_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCliUpdateResult {
    pub binary_path: String,
    pub installed: Option<String>,
    pub output_tail: String,
    pub restarted_agents: usize,
    pub restart_failures: Vec<String>,
}

fn update_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn output_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().rev().take(20).collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n")
}

const UPDATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

fn update_path(prefix_bin: &Path) -> Result<std::ffi::OsString, String> {
    let nvm_bin = dirs::home_dir()
        .as_deref()
        .and_then(super::find_nvm_default_bin);
    let augmented = super::runtime::build_augmented_path(
        dirs::home_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        super::login_shell_path(),
        nvm_bin,
    );
    let mut entries = vec![prefix_bin.to_path_buf()];
    if let Some(augmented) = augmented {
        entries.extend(std::env::split_paths(&augmented));
    }
    entries.dedup();
    std::env::join_paths(entries).map_err(|error| format!("build Claude Code update PATH: {error}"))
}

fn run_update_command(command: Command, timeout: Duration) -> Result<std::process::Output, String> {
    super::output_with_timeout(command, timeout).ok_or_else(|| {
        format!(
            "Claude Code update was stopped after failing to complete within {} or exceeding safe output limits",
            if timeout.as_secs() >= 60 {
                format!("{} minutes", timeout.as_secs() / 60)
            } else {
                format!("{}ms", timeout.as_millis())
            }
        )
    })
}

fn update_binary_with_timeout(
    path: &Path,
    timeout: Duration,
) -> Result<ClaudeCliUpdateResult, String> {
    let _guard = update_lock()
        .try_lock()
        .map_err(|_| "A Claude Code update is already running".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("cannot determine install for {}", path.display()))?;
    let npm = parent.join(if cfg!(windows) { "npm.cmd" } else { "npm" });
    let (program, args): (&Path, Vec<&str>) = if npm.is_file() {
        (&npm, vec!["i", "-g", "@anthropic-ai/claude-code@latest"])
    } else if path.is_file() {
        (path, vec!["update"])
    } else {
        return Err(format!(
            "Unknown Claude Code install type for {}",
            path.display()
        ));
    };
    let path_env = update_path(parent)?;
    let mut command = Command::new(program);
    command.args(args).env("PATH", path_env);
    let output = run_update_command(command, timeout)?;
    let stderr_tail = output_tail(&output.stderr);
    let stdout_tail = output_tail(&output.stdout);
    let combined = [stdout_tail, stderr_tail]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !output.status.success() {
        return Err(format!(
            "Claude Code update failed ({}): {}",
            output.status, combined
        ));
    }
    version_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|key, _| key.path != path);
    super::clear_resolve_cache();
    let installed = installed_version(path)?.map(|version| version.to_string());
    Ok(ClaudeCliUpdateResult {
        binary_path: path.display().to_string(),
        installed,
        output_tail: combined,
        restarted_agents: 0,
        restart_failures: Vec::new(),
    })
}

fn update_binary(path: &Path) -> Result<ClaudeCliUpdateResult, String> {
    update_binary_with_timeout(path, UPDATE_TIMEOUT)
}

pub(crate) fn update_resolved_binary() -> Result<ClaudeCliUpdateResult, String> {
    let path =
        resolved_claude_cli().ok_or_else(|| "Claude Code executable was not found".to_string())?;
    update_binary(&path)
}

pub(crate) fn blocker_for_model(model: Option<&str>) -> Option<ClaudeCliBlocker> {
    let model = model?.trim();
    let required_text = super::claude_model_catalog::minimum_cli_version(model)?;
    let required = Version::parse(required_text)?;
    let path = resolved_claude_cli()?;
    let installed = match installed_version(&path) {
        Ok(Some(version)) => version,
        Ok(None) | Err(_) => return None,
    };
    (installed < required).then(|| ClaudeCliBlocker {
        code: TOO_OLD_ERROR_CODE,
        model: model.to_string(),
        installed: installed.to_string(),
        required: required.to_string(),
        message: format!(
            "Claude Opus 5.5 needs Claude Code {required} or newer. This Mac has {installed}."
        ),
        binary_path: path.display().to_string(),
    })
}

pub(crate) fn blocker_for_record(
    record: &super::ManagedAgentRecord,
    personas: &[super::AgentDefinition],
    global: &super::GlobalAgentConfig,
) -> Result<Option<ClaudeCliBlocker>, String> {
    if record.backend != super::BackendKind::Local {
        return Ok(None);
    }
    let descriptor = super::resolve_effective_harness_descriptor(record, personas, global)
        .map_err(|error| super::user_facing_harness_error(&error))?;
    if !super::known_acp_runtime(&descriptor.command).is_some_and(|runtime| runtime.id == "claude")
    {
        return Ok(None);
    }
    let effective = super::effective_config::resolve_effective_config(record, personas, global)
        .require_resolved()?;
    Ok(blocker_for_model(effective.model.value.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn update_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    #[test]
    fn parses_and_compares_semantic_versions() {
        assert_eq!(
            Version::parse("2.1.280 (Claude Code)"),
            Some(Version(2, 1, 280))
        );
        assert!(Version::parse("2.1.257").unwrap() < Version::parse("2.1.280").unwrap());
        assert!(Version::parse("not-a-version").is_none());
    }

    #[test]
    fn cache_invalidates_when_binary_metadata_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(&path, "#!/bin/sh\necho '2.1.257 (Claude Code)'\n").unwrap();
        let mut permissions = path.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(installed_version(&path).unwrap(), Some(Version(2, 1, 257)));
        std::fs::write(
            &path,
            "#!/bin/sh\necho '2.1.280 (Claude Code)'\n# changed\n",
        )
        .unwrap();
        assert_eq!(installed_version(&path).unwrap(), Some(Version(2, 1, 280)));
    }

    #[test]
    fn npm_prefix_update_rechecks_the_changed_binary() {
        let _guard = update_test_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let claude = bin.join("claude");
        let npm = bin.join("npm");
        std::fs::write(&claude, "#!/bin/sh\necho '2.1.257 (Claude Code)'\n").unwrap();
        std::fs::write(&npm, format!("#!/bin/sh\nprintf '%s\\n' \"#!/bin/sh\" \"echo '2.1.280 (Claude Code)'\" > '{}'\nchmod +x '{}'\necho updated\n", claude.display(), claude.display())).unwrap();
        for path in [&claude, &npm] {
            let mut permissions = path.metadata().unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        assert_eq!(
            installed_version(&claude).unwrap(),
            Some(Version(2, 1, 257))
        );
        let result = update_binary(&claude).unwrap();
        assert_eq!(result.installed.as_deref(), Some("2.1.280"));
        assert!(result.output_tail.contains("updated"));
    }

    #[cfg(unix)]
    #[test]
    fn npm_prefix_update_finds_its_node_with_minimal_process_path() {
        let _guard = update_test_lock();
        const CHILD_MARKER: &str = "BUZZ_TEST_CLAUDE_UPDATE_MINIMAL_PATH_CHILD";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let test_name = std::thread::current()
                .name()
                .expect("libtest thread name")
                .to_string();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &test_name, "--nocapture"])
                .env(CHILD_MARKER, "1")
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "minimal-PATH subprocess failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let claude = bin.join("claude");
        let npm = bin.join("npm");
        let fake_node = bin.join("buzz-test-prefix-node");
        std::fs::write(&claude, "#!/bin/sh\necho '2.1.257 (Claude Code)'\n").unwrap();
        std::fs::write(
            &npm,
            "#!/usr/bin/env buzz-test-prefix-node\n# npm fixture body is consumed by the fake interpreter\n",
        )
        .unwrap();
        std::fs::write(
            &fake_node,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"#!/bin/sh\" \"echo '2.1.280 (Claude Code)'\" > '{}'\nchmod +x '{}'\necho updated-via-prefix-node\n",
                claude.display(),
                claude.display()
            ),
        )
        .unwrap();
        for path in [&claude, &npm, &fake_node] {
            let mut permissions = path.metadata().unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }

        let result = update_binary(&claude).unwrap();
        assert_eq!(result.installed.as_deref(), Some("2.1.280"));
        assert!(result.output_tail.contains("updated-via-prefix-node"));
    }

    #[cfg(unix)]
    #[test]
    fn update_timeout_kills_child_and_releases_single_flight_lock() {
        let _guard = update_test_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let claude = bin.join("claude");
        let npm = bin.join("npm");
        std::fs::write(&claude, "#!/bin/sh\necho '2.1.257 (Claude Code)'\n").unwrap();
        std::fs::write(&npm, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        for path in [&claude, &npm] {
            let mut permissions = path.metadata().unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }

        let started = std::time::Instant::now();
        let error = update_binary_with_timeout(&claude, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("within 100ms"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));

        std::fs::write(
            &npm,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"#!/bin/sh\" \"echo '2.1.280 (Claude Code)'\" > '{}'\nchmod +x '{}'\n",
                claude.display(),
                claude.display()
            ),
        )
        .unwrap();
        let result = update_binary_with_timeout(&claude, Duration::from_secs(2)).unwrap();
        assert_eq!(result.installed.as_deref(), Some("2.1.280"));
    }
}
