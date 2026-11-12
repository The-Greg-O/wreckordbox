//! Read-only native values around the Ledger's one recording boundary.
//! The intent retains the vocabulary used before the write: recovery must
//! not mistake a later schema-file edit for a Library edit.
use crate::stems::Stems;
use rbl_db::{Library, LibraryLocation, OpenMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use wreckordbox_sidecar::{
    dj_facts::{NativeAudio, NativeFact, NativeFacts},
    Kinds, TagSchema, ValueType, TAG_SCHEMA_FILE,
};

type Values = BTreeMap<String, BTreeMap<String, Value>>;
#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema: Option<String>,
    track: Option<String>,
    values: Values,
    #[serde(default)]
    text_dimensions: Vec<String>,
}

pub(crate) fn before(
    location: &LibraryLocation,
    stems: &Stems,
    command: Option<&Value>,
) -> Result<Value, String> {
    let file = stems.sidecar_dir().join(TAG_SCHEMA_FILE);
    let schema = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && file.symlink_metadata().is_err() => {
            None
        }
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let track = command
        .filter(|c| {
            matches!(
                c["name"].as_str(),
                Some("tracks.rate" | "tracks.set_prep_colour")
            )
        })
        .and_then(|c| c["args"]["id"].as_str())
        .map(str::to_owned);
    let text_dimensions = match schema.as_deref() {
        Some(text) => {
            let schema = TagSchema::parse(text)
                .map_err(|errors| format!("invalid Tag schema: {errors:?}"))?;
            let kinds = Kinds::load_declared(stems.sidecar_dir())
                .map_err(|e| e.to_string())?
                .with_schema(&schema)
                .map_err(|errors| format!("invalid Tag schema: {errors:?}"))?;
            schema
                .dimensions
                .iter()
                .filter(|d| {
                    kinds
                        .get(&d.name)
                        .is_some_and(|k| k.value == ValueType::Text)
                })
                .map(|d| d.name.clone())
                .collect()
        }
        None => Vec::new(),
    };
    let values = read(
        location,
        schema.as_deref(),
        track.as_deref(),
        &text_dimensions,
    )?;
    serde_json::to_value(Snapshot {
        schema,
        track,
        values,
        text_dimensions,
    })
    .map_err(|e| e.to_string())
}

pub(crate) fn changed(
    location: &LibraryLocation,
    before: Option<&Value>,
) -> Result<NativeFacts, String> {
    let Some(before) = before else {
        return Ok(NativeFacts::default());
    };
    let before: Snapshot = serde_json::from_value(before.clone()).map_err(|e| e.to_string())?;
    let after = read(
        location,
        before.schema.as_deref(),
        before.track.as_deref(),
        &before.text_dimensions,
    )?;
    let mut facts = Vec::new();
    for (track, fields) in before.values {
        let Some(current) = after.get(&track) else {
            continue;
        };
        for (kind, old) in fields {
            if let Some(value) = current.get(&kind).filter(|value| **value != old) {
                facts.push(NativeFact {
                    track: track.clone(),
                    kind,
                    value: value.clone(),
                });
            }
        }
    }
    if facts.is_empty() {
        return Ok(NativeFacts::default());
    }
    let db = Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())?;
    let mut query = db
        .connection()
        .prepare("SELECT ID FROM djmdContent WHERE rb_local_deleted=0")
        .map_err(|e| e.to_string())?;
    let present = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut audio = BTreeMap::new();
    let paths = db.track_paths();
    for fact in &facts {
        if audio.contains_key(&fact.track) {
            continue;
        }
        let stored = db.stored_path(&fact.track).map_err(|e| e.to_string())?;
        let path = stored.map(|stored| std::path::PathBuf::from(paths.resolve(&stored)));
        let fingerprint = match path {
            Some(path) if path.try_exists().map_err(|e| e.to_string())? => {
                Some(wreckordbox_sidecar::fingerprint_file(&path).map_err(|e| e.to_string())?)
            }
            _ => None,
        };
        let identity = db.connection().query_row("SELECT COALESCE(c.Title,''), COALESCE(a.Name,''), COALESCE(c.Subtitle,''), COALESCE(c.Length,0) FROM djmdContent c LEFT JOIN djmdArtist a ON a.ID=c.ArtistID WHERE c.ID=?1", [&fact.track], |row| Ok(wreckordbox_sidecar::attachments::IdentifyingFields {title:row.get(0)?,artist:row.get(1)?,mix_name:row.get(2)?,duration:row.get(3)?})).map_err(|e| e.to_string())?;
        audio.insert(
            fact.track.clone(),
            NativeAudio {
                fingerprint,
                identity,
            },
        );
    }
    Ok(NativeFacts {
        values: facts,
        audio,
        present,
    })
}

fn read(
    location: &LibraryLocation,
    schema: Option<&str>,
    track: Option<&str>,
    text_dimensions: &[String],
) -> Result<Values, String> {
    let db = Library::open(location.clone(), OpenMode::ReadOnly).map_err(|e| e.to_string())?;
    let conn = db.connection();
    let mut statement = conn
        .prepare(
            "SELECT c.ID, c.Title, a.Name, c.Rating, c.ColorID
        FROM djmdContent c LEFT JOIN djmdArtist a ON a.ID=c.ArtistID
        WHERE c.rb_local_deleted=0 AND (?1 IS NULL OR c.ID=?1)",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([track], |row| {
            let scalar = |column| -> rusqlite::Result<Value> {
                Ok(row
                    .get::<_, Option<String>>(column)?
                    .filter(|s| !s.is_empty())
                    .map_or(Value::Null, Value::String))
            };
            let rating: Option<i64> = row.get(3)?;
            Ok((
                row.get::<_, String>(0)?,
                BTreeMap::from([
                    ("title".to_owned(), scalar(1)?),
                    ("artist".to_owned(), scalar(2)?),
                    (
                        "energy".to_owned(),
                        rating.filter(|r| *r > 0).map_or(Value::Null, |r| json!(r)),
                    ),
                    (
                        "prep_colour".to_owned(),
                        row.get::<_, Option<String>>(4)?
                            .and_then(|id| id.parse::<usize>().ok())
                            .and_then(|id| id.checked_sub(1))
                            .and_then(|at| rbl_index::COLOR_NAMES.get(at))
                            .map_or(Value::Null, |name| json!(name)),
                    ),
                ]),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut values: Values = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    let Some(text) = schema else {
        return Ok(values);
    };
    let schema =
        TagSchema::parse(text).map_err(|errors| format!("invalid Tag schema: {errors:?}"))?;
    let views = schema.deck_view();
    let mut tags = conn
        .prepare(
            "SELECT s.ContentID, category.Name, tag.Name
        FROM djmdSongMyTag s JOIN djmdMyTag tag ON tag.ID=s.MyTagID
        JOIN djmdMyTag category ON category.ID=tag.ParentID
        WHERE s.rb_local_deleted=0 AND tag.rb_local_deleted=0 AND category.rb_local_deleted=0
          AND (?1 IS NULL OR s.ContentID=?1)",
        )
        .map_err(|e| e.to_string())?;
    let rows = tags
        .query_map([track], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut assigned: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for row in rows {
        let (track, category, tag) = row.map_err(|e| e.to_string())?;
        assigned.entry(track).or_default().push((category, tag));
    }
    for (track, fields) in &mut values {
        for view in &views {
            let mut terms: Vec<String> = view
                .tags
                .iter()
                .filter(|tag| {
                    assigned
                        .get(track)
                        .is_some_and(|tags| tags.contains(&(view.label.clone(), tag.label.clone())))
                })
                .map(|tag| tag.id.clone())
                .collect();
            if let Some(dimension) = schema.dimension(&view.dimension) {
                let parents: Vec<String> = terms
                    .iter()
                    .flat_map(|term| dimension.ancestors(term))
                    .map(str::to_owned)
                    .collect();
                terms.retain(|term| !parents.contains(term));
            }
            terms.sort();
            terms.dedup();
            let value =
                if text_dimensions.contains(&view.dimension) {
                    match terms.as_slice() {
                        [] => Value::Null,
                        [term] => json!(term),
                        _ => return Err(format!(
                            "{} is a single-text dimension but track {track} carries several terms",
                            view.dimension
                        )),
                    }
                } else {
                    json!(terms)
                };
            fields.insert(view.dimension.clone(), value);
        }
    }
    Ok(values)
}
