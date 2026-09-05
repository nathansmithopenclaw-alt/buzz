//! Secret-file consumption at the final child-spawn boundary.

use super::AcpError;

#[cfg(all(test, unix))]
mod tests;

pub(super) fn supply_token(
    child: &mut tokio::process::Command,
    home: &std::path::Path,
) -> Result<(), AcpError> {
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        use zeroize::Zeroizing;

        // Never include file contents or provider diagnostics in errors. Opening
        // nonblocking also prevents a substituted FIFO from hanging launch.
        let refused = || {
            AcpError::Protocol(
                "Selected subscription token is missing, unreadable, or unsafe; renew the account in Settings → AI".into(),
            )
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(home.join("oauth-token"))
            .map_err(|_| refused())?;
        let metadata = file.metadata().map_err(|_| refused())?;
        if !metadata.is_file() || metadata.mode() & 0o7777 != 0o600 || metadata.len() > 4096 {
            return Err(refused());
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(4097));
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| refused())?;
        let token = std::str::from_utf8(&bytes).map_err(|_| refused())?;
        // Desktop persists the normalized 108-character setup-token value.
        if token.len() != 108
            || !token.starts_with("sk-ant-oat01-")
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(refused());
        }
        child
            .env_remove("CLAUDE_SECURESTORAGE_CONFIG_DIR")
            .env("CLAUDE_CODE_OAUTH_TOKEN", token);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (child, home);
        Err(AcpError::Protocol(
            "Subscription token file permissions are not supported on this platform".into(),
        ))
    }
}
