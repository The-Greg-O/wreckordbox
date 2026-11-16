//! Native values an Unwind can compare without opening a Writer.
use rbl_db::DbError;
use rbl_library::history::LibraryEdit;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

#[derive(Clone)]
pub(crate) struct Part {
    pub key: String,
    pub kind: String,
    pub shape: Value,
    pub before: Value,
    pub after: Value,
}

pub(crate) fn parts(kind: &str, payload: &Value) -> Option<Vec<Part>> {
    // Validate all columns before using a stored name in SQL.
    crate::ledger::stored::restore(kind, 1, payload)?;
    let mut out = Vec::new();
    match kind {
        "unwind" => {
            for edit in payload["edits"].as_array()? {
                out.extend(parts(edit["kind"].as_str()?, &edit["payload"])?);
            }
        }
        "track" | "track_tags" => {
            let scalars = if kind == "track" { "edits" } else { "comments" };
            for edit in payload[scalars].as_array()? {
                out.push(Part {
                    key: format!("track:{}:{}", edit["track"], edit["column"]),
                    kind: "track".into(),
                    shape: edit.clone(),
                    before: edit["before"].clone(),
                    after: edit["after"].clone(),
                });
            }
            if kind == "track_tags" {
                for tag in payload["tags"].as_array()? {
                    out.push(Part {
                        key: format!("tags:{}", tag["track"]),
                        kind: "track_tags".into(),
                        shape: tag.clone(),
                        before: sorted(&tag["before"]),
                        after: sorted(&tag["after"]),
                    });
                }
            }
        }
        "my_tag_order" | "reorder_playlist_entries" => {
            let field = if kind == "my_tag_order" {
                "parent"
            } else {
                "playlist"
            };
            out.push(Part {
                key: format!("{kind}:{}", payload[field]),
                kind: kind.into(),
                shape: payload.clone(),
                before: payload["before"].clone(),
                after: payload["after"].clone(),
            });
        }
        // These live in the Fact effect journal, not native rows.
        "dj_judgements" => {}
        _ => return None,
    }
    Some(out)
}

fn sorted(value: &Value) -> Value {
    let mut items = value.as_array().cloned().unwrap_or_default();
    items.sort_by_key(Value::to_string);
    items.dedup();
    json!(items)
}

pub(crate) fn current(db: &Connection, part: &Part) -> Result<Option<Value>, DbError> {
    let shape = &part.shape;
    let value = match part.kind.as_str() {
        "track" => {
            let column = shape["column"].as_str().unwrap_or_default();
            let sql =
                format!("SELECT `{column}` FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0");
            return Ok(db
                .query_row(&sql, [shape["track"].as_str().unwrap_or_default()], |r| {
                    r.get::<_, rusqlite::types::Value>(0)
                })
                .optional()?
                .map(|v| crate::ledger::stored::sql_json(&v)));
        }
        "track_tags" => {
            let track = shape["track"].as_str().unwrap_or_default();
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0)",
                [track],
                |r| r.get(0),
            )?;
            if !exists {
                return Ok(None);
            }
            let mut q = db.prepare("SELECT DISTINCT MyTagID FROM djmdSongMyTag WHERE ContentID=?1 AND rb_local_deleted=0 ORDER BY MyTagID")?;
            let ids = q
                .query_map([track], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            json!(ids)
        }
        "my_tag_order" => {
            let mut q = db.prepare("SELECT ID,Seq FROM djmdMyTag WHERE ParentID=?1 AND rb_local_deleted=0 ORDER BY Seq,ID")?;
            let rows = q
                .query_map([shape["parent"].as_str().unwrap_or_default()], |r| {
                    Ok(json!({"id":r.get::<_,String>(0)?,"seq":r.get::<_,Option<i64>>(1)?}))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            json!(rows)
        }
        "reorder_playlist_entries" => {
            let mut q = db.prepare("SELECT ID,TrackNo FROM djmdSongPlaylist WHERE PlaylistID=?1 AND rb_local_deleted=0 ORDER BY TrackNo,ID")?;
            let rows = q
                .query_map([shape["playlist"].as_str().unwrap_or_default()], |r| {
                    Ok(json!({"entry":r.get::<_,String>(0)?,"number":r.get::<_,i64>(1)?}))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            json!(rows)
        }
        _ => return Err(DbError::WriteRefused("Unsupported Unwind value".into())),
    };
    Ok(Some(value))
}

pub(crate) fn edit(part: &Part) -> Option<LibraryEdit> {
    let mut shape = part.shape.clone();
    shape["before"] = part.before.clone();
    shape["after"] = part.after.clone();
    let payload = match part.kind.as_str() {
        "track" => json!({"edits":[shape]}),
        "track_tags" => json!({"tags":[shape],"comments":[]}),
        _ => shape,
    };
    crate::ledger::stored::restore(&part.kind, 1, &payload)
}
