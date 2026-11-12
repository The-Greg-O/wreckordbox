//! Cheap, read-only current work. Status never opens the Sidecar writer.
use std::path::PathBuf;
use std::sync::TryLockError;
use std::time::SystemTime;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{Result, Sidecar, SidecarError};

pub(crate) const SCHEMA: &str =
    "CREATE INDEX run_unfinished ON run (id) WHERE finished_at IS NULL;";
const QUERY: &str =
    "SELECT id, description FROM run INDEXED BY run_unfinished WHERE finished_at IS NULL ORDER BY id";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveRun {
    pub id: i64,
    pub description: String,
}

fn query(db: &Connection) -> Result<Vec<ActiveRun>> {
    let mut statement = db.prepare_cached(QUERY)?;
    let rows = statement.query_map([], |row| {
        Ok(ActiveRun {
            id: row.get(0)?,
            description: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

impl Sidecar {
    /// Read through an already-open writer, without waiting behind an edit.
    pub fn active_runs(&self) -> Result<Vec<ActiveRun>> {
        let db = match self.db.try_lock() {
            Ok(db) => db,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                )
                .into())
            }
        };
        // The writer normally waits for up to five seconds. Status must not
        // hold its connection (or the host) waiting for another process.
        db.busy_timeout(std::time::Duration::ZERO)?;
        let result = query(&db);
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        result
    }
}

/// A cached read-only snapshot when the service has not opened its writer.
/// The caller must prevent that writer from opening during `read`. An existing
/// WAL/journal means a snapshot cannot safely ignore it: answer unknown instead.
/// Immutable mode avoids SQLite creating WAL/SHM files just to read a closed DB.
/// Neither this reader nor the status path ever migrates, creates or tidies it.
pub struct ActiveRunsReader {
    path: PathBuf,
    snapshot: Option<(SystemTime, u64, Connection)>,
}

impl ActiveRunsReader {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            path: dir.join("sidecar.db"),
            snapshot: None,
        }
    }

    pub fn read(&mut self) -> Result<Option<Vec<ActiveRun>>> {
        if !self.path.exists() {
            self.snapshot = None;
            return Ok(Some(Vec::new()));
        }
        for suffix in ["sidecar.db-wal", "sidecar.db-journal"] {
            if self.path.with_file_name(suffix).exists() {
                self.snapshot = None;
                return Ok(None);
            }
        }
        let meta = std::fs::metadata(&self.path).map_err(|source| SidecarError::Io {
            what: "could not inspect",
            path: self.path.clone(),
            source,
        })?;
        let modified = meta.modified().map_err(|source| SidecarError::Io {
            what: "could not inspect",
            path: self.path.clone(),
            source,
        })?;
        if self
            .snapshot
            .as_ref()
            .is_none_or(|(time, len, _)| *time != modified || *len != meta.len())
        {
            // Encode arbitrary filenames, including '#' and '?', as URI path bytes.
            let encoded: String = self
                .path
                .as_os_str()
                .as_encoded_bytes()
                .iter()
                .map(|byte| {
                    if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) {
                        char::from(*byte).to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect();
            let db = Connection::open_with_flags(
                format!("file:{encoded}?immutable=1"),
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            db.busy_timeout(std::time::Duration::ZERO)?;
            self.snapshot = Some((modified, meta.len(), db));
        }
        let Some((_, _, db)) = &self.snapshot else {
            return Ok(None);
        };
        let version: String = db
            .prepare_cached("SELECT value FROM meta WHERE key = 'schema_version'")?
            .query_row([], |row| row.get(0))?;
        if version != crate::store::SCHEMA_VERSION.to_string() {
            return Ok(None);
        }
        query(db).map(Some)
    }
}
