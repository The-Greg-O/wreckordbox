//! What MyTag Projection (wbx-29) reads from the Sidecar, and the one thing
//! it keeps there: beside [`crate::projection`] (Energy and Prep colour),
//! for MyTag **assignments**, one track and one MyTag at a time.
//!
//! - Who made the latest recorded write to each assignment
//!   ([`TagProvenance::assignments`]), read from the Ledger's `track_tags`
//!   entries (every MyTag edit that is a history step: the window's
//!   toggles, `mytags assign`/`unassign`, the tag panel, a projection),
//!   following Undo and Redo back to the edit they step, as for a field. An
//!   entry writes the assignment `(track, tag)` when the tag is in exactly
//!   one of its `before` and `after` sets for that track; its value is
//!   whether the tag is on afterwards.
//! - What the Ledger cannot account for: every [`Barrier`] Energy's
//!   projection counts, and a `track_tags` entry whose payload is missing or
//!   unreadable.
//! - The rows a projection's own write left for an assignment
//!   ([`TagMark`]): every `djmdSongMyTag` row of that track and tag, live or
//!   soft-deleted, with its `rb_local_usn` and `updated_at`, read back in
//!   the transaction that wrote them. A later write of any kind to that
//!   assignment (the DJ in the window or rekordbox, a merge, a player)
//!   leaves other rows, so no clock is ever compared. Kept in the `meta`
//!   table, one key per assignment (`projection_tag:<track>:<tag>`);
//!   [`Sidecar::set_tag_marks`] is the only write in this module.
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]
use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projection::{Barrier, FieldState, Owner};
use crate::{FactsReader, Result, Sidecar, Source, SourceKind};

/// The service command whose MyTag edits are a projection's.
pub const PROJECTION_COMMAND: &str = "projection.mytags.apply";
/// The Ledger kind of the DJ's explicit assertion of MyTag assignments
/// (wbx-29 review): the DJ setting each to the state it already had.
/// Payload `{"assertions": [{"track", "tag", "on"}]}`; no step of Undo.
pub const ASSERTION_KIND: &str = "my_tag_assertion";
/// The `meta` key of an assignment's [`TagMark`], before `<track>:<tag>`.
const MARK_KEY: &str = "projection_tag:";
/// The first key after every mark key (`:` + 1 is `;`).
const MARK_END: &str = "projection_tag;";

/// One `djmdSongMyTag` row of an assignment, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRow {
    pub id: String,
    pub usn: i64,
    pub updated_at: String,
    /// Not soft-deleted.
    pub live: bool,
}

/// The rows a projection's own write left for one assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagMark {
    /// Every row of the assignment, by id, read back in the write's own
    /// transaction.
    pub rows: Vec<TagRow>,
    /// Whether the projection left the tag on.
    pub on: bool,
    /// The Ledger entry of the write that left these rows (its intent's
    /// identifier, which the entry takes when it is recorded). The mark is
    /// evidence only while that entry, confirmed, is the assignment's latest
    /// recorded write: a write cut off before its entry, or one whose entry
    /// waits, leaves a mark that proves nothing (wbx-29 review). 0 for none.
    #[serde(default)]
    pub entry: i64,
    /// The Ledger's order position when it was left: a barrier after it is
    /// newer than the projection's write.
    pub seq: i64,
}

/// The provenance of MyTag assignments.
#[derive(Debug, Clone, Default)]
pub struct TagProvenance {
    /// By `(track, tag)`: only assignments some `track_tags` entry wrote.
    /// The value is `true` or `false` (on or off); `null` for
    /// [`Owner::Original`] and [`Owner::Unknown`].
    pub assignments: HashMap<(String, String), FieldState>,
    /// What the Ledger cannot account for, oldest first.
    pub barriers: Vec<Barrier>,
    /// By `(track, tag)`: the rows a projection's own write left.
    pub marks: HashMap<(String, String), TagMark>,
    /// The highest Ledger entry read.
    pub watermark: i64,
}

/// The tracks a command's JSON names (`args.id`, `args.tracks`,
/// `args.ids`), if it names any.
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

/// One track's MyTags before and after an edit: `(track, before, after)`.
type TagEdit = (String, Vec<String>, Vec<String>);

/// Per assignment: the states each step replaced, to go back to on Undo.
type Stacks = HashMap<(String, String), Vec<(i64, Option<FieldState>)>>;

/// What an entry of `kind` wrote, as `(track, tag, on)`: a `track_tags`
/// edit writes each tag in exactly one of its `before` and `after`; an
/// assertion writes each it names. `None` when the payload does not read.
pub(crate) fn writes(kind: &str, payload: Option<&str>) -> Option<Vec<(String, String, bool)>> {
    if kind == ASSERTION_KIND {
        let payload: Value = serde_json::from_str(payload?).ok()?;
        return payload["assertions"]
            .as_array()?
            .iter()
            .map(|a| {
                Some((
                    a["track"].as_str()?.to_owned(),
                    a["tag"].as_str()?.to_owned(),
                    a["on"].as_bool()?,
                ))
            })
            .collect();
    }
    let mut found = Vec::new();
    for (track, before, after) in tag_edits(payload)? {
        for tag in before.iter().filter(|tag| !after.contains(tag)) {
            found.push((track.clone(), tag.clone(), false));
        }
        for tag in after.iter().filter(|tag| !before.contains(tag)) {
            found.push((track.clone(), tag.clone(), true));
        }
    }
    Some(found)
}

/// The tracks a MyTag entry names, when its payload reads.
pub(crate) fn tracks_of(kind: &str, payload: Option<&str>) -> Option<Vec<String>> {
    if kind == ASSERTION_KIND {
        return writes(kind, payload).map(|w| w.into_iter().map(|(track, _, _)| track).collect());
    }
    tag_edits(payload).map(|edits| edits.into_iter().map(|(track, _, _)| track).collect())
}

/// `(track, before, after)` of each track a `track_tags` payload names;
/// `None` when it does not read.
fn tag_edits(payload: Option<&str>) -> Option<Vec<TagEdit>> {
    let payload: Value = serde_json::from_str(payload?).ok()?;
    let ids = |list: &Value| -> Option<Vec<String>> {
        list.as_array()?
            .iter()
            .map(|id| id.as_str().map(str::to_owned))
            .collect()
    };
    payload["tags"]
        .as_array()?
        .iter()
        .map(|edit| {
            Some((
                edit["track"].as_str()?.to_owned(),
                ids(&edit["before"])?,
                ids(&edit["after"])?,
            ))
        })
        .collect()
}

struct Row {
    id: i64,
    kind: String,
    action: String,
    source: Source,
    command: Option<String>,
    reverses: Option<i64>,
    payload: Option<String>,
}

fn rows(db: &Connection) -> Result<Vec<Row>> {
    let mut statement = db.prepare_cached(
        "SELECT id, action, source_kind, source_name, json_extract(command, '$.name'),
                reverses, payload, kind
         FROM ledger WHERE store = 'library' AND kind IN ('track_tags', ?1) ORDER BY id",
    )?;
    let rows = statement
        .query_map([ASSERTION_KIND], |row| {
            Ok(Row {
                id: row.get(0)?,
                kind: row.get(7)?,
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
    Ok(rows)
}

fn marks(db: &Connection) -> Result<HashMap<(String, String), TagMark>> {
    let mut statement =
        db.prepare_cached("SELECT key, value FROM meta WHERE key >= ?1 AND key < ?2")?;
    let rows = statement
        .query_map(params![MARK_KEY, MARK_END], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // A mark that does not read is no mark: the assignment is then the DJ's.
    Ok(rows
        .into_iter()
        .filter_map(|(key, value)| {
            let (track, tag) = key.strip_prefix(MARK_KEY)?.split_once(':')?;
            let mark = serde_json::from_str(&value).ok()?;
            Some(((track.to_owned(), tag.to_owned()), mark))
        })
        .collect())
}

fn provenance(db: &Connection) -> Result<TagProvenance> {
    let watermark: i64 = db.query_row("SELECT COALESCE(MAX(id), 0) FROM ledger", [], |row| {
        row.get(0)
    })?;
    let mut provenance = TagProvenance {
        watermark,
        ..TagProvenance::default()
    };
    // id -> (action, reverses, owner when it is an edit)
    let mut steps: HashMap<i64, (String, Option<i64>, Owner)> = HashMap::new();
    // Per assignment: the states each step replaced, to go back to on Undo.
    let mut stacks: Stacks = HashMap::new();
    for row in rows(db)? {
        let owner = if row.command.as_deref() == Some(PROJECTION_COMMAND) {
            Owner::Projection
        } else if row.source.kind == SourceKind::Dj {
            Owner::Dj
        } else {
            Owner::Other(row.source.clone())
        };
        steps.insert(row.id, (row.action.clone(), row.reverses, owner));
        // The edit a step steps, through any number of Undo and Redo.
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
        // A payload that does not read is a barrier (`projection::barriers`).
        let Some(written) = writes(&row.kind, row.payload.as_deref()) else {
            continue;
        };
        {
            for (track, tag, on) in written {
                let key = (track.clone(), tag.clone());
                let current = provenance.assignments.get(&key).cloned();
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
                                let replaced = stack[at].1.clone();
                                stack.truncate(at);
                                replaced.map_or(
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
                            value: Value::Bool(on),
                            entry: row.id,
                        }
                    }
                    _ => unknown,
                };
                provenance.assignments.insert(key, next);
            }
        }
    }
    provenance.barriers = crate::projection::barriers(db)?;
    provenance.marks = marks(db)?;
    Ok(provenance)
}

/// Whether a MyTag entry newer than `watermark` could concern one of
/// `tracks`: a `track_tags` entry naming one, or an unconfirmed or
/// unreadable entry that names one or cannot say.
fn written_since(db: &Connection, watermark: i64, tracks: &[String]) -> Result<bool> {
    let mut statement = db.prepare_cached(
        "SELECT kind, payload, command FROM ledger
         WHERE id > ?1 AND store = 'library' AND kind IN ('track_tags', ?2, 'unconfirmed')",
    )?;
    let rows = statement
        .query_map(params![watermark, ASSERTION_KIND], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let named = |track: &str| tracks.iter().any(|t| t == track);
    Ok(rows.into_iter().any(|(kind, payload, command)| {
        let read = (kind != "unconfirmed")
            .then(|| tracks_of(&kind, payload.as_deref()))
            .flatten();
        match read {
            Some(found) => found.iter().any(|track| named(track)),
            _ => {
                tracks_named(command.as_deref()).is_none_or(|found| found.iter().any(|t| named(t)))
            }
        }
    }))
}

impl Sidecar {
    /// The provenance of every MyTag assignment ([`TagProvenance`]).
    pub fn tag_provenance(&self) -> Result<TagProvenance> {
        provenance(&self.db())
    }

    /// Whether a MyTag entry newer than `watermark` could concern one of
    /// `tracks`.
    pub fn tags_written_since(&self, watermark: i64, tracks: &[String]) -> Result<bool> {
        written_since(&self.db(), watermark, tracks)
    }

    /// Keeps `marks`, by `(track, tag)`, each replacing the assignment's
    /// earlier one, together.
    pub fn set_tag_marks(&self, marks: &[((String, String), TagMark)]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut put = tx.prepare_cached(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            )?;
            for ((track, tag), mark) in marks {
                let value = serde_json::to_string(mark).unwrap_or_default();
                put.execute(params![format!("{MARK_KEY}{track}:{tag}"), value])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl FactsReader {
    /// [`Sidecar::tag_provenance`] on a read-only snapshot.
    pub fn tag_provenance(&self) -> Result<TagProvenance> {
        provenance(self.connection())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ledger::{Action, NewEntry, Store};

    fn append(
        sidecar: &Sidecar,
        source: &Source,
        action: Action,
        command: Option<&Value>,
        reverses: Option<i64>,
        payload: Option<&Value>,
    ) -> i64 {
        let run = sidecar.begin_run(source, "test").unwrap();
        sidecar
            .append(&NewEntry {
                id: None,
                run,
                store: Store::Library,
                source,
                at: None,
                action,
                kind: "track_tags",
                label: "test",
                command,
                reverses,
                part_of: None,
                reversible: payload.is_some(),
                format: 1,
                payload,
            })
            .unwrap()
    }

    /// Each assignment's owner is the latest recorded write to it; Undo of a
    /// projection's write gives the assignment back to what it was before.
    #[test]
    fn each_assignment_is_owned_by_its_latest_recorded_write_through_undo() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let dj = Source::new(SourceKind::Dj, "dj");
        let tool = Source::new(SourceKind::Tool, "projection");
        let projected = serde_json::json!({"name": PROJECTION_COMMAND, "args": {}});
        let put_on =
            serde_json::json!({"tags": [{"track": "1", "before": ["a"], "after": ["a", "b"]}]});
        let edit = append(
            &sidecar,
            &tool,
            Action::Edit,
            Some(&projected),
            None,
            Some(&put_on),
        );
        let dj_off = serde_json::json!({"tags": [{"track": "2", "before": ["a"], "after": []}]});
        append(&sidecar, &dj, Action::Edit, None, None, Some(&dj_off));
        let found = sidecar.tag_provenance().unwrap();
        let state = |track: &str, tag: &str, found: &TagProvenance| {
            found
                .assignments
                .get(&(track.to_owned(), tag.to_owned()))
                .cloned()
        };
        let b = state("1", "b", &found).unwrap();
        assert_eq!((b.owner, b.value), (Owner::Projection, Value::Bool(true)));
        assert!(state("1", "a", &found).is_none(), "unchanged: not written");
        let off = state("2", "a", &found).unwrap();
        assert_eq!((off.owner, off.value), (Owner::Dj, Value::Bool(false)));

        append(&sidecar, &dj, Action::Undo, None, Some(edit), Some(&put_on));
        let undone = state("1", "b", &sidecar.tag_provenance().unwrap()).unwrap();
        assert_eq!(undone.owner, Owner::Original);
    }

    /// The DJ's assertion of an assignment as it is (on or off) makes it the
    /// DJ's, though no membership changed.
    #[test]
    fn an_assertion_makes_the_assignment_the_djs_on_or_off() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let dj = Source::new(SourceKind::Dj, "dj");
        let run = sidecar.begin_run(&dj, "test").unwrap();
        let payload = serde_json::json!({"assertions": [
            {"track": "1", "tag": "a", "on": true},
            {"track": "1", "tag": "b", "on": false},
        ]});
        sidecar
            .append(&NewEntry {
                id: None,
                run,
                store: Store::Library,
                source: &dj,
                at: None,
                action: Action::Edit,
                kind: ASSERTION_KIND,
                label: "Track Edit",
                command: None,
                reverses: None,
                part_of: None,
                reversible: false,
                format: 1,
                payload: Some(&payload),
            })
            .unwrap();
        let found = sidecar.tag_provenance().unwrap();
        for (tag, on) in [("a", true), ("b", false)] {
            let state = &found.assignments[&("1".to_owned(), tag.to_owned())];
            assert_eq!((&state.owner, &state.value), (&Owner::Dj, &Value::Bool(on)));
        }
        assert_eq!(found.barriers, Vec::new());
        assert!(sidecar.tags_written_since(0, &["1".into()]).unwrap());
    }

    #[test]
    fn an_unreadable_mytag_entry_is_a_barrier_and_marks_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = Sidecar::open(dir.path()).unwrap();
        let dj = Source::new(SourceKind::Dj, "dj");
        let named =
            serde_json::json!({"name": "mytags.assign", "args": {"id": "9", "tracks": ["3"]}});
        let broken = serde_json::json!({"tags": "?"});
        append(
            &sidecar,
            &dj,
            Action::Edit,
            Some(&named),
            None,
            Some(&broken),
        );
        let mark = TagMark {
            rows: vec![TagRow {
                id: "r".into(),
                usn: 4,
                updated_at: "t".into(),
                live: true,
            }],
            on: true,
            seq: 2,
            entry: 1,
        };
        sidecar
            .set_tag_marks(&[(("3".into(), "9".into()), mark.clone())])
            .unwrap();
        let found = sidecar.tag_provenance().unwrap();
        assert_eq!(
            found
                .barriers
                .iter()
                .map(|b| b.tracks.clone())
                .collect::<Vec<_>>(),
            vec![Some(vec!["9".to_owned(), "3".to_owned()])]
        );
        assert_eq!(found.marks.get(&("3".into(), "9".into())), Some(&mark));
        assert!(sidecar.tags_written_since(0, &["3".into()]).unwrap());
        assert!(!sidecar
            .tags_written_since(found.watermark, &["3".into()])
            .unwrap());
    }
}
