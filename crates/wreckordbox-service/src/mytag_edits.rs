//! Editing the DJ's vocabulary directly: creating, renaming, reordering,
//! merging and deleting My Tags within the My Tag categories, and putting
//! them on tracks or taking them off.
//!
//! Every edit takes the Library's single edit path
//! (`rbl_library::LibraryCore::edit`) and makes the writes the window makes
//! for the same edit (`rbl_db::write::Writer`'s My Tag functions and
//! `rbl_library::my_tags`), so each one refreshes the index and tells every
//! client. A request is checked in full before anything is written: one
//! that names something the Library does not have changes nothing.

use rbl_db::export_info::MyTagRow;
use rbl_db::write::MY_TAG_LIMIT;
use rbl_index::smart::my_tag_key;
use rbl_library::read::my_tag_catalogue;
use rbl_library::{my_tags, Touched};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::{track, OpenLibrary};
use crate::protocol::{codes as shared_codes, LibraryChanged, ServiceError};
use crate::reading::{codes as reading_codes, MyTag};
use crate::server::open_error;

/// The names of the commands this module answers.
pub mod commands {
    /// A new My Tag at the end of a category: takes a
    /// [`Create`](super::Create), answers a [`Created`](super::Created).
    pub const MYTAGS_CREATE: &str = "mytags.create";
    /// Another name for a My Tag or for a category: takes a
    /// [`Rename`](super::Rename), answers a [`Renamed`](super::Renamed).
    pub const MYTAGS_RENAME: &str = "mytags.rename";
    /// One My Tag (or one category) to another place among its siblings:
    /// takes a [`Move`](super::Move), answers a
    /// [`Reordered`](super::Reordered).
    pub const MYTAGS_MOVE: &str = "mytags.move";
    /// Every tag of one category, or every category, in a new order: takes
    /// a [`Reorder`](super::Reorder), answers a
    /// [`Reordered`](super::Reordered).
    pub const MYTAGS_REORDER: &str = "mytags.reorder";
    /// Every track of one My Tag moved to another, and the first removed:
    /// takes a [`Merge`](super::Merge), answers a [`Merged`](super::Merged).
    pub const MYTAGS_MERGE: &str = "mytags.merge";
    /// A My Tag removed from its category and from every track: takes a
    /// [`Delete`](super::Delete), answers a [`Deleted`](super::Deleted).
    pub const MYTAGS_DELETE: &str = "mytags.delete";
    /// A My Tag put on one or many tracks: takes an
    /// [`Assign`](super::Assign), answers an [`Assigned`](super::Assigned).
    pub const MYTAGS_ASSIGN: &str = "mytags.assign";
    /// A My Tag taken off one or many tracks: takes an
    /// [`Assign`](super::Assign), answers an [`Assigned`](super::Assigned).
    pub const MYTAGS_UNASSIGN: &str = "mytags.unassign";
    /// A new My Tag category: takes a
    /// [`CreateCategory`](super::CreateCategory) and always refuses with
    /// [`MY_TAG_CATEGORY_LIMIT`](super::codes::MY_TAG_CATEGORY_LIMIT),
    /// because rekordbox fixes the categories. Rename one instead.
    pub const MYTAGS_CREATE_CATEGORY: &str = "mytags.create_category";
}

/// The error codes these commands add to [`crate::codes`]. An unknown tag is
/// [`crate::reading::codes::MY_TAG_NOT_FOUND`], an unknown track
/// [`crate::codes::TRACK_NOT_FOUND`].
pub mod codes {
    /// The Library has no My Tag category with that identifier.
    pub const MY_TAG_CATEGORY_NOT_FOUND: &str = "my_tag_category_not_found";
    /// The category already has a tag, or the Library a category, of that
    /// name.
    pub const MY_TAG_NAME_TAKEN: &str = "my_tag_name_taken";
    /// A My Tag or a category cannot have an empty name.
    pub const INVALID_MY_TAG_NAME: &str = "invalid_my_tag_name";
    /// The category already holds as many tags as rekordbox allows.
    pub const MY_TAG_LIMIT: &str = "my_tag_limit";
    /// A new order must name every tag of one category, or every category,
    /// once each and nothing else.
    pub const INVALID_MY_TAG_ORDER: &str = "invalid_my_tag_order";
    /// rekordbox fixes the My Tag categories at four: none can be made.
    pub const MY_TAG_CATEGORY_LIMIT: &str = "my_tag_category_limit";
    /// A My Tag cannot be merged into itself.
    pub const MY_TAG_MERGE_INTO_ITSELF: &str = "my_tag_merge_into_itself";
}

/// The arguments of `mytags.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Create {
    /// The category's identifier, as `mytags.list` gives it.
    pub category: String,
    pub name: String,
}

/// What `mytags.create` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Created {
    pub my_tag: MyTag,
    /// The Library's generation after the edit: the one the change event
    /// carries.
    pub generation: u32,
}

/// The arguments of `mytags.create_category`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCategory {
    pub name: String,
}

/// The arguments of `mytags.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    /// The identifier of a My Tag or of a category.
    pub id: String,
    pub name: String,
    /// Whether rekordbox's Add "My Tag" to the "Comments" is on, in which
    /// case the comments of the tracks concerned are rewritten with their
    /// tags, as the window rewrites them. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub comments: bool,
}

/// Whether an identifier named a My Tag or a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Tag,
    Category,
}

/// What `mytags.rename` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renamed {
    pub id: String,
    pub name: String,
    pub previous_name: String,
    pub kind: Kind,
    pub generation: u32,
}

/// The arguments of `mytags.move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    /// The identifier of a My Tag or of a category.
    pub id: String,
    /// Its new place among its siblings, from 1. A place past the end is
    /// the end.
    pub position: usize,
}

/// The arguments of `mytags.reorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reorder {
    /// Every tag of one category, or every category, in the order wanted.
    pub order: Vec<String>,
}

/// What `mytags.move` and `mytags.reorder` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reordered {
    /// The category whose tags were put in order; `null` when it was the
    /// categories themselves.
    pub category: Option<String>,
    /// The identifier of that category; `null` likewise.
    pub category_id: Option<String>,
    /// The siblings as they now stand.
    pub order: Vec<Placed>,
    pub generation: u32,
}

/// One My Tag or category in an order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub id: String,
    pub name: String,
}

/// The arguments of `mytags.merge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merge {
    /// The My Tag to remove, once its tracks carry `into`.
    pub from: String,
    /// The My Tag that takes its tracks, in any category.
    pub into: String,
    /// Whether rekordbox's Add "My Tag" to the "Comments" is on, in which
    /// case the comments of the tracks concerned are rewritten with their
    /// tags, as the window rewrites them. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub comments: bool,
}

/// What `mytags.merge` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    /// The tag that is gone.
    pub from: MyTag,
    pub into: MyTag,
    /// How many tracks carried `from`, each of which now carries `into`.
    pub tracks: usize,
    pub generation: u32,
}

/// The arguments of `mytags.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delete {
    pub id: String,
    /// Whether rekordbox's Add "My Tag" to the "Comments" is on, in which
    /// case the comments of the tracks concerned are rewritten with their
    /// tags, as the window rewrites them. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub comments: bool,
}

/// What `mytags.delete` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deleted {
    /// The tag that is gone.
    pub my_tag: MyTag,
    /// How many tracks carried it.
    pub tracks: usize,
    pub generation: u32,
}

/// The arguments of `mytags.assign` and `mytags.unassign`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assign {
    /// The My Tag's identifier.
    pub id: String,
    /// The identifiers of the tracks, as `tracks.list` gives them. At least
    /// one; a track named twice counts once.
    pub tracks: Vec<String>,
    /// Whether rekordbox's Add "My Tag" to the "Comments" is on, in which
    /// case the comments of the tracks concerned are rewritten with their
    /// tags, as the window rewrites them. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub comments: bool,
}

/// What `mytags.assign` and `mytags.unassign` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assigned {
    pub my_tag: MyTag,
    /// Each track named, in the order named.
    pub tracks: Vec<AssignedTrack>,
    /// How many of them changed: the rest already carried the tag, or
    /// already lacked it.
    pub changed: usize,
    pub generation: u32,
}

/// One track of an [`Assigned`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignedTrack {
    pub id: String,
    pub title: String,
    /// `false` when the track already carried the tag (or, taking it off,
    /// already lacked it).
    pub changed: bool,
}

/// Whether `command` is one of [`commands`].
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::MYTAGS_CREATE
            | commands::MYTAGS_RENAME
            | commands::MYTAGS_MOVE
            | commands::MYTAGS_REORDER
            | commands::MYTAGS_MERGE
            | commands::MYTAGS_DELETE
            | commands::MYTAGS_ASSIGN
            | commands::MYTAGS_UNASSIGN
            | commands::MYTAGS_CREATE_CATEGORY
    )
}

/// Makes the edit `command` names, and answers what to tell the caller and
/// what to tell every client. An error means nothing changed.
pub(crate) fn answer(
    library: &OpenLibrary,
    command: &str,
    args: &Value,
) -> Result<(Value, LibraryChanged), ServiceError> {
    let vocabulary = Vocabulary::read(library)?;
    match command {
        commands::MYTAGS_CREATE => {
            answered(create(library, &vocabulary, &arguments(command, args)?)?)
        }
        commands::MYTAGS_CREATE_CATEGORY => {
            let request: CreateCategory = arguments(command, args)?;
            let have = vocabulary.tags_in(ROOT).count();
            Err(ServiceError::new(
                codes::MY_TAG_CATEGORY_LIMIT,
                format!(
                    "the MyTag category \"{}\" cannot be made: rekordbox fixes the categories at four and this Library has {have}; rename one instead",
                    request.name
                ),
            ))
        }
        commands::MYTAGS_RENAME => {
            answered(rename(library, &vocabulary, &arguments(command, args)?)?)
        }
        commands::MYTAGS_MOVE => {
            let request: Move = arguments(command, args)?;
            let moved = vocabulary.row(&request.id)?;
            let mut order: Vec<String> = vocabulary
                .tags_in(&moved.parent)
                .filter(|row| row.id != moved.id)
                .map(|row| row.id.clone())
                .collect();
            let place = request.position.saturating_sub(1).min(order.len());
            order.insert(place, moved.id.clone());
            answered(reorder(library, &vocabulary, &order)?)
        }
        commands::MYTAGS_REORDER => {
            let request: Reorder = arguments(command, args)?;
            answered(reorder(library, &vocabulary, &request.order)?)
        }
        commands::MYTAGS_MERGE => {
            answered(merge(library, &vocabulary, &arguments(command, args)?)?)
        }
        commands::MYTAGS_DELETE => {
            answered(delete(library, &vocabulary, &arguments(command, args)?)?)
        }
        commands::MYTAGS_ASSIGN => answered(assign(
            library,
            &vocabulary,
            &arguments(command, args)?,
            true,
        )?),
        commands::MYTAGS_UNASSIGN => answered(assign(
            library,
            &vocabulary,
            &arguments(command, args)?,
            false,
        )?),
        unknown => Err(ServiceError::new(
            shared_codes::UNKNOWN_COMMAND,
            format!("unknown command: {unknown}"),
        )),
    }
}

fn create(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    request: &Create,
) -> Result<(Created, LibraryChanged), ServiceError> {
    let category = vocabulary.category(&request.category)?;
    vocabulary.name_is_free(&category.id, &request.name, None)?;
    if vocabulary.tags_in(&category.id).count() >= MY_TAG_LIMIT {
        return Err(ServiceError::new(
            codes::MY_TAG_LIMIT,
            format!(
                "the category {} already holds {MY_TAG_LIMIT} MyTags, the most rekordbox allows",
                category.name
            ),
        ));
    }
    let (parent, name) = (category.id.clone(), request.name.clone());
    // As the window does it: rekordbox's Adding My Tag makes the tag under
    // a stock name, and the name the DJ types is a rename.
    let (id, changed) = library.edit(Touched::Tracks, move |writer| {
        let made = writer.create_my_tag(&parent)?;
        if let Err(error) = writer.rename_my_tag(&made.id, &name) {
            // Nothing more can be done if this fails too; the rename's
            // failure is the one to report.
            let _ = writer.delete_my_tag(&made.id);
            return Err(error);
        }
        Ok(made.id)
    })?;
    let created = Created {
        my_tag: MyTag {
            id,
            name: request.name.clone(),
            category_id: category.id.clone(),
            category: category.name.clone(),
        },
        generation: changed.generation,
    };
    Ok((created, changed))
}

fn rename(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    request: &Rename,
) -> Result<(Renamed, LibraryChanged), ServiceError> {
    let row = vocabulary.row(&request.id)?;
    vocabulary.name_is_free(&row.parent, &request.name, Some(&row.id))?;
    // The comments hold tag names, so a category's new name changes none.
    let rewritten = if request.comments && kind_of(row) == Kind::Tag {
        carriers(library, &row.id)?
    } else {
        Vec::new()
    };
    let (id, name) = (row.id.clone(), request.name.clone());
    let ((), changed) = library.edit(Touched::Tracks, move |writer| {
        writer.rename_my_tag(&id, &name)?;
        my_tags::rewrite_comments(writer, &rewritten)
    })?;
    let renamed = Renamed {
        id: row.id.clone(),
        name: request.name.clone(),
        previous_name: row.name.clone(),
        kind: kind_of(row),
        generation: changed.generation,
    };
    Ok((renamed, changed))
}

/// Puts one run of siblings in `order`, which must be exactly that run.
fn reorder(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    order: &[String],
) -> Result<(Reordered, LibraryChanged), ServiceError> {
    let invalid = |why: String| ServiceError::new(codes::INVALID_MY_TAG_ORDER, why);
    let rows: Vec<&MyTagRow> = order
        .iter()
        .map(|id| vocabulary.row(id))
        .collect::<Result<_, _>>()?;
    let Some(first) = rows.first() else {
        return Err(invalid("name the MyTags to put in order".to_owned()));
    };
    let what = if first.parent == ROOT {
        "every MyTag category"
    } else {
        "every MyTag of the category"
    };
    for (place, row) in rows.iter().enumerate() {
        if row.parent != first.parent {
            return Err(invalid(format!(
                "MyTag {} is not beside MyTag {}: name {what}, and only those",
                row.id, first.id
            )));
        }
        if rows.iter().take(place).any(|earlier| earlier.id == row.id) {
            return Err(invalid(format!("MyTag {} is named more than once", row.id)));
        }
    }
    if let Some(missing) = vocabulary
        .tags_in(&first.parent)
        .find(|sibling| !rows.iter().any(|row| row.id == sibling.id))
    {
        return Err(invalid(format!(
            "the order leaves out MyTag {} \"{}\": name {what}",
            missing.id, missing.name
        )));
    }
    let wanted = order.to_vec();
    // The write the window's scripting bridge makes for the same edit.
    let (_recorded, changed) = library.edit(Touched::Tracks, move |writer| {
        writer.reorder_my_tags_with_undo(&wanted)
    })?;
    let category = vocabulary.rows.iter().find(|row| row.id == first.parent);
    let reordered = Reordered {
        category: category.map(|row| row.name.clone()),
        category_id: category.map(|row| row.id.clone()),
        order: rows
            .iter()
            .map(|row| Placed {
                id: row.id.clone(),
                name: row.name.clone(),
            })
            .collect(),
        generation: changed.generation,
    };
    Ok((reordered, changed))
}

/// Merging has no equivalent in the window, so it is built from the writes
/// the window does make, as one edit: the tick that puts `into` on every
/// track carrying `from`, then the delete of `from`, which takes it off
/// them.
fn merge(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    request: &Merge,
) -> Result<(Merged, LibraryChanged), ServiceError> {
    let from = vocabulary.tag(&request.from)?;
    let into = vocabulary.tag(&request.into)?;
    if from.id == into.id {
        return Err(ServiceError::new(
            codes::MY_TAG_MERGE_INTO_ITSELF,
            format!("MyTag {} cannot be merged into itself", from.id),
        ));
    }
    let carriers = carriers(library, &from.id)?;
    let (tracks, gone, kept) = (carriers.clone(), from.id.clone(), into.id.clone());
    let comments = request.comments;
    let ((), changed) = library.edit(Touched::Tracks, move |writer| {
        let moved = my_tags::put_on_tracks(writer, &tracks, &kept, true)?;
        if let Err(error) = writer.delete_my_tag(&gone) {
            my_tags::undo_tag_edits(writer, &moved);
            return Err(error);
        }
        if comments {
            my_tags::rewrite_comments(writer, &tracks)?;
        }
        Ok(())
    })?;
    let merged = Merged {
        from,
        into,
        tracks: carriers.len(),
        generation: changed.generation,
    };
    Ok((merged, changed))
}

fn delete(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    request: &Delete,
) -> Result<(Deleted, LibraryChanged), ServiceError> {
    let my_tag = vocabulary.tag(&request.id)?;
    let carriers = carriers(library, &my_tag.id)?;
    let id = my_tag.id.clone();
    let rewritten = if request.comments {
        carriers.clone()
    } else {
        Vec::new()
    };
    let ((), changed) = library.edit(Touched::Tracks, move |writer| {
        writer.delete_my_tag(&id)?;
        my_tags::rewrite_comments(writer, &rewritten)
    })?;
    let deleted = Deleted {
        my_tag,
        tracks: carriers.len(),
        generation: changed.generation,
    };
    Ok((deleted, changed))
}

/// The tick of the window's My Tag panel: the tag goes on each track
/// (`on`), or comes off.
fn assign(
    library: &OpenLibrary,
    vocabulary: &Vocabulary,
    request: &Assign,
    on: bool,
) -> Result<(Assigned, LibraryChanged), ServiceError> {
    let my_tag = vocabulary.tag(&request.id)?;
    if request.tracks.is_empty() {
        return Err(ServiceError::new(
            shared_codes::BAD_REQUEST,
            "name at least one track",
        ));
    }
    let index = library.index()?;
    let key = my_tag_key(&my_tag.id);
    let mut tracks: Vec<AssignedTrack> = Vec::with_capacity(request.tracks.len());
    for id in &request.tracks {
        let Some(row) = index.row_of(id) else {
            return Err(ServiceError::new(
                shared_codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ));
        };
        let found = track(&index, row);
        if tracks.iter().any(|named| named.id == found.id) {
            continue;
        }
        let carried = index.my_tag_keys(row as usize).contains(&key);
        tracks.push(AssignedTrack {
            id: found.id,
            title: found.title,
            changed: carried != on,
        });
    }
    let ids: Vec<String> = tracks.iter().map(|named| named.id.clone()).collect();
    let tag = my_tag.id.clone();
    let comments = request.comments;
    let (_recorded, changed) = library.edit(Touched::Tracks, move |writer| {
        let tags = my_tags::put_on_tracks(writer, &ids, &tag, on)?;
        let comments = if comments {
            my_tags::refresh_comments(writer, &ids, &tags)?
        } else {
            Vec::new()
        };
        Ok((tags, comments))
    })?;
    let assigned = Assigned {
        my_tag,
        changed: tracks.iter().filter(|named| named.changed).count(),
        tracks,
        generation: changed.generation,
    };
    Ok((assigned, changed))
}

/// The identifiers of the tracks that carry the My Tag `id`, in the
/// collection's order.
fn carriers(library: &OpenLibrary, id: &str) -> Result<Vec<String>, ServiceError> {
    let index = library.index()?;
    let key = my_tag_key(id);
    Ok((0..index.len())
        .filter(|&row| index.my_tag_keys(row).contains(&key))
        .filter_map(|row| index.ids.get(row).map(u64::to_string))
        .collect())
}

fn kind_of(row: &MyTagRow) -> Kind {
    if row.parent == ROOT {
        Kind::Category
    } else {
        Kind::Tag
    }
}

/// What a category has for a parent.
const ROOT: &str = "root";

/// The My Tag categories and tags as the Library holds them now, read from
/// the database and not from the index, which keeps no category
/// identifiers.
struct Vocabulary {
    rows: Vec<MyTagRow>,
}

impl Vocabulary {
    fn read(library: &OpenLibrary) -> Result<Self, ServiceError> {
        Ok(Self {
            rows: library.with_db(|db| {
                rbl_db::export_info::my_tags(db.connection()).map_err(|e| open_error(&e))
            })?,
        })
    }

    /// The category with this identifier.
    fn category(&self, id: &str) -> Result<&MyTagRow, ServiceError> {
        my_tag_catalogue(&self.rows)
            .categories
            .into_iter()
            .find(|category| category.id == id)
            .ok_or_else(|| {
                ServiceError::new(
                    codes::MY_TAG_CATEGORY_NOT_FOUND,
                    format!("there is no MyTag category {id} in the Library"),
                )
            })
    }

    /// The My Tag or the category with this identifier. A tag left behind
    /// in a category that is gone is not one the Library shows.
    fn row(&self, id: &str) -> Result<&MyTagRow, ServiceError> {
        let catalogue = my_tag_catalogue(&self.rows);
        let found = catalogue.categories.iter().find_map(|category| {
            if category.id == id {
                return Some(*category);
            }
            catalogue.tags_of(category).find(|tag| tag.id == id)
        });
        found.ok_or_else(|| {
            ServiceError::new(
                reading_codes::MY_TAG_NOT_FOUND,
                format!("there is no MyTag {id} in the Library"),
            )
        })
    }

    /// The My Tag with this identifier, named with its category. A
    /// category is not a tag.
    fn tag(&self, id: &str) -> Result<MyTag, ServiceError> {
        let row = self.row(id)?;
        let category = self.rows.iter().find(|parent| parent.id == row.parent);
        match category {
            Some(category) if row.parent != ROOT => Ok(MyTag {
                id: row.id.clone(),
                name: row.name.clone(),
                category_id: category.id.clone(),
                category: category.name.clone(),
            }),
            _ => Err(ServiceError::new(
                reading_codes::MY_TAG_NOT_FOUND,
                format!("{id} is a MyTag category, not a MyTag"),
            )),
        }
    }

    /// The tags of the category `parent`, or the categories when it is
    /// `root`, in order.
    fn tags_in<'a>(&'a self, parent: &'a str) -> impl Iterator<Item = &'a MyTagRow> + 'a {
        self.rows.iter().filter(move |row| row.parent == parent)
    }

    /// Refuses `name` when it is empty or another row under `parent` than
    /// `except` already has it, compared exactly as rekordbox compares.
    fn name_is_free(
        &self,
        parent: &str,
        name: &str,
        except: Option<&str>,
    ) -> Result<(), ServiceError> {
        if name.is_empty() {
            return Err(ServiceError::new(
                codes::INVALID_MY_TAG_NAME,
                "a MyTag needs a name",
            ));
        }
        let taken = self
            .tags_in(parent)
            .any(|row| row.name == name && Some(row.id.as_str()) != except);
        if taken {
            return Err(ServiceError::new(
                codes::MY_TAG_NAME_TAKEN,
                format!("there is already a MyTag called \"{name}\" there"),
            ));
        }
        Ok(())
    }
}

fn answered<T: Serialize>(
    (result, changed): (T, LibraryChanged),
) -> Result<(Value, LibraryChanged), ServiceError> {
    let value = serde_json::to_value(&result)
        .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()))?;
    Ok((value, changed))
}

fn arguments<T: serde::de::DeserializeOwned>(
    command: &str,
    args: &Value,
) -> Result<T, ServiceError> {
    serde_json::from_value(args.clone()).map_err(|e| {
        ServiceError::new(
            shared_codes::BAD_REQUEST,
            format!("bad arguments to {command}: {e}"),
        )
    })
}
