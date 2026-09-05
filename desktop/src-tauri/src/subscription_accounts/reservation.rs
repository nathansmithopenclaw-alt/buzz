//! Durable fail-closed reservation closes the spawn-to-runtime-receipt gap.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Reservation {
    token: String,
    pid: Option<u32>,
}

#[derive(Clone, Debug)]
pub(super) struct ReservationHandle {
    path: PathBuf,
    token: String,
}

fn path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if uuid::Uuid::parse_str(id)
        .ok()
        .is_none_or(|value| value.to_string() != id)
    {
        return Err("Invalid account id".into());
    }
    super::store::private_dir(root, false)?;
    Ok(root.join(format!("{id}.lease.json")))
}

fn read(path: &Path) -> Result<Option<Reservation>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Runtime reservation unavailable".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err("Invalid runtime reservation".into());
    }
    let data = fs::read(path).map_err(|_| "Runtime reservation unavailable")?;
    let value: Reservation =
        serde_json::from_slice(&data).map_err(|_| "Invalid runtime reservation")?;
    if uuid::Uuid::parse_str(&value.token).is_err() || value.pid == Some(0) {
        return Err("Invalid runtime reservation".into());
    }
    Ok(Some(value))
}

/// Unknown-start reservations survive crash. They require operator verification
/// of orphan cleanup; absence of a runtime receipt is not evidence of no child.
pub(super) fn ensure_clear(root: &Path, id: &str) -> Result<(), String> {
    let path = path(root, id)?;
    let Some(reservation) = read(&path)? else {
        return Ok(());
    };
    match reservation.pid {
        Some(pid) if !crate::managed_agents::process_is_running(pid) => {
            fs::remove_file(path).map_err(|_| "Could not clear stopped runtime reservation")?;
            Ok(())
        }
        Some(_) => Err("Account is reserved by a running agent; stop it before continuing".into()),
        None => Err("Runtime start was interrupted before its process was recorded. Operator verification of orphan cleanup is required before this account can be recovered".into()),
    }
}

pub(super) fn create(root: &Path, id: &str, token: &str) -> Result<ReservationHandle, String> {
    ensure_clear(root, id)?;
    let path = path(root, id)?;
    let value = Reservation {
        token: token.into(),
        pid: None,
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| "Could not encode runtime reservation")?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "Could not reserve account before runtime start")?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "Could not persist runtime reservation")?;
    fs::File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "Could not persist account reservation directory")?;
    Ok(ReservationHandle {
        path,
        token: token.into(),
    })
}

impl ReservationHandle {
    pub(super) fn owns_running_process(&self, pid: u32) -> Result<bool, String> {
        Ok(read(&self.path)?.is_some_and(|current| {
            current.token == self.token
                && current.pid == Some(pid)
                && crate::managed_agents::process_is_running(pid)
        }))
    }

    pub(super) fn attach_pid(&self, pid: u32) -> Result<(), String> {
        let current = read(&self.path)?.ok_or("Runtime reservation disappeared")?;
        if current.token != self.token {
            return Err("Runtime reservation generation changed".into());
        }
        let bytes = serde_json::to_vec(&Reservation {
            token: self.token.clone(),
            pid: Some(pid),
        })
        .map_err(|_| "Could not encode runtime reservation")?;
        crate::managed_agents::atomic_write_json_restricted(&self.path, &bytes)
            .map_err(|_| "Could not record account runtime process".to_string())
    }

    pub(super) fn release(&self) -> Result<(), String> {
        if read(&self.path)?.is_some_and(|current| current.token == self.token) {
            fs::remove_file(&self.path)
                .map_err(|_| "Could not clear stopped runtime reservation")?;
        }
        Ok(())
    }
}
