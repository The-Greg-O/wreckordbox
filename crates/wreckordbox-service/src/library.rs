//! What the service has of the Library: the Library core (the loaded
//! index, the change generation and the single edit path). Its own when it
//! runs by itself; the window's when the window hosts it.
//!
//! Queries are answered from the core's index with the same search the
//! window uses, and an edit takes the core's one edit path, which the
//! window's edits take too: the write guards, the write, and the re-read of
//! what it touched. So a client and the window cannot disagree about what
//! matches or about what an edit does.

use std::path::PathBuf;
use std::sync::Arc;

use rbl_db::{DbError, LibraryLocation, OpenMode};
use rbl_index::{SortColumn, TrackFilter, TrackSource, ViewSpec};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{History, Host, LibraryCore, NoHost, Touched};

use crate::protection;
use crate::protocol::{
    codes, ChangeKind, LibraryChanged, RateTrack, RatedTrack, ServiceError, Track, TrackList,
    TrackQuery, DEFAULT_TRACK_LIMIT,
};
use crate::server::open_error;

/// The most stars a track can have.
const MAX_RATING: u8 = 5;

pub(crate) struct OpenLibrary {
    /// The one in-memory Library. The service's own when it runs by itself;
    /// the window's when the window hosts the service, so an edit from
    /// either is the same edit under one generation.
    core: Arc<LibraryCore>,
    /// A read-only connection a service by itself keeps open; every write
    /// opens its own inside the core and closes it again.
    db: Option<rbl_db::Library>,
    /// What the core borrows for an edit: nothing for a service by itself,
    /// the window's recovery and edit history when the window hosts.
    host: Arc<dyn Host + Send + Sync>,
}

impl OpenLibrary {
    /// Opens the Library and indexes it. Reading every row is also what
    /// proves the file reads as a Library. `recovery_dir` is where the
    /// window keeps its recovery journals.
    pub(crate) fn open(
        location: LibraryLocation,
        recovery_dir: PathBuf,
    ) -> Result<Self, ServiceError> {
        let db = rbl_db::Library::open(location.clone(), OpenMode::ReadOnly)
            .map_err(|e| open_error(&e))?;
        let started = std::time::Instant::now();
        let (index, _) = rbl_index::load(&db).map_err(|e| open_error(&DbError::from(e)))?;
        let load_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let core = LibraryCore::new(recovery_dir);
        core.set_library(
            &NoHost,
            index,
            false,
            db.schema().db_version,
            load_ms,
            location,
        );
        Ok(Self {
            core: Arc::new(core),
            db: Some(db),
            host: Arc::new(NoHost),
        })
    }

    /// The Library a front end already has open, served as it stands: the
    /// front end loads it, and reloads it, through the same core.
    pub(crate) fn shared(core: Arc<LibraryCore>, host: Arc<dyn Host + Send + Sync>) -> Self {
        Self {
            core,
            db: None,
            host,
        }
    }

    /// Hears every change to the Library, whoever made it; see
    /// [`LibraryCore::watch`]. The core itself is not handed out, so that
    /// nothing outside this file can write through it past
    /// [`Self::change`].
    pub(crate) fn watch(&self, watcher: impl FnMut(rbl_library::Change) -> bool + Send + 'static) {
        self.core.watch(watcher);
    }

    /// Tracks in the Library, not counting deleted ones.
    pub(crate) fn track_count(&self) -> Result<u32, ServiceError> {
        Ok(u32::try_from(self.index()?.len()).unwrap_or(u32::MAX))
    }

    /// Runs `read` on a read-only connection, for what the index does not
    /// hold. A service by itself keeps one open; a service the window hosts
    /// opens one for the read, as the window may replace the file under it
    /// (a restore).
    pub(crate) fn with_db<T>(
        &self,
        read: impl FnOnce(&rbl_db::Library) -> Result<T, ServiceError>,
    ) -> Result<T, ServiceError> {
        if let Some(db) = &self.db {
            return read(db);
        }
        // A hosting window that has not read the Library yet: the same
        // answer the index gives, so a client waits and asks again rather
        // than being told there is no Library.
        self.index()?;
        read(&self.core.open_read_only().map_err(|e| open_error(&e))?)
    }

    /// Runs `read` while no edit is under way: it holds the core's edit
    /// gate, which every edit (the window's included) holds from its write
    /// to the re-read of the index. So the index and the database `read`
    /// sees are the same Library, and nothing is edited until it returns.
    /// It writes nothing; keep it short, as an edit waits for it.
    pub(crate) fn steady<T>(&self, read: impl FnOnce() -> T) -> T {
        let _gate = self.core.edit_gate().lock();
        read()
    }

    /// The index and its generation, read under one gate: a Stick load
    /// checks its confirmation against exactly the snapshot it writes from
    /// (wbx-70).
    pub(crate) fn index_at_generation(
        &self,
    ) -> Result<(Arc<rbl_index::Library>, u64), ServiceError> {
        self.core
            .library_at_generation()
            .map(|(index, generation)| (index, u64::from(generation)))
            .map_err(|e| ServiceError::new(codes::LIBRARY_LOADING, e.to_string()))
    }

    /// The folder the Library's analysis files are kept under.
    pub(crate) fn share_root(&self) -> std::path::PathBuf {
        self.core.share_root()
    }

    /// The index as it stands now: a snapshot an edit does not change.
    pub(crate) fn index(&self) -> Result<Arc<rbl_index::Library>, ServiceError> {
        self.core
            .library()
            .map_err(|e| ServiceError::new(codes::LIBRARY_LOADING, e.to_string()))
    }

    /// The collection in its own order, narrowed as the window's search box
    /// narrows it, then cut to the query's limit.
    pub(crate) fn tracks(&self, query: &TrackQuery) -> Result<TrackList, ServiceError> {
        let index = self.index()?;
        let view = index.open_view(&ViewSpec {
            source: TrackSource::Collection,
            sort: SortColumn::TrackNo,
            descending: false,
            query: query.filter.clone(),
            filter: TrackFilter::default(),
        });
        let limit = query.limit.unwrap_or(DEFAULT_TRACK_LIMIT);
        let tracks = view
            .window(0, limit)
            .iter()
            .map(|&row| track(&index, row))
            .collect();
        Ok(TrackList {
            tracks,
            total: view.len(),
        })
    }

    /// The service's one way to change the Library: every command that
    /// does comes through here, whatever it changes and however the edit
    /// history treats it. Library Protection is enforced by the core
    /// itself, under the gate that serialises edits ([`LibraryCore::write_then`]
    /// refuses every write while it is on), so a refusal comes back from
    /// `write` before anything is written, recorded in the edit history or
    /// told to a client; here it becomes [`codes::LIBRARY_PROTECTED`].
    ///
    /// `write` is handed the core and the host to make one of the core's
    /// edits with ([`LibraryCore::edit`], `recorded_edit`,
    /// `permanent_edit`). This is the only place in the crate that reads
    /// `self.host`, which every writing method of the core needs, and the
    /// core is never handed out of this file; `tests/one_edit_gate.rs`
    /// lints for both.
    ///
    /// Reading does not come this way, and neither does writing to the
    /// Sidecar, which is not the Library.
    fn change<T>(
        &self,
        write: impl FnOnce(&LibraryCore, &dyn Host) -> Result<(T, rbl_library::Change), DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        write(&self.core, &*self.host)
            .map(|(value, change)| (value, change.into()))
            .map_err(|e| edit_error(&e))
    }

    /// Gives the core's edit history `ledger` to tell of every edit from
    /// now on (the Ledger, bead wbx-21).
    pub(crate) fn keep_ledger(&self, ledger: Arc<dyn rbl_library::ledger::Ledger>) {
        self.core.history().lock().keep_ledger(Some(ledger));
    }

    /// Takes the Ledger off the core once no edit is under way: the edit
    /// gate is taken first, so an edit in progress (a hosting window's) is
    /// over, and recorded, before `close` runs and the Ledger is removed.
    pub(crate) fn let_go_of_ledger(&self, close: impl FnOnce()) {
        let _gate = self.core.edit_gate().lock();
        close();
        self.core.history().lock().keep_ledger(None);
    }

    /// Library Protection as the core holds it.
    pub(crate) fn protection(&self) -> protection::Protection {
        self.core.protection().into()
    }

    /// Turns Library Protection on or off through the core, for the service
    /// and the window alike, and answers what it was.
    pub(crate) fn protect(&self, on: bool) -> Result<protection::ProtectionSet, ServiceError> {
        let turned = self.core.set_protection(on).map_err(|e| {
            ServiceError::new(
                codes::SERVICE_FAILED,
                format!("could not record Library Protection: {e}"),
            )
        })?;
        Ok(protection::ProtectionSet {
            protect_library: turned.now.protect_library,
            previous: turned.previous,
            revision: turned.now.revision,
        })
    }

    /// Hears every change of Library Protection made through the core, in
    /// order; see [`LibraryCore::watch_protection`].
    pub(crate) fn watch_protection(
        &self,
        watcher: impl FnMut(rbl_library::protection::ProtectionState) -> bool + Send + 'static,
    ) {
        self.core.watch_protection(watcher);
    }

    /// Makes `action`'s writes through the Library's single edit path (the
    /// write guards, the writes, the re-read of what they `touched`), and
    /// answers its result beside what to tell every client. Outside the edit
    /// history: it only ends the Redo branch. For an edit Undo can take
    /// back use [`Self::recorded_edit`]; for one that removes rows for good,
    /// [`Self::permanent_edit`].
    pub(crate) fn edit<T>(
        &self,
        touched: Touched,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        self.change(|core, host| core.edit(host, History::Outside, touched, action))
    }

    /// [`Self::edit`] for an edit Undo can take back: `action` also answers
    /// the entry the edit history keeps under `label`, as the window's own
    /// edit of the kind would. Undo in the hosting window takes it back.
    /// What the Ledger called the latest entry Undo would take back
    /// (wbx-65: to read what it could not record).
    pub(crate) fn latest_ledger_entry(&self) -> Option<i64> {
        self.core.history().lock().latest_ledger_entry()
    }

    pub(crate) fn recorded_edit<T>(
        &self,
        touched: Touched,
        label: &'static str,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<(T, LibraryEdit), DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        self.change(|core, host| core.recorded_edit(host, touched, label, action))
    }

    /// [`Self::recorded_edit`] for a deliberate hand set (wbx-26): the
    /// unchanged edits `action` answers are the DJ's assertion, told to the
    /// Ledger, no step of the history ([`LibraryCore::asserting_edit`]).
    pub(crate) fn asserting_edit<T>(
        &self,
        touched: Touched,
        label: &'static str,
        action: impl FnOnce(
            &mut rbl_db::write::Writer,
        ) -> Result<(T, Vec<rbl_db::write::TrackEdit>), DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        self.change(|core, host| core.asserting_edit(host, touched, label, action))
    }

    /// [`Self::recorded_edit`] for the DJ's hand set of MyTags (wbx-29):
    /// the assignments `action` left as they were are the DJ's assertion
    /// ([`LibraryCore::asserting_tag_edit`]), no step of the history.
    pub(crate) fn asserting_tag_edit<T>(
        &self,
        touched: Touched,
        tracks: &[String],
        label: &'static str,
        action: impl FnOnce(
            &mut rbl_db::write::Writer,
        ) -> Result<
            (T, LibraryEdit, Vec<rbl_library::ledger::TagAssertion>),
            DbError,
        >,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        self.change(|core, host| core.asserting_tag_edit(host, touched, tracks, label, action))
    }

    /// [`Self::edit`] for an edit that removes rows for good: the edit
    /// history is emptied, as the window empties it after its own.
    pub(crate) fn permanent_edit<T>(
        &self,
        touched: Touched,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        self.change(|core, host| core.permanent_edit(host, touched, action))
    }

    /// Sets one track's rating through the Library's single edit path, and
    /// answers what to tell the caller and what to tell every client.
    ///
    /// The write is the one the window's rating command makes
    /// (`set_rating_with_undo`) and is recorded in the core's edit history
    /// as the window's is, so Undo in the window takes it back.
    pub(crate) fn rate(
        &self,
        edit: &RateTrack,
    ) -> Result<(RatedTrack, LibraryChanged), ServiceError> {
        if edit.rating > MAX_RATING {
            return Err(ServiceError::new(
                codes::INVALID_RATING,
                format!(
                    "{} is not a rating: a rating is 0 to {MAX_RATING} stars",
                    edit.rating
                ),
            ));
        }
        let before = self.index()?;
        let Some(row) = before.row_of(&edit.id) else {
            return Err(ServiceError::new(
                codes::TRACK_NOT_FOUND,
                format!("there is no track {} in the Library", edit.id),
            ));
        };
        let previous = track(&before, row);
        let id = previous.id.clone();
        let stars = edit.rating;
        let touched = Touched::Metadata(vec![id.clone()]);
        let ((), change) = if edit.assert {
            self.asserting_edit(touched, TRACK_EDIT, |writer| {
                let (_, recorded) = writer.set_rating_with_undo(&id, stars)?;
                Ok(((), vec![recorded]))
            })?
        } else {
            self.recorded_edit(touched, TRACK_EDIT, |writer| {
                let (_, recorded) = writer.set_rating_with_undo(&id, stars)?;
                Ok(((), LibraryEdit::Track(vec![recorded])))
            })?
        };
        // Read back from the index the edit refreshed, not assumed.
        let after = self.index()?;
        let rating = after
            .row_of(&id)
            .map_or(stars, |row| track(&after, row).rating);
        Ok((
            RatedTrack {
                id,
                title: previous.title,
                rating,
                previous_rating: previous.rating,
                generation: change.generation,
            },
            change,
        ))
    }
}

impl From<rbl_library::Change> for LibraryChanged {
    fn from(change: rbl_library::Change) -> Self {
        Self {
            kind: match change.kind {
                rbl_library::ChangeKind::Library => ChangeKind::Library,
                rbl_library::ChangeKind::TagList => ChangeKind::TagList,
            },
            generation: change.generation,
        }
    }
}

pub(crate) fn track(index: &rbl_index::Library, row: rbl_index::Row) -> Track {
    let at = row as usize;
    let bpm_x100 = index.bpm_x100.get(at).copied().unwrap_or(0);
    Track {
        id: index.ids.get(at).copied().unwrap_or(0).to_string(),
        title: index.title.get(at).to_owned(),
        artist: index.artist_name(row).to_owned(),
        bpm: (bpm_x100 > 0).then(|| f64::from(bpm_x100) / 100.0),
        key: index.key_name(row).to_owned(),
        rating: index.rating.get(at).copied().unwrap_or(0),
    }
}

/// Why an edit failed. A guard's refusal keeps its own words, which are the
/// ones the window shows.
fn edit_error(error: &DbError) -> ServiceError {
    match error {
        refused if rbl_library::protection::refused(refused) => protection::refusal(),
        // A Library on a protected Stick (wbx-45.11): the Stick's own words.
        DbError::WriteRefused(why) if rbl_library::stick::refused(error) => {
            ServiceError::new(codes::STICK_PROTECTED, why.clone())
        }
        DbError::WriteRefused(why) => ServiceError::new(codes::WRITE_REFUSED, why.clone()),
        // The core has no Library yet: a hosting window is still reading it.
        DbError::NotInstalled(why) if why == rbl_library::NOT_LOADED => {
            ServiceError::new(codes::LIBRARY_LOADING, why.clone())
        }
        other => open_error(other),
    }
}

/// The Library as the shared Stick write planner reads it (wbx-70): the
/// core's read gates (the edit gate and the analysis lock, which the export
/// selection holds while it reads) and a read-only connection. The core is
/// not handed out, and nothing writes through these.
impl wreckordbox_stick_write::selection::LibrarySource for OpenLibrary {
    fn edit_gate(&self) -> &parking_lot::ReentrantMutex<()> {
        self.core.edit_gate()
    }

    fn analysis_write(&self) -> &parking_lot::Mutex<()> {
        self.core.analysis_write()
    }

    fn read_db(
        &self,
        read: &mut dyn FnMut(&rbl_db::Library) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        if let Some(db) = &self.db {
            return read(db);
        }
        read(&self.core.open_read_only()?)
    }
}
