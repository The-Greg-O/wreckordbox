//! A row's MyTags for the view shell's Library (#55): each tag as itself,
//! with its id and the name of its MyTag column (category), so the window
//! never rebuilds tags from the joined display names, which break on a tag
//! whose name holds a comma.
//!
//! Read beside `rbl_db::details::my_tag_names` rather than in it: `rbl-db` is
//! not reshaped (ADR 0003).

use rusqlite::Connection;
use serde_json::{json, Value};

/// The track's MyTags in the Library's order, as `{ id, name, category }`.
/// A read that fails gives no tags, as the joined-name column does.
pub(crate) fn row_my_tags(conn: &Connection, id: &str) -> Value {
    let Ok(mut stmt) = conn.prepare_cached(
        "SELECT t.ID, COALESCE(t.Name, ''), COALESCE(c.Name, '') FROM djmdSongMyTag s
         JOIN djmdMyTag t ON t.ID = s.MyTagID AND t.rb_local_deleted = 0
         LEFT JOIN djmdMyTag c ON c.ID = t.ParentID AND c.rb_local_deleted = 0
         WHERE s.ContentID = ?1 AND s.rb_local_deleted = 0
         ORDER BY s.TrackNo, s.MyTagID",
    ) else {
        return json!([]);
    };
    let tags: Vec<Value> = stmt
        .query_map([id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map(|rows| {
            rows.filter_map(std::result::Result::ok)
                .filter(|(_, name, _)| !name.is_empty())
                .map(|(id, name, category)| json!({ "id": id, "name": name, "category": category }))
                .collect()
        })
        .unwrap_or_default();
    Value::Array(tags)
}
