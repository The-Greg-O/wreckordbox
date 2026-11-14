//! Cleanup is a write, never part of reading a tag. Only old, exactly named
//! replacements of this audio file, from a process that is no longer alive.
use std::{fs, io, path::Path, time::Duration};

pub fn cleanup_stale(path: &Path) -> io::Result<usize> {
    cleanup_stale_checked(path, &|| Ok(()))
}

pub(super) fn cleanup_stale_checked(
    path: &Path,
    authorize: &impl Fn() -> io::Result<()>,
) -> io::Result<usize> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(0);
    };
    let prefix = format!(".{}.rbx-key-", name.to_string_lossy());
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".tmp"))
        else {
            continue;
        };
        let Some((pid, stamp)) = rest.split_once('-') else {
            continue;
        };
        if stamp.is_empty() || !stamp.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let old = entry
            .metadata()?
            .modified()?
            .elapsed()
            .is_ok_and(|age| age > Duration::from_secs(3600));
        if !old {
            continue;
        }
        let pid = sysinfo::Pid::from_u32(pid);
        let mut processes = sysinfo::System::new();
        processes.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if processes.process(pid).is_some() {
            continue;
        }
        authorize()?;
        fs::remove_file(entry.path())?;
        removed += 1;
    }
    Ok(removed)
}
