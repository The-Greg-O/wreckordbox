//! The unfinished copies a refused backup left on a Stick, and their
//! removal once that Stick is back and unlocked (wbx-45.25).
//!
//! A backup stopped by Stick Protection (wbx-45.14: the Stick locked,
//! ejected or replaced mid-copy) writes nothing more and cleans nothing up,
//! so its `.partial-<id>` copy stays on the Stick. What it left is recorded
//! here, in the app's own state folder (never on the Stick): the backup
//! folder and the exact names it made, each with the random id that backup
//! chose.
//!
//! [`backup_leftovers`] is a read: it lists what of that record is in the
//! backup folder now and, when there is something, plans its removal
//! (`stick_approval`), bound to this very mount of the folder. Only the DJ's
//! hold turns the plan into a token (`stick_write_approve`).
//! [`sweep_backup_leftovers`] then enters Stick Protection (refused on a
//! protected Stick: nothing removed), uses the token up (refused when the
//! Stick was replaced or what is there changed since the offer) and removes
//! each recorded copy, checking the Stick's authorization before every one.
//! Nothing else in the folder is ever removed: not a look-alike name, not a
//! name this app did not record, not a link.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backup_stick::Made;
use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;
use crate::stick_approval::{self, StickOperation};

/// The record, in the state folder.
const FILE: &str = "backup-leftovers.json";

/// One unfinished copy a refused backup left in `folder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Leftover {
    folder: PathBuf,
    name: String,
    /// The staged copy (a folder), or the unfinished archive (a file).
    directory: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    leftovers: Vec<Leftover>,
}

/// What the window offers to remove from the backup folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverOffer {
    /// The backup folder is on a Stick.
    pub on_stick: bool,
    pub stick_name: String,
    /// Stick Protection refuses the removal now: the DJ must unlock it.
    pub protected: bool,
    /// The names this app's refused backups left there, as they are now.
    pub leftovers: Vec<String>,
    /// The removal's plan id, for the DJ's hold; `None` when there is
    /// nothing to remove.
    pub plan: Option<String>,
}

/// The name a backup gives its unfinished copy (`.partial-<id>`) or
/// archive (`.partial-<id>.zip`), `<id>` a UUID as this app writes it:
/// whether it names a folder; `None` for any other name.
fn own_name(name: &str) -> Option<bool> {
    let rest = name.strip_prefix(".partial-")?;
    let (id, directory) = match rest.strip_suffix(".zip") {
        Some(id) => (id, false),
        None => (rest, true),
    };
    let parsed = uuid::Uuid::parse_str(id).ok()?;
    (parsed.hyphenated().to_string() == id).then_some(directory)
}

fn read(state_dir: &Path) -> Record {
    std::fs::read(state_dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn store(state_dir: &Path, record: &Record) -> AppResult<()> {
    crate::durable::create_dir_all(state_dir).map_err(|e| AppError::internal(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| AppError::internal(e.to_string()))?;
    crate::durable::write(&state_dir.join(FILE), &bytes)
        .map_err(|e| AppError::internal(e.to_string()))
}

/// Records what a backup to `folder`, refused by Stick Protection, left
/// there unfinished (`made`, by the names each has now). Written to the
/// state folder only.
pub(crate) fn record(state_dir: &Path, folder: &Path, made: &[(PathBuf, Made)]) {
    let mut record = read(state_dir);
    let before = record.leftovers.len();
    for (path, what) in made {
        if *what != Made::Unfinished || path.parent() != Some(folder) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(directory) = own_name(name) else {
            continue;
        };
        let leftover = Leftover {
            folder: folder.to_path_buf(),
            name: name.to_owned(),
            directory,
        };
        if !record.leftovers.contains(&leftover) {
            record.leftovers.push(leftover);
        }
    }
    if record.leftovers.len() != before {
        if let Err(e) = store(state_dir, &record) {
            tracing::warn!(error = %e, "could not record the unfinished backup left on a Stick");
        }
    }
}

/// The recorded leftovers in `folder` that are there now, as the backup
/// made them (a real folder or file, not a link).
fn present(state_dir: &Path, folder: &Path) -> Vec<Leftover> {
    let mut found: Vec<Leftover> = read(state_dir)
        .leftovers
        .into_iter()
        .filter(|l| l.folder == folder && own_name(&l.name) == Some(l.directory))
        .filter(|l| is_as_made(&folder.join(&l.name), l.directory))
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Whether `path` is a real folder (`directory`) or file, as a backup made
/// it: never a link.
fn is_as_made(path: &Path, directory: bool) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| {
        !meta.file_type().is_symlink()
            && if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            }
    })
}

fn operation(leftovers: &[Leftover]) -> StickOperation {
    StickOperation::SweepBackupLeftovers {
        names: leftovers.iter().map(|l| l.name.clone()).collect(),
    }
}

/// What may be removed from the backup folder, and the plan for it. Writes
/// nothing.
pub fn offer(state: &AppState) -> AppResult<LeftoverOffer> {
    let folder = state.backup_destination();
    let Some(root) = crate::stick_guard::stick_of(&folder) else {
        return Ok(LeftoverOffer::default());
    };
    let protection = wreckordbox_stick_protection::status(state.backup_dir(), &root);
    // Under the Stick's execution lock, as a write would read it: nothing
    // writes there while the offer is made and bound.
    let _lock = stick_approval::lock(state.backup_dir(), &root)?;
    let leftovers = present(state.backup_dir(), &folder);
    let plan = if leftovers.is_empty() {
        None
    } else {
        Some(stick_approval::propose(
            &folder,
            operation(&leftovers),
            None,
        )?)
    };
    Ok(LeftoverOffer {
        on_stick: true,
        stick_name: protection.name,
        protected: protection.protected,
        leftovers: leftovers.into_iter().map(|l| l.name).collect(),
        plan,
    })
}

/// Removes `leftovers` from `folder`, each only while the Stick is still
/// the one entered and unlocked, and only as the backup made it; each
/// removed is dropped from the record. Answers the names removed.
///
/// `access`: the capability, Stick Protection's leave (wbx-45.11).
fn remove_leftovers(
    access: &crate::stick_guard::StickAccess,
    state_dir: &Path,
    folder: &Path,
    leftovers: &[Leftover],
) -> AppResult<Vec<String>> {
    let mut removed = Vec::new();
    let mut outcome = Ok(());
    for leftover in leftovers {
        if let Err(refused) = access.recheck() {
            outcome = Err(refused);
            break;
        }
        let path = folder.join(&leftover.name);
        if !is_as_made(&path, leftover.directory) {
            continue;
        }
        let gone = if leftover.directory {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = gone {
            outcome = Err(AppError::internal(format!(
                "“{}” could not be removed from the backup folder.",
                leftover.name
            ))
            .with_detail(e.to_string()));
            break;
        }
        removed.push(leftover.clone());
    }
    let mut record = read(state_dir);
    record.leftovers.retain(|l| !removed.contains(l));
    store(state_dir, &record)?;
    outcome.map(|()| removed.into_iter().map(|l| l.name).collect())
}

/// What is offered for removal from the backup folder; writes nothing.
#[tauri::command]
pub async fn backup_leftovers(state: tauri::State<'_, Arc<AppState>>) -> AppResult<LeftoverOffer> {
    let state = Arc::clone(&state);
    blocking("backup_leftovers", move || offer(&state)).await
}

/// Removes what this app's refused backups left in the backup folder, with
/// the DJ's confirmation of the offer (`approval`, from
/// `stick_write_approve`). Answers the names removed.
#[tauri::command]
pub async fn sweep_backup_leftovers(
    state: tauri::State<'_, Arc<AppState>>,
    approval: Option<String>,
) -> AppResult<Vec<String>> {
    let state = Arc::clone(&state);
    blocking("sweep_backup_leftovers", move || {
        let folder = state.backup_destination();
        if crate::stick_guard::stick_of(&folder).is_none() {
            return Err(AppError::new(
                ErrorKind::NotFound,
                "The backup folder is not on a Stick.",
            ));
        }
        // A removal is a write to the Stick: Stick Protection first, under
        // its lock (refused when protected: nothing is removed), then the
        // DJ's confirmation of exactly what was offered, on this very mount.
        let access = crate::stick_guard::enter(state.backup_dir(), &folder)?;
        let leftovers = present(state.backup_dir(), &folder);
        crate::stick_approval::consume(approval.as_deref(), &operation(&leftovers), &folder, 0)?;
        remove_leftovers(&access, state.backup_dir(), &folder, &leftovers)
    })
    .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    // Temporary folders stand in for Sticks. A backup is stopped mid-copy
    // by locking or replacing its Stick from inside its own progress
    // report, as `backup_stick`'s tests do.

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::backups::create_with_progress;

    fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let location =
            rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        let state = AppState::unprotected(dir.path().join("state"));
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

    fn names(root: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    /// A backup to the rig's Stick, stopped mid-copy by locking it; the
    /// `.partial-` names it left there.
    fn refused_backup(state: &AppState, stick: &Path) -> Vec<String> {
        let state_dir = state.backup_dir().to_path_buf();
        let mut done = false;
        let result = create_with_progress(state, &mut |phase, copied, _, _| {
            if !done && phase == "copying" && copied > 0 {
                done = true;
                wreckordbox_stick_protection::lock(&state_dir, stick).unwrap();
            }
            Ok(())
        });
        let error = result.expect_err("the backup stops");
        assert_eq!(error.kind, ErrorKind::StickProtected, "{error:?}");
        let left: Vec<String> = names(stick)
            .into_iter()
            .filter(|n| n.starts_with(".partial-"))
            .collect();
        assert!(!left.is_empty(), "the backup left its unfinished copy");
        left
    }

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    struct Window {
        app: tauri::App<tauri::test::MockRuntime>,
    }

    impl Window {
        fn new(state: AppState) -> Self {
            Self {
                app: crate::stick_guard::testing::app(state),
            }
        }
        fn state(&self) -> Arc<AppState> {
            Arc::clone(&tauri::Manager::state::<Arc<AppState>>(&self.app))
        }
        fn offer(&self) -> LeftoverOffer {
            run(backup_leftovers(tauri::Manager::state(&self.app))).unwrap()
        }
        /// The DJ's hold on the offer's plan.
        fn hold(&self, offer: &LeftoverOffer) -> Option<String> {
            let plan = offer.plan.as_deref().expect("a plan to confirm");
            Some(stick_approval::approve(self.state().backup_dir(), plan).unwrap())
        }
        fn sweep(&self, approval: Option<String>) -> AppResult<Vec<String>> {
            run(sweep_backup_leftovers(
                tauri::Manager::state(&self.app),
                approval,
            ))
        }
    }

    #[test]
    fn the_copy_a_refused_backup_left_is_offered_and_removed_once_the_stick_is_unlocked_and_nothing_else_is(
    ) {
        let (_dir, state, stick) = fixture();
        let left = refused_backup(&state, &stick);
        // Look-alikes this app did not record: kept, byte for byte.
        let recorded_id = left[0]
            .trim_start_matches(".partial-")
            .trim_end_matches(".zip")
            .to_owned();
        let foreign = [
            ".partial-another-process".to_owned(),
            format!(".partial-{}", uuid::Uuid::new_v4()),
            format!(".partial-{recorded_id}.bak"),
            format!(".partial-{}", recorded_id.to_uppercase()),
        ];
        for name in &foreign {
            fs::create_dir(stick.join(name)).unwrap();
            fs::write(stick.join(name).join("keep"), b"theirs").unwrap();
        }
        fs::write(stick.join("partial-notes.txt"), b"keep").unwrap();
        crate::stick_guard::testing::unlock(&state, &stick);
        let kept: Vec<_> = walk(&stick)
            .into_iter()
            .filter(|(path, _)| !left.iter().any(|l| path.starts_with(stick.join(l))))
            .collect();

        let window = Window::new(state);
        let offer = window.offer();
        assert!(offer.on_stick && !offer.protected, "{offer:?}");
        assert_eq!(offer.leftovers, left, "exactly what the backup left");
        let removed = window.sweep(window.hold(&offer)).unwrap();

        assert_eq!(removed, left);
        assert_eq!(walk(&stick), kept, "everything else is left as it was");
        assert_eq!(
            window.offer().leftovers,
            Vec::<String>::new(),
            "nothing more to offer"
        );
        assert_eq!(window.offer().plan, None);
    }

    #[test]
    fn a_locked_stick_is_left_byte_for_byte_unchanged_and_the_refusal_is_shown() {
        let (_dir, state, stick) = fixture();
        let left = refused_backup(&state, &stick);
        let window = Window::new(state);
        // Still locked: the offer says so, and holding it removes nothing.
        let offer = window.offer();
        assert!(offer.protected, "{offer:?}");
        assert_eq!(offer.leftovers, left);
        let before = walk(&stick);
        let refused = window.sweep(window.hold(&offer)).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::StickProtected, "{refused:?}");
        assert_eq!(walk(&stick), before, "not a byte changed");
        // Still recorded: offered again once it is unlocked.
        crate::stick_guard::testing::unlock(&window.state(), &stick);
        let offer = window.offer();
        assert_eq!(offer.leftovers, left);
        assert_eq!(window.sweep(window.hold(&offer)).unwrap(), left);
    }

    #[test]
    fn a_stick_replaced_between_the_offer_and_the_removal_removes_nothing() {
        let (dir, state, stick) = fixture();
        let left = refused_backup(&state, &stick);
        crate::stick_guard::testing::unlock(&state, &stick);
        let window = Window::new(state);
        let offer = window.offer();
        let approval = window.hold(&offer);
        // Taken out; another Stick put in at the same place, unlocked too,
        // even holding folders of the very same names.
        let old = dir.path().join("sticks/GIG-taken-out");
        fs::rename(&stick, &old).unwrap();
        fs::create_dir_all(&stick).unwrap();
        for name in &left {
            fs::create_dir(stick.join(name)).unwrap();
            fs::write(stick.join(name).join("keep"), b"another stick").unwrap();
        }
        crate::stick_guard::testing::unlock(&window.state(), &stick);
        let (old_before, new_before) = (walk(&old), walk(&stick));

        let refused = window.sweep(approval).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Conflict, "{refused:?}");
        assert!(refused.message.contains("not the one"), "{}", refused.message);
        assert_eq!(
            walk(&stick),
            new_before,
            "the Stick put in its place is untouched"
        );
        assert_eq!(walk(&old), old_before, "and so is the one taken out");
    }

    #[test]
    fn nothing_is_removed_without_the_djs_confirmation() {
        let (_dir, state, stick) = fixture();
        let _left = refused_backup(&state, &stick);
        crate::stick_guard::testing::unlock(&state, &stick);
        let window = Window::new(state);
        let offer = window.offer();
        let before = walk(&stick);
        assert!(window.sweep(None).is_err(), "no token");
        // The plan id alone, not confirmed, is no token either.
        assert!(
            window.sweep(offer.plan.clone()).is_err(),
            "an unconfirmed plan"
        );
        assert_eq!(walk(&stick), before);
    }

    #[test]
    fn a_leftover_changed_since_the_offer_is_not_removed_on_that_offer() {
        let (_dir, state, stick) = fixture();
        let left = refused_backup(&state, &stick);
        crate::stick_guard::testing::unlock(&state, &stick);
        let window = Window::new(state);
        let approval = window.hold(&window.offer());
        // One of the offered copies replaced by a link of the same name.
        let target = stick.join("elsewhere");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"keep").unwrap();
        let first = stick.join(&left[0]);
        if first.is_dir() {
            fs::remove_dir_all(&first).unwrap();
        } else {
            fs::remove_file(&first).unwrap();
        }
        std::os::unix::fs::symlink(&target, &first).unwrap();
        let before = walk(&stick);
        assert!(
            window.sweep(approval).is_err(),
            "what is there is not what was offered"
        );
        assert_eq!(walk(&stick), before);
    }

    #[test]
    fn only_this_apps_own_names_are_taken_for_its_own() {
        let id = uuid::Uuid::new_v4().hyphenated().to_string();
        assert_eq!(own_name(&format!(".partial-{id}")), Some(true));
        assert_eq!(own_name(&format!(".partial-{id}.zip")), Some(false));
        for other in [
            ".partial-another-process".to_owned(),
            format!(".partial-{}", id.to_uppercase()),
            format!(".partial-{}", id.replace('-', "")),
            format!(".partial-{id}.bak"),
            format!("partial-{id}"),
            format!(".discard-{id}"),
        ] {
            assert_eq!(own_name(&other), None, "{other}");
        }
    }
}
