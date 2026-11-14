//! Stick load reports kept in the app's own state folder (wbx-71), so that
//! a load's outcome reaches the DJ even when the window closed while it
//! ran: never on the Stick, never in the Library. One file per load,
//! `stick-load-reports/<operation id>.json`, written whole (temporary file,
//! synced, renamed) before the load command lets the window's shutdown
//! complete. A report stays until the DJ dismisses it.
//!
//! A kept report: `{ id, stick: { path, name, volume }, finished, report,
//! error }`: `report` is the service's `sticks.load` answer as it gave it
//! (outcome, what was written, not written, unchanged, every Native stems
//! outcome and reason, its error), `error` the refusal or failure when the
//! service gave no report (`{ code, message }`).
//!
//! The shell owns a report's name (wbx-72 verification review): it issues
//! each id itself (`reserve_report`, a fresh UUID reserved with create-new
//! semantics before the load runs), a load may use only an id the shell
//! reserved and nobody has used (`claim_report`), and keeping a report
//! never replaces a file that is there: a second load can never overwrite a
//! kept, undismissed report, or a running load's.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

const FOLDER: &str = "stick-load-reports";

fn folder(state_dir: &Path) -> PathBuf {
    state_dir.join(FOLDER)
}

/// An operation id the window or the shell made: letters, digits and
/// dashes only, so it names a file in the folder and nothing else.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn reserved(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!(".{id}.reserved"))
}

fn running(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!(".{id}.running"))
}

/// A fresh report id, reserved here (created new; never one that exists).
pub fn reserve_report(state_dir: &Path) -> std::io::Result<String> {
    let dir = folder(state_dir);
    std::fs::create_dir_all(&dir)?;
    loop {
        let id = uuid::Uuid::new_v4().to_string();
        if dir.join(format!("{id}.json")).exists() || running(&dir, &id).exists() {
            continue;
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(reserved(&dir, &id))
        {
            Ok(_) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

/// Takes reservation `id` for one load: true only when the shell reserved
/// it and no load has used it (the reservation is moved, so a second claim
/// of the same id fails).
pub fn claim_report(state_dir: &Path, id: &str) -> bool {
    if !valid_id(id) {
        return false;
    }
    let dir = folder(state_dir);
    if dir.join(format!("{id}.json")).exists() {
        return false;
    }
    std::fs::rename(reserved(&dir, id), running(&dir, id)).is_ok()
}

/// Keeps `kept` (its `id` names the file), durably, before returning; an
/// existing report of that name is never replaced (an error instead).
pub(crate) fn keep_report(state_dir: &Path, kept: &Value) -> std::io::Result<()> {
    let id = kept["id"]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a load report needs a plain id",
            )
        })?;
    let dir = folder(state_dir);
    std::fs::create_dir_all(&dir)?;
    let temporary = dir.join(format!(".{id}.json.partial"));
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(kept).unwrap_or_default())?;
        file.sync_all()?;
    }
    // Linked into place, which fails if the name is taken: never renamed over.
    let linked = std::fs::hard_link(&temporary, dir.join(format!("{id}.json")));
    let _ = std::fs::remove_file(&temporary);
    linked?;
    let _ = std::fs::remove_file(running(&dir, id));
    if let Ok(dir) = std::fs::File::open(&dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Every report not yet dismissed, oldest first. A file that cannot be read
/// is left where it is and skipped.
pub fn kept_reports(state_dir: &Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(folder(state_dir)) else {
        return Vec::new();
    };
    let mut reports: Vec<Value> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| serde_json::from_slice(&std::fs::read(path).ok()?).ok())
        .collect();
    reports.sort_by(|a, b| {
        (a["finished"].as_str(), a["id"].as_str()).cmp(&(b["finished"].as_str(), b["id"].as_str()))
    });
    reports
}

/// The DJ has read report `id`: it is removed. An unknown id is no error.
pub fn dismiss_report(state_dir: &Path, id: &str) -> std::io::Result<()> {
    if !valid_id(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a load report id",
        ));
    }
    match std::fs::remove_file(folder(state_dir).join(format!("{id}.json"))) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
