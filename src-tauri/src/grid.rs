//! Beat grid editing: the GRID panel's buttons, from the playhead to the
//! user's library.
//!
//! Ghidra evidence and the assumption inventory live in the private
//! pre-release beat-grid audit. PQTZ edits conditionally preserve compatible
//! PQT2 payloads, update BPM and the analysis revision, and obey the library
//! writer gate. Analysis lock is bit 0x80 in djmdContent.Analysed; legacy
//! local locks are retained until explicitly unlocked. Undo uses bounded
//! session snapshots and crash-recoverable file/database writes.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rbl_anlz::grid::{ apply_with_duration, tempo_x100, Edit};
use rbl_anlz::Beat;
use rbl_index::Library;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::{blocking, reload, write_error};
use crate::error::{AppError, AppResult, ErrorKind};
use crate::state::AppState;
use rbl_library::grid_step::{GridBeat, GridStep};
use rbl_library::history::LibraryEdit;

#[path = "grid_ledger.rs"]
mod ledger;
#[path = "grid_lock_selection.rs"]
mod lock_selection;
#[path = "grid_lock_recording.rs"]
mod lock_recording;

/// Grids kept for undo per track, per session. A grid is at most a few
/// hundred kilobytes, and a hundred steps is more than a hand edits.
const HISTORY_CAP: usize = 100;

/// One edit, as the panel sends it.
///
/// The shape mirrors `rbl_anlz::grid::Edit` field for field; the panel
/// spells the kind in camelCase, as the rest of the wire does.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GridEdit {
    /// Shift every beat by this many milliseconds; positive is later.
    Nudge { ms: i32 },
    Double,
    Halve,
    /// Make the beat nearest this time the downbeat.
    Downbeat { time_ms: u32 },
    /// Re-space the grid at this tempo with a beat held at the anchor.
    Tempo { bpm_x100: u16, anchor_ms: u32 },
    /// Move the target beat by milliseconds, holding the first editable beat.
    Stretch { by_ms: i32, time_ms: u32 },
    Tap { bpm: f64, anchor_ms: u32 },
    /// Move the grid so the beat nearest this time lands on it.
    Align { time_ms: u32 },
}

impl From<GridEdit> for Edit {
    fn from(edit: GridEdit) -> Self {
        match edit {
            GridEdit::Nudge { ms } => Self::Nudge(ms),
            GridEdit::Double => Self::Double,
            GridEdit::Halve => Self::Halve,
            GridEdit::Downbeat { time_ms } => Self::Downbeat { time_ms },
            GridEdit::Tempo { bpm_x100, anchor_ms } => Self::Tempo { bpm_x100, anchor_ms },
            GridEdit::Stretch { by_ms, time_ms } => Self::Stretch { by_ms, time_ms },
            GridEdit::Tap { bpm, anchor_ms } => Self::Tap { bpm, anchor_ms },
            GridEdit::Align { time_ms } => Self::Align { time_ms },
        }
    }
}

/// What the panel needs to know about a track's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GridStateDto {
    /// The grid's tempo x100, from its first beat; 0 without a grid.
    pub bpm_x100: u32,
    pub beats: u32,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<&'static str>,
    pub redo_label: Option<&'static str>,
    pub locked: bool,
}

/// What one action asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridAction {
    /// An edit, to the whole grid or from the beat nearest `from_ms` on.
    Edit { edit: GridEdit, from_ms: Option<u32> },
    Undo,
    Redo,
}

/// Transient editor options; never serialized into an analysis file.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridOptions {
    #[serde(default)]
    allow_dynamic: bool,
    transaction: Option<String>,
    duration_ms: Option<u32>,
    /// The track's analysis when it was registered after the index was
    /// read; the command fills it in, the frontend never sends it.
    #[serde(skip)]
    registered: Option<String>,
}

impl GridOptions {
    /// Options naming the run (a tap run, a held nudge) an edit belongs to:
    /// the edits of one run are one step of the shared history.
    pub fn with_transaction(transaction: Option<String>) -> Self {
        Self { transaction, ..Self::default() }
    }
}

/// rekordbox stores its analysis lock in bit 7 of Analysed.
pub(crate) fn database_locked(location: &rbl_db::LibraryLocation, track: &str) -> AppResult<bool> {
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).map_err(write_error)?;
    locked_in(&db, track).map_err(|e| AppError::internal(e.to_string()))
}

/// [`database_locked`] over a handle the caller already has, for a
/// loop over many tracks: opening the library derives its key again each
/// time, which is the cost of the whole check (#284).
pub(crate) fn locked_in(db: &rbl_db::Library, track: &str) -> Result<bool, rbl_db::DbError> {
    Ok(db.connection().query_row("SELECT (COALESCE(Analysed, 0) & 128) != 0 FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0", [track], |r| r.get(0))?)
}

/// What an action did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridOutcome {
    pub state: GridStateDto,
    /// Whether `djmdContent.BPM` changed, so the library has to be re-read.
    pub bpm_changed: bool,
    /// Whether the files were rewritten: false when the edit changed nothing.
    pub written: bool,
    /// The grid as it now stands, for the deck's metronome: milliseconds and
    /// the beat's number in its bar.
    pub beats: Vec<(u32, u8)>,
}

#[derive(Debug, Default)]
struct History {
    transaction: Option<String>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

/// One recorded edit: the grid either side of it, so Undo and Redo can
/// check that the grid on disk is still the one the edit left (or found)
/// before writing the other over it.
#[derive(Debug)]
struct HistoryEntry {
    /// The grid before the edit: what Undo writes back, and what Redo
    /// expects to find.
    before: Vec<Beat>,
    /// The grid the edit left: what Redo writes, and what Undo expects to
    /// find.
    after: Vec<Beat>,
    label: &'static str,
}

impl GridEdit {
    fn label(self) -> &'static str {
        match self {
            Self::Nudge { ms } if ms < 0 => "Shift Beat Grid Left",
            Self::Nudge { .. } => "Shift Beat Grid Right",
            Self::Double => "Double Tempo",
            Self::Halve => "Halve Tempo",
            Self::Downbeat { .. } => "Set Downbeat",
            Self::Tempo { .. } => "Set Tempo",
            Self::Tap { .. } => "Tap Tempo",
            Self::Stretch { .. } => "Adjust Tempo",
            Self::Align { .. } => "Align Beat Grid",
        }
    }
}

/// The session's grid editing state: histories, the lock list, and which
/// files have been backed up. Managed by the app beside `AppState`.
#[derive(Debug)]
pub struct GridEditor {
    editing: parking_lot::Mutex<()>,
    journal_root: PathBuf,
    histories: parking_lot::Mutex<HashMap<String, History>>,
    /// Loaded from `locks_path` on first use; `None` until then.
    locks: parking_lot::Mutex<Option<BTreeSet<String>>>,
    locks_path: PathBuf,
    /// Where analysis files are copied before their first rewrite.
    backup_dir: PathBuf,
    /// Files checked for a backup this session, so the check is one per file.
    backed_up: parking_lot::Mutex<HashSet<PathBuf>>,
}

impl Default for GridEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl GridEditor {
    pub fn clear_history(&self) { self.histories.lock().clear(); }

    /// The editor for the installed library: locks and backups under the
    /// app's own data directory, beside the database backups.
    pub fn new() -> Self {
        let root = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("wreckordbox");
        Self::at(&root)
    }

    /// An editor keeping its lock list and backups under `root`.
    pub fn at(root: &Path) -> Self {
        Self {
            editing: parking_lot::Mutex::new(()),
            journal_root: root.join("backups"),
            histories: parking_lot::Mutex::new(HashMap::new()),
            locks: parking_lot::Mutex::new(None),
            locks_path: root.join("grid-locks.json"),
            backup_dir: root.join("backups/analysis"),
            backed_up: parking_lot::Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn forget_history(&self, track: &str) { self.histories.lock().remove(track); }

    /// Whether the track's grid is locked against editing.
    pub fn is_locked(&self, track: &str) -> bool {
        self.with_locks(|locks| locks.contains(track))
    }

    /// Locks or unlocks a track's grid, and records it for the next session.
    pub fn set_locked(&self, track: &str, on: bool) -> AppResult<()> {
        let mut held = self.locks.lock();
        let locks = held.get_or_insert_with(|| read_locks(&self.locks_path));
        let changed = if on { locks.insert(track.to_owned()) } else { locks.remove(track) };
        if !changed {
            return Ok(());
        }
        let text = serde_json::to_string(&locks.iter().collect::<Vec<_>>())
            .map_err(|e| AppError::internal(format!("the lock list could not be encoded: {e}")))?;
        if let Some(parent) = self.locks_path.parent() {
            // perf-ok: called from the commands' blocking closures only
            std::fs::create_dir_all(parent).map_err(|e| lock_error(&self.locks_path, &e))?;
        }
        write_atomically(&self.locks_path, text.as_bytes()).map_err(|e| lock_error(&self.locks_path, &e))
    }

    fn with_locks<T>(&self, f: impl FnOnce(&BTreeSet<String>) -> T) -> T {
        let mut held = self.locks.lock();
        f(held.get_or_insert_with(|| read_locks(&self.locks_path)))
    }

    fn state_for(&self, track: &str, beats: &[Beat]) -> GridStateDto {
        let histories = self.histories.lock();
        let history = histories.get(track);
        GridStateDto {
            bpm_x100: u32::from(tempo_x100(beats)),
            beats: u32::try_from(beats.len()).unwrap_or(u32::MAX),
            can_undo: history.is_some_and(|h| !h.undo.is_empty()),
            can_redo: history.is_some_and(|h| !h.redo.is_empty()),
            undo_label: history.and_then(|h| h.undo.last().map(|entry| entry.label)),
            redo_label: history.and_then(|h| h.redo.last().map(|entry| entry.label)),
            locked: self.is_locked(track),
        }
    }

    /// Copies a file under the backup directory at `relative`, once: a
    /// copy already there is the file as rekordbox wrote it, and stays.
    fn back_up(&self, file: &Path, relative: &str, extension: &str) -> AppResult<()> {
        if self.backed_up.lock().contains(file) || !file.is_file() {
            return Ok(());
        }
        let copy = self
            .backup_dir
            .join(relative.trim_start_matches(['/', '\\']))
            .with_extension(extension);
        if !copy.exists() {
            if let Some(parent) = copy.parent() {
                // perf-ok: called from the commands' blocking closures only
                std::fs::create_dir_all(parent).map_err(|e| file_error("backed up", &copy, &e))?;
            }
            std::fs::copy(file, &copy).map_err(|e| file_error("backed up", &copy, &e))?;
        }
        self.backed_up.lock().insert(file.to_path_buf());
        Ok(())
    }
}

fn read_locks(path: &Path) -> BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return BTreeSet::new() };
    match serde_json::from_str::<Vec<String>>(&text) {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            // A list that cannot be read locks nothing, and says so once
            // rather than every edit; the next lock rewrites it whole.
            tracing::warn!(path = %path.display(), error = %e, "the grid lock list could not be read");
            BTreeSet::new()
        }
    }
}

fn lock_error(path: &Path, error: &std::io::Error) -> AppError {
    AppError::new(ErrorKind::Internal, "The grid lock could not be saved.")
        .with_detail(format!("{}: {error}", path.display()))
}

fn file_error(what: &str, path: &Path, error: &std::io::Error) -> AppError {
    AppError::new(ErrorKind::Internal, format!("The analysis file could not be {what}."))
        .with_detail(format!("{}: {error}", path.display()))
}

/// Writes a file whole, through a sibling and a rename, so a crash midway
/// leaves the old file rather than half of the new one.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    crate::durable::write(path, bytes)
}

/// A track's analysis files, resolved.
struct Files {
    relative: String,
    dat: PathBuf,
    ext: PathBuf,
}

/// `registered` is the track's analysis when it was registered after the
/// index was read, which the index does not show yet.
fn files_of(library: &Library, share: &Path, track: &str, registered: Option<&str>) -> AppResult<Files> {
    let Some(row) = library.row_of(track) else {
        return Err(AppError::new(ErrorKind::NotFound, "That track is not in the library."));
    };
    let indexed = library.analysis_path.get(row as usize);
    let relative = if indexed.is_empty() { registered.unwrap_or_default() } else { indexed };
    if relative.is_empty() {
        return Err(AppError::new(ErrorKind::NotFound, "That track has not been analysed, so it has no grid to edit."));
    }
    let dat = rbl_anlz::resolve(share, relative);
    let ext = rbl_anlz::sibling(&dat, "EXT");
    Ok(Files { relative: relative.to_owned(), dat, ext })
}

fn read_dat(dat: &Path) -> AppResult<(rbl_anlz::Anlz, Vec<Beat>)> {
    let parsed = rbl_anlz::Anlz::read(dat).map_err(|e| {
        AppError::new(ErrorKind::NotFound, "That track's analysis file could not be read.")
            .with_detail(format!("{}: {e}", dat.display()))
    })?;
    let Some(beats) = parsed.beat_grid().filter(|beats| !beats.is_empty()) else {
        return Err(AppError::new(ErrorKind::NotFound, "That track has no beat grid to edit."));
    };
    let offset = i64::from(parsed.grid_offset().unwrap_or(0));
    let beats = beats.into_iter().filter_map(|beat| u32::try_from(i64::from(beat.time_ms) + offset).ok().map(|time_ms| Beat { time_ms, ..beat })).collect();
    Ok((parsed, beats))
}

/// The grid as the panel sees it, without changing anything.
pub fn state_of(editor: &GridEditor, library: &Library, share: &Path, track: &str) -> AppResult<GridStateDto> {
    state_with(editor, library, share, track, None)
}

/// [`state_of`] for a track whose analysis may have been registered after
/// the index was read.
pub(crate) fn state_with(editor: &GridEditor, library: &Library, share: &Path, track: &str,
    registered: Option<&str>) -> AppResult<GridStateDto> {
    let files = files_of(library, share, track, registered)?;
    let (_, beats) = read_dat(&files.dat)?;
    Ok(editor.state_for(track, &beats))
}

/// Applies one action to a track's grid: the files, the history and, through
/// `set_bpm`, the database row when the tempo changed.
///
/// Apart from the commands so it can be tested against a fixture in a
/// tempdir without a Tauri app. `set_bpm` is called with the new tempo x100
/// only when it differs from the old; it opens the writer, which is what
/// takes the database backup and re-checks the gate. If it fails, the files
/// are put back as they were, so the row and the grid never disagree.
pub fn apply(
    editor: &GridEditor,
    library: &Library,
    location: &rbl_db::LibraryLocation,
    track: &str,
    action: GridAction,
    set_bpm: &mut dyn FnMut(u32) -> AppResult<()>,
) -> AppResult<GridOutcome> {
    apply_options(editor, library, location, track, action, &GridOptions::default(), set_bpm, None)
}

/// Records what just happened in the track's undo/redo history — the part
/// of [`apply_options`] after the write has already landed. `before` and
/// `after` are the grid the action found and the grid it wrote.
fn record_history(editor: &GridEditor, track: &str, action: GridAction, before: Vec<Beat>, after: Vec<Beat>,
    options: &GridOptions) {
    let mut histories = editor.histories.lock();
    let history = histories.entry(track.to_owned()).or_default();
    match action {
        GridAction::Edit { edit, .. } => {
            history.redo.clear();
            let transaction = options.transaction.as_ref().filter(|_| matches!(edit, GridEdit::Tap { .. }));
            match history.undo.last_mut() {
                // A tap in the same run joins the entry the run started:
                // Undo still goes back to the grid before the first tap.
                Some(open) if transaction.is_some() && history.transaction.as_ref() == transaction => open.after = after,
                _ => history.undo.push(HistoryEntry { before, after, label: edit.label() }),
            }
            history.transaction = transaction.cloned();
            if history.undo.len() > HISTORY_CAP { history.undo.remove(0); }
        }
        GridAction::Undo => {
            history.transaction = None;
            if let Some(entry) = history.undo.pop() { history.redo.push(entry); }
        }
        GridAction::Redo => {
            history.transaction = None;
            if let Some(entry) = history.redo.pop() { history.undo.push(entry); }
        }
    }
}

fn shared_beats(beats: &[Beat]) -> std::sync::Arc<[GridBeat]> {
    beats.iter().map(|b| GridBeat { time_ms: b.time_ms, beat_number: b.beat_number, tempo_x100: b.tempo_x100 }).collect()
}

fn file_beats(beats: &[GridBeat]) -> Vec<Beat> {
    beats.iter().map(|b| Beat { time_ms: b.time_ms, beat_number: b.beat_number, tempo_x100: b.tempo_x100 }).collect()
}

/// The shared history's next entry for Undo (`action`) or Redo, when it is
/// a grid step of `track`, with its label.
fn shared_next(state: &AppState, track: &str, undo: bool) -> Option<(GridStep, &'static str)> {
    let history = state.edit_history().lock();
    match history.next(undo) {
        Some(entry) => match &entry.edit {
            LibraryEdit::Grid(step) if step.track == track => Some((step.clone(), entry.label)),
            _ => None,
        },
        None => None,
    }
}

/// The grid an Undo or Redo of the shared history's next entry writes: the
/// entry must be a grid step of `track` (the grid panel's Undo never reaches
/// past another edit), and the grid on disk the one it expects. One that
/// finds a grid changed since leaves the history alone of all entries, as
/// the core's refused steps do (#506).
fn shared_target(state: &AppState, track: &str, action: GridAction, beats: &[Beat]) -> AppResult<Vec<Beat>> {
    let undo = action == GridAction::Undo;
    let Some((step, label)) = shared_next(state, track, undo) else {
        let what = if undo { "undo" } else { "redo" };
        return Err(AppError::new(ErrorKind::NotFound, format!("Nothing to {what} on this track's beat grid.")));
    };
    let (expected, target) = if undo { (&step.after, &step.before) } else { (&step.before, &step.after) };
    if file_beats(expected) != beats {
        state.edit_history().lock().drop_next(undo);
        rbl_library::Host::history_changed(state);
        return Err(conflict(action, label));
    }
    Ok(file_beats(target))
}

/// Records the window's grid action in the shared history: an edit as an
/// entry (an edit of the same tap run or held nudge joins the one before
/// it), an Undo or Redo as the step of the entry it took back or made again.
fn record_shared(state: &AppState, track: &str, action: GridAction, before: &[Beat], after: &[Beat], options: &GridOptions) {
    {
        let mut history = state.edit_history().lock();
        match action {
            GridAction::Edit { edit, .. } => {
                let transaction = options.transaction.clone()
                    .filter(|_| matches!(edit, GridEdit::Tap { .. } | GridEdit::Nudge { .. }));
                history.record_grid(GridStep {
                    track: track.to_owned(),
                    before: shared_beats(before),
                    after: shared_beats(after),
                    transaction,
                }, edit.label());
            }
            GridAction::Undo => { history.stepped_elsewhere(true); }
            GridAction::Redo => { history.stepped_elsewhere(false); }
        }
    }
    rbl_library::Host::history_changed(state);
}

/// What the grid panel offers to undo and redo of `track`: the shared
/// history's next entries, when they are grid steps of this track.
pub(crate) fn with_shared_history(state: &AppState, track: &str, mut dto: GridStateDto) -> GridStateDto {
    let undo = shared_next(state, track, true);
    let redo = shared_next(state, track, false);
    dto.can_undo = undo.is_some();
    dto.can_redo = redo.is_some();
    dto.undo_label = undo.map(|(_, label)| label);
    dto.redo_label = redo.map(|(_, label)| label);
    dto
}

/// Undo or Redo found a grid other than the one its entry expects:
/// something outside this history (rekordbox, say, while RBXport stayed
/// open) changed the grid since. The newer grid is kept, as #506 keeps a
/// newer library value, in the same words.
fn conflict(action: GridAction, label: &str) -> AppError {
    let what = if action == GridAction::Undo { "undo" } else { "redo" };
    AppError::new(ErrorKind::Conflict,
        format!("Can't {what} {label}: the track's beat grid has changed since. The newer change was kept."))
}

#[allow(clippy::too_many_arguments, reason = "grid inputs and the optional owning file operation")]
fn apply_options(editor: &GridEditor, library: &Library, location: &rbl_db::LibraryLocation,
    track: &str, action: GridAction, options: &GridOptions,
    set_bpm: &mut dyn FnMut(u32) -> AppResult<()>,
    recording: Option<(&mut rbl_library::file_edit::FileEdit<'_>, &AppState)>) -> AppResult<GridOutcome> {
    // The writer's own rule, applied to the files as well as the row: a
    // fixture in a tempdir is not the installed library, so its tests run
    // whether or not rekordbox is open.
    if let Some(reason) = rbl_db::write_refusal_reason(
        location.is_real_install,
        rbl_db::test_mode(),
        rbl_db::is_rekordbox_running(),
    ) {
        return Err(AppError::new(ErrorKind::ReadOnly, reason));
    }
    let _editing = editor.editing.lock();
    crate::file_journal::recover(&editor.journal_root, location)?;
    let files = files_of(library, &location.share_root, track, options.registered.as_deref())?;
    let (parsed, beats) = read_dat(&files.dat)?;

    let next: Vec<Beat> = match action {
        GridAction::Edit { edit, from_ms } => {
            if editor.is_locked(track) || database_locked(location, track)? {
                return Err(AppError::new(ErrorKind::ReadOnly, "The beat grid is locked. Unlock it to edit."));
            }
            rbl_anlz::grid::validate(&beats, from_ms, edit.into())
                .map_err(|message| AppError::new(ErrorKind::Malformed, message))?;
            // A tapped tempo respaces the grid as a typed one does, so it
            // replaces tempo changes too: asked first (wbx-64 review 4).
            if matches!(edit, GridEdit::Stretch { .. } | GridEdit::Tempo { .. } | GridEdit::Tap { .. })
                && !options.allow_dynamic && rbl_anlz::grid::is_dynamic_from(&beats, from_ms) {
                return Err(AppError::new(ErrorKind::Malformed, "Confirm replacing this section's tempo changes before adjusting its tempo."));
            }
            let row = library.row_of(track).map(|row| row as usize);
            let duration = options.duration_ms.or_else(|| row.and_then(|i| library.length_sec.get(i)).map(|s| s.saturating_mul(1000)))
                .unwrap_or_else(|| beats.last().map_or(0, |b| b.time_ms));
            let next = apply_with_duration(&beats, from_ms, edit.into(), duration);
            if next.is_empty() {
                return Err(AppError::new(ErrorKind::Malformed, "That edit would leave the track without a beat."));
            }
            if next == beats {
                let found = editor.state_for(track, &beats);
                return Ok(GridOutcome {
                    state: match recording.as_ref() { Some((_, state)) => with_shared_history(state, track, found), None => found },
                    bpm_changed: false,
                    written: false,
                    beats: wire_beats(&beats),
                });
            }
            next
        }
        GridAction::Undo | GridAction::Redo if recording.is_some() => {
            match recording.as_ref() {
                Some((_, state)) => shared_target(state, track, action, &beats)?,
                None => return Err(AppError::internal("no window to step the shared history in")),
            }
        }
        GridAction::Undo | GridAction::Redo => {
            let mut histories = editor.histories.lock();
            let history = histories.entry(track.to_owned()).or_default();
            let from = if action == GridAction::Undo { &history.undo } else { &history.redo };
            let Some(entry) = from.last() else {
                let what = if action == GridAction::Undo { "undo" } else { "redo" };
                return Err(AppError::new(ErrorKind::NotFound, format!("Nothing to {what}.")));
            };
            let (expected, target) = if action == GridAction::Undo { (&entry.after, &entry.before) } else { (&entry.before, &entry.after) };
            if *expected != beats {
                // Only that entry goes, as #506 drops a conflicting library
                // entry: left on top it would block every older one, which
                // may still fit (an outside write that put the grid back).
                let label = entry.label;
                let from = if action == GridAction::Undo { &mut history.undo } else { &mut history.redo };
                from.pop();
                return Err(conflict(action, label));
            }
            target.clone()
        }
    };

    let row_host = recording.as_ref().map(|(_, host)| *host);
    let record = match recording {
        Some((guard, _)) => Some(ledger::begin(guard, track, action, &files.relative, &beats, &next)?),
        None => None,
    };
    editor.back_up(&files.dat, &files.relative, "DAT")?;
    editor.back_up(&files.ext, &files.relative, "EXT")?;
    let mut changed_files = vec![(files.dat.clone(), parsed.with_beat_grid(&next))];
    if files.ext.exists() {
        let ext = rbl_anlz::Anlz::read(&files.ext).map_err(|e| AppError::internal(e.to_string()))?;
        if let Some(cleared) = ext.with_extended_grid_edit(&parsed.beat_grid().unwrap_or_default(), &next, parsed.grid_offset().unwrap_or(0)) { changed_files.push((files.ext.clone(), cleared)); }
    }
    let old_bpm = u32::from(tempo_x100(&beats));
    let new_bpm = u32::from(tempo_x100(&next));
    let bpm_changed = new_bpm != old_bpm;
    let journal = match crate::file_journal::FileJournal::prepare(&editor.journal_root, location, track,
        new_bpm, None, true, &changed_files) {
        Ok(journal) => journal,
        Err(error) => { if let Some(record) = record { record.cancelled(); } return Err(error); }
    };
    if let Err(e) = journal.publish() {
        journal.rollback()?;
        if let Some(record) = record { record.cancelled(); }
        return Err(e);
    }
    let saved = if let (Some(record), Some(host)) = (&record, row_host) {
        // The history's own write: a step of the shared history (wbx-64),
        // which must not end the Redo branch it is about to step onto.
        record.write_history_row(host, |writer| writer.save_grid_revision(track, new_bpm)).map_err(write_error)
    } else { set_bpm(new_bpm) };
    if let Err(refused) = saved {
        journal.reconcile(location)?;
        return Err(refused);
    }
    journal.commit()?;
    if let (Some(record), Some(state)) = (record, row_host) {
        // The window's edits are steps of the one shared history, in the
        // order the DJ made them (wbx-64); the Ledger's entry is that record.
        record.written_in_history();
        record_shared(state, track, action, &beats, &next, options);
    } else {
        record_history(editor, track, action, beats, next.clone(), options);
    }

    Ok(GridOutcome {
        state: {
            let found = GridStateDto { locked: editor.is_locked(track) || database_locked(location, track)?, ..editor.state_for(track, &next) };
            match row_host { Some(state) => with_shared_history(state, track, found), None => found }
        },
        bpm_changed,
        written: true,
        beats: wire_beats(&next),
    })
}

/// The Info panel's BPM field: the grid editor's Set Tempo over the whole
/// track, through the same durable file/row transaction and recorded in the
/// same per-track history, so the grid panel's Undo puts the grid and
/// `djmdContent.BPM` back together and earlier grid edits stay undoable
/// (#505; rekordbox's `djplay::BeatGridAdjustment::ApplyNewBpm` is undoable
/// too, 7.2.19 symbols).
pub(crate) fn set_tempo(state: &AppState, editor: &GridEditor, track: &str, value: &str) -> AppResult<()> {
    let bpm: f64 = value.trim().parse().map_err(|_| AppError::new(ErrorKind::Malformed, "Enter a BPM from 40 to 499."))?;
    if !bpm.is_finite() || !(40.0..=499.0).contains(&bpm) {
        return Err(AppError::new(ErrorKind::Malformed, "Enter a BPM from 40 to 499."));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "validated 40..=499 above")]
    let bpm_x100 = (bpm * 100.0).round() as u16;
    let mut guard = state.file_edit()?;
    let location = state.location()?;
    if database_locked(&location, track)? { return Err(AppError::new(ErrorKind::ReadOnly, "The beat grid is locked.")); }
    crate::file_journal::recover(state.backup_dir(), &location)?;
    let library = state.library()?;
    let row = library.row_of(track).ok_or_else(|| AppError::new(ErrorKind::NotFound, "That track is no longer in the library."))?;
    let relative = state.analysis_path_of(&library, row as usize, track);
    let mut set_bpm = |bpm_x100: u32| state.write(|w| w.save_grid_revision(track, bpm_x100)).map_err(write_error);
    if relative.is_empty() {
        // No analysis, so no grid and nothing for the grid history.
        return set_bpm(u32::from(bpm_x100));
    }
    let (_, beats) = read_dat(&rbl_anlz::resolve(&location.share_root, &relative))?;
    if rbl_anlz::grid::is_dynamic_from(&beats, None) { return Err(AppError::new(ErrorKind::Malformed, "Use the grid BPM field to confirm replacing tempo changes.")); }
    let options = GridOptions {
        registered: state.registered_analysis(track),
        duration_ms: library.length_sec.get(row as usize).map(|s| s.saturating_mul(1000)),
        ..GridOptions::default()
    };
    let edit = GridAction::Edit { edit: GridEdit::Tempo { bpm_x100, anchor_ms: 0 }, from_ms: None };
    let outcome = apply_options(editor, &library, &location, track, edit, &options, &mut set_bpm, Some((&mut guard, state)))?;
    if !outcome.written {
        // The grid already stands at that tempo: nothing to record, but the
        // row still takes the typed value, as it did before #505.
        set_bpm(u32::from(bpm_x100))?;
    }
    Ok(())
}

fn wire_beats(beats: &[Beat]) -> Vec<(u32, u8)> {
    beats.iter().map(|beat| (beat.time_ms, u8::try_from(beat.beat_number).unwrap_or(0))).collect()
}

/// One grid action as the window's commands run it: refused before any
/// analysis file is published while the Library may not be changed
/// (Library Protection among it), then the files and the row as one edit.
pub(crate) fn guarded(state: &AppState, editor: &GridEditor, library: &Library, location: &rbl_db::LibraryLocation,
    track: &str, action: GridAction, options: &GridOptions) -> AppResult<GridOutcome> {
    let mut guard = state.file_edit()?;
    let mut set_bpm = |bpm_x100: u32| state.write(|writer| writer.save_grid_revision(track, bpm_x100)).map_err(write_error);
    apply_options(editor, library, location, track, action, options, &mut set_bpm, Some((&mut guard, state)))
}

/// Runs one action under `blocking` (a `spawn_blocking` thread: the files
/// and the row are disk work), then tells the interface and the deck what
/// changed.
#[allow(clippy::too_many_arguments, reason = "three managed states, the app, and what the command was given")]
async fn run<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    player: State<'_, Arc<crate::player::Player>>,
    editor: State<'_, Arc<GridEditor>>,
    name: &'static str,
    track: String,
    action: GridAction,
    deck: Option<String>,
    mut options: GridOptions,
) -> AppResult<GridStateDto> {
    let library = state.library()?;
    let location = state.location()?;
    options.registered = state.registered_analysis(&track);
    let state = Arc::clone(&state);
    let editor = Arc::clone(&editor);
    let outcome = {
        let state = Arc::clone(&state);
        let track = track.clone();
        blocking(name, move || guarded(&state, &editor, &library, &location, &track, action, &options))
        .await
    };
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(refused) => {
            // A refused Undo or Redo may have dropped an entry that no
            // longer fits the grid; the panel refetches, so its buttons say so.
            if matches!(action, GridAction::Undo | GridAction::Redo) {
                let _ = tauri::Emitter::emit(&app, "grid:changed", &track);
            }
            return Err(refused);
        }
    };
    if !outcome.written {
        return Ok(outcome.state);
    }
    // The deck's metronome plays the grid it was given on load; give it this one.
    if let (Some(deck), Some(engine)) = (deck, player.opened()) {
        let grid: Vec<(u32, bool)> = outcome.beats.iter().map(|&(ms, number)| (ms, number == 1)).collect();
        engine.set_metronome_grid(crate::player::deck_of(&deck), &grid);
    }
    // The track's id, inside the event cap: every deck showing the track
    // refetches its grid. A changed tempo is a changed column, so the
    // library is re-read as it is after any other track edit.
    let _ = tauri::Emitter::emit(&app, "grid:changed", &track);
    if outcome.bpm_changed {
        reload(app, state).await?;
    }
    Ok(outcome.state)
}

/// The grid as the panel shows it: tempo, beat count, undo, redo, lock.
#[tauri::command]
pub async fn grid_state(
    state: State<'_, Arc<AppState>>,
    editor: State<'_, Arc<GridEditor>>,
    track: String,
) -> AppResult<GridStateDto> {
    let library = state.library()?;
    let location = state.location()?;
    let registered = state.registered_analysis(&track);
    let editor = Arc::clone(&editor);
    let state = Arc::clone(&state);
    blocking("grid_state", move || {
        let mut result = state_with(&editor, &library, &location.share_root, &track, registered.as_deref())?;
        result.locked |= database_locked(&location, &track)?;
        Ok(with_shared_history(&state, &track, result))
    }).await
}

/// One grid edit. `from_ms` names the beat from which it applies — the scope
/// point, or the playhead for the from-here buttons — and `deck` the deck
/// the track is loaded on, so its metronome follows.
#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "a command takes its states and its arguments flat")]
pub async fn grid_edit<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    player: State<'_, Arc<crate::player::Player>>,
    editor: State<'_, Arc<GridEditor>>,
    track: String,
    edit: GridEdit,
    from_ms: Option<u32>,
    deck: Option<String>,
    options: Option<GridOptions>,
) -> AppResult<GridStateDto> {
    run(app, state, player, editor, "grid_edit", track, GridAction::Edit { edit, from_ms }, deck, options.unwrap_or_default()).await
}

#[tauri::command]
pub async fn grid_undo<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    player: State<'_, Arc<crate::player::Player>>,
    editor: State<'_, Arc<GridEditor>>,
    track: String,
    deck: Option<String>,
) -> AppResult<GridStateDto> {
    run(app, state, player, editor, "grid_undo", track, GridAction::Undo, deck, GridOptions::default()).await
}

#[tauri::command]
pub async fn grid_redo<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    player: State<'_, Arc<crate::player::Player>>,
    editor: State<'_, Arc<GridEditor>>,
    track: String,
    deck: Option<String>,
) -> AppResult<GridStateDto> {
    run(app, state, player, editor, "grid_redo", track, GridAction::Redo, deck, GridOptions::default()).await
}

/// Sets the Library's native analysis-lock bit through the recorded hand-edit
/// path, then clears the legacy local editor lock separately.
#[tauri::command]
pub async fn grid_lock<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    editor: State<'_, Arc<GridEditor>>,
    track: String,
    on: bool,
) -> AppResult<GridStateDto> {
    let library = state.library()?;
    let location = state.location()?;
    let registered = state.registered_analysis(&track);
    let editor = Arc::clone(&editor);
    let state = Arc::clone(&state);
    let changed = track.clone();
    let notify = app.clone();
    let result = blocking("grid_lock", move || {
        if let Some(change) = lock_recording::set(&state, &track, on)? {
            let _ = tauri::Emitter::emit(&notify, "library:changed", change.generation);
        }
        // Native acceptance/recording is complete and its gate released before
        // touching editor files or reading analysis bytes for the response.
        editor.set_locked(&track, false)?;
        let mut result = state_with(&editor, &library, &location.share_root, &track, registered.as_deref())?;
        result.locked = database_locked(&location, &track)?;
        Ok(with_shared_history(&state, &track, result))
    }).await?;
    let _ = tauri::Emitter::emit(&app, "grid:changed", &changed);
    Ok(result)
}

/// Set the native analysis lock for a selection as one window action.
#[tauri::command]
pub async fn grid_lock_selection<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    editor: State<'_, Arc<GridEditor>>,
    tracks: Vec<String>,
    on: bool,
) -> AppResult<()> {
    let state = Arc::clone(&state);
    let editor = Arc::clone(&editor);
    blocking("grid_lock_selection", move || lock_selection::set(&app, &state, &editor, &tracks, on)).await
}

#[cfg(test)]
mod tests {
    // Every test builds its own library and share tree in a tempdir, the way
    // the cue tests do; `RBXPORT_TEST` cannot reach a fixture.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use rbl_anlz::AnlzBuilder;
    use rbl_db::fixture::{self, track_id, Shape};
    use rbl_db::{Library as Db, OpenMode};

    const RELATIVE: &str = "/PIONEER/USBANLZ/ab1/cd2/ANLZ0000.DAT";

    struct Fixture {
        dir: tempfile::TempDir,
        location: rbl_db::LibraryLocation,
        library: Library,
        editor: GridEditor,
        /// Every BPM the action asked the row to take, in order.
        bpms: Vec<u32>,
    }

    /// Eight beats at 120 BPM from 500 ms, numbered from the first.
    fn grid() -> Vec<Beat> {
        (0..8)
            .map(|i| Beat { beat_number: (i % 4) + 1, tempo_x100: 12_000, time_ms: 500 + u32::from(i) * 500 })
            .collect()
    }

    fn open() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let location = fixture::build(dir.path(), Shape::default()).expect("build the fixture");
        fixture::set_analysis_path(&location, 1, RELATIVE).unwrap();
        let dat = rbl_anlz::resolve(&location.share_root, RELATIVE);
        std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
        let mut file = AnlzBuilder::new();
        file.path("/x.mp3").beat_grid(&grid()).vbr_table_zero().cue_lists(false);
        std::fs::write(&dat, file.finish()).unwrap();
        // An `.EXT` with a filled extended grid, as rekordbox writes one.
        let mut ext = AnlzBuilder::new();
        ext.path("/x.mp3").cue_lists(true);
        let mut header = vec![0_u8; 44];
        header[4..8].copy_from_slice(&0x0100_0002_u32.to_be_bytes());
        ext.raw(rbl_core::FourCc::new(b"PQT2"), header, vec![0, 1, 0, 2, 0, 3]);
        std::fs::write(dat.with_extension("EXT"), ext.finish()).unwrap();

        let db = Db::open(location.clone(), OpenMode::ReadOnly).expect("read-only");
        let (library, _) = rbl_index::load(&db).expect("index");
        let editor = GridEditor::at(&dir.path().join("app"));
        Fixture { dir, location, library, editor, bpms: Vec::new() }
    }

    impl Fixture {
        fn track() -> String {
            track_id(1)
        }

        fn run(&mut self, action: GridAction) -> AppResult<GridOutcome> {
            let track = Self::track();
            let bpms = &mut self.bpms;
            let mut set_bpm = |bpm: u32| {
                bpms.push(bpm);
                Ok(())
            };
            apply(&self.editor, &self.library, &self.location, &track, action, &mut set_bpm)
        }

        fn edit(&mut self, edit: GridEdit) -> GridOutcome {
            self.run(GridAction::Edit { edit, from_ms: None }).unwrap()
        }

        fn dat(&self) -> rbl_anlz::Anlz {
            rbl_anlz::Anlz::read(&rbl_anlz::resolve(&self.location.share_root, RELATIVE)).unwrap()
        }

        fn ext(&self) -> rbl_anlz::Anlz {
            rbl_anlz::Anlz::read(&rbl_anlz::resolve(&self.location.share_root, RELATIVE).with_extension("EXT")).unwrap()
        }

        fn times(&self) -> Vec<u32> {
            self.dat().beat_grid().unwrap().iter().map(|b| b.time_ms).collect()
        }
    }

    fn app_state(f: &Fixture) -> AppState {
        let db = Db::open(f.location.clone(), OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        let state = AppState::unprotected(f.dir.path().join("app/backups"));
        state.set_library(library, false, db.schema().db_version, 0, f.location.clone());
        crate::backups::create(&state).unwrap();
        state
    }

    #[test]
    fn typed_bpm_keeps_the_row_and_both_analysis_files_consistent() {
        let f = open();
        let state = app_state(&f);
        set_tempo(&state, &f.editor, &Fixture::track(), "150").unwrap();
        let beats = f.dat().beat_grid().unwrap();
        assert!(beats.iter().any(|b| b.time_ms == 500), "the anchor remains, with one preceding beat filled");
        assert_eq!(beats[2].time_ms, 900);
        assert!(beats.iter().all(|b| b.tempo_x100 == 15_000));
        assert_eq!(f.ext().section(b"PQT2").unwrap().payload, [] as [u8; 0]);
        let bpm: u32 = state.read_db(|db| Ok(db.connection().query_row(
            "SELECT BPM FROM djmdContent WHERE ID=?1", [Fixture::track()], |r| r.get(0))?)).unwrap();
        assert_eq!(bpm, 15_000);
        set_tempo(&state, &f.editor, &track_id(2), "128.5").unwrap();
        let bpm: u32 = state.read_db(|db| Ok(db.connection().query_row(
            "SELECT BPM FROM djmdContent WHERE ID=?1", [track_id(2)], |r| r.get(0))?)).unwrap();
        assert_eq!(bpm, 12_850, "a track without analysis still gets its BPM");
    }

    #[test]
    fn a_refused_typed_bpm_restores_dat_and_ext() {
        let f = open();
        let state = app_state(&f);
        let dat = rbl_anlz::resolve(&f.location.share_root, RELATIVE);
        let before_dat = std::fs::read(&dat).unwrap();
        let before_ext = std::fs::read(dat.with_extension("EXT")).unwrap();
        let db = Db::open(f.location.clone(), OpenMode::ReadWrite).unwrap();
        db.connection().execute_batch("CREATE TRIGGER refuse_bpm BEFORE UPDATE OF BPM ON djmdContent BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
        drop(db);
        assert!(set_tempo(&state, &f.editor, &Fixture::track(), "150").is_err());
        assert_eq!(std::fs::read(&dat).unwrap(), before_dat);
        assert_eq!(std::fs::read(dat.with_extension("EXT")).unwrap(), before_ext);
        for invalid in ["NaN", "inf", "0", "401", "not a tempo"] {
            assert!(set_tempo(&state, &f.editor, &Fixture::track(), invalid).is_err());
        }
        assert_eq!(std::fs::read(&dat).unwrap(), before_dat);
    }

    #[test]
    fn accepted_tap_run_shares_one_undo_transaction() {
        let f = open();
        let original = f.dat().beat_grid().unwrap();
        let options = GridOptions { transaction: Some("tap-test-run".into()), duration_ms: Some(4500), ..GridOptions::default() };
        let mut save = |_bpm| Ok(());
        for bpm in [120.0, 121.25] {
            apply_options(&f.editor, &f.library, &f.location, &Fixture::track(), GridAction::Edit {
                edit: GridEdit::Tap { bpm, anchor_ms: 1600 }, from_ms: None,
            }, &options, &mut save, None).unwrap();
        }
        assert_eq!(f.editor.histories.lock()[&Fixture::track()].undo.len(), 1);
        let last_tap = f.dat().beat_grid().unwrap();
        apply(&f.editor, &f.library, &f.location, &Fixture::track(), GridAction::Undo, &mut save).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), original);
        // Redo lands on the run's last tap, not its first.
        apply(&f.editor, &f.library, &f.location, &Fixture::track(), GridAction::Redo, &mut save).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), last_tap);
    }

    /// The bytes and modification times of the track's `.DAT` and `.EXT`.
    fn files_now(f: &Fixture) -> Vec<(Vec<u8>, std::time::SystemTime)> {
        let dat = rbl_anlz::resolve(&f.location.share_root, RELATIVE);
        [dat.clone(), dat.with_extension("EXT")]
            .iter()
            .map(|file| (std::fs::read(file).unwrap(), std::fs::metadata(file).unwrap().modified().unwrap()))
            .collect()
    }

    fn nudge() -> GridAction {
        GridAction::Edit { edit: GridEdit::Nudge { ms: 20 }, from_ms: None }
    }

    /// wbx-43 review A: with Library Protection on, a grid edit, its Undo,
    /// its Redo and the Info panel's typed BPM are refused before any
    /// analysis file is published: each file's bytes and modification time
    /// and the row's BPM stay as they were, and the history keeps both steps.
    #[test]
    fn grid_edits_undo_redo_and_a_typed_bpm_leave_every_file_alone_while_protection_is_on() {
        let f = open();
        let state = app_state(&f);
        let track = Fixture::track();
        let options = GridOptions::default();
        guarded(&state, &f.editor, &f.library, &f.location, &track, nudge(), &options).unwrap();
        guarded(&state, &f.editor, &f.library, &f.location, &track, nudge(), &options).unwrap();
        guarded(&state, &f.editor, &f.library, &f.location, &track, GridAction::Undo, &options).unwrap();
        state.core().set_protection(true).unwrap();
        let files = files_now(&f);
        let bpm = row_bpm(&state);

        for action in [nudge(), GridAction::Undo, GridAction::Redo] {
            let refused = guarded(&state, &f.editor, &f.library, &f.location, &track, action, &options).unwrap_err();
            assert_eq!(refused.message, rbl_library::protection::MESSAGE, "{action:?}");
        }
        let typed = set_tempo(&state, &f.editor, &track, "150").unwrap_err();

        assert_eq!(typed.message, rbl_library::protection::MESSAGE);
        assert_eq!(files_now(&f), files);
        assert_eq!(row_bpm(&state), bpm);
        let grid = with_shared_history(&state, &track, f.editor.state_for(&track, &f.dat().beat_grid().unwrap()));
        assert!(grid.can_undo && grid.can_redo);

        state.core().set_protection(false).unwrap();
        guarded(&state, &f.editor, &f.library, &f.location, &track, GridAction::Redo, &options).unwrap();
        assert_ne!(files_now(&f), files);
    }

    fn row_bpm(state: &AppState) -> u32 {
        state.read_db(|db| Ok(db.connection().query_row(
            "SELECT BPM FROM djmdContent WHERE ID=?1", [Fixture::track()], |r| r.get(0))?)).unwrap()
    }

    /// A grid action as the window makes it: a step of the shared history.
    fn in_window(f: &Fixture, state: &AppState, action: GridAction) -> AppResult<GridOutcome> {
        guarded(state, &f.editor, &f.library, &f.location, &Fixture::track(), action, &GridOptions::default())
    }

    #[test]
    fn a_typed_bpm_is_one_grid_undo_step_and_keeps_the_earlier_ones() {
        // #505: the Info panel's BPM is recorded as the grid editor's Set
        // Tempo. Undo puts the grid and the row back together, Redo puts the
        // typed tempo back, and the grid edit made before it stays undoable.
        // Both are steps of the one shared history (wbx-64).
        let f = open();
        let state = app_state(&f);
        in_window(&f, &state, GridAction::Edit { edit: GridEdit::Nudge { ms: 20 }, from_ms: None }).unwrap();
        let nudged = f.dat().beat_grid().unwrap();
        assert_eq!(row_bpm(&state), 12_000);

        set_tempo(&state, &f.editor, &Fixture::track(), "150").unwrap();
        let typed = f.dat().beat_grid().unwrap();
        assert!(typed.iter().all(|b| b.tempo_x100 == 15_000));
        assert_eq!(row_bpm(&state), 15_000);
        let after_typing = with_shared_history(&state, &Fixture::track(), f.editor.state_for(&Fixture::track(), &typed));
        assert_eq!((after_typing.can_undo, after_typing.undo_label, after_typing.can_redo), (true, Some("Set Tempo"), false));

        let undone = in_window(&f, &state, GridAction::Undo).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), nudged, "the grid is the one before the typed BPM");
        assert_eq!(row_bpm(&state), 12_000, "and so is the row");
        assert_eq!((undone.state.undo_label, undone.state.redo_label), (Some("Shift Beat Grid Right"), Some("Set Tempo")));

        in_window(&f, &state, GridAction::Redo).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), typed);
        assert_eq!(row_bpm(&state), 15_000);

        in_window(&f, &state, GridAction::Undo).unwrap();
        in_window(&f, &state, GridAction::Undo).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), grid(), "the earlier nudge undid too");
        assert_eq!(row_bpm(&state), 12_000);
        assert!(!with_shared_history(&state, &Fixture::track(), f.editor.state_for(&Fixture::track(), &grid())).can_undo);
    }

    #[test]
    fn a_typed_bpm_after_an_undo_ends_the_redo_branch() {
        let f = open();
        let state = app_state(&f);
        in_window(&f, &state, GridAction::Edit { edit: GridEdit::Double, from_ms: None }).unwrap();
        in_window(&f, &state, GridAction::Undo).unwrap();
        set_tempo(&state, &f.editor, &Fixture::track(), "125").unwrap();
        let now = f.dat().beat_grid().unwrap();
        let after = with_shared_history(&state, &Fixture::track(), f.editor.state_for(&Fixture::track(), &now));
        assert_eq!((after.can_redo, after.undo_label), (false, Some("Set Tempo")));
    }

    #[test]
    fn a_refused_typed_bpm_records_nothing() {
        let mut f = open();
        f.edit(GridEdit::Nudge { ms: 10 });
        let state = app_state(&f);
        let db = Db::open(f.location.clone(), OpenMode::ReadWrite).unwrap();
        db.connection().execute_batch("CREATE TRIGGER refuse_bpm BEFORE UPDATE OF BPM ON djmdContent BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
        drop(db);
        assert!(set_tempo(&state, &f.editor, &Fixture::track(), "150").is_err());
        let history = &f.editor.histories.lock()[&Fixture::track()];
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].label, "Shift Beat Grid Right");
    }

    /// Writes `beats` into the fixture's analysis file the way another
    /// program would: outside this editor and its history.
    fn written_elsewhere(f: &Fixture, beats: &[Beat]) {
        let dat = rbl_anlz::resolve(&f.location.share_root, RELATIVE);
        std::fs::write(&dat, f.dat().with_beat_grid(beats)).unwrap();
    }

    #[test]
    fn an_undo_over_a_grid_changed_elsewhere_is_refused_and_keeps_the_newer_grid() {
        // #506's policy on the grid history: a grid written since the entry
        // (rekordbox, say, while RBXport stayed open) is kept, and the
        // Undo says why it did not happen.
        let mut f = open();
        f.edit(GridEdit::Double);
        let elsewhere: Vec<Beat> = grid().into_iter().map(|b| Beat { time_ms: b.time_ms + 7, ..b }).collect();
        written_elsewhere(&f, &elsewhere);
        let refused = f.run(GridAction::Undo).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Conflict);
        assert_eq!(refused.message, "Can't undo Double Tempo: the track's beat grid has changed since. The newer change was kept.");
        assert_eq!(f.dat().beat_grid().unwrap(), elsewhere, "the newer grid stays");
        assert_eq!(f.bpms.len(), 1, "the row was not written either");
        let now = f.editor.state_for(&Fixture::track(), &elsewhere);
        assert!(!now.can_undo && !now.can_redo, "the conflicting entry was dropped");
    }

    #[test]
    fn a_refused_undo_drops_only_its_entry_and_the_older_ones_still_undo() {
        // A grid put back elsewhere to the one before the newest edit: that
        // edit's Undo is refused and dropped, the one before it still fits.
        let mut f = open();
        f.edit(GridEdit::Nudge { ms: 10 });
        let nudged = f.dat().beat_grid().unwrap();
        f.edit(GridEdit::Double);
        written_elsewhere(&f, &nudged);
        assert_eq!(f.run(GridAction::Undo).unwrap_err().kind, ErrorKind::Conflict);
        let left = f.editor.state_for(&Fixture::track(), &nudged);
        assert_eq!((left.undo_label, left.can_redo), (Some("Shift Beat Grid Right"), false));
        f.run(GridAction::Undo).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), grid());
    }

    #[test]
    fn a_redo_over_a_grid_changed_elsewhere_is_refused_and_other_tracks_keep_theirs() {
        let mut f = open();
        f.edit(GridEdit::Halve);
        f.run(GridAction::Undo).unwrap();
        f.editor.histories.lock().entry("other".into()).or_default().undo.push(HistoryEntry { before: grid(), after: grid(), label: "Set Tempo" });
        let elsewhere: Vec<Beat> = grid().into_iter().map(|b| Beat { time_ms: b.time_ms + 3, ..b }).collect();
        written_elsewhere(&f, &elsewhere);
        let refused = f.run(GridAction::Redo).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Conflict);
        assert_eq!(refused.message, "Can't redo Halve Tempo: the track's beat grid has changed since. The newer change was kept.");
        assert_eq!(f.dat().beat_grid().unwrap(), elsewhere);
        assert!(f.editor.state_for("other", &grid()).can_undo, "an unrelated track's history is preserved");
    }

    #[test]
    fn the_edit_wire_shape_is_a_tagged_kind() {
        let edit: GridEdit = serde_json::from_str(r#"{"kind":"nudge","ms":-3}"#).unwrap();
        assert_eq!(edit, GridEdit::Nudge { ms: -3 });
        let edit: GridEdit = serde_json::from_str(r#"{"kind":"tempo","bpmX100":12800,"anchorMs":500}"#).unwrap();
        assert_eq!(edit, GridEdit::Tempo { bpm_x100: 12_800, anchor_ms: 500 });
        assert!(serde_json::from_str::<GridEdit>(r#"{"kind":"reset"}"#).is_err());
        // Times are unsigned: the panel clamps a playhead before the track's
        // start to zero, because Tauri refuses this with a bare string (#107).
        assert!(serde_json::from_str::<GridEdit>(r#"{"kind":"downbeat","timeMs":-120}"#).is_err());
        assert_eq!(Edit::from(GridEdit::Halve), Edit::Halve);
    }

    /// 200 BPM from 100 ms for ten beats, then a tempo change to 100 BPM:
    /// the grid in the #107 recording.
    fn with_tempo_change(f: &Fixture) {
        let mut beats: Vec<Beat> = (0..10_u16).map(|i| Beat { beat_number: (i % 4) + 1, tempo_x100: 20_000, time_ms: 100 + u32::from(i) * 300 }).collect();
        beats.extend((0..10_u16).map(|i| Beat { beat_number: ((10 + i) % 4) + 1, tempo_x100: 10_000, time_ms: 3100 + u32::from(i) * 600 }));
        let mut file = AnlzBuilder::new();
        file.path("/x.mp3").beat_grid(&beats).vbr_table_zero().cue_lists(false);
        std::fs::write(rbl_anlz::resolve(&f.location.share_root, RELATIVE), file.finish()).unwrap();
    }

    fn confirmed(f: &Fixture, edit: GridEdit, from_ms: Option<u32>, allow_dynamic: bool) -> AppResult<GridOutcome> {
        let options = GridOptions { allow_dynamic, duration_ms: Some(9100), ..GridOptions::default() };
        apply_options(&f.editor, &f.library, &f.location, &Fixture::track(), GridAction::Edit { edit, from_ms }, &options, &mut |_| Ok(()), None)
    }

    #[test]
    fn a_confirmed_tempo_edit_over_tempo_changes_replaces_them() {
        // #107 rows 2 and 3: rekordbox asks, then deletes the tempo changes in
        // scope. Unconfirmed, the backend refuses with a reason; confirmed, the
        // whole track (or everything from the scope point) takes one tempo.
        let f = open();
        with_tempo_change(&f);
        let refused = confirmed(&f, GridEdit::Stretch { by_ms: -1, time_ms: 400 }, None, false).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Malformed);
        assert_ne!(refused.message, "");

        confirmed(&f, GridEdit::Stretch { by_ms: -1, time_ms: 400 }, None, true).unwrap();
        let beats = f.dat().beat_grid().unwrap();
        assert!(beats.iter().all(|b| b.tempo_x100 == beats[0].tempo_x100), "no tempo change survives");
        assert_eq!(beats[0].time_ms, 100);

        with_tempo_change(&f);
        confirmed(&f, GridEdit::Tempo { bpm_x100: 12_000, anchor_ms: 0 }, None, true).unwrap();
        assert!(f.dat().beat_grid().unwrap().iter().all(|b| b.tempo_x100 == 12_000));

        // From a beat inside the first section: the head before it stays.
        with_tempo_change(&f);
        confirmed(&f, GridEdit::Tempo { bpm_x100: 15_000, anchor_ms: 1000 }, Some(1000), true).unwrap();
        let beats = f.dat().beat_grid().unwrap();
        assert!(beats.iter().filter(|b| b.time_ms < 1000).all(|b| b.tempo_x100 == 20_000));
        assert!(beats.iter().filter(|b| b.time_ms >= 1000).all(|b| b.tempo_x100 == 15_000));
    }

    /// wbx-64 review 4: a tapped tempo over tempo changes replaces them, so
    /// it is refused unless the DJ confirmed, as tempo and stretch are; the
    /// refusal leaves every beat where it was.
    #[test]
    fn a_tap_over_tempo_changes_is_refused_unless_confirmed() {
        let f = open();
        with_tempo_change(&f);
        let before = f.dat().beat_grid().unwrap();
        let refused = confirmed(&f, GridEdit::Tap { bpm: 124.0, anchor_ms: 400 }, None, false).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Malformed);
        assert_eq!(f.dat().beat_grid().unwrap(), before, "a refused tap moves no beat");

        confirmed(&f, GridEdit::Tap { bpm: 124.0, anchor_ms: 400 }, None, true).unwrap();
        assert!(f.dat().beat_grid().unwrap().iter().all(|b| b.tempo_x100 == 12_400));
    }

    #[test]
    fn a_downbeat_at_the_start_puts_beat_one_at_zero() {
        // What the panel sends for "set 1st beat" with the playhead before
        // the track: rekordbox puts the bar's first beat at the very start
        // (as reported in #107).
        let mut f = open();
        f.edit(GridEdit::Downbeat { time_ms: 0 });
        let beats = f.dat().beat_grid().unwrap();
        assert_eq!((beats[0].time_ms, beats[0].beat_number), (0, 1));
        assert_eq!(beats[1].time_ms, 500);
    }

    #[test]
    fn a_nudge_rewrites_the_grid_empties_the_extended_grid_and_leaves_the_row() {
        let mut f = open();
        let before_ext = f.ext();
        assert_ne!(before_ext.section(b"PQT2").unwrap().payload, [] as [u8; 0]);

        let outcome = f.edit(GridEdit::Nudge { ms: 20 });
        assert!(outcome.written);
        assert!(!outcome.bpm_changed);
        assert_eq!(f.bpms, vec![12000], "phase edits also update the analysis revision");
        assert_eq!(&f.times()[..9], &[20,520,1020,1520,2020,2520,3020,3520,4020]);
        assert_eq!(outcome.beats[0], (20, 4));
        assert!(outcome.state.can_undo);
        assert_eq!(outcome.state.bpm_x100, 12000);

        // Only the grid changed in the `.DAT`, and only `PQT2` in the `.EXT`.
        let dat = f.dat();
        assert_eq!(dat.path().as_deref(), Some("/x.mp3"));
        assert_eq!(dat.sections.len(), 5);
        let ext = f.ext();
        assert_eq!(ext.section(b"PQT2").unwrap().payload, [] as [u8; 0]);
        assert_eq!(ext.sections.len(), before_ext.sections.len());
        assert_eq!(ext.sections[0], before_ext.sections[0]);
    }

    #[test]
    fn a_tempo_change_sets_the_row_and_undo_sets_it_back() {
        let mut f = open();
        let outcome = f.edit(GridEdit::Stretch { by_ms: -2, time_ms: 1000 });
        assert!(outcome.bpm_changed);
        assert_eq!(f.bpms, vec![12_048]);
        assert_eq!(outcome.state.bpm_x100, 12_048);

        let undone = f.run(GridAction::Undo).unwrap();
        assert_eq!(f.bpms, vec![12_048, 12_000]);
        assert_eq!(f.times(), (0..8).map(|i| 500 + i * 500).collect::<Vec<_>>());
        assert_eq!(undone.state, GridStateDto { bpm_x100: 12_000, beats: 8, can_undo: false, can_redo: true, undo_label: None, redo_label: Some("Adjust Tempo"), locked: false });

        let redone = f.run(GridAction::Redo).unwrap();
        assert_eq!(f.bpms, vec![12_048, 12_000, 12_048]);
        assert!(redone.state.can_undo);
        assert_eq!(redone.state.undo_label, Some("Adjust Tempo"));
        assert_eq!(redone.state.redo_label, None);
        assert!(!redone.state.can_redo);

        // A new edit forgets what could have been redone.
        f.run(GridAction::Undo).unwrap();
        assert!(f.editor.state_for(&Fixture::track(), &grid()).can_redo);
        let edited = f.edit(GridEdit::Nudge { ms: 1 });
        assert!(!edited.state.can_redo, "a fresh edit ends the redo stack");
        assert_eq!(edited.state.undo_label, Some("Shift Beat Grid Right"));
        assert_eq!(edited.state.redo_label, None);
        let err = f.run(GridAction::Redo).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    #[test]
    fn nothing_to_undo_is_said_not_written() {
        let mut f = open();
        let err = f.run(GridAction::Undo).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert_eq!(f.times()[0], 500);
    }

    #[test]
    fn an_edit_that_changes_nothing_writes_nothing() {
        let mut f = open();
        f.edit(GridEdit::Nudge { ms: 0 });
        let stamp = std::fs::metadata(rbl_anlz::resolve(&f.location.share_root, RELATIVE)).unwrap().modified().unwrap();
        let outcome = f.edit(GridEdit::Nudge { ms: 0 });
        assert!(!outcome.written);
        assert!(outcome.state.can_undo);
        let after = std::fs::metadata(rbl_anlz::resolve(&f.location.share_root, RELATIVE)).unwrap().modified().unwrap();
        assert_eq!(stamp, after);
    }

    #[test]
    fn an_edit_from_a_point_keeps_the_head() {
        let mut f = open();
        f.run(GridAction::Edit { edit: GridEdit::Nudge { ms: 100 }, from_ms: Some(2500) }).unwrap();
        assert_eq!(&f.times()[..9], &[0,500,1000,1500,2000,2600,3100,3600,4100]);
    }

    #[test]
    fn a_refused_undo_keeps_the_history_and_both_file_images() {
        let mut f = open();
        f.edit(GridEdit::Double);
        let before_dat = f.dat().beat_grid().unwrap();
        let before_ext = std::fs::read(rbl_anlz::resolve(&f.location.share_root, RELATIVE).with_extension("EXT")).unwrap();
        let mut refuse = |_bpm: u32| Err(AppError::new(ErrorKind::ReadOnly, "no"));
        assert!(apply(&f.editor, &f.library, &f.location, &Fixture::track(), GridAction::Undo, &mut refuse).is_err());
        assert_eq!(f.dat().beat_grid().unwrap(), before_dat);
        assert_eq!(std::fs::read(rbl_anlz::resolve(&f.location.share_root, RELATIVE).with_extension("EXT")).unwrap(), before_ext);
        assert!(f.editor.state_for(&Fixture::track(), &before_dat).can_undo);
        assert!(!f.editor.state_for(&Fixture::track(), &before_dat).can_redo);
        f.run(GridAction::Undo).unwrap();
        assert_eq!(f.dat().beat_grid().unwrap(), grid());
    }

    #[test]
    fn a_row_that_refuses_the_tempo_gets_its_grid_back() {
        let mut f = open();
        let mut refuse = |_bpm: u32| Err(AppError::new(ErrorKind::ReadOnly, "no"));
        let err = apply(
            &f.editor,
            &f.library,
            &f.location,
            &Fixture::track(),
            GridAction::Edit { edit: GridEdit::Double, from_ms: None },
            &mut refuse,
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::ReadOnly);
        assert_eq!(f.times().len(), 8, "the doubled grid was put back");
        assert!(!f.run(GridAction::Undo).is_ok_and(|o| o.written), "nothing to undo either");
    }

    #[test]
    fn the_lock_refuses_edits_and_outlives_the_editor() {
        let mut f = open();
        f.editor.set_locked(&Fixture::track(), true).unwrap();
        let err = f.edit_result(GridEdit::Halve).unwrap_err();
        assert_eq!(err.kind, ErrorKind::ReadOnly);
        assert_eq!(f.times().len(), 8);
        assert!(state_of(&f.editor, &f.library, &f.location.share_root, &Fixture::track()).unwrap().locked);

        let again = GridEditor::at(&f.dir.path().join("app"));
        assert!(again.is_locked(&Fixture::track()));
        again.set_locked(&Fixture::track(), false).unwrap();
        assert!(!GridEditor::at(&f.dir.path().join("app")).is_locked(&Fixture::track()));
        assert!(!again.is_locked("other"));
    }

    impl Fixture {
        fn edit_result(&mut self, edit: GridEdit) -> AppResult<GridOutcome> {
            self.run(GridAction::Edit { edit, from_ms: None })
        }
    }

    #[test]
    fn the_first_rewrite_backs_the_files_up_and_later_ones_do_not_touch_the_copy() {
        let mut f = open();
        f.edit(GridEdit::Nudge { ms: 5 });
        let copy = f.dir.path().join("app/backups/analysis/PIONEER/USBANLZ/ab1/cd2/ANLZ0000.DAT");
        let kept = rbl_anlz::Anlz::read(&copy).unwrap().beat_grid().unwrap();
        assert_eq!(kept[0].time_ms, 500, "the copy is the grid before any edit");
        assert!(copy.with_extension("EXT").is_file());
        f.edit(GridEdit::Nudge { ms: 5 });
        // A second session finds the copy already there and leaves it.
        let later = GridEditor::at(&f.dir.path().join("app"));
        let mut none = |_bpm: u32| Ok(());
        apply(&later, &f.library, &f.location, &Fixture::track(), GridAction::Edit { edit: GridEdit::Nudge { ms: 5 }, from_ms: None }, &mut none).unwrap();
        assert_eq!(rbl_anlz::Anlz::read(&copy).unwrap().beat_grid().unwrap()[0].time_ms, 500);
        assert_eq!(f.times()[0], 15);
    }

    #[test]
    fn an_edit_past_the_history_cap_drops_the_oldest_grid_so_the_first_one_cannot_be_undone_to() {
        let mut f = open();
        // One edit more than the cap fits: each pushes the grid it replaced,
        // and the push past the cap takes the front of the stack off.
        let edits = HISTORY_CAP + 1;
        for _ in 0..edits {
            f.edit(GridEdit::Nudge { ms: 1 });
        }
        let cap = u32::try_from(HISTORY_CAP).unwrap();
        assert_eq!(f.times()[0], cap + 1);
        assert_eq!(f.editor.histories.lock()[&Fixture::track()].undo.len(), HISTORY_CAP, "the stack stops at the cap");

        // Undoing as far as the stack goes lands on the grid after the first
        // edit, not on the grid the track started with: the entry holding it
        // was the one dropped, and nothing else keeps it. The backup on disk
        // is what the original grid survives in.
        for _ in 0..HISTORY_CAP {
            f.run(GridAction::Undo).unwrap();
        }
        assert_eq!(f.times()[0], 1, "one edit in, not back at 500");
        let err = f.run(GridAction::Undo).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(!f.editor.state_for(&Fixture::track(), &grid()).can_undo);

        // Everything undone is still redoable: the cap is on the undo stack
        // after a push, and a redo puts grids back without being capped.
        assert_eq!(f.editor.histories.lock()[&Fixture::track()].redo.len(), HISTORY_CAP);
        for _ in 0..HISTORY_CAP {
            f.run(GridAction::Redo).unwrap();
        }
        assert_eq!(f.times()[0], cap + 1);
        assert_eq!(f.editor.histories.lock()[&Fixture::track()].undo.len(), HISTORY_CAP);
        assert!(f.bpms.iter().all(|b| *b == 12000));
    }

    #[test]
    fn a_track_without_analysis_has_no_grid_to_edit() {
        let mut f = open();
        let mut none = |_bpm: u32| Ok(());
        let err = apply(&f.editor, &f.library, &f.location, &track_id(2), GridAction::Edit { edit: GridEdit::Double, from_ms: None }, &mut none).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        let err = state_of(&f.editor, &f.library, &f.location.share_root, "no-such-track").unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(f.run(GridAction::Redo).is_err());
    }
}
