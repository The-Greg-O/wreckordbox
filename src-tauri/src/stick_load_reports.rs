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

/// Keeps `kept` (its `id` names the file), durably, before returning.
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
    std::fs::rename(&temporary, dir.join(format!("{id}.json")))?;
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
