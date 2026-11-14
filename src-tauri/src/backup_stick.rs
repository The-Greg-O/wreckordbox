//! A backup whose folder is on a Stick that was locked, ejected or replaced
//! while it copied (wbx-45.14).
//!
//! The backup checks the Stick's authorization and identity at every file
//! and chunk it copies (`backups::create_with_progress`), so it stops at the
//! next one. Nothing more is written and nothing is cleaned up (a cleanup is
//! a write too); the DJ is told what is on the Stick.

use std::path::Path;

use crate::error::AppError;

/// `stopped`, the error a backup ended with, said with what it left on the
/// Stick when the reason is the Stick's authorization; any other error is
/// answered as it is.
pub(crate) fn stopped(
    stopped: AppError,
    access: &crate::stick_guard::StickAccess,
    root: &Path,
    leftovers: &[&Path],
    copied: u64,
) -> AppError {
    let Some(stick) = access.stick() else {
        return stopped;
    };
    if access.recheck().is_ok() {
        return stopped;
    }
    let name = stick.file_name().map_or_else(
        || stick.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    // Looked at on the Stick, which is only read: on a replaced Stick these
    // names are simply not there.
    let left: Vec<String> = leftovers
        .iter()
        .filter(|path| path.starts_with(root) && std::fs::symlink_metadata(path).is_ok())
        .filter_map(|path| {
            path.file_name()
                .map(|n| format!("“{}”", n.to_string_lossy()))
        })
        .collect();
    let what = if left.is_empty() {
        "nothing of it is left there".to_owned()
    } else {
        format!(
            "its unfinished copy is left in the backup folder as {} (hidden; delete it once {name} is unlocked)",
            left.join(" and ")
        )
    };
    AppError::new(
        stopped.kind,
        format!(
            "{} The backup to {name} stopped after copying {}; {what}. Nothing else on {name} was changed.",
            stopped.message.trim_end(),
            size(copied),
        ),
    )
}

fn size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", value / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", value / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", value / (1024.0 * 1024.0 * 1024.0))
    }
}
