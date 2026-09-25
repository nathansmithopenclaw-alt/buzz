//! Codex 0.144 project-layer boundary for connection agents.
//!
//! Its generated home pins `project_root_markers = [".git"]`. Native discovery
//! finds the nearest marker, or uses cwd itself when none exists, then reads
//! .codex/config.toml from that root through cwd. Do not walk to HOME when the
//! working directory is not a repository: that would treat user config as a
//! project layer. This check is repeated on every launch, never cached at home
//! generation time.
use std::{io::Read, path::Path};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

fn display(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn validate(cwd: &Path) -> Result<(), String> {
    let cwd = std::fs::canonicalize(cwd)
        .map_err(|_| format!("Cannot inspect Codex workspace {}", display(cwd)))?;
    let mut root = cwd.as_path();
    for ancestor in cwd.ancestors() {
        let marker = ancestor.join(".git");
        match std::fs::metadata(&marker) {
            Ok(_) => {
                root = ancestor;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(format!(
                    "Cannot inspect Codex project root {}",
                    display(&marker)
                ))
            }
        }
    }
    for directory in cwd.ancestors() {
        inspect(&directory.join(".codex/config.toml"))?;
        if directory == root {
            break;
        }
    }
    Ok(())
}

fn inspect(path: &Path) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO masquerading as a config must fail rather than hang spawn.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if std::fs::symlink_metadata(path).is_ok() {
                Err(format!(
                    "Cannot read Codex project config {}",
                    display(path)
                ))
            } else {
                Ok(())
            };
        }
        Err(_) => {
            return Err(format!(
                "Cannot read Codex project config {}",
                display(path)
            ))
        }
    };
    if !file
        .metadata()
        .map_err(|_| format!("Cannot read Codex project config {}", display(path)))?
        .is_file()
    {
        return Err(format!(
            "Cannot read Codex project config {}: expected a regular file",
            display(path)
        ));
    }
    let mut raw = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|_| format!("Cannot read Codex project config {}", display(path)))?;
    if raw.len() as u64 > MAX_CONFIG_BYTES {
        return Err(format!(
            "Codex project config {} exceeds 1 MiB",
            display(path)
        ));
    }
    // Parser errors can echo a config value. Only the path, never its contents,
    // belongs in the error returned to the UI or a channel.
    let config: toml::Table = toml::from_str(&raw)
        .map_err(|_| format!("Cannot parse Codex project config {}", display(path)))?;
    let Some(servers) = config.get("mcp_servers") else {
        return Ok(());
    };
    let servers = servers
        .as_table()
        .ok_or_else(|| format!("Invalid MCP server declarations in {}", display(path)))?;
    if servers.is_empty() {
        return Ok(());
    }
    let names = servers
        .keys()
        .map(|name| {
            name.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Saved-connection Codex cannot start: {} declares MCP servers ({names}). Use a workspace without project MCP servers.",
        display(path)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &Path, text: &str) {
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        std::fs::write(root.join(".codex/config.toml"), text).unwrap();
    }

    #[test]
    fn non_repository_nest_does_not_load_parent_user_config() {
        let home = tempfile::tempdir().unwrap();
        config(
            home.path(),
            "[mcp_servers.account]\ncommand='SYNTHETIC_SERVER_COMMAND'\n",
        );
        let nest = home.path().join("nest");
        std::fs::create_dir(&nest).unwrap();
        validate(&nest).unwrap();
    }

    #[test]
    fn nearest_repository_bounds_all_project_layers_and_reason_names_source() {
        let home = tempfile::tempdir().unwrap();
        config(
            home.path(),
            "[mcp_servers.outside]\ncommand='SYNTHETIC_SERVER_COMMAND'\n",
        );
        let repo = home.path().join("repo");
        let cwd = repo.join("nested/work");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(repo.join(".git"), "gitdir: fixture").unwrap();
        validate(&cwd).unwrap();
        config(
            &repo,
            "[mcp_servers.accounts]\nenabled=false\ncommand='SYNTHETIC_SERVER_COMMAND'\n",
        );
        let error = validate(&cwd).unwrap_err();
        assert!(error.contains(&display(&repo.join(".codex/config.toml"))));
        assert!(error.contains("accounts"));
        assert!(!error.contains("SYNTHETIC_SERVER_COMMAND"));
        assert!(!error.contains("outside"));
    }

    #[test]
    fn project_server_added_after_home_generation_refuses_the_next_launch() {
        let workspace = tempfile::tempdir().unwrap();
        // Existing generated provider config does not freeze the workspace policy.
        validate(workspace.path()).unwrap();
        config(
            workspace.path(),
            "[mcp_servers.added_later]\ncommand='fixture'\n",
        );
        assert!(validate(workspace.path())
            .unwrap_err()
            .contains("added_later"));
    }

    #[test]
    fn unreadable_or_malformed_layers_fail_without_echoing_values() {
        let workspace = tempfile::tempdir().unwrap();
        config(workspace.path(), "key = 'SYNTHETIC_SECRET\n");
        let error = validate(workspace.path()).unwrap_err();
        assert!(error.contains("Cannot parse"));
        assert!(!error.contains("SYNTHETIC_SECRET"));
        let path = workspace.path().join(".codex/config.toml");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(validate(workspace.path())
            .unwrap_err()
            .contains("Cannot read"));
    }
}
