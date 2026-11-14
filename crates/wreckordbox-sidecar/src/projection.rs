//! What Projection (wbx-26) reads from the Sidecar, and the one thing it
//! keeps there. Every read is one query whatever the size of the Library,
//! so a projection never asks the store once per track.
//!
//! - Every Fact of one kind at once ([`Sidecar::facts_of_kind`]).
//! - Who made the latest recorded write to each native field
//!   ([`Provenance::fields`]), read field by field from `track` entries
//!   (every rating, colour and other track edit names its track and
//!   column), following Undo and Redo back to the edit they step: an Undo
//!   restores the state before the edit it takes back, a Redo puts that
//!   edit's owner and value back. A chain that cannot be followed leaves
//!   the field [`Owner::Unknown`].
//! - What the Ledger cannot account for ([`Barrier`]): an intent no entry
//!   settled, an `unconfirmed` entry (a write begun and never confirmed), or
//!   a `track` entry whose payload is missing or unreadable. Each names the
//!   tracks it could concern when its command says (`args.id`,
//!   `args.tracks`); otherwise it could concern any track.
//! - The revision a projection's own write left on a track row ([`Mark`]):
//!   the USN and `updated_at` read back under the edit gate after the
//!   write, and the values written. Marks live in the Sidecar database's
//!   `meta` table, one key per track (`projection_row:<track>`), so they are
//!   backed up and restored with the Ledger, and need no change of schema.
//!   [`Sidecar::set_projection_marks`] is the only write in this module.
use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{read_fact, select};
use crate::{Fact, FactsReader, Result, Sidecar, Source, SourceKind};

/// Who made the write that left a field as it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    /// A `projection.apply` (or a Redo of one).
    Projection,
    /// The DJ.
    Dj,
    /// Anyone else the Ledger names: an agent, a tool, a player.
    Other(Source),
    /// What the field held before any recorded write, back after an Undo.
    Original,
    /// The Ledger cannot say (an Undo of an edit it does not hold).
    Unknown,
}

/// The effective latest recorded write to one field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldState {
    pub owner: Owner,
    /// The value it left, as the Ledger stores values (`null`, a whole
    /// number, text). `null` for [`Owner::Original`] and [`Owner::Unknown`].
    pub value: Value,
    /// The Ledger entry that left it.
    pub entry: i64,
}

/// A write whose provenance is incomplete.
#[derive(Debug, Clone, PartialEq)]
pub struct Barrier {
    /// Its place in the Ledger's order.
    pub seq: i64,
    /// The tracks it could concern; `None` when that cannot be bounded.
    pub tracks: Option<Vec<String>>,
    /// The Ledger entry in which the DJ cleared it, if they did
    /// ([`crate::projection_barriers`]). A cleared barrier no longer stops
    /// an empty field being filled; it makes nothing a projection's.
    pub cleared: Option<i64>,
}

impl Barrier {
    /// Whether it could concern `track`.
    #[must_use]
    pub fn concerns(&self, track: &str) -> bool {
        self.tracks
            .as_ref()
            .is_none_or(|tracks| tracks.iter().any(|t| t == track))
    }
}

/// The revision of a track row as a projection's own write left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// The row's `rb_local_usn`, read back after the write.
    pub usn: i64,
    /// The row's `updated_at`, read back after the write, as stored.
    pub updated_at: String,
    /// The Ledger's order position when it was left: a barrier after it is
    /// newer than the projection.
    pub seq: i64,
    /// The columns as a projection left them (`Rating`, `ColorID`), as whole
    /// numbers.
    pub fields: BTreeMap<String, i64>,
    /// Per column, the Ledger's order position when the projection wrote
    /// THAT field: a barrier after it is newer than the field's ownership.
    /// A column missing here (a mark kept before wbx-xme) is at `seq`. A
    /// field is carried into a later mark only with its own position, and
    /// only while no barrier after it could concern the track, cleared or
    /// not, so filling one field never moves another's position past a
    /// barrier (wbx-xme review).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_seqs: BTreeMap<String, i64>,
}

impl Mark {
    /// The Ledger's order position at which `column`'s value was written.
    #[must_use]
    pub fn seq_of(&self, column: &str) -> i64 {
        self.field_seqs.get(column).copied().unwrap_or(self.seq)
    }
}

/// The provenance of native fields.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// By `(track, column)`: only fields some `track` entry wrote.
    pub fields: HashMap<(String, String), FieldState>,
    /// What the Ledger cannot account for, oldest first.
    pub barriers: Vec<Barrier>,
    /// By track: the revision a projection's own write left.
    pub marks: HashMap<String, Mark>,
    /// The highest Ledger entry read: anything after it is newer than this
    /// provenance.
    pub watermark: i64,
}

/// The service command whose edits are a projection's.
const PROJECTION_COMMAND: &str = "projection.apply";
/// The `meta` key of a track's [`Mark`], before the track's identifier.
const MARK_KEY: &str = "projection_row:";
/// The first key after every mark key (`:` + 1 is `;`).
const MARK_END: &str = "projection_row;";

fn facts_of_kind(db: &Connection, kind: &str) -> Result<Vec<Fact>> {
    let mut statement = db.prepare_cached(&select("WHERE fact.kind = ?1 ORDER BY fact.id DESC"))?;
    let facts = statement
        .query_map(params![kind], read_fact)?
        .collect::<rusqlite::Result<Vec<Fact>>>()?;
    Ok(facts)
}

/// The states each step of one field replaced, by the edit it steps.
type Steps = Vec<(i64, Option<FieldState>)>;

struct Row {
    id: i64,
    action: String,
    source: Source,
    command: Option<String>,
    reverses: Option<i64>,
    payload: Option<String>,
}

/// The tracks a command's JSON (`{"name","args"}`) names, if it names any.
fn tracks_named(command: Option<&str>) -> Option<Vec<String>> {
    let command: Value = serde_json::from_str(command?).ok()?;
    let args = &command["args"];
    let mut tracks: Vec<String> = Vec::new();
    if let Some(id) = args["id"].as_str() {
        tracks.push(id.to_owned());
    }
    for key in ["tracks", "ids"] {
        if let Some(list) = args[key].as_array() {
            tracks.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    (!tracks.is_empty()).then_some(tracks)
}

/// The texts listed under `key` in the JSON `text` (or, with `field`, that
/// field of each object listed); `None` when there are none.
fn listed(text: Option<&str>, key: &str, field: Option<&str>) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(text?).ok()?;
    let mut found: Vec<String> = value[key]
        .as_array()?
        .iter()
        .filter_map(|item| match field {
            Some(field) => item[field].as_str(),
            None => item.as_str(),
        })
        .map(str::to_owned)
        .collect();
    found.sort_unstable();
    found.dedup();
    (!found.is_empty()).then_some(found)
}

/// Whether `payload` is a `track` payload whose edits can be read.
fn edits_of(payload: Option<&str>) -> Option<Vec<Value>> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    payload["edits"].as_array().cloned()
}

pub(crate) fn barriers(db: &Connection) -> Result<Vec<Barrier>> {
    let mut found = Vec::new();
    let mut statement = db.prepare_cached(
        "SELECT id, kind, payload, command FROM ledger
         WHERE store = 'library' AND kind IN ('unconfirmed', 'track') ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (seq, kind, payload, command) in rows {
        if kind == "unconfirmed" {
            // Its command's tracks, else those its recovery evidence named.
            let recovered = || listed(payload.as_deref(), "tracks", None);
            found.push(Barrier {
                seq,
                tracks: tracks_named(command.as_deref()).or_else(recovered),
                cleared: None,
            });
        } else if edits_of(payload.as_deref()).is_none() {
            found.push(Barrier {
                seq,
                tracks: tracks_named(command.as_deref()),
                cleared: None,
            });
        }
    }
    let mut open = db.prepare_cached(
        "SELECT id, command, (SELECT snapshot FROM dj_intent WHERE dj_intent.id = ledger_intent.id)
         FROM ledger_intent ORDER BY id",
    )?;
    let intents = open
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (seq, command, snapshot) in intents {
        // Its command's tracks, else those its bounded evidence names.
        let evidence = || listed(snapshot.as_deref(), "values", Some("track"));
        found.push(Barrier {
            seq,
            tracks: tracks_named(command.as_deref()).or_else(evidence),
            cleared: None,
        });
    }
    let cleared = crate::projection_barriers::cleared(db)?;
    for barrier in &mut found {
        barrier.cleared = cleared.get(&barrier.seq).copied();
    }
    Ok(found)
}

fn marks(db: &Connection) -> Result<HashMap<String, Mark>> {
    let mut statement =
        db.prepare_cached("SELECT key, value FROM meta WHERE key >= ?1 AND key < ?2")?;
    let rows = statement
        .query_map(params![MARK_KEY, MARK_END], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // A mark that does not read is no mark: the field is then the DJ's.
    Ok(rows
        .into_iter()
        .filter_map(|(key, value)| {
            let track = key.strip_prefix(MARK_KEY)?.to_owned();
            serde_json::from_str(&value).ok().map(|mark| (track, mark))
        })
        .collect())
}

fn provenance(db: &Connection, columns: &[&str]) -> Result<Provenance> {
    let watermark: i64 = db.query_row("SELECT COALESCE(MAX(id), 0) FROM ledger", [], |row| {
        row.get(0)
    })?;
    let mut statement = db.prepare_cached(
        "SELECT id, action, source_kind, source_name,
                json_extract(command, '$.name'), reverses, payload
         FROM ledger WHERE store = 'library' AND kind = 'track' ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                action: row.get(1)?,
                source: Source {
                    kind: SourceKind::from_stored(&row.get::<_, String>(2)?),
                    name: row.get(3)?,
                },
                command: row.get(4)?,
                reverses: row.get(5)?,
                payload: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<Row>>>()?;
    let mut provenance = Provenance {
        watermark,
        ..Provenance::default()
    };
    // id -> (action, reverses, owner when it is an edit)
    let mut steps: HashMap<i64, (String, Option<i64>, Owner)> = HashMap::new();
    // Per field: the states each step replaced, to go back to on Undo.
    let mut stacks: HashMap<(String, String), Steps> = HashMap::new();
    for row in rows {
        let owner = if row.command.as_deref() == Some(PROJECTION_COMMAND) {
            Owner::Projection
        } else if row.source.kind == SourceKind::Dj {
            Owner::Dj
        } else {
            Owner::Other(row.source.clone())
        };
        steps.insert(row.id, (row.action.clone(), row.reverses, owner.clone()));
        // The edit a step steps, through Undo and Redo, and its owner.
        let origin = {
            let mut at = row.id;
            let mut hops = 0;
            loop {
                match steps.get(&at) {
                    Some((action, Some(reverses), _)) if action != "edit" && hops < 100_000 => {
                        at = *reverses;
                        hops += 1;
                    }
                    Some((action, _, owner)) if action == "edit" => {
                        break Some((at, owner.clone()))
                    }
                    _ => break None,
                }
            }
        };
        // A payload that does not read is a barrier (`barriers`).
        let Some(edits) = edits_of(row.payload.as_deref()) else {
            continue;
        };
        for edit in edits {
            let (Some(track), Some(column)) = (edit["track"].as_str(), edit["column"].as_str())
            else {
                continue;
            };
            if !columns.contains(&column) {
                continue;
            }
            let key = (track.to_owned(), column.to_owned());
            let current = provenance.fields.get(&key).cloned();
            let stack = stacks.entry(key.clone()).or_default();
            let unknown = FieldState {
                owner: Owner::Unknown,
                value: Value::Null,
                entry: row.id,
            };
            let next = match (row.action.as_str(), &origin) {
                ("undo", Some((origin, _))) => {
                    match stack.iter().rposition(|(step, _)| step == origin) {
                        Some(at) => {
                            let before = stack[at].1.clone();
                            stack.truncate(at);
                            before.map_or(
                                FieldState {
                                    owner: Owner::Original,
                                    value: Value::Null,
                                    entry: row.id,
                                },
                                |state| FieldState {
                                    entry: row.id,
                                    ..state
                                },
                            )
                        }
                        None => unknown,
                    }
                }
                ("edit" | "redo", Some((origin, owner))) => {
                    stack.push((*origin, current));
                    FieldState {
                        owner: owner.clone(),
                        value: edit["after"].clone(),
                        entry: row.id,
                    }
                }
                _ => unknown,
            };
            provenance.fields.insert(key, next);
        }
    }
    provenance.barriers = barriers(db)?;
    provenance.marks = marks(db)?;
    Ok(provenance)
}

/// Whether an entry newer than `watermark` could concern one of `tracks`.
fn written_since(db: &Connection, watermark: i64, tracks: &[String]) -> Result<bool> {
    let mut statement = db.prepare_cached(
        "SELECT kind, payload, command FROM ledger
         WHERE id > ?1 AND store = 'library' AND kind IN ('track', 'unconfirmed')",
    )?;
    let rows = statement
        .query_map(params![watermark], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let named = |track: &str| tracks.iter().any(|t| t == track);
    Ok(rows.into_iter().any(|(kind, payload, command)| {
        match (kind.as_str(), edits_of(payload.as_deref())) {
            ("track", Some(edits)) => edits
                .iter()
                .any(|edit| edit["track"].as_str().is_some_and(named)),
            // Unconfirmed, or unreadable: newer than the plan and possibly
            // about these tracks.
            _ => {
                tracks_named(command.as_deref()).is_none_or(|found| found.iter().any(|t| named(t)))
            }
        }
    }))
}

impl Sidecar {
    /// Every Fact of `kind`, of every track, newest first: archived and
    /// stale ones included, for the caller to rank.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(&self.db(), kind)
    }

    /// The provenance of each of `columns`, per track ([`Provenance`]).
    pub fn provenance(&self, columns: &[&str]) -> Result<Provenance> {
        provenance(&self.db(), columns)
    }

    /// Whether an entry newer than `watermark` could concern one of
    /// `tracks`: a `track` entry naming one, or an unconfirmed or
    /// unreadable entry that names one or cannot say.
    pub fn written_since(&self, watermark: i64, tracks: &[String]) -> Result<bool> {
        written_since(&self.db(), watermark, tracks)
    }

    /// The Ledger's order position now: every entry and intent so far is at
    /// or before it.
    pub fn ledger_position(&self) -> Result<i64> {
        Ok(self.db().query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM ledger_order",
            [],
            |row| row.get(0),
        )?)
    }

    /// Keeps `marks`, each replacing the track's earlier one, together.
    pub fn set_projection_marks(&self, marks: &[(String, Mark)]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut put = tx.prepare_cached(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?;
            for (track, mark) in marks {
                let value = serde_json::to_string(mark).unwrap_or_default();
                put.execute(params![format!("{MARK_KEY}{track}"), value])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl FactsReader {
    /// [`Sidecar::facts_of_kind`] on a read-only snapshot.
    pub fn facts_of_kind(&self, kind: &str) -> Result<Vec<Fact>> {
        facts_of_kind(self.connection(), kind)
    }

    /// [`Sidecar::provenance`] on a read-only snapshot.
    pub fn provenance(&self, columns: &[&str]) -> Result<Provenance> {
        provenance(self.connection(), columns)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ledger::{Action, NewEntry, Store};

    fn entry<'a>(
        run: crate::RunId,
        source: &'a Source,
        kind: &'a str,
        command: Option<&'a Value>,
        payload: Option<&'a Value>,
    ) -> NewEntry<'a> {
        NewEntry {
            id: None,
            run,
            store: Store::Library,
            source,
            at: None,
            action: Action::Edit,
            kind,
            label: "test",
            command,
            reverses: None,
            part_of: None,
            reversible: payload.is_some(),
            format: 1,
            payload,
        }
    }

    /// Missing or unreadable payloads and unconfirmed writes are barriers,
    /// bounded to the tracks their command names, or to none.
    #[test]
    fn incomplete_provenance_is_a_barrier_bounded_by_what_its_command_names() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let dj = Source::new(SourceKind::Dj, "dj");
        let run = sidecar.begin_run(&dj, "test").unwrap();
        let named =
            serde_json::json!({"name": "tracks.rate", "args": {"id": "10001", "rating": 3}});
        let readable = serde_json::json!({"edits": [{"track": "10002", "column": "Rating", "before": 0, "after": 3}]});
        let unreadable = serde_json::json!({"no": "edits"});
        sidecar
            .append(&entry(run, &dj, "track", None, Some(&readable)))
            .unwrap();
        sidecar
            .append(&entry(run, &dj, "track", Some(&named), None))
            .unwrap();
        sidecar
            .append(&entry(run, &dj, "unconfirmed", Some(&named), None))
            .unwrap();
        sidecar
            .append(&entry(run, &dj, "track", None, Some(&unreadable)))
            .unwrap();
        let found = sidecar.provenance(&["Rating"]).unwrap();
        let bounded: Vec<Option<Vec<String>>> =
            found.barriers.iter().map(|b| b.tracks.clone()).collect();
        assert_eq!(
            bounded,
            vec![
                Some(vec!["10001".to_owned()]),
                Some(vec!["10001".to_owned()]),
                None
            ]
        );
        assert!(found.barriers[0].concerns("10001") && !found.barriers[0].concerns("10002"));
        assert!(
            found.barriers[2].concerns("10002"),
            "unbounded concerns every track"
        );
        // The readable entry still gives its field an owner.
        assert_eq!(
            found.fields[&("10002".to_owned(), "Rating".to_owned())].owner,
            Owner::Dj
        );
    }

    #[test]
    fn marks_are_kept_and_read_back_per_track() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let mark = Mark {
            usn: 7,
            updated_at: "2026-11-13 10:00:00.000 +00:00".into(),
            seq: 3,
            fields: BTreeMap::from([("Rating".to_owned(), 4)]),
            field_seqs: BTreeMap::new(),
        };
        sidecar
            .set_projection_marks(&[("10001".into(), mark.clone())])
            .unwrap();
        let found = sidecar.provenance(&["Rating"]).unwrap();
        assert_eq!(found.marks.get("10001"), Some(&mark));
        assert_eq!(found.marks.len(), 1);
    }
}
