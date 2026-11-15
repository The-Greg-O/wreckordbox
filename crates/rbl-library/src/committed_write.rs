//! SQLite's `total_changes` includes rolled-back statements. A separate read-only
//! connection's `data_version` changes only when a transaction becomes visible.
//! The core's existing edit gate covers this observation and the entire write.
use rbl_db::{DbError, Library, LibraryLocation, OpenMode};
pub(crate) struct Observer {
    db: Library,
    before: i64,
}
impl Observer {
    pub(crate) fn open(location: LibraryLocation) -> Result<Self, DbError> {
        let db = Library::open(location, OpenMode::ReadOnly)?;
        let before = version(&db)?;
        Ok(Self { db, before })
    }
    pub(crate) fn changed(&self) -> Result<bool, DbError> {
        Ok(version(&self.db)? != self.before)
    }
}
fn version(db: &Library) -> Result<i64, DbError> {
    Ok(db
        .connection()
        .query_row("PRAGMA main.data_version", [], |r| r.get(0))?)
}
