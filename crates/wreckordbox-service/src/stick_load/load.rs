//! The dry run and the load.
//!
//! The lock order is the window's (`stick_guard`): the service's hold on
//! the Library, then the Library core's edit gate while the selection is
//! read, then the Stick's execution lock (shared across processes with the
//! window, the CLI, a lock and an eject). The gate is let go before the
//! long write; the execution lock is held to the end of it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use wreckordbox_stick_protection::{Refused, Volume};
use wreckordbox_stick_write::approval::{self, Bound, StickOperation};
use wreckordbox_stick_write::plan::{export_plan, ExportPlan, PlaylistChange, EXPORT_CACHE};
use wreckordbox_stick_write::selection::{ExportSelection, LibrarySource};
use wreckordbox_stick_write::StickWriteError;

use super::sticks;
use super::{
    codes, events, ChangedTrack, Changes, Chosen, Counts, DryRun, DryRunArgs, Held, Load, LoadArgs,
    LoadError, Loads, NativeStemsLine, PlaylistChanges, RewrittenPlaylist, Space, TrackChanges,
    TrackLine,
};
use crate::library::OpenLibrary;
use crate::native_stems::{self, CurrentStems, Found};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::Stems;

/// Runs a closure on the service's Library, under its hold.
pub(crate) type WithLibrary<'a> =
    dyn Fn(&mut dyn FnMut(&OpenLibrary)) -> Result<(), ServiceError> + 'a;

fn with_library<T>(
    with: &WithLibrary<'_>,
    f: impl FnOnce(&OpenLibrary) -> Result<T, ServiceError>,
) -> Result<T, ServiceError> {
    let mut f = Some(f);
    let mut out = None;
    with(&mut |library| {
        if let Some(f) = f.take() {
            out = Some(f(library));
        }
    })?;
    out.unwrap_or_else(|| {
        Err(ServiceError::new(
            service_codes::SERVICE_FAILED,
            "the Library was not read",
        ))
    })
}

/// In test mode only, a file of this name in the state folder holding a
/// number stops the next load after that many progress steps: a load that
/// fails once it has started writing. Read, and removed, by each load.
pub const TEST_FAIL_AFTER_FILE: &str = "test-stick-load-fail-after";
/// In test mode only, a file of this name in the state folder holding an
/// export stage (`copying`, `publishing`, ...) pauses the next load at that
/// stage, after writing [`TEST_PAUSED_FILE`], until the file is removed.
pub const TEST_PAUSE_AT_FILE: &str = "test-stick-load-pause-at";
/// Written by a load paused by [`TEST_PAUSE_AT_FILE`].
pub const TEST_PAUSED_FILE: &str = "test-stick-load-paused";

/// About what one second of one Native stem takes: Ogg Vorbis at quality
/// 5 is some 160 kbit/s for stereo.
const STEM_BYTES_PER_SECOND: u64 = 20_000;
/// Each Stem file's headers and last page, at most, beyond that.
const STEM_FILE_OVERHEAD: u64 = 8 * 1024;

/// The write a load is, as the confirmation binds it: a sync of these
/// playlists, nothing deleted that the Sync Manager would keep, the Stick's
/// own settings kept.
fn operation(playlists: &[String]) -> StickOperation {
    StickOperation::Sync {
        playlists: playlists.to_vec(),
        delete_unlisted_music: false,
        automatic: false,
        defaults: serde_json::Value::Null,
        compatibility: serde_json::Value::Null,
    }
}

fn arguments<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, ServiceError> {
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

/// The root of the Stick at `stick`, refused when it is not one.
fn stick_root(stick: &Path) -> Result<PathBuf, ServiceError> {
    if !stick.is_absolute() {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            format!(
                "{} is not an absolute path: say where the Stick is mounted in full",
                stick.display()
            ),
        ));
    }
    if stick
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            format!(
                "{} climbs with `..`: say where the Stick is mounted, plainly",
                stick.display()
            ),
        ));
    }
    if !stick.is_dir() {
        return Err(ServiceError::new(
            crate::native_stems::codes::STICK_NOT_FOUND,
            format!("there is no Stick at {}", stick.display()),
        ));
    }
    match wreckordbox_stick_protection::classify(stick) {
        Volume::Stick { root } if stick.canonicalize().ok().as_ref() == Some(&root) => Ok(root),
        Volume::Stick { root } => Err(ServiceError::new(
            codes::NOT_A_STICK,
            format!(
                "{} is a folder inside the Stick at {}: name the Stick itself",
                stick.display(),
                root.display()
            ),
        )),
        Volume::Ordinary => Err(ServiceError::new(
            codes::NOT_A_STICK,
            format!(
                "{} is not a Stick: only a mounted Stick is loaded",
                stick.display()
            ),
        )),
    }
}

fn chosen(playlists: &[String]) -> Result<(), ServiceError> {
    if playlists.is_empty() {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            "choose at least one playlist to load",
        ));
    }
    Ok(())
}

/// A shared planner's refusal in the service's codes.
fn planning(error: StickWriteError) -> ServiceError {
    match error {
        StickWriteError::NotFound(message) => {
            ServiceError::new(crate::tracks::PLAYLIST_NOT_FOUND, message)
        }
        StickWriteError::Conflict(message) => {
            ServiceError::new(codes::STICK_LOAD_UNCONFIRMED, message)
        }
        changed @ StickWriteError::Changed(_) => {
            ServiceError::new(codes::STICK_LOAD_CHANGED, changed.to_string())
        }
        StickWriteError::Read {
            error: wreckordbox_stick_outline::ReadError::UnfinishedWrite,
            ..
        } => unfinished(),
        other => ServiceError::new(codes::STICK_LOAD_FAILED, other.to_string()),
    }
}

fn unfinished() -> ServiceError {
    ServiceError::new(
        codes::STICK_UNFINISHED_WRITE,
        "a write to this Stick did not finish, and nothing on it has been changed: open the Stick in the window and choose \"Finish or roll back the unfinished write\" first",
    )
}

fn protection_refused(refused: &Refused) -> ServiceError {
    let code = match refused {
        Refused::Protected { .. } => service_codes::STICK_PROTECTED,
        _ => crate::native_stems::codes::STICK_NOT_FOUND,
    };
    ServiceError::new(code, refused.to_string())
}

fn line(id: u64, title: &str) -> TrackLine {
    TrackLine {
        id: if id == 0 {
            String::new()
        } else {
            id.to_string()
        },
        title: title.to_owned(),
    }
}

/// A Native stems set a load writes: the track, and the set as it was
/// planned.
struct PlannedStems {
    found: Found,
    current: CurrentStems,
    /// What the set it replaces on the Stick takes, if there is one.
    replaced_bytes: u64,
}

/// Everything a dry run reports and a load does, worked out from one
/// snapshot of the Library and the Stick as they are.
struct Planned {
    playlists: Vec<Chosen>,
    selection: ExportSelection,
    changes: Changes,
    space: Space,
    stems: Vec<PlannedStems>,
    /// What the load writes and the identity of each input it depends on:
    /// what its plan id is bound to.
    bound: Bound,
}

/// Works out a load of `playlists` to the Stick at `root`. Call holding
/// the Library's edit gate and the Stick's execution lock; writes nothing,
/// to the Stick, the Library or the Sidecar.
///
/// wbx-83 (Mutants rendered at load time) belongs here: render the chosen
/// playlists' Mutants first, then substitute each rendered file for its
/// entry's track in `selection` before it is planned, so the plan, the
/// bound result, the confirmation and the export all see the rendered
/// audio (and the rendered file's identity joins `tracks` in the bound).
fn plan_load(
    state_dir: &Path,
    library: &OpenLibrary,
    stems: &Stems,
    index: &rbl_index::Library,
    root: &Path,
    playlists: &[String],
) -> Result<Planned, ServiceError> {
    let share = library.share_root();
    let selection = ExportSelection::from_playlists(library, index, &share, playlists, false)
        .map_err(planning)?;
    let selection = selection
        .for_stick_inspected(library, index, &share, root, false)
        .map_err(planning)?
        .into_owned();
    let plan = export_plan(library, index, root, &selection, None).map_err(planning)?;

    // Native stems, on a Spare stick only: each track loaded that has a
    // complete set of current Stems, read without opening a writer.
    let mut planned_stems = Vec::new();
    if wreckordbox_native_stems::is_spare_stick(root) {
        let missing: HashSet<u64> = plan.missing.iter().map(|t| t.id).collect();
        let mut seen = HashSet::new();
        for track in &selection.tracks {
            if track.id == 0 || missing.contains(&track.id) || !seen.insert(track.id) {
                continue;
            }
            let Ok(found) = native_stems::parent(library, &track.id.to_string()) else {
                continue;
            };
            if let Some(current) = native_stems::current_stems_read_only(stems, &found) {
                planned_stems.push(PlannedStems {
                    replaced_bytes: replaced_set_bytes(state_dir, root, track.id),
                    found,
                    current,
                });
            }
        }
    }
    let names = index.playlists();
    let chosen = playlists
        .iter()
        .map(|id| Chosen {
            id: id.clone(),
            name: id
                .parse::<u64>()
                .ok()
                .and_then(|n| names.index_of(n))
                .map_or_else(String::new, |i| names.name(i).to_owned()),
        })
        .collect();
    let changes = changes(&plan, &planned_stems);
    let space = space(state_dir, root, &plan, &planned_stems);
    let mut bound = Bound::new();
    bound.insert("tracks".to_owned(), track_identities(&selection));
    bound.insert("playlists".to_owned(), playlist_lines(&selection));
    bound.insert("changes".to_owned(), change_lines(&changes));
    bound.insert(
        "native_stems".to_owned(),
        planned_stems.iter().map(stems_line).collect(),
    );
    Ok(Planned {
        playlists: chosen,
        selection,
        changes,
        space,
        stems: planned_stems,
        bound,
    })
}

/// A file's identity: another file at the same path, or the same file
/// rewritten, is another identity.
fn file_identity(path: &Path) -> String {
    let Ok(metadata) = std::fs::metadata(path) else {
        return "missing".to_owned();
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt as _;
        metadata.ino()
    };
    #[cfg(not(unix))]
    let inode = 0;
    format!(
        "{} bytes, modified {modified}ns, inode {inode}",
        metadata.len()
    )
}

/// Each track the load writes, with its source audio file's identity.
fn track_identities(selection: &ExportSelection) -> Vec<String> {
    let mut seen = HashSet::new();
    selection
        .tracks
        .iter()
        .filter(|t| {
            seen.insert(rbl_export::track_key(
                t.id,
                &t.source_path.to_string_lossy(),
            ))
        })
        .map(|t| {
            format!(
                "#{} \"{}\" from {} ({})",
                t.id,
                t.title,
                t.source_path.display(),
                file_identity(&t.source_path)
            )
        })
        .collect()
}

/// Each playlist and folder the load writes, with its entries in order.
fn playlist_lines(selection: &ExportSelection) -> Vec<String> {
    selection
        .playlists
        .iter()
        .map(|p| {
            if p.folder {
                format!("folder {}", p.name)
            } else {
                let entries: Vec<String> = p
                    .track_indices
                    .iter()
                    .filter_map(|i| selection.tracks.get(*i))
                    .map(|t| format!("#{}", t.id))
                    .collect();
                format!("playlist {}: {}", p.name, entries.join(" "))
            }
        })
        .collect()
}

fn stems_line(planned: &PlannedStems) -> String {
    format!(
        "#{} \"{}\": {}",
        planned.found.track().id,
        planned.found.track().title,
        planned.current.identity
    )
}

/// The changes, one line each: what a plan id is bound to.
fn change_lines(changes: &Changes) -> Vec<String> {
    let t = |kind: &str, line: &TrackLine| format!("{kind} track #{} \"{}\"", line.id, line.title);
    let mut out = Vec::new();
    out.extend(changes.tracks.added.iter().map(|l| t("add", l)));
    out.extend(changes.tracks.changed.iter().map(|c| {
        format!(
            "change track #{} \"{}\": {}",
            c.id,
            c.title,
            c.fields.join(", ")
        )
    }));
    out.extend(
        changes
            .tracks
            .audio_changed
            .iter()
            .map(|l| t("new audio for", l)),
    );
    out.extend(changes.tracks.removed.iter().map(|l| t("remove", l)));
    out.extend(changes.tracks.missing.iter().map(|l| t("missing", l)));
    out.extend(
        changes
            .playlists
            .created
            .iter()
            .map(|n| format!("create playlist {n}")),
    );
    out.extend(changes.playlists.rewritten.iter().map(|p| {
        format!(
            "write playlist {} again: +{:?} -{:?} reordered {:?}",
            p.name, p.added, p.removed, p.reordered
        )
    }));
    out.extend(
        changes
            .playlists
            .removed
            .iter()
            .map(|n| format!("remove playlist {n}")),
    );
    out.extend(
        changes
            .native_stems
            .iter()
            .map(|l| t("native stems for", l)),
    );
    out
}

fn changes(plan: &ExportPlan, stems: &[PlannedStems]) -> Changes {
    let lines = |tracks: &[wreckordbox_stick_write::plan::PlannedTrack]| -> Vec<TrackLine> {
        tracks.iter().map(|t| line(t.id, &t.title)).collect()
    };
    let tracks = TrackChanges {
        added: lines(&plan.added),
        changed: plan
            .track_changes
            .iter()
            .map(|c| ChangedTrack {
                id: if c.id == 0 {
                    String::new()
                } else {
                    c.id.to_string()
                },
                title: c.title.clone(),
                fields: c.fields.clone(),
            })
            .collect(),
        audio_changed: lines(&plan.audio_changed),
        removed: lines(&plan.removed),
        missing: lines(&plan.missing),
    };
    let mut playlists = PlaylistChanges {
        removed: plan.playlists_removed.clone(),
        ..PlaylistChanges::default()
    };
    for p in plan.playlists.iter().filter(|p| !p.folder) {
        match p.change {
            PlaylistChange::Created => playlists.created.push(p.name.clone()),
            PlaylistChange::Resynchronised => {
                if p.reordered == Some(true) {
                    playlists.reordered.push(p.name.clone());
                }
                playlists.rewritten.push(RewrittenPlaylist {
                    name: p.name.clone(),
                    added: p.added,
                    removed: p.removed,
                    reordered: p.reordered,
                });
            }
        }
    }
    let native_stems: Vec<TrackLine> = stems
        .iter()
        .map(|s| TrackLine {
            id: s.found.track().id.clone(),
            title: s.found.track().title.clone(),
        })
        .collect();
    let counts = Counts {
        tracks_added: tracks.added.len(),
        tracks_changed: tracks.changed.len(),
        audio_changed: tracks.audio_changed.len(),
        tracks_removed: tracks.removed.len(),
        tracks_missing: tracks.missing.len(),
        audio_reused: plan.audio_reused,
        playlists_created: playlists.created.len(),
        playlists_rewritten: playlists.rewritten.len(),
        playlists_reordered: playlists.reordered.len(),
        playlists_removed: playlists.removed.len(),
        native_stems: native_stems.len(),
    };
    Changes {
        counts,
        tracks,
        playlists,
        native_stems,
        analysis_written: plan.analysis_written,
        audio_estimated: plan.audio_estimated,
    }
}

/// `bytes` in whole clusters of `unit`; an empty file takes none.
fn clusters(bytes: u64, unit: u64) -> u64 {
    bytes.div_ceil(unit).saturating_mul(unit)
}

/// The Stick's `STEMS` folder, found as the writer finds it: ignoring ASCII
/// case.
fn stems_folder(root: &Path) -> Option<PathBuf> {
    std::fs::read_dir(root).ok()?.flatten().find_map(|entry| {
        let name = entry.file_name();
        (name.to_string_lossy().eq_ignore_ascii_case("STEMS")
            && entry.file_type().is_ok_and(|t| t.is_dir()))
        .then(|| entry.path())
    })
}

/// What the Native stems set the track already has on the Stick takes, in
/// whole clusters: the folder the deck finds it by (the key of the audio
/// the Stick's record holds for it, with the layout's `_` form), its files
/// and the folder itself. Zero when it has none.
fn replaced_set_bytes(state_dir: &Path, root: &Path, track: u64) -> u64 {
    let unit = sticks::cluster_bytes(state_dir, root);
    let Some(stems) = stems_folder(root) else {
        return 0;
    };
    let Some(audio) = rbl_export::Manifest::load(root).and_then(|m| {
        m.tracks
            .into_iter()
            .find(|t| t.library_id == track)
            .map(|t| t.audio)
    }) else {
        return 0;
    };
    let Some(key) = Path::new(&audio)
        .file_stem()
        .map(|k| k.to_string_lossy().into_owned())
    else {
        return 0;
    };
    [key.clone(), format!("{key}_")]
        .iter()
        .map(|name| stems.join(name))
        .find(|folder| folder.is_dir())
        .map_or(0, |folder| {
            std::fs::read_dir(&folder).map_or(0, |entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| clusters(m.len(), unit))
                    .sum::<u64>()
                    + unit
            })
        })
}

/// The room a load needs and leaves: the export's own reckoning, then the
/// Native stems written one set at a time after it.
///
/// A set takes its four files and its `source.json` in whole clusters, and
/// its folder; while it is written it is a working folder beside the set it
/// replaces (and a note naming that set), which goes only once the new one
/// is in; a new `STEMS` folder takes a cluster. So the most a set needs at
/// once is the new set whole with the old one still there.
fn space(state_dir: &Path, root: &Path, plan: &ExportPlan, stems: &[PlannedStems]) -> Space {
    let free_before = sticks::free_bytes(state_dir, root);
    let unit = sticks::cluster_bytes(state_dir, root);
    let export_net = i128::from(plan.net_additional_bytes);
    let mut running = 0_i128; // stems' net so far, after the export
    let mut stems_peak = 0_i128;
    if !stems.is_empty() && stems_folder(root).is_none() {
        running += i128::from(unit);
    }
    for set in stems {
        let seconds = set.current.samples.div_ceil(44_100).max(1);
        let file = seconds * STEM_BYTES_PER_SECOND + STEM_FILE_OVERHEAD;
        let whole = 4 * clusters(file, unit) + unit /* source.json */ + unit /* folder */;
        let note = if set.replaced_bytes > 0 { unit } else { 0 };
        stems_peak = stems_peak.max(running + i128::from(whole + note));
        running += i128::from(whole) - i128::from(set.replaced_bytes);
    }
    stems_peak = stems_peak.max(running);
    let needed = i128::from(plan.peak_bytes)
        .max(export_net + stems_peak)
        .max(0);
    let needed_bytes = u64::try_from(needed).unwrap_or(u64::MAX);
    let net = export_net + running;
    Space {
        free_before,
        needed_bytes,
        free_after: free_before
            .map(|free| u64::try_from((i128::from(free) - net).max(0)).unwrap_or(u64::MAX)),
        shortfall_bytes: free_before
            .filter(|free| needed_bytes > *free)
            .map(|free| needed_bytes - free),
        native_stems_bytes: u64::try_from(stems_peak.max(0)).unwrap_or(u64::MAX),
        approximate: plan.size_uncertain || !stems.is_empty(),
    }
}

/// Answers `sticks.list`.
pub(crate) fn list(state_dir: &Path) -> super::Sticks {
    super::Sticks {
        sticks: sticks::list(state_dir),
    }
}

/// Why a plan that could not be worked out in full cannot be loaded.
fn incomplete(root: &Path) -> String {
    format!(
        "the database on {} is in WAL mode or has SQLite files beside it (rekordbox or a player had it open), so what a load would copy cannot be read without writing to the Stick; nothing can be loaded until rekordbox has closed it (open and eject it in rekordbox, or load it once from the window)",
        root.display()
    )
}

/// Answers `sticks.dry_run`: what the load would do, and the plan id its
/// load must carry, bound to exactly that. Writes nothing to the Stick, the
/// Library or the Sidecar.
pub(crate) fn dry_run(
    state_dir: &Path,
    stems: &Stems,
    request: &Request,
    with: &WithLibrary<'_>,
) -> Result<DryRun, ServiceError> {
    let args: DryRunArgs = arguments(request)?;
    chosen(&args.playlists)?;
    let root = stick_root(&args.stick)?;
    with_library(with, |library| {
        // As the window's planner: the edit gate, then the Stick's lock;
        // nothing writes to the Stick or edits the Library meanwhile.
        let _gate = library.edit_gate().lock();
        let _lock = approval::lock(state_dir, &root).map_err(planning)?;
        let _lease = rbl_core::durable::read_lease(&root)
            .map_err(|e| ServiceError::new(codes::STICK_LOAD_FAILED, e.to_string()))?;
        if rbl_export::recovery_pending(&root) {
            return Err(unfinished());
        }
        let (index, generation) = library.index_at_generation()?;
        let planned = plan_load(state_dir, library, stems, &index, &root, &args.playlists)?;
        // A plan that could not be worked out in full is never executable.
        let (plan, not_loadable) = if planned.changes.audio_estimated {
            let id = approval::propose_bound(
                &root,
                operation(&args.playlists),
                Some(generation),
                planned.bound,
            )
            .map_err(planning)?;
            (Some(id), None)
        } else {
            (None, Some(incomplete(&root)))
        };
        Ok(DryRun {
            stick: sticks::describe(state_dir, &root),
            playlists: planned.playlists,
            changes: planned.changes,
            space: planned.space,
            plan,
            not_loadable,
        })
    })
}

/// A test-mode pause at `stage`, armed by [`TEST_PAUSE_AT_FILE`].
fn pause_for_test(state_dir: &Path, armed: Option<&str>, stage: &str) {
    if armed != Some(stage) {
        return;
    }
    let pause = state_dir.join(TEST_PAUSE_AT_FILE);
    let _ = std::fs::write(state_dir.join(TEST_PAUSED_FILE), stage);
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while pause.exists() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(state_dir.join(TEST_PAUSED_FILE));
}

/// A test-mode hook's value, read and removed.
fn test_hook(state_dir: &Path, file: &str) -> Option<String> {
    if !sticks::testing() {
        return None;
    }
    let path = state_dir.join(file);
    let value = std::fs::read_to_string(&path).ok()?;
    if file != TEST_PAUSE_AT_FILE {
        let _ = std::fs::remove_file(&path);
    }
    Some(value.trim().to_owned())
}

/// Answers `sticks.load`.
#[allow(
    clippy::too_many_lines,
    reason = "the load's steps in order: protection, confirmation, room, export, Native stems, read back"
)]
pub(crate) fn load(
    state_dir: &Path,
    stems: &Arc<Stems>,
    request: &Request,
    with: &WithLibrary<'_>,
    loads: &Loads,
) -> Result<Load, ServiceError> {
    let args: LoadArgs = arguments(request)?;
    chosen(&args.playlists)?;
    let root = stick_root(&args.stick)?;
    if loads.stopping() {
        return Err(ServiceError::new(
            service_codes::SERVICE_FAILED,
            "the Library service is stopping; nothing was written. Make a new dry run",
        ));
    }
    let (authorized, planned) = with_library(with, |library| {
        // The lock order: the edit gate while the selection is read; then
        // Stick Protection, which takes the Stick's execution lock and holds
        // it to the end of the write, and refuses a Stick the DJ has not
        // unlocked (wbx-45.11).
        let _gate = library.edit_gate().lock();
        let authorized = wreckordbox_stick_protection::authorize_stick(state_dir, &root)
            .map_err(|refused| protection_refused(&refused))?;
        let (index, generation) = library.index_at_generation()?;
        // What the load would write now, worked out again under the lock.
        let planned = plan_load(state_dir, library, stems, &index, &root, &args.playlists)?;
        // The confirmation: the dry run's plan id, for exactly this load, on
        // the Stick and the Library as they were planned, and for exactly
        // the result it reported. Used up here, whatever follows.
        let unconfirmed = |error: StickWriteError| match error {
            changed @ StickWriteError::Changed(_) => {
                ServiceError::new(codes::STICK_LOAD_CHANGED, changed.to_string())
            }
            other => ServiceError::new(codes::STICK_LOAD_UNCONFIRMED, other.to_string()),
        };
        let token = approval::approve(state_dir, &args.plan).map_err(unconfirmed)?;
        approval::consume_bound(
            Some(&token),
            &operation(&args.playlists),
            &root,
            generation,
            &planned.bound,
        )
        .map_err(unconfirmed)?;
        if !planned.changes.audio_estimated {
            return Err(ServiceError::new(
                codes::STICK_LOAD_INCOMPLETE,
                incomplete(&root),
            ));
        }
        if let Some(short) = planned.space.shortfall_bytes {
            return Err(ServiceError::new(
                codes::STICK_TOO_SMALL,
                format!(
                    "the load would not fit on {}: it needs {} bytes and {} are free, {short} bytes too few. Nothing was written",
                    root.display(),
                    planned.space.needed_bytes,
                    planned.space.free_before.unwrap_or(0),
                ),
            ));
        }
        Ok((authorized, planned))
    })?;
    if rbl_db::is_rekordbox_app_running() {
        return Err(ServiceError::new(
            service_codes::WRITE_REFUSED,
            "quit rekordbox before loading a Stick, so only one program writes its libraries. Nothing was written",
        ));
    }

    // The export, the window's own path, unchanged.
    let path = root.display().to_string();
    let progress = |stage: &str, done: usize, total: usize, track: serde_json::Value| {
        stems.publish(
            events::STICK_LOAD_PROGRESS,
            json!({ "stick": path, "stage": stage, "done": done, "total": total, "track": track }),
        );
    };
    // The export names a track by title only: its id when one track loaded
    // has that title, unknown (null) when several do.
    let mut titled: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for track in &planned.selection.tracks {
        let ids = titled.entry(track.title.as_str()).or_default();
        if !ids.contains(&track.id) {
            ids.push(track.id);
        }
    }
    let by_title = |title: &str| -> serde_json::Value {
        if title.is_empty() {
            return serde_json::Value::Null;
        }
        match titled.get(title).map(Vec::as_slice) {
            Some([id]) if *id != 0 => json!({ "id": id.to_string(), "title": title }),
            _ => json!({ "id": null, "title": title }),
        }
    };
    let fail_after: Option<usize> =
        test_hook(state_dir, TEST_FAIL_AFTER_FILE).and_then(|n| n.parse().ok());
    let pause_at = test_hook(state_dir, TEST_PAUSE_AT_FILE);
    let progressed = std::cell::Cell::new(0_usize);
    let lost = std::cell::RefCell::new(None::<Refused>);
    // Once the audio is copied, before anything is published: each source
    // file is still the one planned.
    let copied = std::cell::Cell::new(false);
    let diverged = std::cell::RefCell::new(None::<Vec<String>>);
    let stopped = std::cell::Cell::new(false);
    let cancelled = || {
        if fail_after.is_some_and(|after| progressed.get() >= after) {
            return true;
        }
        if lost.borrow().is_some() || diverged.borrow().is_some() || stopped.get() {
            return true;
        }
        if loads.stopping() {
            stopped.set(true);
            return true;
        }
        match authorized.recheck() {
            Ok(()) => false,
            Err(refused) => {
                *lost.borrow_mut() = Some(refused);
                true
            }
        }
    };
    let preferred = rbl_devices::list()
        .into_iter()
        .find(|device| device.mount_point == root)
        .and_then(|device| rbl_export::ExportRoot::for_file_system(&device.file_system));
    let selection = &planned.selection;
    let exported = rbl_export::export_cancellable(
        &root,
        &selection.tracks,
        &selection.playlists,
        &selection.my_tags,
        &rbl_export::ExportOptions {
            sync: Some(&selection.sync),
            root: preferred,
            keep_unlisted: selection.keep_unlisted,
            cache: Some(&EXPORT_CACHE),
            ..Default::default()
        },
        &mut |p| {
            progressed.set(progressed.get() + 1);
            if !copied.get() && !matches!(p.stage, "checking" | "copying" | "preparing") {
                copied.set(true);
                let mut now = Bound::new();
                now.insert("tracks".to_owned(), track_identities(selection));
                let mut was = Bound::new();
                was.insert(
                    "tracks".to_owned(),
                    planned.bound.get("tracks").cloned().unwrap_or_default(),
                );
                let changed = approval::differences(&was, &now);
                if !changed.is_empty() {
                    *diverged.borrow_mut() = Some(changed);
                }
            }
            pause_for_test(state_dir, pause_at.as_deref(), p.stage);
            progress(p.stage, p.done, p.total, by_title(&p.title));
        },
        &cancelled,
    );
    let error = match exported {
        Ok(report) if report.verified => None,
        Ok(_) => Some(LoadError {
            code: codes::STICK_LOAD_FAILED.to_owned(),
            message: "the library written to the Stick did not read back correctly".to_owned(),
        }),
        Err(error) => Some(if let Some(refused) = lost.borrow().as_ref() {
            LoadError {
                code: protection_refused(refused).code,
                message: format!("the load stopped: {refused}"),
            }
        } else if let Some(changed) = diverged.borrow().as_ref() {
            LoadError {
                code: codes::STICK_LOAD_CHANGED.to_owned(),
                message: format!(
                    "the load stopped before publishing anything: a track's audio changed while it was copied ({}). Make a new dry run",
                    changed.join("; ")
                ),
            }
        } else if stopped.get() {
            LoadError {
                code: service_codes::SERVICE_FAILED.to_owned(),
                message: "the load stopped because the Library service was stopped (the window took the Library over, or it was asked to stop)".to_owned(),
            }
        } else {
            LoadError {
                code: codes::STICK_LOAD_FAILED.to_owned(),
                message: match &error {
                    rbl_export::ExportError::Io(io)
                        if io.kind() == std::io::ErrorKind::StorageFull =>
                    {
                        format!("not enough space available on the Stick ({io})")
                    }
                    other => format!("the load failed: {other}"),
                },
            }
        }),
    };

    // Native stems, after the export, on a Spare stick: each the set the dry
    // run reported, with every check `stems.write` makes, Stick Protection
    // first.
    let mut native = Vec::new();
    let total = planned.stems.len();
    for (done, set) in planned.stems.into_iter().enumerate() {
        let track = TrackLine {
            id: set.found.track().id.clone(),
            title: set.found.track().title.clone(),
        };
        let (outcome, reason) = if error.is_some() {
            ("not_written", Some("the export failed first".to_owned()))
        } else if loads.stopping() {
            (
                "not_written",
                Some("the Library service was stopped".to_owned()),
            )
        } else if native_stems::current_stems_read_only(stems, &set.found).map(|now| now.identity)
            != Some(set.current.identity.clone())
        {
            (
                "not_written",
                Some("its Stems changed since the dry run; make a new dry run".to_owned()),
            )
        } else {
            progress("native_stems", done, total, json!(track));
            match native_stems::write_one(stems, state_dir, set.found, &root) {
                Ok(written) if written.outcome == "written" => ("written", None),
                Ok(skipped) => ("skipped", skipped.skipped.map(|s| s.reason)),
                Err(error) => ("failed", Some(error.message)),
            }
        };
        native.push(NativeStemsLine {
            track,
            outcome: outcome.to_owned(),
            reason,
        });
    }

    // Read back: the Stick compared with the plan again.
    let after = with_library(with, |library| {
        let _gate = library.edit_gate().lock();
        let (index, _) = library.index_at_generation()?;
        export_plan(library, &index, &root, &planned.selection, None).map_err(planning)
    });
    let report = read_back(&planned.selection, &planned.changes, after.ok().as_ref());
    let outcome = if error.is_some() {
        "failed"
    } else if report.1.tracks.is_empty()
        && report.1.playlists.is_empty()
        && native.iter().all(|n| n.outcome == "written")
    {
        "loaded"
    } else {
        "incomplete"
    };
    progress(
        if error.is_some() { "failed" } else { "done" },
        0,
        0,
        serde_json::Value::Null,
    );
    drop(authorized);
    let (written, not_written, unchanged) = report;
    Ok(Load {
        stick: sticks::describe(state_dir, &root),
        outcome: outcome.to_owned(),
        error,
        planned: planned.changes,
        written,
        not_written,
        unchanged,
        native_stems: native,
    })
}

/// The planned changes the Stick shows and does not show, read back by
/// planning the same selection against it again (`after`): a change still
/// planned is not on the Stick (an added track still to add, a field still
/// different, a playlist's entries or order still different, audio still
/// to copy, a removal still to make). What needed no change is
/// `unchanged`. When the Stick cannot be read back, every change counts
/// as not written.
fn read_back(
    selection: &ExportSelection,
    planned: &Changes,
    after: Option<&ExportPlan>,
) -> (Held, Held, Held) {
    let after = after.map(|plan| changes(plan, &[]));
    let (mut written, mut not_written, mut unchanged) =
        (Held::default(), Held::default(), Held::default());
    let mut push = |done: bool, line: &TrackLine| {
        let into = if done { &mut written } else { &mut not_written };
        if !into.tracks.contains(line) {
            into.tracks.push(line.clone());
        }
    };
    let still = |list: fn(&Changes) -> &Vec<TrackLine>, line: &TrackLine| {
        after.as_ref().is_none_or(|a| list(a).contains(line))
    };
    for t in &planned.tracks.added {
        push(!still(|c| &c.tracks.added, t), t);
    }
    for t in &planned.tracks.audio_changed {
        push(!still(|c| &c.tracks.audio_changed, t), t);
    }
    for t in &planned.tracks.removed {
        push(!still(|c| &c.tracks.removed, t), t);
    }
    for c in &planned.tracks.changed {
        let pending = after.as_ref().is_none_or(|a| {
            a.tracks
                .changed
                .iter()
                .any(|n| n.id == c.id && n.fields.iter().any(|f| c.fields.contains(f)))
        });
        push(
            !pending,
            &TrackLine {
                id: c.id.clone(),
                title: c.title.clone(),
            },
        );
    }
    let mut playlist = |done: bool, name: &str| {
        let into = if done { &mut written } else { &mut not_written };
        into.playlists.push(name.to_owned());
    };
    for name in &planned.playlists.created {
        playlist(
            after
                .as_ref()
                .is_some_and(|a| !a.playlists.created.contains(name)),
            name,
        );
    }
    let same = |p: &RewrittenPlaylist| {
        p.added == Some(0) && p.removed == Some(0) && p.reordered == Some(false)
    };
    for p in &planned.playlists.rewritten {
        if same(p) {
            unchanged.playlists.push(p.name.clone());
            continue;
        }
        let done = after.as_ref().is_some_and(|a| {
            a.playlists
                .rewritten
                .iter()
                .any(|n| n.name == p.name && same(n))
        });
        playlist(done, &p.name);
    }
    for name in &planned.playlists.removed {
        playlist(
            after
                .as_ref()
                .is_some_and(|a| !a.playlists.removed.contains(name)),
            name,
        );
    }
    // Tracks that needed no change.
    let touched: HashSet<&str> = planned
        .tracks
        .added
        .iter()
        .chain(&planned.tracks.audio_changed)
        .chain(&planned.tracks.removed)
        .chain(&planned.tracks.missing)
        .map(|t| t.id.as_str())
        .chain(planned.tracks.changed.iter().map(|c| c.id.as_str()))
        .collect();
    let mut seen = HashSet::new();
    for track in &selection.tracks {
        let line = line(track.id, &track.title);
        if seen.insert(line.clone()) && !touched.contains(line.id.as_str()) {
            unchanged.tracks.push(line);
        }
    }
    (written, not_written, unchanged)
}
