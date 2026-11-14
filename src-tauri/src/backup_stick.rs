//! A backup whose folder is on a Stick that was locked, ejected or replaced
//! while it copied (wbx-45.14).
//!
//! Every write the backup makes to its folder is checked first, on every
//! thread (`backup_guard`), so it stops at the next one. Nothing more is
//! written and nothing is cleaned up (a cleanup is a write too); the DJ is
//! told what is on the Stick. What it made there is accounted as it was
//! made, on the Stick as it was entered: after a replacement the folder now
//! mounted at that place is another Stick's and is never read for this.

use std::path::PathBuf;

use crate::error::AppError;

/// What a backup made in its folder, as it stands now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Made {
    /// Its unfinished copy or archive.
    Unfinished,
    /// Its staged copy, moved aside to be deleted.
    SetAside,
    /// The backup itself, named and complete.
    Backup,
}

/// `stopped`, the error a backup ended with, said with what it left on the
/// Stick when the reason is the Stick's authorization (that refusal's kind
/// and words); any other error is answered as it is. `made`: what the
/// backup made in its folder, by the name each has now; `copied`: bytes
/// copied so far.
pub(crate) fn stopped(
    stopped: AppError,
    access: &crate::stick_guard::StickAccess,
    made: &[(PathBuf, Made)],
    copied: u64,
) -> AppError {
    let Some(stick) = access.stick() else {
        return stopped;
    };
    let Err(refused) = access.recheck() else {
        return stopped;
    };
    let name = stick.file_name().map_or_else(
        || stick.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let left: Vec<String> = made
        .iter()
        .filter_map(|(path, what)| {
            let shown = path.file_name()?.to_string_lossy().into_owned();
            Some(match what {
                Made::Unfinished => format!("“{shown}” (unfinished)"),
                Made::SetAside => format!("“{shown}” (unfinished, set aside to be deleted)"),
                Made::Backup => format!("“{shown}” (the backup, complete)"),
            })
        })
        .collect();
    let what = if left.is_empty() {
        format!("nothing of it was written to {name}")
    } else {
        format!(
            "what it wrote stays in the backup folder on {name}, the Stick as it was when the backup began: {} (delete what is unfinished once that Stick is back and unlocked)",
            left.join(", ")
        )
    };
    AppError::new(
        refused.kind,
        format!(
            "{} The backup to {name} stopped after copying {}; {what}. Nothing else was changed.",
            refused.message.trim_end(),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    // A backup to a Stick that is locked or replaced while it copies
    // (wbx-45.14). The backup's own progress report is the hold: the test acts
    // from inside it, at a known file, on the backup's thread. Temporary
    // folders stand in for Sticks.

    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::backups::create_with_progress;
    use crate::state::AppState;

    fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let location =
            rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        let state = AppState::unprotected(dir.path().join("backups"));
        state.set_library(library, false, db.schema().db_version, 0, location);
        let stick = dir.path().join("sticks/GIG");
        fs::create_dir_all(&stick).unwrap();
        crate::stick_guard::testing::unlock(&state, &stick);
        state
            .set_backup_destination(&crate::stick_guard::testing::ordinary(), &stick)
            .unwrap();
        (dir, state, stick)
    }

    fn walk(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut todo = vec![root.to_path_buf()];
        while let Some(dir) = todo.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    out.push((path.clone(), Vec::new()));
                    todo.push(path);
                } else {
                    out.push((path.clone(), fs::read(&path).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    /// Runs a backup and, at the first file it reports copying after the
    /// database (an analysis file, mid-copy), calls `then`; answers the
    /// backup's result and what the Stick held right after `then`.
    fn backup_interrupted(
        state: &AppState,
        stick: &Path,
        then: impl Fn(),
    ) -> (crate::AppResult<String>, Vec<(PathBuf, Vec<u8>)>) {
        let mut at = None;
        let result = create_with_progress(state, &mut |phase, done, _, item| {
            if at.is_none() && phase == "copying" && done > 0 {
                then();
                at = Some((item.to_owned(), walk(stick)));
            }
            Ok(())
        });
        let (_item, held) = at.expect("the backup reached copying");
        (result, held)
    }

    #[test]
    fn a_backup_to_a_stick_locked_mid_copy_stops_at_once_and_says_what_it_wrote() {
        let (_dir, state, stick) = fixture();
        let state_dir = state.backup_dir().to_path_buf();
        let (result, held) = backup_interrupted(&state, &stick, || {
            // A lock that does not wait for the backup's execution lock (the
            // backup's own thread, as an eject is): only the record changes.
            wreckordbox_stick_protection::lock(&state_dir, &stick).unwrap();
        });

        let error = result.expect_err("the backup stops");
        assert_eq!(
            error.kind,
            crate::error::ErrorKind::StickProtected,
            "{error:?}"
        );
        assert_eq!(
            walk(&stick),
            held,
            "nothing more was written to the Stick, nothing removed"
        );
        assert!(
            error.message.contains("stopped after copying"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains(".partial-"),
            "names the unfinished copy left on it: {}",
            error.message
        );
        assert!(
            !fs::read_dir(&stick).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("rbexport-")),
            "no backup was named"
        );
    }

    /// wbx-45.14 round 1 (b): the compression workers write to the Stick
    /// themselves; each write checks it. The Stick is replaced while every
    /// worker is held before a write: from then on not a byte changes on
    /// either Stick (no worker's write, no unfinished archive finalized), and
    /// the report names what is on the original one.
    #[test]
    fn a_stick_replaced_while_the_compression_workers_write_gets_not_a_byte_more() {
        let (dir, state, stick) = fixture();
        let location = state.location().unwrap();
        let tree = rbl_backup::analysis_dir(&location);
        for i in 0..12_u8 {
            let folder = tree.join(format!("P{i:03}"));
            fs::create_dir_all(&folder).unwrap();
            let bytes: Vec<u8> = (0..300_000_u32)
                .map(|n| n.wrapping_mul(31).to_le_bytes()[0] ^ i)
                .collect();
            fs::write(folder.join("ANLZ0000.DAT"), bytes).unwrap();
        }
        let old = dir.path().join("sticks/GIG-taken-out");
        let held =
            crate::test_holds::arm(crate::test_holds::BACKUP_COMPRESSING, state.backup_dir());
        let (result, at_replacement, new_stick) = std::thread::scope(|scope| {
            let backing_up = scope.spawn(|| crate::backups::create(&state));
            held.reached();
            // Held at a worker's write with compression under way: an
            // analysis file's output is already on the Stick.
            assert!(
                walk(&stick).iter().any(|(path, bytes)| {
                    path.to_string_lossy().contains("/analysis/") && !bytes.is_empty()
                }),
                "compression had begun"
            );
            fs::rename(&stick, &old).unwrap();
            fs::create_dir_all(&stick).unwrap();
            fs::write(
                stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
                b"",
            )
            .unwrap();
            let at_replacement = walk(&old);
            let new_stick = walk(&stick);
            held.release();
            (backing_up.join().unwrap(), at_replacement, new_stick)
        });

        let error = result.expect_err("the backup stops");
        assert_eq!(error.kind, crate::error::ErrorKind::NotFound, "{error:?}");
        assert_eq!(
            walk(&old),
            at_replacement,
            "not a byte more on the original Stick"
        );
        assert_eq!(walk(&stick), new_stick, "nor on the one put in its place");
        assert!(error.message.contains(".partial-"), "{}", error.message);
        assert!(
            error.message.contains("as it was when the backup began"),
            "{}",
            error.message
        );
    }

    /// Takes the rig's Stick out and puts another in at the same place.
    fn replace(stick: &Path, old: &Path) {
        fs::rename(stick, old).unwrap();
        fs::create_dir_all(stick).unwrap();
        fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();
    }

    /// wbx-45.14 verification (3): replaced while the database is copied,
    /// the report counts what the copy had done, never zero.
    #[test]
    fn a_stick_replaced_during_the_database_copy_reports_what_it_had_copied() {
        let (dir, state, stick) = fixture();
        let old = dir.path().join("sticks/GIG-taken-out");
        let held = crate::test_holds::arm(
            crate::test_holds::BACKUP_DATABASE_COPYING,
            state.backup_dir(),
        );
        let result = std::thread::scope(|scope| {
            let backing_up = scope.spawn(|| crate::backups::create(&state));
            held.reached();
            replace(&stick, &old);
            held.release();
            backing_up.join().unwrap()
        });
        let error = result.expect_err("the backup stops");
        assert!(
            !error.message.contains("after copying 0 bytes"),
            "{}",
            error.message
        );
        assert!(error.message.contains("(unfinished)"), "{}", error.message);
    }

    /// wbx-45.14 verification (3): replaced after the backup was named, the
    /// report names what is on the original Stick now: the complete backup
    /// and the staged copy set aside, not their obsolete unfinished names.
    #[test]
    fn a_stick_replaced_after_the_backup_is_named_reports_the_names_it_has_now() {
        let (dir, state, stick) = fixture();
        let old = dir.path().join("sticks/GIG-taken-out");
        let held = crate::test_holds::arm(crate::test_holds::BACKUP_PUBLISHED, state.backup_dir());
        let result = std::thread::scope(|scope| {
            let backing_up = scope.spawn(|| crate::backups::create(&state));
            held.reached();
            replace(&stick, &old);
            held.release();
            backing_up.join().unwrap()
        });
        let error = result.expect_err("the backup stops before its folder is made durable");
        let named = fs::read_dir(&old)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with("rbexport-"))
            .expect("the backup is on the original Stick");
        assert!(
            error
                .message
                .contains(&format!("“{named}” (the backup, complete)")),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("set aside to be deleted"),
            "{}",
            error.message
        );
        assert!(
            !error.message.contains(".partial-"),
            "no obsolete names: {}",
            error.message
        );
    }

    #[test]
    fn a_backup_to_a_stick_replaced_mid_copy_writes_nothing_to_the_new_one() {
        let (dir, state, stick) = fixture();
        let old = dir.path().join("sticks/GIG-taken-out");
        let (result, held) = backup_interrupted(&state, &stick, || {
            // Taken out, and another Stick put in at the same place.
            fs::rename(&stick, &old).unwrap();
            fs::create_dir_all(&stick).unwrap();
            fs::write(
                stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
                b"",
            )
            .unwrap();
        });

        let error = result.expect_err("the backup stops");
        assert_eq!(error.kind, crate::error::ErrorKind::NotFound, "{error:?}");
        assert_eq!(
            walk(&stick),
            held,
            "the Stick put in its place is left as it was"
        );
        assert!(
            error.message.contains("stopped after copying"),
            "{}",
            error.message
        );
    }
}
