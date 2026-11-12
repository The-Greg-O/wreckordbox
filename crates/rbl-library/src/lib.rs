//! The Library core: the loaded index, its open views, the change
//! generation, and the single path every Library edit takes.
//!
//! Nothing here knows about a window. Whoever holds a [`LibraryCore`] (the
//! desktop shell today, the Library service of ADR 0002 next) supplies a
//! [`Host`] for the few things the core borrows, and tells its own clients
//! what changed from the [`Change`] an edit returns.
//!
//! The write guards are `rbl-db`'s and are not repeated here: every write
//! opens an [`rbl_db::write::Writer`], which refuses while rekordbox is
//! running and, under `RBXPORT_TEST`, for a detected real Library.

pub mod entry_order;
pub mod history;
pub mod ledger;
pub mod my_tags;
pub mod protection;
pub mod file_edit;
pub mod read;
pub mod stick;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use rbl_index::device::DeviceView;
use rbl_index::folder::FolderView;
use rbl_index::{Library, ListTotals, SearchField, TrackSource, View, ViewSpec};

/// Views are dropped oldest-first past this many, so a user clicking through
/// playlists cannot grow memory without bound.
const MAX_VIEWS: usize = 16;

/// What [`LibraryCore::location`] and [`LibraryCore::library`] say before a
/// Library has been loaded.
pub const NOT_LOADED: &str = "The library has not finished loading yet.";

/// No Library has been loaded yet. Displays as [`NOT_LOADED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotLoaded;

impl core::fmt::Display for NotLoaded {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(NOT_LOADED)
    }
}

impl std::error::Error for NotLoaded {}

/// What an edit changed, and therefore how much has to be re-read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Touched {
    /// Only the playlist tree. Re-reading it costs 24 ms against 233 ms for
    /// the whole library, and it is by far the most common kind of edit.
    Playlists,
    /// A track column changed, so the ranks and the search arena are stale.
    Tracks,
    /// Only the Tag List.
    TagList,
    /// Existing tracks: rating, colour, comment, or play count.
    Metadata(Vec<String>),
    /// History membership, optionally with play counts to refresh.
    Histories(Vec<String>),
    /// One track's cues, re-read on their own as a cue edit does (#504).
    Cues(String),
}

/// Which kind of data a change touched, as a client is told it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Anything a list of tracks or the tree shows: every open list is stale.
    Library,
    /// Only the Tag List: every other view stays as it was, under the same
    /// generation.
    TagList,
}

impl Touched {
    /// The kind of change a client is told about.
    pub fn change_kind(&self) -> ChangeKind {
        if matches!(self, Self::TagList) { ChangeKind::TagList } else { ChangeKind::Library }
    }
}

/// What an edit left behind: the kind of data it touched and the generation
/// the index is at now. A Tag List change keeps the generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub generation: u32,
}

/// Who keeps the edit history straight for a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum History {
    /// The caller keeps the history itself (recording an entry, Undo, Redo).
    Caller,
    /// The write is outside the history: if it changed any row, the
    /// history's Redo branch ends and the [`Host`] is told.
    Outside,
}

/// What the core borrows from whoever holds it. Every method has a default
/// that does nothing, which is all a holder with no window state needs.
pub trait Host {
    /// Finishes or rolls back an analysis-file edit a crash interrupted,
    /// before another write can change what decides its outcome. Called
    /// under the edit gate, and not while an analysis write is being
    /// published. An error refuses the write with its text.
    fn recover_interrupted_files(&self, _location: &rbl_db::LibraryLocation) -> Result<(), String> {
        Ok(())
    }

    /// A write outside the edit history changed at least one row. The core
    /// has ended the history's Redo branch itself.
    fn wrote_outside_history(&self) {}

    /// What the edit history offers changed without a command of the
    /// holder's own doing it: a write outside the history ended a Redo
    /// branch, or [`LibraryCore::recorded_edit`] recorded an entry (a
    /// client's edit, for a window hosting the service). Called under the
    /// edit gate with no other lock of the core's held.
    fn history_changed(&self) {}

    /// The index is about to be replaced by `next`, read afresh. Called
    /// before the core takes its own lock.
    fn library_replacing(&self, _next: &Library) {}

    /// The index was replaced by a fresh read; `another_library` when it is
    /// a different Library from the one before. Called while the core's
    /// lock is held: it must not read the core.
    fn library_replaced(&self, _another_library: bool) {}
}

/// A holder that lends the core nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHost;

impl Host for NoHost {}

/// A library view, and whether it lists the Tag List: the one source a Tag
/// List edit makes stale.
struct OpenView {
    view: Arc<View>,
    tag_list: bool,
}

/// A view of any kind, on its way into the table.
enum Registered {
    Library(OpenView),
    Folder(Arc<FolderView>),
    Device(Arc<DeviceView>),
}

#[derive(Default)]
struct Inner {
    library: Option<Arc<Library>>,
    /// Where the library is, decided once when it is loaded. Every later open
    /// — the reader, the writer, a reload — goes through this rather than
    /// detecting again, which is also what lets a test point the whole core
    /// at a fixture.
    location: Option<rbl_db::LibraryLocation>,
    read_only: bool,
    db_version: Option<i64>,
    load_ms: u64,
    views: HashMap<u32, OpenView>,
    /// The Explorer's views, under the same ids and the same eviction.
    folders: HashMap<u32, Arc<FolderView>>,
    /// The Devices tree's views of a stick's own library, likewise.
    devices: HashMap<u32, Arc<DeviceView>>,
    /// Insertion order, for eviction.
    view_order: Vec<u32>,
    next_view_id: u32,
    /// Bumped when the library is reloaded; invalidates cached pages.
    generation: u32,
}

impl Inner {
    /// Hands out the next id and evicts the oldest view past the cap, whichever
    /// kind it is.
    fn register(&mut self, view: Registered) -> u32 {
        let id = self.next_view_id;
        self.next_view_id = self.next_view_id.wrapping_add(1).max(1);
        match view {
            Registered::Library(view) => {
                self.views.insert(id, view);
            }
            Registered::Folder(view) => {
                self.folders.insert(id, view);
            }
            Registered::Device(view) => {
                self.devices.insert(id, view);
            }
        }
        self.view_order.push(id);
        while self.view_order.len() > MAX_VIEWS {
            let oldest = self.view_order.remove(0);
            self.views.remove(&oldest);
            self.folders.remove(&oldest);
            self.devices.remove(&oldest);
        }
        id
    }
}

/// The loaded Library and the one path that edits it.
pub struct LibraryCore {
    /// Held for the whole of an edit, refresh included. Reentrant, so a
    /// caller can take it around an edit and what it does with the result.
    edit_gate: parking_lot::ReentrantMutex<()>,
    /// Held while an analysis or grid edit publishes its own file journal.
    analysis_write: parking_lot::Mutex<()>,
    /// Stable local home for recovery journals.
    recovery_dir: PathBuf,
    inner: RwLock<Inner>,
    /// Who is told of every change, whoever made it. See [`Self::watch`].
    watchers: parking_lot::Mutex<Vec<Watcher>>,
    /// Why nothing may be written through this core, when its holder has
    /// said so. See [`Self::refuse_writes`].
    refusal: parking_lot::Mutex<Option<String>>,
    /// Undo and Redo of the edits made through this core, whoever made
    /// them. A leaf lock; see [`Self::history`].
    history: parking_lot::Mutex<history::EditHistory>,
    /// Library Protection, kept in the recovery folder; checked under the
    /// edit gate before every write. See [`protection`].
    protection: protection::Switch,
}

type Watcher = Box<dyn FnMut(Change) -> bool + Send>;

impl LibraryCore {
    /// A core with no Library loaded yet, keeping its recovery journals
    /// under `recovery_dir`. The location arrives with the Library, in
    /// [`Self::set_library`].
    pub fn new(recovery_dir: impl Into<PathBuf>) -> Self {
        let recovery_dir = recovery_dir.into();
        Self {
            edit_gate: parking_lot::ReentrantMutex::new(()),
            analysis_write: parking_lot::Mutex::new(()),
            protection: protection::Switch::new(recovery_dir.clone()),
            recovery_dir,
            inner: RwLock::new(Inner { next_view_id: 1, generation: 1, ..Inner::default() }),
            watchers: parking_lot::Mutex::new(Vec::new()),
            refusal: parking_lot::Mutex::new(None),
            history: parking_lot::Mutex::new(history::EditHistory::default()),
        }
    }

    /// Tells `watcher` of every change from now on, in the order they
    /// happen, until it answers `false`: an edit, a reload, anything that
    /// moves the generation or the Tag List, whoever made it. This is how a
    /// service sharing the core with a window tells its clients what the
    /// window did.
    ///
    /// The watcher runs while the core's lock is held. It must not read the
    /// core, block, or do anything slow: hand the change to a channel.
    pub fn watch(&self, watcher: impl FnMut(Change) -> bool + Send + 'static) {
        self.watchers.lock().push(Box::new(watcher));
    }

    /// Call with the core's lock held, so watchers hear changes in order.
    fn tell_watchers(&self, kind: ChangeKind, generation: u32) {
        self.watchers.lock().retain_mut(|watcher| watcher(Change { kind, generation }));
    }

    /// Refuses every write through this core with `reason`, or allows them
    /// again with `None`. For a holder that has found it is not the
    /// Library's one writer: another process has the Library's service.
    pub fn refuse_writes(&self, reason: Option<String>) {
        *self.refusal.lock() = reason;
    }

    /// Why writes are refused, when the holder has refused them.
    pub fn write_refusal(&self) -> Option<String> {
        self.refusal.lock().clone()
    }

    /// The gate every edit holds.
    pub fn edit_gate(&self) -> &parking_lot::ReentrantMutex<()> {
        &self.edit_gate
    }

    /// The lock an analysis or grid edit holds while it publishes its files.
    pub fn analysis_write(&self) -> &parking_lot::Mutex<()> {
        &self.analysis_write
    }

    /// Where recovery journals are kept.
    pub fn recovery_dir(&self) -> &Path {
        &self.recovery_dir
    }

    /// Where the loaded Library is.
    pub fn location(&self) -> Result<rbl_db::LibraryLocation, NotLoaded> {
        self.inner.read().location.clone().ok_or(NotLoaded)
    }

    /// The loaded index.
    pub fn library(&self) -> Result<Arc<Library>, NotLoaded> {
        self.inner.read().library.clone().ok_or(NotLoaded)
    }

    /// Where analysis files live for the loaded Library, or nowhere useful
    /// before it has loaded.
    pub fn share_root(&self) -> PathBuf {
        self.inner.read().location.as_ref().map(|l| l.share_root.clone()).unwrap_or_default()
    }

    /// `(read_only, db_version, load_ms, generation)`.
    pub fn summary(&self) -> (bool, Option<i64>, u64, u32) {
        let inner = self.inner.read();
        (inner.read_only, inner.db_version, inner.load_ms, inner.generation)
    }

    /// The loaded index and its generation, read together: what a write
    /// that takes its contents from the Library checks and uses as one.
    pub fn library_at_generation(&self) -> Result<(Arc<Library>, u32), NotLoaded> {
        let inner = self.inner.read();
        Ok((inner.library.clone().ok_or(NotLoaded)?, inner.generation))
    }

    /// The generation the index is at.
    pub fn generation(&self) -> u32 {
        self.inner.read().generation
    }

    /// A fresh read-only handle to the loaded Library. Blocking.
    pub fn open_read_only(&self) -> Result<rbl_db::Library, rbl_db::DbError> {
        if rbl_backup::journal::pending(&self.recovery_dir) {
            return Err(rbl_db::DbError::WriteRefused("A library restore is unfinished. Restart the app to recover it before reading.".into()));
        }
        let location = self.location().map_err(|e| rbl_db::DbError::NotInstalled(e.to_string()))?;
        rbl_db::Library::open(location, rbl_db::OpenMode::ReadOnly)
    }

    /// Opens the Library for writing, runs one edit against it, then
    /// `refresh` on the same connection, and closes it. Refused while
    /// rekordbox is running, and while Library Protection is on
    /// ([`protection::refusal`]).
    ///
    /// Opened per edit rather than held: holding it would keep the database
    /// open read-write for the life of the process, and rekordbox launching
    /// behind us must be able to take the file back. Database backups are
    /// made only by an explicit request, never automatically on edits.
    ///
    /// The encrypted connection and the edit gate are kept through the
    /// refresh, so a small edit neither pays for another key derivation nor
    /// races a writer.
    ///
    /// With [`History::Outside`], a write that changed any row ends the
    /// history's Redo branch and tells the host, as after any edit the
    /// history does not record ("a non-recorded
    /// edit after an undo starts a new branch"). That holds for every such
    /// writer at once (cue edits, analysis, grid saves, players over LINK,
    /// imports) rather than each having to remember (#506).
    ///
    /// Blocking.
    pub fn write_then<T, U>(
        &self,
        host: &(impl Host + ?Sized),
        history: History,
        edit: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, rbl_db::DbError>,
        refresh: impl FnOnce(&rbl_db::Library, T) -> Result<U, rbl_db::DbError>,
    ) -> Result<U, rbl_db::DbError> {
        self.write_then_prepared(host, history, &ledger::Preparation::default(), edit, refresh)
    }

    /// The same edit path, with bounded recovery targets supplied by its caller.
    pub fn write_then_prepared<T, U>(
        &self,
        host: &(impl Host + ?Sized),
        history: History,
        preparation: &ledger::Preparation,
        edit: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, rbl_db::DbError>,
        refresh: impl FnOnce(&rbl_db::Library, T) -> Result<U, rbl_db::DbError>,
    ) -> Result<U, rbl_db::DbError> {
        let started = std::time::Instant::now();
        let location = self.location().map_err(|e| rbl_db::DbError::NotInstalled(e.to_string()))?;
        let _gate = self.edit_gate.lock();
        if let Some(reason) = self.write_refusal() {
            return Err(rbl_db::DbError::WriteRefused(reason));
        }
        // Under the gate, so an edit that was waiting for it when the DJ
        // turned protection on is refused too.
        if self.protection().protect_library {
            return Err(protection::refusal());
        }
        // Stick Protection (wbx-45.11): a Library on a Stick is written,
        // and recovered below, only while the DJ has it unlocked; held to
        // the end of the write. Under the gate, in the one lock order.
        let _stick = stick::leave(&self.recovery_dir, &location)?;
        if rbl_backup::journal::pending(&self.recovery_dir) {
            return Err(rbl_db::DbError::WriteRefused("A library restore is unfinished. Restart the app to recover it before editing.".into()));
        }
        // Grid/analysis edits hold this mutex while intentionally publishing
        // their own journal. Otherwise recover before another edit can change
        // the USN used to decide an interrupted operation's outcome.
        if let Some(_files) = self.analysis_write.try_lock() {
            host.recover_interrupted_files(&location).map_err(rbl_db::DbError::WriteRefused)?;
        }
        // Nothing is changed that the Ledger could not record: it records
        // durably that a write is begun, or the write is refused. Asked
        // with the history let go: the Ledger may block to write.
        let ledger = self.history.lock().ledger();
        if let Some(ledger) = &ledger {
            ledger.begin().map_err(|why| {
                rbl_db::DbError::WriteRefused(format!("The Ledger cannot record a change, so nothing was changed: {why}"))
            })?;
        }
        let gate_ms = started.elapsed().as_millis();
        let mut writer = match rbl_db::write::Writer::open(location.clone(), self.recovery_dir.clone()) {
            Ok(writer) => writer,
            Err(error) => {
                if let Some(ledger) = &ledger {
                    ledger.written(false);
                }
                return Err(error);
            }
        };
        writer.disable_automatic_backups();
        if history == History::Caller {
            if let Some(ledger) = &ledger { ledger.prepare(writer.library(), preparation); }
        }
        let open_ms = started.elapsed().as_millis();
        let result = edit(&mut writer);
        let edit_ms = started.elapsed().as_millis();
        // Counted before the refresh, which only reads. A failed edit that
        // changed rows before it stopped counts too: its rows may not all
        // have been put back.
        let wrote = writer.library().connection().total_changes() > 0;
        // Told before the refresh, which may fail with the change made.
        if let Some(ledger) = &ledger {
            ledger.written(wrote);
        }
        let result = result.and_then(|value| refresh(writer.library(), value));
        tracing::debug!(gate_ms, open_ms = open_ms - gate_ms,
            edit_ms = edit_ms - open_ms, refresh_ms = started.elapsed().as_millis() - edit_ms,
            "library write phases");
        if wrote && history == History::Outside {
            // A Redo replayed over this write would put back values it has
            // since changed. Undo stays: each entry refuses to run over rows
            // changed since.
            let ended = {
                let mut history = self.history.lock();
                let had = history.redo_label().is_some();
                history.unrecorded(None);
                had
            };
            host.wrote_outside_history();
            if ended {
                host.history_changed();
            }
        }
        result
    }

    /// The whole edit path in one call: the guarded write, the refresh of
    /// what it `touched`, and the change to tell clients about, beside the
    /// edit's own result.
    ///
    /// Blocking.
    pub fn edit<T>(
        &self,
        host: &(impl Host + ?Sized),
        history: History,
        touched: Touched,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, rbl_db::DbError>,
    ) -> Result<(T, Change), rbl_db::DbError> {
        let kind = touched.change_kind();
        self.write_then(host, history, action, |db, value| {
            let generation = self.refresh_after_edit(host, db, touched)?;
            Ok((value, Change { kind, generation }))
        })
    }

    /// Re-reads what an edit `touched` through `db`, the connection that
    /// committed it, publishes the new index and answers its generation.
    /// Shared by desktop and CDJ edits; call it under the edit gate, as the
    /// refresh of [`Self::write_then`].
    pub fn refresh_after_edit(
        &self,
        host: &(impl Host + ?Sized),
        db: &rbl_db::Library,
        touched: Touched,
    ) -> Result<u32, rbl_db::DbError> {
        // What the shell's error type displayed here, kept word for word.
        let not_loaded = |e: NotLoaded| rbl_db::DbError::Open(format!("NotFound: {e}"));
        match touched {
            Touched::Metadata(ids) => self.refresh_metadata(db, &ids, false),
            Touched::Histories(ids) if !ids.is_empty() => self.refresh_metadata(db, &ids, true),
            Touched::Cues(track) => {
                let library = self.library().map_err(not_loaded)?;
                rbl_index::reload_cues_of(db, &library, &track)?;
                Ok(self.generation())
            }
            Touched::Tracks => {
                let started = std::time::Instant::now();
                let (library, _) = rbl_index::load(db)?;
                let load_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                self.set_library(host, library, rbl_db::is_rekordbox_running(), db.schema().db_version, load_ms, db.location().clone());
                Ok(self.generation())
            }
            Touched::TagList => {
                let library = self.library().map_err(not_loaded)?;
                library.set_tag_list(rbl_index::reload_tag_list(db, &library)?);
                Ok(self.invalidate_tag_list_views())
            }
            Touched::Playlists => {
                let library = self.library().map_err(not_loaded)?;
                library.set_playlists(rbl_index::reload_playlists(db, &library)?);
                Ok(self.invalidate_views())
            }
            Touched::Histories(_) => {
                let library = self.library().map_err(not_loaded)?;
                library.set_histories(rbl_index::reload_histories(db, &library)?);
                Ok(self.invalidate_views())
            }
        }
    }

    /// Publishes an index read afresh, drops every open view and bumps the
    /// generation.
    pub fn set_library(
        &self,
        host: &(impl Host + ?Sized),
        library: Library,
        read_only: bool,
        db_version: Option<i64>,
        load_ms: u64,
        location: rbl_db::LibraryLocation,
    ) {
        host.library_replacing(&library);
        let mut inner = self.inner.write();
        let another = inner.location.as_ref().is_some_and(|old| old.master_db != location.master_db);
        inner.library = Some(Arc::new(library));
        inner.location = Some(location);
        inner.read_only = read_only;
        inner.db_version = db_version;
        inner.load_ms = load_ms;
        inner.views.clear();
        inner.folders.clear();
        inner.view_order.clear();
        inner.generation = inner.generation.wrapping_add(1).max(1);
        host.library_replaced(another);
        self.tell_watchers(ChangeKind::Library, inner.generation);
    }

    /// Drops every open view and bumps the generation.
    ///
    /// Used after a playlist edit: the track columns are untouched, but a view
    /// over a playlist whose membership changed is stale, and so are the pages
    /// a client has cached against the old generation.
    pub fn invalidate_views(&self) -> u32 {
        let mut inner = self.inner.write();
        inner.views.clear();
        inner.folders.clear();
        inner.view_order.clear();
        inner.generation = inner.generation.wrapping_add(1).max(1);
        self.tell_watchers(ChangeKind::Library, inner.generation);
        inner.generation
    }

    /// Drops the views over the Tag List and keeps every other. No row or
    /// column of any other view shows Tag List membership, so they stay open
    /// under the same generation and a client keeps the pages it has: a
    /// player tagging a track does not reload the playlist on screen.
    pub fn invalidate_tag_list_views(&self) -> u32 {
        let mut inner = self.inner.write();
        let Inner { views, folders, view_order, generation, .. } = &mut *inner;
        views.retain(|_, open| !open.tag_list);
        view_order.retain(|id| views.contains_key(id) || folders.contains_key(id));
        self.tell_watchers(ChangeKind::TagList, *generation);
        *generation
    }

    /// Publish metadata without changing row identities or analysis files.
    /// Call under the edit gate, using the connection that committed the edit.
    pub fn refresh_metadata(&self, db: &rbl_db::Library, ids: &[String], histories: bool) -> Result<u32, rbl_db::DbError> {
        let mut inner = self.inner.write();
        let current = inner.library.as_ref().ok_or_else(|| rbl_db::DbError::Open("No library loaded".into()))?;
        // Build privately so a failed read never publishes a partial update.
        let mut next = (**current).clone();
        rbl_index::reload_metadata(db, &mut next, ids)?;
        if histories { next.set_histories(rbl_index::reload_histories(db, &next)?); }
        inner.library = Some(Arc::new(next));
        inner.views.clear();
        inner.folders.clear();
        inner.view_order.clear();
        inner.generation = inner.generation.wrapping_add(1).max(1);
        self.tell_watchers(ChangeKind::Library, inner.generation);
        Ok(inner.generation)
    }

    /// Opens a view and returns `(view_id, len, generation, totals)`. The
    /// totals are read before the view is registered, so a burst of other
    /// opens that evicts it cannot turn this open into an error.
    pub fn open_view_scoped(
        &self,
        spec: &ViewSpec,
        field: SearchField,
    ) -> Result<(u32, u32, u32, Option<ListTotals>), NotLoaded> {
        let library = self.library()?;
        let view = library.open_view_scoped(spec, field);
        let len = u32::try_from(view.len()).unwrap_or(u32::MAX);
        let totals = view.totals();

        let mut inner = self.inner.write();
        let tag_list = matches!(spec.source, TrackSource::TagList);
        let id = inner.register(Registered::Library(OpenView { view: Arc::new(view), tag_list }));
        let generation = inner.generation;
        Ok((id, len, generation, totals))
    }

    /// Opens the Explorer's view of one folder: `(view_id, len, generation)`.
    pub fn open_folder_view(&self, view: FolderView) -> (u32, u32, u32) {
        let len = u32::try_from(view.len()).unwrap_or(u32::MAX);
        let mut inner = self.inner.write();
        let id = inner.register(Registered::Folder(Arc::new(view)));
        (id, len, inner.generation)
    }

    /// Opens a view of a library on a stick; the same handle shape.
    pub fn open_device_view(&self, view: DeviceView) -> (u32, u32, u32) {
        let len = u32::try_from(view.len()).unwrap_or(u32::MAX);
        let mut inner = self.inner.write();
        let id = inner.register(Registered::Device(Arc::new(view)));
        (id, len, inner.generation)
    }

    /// The device view behind an id, or `None` for any other kind of id.
    pub fn device_view(&self, view_id: u32) -> Option<Arc<DeviceView>> {
        self.inner.read().devices.get(&view_id).cloned()
    }

    /// The folder view behind an id, or `None` when the id is a library view
    /// or nothing at all.
    pub fn folder_view(&self, view_id: u32) -> Option<Arc<FolderView>> {
        self.inner.read().folders.get(&view_id).cloned()
    }

    /// The library view behind an id, or `None` when it was evicted or never
    /// existed.
    pub fn view(&self, view_id: u32) -> Option<Arc<View>> {
        self.inner.read().views.get(&view_id).map(|open| Arc::clone(&open.view))
    }
}
