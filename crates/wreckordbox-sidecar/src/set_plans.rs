//! Set plans (wbx-39): an ordered choice of tracks built from a Recipe,
//! with the reason for each transition. Held here, never in the Library.
//!
//! One row per plan built, written once, with the Run that built it (made
//! and finished in the same transaction), the Recipe's text exactly as it
//! was read, the pool asked for and the plan as the sequencer wrote it.
//! The plan is kept as the sequencer's own bytes, so two builds from the
//! same Recipe, pool and Facts hold the same text. Editing a plan (wbx-40)
//! adds beside this; it does not rewrite a row.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::now;
use crate::{FactsReader, Result, RunId, Sidecar, Source};

pub(crate) const SCHEMA: &str = "
CREATE TABLE set_plan (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      INTEGER NOT NULL REFERENCES run (id),
    name        TEXT NOT NULL,
    recipe      TEXT NOT NULL,
    recipe_file TEXT,
    pool        TEXT NOT NULL,
    plan        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
) STRICT;
";

/// A plan to keep.
pub struct NewSetPlan<'a> {
    /// Who built it, and the Run's description.
    pub source: &'a Source,
    pub description: &'a str,
    /// The Recipe's name (or its file's), for listing.
    pub name: &'a str,
    /// The Recipe's text as read.
    pub recipe: &'a str,
    /// Where it was read from, when it was a file.
    pub recipe_file: Option<&'a str>,
    /// The track identifiers asked for, in the order asked.
    pub pool: &'a [String],
    /// The sequencer's plan, serialised.
    pub plan: &'a str,
    /// The Run's outcome sentence.
    pub outcome: &'a str,
}

/// A plan as kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlan {
    pub id: i64,
    pub run: i64,
    pub name: String,
    pub recipe: String,
    pub recipe_file: Option<String>,
    pub pool: Vec<String>,
    pub plan: Value,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
}

impl Sidecar {
    /// Keeps a Set plan and the Run that built it, in one transaction.
    ///
    /// # Errors
    /// Nothing is kept: the store refused the write (a Sidecar on a Stick,
    /// for one) or could not be written.
    pub fn record_set_plan(&self, new: &NewSetPlan<'_>) -> Result<SetPlan> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let at = now();
        tx.execute(
            "INSERT INTO run (source_kind, source_name, description, started_at, finished_at, outcome)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            params![
                new.source.kind.as_str(),
                new.source.name,
                new.description,
                at,
                new.outcome
            ],
        )?;
        let run = RunId(tx.last_insert_rowid());
        let pool = serde_json::to_string(new.pool).unwrap_or_else(|_| "[]".to_owned());
        tx.execute(
            "INSERT INTO set_plan (run_id, name, recipe, recipe_file, pool, plan, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run.0,
                new.name,
                new.recipe,
                new.recipe_file,
                pool,
                new.plan,
                at
            ],
        )?;
        let id = tx.last_insert_rowid();
        let kept = set_plan_in(&tx, id)?;
        tx.commit()?;
        kept.ok_or_else(|| {
            crate::SidecarError::Invalid("the Set plan just kept is not there".to_owned())
        })
    }

    /// One Set plan by its identifier.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn set_plan(&self, id: i64) -> Result<Option<SetPlan>> {
        set_plan_in(&self.db(), id)
    }

    /// Every Set plan, newest first.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn set_plans(&self) -> Result<Vec<SetPlan>> {
        set_plans_in(&self.db())
    }
}

impl FactsReader {
    /// [`Sidecar::set_plan`], through the read-only snapshot.
    ///
    /// # Errors
    /// The snapshot could not be read.
    pub fn set_plan(&self, id: i64) -> Result<Option<SetPlan>> {
        set_plan_in(self.connection(), id)
    }

    /// [`Sidecar::set_plans`], through the read-only snapshot.
    ///
    /// # Errors
    /// The snapshot could not be read.
    pub fn set_plans(&self) -> Result<Vec<SetPlan>> {
        set_plans_in(self.connection())
    }
}

const SELECT: &str =
    "SELECT id, run_id, name, recipe, recipe_file, pool, plan, created_at FROM set_plan";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SetPlan> {
    let pool: String = row.get(5)?;
    let plan: String = row.get(6)?;
    Ok(SetPlan {
        id: row.get(0)?,
        run: row.get(1)?,
        name: row.get(2)?,
        recipe: row.get(3)?,
        recipe_file: row.get(4)?,
        pool: serde_json::from_str(&pool).unwrap_or_default(),
        plan: serde_json::from_str(&plan).unwrap_or(Value::Null),
        created_at: row.get(7)?,
    })
}

fn set_plan_in(db: &Connection, id: i64) -> Result<Option<SetPlan>> {
    Ok(db
        .prepare_cached(&format!("{SELECT} WHERE id = ?1"))?
        .query_row([id], row)
        .optional()?)
}

fn set_plans_in(db: &Connection) -> Result<Vec<SetPlan>> {
    let mut statement = db.prepare_cached(&format!("{SELECT} ORDER BY id DESC"))?;
    let plans = statement
        .query_map([], row)?
        .collect::<rusqlite::Result<Vec<SetPlan>>>()?;
    Ok(plans)
}
