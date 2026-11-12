//! The DJ's per-operation confirmation of a write to a Stick (wbx-45.4).
//!
//! This is the window's confirmation, enforced in the backend so that a
//! script or a stale button cannot skip it; it is not the guarantee that a
//! Stick is safe. That is Stick Protection (`stick_guard`, wbx-45.11),
//! which every writer checks first, under the Stick's lock.
//!
//! The hold itself is an interface confirmation: the window times it, and
//! the backend does not. A client that plans and approves at once gets a
//! token (wbx-45.4 review round 4, C1); what it cannot do is write to a
//! Stick the DJ has not unlocked.
//!
//! 1. `stick_write_plan` works out what the write would do without writing
//!    or settling anything, and answers a **plan id**: not usable to write.
//! 2. The DJ looks at the plan and holds to confirm; the window then calls
//!    `stick_write_approve` with the plan id, which checks that the Stick
//!    is still the one planned and unchanged, and answers a **token**.
//!    A cancelled plan is revoked (`stick_write_revoke`).
//! 3. The writer, under the Stick's lock, [`consume`]s the token before
//!    anything touches the Stick, recovery included.
//!
//! A token is bound to the exact operation with every argument that
//! affects what is written (ordered track identities, defaults, formats),
//! to the Stick's mount path and volume identity, to the Stick's revision
//! (a hash of the contents of every file the plan read), to the Library
//! generation the plan read for writes whose contents come from the
//! Library, and to a lifetime. A missing, used, expired or mismatched token
//! is refused with a message that asks for a fresh plan, and is gone
//! afterwards either way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{Result as AppResult, StickWriteError};

/// How long a plan's approval lasts.
pub const APPROVAL_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// One write to a Stick, as the command that performs it describes it:
/// every argument that changes what is written. `defaults` and
/// `compatibility` are as sent (`null` when not sent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StickOperation {
    /// Export Playlist (or Export Folder).
    ExportPlaylist {
        playlist: String,
        delete_unlisted_music: bool,
        defaults: serde_json::Value,
        compatibility: serde_json::Value,
    },
    /// Export Track: tracks in no playlist, in this order.
    ExportTracks {
        tracks: Vec<String>,
        defaults: serde_json::Value,
        compatibility: serde_json::Value,
    },
    /// SYNC of the Sync Manager, for one Stick.
    Sync {
        playlists: Vec<String>,
        delete_unlisted_music: bool,
        automatic: bool,
        defaults: serde_json::Value,
        compatibility: serde_json::Value,
    },
    /// A change to one library's playlists on the Stick, as sent.
    PlaylistEdit {
        format: String,
        edit: serde_json::Value,
    },
    /// Library tracks (in this order) added to one of the Stick's
    /// playlists, those it lacks exported to it first: resolved to the
    /// Stick's own rows by the backend, under the same lock.
    AddLibraryTracks {
        format: String,
        playlist: String,
        tracks: Vec<String>,
        duplicates: bool,
        defaults: serde_json::Value,
        compatibility: serde_json::Value,
    },
    /// Loose files copied to the Stick and added to one of its playlists.
    PlaylistAddFiles {
        format: String,
        playlist: String,
        files: Vec<String>,
    },
    /// The Stick's own settings (DJ System, written to the Stick).
    SaveSettings { settings: serde_json::Value },
    /// The unfinished write on the Stick finished or rolled back.
    Settle,
    /// An empty library put on a blank Stick, from these defaults.
    EnsureLibrary { defaults: serde_json::Value },
    /// The Stick erased and formatted.
    Format { layout: String },
}

impl StickOperation {
    /// Whether what it writes comes from the Library, so a token for it is
    /// bound to the Library generation the plan read.
    #[must_use]
    pub const fn from_library(&self) -> bool {
        matches!(
            self,
            Self::ExportPlaylist { .. }
                | Self::ExportTracks { .. }
                | Self::Sync { .. }
                | Self::AddLibraryTracks { .. }
                | Self::EnsureLibrary { .. }
        )
    }
}

/// `value` as a token binds it: JSON, `null` for none.
pub fn bound<T: Serialize>(value: Option<&T>) -> serde_json::Value {
    value
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// What a token was issued for.
#[derive(Debug, Clone)]
struct Approval {
    stick: PathBuf,
    volume: String,
    revision: String,
    /// The Library generation the plan read; `None` when the write does not
    /// take its contents from the Library.
    generation: Option<u64>,
    issued: Instant,
    op: StickOperation,
    /// Approved by the DJ (`approve`): only then is it a token.
    approved: bool,
    /// What the planned write was worked out to do, and the identities of
    /// what that depends on ([`Bound`]); `None` for a plan that records
    /// only its arguments (the window's).
    bound: Option<Bound>,
}

/// The result a plan was approved for, line by line under named parts
/// (`tracks`, `playlists`, `changes`, `native_stems`, ...): what the write
/// will do and the identity of each input it depends on (a source audio
/// file, a Stem set). A write bound to it is refused unless it works out
/// the same lines again under the Stick's lock (wbx-70).
pub type Bound = std::collections::BTreeMap<String, Vec<String>>;

/// The parts of `now` that differ from `planned`, in words: what is new and
/// what is gone in each, a few lines of each at most.
#[must_use]
pub fn differences(planned: &Bound, now: &Bound) -> Vec<String> {
    const SHOWN: usize = 6;
    let empty = Vec::new();
    let mut out = Vec::new();
    let parts: std::collections::BTreeSet<&String> = planned.keys().chain(now.keys()).collect();
    for part in parts {
        let before = planned.get(part).unwrap_or(&empty);
        let after = now.get(part).unwrap_or(&empty);
        for (word, lines) in [
            (
                "now",
                after
                    .iter()
                    .filter(|l| !before.contains(l))
                    .collect::<Vec<_>>(),
            ),
            (
                "no longer",
                before
                    .iter()
                    .filter(|l| !after.contains(l))
                    .collect::<Vec<_>>(),
            ),
        ] {
            for line in lines.iter().take(SHOWN) {
                out.push(format!("{part}: {word} {line}"));
            }
            if lines.len() > SHOWN {
                out.push(format!("{part}: and {} more {word}", lines.len() - SHOWN));
            }
        }
        if out.is_empty() && before != after {
            out.push(format!("{part}: in another order"));
        }
    }
    out
}

static APPROVALS: LazyLock<Mutex<HashMap<String, Approval>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn approvals() -> MutexGuard<'static, HashMap<String, Approval>> {
    APPROVALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The Stick's execution lock (`wreckordbox_stick_protection::exec_lock`,
/// shared across processes with every writer, a lock and an eject), for a
/// read before a write: planning, confirming, inspecting. Released when
/// dropped. Take the Library edit gate first, if at all.
pub type StickLock = wreckordbox_stick_protection::ExecLock;

/// Takes the execution lock of the Stick at `stick`, waiting for a write
/// already holding it.
pub fn lock(state_dir: &Path, stick: &Path) -> AppResult<StickLock> {
    let root = wreckordbox_stick_protection::device_root(stick)
        .unwrap_or_else(|| stick.canonicalize().unwrap_or_else(|_| stick.to_path_buf()));
    wreckordbox_stick_protection::exec_lock(state_dir, &root).map_err(|e| {
        StickWriteError::Internal {
            message: "The Stick could not be reserved.".to_owned(),
            detail: Some(e.to_string()),
        }
    })
}

/// The mounted directory's identity: another volume at the same path differs.
pub fn volume_identity(stick: &Path) -> AppResult<String> {
    let meta = std::fs::metadata(stick).map_err(|_| gone())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("dev {} ino {}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!("created {:?}", meta.created().ok()))
    }
}

/// A fingerprint of what a plan reads on the Stick, by content: the
/// journal of an unfinished write and the marker at the root, and every
/// file of the export's `rekordbox` folder, its manifest and the settings
/// beside it, each by a hash of its bytes. A change that keeps a file's
/// size and modification time is still a change.
pub fn stick_revision(stick: &Path) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut seen: Vec<String> = Vec::new();
    let mut note = |path: &Path| {
        let rel = path
            .strip_prefix(stick)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut hasher);
                seen.push(format!("{rel}:{}:{:016x}", bytes.len(), hasher.finish()));
            }
            Err(_) => seen.push(format!("{rel}:-")),
        }
    };
    for name in [".rbxport-publication", ".rbxport-disposable"] {
        note(&stick.join(name));
    }
    if let Ok(entries) = std::fs::read_dir(stick) {
        let mut roots: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        roots.sort();
        for root in roots {
            let mut files = Vec::new();
            for name in [
                "DEVSETTING.DAT",
                "MYSETTING.DAT",
                "MYSETTING2.DAT",
                "DJMMYSETTING.DAT",
                "rbxport/manifest.json",
            ] {
                files.push(root.join(name));
            }
            if let Ok(inner) = std::fs::read_dir(root.join("rekordbox")) {
                files.extend(inner.flatten().map(|e| e.path()).filter(|p| p.is_file()));
            }
            files.sort();
            for file in files {
                if file.exists() {
                    note(&file);
                }
            }
        }
    }
    seen.join("\n")
}

fn gone() -> StickWriteError {
    StickWriteError::NotFound(
        "That device is no longer connected. It may have been unplugged or renamed.".to_owned(),
    )
}

fn refused(why: &str) -> StickWriteError {
    StickWriteError::Conflict(format!(
        "{why} Nothing was written to the Stick. Look at what the write would do again and confirm it."
    ))
}

/// [`propose`], the plan also bound to the result it was worked out to
/// have (`bound`): a write that carries it must work out the same result
/// ([`consume_bound`]).
pub fn propose_bound(
    stick: &Path,
    op: StickOperation,
    generation: Option<u64>,
    bound: Bound,
) -> AppResult<String> {
    let id = propose(stick, op, generation)?;
    if let Some(plan) = approvals().get_mut(&id) {
        plan.bound = Some(bound);
    }
    Ok(id)
}

/// Records a plan for `op` on the Stick at `stick`, bound to what is
/// there now, and answers its plan id: not a token. Called by the planners
/// alone: the window's `stick_write_plan` and the service's `sticks.dry_run`.
pub fn propose(stick: &Path, op: StickOperation, generation: Option<u64>) -> AppResult<String> {
    let approval = Approval {
        stick: stick.to_path_buf(),
        volume: volume_identity(stick)?,
        revision: stick_revision(stick),
        generation: if op.from_library() { generation } else { None },
        issued: Instant::now(),
        op,
        approved: false,
        bound: None,
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut all = approvals();
    all.retain(|_, a| a.issued.elapsed() < APPROVAL_LIFETIME);
    all.insert(id.clone(), approval);
    Ok(id)
}

/// The DJ confirmed the plan `plan` (the window's hold): answers the token
/// its writer needs, bound as the plan was. The plan id is used up; a
/// Stick moved, replaced or changed since the plan is refused.
pub fn approve(state_dir: &Path, plan: &str) -> AppResult<String> {
    let Some(mut approval) = approvals().remove(plan).filter(|a| !a.approved) else {
        return Err(refused(
            "This plan was already confirmed, cancelled, or is unknown.",
        ));
    };
    if approval.issued.elapsed() >= APPROVAL_LIFETIME {
        return Err(refused("This plan has expired."));
    }
    let _lock = lock(state_dir, &approval.stick)?;
    if volume_identity(&approval.stick)? != approval.volume {
        return Err(refused(
            "The Stick at this place is not the one the plan was made for.",
        ));
    }
    if stick_revision(&approval.stick) != approval.revision {
        return Err(refused("The Stick changed since the plan was made."));
    }
    approval.approved = true;
    approval.issued = Instant::now();
    let token = uuid::Uuid::new_v4().simple().to_string();
    approvals().insert(token.clone(), approval);
    Ok(token)
}

/// The DJ cancelled the plan `plan`: it can never be confirmed.
pub fn revoke(plan: &str) {
    approvals().retain(|id, a| id != plan || a.approved);
}

/// The plan `plan`, while it waits for the DJ: for tests.
#[doc(hidden)]
#[must_use]
pub fn planned(plan: &str) -> Option<StickOperation> {
    approvals()
        .get(plan)
        .filter(|a| !a.approved)
        .map(|a| a.op.clone())
}

/// Checks and uses up the token for `op` on the Stick at `stick`. Call it
/// holding the Stick's lock (`stick_guard`), before any recovery or write.
/// `generation` is the generation of the Library snapshot the write will
/// take its contents from.
pub fn consume(
    token: Option<&str>,
    op: &StickOperation,
    stick: &Path,
    generation: u64,
) -> AppResult<()> {
    take(token, op, stick, generation).map(drop)
}

/// [`consume`] for a write bound to its result ([`propose_bound`]): also
/// refused when the plan records no result, or when `now`, the result the
/// write works out under the Stick's lock, differs from the one approved
/// ([`StickWriteError::Changed`], saying what changed).
pub fn consume_bound(
    token: Option<&str>,
    op: &StickOperation,
    stick: &Path,
    generation: u64,
    now: &Bound,
) -> AppResult<()> {
    let approval = take(token, op, stick, generation)?;
    let Some(planned) = approval.bound else {
        return Err(refused(
            "This plan does not record what the write would do (it was not made by a dry run of this write).",
        ));
    };
    let changed = differences(&planned, now);
    if changed.is_empty() {
        Ok(())
    } else {
        Err(StickWriteError::Changed(changed))
    }
}

/// Checks and takes out the token for `op` on the Stick at `stick`.
fn take(
    token: Option<&str>,
    op: &StickOperation,
    stick: &Path,
    generation: u64,
) -> AppResult<Approval> {
    if !stick.is_dir() {
        return Err(gone());
    }
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Err(refused("This write to the Stick was not confirmed."));
    };
    // Taken out whatever happens next: a token is good for one try.
    let Some(approval) = approvals().remove(token) else {
        return Err(refused(
            "This confirmation was already used, or is unknown.",
        ));
    };
    if !approval.approved {
        return Err(refused(
            "This write to the Stick was planned but not confirmed.",
        ));
    }
    if approval.issued.elapsed() >= APPROVAL_LIFETIME {
        return Err(refused("This confirmation has expired."));
    }
    if approval.stick != stick {
        return Err(refused("This confirmation was for another Stick."));
    }
    if volume_identity(stick)? != approval.volume {
        return Err(refused(
            "The Stick at this place is not the one the write was confirmed for.",
        ));
    }
    if stick_revision(stick) != approval.revision {
        return Err(refused("The Stick changed since the write was confirmed."));
    }
    if approval.op != *op {
        return Err(refused("This confirmation was for a different write."));
    }
    if op.from_library() && approval.generation.is_some_and(|g| g != generation) {
        return Err(refused(
            "The Library changed since the write was confirmed.",
        ));
    }
    Ok(approval)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stick() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("PIONEER/rekordbox")).unwrap();
        std::fs::write(dir.path().join("PIONEER/rekordbox/export.pdb"), b"one").unwrap();
        dir
    }
    fn export(tracks: &[&str]) -> StickOperation {
        StickOperation::ExportTracks {
            tracks: tracks.iter().map(|t| (*t).to_owned()).collect(),
            defaults: serde_json::Value::Null,
            compatibility: serde_json::Value::Null,
        }
    }
    fn token(stick: &Path, op: &StickOperation, generation: u64) -> String {
        approve(
            &std::env::temp_dir(),
            &propose(stick, op.clone(), Some(generation)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn a_plan_id_is_not_a_token_and_a_token_is_good_once() {
        let dir = stick();
        let op = export(&["1"]);
        assert!(consume(None, &op, dir.path(), 0).is_err());
        let plan = propose(dir.path(), op.clone(), Some(0)).unwrap();
        let unconfirmed = consume(Some(&plan), &op, dir.path(), 0).err().unwrap();
        assert!(
            unconfirmed.to_string().contains("not confirmed"),
            "{unconfirmed}"
        );
        let plan = propose(dir.path(), op.clone(), Some(0)).unwrap();
        let token = approve(&std::env::temp_dir(), &plan).unwrap();
        assert!(
            approve(&std::env::temp_dir(), &plan).is_err(),
            "a plan is confirmed once"
        );
        consume(Some(&token), &op, dir.path(), 0).unwrap();
        let again = consume(Some(&token), &op, dir.path(), 0).err().unwrap();
        assert!(again.to_string().contains("already used"), "{again}");
    }

    #[test]
    fn a_cancelled_plan_cannot_be_confirmed() {
        let dir = stick();
        let plan = propose(dir.path(), export(&["1"]), Some(0)).unwrap();
        revoke(&plan);
        assert!(approve(&std::env::temp_dir(), &plan).is_err());
    }

    #[test]
    fn a_token_binds_every_argument_its_order_the_stick_the_library_and_the_contents() {
        let dir = stick();
        let op = export(&["1", "2"]);
        let other = stick();
        let t = token(dir.path(), &op, 0);
        assert!(
            consume(Some(&t), &export(&["2", "1"]), dir.path(), 0).is_err(),
            "another order"
        );
        let t = token(dir.path(), &op, 0);
        let mut defaults = op.clone();
        if let StickOperation::ExportTracks { defaults: d, .. } = &mut defaults {
            *d = serde_json::json!({ "language": "fr" });
        }
        assert!(
            consume(Some(&t), &defaults, dir.path(), 0).is_err(),
            "other defaults"
        );
        let t = token(dir.path(), &op, 0);
        assert!(
            consume(Some(&t), &op, other.path(), 0).is_err(),
            "another Stick"
        );
        let t = token(dir.path(), &op, 0);
        assert!(
            consume(Some(&t), &op, dir.path(), 1).is_err(),
            "the Library changed"
        );
        let t = token(dir.path(), &op, 0);
        std::fs::write(dir.path().join(".rbxport-publication"), b"{}").unwrap();
        assert!(
            consume(Some(&t), &op, dir.path(), 0).is_err(),
            "an unfinished write appeared"
        );
    }

    #[test]
    fn a_change_that_keeps_size_and_modification_time_is_still_a_change() {
        let dir = stick();
        let file = dir.path().join("PIONEER/rekordbox/export.pdb");
        let when = std::fs::metadata(&file).unwrap().modified().unwrap();
        let op = export(&["1"]);
        let t = token(dir.path(), &op, 0);
        std::fs::write(&file, b"two").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(when)
            .unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), when);
        let refused = consume(Some(&t), &op, dir.path(), 0).err().unwrap();
        assert!(refused.to_string().contains("changed"), "{refused}");
    }

    #[test]
    fn a_stick_changed_between_plan_and_confirmation_is_refused_at_confirmation() {
        let dir = stick();
        let plan = propose(dir.path(), export(&["1"]), Some(0)).unwrap();
        std::fs::write(dir.path().join("PIONEER/rekordbox/export.pdb"), b"changed").unwrap();
        assert!(approve(&std::env::temp_dir(), &plan).is_err());
    }

    #[test]
    fn another_volume_at_the_same_place_is_refused() {
        let parent = tempfile::tempdir().unwrap();
        let mount = parent.path().join("STICK");
        std::fs::create_dir_all(mount.join("PIONEER/rekordbox")).unwrap();
        let op = StickOperation::Settle;
        let t = token(&mount, &op, 0);
        std::fs::remove_dir_all(&mount).unwrap();
        std::fs::create_dir_all(mount.join("PIONEER/rekordbox")).unwrap();
        let refused = consume(Some(&t), &op, &mount, 0).err().unwrap();
        assert!(refused.to_string().contains("not the one"), "{refused}");
    }

    #[test]
    fn an_expired_token_is_refused() {
        let dir = stick();
        let op = StickOperation::Settle;
        let t = token(dir.path(), &op, 0);
        approvals().get_mut(&t).unwrap().issued =
            Instant::now().checked_sub(APPROVAL_LIFETIME).unwrap();
        let expired = consume(Some(&t), &op, dir.path(), 0).err().unwrap();
        assert!(expired.to_string().contains("expired"), "{expired}");
    }

    fn bound(lines: &[(&str, &[&str])]) -> Bound {
        lines
            .iter()
            .map(|(part, l)| {
                (
                    (*part).to_owned(),
                    l.iter().map(|x| (*x).to_owned()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_bound_plan_writes_only_the_result_it_was_approved_for() {
        let dir = stick();
        let op = export(&["1"]);
        let planned = bound(&[("native_stems", &[]), ("tracks", &["#1 A: size 3"])]);
        let token = |b: &Bound| {
            approve(
                &std::env::temp_dir(),
                &propose_bound(dir.path(), op.clone(), Some(0), b.clone()).unwrap(),
            )
            .unwrap()
        };
        let t = token(&planned);
        consume_bound(Some(&t), &op, dir.path(), 0, &planned).unwrap();

        let t = token(&planned);
        let now = bound(&[
            ("native_stems", &["#1 A: stems 7"]),
            ("tracks", &["#1 A: size 4"]),
        ]);
        let Err(StickWriteError::Changed(lines)) =
            consume_bound(Some(&t), &op, dir.path(), 0, &now)
        else {
            panic!("a changed result is refused");
        };
        assert!(
            lines.contains(&"native_stems: now #1 A: stems 7".to_owned()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"tracks: no longer #1 A: size 3".to_owned()),
            "{lines:?}"
        );

        // A plan that records no result never authorises a bound write.
        let t = approve(
            &std::env::temp_dir(),
            &propose(dir.path(), op.clone(), Some(0)).unwrap(),
        )
        .unwrap();
        assert!(consume_bound(Some(&t), &op, dir.path(), 0, &planned).is_err());
    }
}
