//! The dry run and the load.
//!
//! The lock order is the window's (`stick_guard`): the service's hold on
//! the Library, then the Library core's edit gate while the selection is
//! read, then the Stick's execution lock (shared across processes with the
//! window, the CLI, a lock and an eject). The gate is let go before the
//! long write; the execution lock is held to the end of it.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use wreckordbox_stick_protection::{Refused, Volume};
use wreckordbox_stick_write::approval::{self, StickOperation};
use wreckordbox_stick_write::plan::{export_plan, ExportPlan, PlaylistChange, EXPORT_CACHE};
use wreckordbox_stick_write::selection::{ExportSelection, LibrarySource};
use wreckordbox_stick_write::StickWriteError;

use super::sticks;
use super::{
    codes, events, ChangedTrack, Changes, Chosen, Counts, DryRun, DryRunArgs, Held, Load, LoadArgs,
    LoadError, NativeStemsLine, PlaylistChanges, RewrittenPlaylist, Space, TrackChanges, TrackLine,
};
use crate::library::OpenLibrary;
use crate::native_stems::{self, Found};
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

/// Bytes one second of a track's four Native stems takes, about: Ogg
/// Vorbis at quality 5 is some 160 kbit/s for stereo, four files.
const NATIVE_STEMS_BYTES_PER_SECOND: u64 = 4 * 20_000;

/// In test mode only, a file of this name in the state folder holding a
/// number stops the next load after that many progress steps: a load that
/// fails once it has started writing. Read, and removed, by each load.
pub const TEST_FAIL_AFTER_FILE: &str = "test-stick-load-fail-after";

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

/// Everything a dry run reports and a load does, worked out from one
/// snapshot of the Library and the Stick as they are.
struct Planned {
    playlists: Vec<Chosen>,
    selection: ExportSelection,
    changes: Changes,
    space: Space,
    /// The tracks whose Native stems are written, found in the Library.
    stems: Vec<Found>,
}

/// Works out a load of `playlists` to the Stick at `root`. Call holding
/// the Library's edit gate and the Stick's execution lock; writes nothing.
///
/// wbx-83 (Mutants rendered at load time) belongs here: render the chosen
/// playlists' Mutants first, then substitute each rendered file for its
/// entry's track in `selection` before it is planned, so the plan, the
/// confirmation and the export all see the rendered audio.
fn plan_load(
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
        .for_stick_inspected(library, index, &share, root, false, false)
        .map_err(planning)?
        .into_owned();
    let plan = export_plan(library, index, root, &selection, None).map_err(planning)?;

    // Native stems, on a Spare stick only: each track loaded that has a
    // complete set of current Stems.
    let mut stems_found = Vec::new();
    let mut stems_bytes = 0_u64;
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
            if native_stems::has_current_stems(stems, &found) {
                stems_bytes += u64::from(track.duration_sec.max(1)) * NATIVE_STEMS_BYTES_PER_SECOND;
                stems_found.push(found);
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
    let changes = changes(&plan, &stems_found);
    let space = space(root, &plan, stems_bytes);
    Ok(Planned {
        playlists: chosen,
        selection,
        changes,
        space,
        stems: stems_found,
    })
}

fn changes(plan: &ExportPlan, stems: &[Found]) -> Changes {
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
        .map(|f| TrackLine {
            id: f.track().id.clone(),
            title: f.track().title.clone(),
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

fn space(root: &Path, plan: &ExportPlan, stems_bytes: u64) -> Space {
    let free_before = sticks::free_bytes(root);
    let needed_bytes = plan.peak_bytes.saturating_add(stems_bytes);
    let net = plan
        .net_additional_bytes
        .saturating_add(i64::try_from(stems_bytes).unwrap_or(i64::MAX));
    Space {
        free_before,
        needed_bytes,
        free_after: free_before.map(|free| {
            if net >= 0 {
                free.saturating_sub(net.unsigned_abs())
            } else {
                free.saturating_add(net.unsigned_abs())
            }
        }),
        shortfall_bytes: free_before
            .filter(|free| needed_bytes > *free)
            .map(|free| needed_bytes - free),
        approximate: plan.size_uncertain || !plan.audio_estimated || stems_bytes > 0,
    }
}

/// Answers `sticks.list`.
pub(crate) fn list(state_dir: &Path) -> super::Sticks {
    super::Sticks {
        sticks: sticks::list(state_dir),
    }
}

/// Answers `sticks.dry_run`: what the load would do, and the plan id its
/// load must carry. Writes nothing to the Stick or the Library.
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
        let planned = plan_load(library, stems, &index, &root, &args.playlists)?;
        let plan = approval::propose(&root, operation(&args.playlists), Some(generation))
            .map_err(planning)?;
        Ok(DryRun {
            stick: sticks::describe(state_dir, &root),
            playlists: planned.playlists,
            changes: planned.changes,
            space: planned.space,
            plan,
        })
    })
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
) -> Result<Load, ServiceError> {
    let args: LoadArgs = arguments(request)?;
    chosen(&args.playlists)?;
    let root = stick_root(&args.stick)?;
    let (authorized, planned) = with_library(with, |library| {
        // The lock order: the edit gate while the selection is read; then
        // Stick Protection, which takes the Stick's execution lock and holds
        // it to the end of the write, and refuses a Stick the DJ has not
        // unlocked (wbx-45.11).
        let _gate = library.edit_gate().lock();
        let authorized = wreckordbox_stick_protection::authorize_stick(state_dir, &root)
            .map_err(|refused| protection_refused(&refused))?;
        let (index, generation) = library.index_at_generation()?;
        // The confirmation: the dry run's plan id, for exactly this load,
        // on the Stick and the Library as they were planned. Used up here,
        // whatever follows.
        let unconfirmed = |error: StickWriteError| {
            ServiceError::new(codes::STICK_LOAD_UNCONFIRMED, error.to_string())
        };
        let token = approval::approve(state_dir, &args.plan).map_err(unconfirmed)?;
        approval::consume(Some(&token), &operation(&args.playlists), &root, generation)
            .map_err(unconfirmed)?;
        let planned = plan_load(library, stems, &index, &root, &args.playlists)?;
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
    let progress = |stage: &str, done: usize, total: usize, track: Option<TrackLine>| {
        stems.publish(
            events::STICK_LOAD_PROGRESS,
            json!({ "stick": path, "stage": stage, "done": done, "total": total, "track": track }),
        );
    };
    let by_title = |title: &str| -> Option<TrackLine> {
        planned
            .selection
            .tracks
            .iter()
            .find(|t| t.title == title)
            .map(|t| line(t.id, &t.title))
    };
    let fail_after: Option<usize> = std::env::var_os(rbl_devices::TEST_MODE)
        .is_some_and(|v| v == "1")
        .then(|| {
            let file = state_dir.join(TEST_FAIL_AFTER_FILE);
            let after = std::fs::read_to_string(&file).ok()?.trim().parse().ok();
            let _ = std::fs::remove_file(&file);
            after
        })
        .flatten();
    let progressed = std::cell::Cell::new(0_usize);
    let lost = std::cell::RefCell::new(None::<Refused>);
    let cancelled = || {
        if fail_after.is_some_and(|after| progressed.get() >= after) {
            return true;
        }
        if lost.borrow().is_some() {
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
        Err(error) => Some(match lost.borrow().as_ref() {
            Some(refused) => LoadError {
                code: protection_refused(refused).code,
                message: format!("the load stopped: {refused}"),
            },
            None => LoadError {
                code: codes::STICK_LOAD_FAILED.to_owned(),
                message: match &error {
                    rbl_export::ExportError::Io(io)
                        if io.kind() == std::io::ErrorKind::StorageFull =>
                    {
                        format!("not enough space available on the Stick ({io})")
                    }
                    other => format!("the load failed: {other}"),
                },
            },
        }),
    };

    // Native stems, after the export, on a Spare stick: each with every
    // check `stems.write` makes, Stick Protection first.
    let mut native = Vec::new();
    if error.is_none() {
        let total = planned.stems.len();
        for (done, found) in planned.stems.into_iter().enumerate() {
            let track = TrackLine {
                id: found.track().id.clone(),
                title: found.track().title.clone(),
            };
            progress("native_stems", done, total, Some(track.clone()));
            let (outcome, reason) = match native_stems::write_one(stems, state_dir, found, &root) {
                Ok(written) if written.outcome == "written" => ("written", None),
                Ok(skipped) => ("skipped", skipped.skipped.map(|s| s.reason)),
                Err(error) => ("failed", Some(error.message)),
            };
            native.push(NativeStemsLine {
                track,
                outcome: outcome.to_owned(),
                reason,
            });
        }
    }

    // Read back: what the Stick holds of what the load was to put on it.
    let (written, not_written) = read_back(&root, &planned.selection, &planned.changes);
    let outcome = if error.is_some() {
        "failed"
    } else if not_written.tracks.is_empty()
        && not_written.playlists.is_empty()
        && native.iter().all(|n| n.outcome == "written")
    {
        "loaded"
    } else {
        "incomplete"
    };
    progress(if error.is_some() { "failed" } else { "done" }, 0, 0, None);
    drop(authorized);
    Ok(Load {
        stick: sticks::describe(state_dir, &root),
        outcome: outcome.to_owned(),
        error,
        planned: planned.changes,
        written,
        not_written,
        native_stems: native,
    })
}

/// Of the tracks and playlists `selection` was to put on the Stick, those
/// its record holds now and those it does not, read back from the Stick
/// (its manifest, the export's own record of what it wrote).
fn read_back(root: &Path, selection: &ExportSelection, planned: &Changes) -> (Held, Held) {
    let manifest = rbl_export::Manifest::load(root);
    let held_tracks: HashSet<String> = manifest
        .iter()
        .flat_map(|m| &m.tracks)
        .map(rbl_export::ManifestTrack::key)
        .collect();
    let held_playlists: HashSet<u64> = manifest
        .iter()
        .flat_map(|m| &m.playlists)
        .filter(|p| !p.device_only)
        .map(|p| p.library_id)
        .collect();
    let (mut written, mut not_written) = (Held::default(), Held::default());
    let mut seen = HashSet::new();
    for track in &selection.tracks {
        let key = rbl_export::track_key(track.id, &track.source_path.to_string_lossy());
        if !seen.insert(key.clone()) {
            continue;
        }
        let line = line(track.id, &track.title);
        if held_tracks.contains(&key) {
            written.tracks.push(line);
        } else if !planned.tracks.missing.contains(&line) {
            // A track whose audio is missing was never to be written.
            not_written.tracks.push(line);
        }
    }
    for playlist in selection.playlists.iter().filter(|p| !p.folder) {
        if held_playlists.contains(&playlist.id) {
            written.playlists.push(playlist.name.clone());
        } else {
            not_written.playlists.push(playlist.name.clone());
        }
    }
    (written, not_written)
}
