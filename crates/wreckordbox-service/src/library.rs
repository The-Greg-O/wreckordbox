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
use rbl_library::{History, Host, LibraryCore, NoHost, Touched};

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

    /// The Library core, for the service to watch for changes.
    pub(crate) fn core(&self) -> Arc<LibraryCore> {
        Arc::clone(&self.core)
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
        match &self.db {
            Some(db) => read(db),
            None => read(&self.core.open_read_only().map_err(|e| open_error(&e))?),
        }
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

    /// Makes `action`'s writes through the Library's single edit path (the
    /// write guards, the writes, the re-read of what they `touched`), and
    /// answers its result beside what to tell every client. Outside the edit
    /// history, as [`Self::rate`] is.
    pub(crate) fn edit<T>(
        &self,
        touched: Touched,
        action: impl FnOnce(&mut rbl_db::write::Writer) -> Result<T, DbError>,
    ) -> Result<(T, LibraryChanged), ServiceError> {
        let (value, change) = self
            .core
            .edit(&NoHost, History::Outside, touched, action)
            .map_err(|e| edit_error(&e))?;
        let changed = LibraryChanged {
            kind: match change.kind {
                rbl_library::ChangeKind::Library => ChangeKind::Library,
                rbl_library::ChangeKind::TagList => ChangeKind::TagList,
            },
            generation: change.generation,
        };
        Ok((value, changed))
    }

    /// Sets one track's rating through the Library's single edit path, and
    /// answers what to tell the caller and what to tell every client.
    ///
    /// The write is the one the window's rating command makes
    /// (`set_rating_with_undo`), so the edit history can take the entry it
    /// yields once the history lives in the service; until then the edit is
    /// outside the history.
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
        let (_recorded, change) = self
            .core
            .edit(
                &*self.host,
                History::Outside,
                Touched::Metadata(vec![id.clone()]),
                |writer| writer.set_rating_with_undo(&id, stars),
            )
            .map_err(|e| edit_error(&e))?;
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
            change.into(),
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
        DbError::WriteRefused(why) => ServiceError::new(codes::WRITE_REFUSED, why.clone()),
        other => open_error(other),
    }
}
