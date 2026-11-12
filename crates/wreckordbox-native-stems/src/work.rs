//! The Stick's stems folder as the writer works in it: finding it without
//! being led off the Stick, the working names of a write in progress,
//! making a finished set appear, and putting right what an interrupted
//! write left.
//!
//! Nothing here follows a symbolic link. FAT and exFAT have none, so one
//! on the way is not a Stick as a player would see it; and following one
//! would write somewhere the Spare stick marker says nothing about.
//!
//! The working names are short and of this writer's own (`.wbx-…`), so a
//! track's name at the file system's limit still has room to be worked
//! on, and no other tool's working folders are mistaken for these. All of
//! them start with a dot, which the deck and the AZ-MODS app skip.

use std::path::{Path, PathBuf};

use crate::{NativeStemsError, STEMS_FOLDER};

/// What every working name starts with.
const PREFIX: &str = ".wbx-";
/// A set being written.
const STAGING: &str = ".part";
/// The earlier set, set aside while its replacement is renamed in.
const BACKUP: &str = ".old";
/// A file beside a backup, holding the name of the folder it was.
const NOTE: &str = ".dest";
/// The most bytes of a folder name a Stick stores.
const NAME_BYTES: usize = 255;

fn off_stick(path: &Path, why: &str) -> NativeStemsError {
    NativeStemsError::OffStick {
        path: path.to_path_buf(),
        why: why.to_owned(),
    }
}

/// The Stick's stems folder, if it has one: the directory at its root
/// whose name is `STEMS` in any ASCII case, as the deck finds it.
pub(crate) fn stems_dir(stick: &Path) -> Result<Option<PathBuf>, NativeStemsError> {
    entry_named(stick, STEMS_FOLDER)
}

/// Makes the Stick's stems folder.
pub(crate) fn make_stems_dir(stick: &Path) -> Result<PathBuf, NativeStemsError> {
    let dir = stick.join(STEMS_FOLDER);
    std::fs::create_dir(&dir).map_err(|e| NativeStemsError::io(&dir, e))?;
    Ok(dir)
}

/// The directory in `parent` whose name is `name`, ignoring ASCII case and
/// nothing else: the deck's comparison. A symbolic link of that name is
/// refused, whatever it points at; any other kind of file is not a
/// directory, and making one of that name will say so.
pub(crate) fn entry_named(parent: &Path, name: &str) -> Result<Option<PathBuf>, NativeStemsError> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Ok(None);
    };
    for entry in entries.filter_map(Result::ok) {
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|found| found.eq_ignore_ascii_case(name))
        {
            continue;
        }
        // The entry's own kind: a link is a link, not what it leads to.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            return Err(off_stick(
                &entry.path(),
                "is a symbolic link, which may lead off the Stick",
            ));
        }
        if kind.is_dir() {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

/// The working names of one write, all inside the stems folder.
pub(crate) struct Names {
    pub(crate) staging: PathBuf,
    backup: PathBuf,
    note: PathBuf,
}

impl Names {
    /// For the set that goes in `folder`: told apart by a checksum of the
    /// folder's name in the deck's comparison, so they are the same names
    /// each time the same folder is written.
    pub(crate) fn of(stems_dir: &Path, folder: &str) -> Self {
        let tag = crc32fast::hash(folder.to_ascii_lowercase().as_bytes());
        let named = |ending: &str| stems_dir.join(format!("{PREFIX}{tag:08x}{ending}"));
        Self {
            staging: named(STAGING),
            backup: named(BACKUP),
            note: named(NOTE),
        }
    }
}

/// Makes the staged set appear as `dir`: one rename, or for a set that is
/// being made again, the old one set aside (with a note of where it
/// belongs), the new one renamed in, the old one removed.
pub(crate) fn publish(
    stems_dir: &Path,
    names: &Names,
    dir: &Path,
    existing: Option<&Path>,
) -> Result<(), NativeStemsError> {
    let Some(existing) = existing else {
        std::fs::rename(&names.staging, dir).map_err(|e| NativeStemsError::io(dir, e))?;
        flush_dir(stems_dir);
        return Ok(());
    };
    let folder = existing
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    // The note first: from the moment the old set has a working name,
    // there is something on the Stick that says where it goes back.
    remove(&names.note);
    write_new(&names.note, folder.as_bytes())?;
    if let Err(error) = std::fs::rename(existing, &names.backup) {
        remove(&names.note);
        return Err(NativeStemsError::io(existing, error));
    }
    flush_dir(stems_dir);
    if let Err(error) = std::fs::rename(&names.staging, dir) {
        // Put the earlier set back rather than leave the track with none.
        return Err(match std::fs::rename(&names.backup, existing) {
            Ok(()) => {
                remove(&names.note);
                NativeStemsError::io(dir, error)
            }
            // It stays whole under its working name, note beside it.
            Err(_) => NativeStemsError::Rollback {
                dir: dir.to_path_buf(),
                backup: names.backup.clone(),
                source: error,
            },
        });
    }
    flush_dir(stems_dir);
    remove(&names.backup);
    remove(&names.note);
    flush_dir(stems_dir);
    Ok(())
}

/// Puts right what an interrupted write left in the stems folder.
///
/// - A staging folder is an unfinished set: removed.
/// - A backup whose note names a folder that is not there is the only
///   complete set that track has: it is renamed back.
/// - A backup whose note names a folder that is there was replaced: removed.
/// - A backup with no readable note is left alone. Nothing says what it
///   is, so it is not this writer's to delete.
///
/// Only this writer's own working names are touched.
pub(crate) fn recover(stems_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(stems_dir) else {
        return;
    };
    let names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(PREFIX))
        .collect();
    let mut restored = false;
    for name in &names {
        let path = stems_dir.join(name);
        if name.ends_with(STAGING) {
            remove(&path);
        } else if let Some(tag) = name.strip_suffix(BACKUP) {
            let note = stems_dir.join(format!("{tag}{NOTE}"));
            let Some(folder) = noted_folder(&note) else {
                continue;
            };
            if !is_dir_itself(&path) {
                continue;
            }
            match entry_named(stems_dir, &folder) {
                Ok(None) if !stems_dir.join(&folder).exists() => {
                    if std::fs::rename(&path, stems_dir.join(&folder)).is_ok() {
                        restored = true;
                        remove(&note);
                    }
                }
                Ok(Some(_)) => {
                    remove(&path);
                    remove(&note);
                }
                // Something else has the name: leave both as they are.
                Ok(None) | Err(_) => {}
            }
        }
    }
    // A note whose backup is gone says nothing any more.
    for name in &names {
        if let Some(tag) = name.strip_suffix(NOTE) {
            if std::fs::symlink_metadata(stems_dir.join(format!("{tag}{BACKUP}"))).is_err() {
                remove(&stems_dir.join(name));
            }
        }
    }
    if restored {
        flush_dir(stems_dir);
    }
}

/// The folder a note names, when it is a regular file holding one name a
/// set could have: not empty, not dot-named, no separator, short enough.
fn noted_folder(note: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(note).ok()?;
    if !metadata.is_file() || metadata.len() > NAME_BYTES as u64 {
        return None;
    }
    let folder = String::from_utf8(std::fs::read(note).ok()?).ok()?;
    let usable = !folder.is_empty()
        && !folder.starts_with('.')
        && !folder.contains(['/', '\\', '\0'])
        && folder.len() <= NAME_BYTES;
    usable.then_some(folder)
}

fn is_dir_itself(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Removes whatever is at `path`: a folder with all in it, a file, or a
/// link (the link, never what it points at). Nothing there is fine.
pub(crate) fn remove(path: &Path) {
    if is_dir_itself(path) {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Writes a file that must not be there yet, flushed to the Stick. A link
/// left under the name is an error, not a way out.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeStemsError> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| NativeStemsError::io(path, e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| NativeStemsError::io(path, e))
}

/// Asks for a folder's entries to reach the Stick. Not every file system
/// can; the files themselves are already flushed.
pub(crate) fn flush_dir(dir: &Path) {
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
}
