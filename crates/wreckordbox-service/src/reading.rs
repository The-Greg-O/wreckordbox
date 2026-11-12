//! Reading the DJ's vocabulary and what depends on it: the My Tag categories
//! and their tags, which tags a track carries, the playlist tree with its
//! intelligent playlists' rules, and play history.
//!
//! Every answer comes from what the window reads: the Library core's index
//! (`rbl_index`: track tags, playlist and history membership, intelligent
//! playlist rules and what they admit) and the Library core's reading of the
//! trees and of the My Tag catalogue (`rbl_library::read`), which the
//! window's rail and its scripting bridge use too.

use std::collections::{HashMap, HashSet};

use rbl_db::export_info::MyTagRow;
use rbl_index::smart::{self, my_tag_key, Item, Logic, Operator, Property, SmartRule};
use rbl_index::{Playlists, Row, SortColumn, TagFilter, TrackFilter, TrackSource, ViewSpec, NO_ID};
use rbl_library::read::{list_tree, my_tag_catalogue, ListEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::{track, OpenLibrary};
use crate::protocol::{codes as shared_codes, ServiceError, Track, DEFAULT_TRACK_LIMIT};
use crate::server::open_error;

/// The names of the commands this module answers.
pub mod commands {
    /// The My Tag categories and their tags in stored order, each tag with
    /// how many tracks carry it: answers a
    /// [`MyTagCategories`](super::MyTagCategories).
    pub const MYTAGS_LIST: &str = "mytags.list";
    /// The My Tags one track carries: takes an [`OfTrack`](super::OfTrack),
    /// answers a [`TrackMyTags`](super::TrackMyTags).
    pub const MYTAGS_OF_TRACK: &str = "mytags.of_track";
    /// The tracks carrying one My Tag: takes a [`ListQuery`](super::ListQuery)
    /// naming the tag, answers a [`TaggedTracks`](super::TaggedTracks).
    pub const MYTAGS_TRACKS: &str = "mytags.tracks";
    /// The playlist tree: folders, playlists and intelligent playlists with
    /// their rules. Answers a [`PlaylistTree`](super::PlaylistTree).
    pub const PLAYLISTS_TREE: &str = "playlists.tree";
    /// One playlist's tracks in its order: takes a
    /// [`ListQuery`](super::ListQuery), answers a
    /// [`PlaylistTracks`](super::PlaylistTracks).
    pub const PLAYLISTS_TRACKS: &str = "playlists.tracks";
    /// The occasions play history was recorded on: answers a
    /// [`HistoryList`](super::HistoryList).
    pub const HISTORY_LIST: &str = "history.list";
    /// One occasion's tracks in the order they were played: takes a
    /// [`ListQuery`](super::ListQuery), answers a
    /// [`HistoryTracks`](super::HistoryTracks).
    pub const HISTORY_TRACKS: &str = "history.tracks";
}

/// The error codes these commands add to [`crate::codes`].
pub mod codes {
    /// The Library has no My Tag with that identifier.
    pub const MY_TAG_NOT_FOUND: &str = "my_tag_not_found";
    /// The Library has no playlist or folder with that identifier.
    pub const PLAYLIST_NOT_FOUND: &str = "playlist_not_found";
    /// The Library has no history occasion with that identifier.
    pub const HISTORY_NOT_FOUND: &str = "history_not_found";
}

/// What `mytags.list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTagCategories {
    /// In the order rekordbox and the player's Track Filter show them.
    pub categories: Vec<MyTagCategory>,
}

/// One My Tag category and its tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTagCategory {
    pub id: String,
    pub name: String,
    /// In their stored order.
    pub tags: Vec<MyTagCount>,
}

/// One My Tag of a category and how much it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTagCount {
    pub id: String,
    pub name: String,
    /// How many tracks carry the tag.
    pub tracks: usize,
}

/// One My Tag, named with its category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTag {
    pub id: String,
    pub name: String,
    pub category_id: String,
    /// The category's name.
    pub category: String,
}

/// The arguments of `mytags.of_track`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfTrack {
    /// The track's identifier, as `tracks.list` gives it.
    pub id: String,
}

/// What `mytags.of_track` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMyTags {
    pub id: String,
    pub title: String,
    /// In the catalogue's order: by category, then by the tag's place in it.
    pub my_tags: Vec<MyTag>,
}

/// The arguments of the commands that list one thing's tracks: a My Tag's, a
/// playlist's or a history occasion's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListQuery {
    /// The identifier of the My Tag, playlist or occasion.
    pub id: String,
    /// The most tracks to return; [`DEFAULT_TRACK_LIMIT`] when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// What `mytags.tracks` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaggedTracks {
    pub my_tag: MyTag,
    /// In the collection's own order, as `tracks.list` gives them.
    pub tracks: Vec<Track>,
    /// How many tracks carry the tag, which is more than were returned when
    /// the limit cut the list short.
    pub total: usize,
}

/// What `playlists.tree` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistTree {
    /// Depth-first in the order the window's tree shows them: each folder
    /// followed by what is in it.
    pub playlists: Vec<PlaylistNode>,
}

/// What kind of entry of the playlist tree something is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistKind {
    Folder,
    Playlist,
    /// A playlist whose tracks are whatever its rule admits.
    Intelligent,
}

/// One entry of the playlist tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistNode {
    pub id: String,
    pub name: String,
    pub kind: PlaylistKind,
    /// The folder it is in; `null` at the top of the tree.
    pub parent_id: Option<String>,
    /// 1 at the top of the tree.
    pub depth: u32,
    /// How many tracks it lists: a playlist's own, an intelligent
    /// playlist's as its rule admits them now. `null` for a folder.
    pub tracks: Option<usize>,
    /// An intelligent playlist's rule. Left out for a folder and a
    /// playlist, and for an intelligent playlist whose stored rule does not
    /// read as one, which lists no tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
}

/// An intelligent playlist's rule, or a group nested in one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// `all`: every condition must hold. `any`: one is enough.
    #[serde(rename = "match")]
    pub logic: RuleLogic,
    pub conditions: Vec<RuleItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleLogic {
    All,
    Any,
}

/// One line of a rule: a condition (it has a `property`), or a nested group
/// (it has `match` and `conditions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuleItem {
    Condition(RuleCondition),
    Group(Rule),
}

/// One condition of a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleCondition {
    /// What it looks at: `artist`, `album`, `album_artist`,
    /// `original_artist`, `composer`, `remixer`, `mix_name`, `genre`,
    /// `label`, `key`, `title`, `comment`, `file_name`, `bpm`, `rating`,
    /// `color`, `play_count`, `duration`, `year`, `date_added`,
    /// `date_created`, `date_released`, `my_tag`, or `unsupported` for a
    /// property the Library's index does not hold, which never matches.
    pub property: String,
    /// `equal`, `not_equal`, `greater`, `less`, `in_range`, `in_last`,
    /// `not_in_last`, `contains`, `not_contains`, `starts_with` or
    /// `ends_with`.
    pub operator: String,
    /// What it compares with, as rekordbox stores it. For `my_tag`, the
    /// tag's identifier.
    pub value: String,
    /// The upper end of an `in_range`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value_to: String,
    /// The unit of an `in_last` count: `day`, `week`, `month` or `year`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// For `my_tag`, the tag the condition names, when the Library still
    /// has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_tag: Option<MyTag>,
}

/// A playlist, named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistRef {
    pub id: String,
    pub name: String,
    pub kind: PlaylistKind,
}

/// One track of an ordered list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListedTrack {
    /// Its place in the list, from 1.
    pub position: usize,
    /// In a playlist: the entry's own identifier, which stays the same when
    /// the playlist is reordered (a Mutant is made for it). Absent in a
    /// folder's list, an intelligent playlist's and play history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(flatten)]
    pub track: Track,
}

/// What `playlists.tracks` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistTracks {
    pub playlist: PlaylistRef,
    /// In the playlist's order. An intelligent playlist's are what its rule
    /// admits now; a folder's are those of every playlist in it, each track
    /// once, as the window lists a folder.
    pub tracks: Vec<ListedTrack>,
    /// How many tracks the list holds, which is more than were returned
    /// when the limit cut it short.
    pub total: usize,
}

/// What `history.list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryList {
    /// Oldest first, as the window's tree files them by year and month.
    pub occasions: Vec<Occasion>,
}

/// One occasion play history was recorded on: a gig, or a session at home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occasion {
    pub id: String,
    /// As rekordbox named it, for example `HISTORY 2026-09-01`.
    pub name: String,
    /// The year and month it is filed under, for example `2026/September`;
    /// empty when it is filed under none.
    pub folder: String,
    /// How many plays it holds; a track played twice counts twice.
    pub tracks: usize,
}

/// What `history.tracks` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryTracks {
    pub occasion: Occasion,
    /// In the order they were played; a track played twice is there twice.
    pub tracks: Vec<ListedTrack>,
    /// How many plays the occasion holds, which is more than were returned
    /// when the limit cut the list short.
    pub total: usize,
}

/// Whether `command` is one of [`commands`].
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::MYTAGS_LIST
            | commands::MYTAGS_OF_TRACK
            | commands::MYTAGS_TRACKS
            | commands::PLAYLISTS_TREE
            | commands::PLAYLISTS_TRACKS
            | commands::HISTORY_LIST
            | commands::HISTORY_TRACKS
    )
}

/// Answers one of [`commands`] from the open Library.
pub(crate) fn answer(
    library: &OpenLibrary,
    command: &str,
    args: &Value,
) -> Result<Value, ServiceError> {
    let reader = Reader::new(library)?;
    match command {
        commands::MYTAGS_LIST => to_value(&reader.my_tags()),
        commands::MYTAGS_OF_TRACK => to_value(&reader.my_tags_of(&arguments(command, args)?)?),
        commands::MYTAGS_TRACKS => to_value(&reader.tagged(&arguments(command, args)?)?),
        commands::PLAYLISTS_TREE => to_value(&reader.playlist_tree()),
        commands::PLAYLISTS_TRACKS => {
            let mut found = reader.playlist_tracks(&arguments(command, args)?)?;
            if found.playlist.kind == PlaylistKind::Playlist {
                crate::entries::attach(
                    library,
                    &reader.index,
                    &found.playlist.id,
                    &mut found.tracks,
                )?;
            }
            to_value(&found)
        }
        commands::HISTORY_LIST => to_value(&reader.occasions()),
        commands::HISTORY_TRACKS => to_value(&reader.history_tracks(&arguments(command, args)?)?),
        unknown => Err(ServiceError::new(
            shared_codes::UNKNOWN_COMMAND,
            format!("unknown command: {unknown}"),
        )),
    }
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

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(shared_codes::SERVICE_FAILED, e.to_string()))
}

/// One snapshot of the index and the My Tag catalogue, to answer one command
/// from.
struct Reader {
    index: std::sync::Arc<rbl_index::Library>,
    /// Every live My Tag row, as the window's scripting bridge reads them.
    /// The index knows the tags by name and number only; the categories'
    /// identifiers are here.
    my_tag_rows: Vec<MyTagRow>,
}

impl Reader {
    fn new(library: &OpenLibrary) -> Result<Self, ServiceError> {
        Ok(Self {
            index: library.index()?,
            my_tag_rows: library.with_db(|db| {
                rbl_db::export_info::my_tags(db.connection()).map_err(|e| open_error(&e))
            })?,
        })
    }

    /// Every tag with its category, in the catalogue's order.
    fn catalogue(&self) -> Vec<MyTag> {
        let catalogue = my_tag_catalogue(&self.my_tag_rows);
        catalogue
            .categories
            .iter()
            .flat_map(|category| {
                catalogue.tags_of(category).map(|tag| MyTag {
                    id: tag.id.clone(),
                    name: tag.name.clone(),
                    category_id: category.id.clone(),
                    category: category.name.clone(),
                })
            })
            .collect()
    }

    fn my_tags(&self) -> MyTagCategories {
        // One pass over the tracks' tags, as the index holds them.
        let mut carried: HashMap<i32, usize> = HashMap::new();
        for row in 0..self.index.len() {
            for &key in self.index.my_tag_keys(row) {
                *carried.entry(key).or_default() += 1;
            }
        }
        let catalogue = my_tag_catalogue(&self.my_tag_rows);
        let categories = catalogue
            .categories
            .iter()
            .map(|category| MyTagCategory {
                id: category.id.clone(),
                name: category.name.clone(),
                tags: catalogue
                    .tags_of(category)
                    .map(|tag| MyTagCount {
                        id: tag.id.clone(),
                        name: tag.name.clone(),
                        tracks: carried.get(&my_tag_key(&tag.id)).copied().unwrap_or(0),
                    })
                    .collect(),
            })
            .collect();
        MyTagCategories { categories }
    }

    fn my_tags_of(&self, query: &OfTrack) -> Result<TrackMyTags, ServiceError> {
        let Some(row) = self.index.row_of(&query.id) else {
            return Err(ServiceError::new(
                shared_codes::TRACK_NOT_FOUND,
                format!("there is no track {} in the Library", query.id),
            ));
        };
        let carried: HashSet<i32> = self
            .index
            .my_tag_keys(row as usize)
            .iter()
            .copied()
            .collect();
        let found = track(&self.index, row);
        Ok(TrackMyTags {
            id: found.id,
            title: found.title,
            my_tags: self
                .catalogue()
                .into_iter()
                .filter(|tag| carried.contains(&my_tag_key(&tag.id)))
                .collect(),
        })
    }

    fn tagged(&self, query: &ListQuery) -> Result<TaggedTracks, ServiceError> {
        let Some(my_tag) = self.catalogue().into_iter().find(|tag| tag.id == query.id) else {
            return Err(ServiceError::new(
                codes::MY_TAG_NOT_FOUND,
                format!("there is no MyTag {} in the Library", query.id),
            ));
        };
        // The window's Track Filter with this one tag ticked.
        let view = self.index.open_view(&ViewSpec {
            source: TrackSource::Collection,
            sort: SortColumn::TrackNo,
            descending: false,
            query: String::new(),
            filter: TrackFilter {
                tags: vec![TagFilter {
                    picks: vec![vec![my_tag_key(&my_tag.id)]],
                    any: false,
                }],
                ..TrackFilter::default()
            },
        });
        let tracks = view
            .window(0, limit_of(query))
            .iter()
            .map(|&row| track(&self.index, row))
            .collect();
        Ok(TaggedTracks {
            my_tag,
            tracks,
            total: view.len(),
        })
    }

    fn playlist_tree(&self) -> PlaylistTree {
        let playlists = self.index.playlists();
        let catalogue = self.catalogue();
        let nodes = list_tree(&playlists, false)
            .into_iter()
            .map(|entry| {
                let kind = kind_of(&entry);
                let (tracks, rule) = match kind {
                    PlaylistKind::Folder => (None, None),
                    PlaylistKind::Playlist => (Some(entry.members), None),
                    PlaylistKind::Intelligent => (
                        Some(
                            self.index
                                .source_rows_unlocked(
                                    &playlists,
                                    &TrackSource::SmartPlaylist(entry.index),
                                )
                                .len(),
                        ),
                        playlists
                            .smart_rule(entry.index)
                            .map(|rule| structured(&rule, &catalogue)),
                    ),
                };
                PlaylistNode {
                    id: id_of(&playlists, entry.index),
                    parent_id: parent_of(&playlists, entry.index),
                    name: entry.name,
                    kind,
                    depth: entry.depth,
                    tracks,
                    rule,
                }
            })
            .collect();
        PlaylistTree { playlists: nodes }
    }

    fn playlist_tracks(&self, query: &ListQuery) -> Result<PlaylistTracks, ServiceError> {
        let playlists = self.index.playlists();
        let found = query.id.parse().ok().and_then(|id| playlists.index_of(id));
        let Some(index) = found else {
            return Err(ServiceError::new(
                codes::PLAYLIST_NOT_FOUND,
                format!("there is no playlist {} in the Library", query.id),
            ));
        };
        // The sources the window opens a list of each kind from.
        let (kind, source) = if playlists.is_folder(index) {
            (PlaylistKind::Folder, TrackSource::PlaylistFolder(index))
        } else if playlists.is_smart(index) {
            (PlaylistKind::Intelligent, TrackSource::SmartPlaylist(index))
        } else {
            (PlaylistKind::Playlist, TrackSource::Playlist(index))
        };
        let rows = self.index.source_rows_unlocked(&playlists, &source);
        Ok(PlaylistTracks {
            playlist: PlaylistRef {
                id: id_of(&playlists, index),
                name: playlists.name(index).to_owned(),
                kind,
            },
            tracks: self.listed(&rows, limit_of(query)),
            total: rows.len(),
        })
    }

    /// The history sessions, each with the folders it is filed under.
    fn occasions(&self) -> HistoryList {
        let histories = self.index.histories();
        let mut path: Vec<String> = Vec::new();
        let mut occasions = Vec::new();
        for entry in list_tree(&histories, true) {
            path.truncate(entry.depth.saturating_sub(1) as usize);
            if entry.folder {
                path.push(entry.name);
            } else {
                occasions.push(Occasion {
                    id: id_of(&histories, entry.index),
                    name: entry.name,
                    folder: path.join("/"),
                    tracks: entry.members,
                });
            }
        }
        HistoryList { occasions }
    }

    fn history_tracks(&self, query: &ListQuery) -> Result<HistoryTracks, ServiceError> {
        let Some(occasion) = self
            .occasions()
            .occasions
            .into_iter()
            .find(|occasion| occasion.id == query.id)
        else {
            return Err(ServiceError::new(
                codes::HISTORY_NOT_FOUND,
                format!("there is no history occasion {} in the Library", query.id),
            ));
        };
        let index = occasion
            .id
            .parse()
            .ok()
            .and_then(|id| self.index.histories().index_of(id));
        let rows = index.map_or_else(Vec::new, |index| {
            self.index.source_rows(&TrackSource::History(index))
        });
        Ok(HistoryTracks {
            occasion,
            tracks: self.listed(&rows, limit_of(query)),
            total: rows.len(),
        })
    }

    /// The first `limit` of `rows`, numbered from 1.
    fn listed(&self, rows: &[Row], limit: usize) -> Vec<ListedTrack> {
        rows.iter()
            .take(limit)
            .enumerate()
            .map(|(at, &row)| ListedTrack {
                position: at + 1,
                entry_id: None,
                track: track(&self.index, row),
            })
            .collect()
    }
}

fn limit_of(query: &ListQuery) -> usize {
    query.limit.unwrap_or(DEFAULT_TRACK_LIMIT)
}

fn kind_of(entry: &ListEntry) -> PlaylistKind {
    if entry.folder {
        PlaylistKind::Folder
    } else if entry.smart {
        PlaylistKind::Intelligent
    } else {
        PlaylistKind::Playlist
    }
}

fn id_of(lists: &Playlists, index: usize) -> String {
    lists.ids.get(index).copied().unwrap_or(0).to_string()
}

fn parent_of(lists: &Playlists, index: usize) -> Option<String> {
    let parent = lists.parent.get(index).copied().unwrap_or(NO_ID);
    (parent != NO_ID && (parent as usize) < lists.len()).then(|| id_of(lists, parent as usize))
}

/// A parsed rule in the shape clients are sent.
fn structured(rule: &SmartRule, catalogue: &[MyTag]) -> Rule {
    group(&rule.root, catalogue)
}

fn group(group_of: &smart::Group, catalogue: &[MyTag]) -> Rule {
    Rule {
        logic: match group_of.logic {
            Logic::All => RuleLogic::All,
            Logic::Any => RuleLogic::Any,
        },
        conditions: group_of
            .items
            .iter()
            .map(|item| match item {
                Item::Condition(condition) => RuleItem::Condition(RuleCondition {
                    property: property_name(condition.property).to_owned(),
                    operator: operator_name(condition.operator).to_owned(),
                    value: condition.left.clone(),
                    value_to: condition.right.clone(),
                    unit: condition.unit.clone(),
                    my_tag: (condition.property == Property::MyTag)
                        .then(|| {
                            // Matched as the rule itself is: by the tag's
                            // number.
                            let wanted = my_tag_key(&condition.left);
                            catalogue
                                .iter()
                                .find(|tag| my_tag_key(&tag.id) == wanted)
                                .cloned()
                        })
                        .flatten(),
                }),
                Item::Group(nested) => RuleItem::Group(group(nested, catalogue)),
            })
            .collect(),
    }
}

fn property_name(property: Property) -> &'static str {
    match property {
        Property::Artist => "artist",
        Property::Album => "album",
        Property::AlbumArtist => "album_artist",
        Property::OriginalArtist => "original_artist",
        Property::Composer => "composer",
        Property::Remixer => "remixer",
        Property::MixName => "mix_name",
        Property::Genre => "genre",
        Property::Label => "label",
        Property::Key => "key",
        Property::Title => "title",
        Property::Comment => "comment",
        Property::FileName => "file_name",
        Property::Bpm => "bpm",
        Property::Rating => "rating",
        Property::Color => "color",
        Property::PlayCount => "play_count",
        Property::Duration => "duration",
        Property::Year => "year",
        Property::DateAdded => "date_added",
        Property::DateCreated => "date_created",
        Property::DateReleased => "date_released",
        Property::MyTag => "my_tag",
        Property::Unsupported => "unsupported",
    }
}

fn operator_name(operator: Operator) -> &'static str {
    match operator {
        Operator::Equal => "equal",
        Operator::NotEqual => "not_equal",
        Operator::Greater => "greater",
        Operator::Less => "less",
        Operator::InRange => "in_range",
        Operator::InLast => "in_last",
        Operator::NotInLast => "not_in_last",
        Operator::Contains => "contains",
        Operator::NotContains => "not_contains",
        Operator::StartsWith => "starts_with",
        Operator::EndsWith => "ends_with",
    }
}
