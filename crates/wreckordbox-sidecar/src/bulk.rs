//! The operation's Run is attached to its outstanding intent atomically.
use rusqlite::params;
use crate::{Sidecar, Source, RunId, Result};
impl Sidecar {
    /// Called on the first confirmed batch, not for refused/unchanged work.
    pub fn begin_bulk_run(&self, intent: Option<i64>, source: &Source, description: &str) -> Result<RunId> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO run (source_kind,source_name,description,started_at) VALUES (?1,?2,?3,?4)",
            params![source.kind.as_str(),source.name,description,crate::store::now()])?;
        let run = tx.last_insert_rowid();
        tx.execute("INSERT INTO ledger_owned_run (run_id) VALUES (?1)",[run])?;
        if let Some(intent) = intent {
            tx.execute("UPDATE ledger_intent SET run_id=?1 WHERE id=?2",params![run,intent])?;
        }
        tx.commit()?;
        Ok(RunId(run))
    }
}
