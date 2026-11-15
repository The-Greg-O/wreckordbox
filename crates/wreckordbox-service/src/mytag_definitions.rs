//! MyTag definitions in MyTag Projection (wbx-14.14): the tags the Tag
//! schema's deck view has and the Library lacks are made, a tag the
//! projection itself made or renamed is renamed and placed as the deck view
//! says, and everything the DJ made is left as it is and reported.
//!
//! **Which MyTag a deck tag is.** First the MyTag of exactly the deck tag's
//! text in its category (as for assignments). Failing that, the MyTag a
//! definition mark links to the same term ([`DefinitionMark`]: written by
//! every apply that wrote, for each deck tag it found or made), when it is
//! still in that category: the deck tag was relabelled in the schema.
//! Failing both, the MyTag is made (`create_tag`), at the end of its
//! category, unless the category already holds its fifty, or a MyTag
//! linked to the term was removed since (an Undo of the projection that
//! made it, or a delete by hand): that absence is the DJ's (`removed`), as
//! an Undone assignment is.
//!
//! **Who owns a definition.** A projection may rename or move a MyTag only
//! when its mark says a projection made or renamed it, that write's Ledger
//! entry was recorded, the row is still exactly as that write left it
//! (name, `Seq`, `rb_local_usn`, `updated_at`: any later write of any kind,
//! an Undo or a Redo included, gives it another update number) and no
//! barrier came after it. Every other MyTag is the DJ's: never renamed,
//! moved relative to the DJ's others, or removed. A relabelled deck tag whose
//! MyTag is the DJ's is `kept` (`rename_tag`) and its assignments blocked
//! (`definition_kept`). Categories are always the DJ's (rekordbox's four
//! rows): a deck label no category has is reported, never written.
//!
//! **Order.** The deck view's tags, in its order, then the category's other
//! MyTags in their order. The projection writes that order only when it
//! keeps the DJ's MyTags in the order they have among themselves (it places
//! only its own); otherwise the order is `kept` (`order_tags`). The
//! categories' order is the DJ's: a different deck order is reported only.
//!
//! Definitions take no confidence and wait for no policy: they are what the
//! DJ's own Tag schema says. They are written with the assignments in ONE
//! transaction and are one Undo step with them
//! ([`rbl_library::my_tag_definitions`]).
#![allow(
    clippy::doc_markdown,
    reason = "MyTag and the like are the glossary's words, not code items"
)]

use std::collections::HashMap;

use rbl_db::DbError;
use rbl_library::my_tag_definitions::{
    Created, DefinitionChanges, Definitions, NewTag, Ordered, Renamed,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::mytag_definitions::{DefinitionMark, DefinitionMarks};
use wreckordbox_sidecar::projection::Barrier;
use wreckordbox_sidecar::tag_schema::DeckView;

/// A change to the MyTags themselves: planned, made, or kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// `create_tag`, `rename_tag`, `order_tags`, `order_categories` or
    /// `rename_category` (only ever kept).
    pub change: String,
    /// The deck category's label (empty for `order_categories`).
    pub category: String,
    #[serde(default)]
    pub category_id: Option<String>,
    /// The MyTag's text as the deck shows it (empty for an order).
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dimension: String,
    #[serde(default)]
    pub term: String,
    /// The MyTag renamed (or kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Its text now, for a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The order wanted, as texts, for an order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Why it is kept: `set_by_dj`, `no_record`, `written_since`,
    /// `provenance_incomplete`, `category_full`, `removed` (a MyTag linked
    /// to the term was taken away since, by an Undo or by hand: never made
    /// again by a projection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One live `djmdMyTag` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub id: String,
    pub parent: String,
    pub name: String,
    pub seq: Option<i64>,
    pub usn: i64,
    pub updated_at: String,
    pub category: bool,
}

/// Every live `djmdMyTag` row, categories and tags, by parent and `Seq`.
pub(crate) fn rows(conn: &Connection) -> Result<Vec<Row>, DbError> {
    let mut statement = conn.prepare(
        "SELECT ID, COALESCE(ParentID, ''), COALESCE(Name, ''), Seq,
                COALESCE(rb_local_usn, 0), COALESCE(updated_at, ''), COALESCE(Attribute, 0)
         FROM djmdMyTag WHERE rb_local_deleted = 0 ORDER BY ParentID, Seq, ID",
    )?;
    let rows = statement
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                parent: r.get(1)?,
                name: r.get(2)?,
                seq: r.get(3)?,
                usn: r.get(4)?,
                updated_at: r.get(5)?,
                category: r.get::<_, i64>(6)? == 1,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Which MyTag a deck tag's assignments go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// A MyTag in the Library.
    Tag(String),
    /// The `n`th MyTag this apply makes.
    ToMake(usize),
    /// Nowhere: the reason its assignments are blocked.
    Blocked(&'static str),
}

/// An item of a wanted order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    Tag(String),
    New(usize),
}

/// A MyTag to make, as planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToMake {
    pub category: String,
    pub name: String,
    pub dimension: String,
    pub term: String,
}

/// The definitions part of a plan.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    /// Planned changes, for the report.
    pub listed: Vec<Definition>,
    /// What is the DJ's and stays, for the report.
    pub kept: Vec<Definition>,
    /// By `(category id, term)`.
    pub resolved: HashMap<(String, String), Resolved>,
    pub make: Vec<ToMake>,
    /// `(tag, new name)`.
    pub rename: Vec<(String, String)>,
    /// `(category, wanted order)`.
    pub order: Vec<(String, Vec<Item>)>,
    /// `(tag, category, dimension, term)` of every deck tag found, for the
    /// links the apply keeps.
    pub links: Vec<(String, String, String, String)>,
    /// The rows the plan read, compared again under the edit gate.
    pub read: Vec<Row>,
    /// The marks and barriers it was decided by.
    pub marks: DefinitionMarks,
    pub barriers: Vec<Barrier>,
}

impl Plan {
    pub fn writes(&self) -> bool {
        !self.make.is_empty() || !self.rename.is_empty() || !self.order.is_empty()
    }
}

/// Why a projection may not rename or move `row`: `None` when it may.
fn not_owned(row: &Row, marks: &DefinitionMarks, barriers: &[Barrier]) -> Option<&'static str> {
    let Some(mark) = marks.marks.get(&row.id) else {
        return Some("no_record");
    };
    if !mark.owned {
        return Some("set_by_dj");
    }
    if !marks.recorded.contains(&mark.entry)
        || mark.name != row.name
        || mark.seq != row.seq
        || mark.usn != row.usn
        || mark.updated_at != row.updated_at
        || mark.category != row.parent
    {
        return Some("written_since");
    }
    // A definition is no track's: any barrier after the write could be it.
    if barriers.iter().any(|b| b.seq > mark.ledger_seq) {
        return Some("provenance_incomplete");
    }
    None
}

/// The definitions part of the plan for the deck view's categories, each
/// placed at its Library category (`None`: missing or ambiguous).
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the deck view, in order, as the assignments' plan"
)]
pub(crate) fn plan(
    views: &[DeckView],
    placed: &[Option<String>],
    read: Vec<Row>,
    marks: &DefinitionMarks,
    barriers: &[Barrier],
) -> Plan {
    let mut plan = Plan::default();
    let live = |id: &str| read.iter().find(|r| r.id == id);
    for (view, category) in views.iter().zip(placed) {
        let Some(category) = category else {
            // A deck label no category has, where a link knows the
            // category the dimension was in: the DJ's, reported.
            let linked = marks
                .marks
                .values()
                .filter(|m| m.dimension == view.dimension)
                .find_map(|m| live(&m.category).filter(|c| c.category));
            if let Some(row) = linked {
                plan.kept.push(Definition {
                    change: "rename_category".into(),
                    category: view.label.clone(),
                    category_id: Some(row.id.clone()),
                    label: view.label.clone(),
                    dimension: view.dimension.clone(),
                    term: String::new(),
                    tag: None,
                    from: Some(row.name.clone()),
                    order: Vec::new(),
                    reason: Some("set_by_dj".into()),
                });
            }
            continue;
        };
        let there: Vec<&Row> = read
            .iter()
            .filter(|r| r.parent == *category && !r.category)
            .collect();
        let labels: Vec<&str> = view.tags.iter().map(|t| t.label.as_str()).collect();
        let base = |change: &str, tag: &wreckordbox_sidecar::tag_schema::DeckTag| Definition {
            change: change.into(),
            category: view.label.clone(),
            category_id: Some(category.clone()),
            label: tag.label.clone(),
            dimension: view.dimension.clone(),
            term: tag.id.clone(),
            tag: None,
            from: None,
            order: Vec::new(),
            reason: None,
        };
        let mut room = 50usize.saturating_sub(there.len());
        // Wanted order: (item, label).
        let mut wanted: Vec<(Item, String)> = Vec::new();
        let mut taken: Vec<String> = Vec::new();
        for tag in &view.tags {
            let key = (category.clone(), tag.id.clone());
            // A kept MyTag still stands at its term's place in the order.
            let mut stays: Option<&Row> = None;
            let exact: Vec<&&Row> = there.iter().filter(|r| r.name == tag.label).collect();
            let resolved = match exact.as_slice() {
                [one] => {
                    plan.links.push((
                        one.id.clone(),
                        category.clone(),
                        view.dimension.clone(),
                        tag.id.clone(),
                    ));
                    Resolved::Tag(one.id.clone())
                }
                [_, _, ..] => Resolved::Blocked("ambiguous_my_tag"),
                [] => {
                    // Relabelled: a link to this term, still in this
                    // category, whose text is no other deck tag's.
                    let linked = there.iter().find(|r| {
                        marks
                            .marks
                            .get(&r.id)
                            .is_some_and(|m| m.dimension == view.dimension && m.term == tag.id)
                            && !labels.contains(&r.name.as_str())
                    });
                    // A MyTag linked to this term that is gone (an Undo, a
                    // delete by hand): its absence is the DJ's.
                    let removed = linked.is_none()
                        && marks.marks.iter().any(|(id, m)| {
                            m.dimension == view.dimension
                                && m.term == tag.id
                                && m.category == *category
                                && live(id).is_none()
                        });
                    match linked {
                        None if removed => {
                            let mut kept = base("create_tag", tag);
                            kept.reason = Some("removed".into());
                            plan.kept.push(kept);
                            Resolved::Blocked("definition_kept")
                        }
                        Some(row) => match not_owned(row, marks, barriers) {
                            None => {
                                let mut listed = base("rename_tag", tag);
                                listed.tag = Some(row.id.clone());
                                listed.from = Some(row.name.clone());
                                plan.listed.push(listed);
                                plan.rename.push((row.id.clone(), tag.label.clone()));
                                plan.links.push((
                                    row.id.clone(),
                                    category.clone(),
                                    view.dimension.clone(),
                                    tag.id.clone(),
                                ));
                                Resolved::Tag(row.id.clone())
                            }
                            Some(reason) => {
                                let mut kept = base("rename_tag", tag);
                                kept.tag = Some(row.id.clone());
                                kept.from = Some(row.name.clone());
                                kept.reason = Some(reason.into());
                                plan.kept.push(kept);
                                stays = Some(row);
                                Resolved::Blocked("definition_kept")
                            }
                        },
                        None if room == 0 => {
                            let mut kept = base("create_tag", tag);
                            kept.reason = Some("category_full".into());
                            plan.kept.push(kept);
                            Resolved::Blocked("category_full")
                        }
                        None => {
                            room -= 1;
                            plan.listed.push(base("create_tag", tag));
                            plan.make.push(ToMake {
                                category: category.clone(),
                                name: tag.label.clone(),
                                dimension: view.dimension.clone(),
                                term: tag.id.clone(),
                            });
                            Resolved::ToMake(plan.make.len() - 1)
                        }
                    }
                }
            };
            match &resolved {
                Resolved::Tag(id) => {
                    taken.push(id.clone());
                    wanted.push((Item::Tag(id.clone()), tag.label.clone()));
                }
                Resolved::ToMake(n) => wanted.push((Item::New(*n), tag.label.clone())),
                Resolved::Blocked(_) => {
                    if let Some(row) = stays {
                        taken.push(row.id.clone());
                        wanted.push((Item::Tag(row.id.clone()), row.name.clone()));
                    }
                }
            }
            plan.resolved.insert(key, resolved);
        }
        // The category's other MyTags keep their order, after the deck's.
        for row in &there {
            if !taken.contains(&row.id) {
                wanted.push((Item::Tag(row.id.clone()), row.name.clone()));
            }
        }
        // Now: the run as it is, with the made tags at its end.
        let mut now: Vec<Item> = there.iter().map(|r| Item::Tag(r.id.clone())).collect();
        now.extend(wanted.iter().filter_map(|(item, _)| match item {
            Item::New(n) => Some(Item::New(*n)),
            Item::Tag(_) => None,
        }));
        let order: Vec<Item> = wanted.iter().map(|(item, _)| item.clone()).collect();
        if order == now {
            continue;
        }
        let djs = |item: &Item| match item {
            Item::New(_) => false,
            Item::Tag(id) => {
                let renamed = plan.rename.iter().any(|(r, _)| r == id);
                !renamed && live(id).is_none_or(|row| not_owned(row, marks, barriers).is_some())
            }
        };
        let theirs = |list: &[Item]| list.iter().filter(|i| djs(i)).cloned().collect::<Vec<_>>();
        let mut listed = Definition {
            change: "order_tags".into(),
            category: view.label.clone(),
            category_id: Some(category.clone()),
            label: String::new(),
            dimension: view.dimension.clone(),
            term: String::new(),
            tag: None,
            from: None,
            order: wanted.iter().map(|(_, label)| label.clone()).collect(),
            reason: None,
        };
        if theirs(&order) == theirs(&now) {
            plan.listed.push(listed);
            plan.order.push((category.clone(), order));
        } else {
            listed.reason = Some("set_by_dj".into());
            plan.kept.push(listed);
        }
    }
    // The categories' order: the DJ's; reported when the deck view lists
    // its categories in another order than the Library has them.
    let categories: Vec<&Row> = read.iter().filter(|r| r.category).collect();
    let deck: Vec<&Row> = placed
        .iter()
        .flatten()
        .filter_map(|id| categories.iter().find(|r| r.id == *id).copied())
        .collect();
    let library: Vec<&Row> = categories
        .iter()
        .filter(|r| deck.iter().any(|d| d.id == r.id))
        .copied()
        .collect();
    if deck.iter().map(|r| &r.id).ne(library.iter().map(|r| &r.id)) {
        plan.kept.push(Definition {
            change: "order_categories".into(),
            category: String::new(),
            category_id: None,
            label: String::new(),
            dimension: String::new(),
            term: String::new(),
            tag: None,
            from: None,
            order: deck.iter().map(|r| r.name.clone()).collect(),
            reason: Some("set_by_dj".into()),
        });
    }
    plan.read = read;
    plan.marks = marks.clone();
    plan.barriers = barriers.to_vec();
    plan
}

/// The changes to write, with the identifiers chosen for the made tags.
pub(crate) fn changes(plan: &Plan, ids: &[(String, String)]) -> DefinitionChanges {
    let id_of = |item: &Item| match item {
        Item::Tag(id) => id.clone(),
        Item::New(n) => ids[*n].0.clone(),
    };
    DefinitionChanges {
        create: plan
            .make
            .iter()
            .zip(ids)
            .map(|(make, (id, uuid))| NewTag {
                id: id.clone(),
                uuid: uuid.clone(),
                category: make.category.clone(),
                name: make.name.clone(),
            })
            .collect(),
        rename: plan.rename.clone(),
        order: plan
            .order
            .iter()
            .map(|(category, items)| (category.clone(), items.iter().map(id_of).collect()))
            .collect(),
    }
}

/// The marks an apply keeps, from the rows its transaction left: owned for
/// every MyTag it made, renamed or moved that was its own (or new), a link
/// alone for the DJ's.
pub(crate) fn marks_after(
    plan: &Plan,
    ids: &[(String, String)],
    after: &[Row],
) -> Vec<(String, DefinitionMark)> {
    let (marks, barriers) = (&plan.marks, plan.barriers.as_slice());
    let mut kept: Vec<(String, DefinitionMark)> = Vec::new();
    let mut put = |id: &str, owned: bool, category: &str, dimension: &str, term: &str| {
        let Some(row) = after.iter().find(|r| r.id == id) else {
            return;
        };
        if kept.iter().any(|(k, _)| k == id) {
            return;
        }
        kept.push((
            id.to_owned(),
            DefinitionMark {
                category: category.to_owned(),
                dimension: dimension.to_owned(),
                term: term.to_owned(),
                owned,
                name: row.name.clone(),
                seq: row.seq,
                usn: row.usn,
                updated_at: row.updated_at.clone(),
                entry: 0,
                ledger_seq: 0,
            },
        ));
    };
    for (make, (id, _)) in plan.make.iter().zip(ids) {
        put(id, true, &make.category, &make.dimension, &make.term);
    }
    for (id, category, dimension, term) in &plan.links {
        let before = plan.read.iter().find(|r| r.id == *id);
        let owned = plan.rename.iter().any(|(r, _)| r == id)
            || before.is_some_and(|row| not_owned(row, marks, barriers).is_none());
        // An owned MyTag left as it was keeps the mark it has.
        let unchanged = before == after.iter().find(|r| r.id == *id);
        if owned && unchanged {
            continue;
        }
        if !owned
            && marks
                .marks
                .get(id)
                .is_some_and(|m| !m.owned && m.term == *term && unchanged)
        {
            continue;
        }
        put(id, owned, category, dimension, term);
    }
    kept
}

/// The links (no ownership) an apply that writes nothing keeps: one for
/// each deck tag found whose MyTag has no mark for its term.
pub(crate) fn links_to_keep(plan: &Plan) -> Vec<(String, DefinitionMark)> {
    plan.links
        .iter()
        .filter(|(id, _, dimension, term)| {
            plan.marks
                .marks
                .get(id)
                .is_none_or(|m| m.dimension != *dimension || m.term != *term)
        })
        .filter_map(|(id, category, dimension, term)| {
            let row = plan.read.iter().find(|r| r.id == *id)?;
            Some((
                id.clone(),
                DefinitionMark {
                    category: category.clone(),
                    dimension: dimension.clone(),
                    term: term.clone(),
                    owned: false,
                    name: row.name.clone(),
                    seq: row.seq,
                    usn: row.usn,
                    updated_at: row.updated_at.clone(),
                    entry: 0,
                    ledger_seq: 0,
                },
            ))
        })
        .collect()
}

/// The stored form of [`Definitions`], in a `track_tags` Ledger payload.
#[must_use]
pub fn stored(definitions: &Definitions) -> Value {
    let run = |run: &[(String, Option<i64>)]| -> Vec<Value> {
        run.iter()
            .map(|(id, seq)| json!({"id": id, "seq": seq}))
            .collect()
    };
    json!({
        "created": definitions.created.iter().map(|c| json!({
            "id": c.id, "uuid": c.uuid, "category": c.category, "name": c.name, "seq": c.seq,
        })).collect::<Vec<Value>>(),
        "renamed": definitions.renamed.iter().map(|r| json!({
            "id": r.id, "before": r.before, "after": r.after,
        })).collect::<Vec<Value>>(),
        "ordered": definitions.ordered.iter().map(|o| json!({
            "parent": o.parent, "before": run(&o.before), "after": run(&o.after),
        })).collect::<Vec<Value>>(),
    })
}

/// [`Definitions`] from their stored form; `None` when it does not read.
#[must_use]
pub fn restored(value: &Value) -> Option<Definitions> {
    let text = |v: &Value, key: &str| v.get(key)?.as_str().map(str::to_owned);
    let run = |run: &Value| -> Option<Vec<(String, Option<i64>)>> {
        run.as_array()?
            .iter()
            .map(|item| {
                let seq = item.get("seq")?;
                let seq = if seq.is_null() {
                    None
                } else {
                    Some(seq.as_i64()?)
                };
                Some((text(item, "id")?, seq))
            })
            .collect()
    };
    Some(Definitions {
        created: value
            .get("created")?
            .as_array()?
            .iter()
            .map(|c| {
                Some(Created {
                    id: text(c, "id")?,
                    uuid: text(c, "uuid")?,
                    category: text(c, "category")?,
                    name: text(c, "name")?,
                    seq: c.get("seq")?.as_i64()?,
                })
            })
            .collect::<Option<_>>()?,
        renamed: value
            .get("renamed")?
            .as_array()?
            .iter()
            .map(|r| {
                Some(Renamed {
                    id: text(r, "id")?,
                    before: text(r, "before")?,
                    after: text(r, "after")?,
                })
            })
            .collect::<Option<_>>()?,
        ordered: value
            .get("ordered")?
            .as_array()?
            .iter()
            .map(|o| {
                Some(Ordered {
                    parent: text(o, "parent")?,
                    before: run(o.get("before")?)?,
                    after: run(o.get("after")?)?,
                })
            })
            .collect::<Option<_>>()?,
    })
}

/// The rows under the edit gate, compared with what the plan read.
pub(crate) fn unchanged_since(conn: &Connection, plan: &Plan) -> Result<bool, DbError> {
    Ok(rows(conn)? == plan.read)
}
