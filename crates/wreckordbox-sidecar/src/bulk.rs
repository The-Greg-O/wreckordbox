//! The operation's Run is attached to its outstanding intent atomically.
use crate::{Result, RunId, Sidecar, Source};
use rusqlite::params;
impl Sidecar {
    /// Called on the first confirmed batch, not for refused/unchanged work.
    pub fn begin_bulk_run(
        &self,
        intent: Option<i64>,
        source: &Source,
        description: &str,
    ) -> Result<RunId> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO run (source_kind,source_name,description,started_at) VALUES (?1,?2,?3,?4)",
            params![
                source.kind.as_str(),
                source.name,
                description,
                crate::store::now()
            ],
        )?;
        let run = tx.last_insert_rowid();
        tx.execute("INSERT INTO ledger_owned_run (run_id) VALUES (?1)", [run])?;
        if let Some(intent) = intent {
            tx.execute(
                "UPDATE ledger_intent SET run_id=?1 WHERE id=?2",
                params![run, intent],
            )?;
        }
        tx.commit()?;
        Ok(RunId(run))
    }
}

/// Unused sequence slots are discarded if another Ledger/Fact writer intervenes.
/// Thus a bounded reservation transaction never places future bulk writes before
/// a later DJ edit. Gaps are harmless; every assigned id is durable already.
#[derive(Default)]
pub struct OrderLease {
    next: i64,
    end: i64,
}
impl Sidecar {
    pub fn reserve_bulk_ids(&self, lease: &mut OrderLease, count: usize) -> Result<Vec<i64>> {
        use std::sync::atomic::Ordering;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            if lease.next == 0
                || lease.next > lease.end
                || self.order_seen.load(Ordering::Acquire) > lease.end
            {
                let mut db = self.db();
                let tx = db.transaction()?;
                let mut first = 0;
                for index in 0..128 {
                    tx.execute("INSERT INTO ledger_order DEFAULT VALUES", [])?;
                    if index == 0 {
                        first = tx.last_insert_rowid();
                    }
                }
                let end = tx.last_insert_rowid();
                tx.commit()?;
                // Also clears a conservative invalidation left by a rollback.
                self.order_seen.store(end, Ordering::Release);
                lease.next = first;
                lease.end = end;
            }
            ids.push(lease.next);
            lease.next += 1;
        }
        Ok(ids)
    }

    /// One bounded transaction for already ordered entries, without deriving Facts.
    /// Explicit ids make a replay after interruption idempotent.
    pub fn append_bulk(&self, entries: &[crate::ledger::NewEntry<'_>]) -> Result<()> {
        if entries.len() > 128 {
            return Err(crate::SidecarError::Invalid(
                "bulk recording batch exceeds 128 entries".into(),
            ));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        for entry in entries {
            crate::ledger::append_in(&tx, entry)?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ledger::{Action, NewEntry, Store};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[test]
    fn three_hundred_results_use_six_bounded_transactions_and_no_facts() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let source = Source::new(crate::SourceKind::Dj, "dj");
        let run = sidecar.begin_run(&source, "fixture").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        sidecar.db().commit_hook(Some(move || {
            seen.fetch_add(1, Ordering::Relaxed);
            false
        }));
        let ids = sidecar
            .reserve_bulk_ids(&mut OrderLease::default(), 300)
            .unwrap();
        for chunk in ids.chunks(128) {
            let entries: Vec<_> = chunk
                .iter()
                .map(|id| NewEntry {
                    id: Some(*id),
                    run,
                    store: Store::Library,
                    source: &source,
                    at: Some(123),
                    action: Action::Write,
                    kind: "import",
                    label: "fixture",
                    command: None,
                    reverses: None,
                    part_of: None,
                    reversible: false,
                    format: 1,
                    payload: None,
                })
                .collect();
            sidecar.append_bulk(&entries).unwrap();
        }
        assert_eq!(
            count.load(Ordering::Relaxed),
            6,
            "3 reservations + 3 append commits, not 300"
        );
        assert_eq!(sidecar.run_entries(run, 1000, 0).unwrap().1, 300);
        assert_eq!(
            sidecar
                .db()
                .query_row("SELECT COUNT(*) FROM dj_fact", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn a_failed_batch_rolls_back_every_entry_and_replay_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let source = Source::new(crate::SourceKind::Tool, "analysis");
        let run = sidecar.begin_run(&source, "fixture").unwrap();
        let ids = sidecar
            .reserve_bulk_ids(&mut OrderLease::default(), 2)
            .unwrap();
        let make = |id, run| NewEntry {
            id: Some(id),
            run,
            store: Store::Library,
            source: &source,
            at: Some(123),
            action: Action::Write,
            kind: "analysis",
            label: "fixture",
            command: None,
            reverses: None,
            part_of: None,
            reversible: false,
            format: 1,
            payload: None,
        };
        assert!(sidecar
            .append_bulk(&[make(ids[0], run), make(ids[1], RunId(-999))])
            .is_err());
        assert_eq!(sidecar.run_entries(run, 10, 0).unwrap().1, 0);
        let entries = [make(ids[0], run), make(ids[1], run)];
        sidecar.append_bulk(&entries).unwrap();
        sidecar.append_bulk(&entries).unwrap();
        assert_eq!(sidecar.run_entries(run, 10, 0).unwrap().1, 2);
    }
}
