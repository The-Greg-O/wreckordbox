//! Native DJ judgements come from the recorded edit, never a Library-wide diff.
//! No audio is opened here. Mapping is best effort; omissions belong to the
//! Ledger entry, and cannot prevent either that entry or a later repair edit.
use crate::stems::Stems;
use rbl_db::{Library, LibraryLocation, OpenMode};
use rbl_library::ledger::Preparation;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use wreckordbox_sidecar::{
    dj_facts::{NativeFact, NativeFacts},
    Kinds, TagSchema, ValueType, TAG_SCHEMA_FILE,
};

fn database<'a>(
    slot: &'a mut Option<Result<Library, String>>,
    location: &LibraryLocation,
) -> Result<&'a Library, String> {
    slot.get_or_insert_with(|| {
        Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

pub(crate) fn derive(
    location: &LibraryLocation,
    stems: &Stems,
    kind: &str,
    payload: Option<&Value>,
    asserted: bool,
) -> NativeFacts {
    let mut result = NativeFacts::default();
    let Some(payload) = payload else {
        return result;
    };
    let mut db = None;
    if kind == "track" {
        for edit in payload["edits"].as_array().into_iter().flatten() {
            let track = edit["track"].as_str().unwrap_or_default();
            let column = edit["column"].as_str().unwrap_or_default();
            // An unchanged field is a Fact only in an explicit assertion
            // (`Ledger::assert`, wbx-26), never because a write left it so.
            if edit["before"] == edit["after"] && !asserted {
                continue;
            }
            let value = &edit["after"];
            let (kind, value) = match column {
                "Rating" => (
                    "energy",
                    match value.as_i64() {
                        Some(0) | None if value.is_null() || value == 0 => Ok(Value::Null),
                        Some(n) if (1..=5).contains(&n) => Ok(json!(n)),
                        _ => Err("rating is outside 0–5".into()),
                    },
                ),
                "ColorID" => ("prep_colour", colour(value)),
                "Title" => ("title", text(value)),
                "ArtistID" => (
                    "artist",
                    if value.is_null() || value == "" {
                        Ok(Value::Null)
                    } else {
                        database(&mut db, location).and_then(|db| {
                            db.connection()
                                .query_row(
                                    "SELECT Name FROM djmdArtist WHERE ID=?1",
                                    [value.as_str().unwrap_or_default()],
                                    |r| r.get::<_, Option<String>>(0),
                                )
                                .map(|v| {
                                    v.filter(|s| !s.is_empty())
                                        .map_or(Value::Null, Value::String)
                                })
                                .map_err(|e| e.to_string())
                        })
                    },
                ),
                _ => continue,
            };
            match value {
                Ok(value) => result.values.push(NativeFact {
                    track: track.into(),
                    kind: kind.into(),
                    value,
                    before: None,
                }),
                Err(why) => result.skip(track, kind, why),
            }
        }
    } else if kind == "track_tags" {
        derive_tags(location, stems, payload, &mut result);
    } else if kind == "dj_judgements" {
        // The DJ's own judgements, set as such (wbx-65): each is the whole
        // answer for its kind, so no provenance of unchanged terms is
        // asked for; `null` withdraws the opinion.
        for change in payload["changes"].as_array().into_iter().flatten() {
            if change["before"] == change["after"] {
                continue;
            }
            result.values.push(NativeFact {
                track: change["track"].as_str().unwrap_or_default().into(),
                kind: change["kind"].as_str().unwrap_or_default().into(),
                value: change["after"].clone(),
                before: None,
            });
        }
    }
    result
}

struct TagMapping {
    schema: TagSchema,
    kinds: Kinds,
    catalogue: BTreeMap<String, (String, String)>,
    terms: BTreeMap<String, (String, String)>,
    categories: BTreeMap<String, String>,
}
impl TagMapping {
    fn load(location: &LibraryLocation, stems: &Stems) -> Result<Self, String> {
        let path = stems.sidecar_dir().join(TAG_SCHEMA_FILE);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("Tag schema unavailable: {e}"))?;
        let schema = TagSchema::parse(&text).map_err(|e| format!("invalid Tag schema: {e:?}"))?;
        let kinds = Kinds::load_declared(stems.sidecar_dir())
            .map_err(|e| e.to_string())?
            .with_schema(&schema)
            .map_err(|e| format!("invalid dimension: {e:?}"))?;
        let db = Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())?;
        let mut query = db
            .connection()
            .prepare(
                "SELECT tag.ID, category.Name, tag.Name FROM djmdMyTag tag
             JOIN djmdMyTag category ON category.ID=tag.ParentID
             WHERE tag.rb_local_deleted=0 AND category.rb_local_deleted=0",
            )
            .map_err(|e| e.to_string())?;
        let catalogue: BTreeMap<String, (String, String)> = query
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let views = schema.deck_view();
        let terms = catalogue
            .iter()
            .filter_map(|(id, (category, label))| {
                views
                    .iter()
                    .find(|view| view.label == *category)
                    .and_then(|view| {
                        view.tags
                            .iter()
                            .find(|tag| tag.label == *label)
                            .map(|tag| (id.clone(), (view.dimension.clone(), tag.id.clone())))
                    })
            })
            .collect();
        let categories = views
            .into_iter()
            .map(|view| (view.dimension, view.label))
            .collect();
        Ok(Self {
            schema,
            kinds,
            catalogue,
            terms,
            categories,
        })
    }

    fn value(&self, name: &str, selected: &BTreeSet<&str>, strict: bool) -> Result<Value, String> {
        let dimension = self
            .schema
            .dimension(name)
            .ok_or_else(|| "dimension is absent".to_owned())?;
        let category = self
            .categories
            .get(name)
            .ok_or_else(|| "dimension has no deck view".to_owned())?;
        for id in selected {
            if self.catalogue.get(*id).is_none_or(|(c, _)| c == category)
                && !self.terms.contains_key(*id)
            {
                return Err(format!("tag {id} cannot be mapped in dimension {name}"));
            }
        }
        let terms: BTreeSet<_> = selected
            .iter()
            .filter_map(|id| self.terms.get(*id))
            .filter(|(d, _)| d == name)
            .map(|(_, term)| term.as_str())
            .collect();
        let parents: BTreeSet<_> = terms
            .iter()
            .flat_map(|id| dimension.ancestors(id))
            .collect();
        // Same order and ancestor reduction as schema.convert.
        let terms: Vec<_> = dimension
            .terms
            .iter()
            .filter(|term| terms.contains(term.id.as_str()) && !parents.contains(term.id.as_str()))
            .map(|term| term.id.as_str())
            .collect();
        if strict
            && self
                .kinds
                .get(name)
                .is_some_and(|k| k.value == ValueType::Text)
        {
            match terms.as_slice() {
                [] => Ok(Value::Null),
                [term] => Ok(json!(term)),
                _ => Err(format!(
                    "{name} takes one term but the native edit chose several"
                )),
            }
        } else {
            Ok(json!(terms))
        }
    }
}

fn derive_tags(
    location: &LibraryLocation,
    stems: &Stems,
    payload: &Value,
    result: &mut NativeFacts,
) {
    let mapped = TagMapping::load(location, stems);
    for edit in payload["tags"].as_array().into_iter().flatten() {
        let track = edit["track"].as_str().unwrap_or_default();
        if edit["before"] == edit["after"] {
            continue;
        }
        let mapped = match &mapped {
            Ok(mapped) => mapped,
            Err(why) => {
                result.skip(track, "mytags", why.clone());
                continue;
            }
        };
        let before = ids(&edit["before"]);
        let after = ids(&edit["after"]);
        let mut dimensions = BTreeSet::new();
        for id in before.symmetric_difference(&after) {
            match mapped.terms.get(*id) {
                Some((dimension, _)) => {
                    dimensions.insert(dimension.as_str());
                }
                None => result.skip(
                    track,
                    "mytags",
                    format!("tag {id} is outside the Tag schema; no judgement inferred"),
                ),
            }
        }
        for name in dimensions {
            match mapped.value(name, &after, true).and_then(|after| {
                mapped
                    .value(name, &before, false)
                    .map(|before| (before, after))
            }) {
                Ok((before, value)) => result.values.push(NativeFact {
                    track: track.into(),
                    kind: name.into(),
                    value,
                    before: Some(before),
                }),
                Err(why) => result.skip(track, name, why),
            }
        }
    }
}

fn ids(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
fn text(value: &Value) -> Result<Value, String> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(s) if s.is_empty() => Ok(Value::Null),
        Value::String(_) => Ok(value.clone()),
        _ => Err("native text is not text".into()),
    }
}
fn colour(value: &Value) -> Result<Value, String> {
    if value.is_null() || value == "" || value == "0" || value == 0 {
        return Ok(Value::Null);
    }
    let n = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()));
    n.and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| rbl_index::COLOR_NAMES.get(n))
        .map(|name| json!(name))
        .ok_or_else(|| "unknown native Prep colour".into())
}

/// Recovery only: bounded rows, read on the writer's already-open connection.
/// No configuration or audio reads, and errors become recovery metadata.
pub(crate) fn snapshot(db: &Library, preparation: &Preparation, command: Option<&Value>) -> Value {
    let mut tracks = preparation.tracks.clone();
    let mut columns = vec!["Rating", "ColorID", "Title", "ArtistID"];
    if let Some(command) = command {
        match command["name"].as_str() {
            Some("tracks.rate") => {
                columns = vec!["Rating"];
                tracks = command["args"]["id"]
                    .as_str()
                    .map(|s| vec![s.into()])
                    .unwrap_or_default();
            }
            Some("tracks.set_prep_colour") => {
                columns = vec!["ColorID"];
                tracks = command["args"]["id"]
                    .as_str()
                    .map(|s| vec![s.into()])
                    .unwrap_or_default();
            }
            Some("mytags.assign" | "mytags.unassign") => {
                columns = vec!["MyTags"];
                tracks = command["args"]["tracks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            // The tracks of the accept's resolved selection (wbx-27).
            Some("review.accept") => columns = vec!["Rating", "ColorID"],
            Some(_) => return Value::Null,
            None => {}
        }
    }
    let fields = if preparation.fields.is_empty() {
        tracks
            .into_iter()
            .flat_map(|track| {
                columns
                    .iter()
                    .map(move |column| (track.clone(), (*column).to_owned()))
            })
            .collect()
    } else {
        preparation.fields.clone()
    };
    if fields.is_empty() {
        return Value::Null;
    }
    let mut values = Vec::new();
    for (track, column) in fields {
        match native(db, &track, &column) {
            Ok(value) => values.push(json!({"track":track,"column":column,"before":value})),
            Err(why) => return json!({"version":2,"error":why}),
        }
    }
    json!({"version":2,"values":values,"undo_of":preparation.undo_of,"redo_of":preparation.redo_of})
}
fn native(db: &Library, track: &str, column: &str) -> Result<Value, String> {
    if column == "MyTags" {
        let mut query=db.connection().prepare("SELECT MyTagID FROM djmdSongMyTag WHERE ContentID=?1 AND rb_local_deleted=0 ORDER BY MyTagID").map_err(|e|e.to_string())?;
        return query
            .query_map([track], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map(|tags| json!(tags))
            .map_err(|e| e.to_string());
    }
    // The column is chosen only from the fixed native field list above.
    let sql = match column {
        "Rating" => "SELECT Rating FROM djmdContent WHERE ID=?1",
        "ColorID" => "SELECT ColorID FROM djmdContent WHERE ID=?1",
        "Title" => "SELECT Title FROM djmdContent WHERE ID=?1",
        "ArtistID" => "SELECT ArtistID FROM djmdContent WHERE ID=?1",
        _ => return Err("unknown recovery column".into()),
    };
    db.connection()
        .query_row(sql, [track], |r| r.get::<_, rusqlite::types::Value>(0))
        .map(|v| match v {
            rusqlite::types::Value::Integer(n) => json!(n),
            rusqlite::types::Value::Text(s) => json!(s),
            _ => Value::Null,
        })
        .map_err(|e| e.to_string())
}
/// An unconfirmed entry may reconcile only its bounded recovery evidence.
pub(crate) fn recover(
    location: &LibraryLocation,
    stems: &Stems,
    snapshot: Option<&Value>,
) -> NativeFacts {
    let mut facts = NativeFacts::default();
    let Some(snapshot) = snapshot.filter(|s| s["version"] == 2) else {
        facts.skip(
            "",
            "recovery",
            "no bounded recovery evidence; no DJ judgement inferred",
        );
        return facts;
    };
    if let Some(error) = snapshot["error"].as_str() {
        facts.skip("", "recovery", error);
        return facts;
    }
    let db = match Library::open(location.clone(), OpenMode::ReadOnly) {
        Ok(db) => db,
        Err(e) => {
            facts.skip("", "recovery", e.to_string());
            return facts;
        }
    };
    let mut edits = Vec::new();
    let mut tags = Vec::new();
    for row in snapshot["values"].as_array().into_iter().flatten() {
        let track = row["track"].as_str().unwrap_or_default();
        let column = row["column"].as_str().unwrap_or_default();
        match native(&db, track, column) {
            Ok(after) if after != row["before"] => {
                let edit =
                    json!({"track":track,"column":column,"before":row["before"],"after":after});
                if column == "MyTags" {
                    tags.push(edit);
                } else {
                    edits.push(edit);
                }
            }
            Ok(_) => {}
            Err(e) => facts.skip(track, "recovery", e),
        }
    }
    if edits.is_empty() && tags.is_empty() {
        return facts;
    }
    if let Some(id) = snapshot["undo_of"].as_i64() {
        facts.undo_of = Some(id);
        return facts;
    }
    let native = derive(
        location,
        stems,
        "track",
        Some(&json!({"edits":edits})),
        false,
    );
    let tagged = derive(
        location,
        stems,
        "track_tags",
        Some(&json!({"tags":tags})),
        false,
    );
    facts.values.extend(native.values);
    facts.values.extend(tagged.values);
    facts.skipped.extend(native.skipped);
    facts.skipped.extend(tagged.skipped);
    facts.redo_of = snapshot["redo_of"].as_i64();
    facts
}
