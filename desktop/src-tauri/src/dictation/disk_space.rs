//! Space available to this user on the model-storage volume.

use std::path::Path;

pub(crate) fn available(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|ancestor| ancestor.exists())?;
    available_on_volume(existing)
}

#[cfg(unix)]
fn available_on_volume(path: &Path) -> Option<u64> {
    use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    // The native API writes this complete struct on success. Both pointers
    // remain valid for the call; no filesystem data is dereferenced by Rust.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // Success above guarantees that statvfs initialized the output.
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::unnecessary_cast)] // libc widths differ between Unix targets.
    (stats.f_bavail as u64).checked_mul(stats.f_frsize as u64)
}

#[cfg(windows)]
fn available_on_volume(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // The path is NUL-terminated and the output pointer is valid for the call.
    let success = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (success != 0).then_some(available)
}

#[cfg(not(any(unix, windows)))]
fn available_on_volume(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn probes_the_volume_for_a_not_yet_created_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(super::available(&dir.path().join("missing/models")).is_some());
    }
}
