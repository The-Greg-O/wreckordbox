//! `backup restore` and `backup list`: backups of the Library and its
//! Sidecar, from a terminal. These never start a Library service: a restore
//! needs the Library to be owned by nothing else.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::{backup, codes, Config, ServiceError};

#[derive(Subcommand)]
pub enum BackupCommands {
    /// Restore a backup as the Library, and its Sidecar with it.
    ///
    /// The whole backup is put back: the Library's database, analysis,
    /// artwork and settings files, and, when the backup holds one, the
    /// Sidecar (Facts, Runs, the kinds file, the Tag schema, Recipes, and
    /// Stem audio if the backup included it). The two always go back
    /// together. A backup made before backups carried the Sidecar restores
    /// the Library and leaves the Sidecar as it is.
    ///
    /// Refused, with nothing changed, while the wreckordbox window or a
    /// Library service has the Library (quit the window, or run `wreckordbox
    /// stop`), while rekordbox runs, and for a damaged backup or another
    /// Library's. If it stops midway, the next program to open the Library
    /// finishes it or puts everything back first.
    Restore {
        /// The backup: a `.zip` made by wreckordbox.
        #[arg(value_name = "BACKUP")]
        archive: PathBuf,
        /// Confirm that the Library and Sidecar on this machine are to be
        /// replaced. Without it nothing is done.
        #[arg(long)]
        yes: bool,
    },
    /// List this Library's backups in the backup folder, newest first.
    List,
}

pub fn run(config: &Config, json: bool, command: &BackupCommands) -> Result<String, ServiceError> {
    let library = wreckordbox_service::locate_library()?;
    match command {
        BackupCommands::Restore { archive, yes } => {
            let archive = std::path::absolute(archive).unwrap_or_else(|_| archive.clone());
            if !*yes {
                return Err(ServiceError::new(
                    codes::RESTORE_REFUSED,
                    format!(
                        "restoring {} replaces the Library at {} and its Sidecar. Run again with --yes to restore.",
                        archive.display(),
                        library.master_db.display()
                    ),
                ));
            }
            let mut last = String::new();
            let restored = backup::restore(&library, config, &archive, &mut |phase, _, _, _| {
                if !json && phase != last {
                    let _ = writeln!(std::io::stderr(), "{}", phase_text(phase));
                    phase.clone_into(&mut last);
                }
            })?;
            Ok(if json { json!(restored).to_string() } else { restored_text(&restored) })
        }
        BackupCommands::List => {
            let backups = backup::list(&library, config)?;
            Ok(if json { json!({ "backups": backups }).to_string() } else { list_text(&backups) })
        }
    }
}

fn phase_text(phase: &str) -> &'static str {
    match phase {
        "preparing" => "Checking the backup...",
        "unpacking" => "Unpacking...",
        "validating" => "Checking what was unpacked...",
        _ => "Replacing the Library's files...",
    }
}

fn restored_text(restored: &backup::Restored) -> String {
    let mut text = format!("Restored {} as the Library at {}.", restored.archive.display(), restored.library.display());
    match &restored.sidecar {
        Some(sidecar) => {
            let _ = write!(text, "\nThe Sidecar at {} was restored with it.", sidecar.display());
            if !restored.stems {
                text.push_str(" Stem audio was not in the backup: a track whose Stem audio is not on this machine needs separating again.");
            }
        }
        None => text.push_str("\nThis backup holds no Sidecar; the Sidecar was left as it was."),
    }
    text
}

fn list_text(backups: &[backup::Backup]) -> String {
    if backups.is_empty() {
        return "No backups.".to_owned();
    }
    backups
        .iter()
        .map(|backup| {
            let holds = match (backup.sidecar, backup.stems) {
                (true, true) => "Library, Sidecar and Stem audio",
                (true, false) => "Library and Sidecar",
                _ => "Library only",
            };
            #[allow(clippy::cast_precision_loss, reason = "a size shown to one decimal place")]
            let megabytes = backup.bytes as f64 / 1_048_576.0;
            format!("{}  {megabytes:.1} MB  {holds}", backup.path.display())
        })
        .collect::<Vec<_>>()
        .join("\n")
}
