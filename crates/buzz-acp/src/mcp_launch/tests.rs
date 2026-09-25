use super::*;
use crate::acp::EnvVar;

fn server() -> McpServer {
    McpServer {
        name: "buzz-report".into(),
        command: "python3".into(),
        args: vec!["-c".into(), "import hashlib,os; assert hashlib.sha256(os.environ['BUZZ_PRIVATE_KEY'].encode()).hexdigest()=='bd325cb033535ee6066f06b61b5fd1e39c469c3da933f5fb9ee79b894c5fe4bd'; assert hashlib.sha256(os.environ['BUZZ_AUTH_TAG'].encode()).hexdigest()=='f970892a3efd3860724740247d9e2eaf31d45d07fb61aa2cc7598c0ea641c9f2'".into()],
        env: vec![
            EnvVar { name: "BUZZ_PRIVATE_KEY".into(), value: "nsec1-synthetic-argv-regression".into() },
            EnvVar { name: "BUZZ_AUTH_TAG".into(), value: "synthetic-owner-attestation-argv-regression".into() },
        ],
    }
}

#[test]
fn private_files_preserve_config_and_are_removed_on_drop() {
    let (servers, files) = prepare(vec![server()]).unwrap();
    let path = Path::new(&servers[0].args[1]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert!(command_from_file(path).unwrap().status().unwrap().success());
    drop(files);
    assert!(!path.exists());
    assert!(command_from_file(path).is_err());
}

#[test]
fn ordinary_server_is_unchanged_and_invalid_file_fails_closed() {
    let mut ordinary = server();
    ordinary.env.clear();
    let original = serde_json::to_value(&ordinary).unwrap();
    let (servers, files) = prepare(vec![ordinary]).unwrap();
    assert_eq!(serde_json::to_value(&servers[0]).unwrap(), original);
    assert!(files.is_none());
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"not json synthetic-key").unwrap();
    assert_eq!(
        command_from_file(file.path()).unwrap_err().to_string(),
        "Invalid MCP launch configuration"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        symlink(file.path(), &link).unwrap();
        assert!(command_from_file(&link).is_err());
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(command_from_file(file.path()).is_err());
    }
}
