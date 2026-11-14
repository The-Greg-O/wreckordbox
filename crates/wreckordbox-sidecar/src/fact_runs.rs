//! The Run each of some Facts was recorded in (wbx-74), read on the
//! writer's connection or on a read-only snapshot alike: the Review view
//! groups the Facts waiting for the DJ by Run. A Fact's Run never changes,
//! so this read needs nothing else to hold still.
use rusqlite::{Connection, OptionalExtension};

use crate::store::{run_in, Run, RunId, Sidecar};
use crate::{FactsReader, Result};

/// Each of `facts` with the Run it was recorded in; a Fact this Sidecar
/// does not hold is left out.
fn runs_of(db: &Connection, facts: &[i64]) -> Result<Vec<(i64, Run)>> {
    let mut out = Vec::with_capacity(facts.len());
    let mut runs: std::collections::HashMap<i64, Run> = std::collections::HashMap::new();
    let mut statement = db.prepare_cached("SELECT run_id FROM attached_fact WHERE id = ?1")?;
    for fact in facts {
        let Some(run) = statement
            .query_row([fact], |row| row.get::<_, i64>(0))
            .optional()?
        else {
            continue;
        };
        let found = match runs.entry(run) {
            std::collections::hash_map::Entry::Occupied(known) => known.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => match run_in(db, RunId(run))? {
                Some(found) => slot.insert(found),
                None => continue,
            },
        };
        out.push((*fact, found.clone()));
    }
    Ok(out)
}

impl Sidecar {
    /// Each of `facts` with the Run it was recorded in. Reads only.
    pub fn runs_of_facts(&self, facts: &[i64]) -> Result<Vec<(i64, Run)>> {
        runs_of(&self.db(), facts)
    }
}

impl FactsReader {
    /// [`Sidecar::runs_of_facts`] on a read-only snapshot.
    pub fn runs_of_facts(&self, facts: &[i64]) -> Result<Vec<(i64, Run)>> {
        runs_of(self.connection(), facts)
    }
}
