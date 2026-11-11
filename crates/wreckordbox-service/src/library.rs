//! What the service owns of the Library: the open database and the
//! in-memory index over it.
//!
//! Queries are answered from the index with the same search the window
//! uses, so a client and the window cannot disagree about what matches.
//! An edit belongs here too: write through the database, reload the rows it
//! touched into `index` (`rbl_index::reload_metadata` and its siblings), and
//! only then tell the other clients.

use rbl_db::{DbError, LibraryLocation, OpenMode};
use rbl_index::{SortColumn, TrackFilter, TrackSource, ViewSpec};

use crate::protocol::{ServiceError, Track, TrackList, TrackQuery, DEFAULT_TRACK_LIMIT};
use crate::server::open_error;

pub(crate) struct OpenLibrary {
    db: rbl_db::Library,
    index: rbl_index::Library,
}

impl OpenLibrary {
    /// Opens the Library and indexes it. Reading every row is also what
    /// proves the file reads as a Library.
    pub(crate) fn open(location: LibraryLocation) -> Result<Self, ServiceError> {
        let db = rbl_db::Library::open(location, OpenMode::ReadOnly).map_err(|e| open_error(&e))?;
        let (index, _) = rbl_index::load(&db).map_err(|e| open_error(&DbError::from(e)))?;
        Ok(Self { db, index })
    }

    /// Tracks in the Library, not counting deleted ones.
    pub(crate) fn track_count(&self) -> Result<u32, ServiceError> {
        self.db.live_track_count().map_err(|e| open_error(&e))
    }

    /// The collection in its own order, narrowed as the window's search box
    /// narrows it, then cut to the query's limit.
    pub(crate) fn tracks(&self, query: &TrackQuery) -> TrackList {
        let view = self.index.open_view(&ViewSpec {
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
            .map(|&row| self.track(row))
            .collect();
        TrackList {
            tracks,
            total: view.len(),
        }
    }

    fn track(&self, row: rbl_index::Row) -> Track {
        let index = row as usize;
        let bpm_x100 = self.index.bpm_x100.get(index).copied().unwrap_or(0);
        Track {
            id: self.index.ids.get(index).copied().unwrap_or(0).to_string(),
            title: self.index.title.get(index).to_owned(),
            artist: self.index.artist_name(row).to_owned(),
            bpm: (bpm_x100 > 0).then(|| f64::from(bpm_x100) / 100.0),
            key: self.index.key_name(row).to_owned(),
            rating: self.index.rating.get(index).copied().unwrap_or(0),
        }
    }
}
