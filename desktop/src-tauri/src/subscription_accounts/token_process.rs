//! setup-token uses an interactive terminal; its transcript stays in zeroizing memory.
#[cfg(unix)]
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
#[cfg(unix)]
use std::{
    io::Read,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
#[cfg(unix)]
use zeroize::Zeroizing;

#[cfg(unix)]
pub(super) fn run(
    command: Command,
    timeout: Duration,
    cancelled: &Arc<AtomicBool>,
) -> Result<Zeroizing<Vec<u8>>, String> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|_| "Could not open provider login terminal")?;
    let mut builder = CommandBuilder::new(command.get_program());
    builder.args(command.get_args());
    builder.env_clear();
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            builder.env(key, value);
        }
    }
    builder.env("TERM", "xterm-256color");
    if let Some(cwd) = command.get_current_dir() {
        builder.cwd(cwd);
    }
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|_| "Could not read provider login terminal")?;
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|_| "Could not start provider token command")?;
    drop(pair.slave);
    let Some(pid) = child.process_id() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Provider login process identity unavailable".into());
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::with_capacity(65537));
        let result = reader.by_ref().take(65537).read_to_end(&mut bytes);
        // Unix PTYs may signal EIO at EOF after the slave exits.
        let valid = result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(|error| error.raw_os_error() == Some(libc::EIO));
        let _ = sender.send(if valid {
            Ok(bytes)
        } else {
            Err("Could not capture provider token output".to_string())
        });
    });
    let started = Instant::now();
    let result = loop {
        if cancelled.load(Ordering::SeqCst) {
            break Err("Token mint cancelled".to_string());
        }
        if started.elapsed() >= timeout {
            break Err("Token mint timed out; renew to retry".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err("Provider token command failed; retry browser authorization".into())
                }
            }
            Err(_) => break Err("Could not inspect provider token process".into()),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let cleanup = super::process::kill_group(pid);
    if cleanup.is_err() {
        let _ = child.kill();
    }
    let reap = child
        .wait()
        .map_err(|_| "Could not reap provider token process");
    drop(pair.master);
    cleanup?;
    reap?;
    result?;
    let bytes = receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "Provider token output did not close")??;
    if bytes.len() > 65536 {
        return Err("Provider token output exceeded limit".into());
    }
    Ok(bytes)
}

#[cfg(not(unix))]
pub(super) fn run(
    _command: std::process::Command,
    _timeout: std::time::Duration,
    _cancelled: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    Err("Token mint containment unavailable on this platform".into())
}
