//! Facts read without opening the Sidecar's writer (wbx-62).
//!
//! The same contract as [`ActiveRunsReader`](crate::ActiveRunsReader): a
//! query that only shows things (the Intake queue's proposed titles and
//! artists) must never be what creates, migrates or tidies a Sidecar.
//! Opening the writer does all three (it sets WAL, migrates an older
//! schema and clears the work folder), so a reader that finds no writer
//! open reads an immutable, read-only snapshot of the file instead.
//!
//! The caller must keep the writer from opening while it reads (the
//! service holds its Sidecar slot for the whole read). A WAL or rollback
//! journal beside the file means the file alone is not the whole truth,
//! and a schema other than this build's cannot be read with its queries:
//! both answer [`FactsSnapshot::Unknown`] rather than a guess.
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

use crate::store::{read_fact, select, SCHEMA_VERSION};
use crate::{Fact, Result};

/// What a read-only look at a Sidecar's folder found.
pub enum FactsSnapshot {
    /// There is no Sidecar: no Facts at all.
    Absent,
    /// There is one, but it cannot be read safely now; the reason, for the DJ.
    Unknown(String),
    /// A read-only snapshot of it.
    Open(FactsReader),
}

/// A read-only, immutable connection to a Sidecar's database.
pub struct FactsReader {
    db: Connection,
}

impl FactsSnapshot {
    /// Looks at the Sidecar in `dir` without changing anything there.
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("sidecar.db");
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Ok(Self::Unknown(format!(
                    "{} is not a database file",
                    path.display()
                )))
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && path.symlink_metadata().is_err() =>
            {
                return Ok(Self::Absent)
            }
            Err(error) => return Err(crate::io("cannot inspect the Sidecar", &path)(error)),
        }
        for suffix in ["sidecar.db-wal", "sidecar.db-journal"] {
            if path.with_file_name(suffix).exists() {
                return Ok(Self::Unknown(
                    "the Sidecar is being written by another program".to_owned(),
                ));
            }
        }
        // Encode the path as URI bytes, so a name with '#' or '?' is still a path.
        let encoded: String = path
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
        let version: String = db
            .prepare_cached("SELECT value FROM meta WHERE key = 'schema_version'")?
            .query_row([], |row| row.get(0))?;
        if version != SCHEMA_VERSION.to_string() {
            return Ok(Self::Unknown(format!(
                "the Sidecar is at schema version {version}; this build reads {SCHEMA_VERSION} and only the service's own opening of it may change it"
            )));
        }
        Ok(Self::Open(FactsReader { db }))
    }
}

impl FactsReader {
    /// The track's Facts of `kind`, newest first, as [`Sidecar::facts`](crate::Sidecar::facts).
    pub fn facts(&self, track_id: &str, kind: &str) -> Result<Vec<Fact>> {
        let mut statement = self.db.prepare_cached(&select(
            "WHERE fact.effective_track_id = ?1 AND fact.kind = ?2 ORDER BY fact.id DESC",
        ))?;
        let facts = statement
            .query_map(params![track_id, kind], read_fact)?
            .collect::<rusqlite::Result<Vec<Fact>>>()?;
        Ok(facts)
    }
}

impl FactsReader {
    pub(crate) fn connection(&self) -> &Connection {
        &self.db
    }

    /// Read Mutants through this immutable snapshot, without migration or cleanup.
    pub fn mutants(&self) -> crate::mutants::MutantReader<'_> {
        crate::mutants::MutantReader::new(&self.db)
    }
}
