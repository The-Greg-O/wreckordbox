//! The small, current-work reading used by the window's status line.
use serde::{Deserialize, Serialize};

use crate::{Result, Sidecar};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveRun {
    pub id: i64,
    pub description: String,
}

impl Sidecar {
    /// All unfinished Runs, oldest first. Completed work never crowds out
    /// an older Run that is still in progress (there is no history limit).
    pub fn active_runs(&self) -> Result<Vec<ActiveRun>> {
        let db = self.db();
        let mut query =
            db.prepare("SELECT id, description FROM run WHERE finished_at IS NULL ORDER BY id")?;
        let rows = query.query_map([], |row| {
            Ok(ActiveRun {
                id: row.get(0)?,
                description: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
