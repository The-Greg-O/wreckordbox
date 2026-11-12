//! The Ledger (ADR 0004) as the service keeps it: every edit to the
//! Library, whoever made it, written to the Sidecar with its source, its
//! Run and its exact before and after values; and the `runs.list` and
//! `runs.show` commands that read it back.
//!
//! # Where it attaches
//!
//! The service gives the Library core's edit history a [`Keeper`]
//! (`rbl_library::ledger::Ledger`). The history's one recording point
//! hands it every recorded edit and every Undo and Redo with the payload
//! the history keeps; the core's write path names the edits the history
//! does not keep, and asks before each write whether a change could be
//! recorded. With a window hosting the service that is the window's core,
//! so the window's own edits are recorded by the same code.
//!
//! # Who made it, and in which Run
//!
//! - A client's command: the source and Run its request names
//!   ([`RunArg`], the request's `run`). With no Run named, the command is a
//!   Run of its own, made when its first change is recorded and ended with
//!   the command. With no source named, the source is
//!   `tool:`[`UNNAMED_SOURCE`], never the DJ.
//! - Anything else (an edit in the window, a command the window itself
//!   asks of the service it hosts): the DJ, in the window's Run
//!   ([`WINDOW_RUN`]). That Run starts with the first such edit and ends
//!   when the DJ has made none for [`WINDOW_PAUSE`], or when the service
//!   stops; the next edit starts another. An Undo or Redo in the window is
//!   an entry of the window's Run, whoever made the edit it steps.
//!
//! # What an entry holds
//!
//! [`stored`] is the stable, versioned form of the history's payloads.
//! An entry whose payload is stored is `reversible`: [`stored::restore`]
//! rebuilds the history's own edit from it, to check and to invert.
//! Every other kind of edit is named (its `kind` and `label`), with no
//! payload.
//!
//! # When something fails
//!
//! The Library and the Sidecar have no transaction in common, so the
//! Ledger is written around the Library write, never inside it:
//!
//! 1. Before any write to the Library the core calls [`Keeper::begin`]: it
//!    settles what an earlier service left (below), writes any entry still
//!    waiting, and commits an *intent* to the Sidecar that reserves the
//!    entry's identifier. If any of that fails, the Library write is
//!    refused and nothing changes.
//! 2. The Library write commits. The core then says whether any row
//!    changed ([`Keeper::written`]), before it re-reads anything.
//! 3. The history records the edit, also when the re-read failed; the
//!    entry is appended under its reserved identifier and settles its
//!    intent in the same transaction.
//! 4. If that append fails, the entry, with its identifier, is kept in
//!    memory and in [`SPILL_FILE`] (fsynced), and is written before the
//!    next Library write is let through. Appending it again under the same
//!    identifier changes nothing, so a replay cut short is safe.
//! 5. A write that changed rows and was never recorded (a caller whose
//!    refresh failed before it recorded) gets an entry of kind
//!    [`UNCONFIRMED`] under its identifier when the next write begins or
//!    the service stops.
//!
//! On start and at write admission, the Keeper settles what an earlier
//! service left: its kept entries are written,
//! each intent no entry settled becomes an [`UNCONFIRMED`] entry saying
//! the change was begun and not confirmed (whether the Library committed
//! it is not known), and the Runs it owned (the window's, a command's
//! own) and never ended are ended. A spill file that cannot be read whole
//! is left exactly as it is, and every write and every read of the Ledger
//! is refused, naming it, until the DJ moves it aside.
//!
//! A Run named on a request is admitted for the length of the command:
//! `runs.finish` waits for the commands in it and admits no other
//! meanwhile. When the service stops, no write is begun, the Library's
//! edit gate is taken so the window's edit in progress is over and
//! recorded, and only then is the Ledger taken off the core.
//!
//! Bulk analysis and local-file imports use `rbl_library::bulk::Operation`:
//! one operation intent and lazy Run, with confirmed results flushed in bounded
//! batches. Only writes outside Undo history join it; recorded playlist edits
//! retain their ordinary preparation and transactional Mutant Follows. See
//! `bulk_ledger.rs` for bounded degradation when recording fails.
//!
//! # Locks
//!
//! The Keeper is called under the Library's edit gate, and for an entry
//! under the edit history's lock. It takes only its own state lock and
//! the Sidecar's, both leaves: it never takes the service's Library lock,
//! the edit gate or the history, and never calls a host or sends an
//! event.

#[path = "bulk_ledger.rs"]
mod bulk;
#[path = "bulk_spill.rs"]
mod bulk_spill;
#[path = "file_ledger.rs"]
mod files;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rbl_library::history::LibraryEdit;
use rbl_library::ledger::{Link, Step, Unkept};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::ledger::{Action, Count, Intent, NewEntry, Store};
use wreckordbox_sidecar::{Run, RunId, Sidecar, Source, SourceKind};

use crate::facts::SourceArg;
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::Stems;

/// Command names.
pub mod commands {
    /// Runs, newest first, with how much each changed.
    pub const RUNS_LIST: &str = "runs.list";
    /// One Run and what it changed.
    pub const RUNS_SHOW: &str = "runs.show";
}

/// The name of the `tool` source a client's edit has when the client names
/// none. A client's edit is never the DJ's unless the client says so.
pub const UNNAMED_SOURCE: &str = "wreckordbox";

/// The description of the Run the DJ's edits in the window are grouped in.
pub const WINDOW_RUN: &str = "Edits in the window";

/// How long the DJ makes no edit in the window before the window's Run
/// ends and the next edit starts another.
pub const WINDOW_PAUSE: Duration = Duration::from_secs(5 * 60);

/// Overrides [`WINDOW_PAUSE`], in seconds; read when the service starts.
pub const WINDOW_PAUSE_ENV: &str = "WRECKORDBOX_WINDOW_RUN_PAUSE_SECS";

/// Entries the Ledger could not take when they were made, one JSON object
/// a line, in the Sidecar's folder. Emptied into the Ledger before the next
/// write to the Library.
pub const SPILL_FILE: &str = "ledger-spill.jsonl";

/// The kind of an entry for an edit the history records nothing for.
pub const UNRECORDED: &str = "unrecorded";
/// The kind of an entry for an edit that removed rows for good.
pub const PERMANENT: &str = "permanent";

/// How many Runs or entries a page holds when the caller does not say.
pub const DEFAULT_PAGE_LIMIT: usize = 50;

/// The Run a command's changes belong to, as its request names it: the
/// request's `run`, beside `command` and `args`.
///
/// `{"id": 7}` joins the open Run 7, which stays open. Without `id` the
/// command is a Run of its own, by `source` and called `description`.
/// `source` beside `id` is the source of this command's changes inside
/// that Run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunArg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The arguments of `runs.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRuns {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// The arguments of `runs.show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowRun {
    pub id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// A Run and how much it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    #[serde(flatten)]
    pub run: Run,
    /// How many changes it made in all.
    pub changes: u64,
    /// The same by store and kind: the Library's edits by kind of edit,
    /// the Sidecar's by kind of Fact.
    pub counts: Vec<Count>,
}

/// What `runs.list` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunList {
    /// Newest first.
    pub runs: Vec<RunSummary>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    /// The `offset` of the next page; `null` on the last.
    pub next_offset: Option<usize>,
}

/// One value an entry changed, for a person to read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueChange {
    /// What was changed: `track 10003`.
    pub subject: String,
    /// Which of its fields: the Library's column, `MyTags`, a Fact's kind.
    pub field: String,
    pub before: Value,
    pub after: Value,
}

/// One entry of the Ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub store: Store,
    /// The entry's identifier; for a recorded Fact (`action: "record"`),
    /// the Fact's.
    pub id: i64,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub at: i64,
    pub source: Source,
    pub action: Action,
    /// The kind of edit, or the Fact's kind.
    pub kind: String,
    /// The edit's name for a person, as Undo shows it.
    pub label: String,
    /// The operation that made it: `{"name", "args"}`. `files.edit` is a
    /// stored descriptor for a file-only edit, not a callable service command.
    pub command: Option<Value>,
    /// The entry an Undo or Redo steps.
    pub reverses: Option<i64>,
    /// The entry this one carries on: a later slice of one Add Tracks to
    /// Playlist names the first, and is reversed with it.
    pub part_of: Option<i64>,
    /// Whether `payload` is everything needed to take the change back.
    pub reversible: bool,
    /// The version of `payload`'s shape.
    pub format: u32,
    /// The exact before and after values, in the stored form.
    pub payload: Option<Value>,
    /// The same for a person: for an `undo`, as the Undo changed them.
    pub values: Vec<ValueChange>,
    /// For a recorded Fact: the Fact, and the one it took the place of.
    pub fact: Option<crate::facts::Fact>,
    pub replaced: Option<crate::facts::Fact>,
    /// Native judgements skipped without losing or refusing the Library edit.
    #[serde(default)]
    pub facts_not_recorded: Vec<wreckordbox_sidecar::dj_facts::Omission>,
}

/// What `runs.show` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunShown {
    /// Accepted named writes waiting for recording; paging uses the same offset/limit.
    #[serde(default)]
    pub pending_recording: Value,
    pub run: RunSummary,
    /// In the order they were made.
    pub entries: Vec<LedgerEntry>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub next_offset: Option<usize>,
}

/// The stable, versioned form of the edit history's payloads.
///
/// Format 1, by `kind` (a `value` is a SQLite value: `null`, a whole
/// number, text, `{"real": 1.5}` or `{"blob": "<hex>"}`):
///
/// | kind | payload |
/// |---|---|
/// | `track` | `{"edits": [{"track", "column", "before": value, "after": value}]}` |
/// | `track_tags` | `{"tags": [{"track", "before": [tag ids], "after": [tag ids]}], "comments": [as a `track` edit]}` |
/// | `my_tag_order` | `{"parent", "before": [{"id", "seq"}], "after": [{"id", "seq"}]}` |
/// | `reorder_playlist_entries` | `{"playlist", "before": [{"entry", "number"}], "after": [{"entry", "number"}]}` |
///
/// Every other kind is named by [`kind_of`] and has no stored payload yet.
pub mod stored {
    use rbl_db::write::stored::{MyTagOrderParts, TrackEditParts, TrackTagEditParts};
    use rbl_db::write::{MyTagOrder, TrackEdit, TrackTagEdit};
    use rbl_library::history::LibraryEdit;
    use rusqlite::types::Value as Sql;
    use serde_json::{json, Value};

    use super::ValueChange;

    /// The version of the shapes written now.
    pub const FORMAT: u32 = 1;

    /// The stable name of an edit's kind.
    #[must_use]
    pub fn kind_of(edit: &LibraryEdit) -> &'static str {
        match edit {
            LibraryEdit::DeletePlaylist(_) => "delete_playlist",
            LibraryEdit::RenamePlaylist(_) => "rename_playlist",
            LibraryEdit::MovePlaylist(_) => "move_playlist",
            LibraryEdit::ReplacePlaylist(_) => "replace_playlist",
            LibraryEdit::ReplacePlaylistContents(_) => "replace_playlist_contents",
            LibraryEdit::SortPlaylists(_) => "sort_playlists",
            LibraryEdit::RemovePlaylistTracks(_) => "remove_playlist_tracks",
            LibraryEdit::AddPlaylistTracks { .. } => "add_playlist_tracks",
            LibraryEdit::ReorderPlaylist(_) => "reorder_playlist",
            LibraryEdit::ReorderEntries(_) => "reorder_playlist_entries",
            LibraryEdit::CopyPlaylist(_) => "copy_playlist",
            LibraryEdit::Track(_) => "track",
            LibraryEdit::KeyEdit { .. } => "key_edit",
            LibraryEdit::TrackTags { .. } => "track_tags",
            LibraryEdit::MyTagOrder(_) => "my_tag_order",
            LibraryEdit::Cues(_) => "cues",
            LibraryEdit::CreatePlaylist(_) => "create_playlist",
            LibraryEdit::PlaylistEdit(_) => "playlist_edit",
            LibraryEdit::Relocate(_) => "relocate",
            LibraryEdit::ListRemoval(_) => "list_removal",
            LibraryEdit::Judgements(_) => "dj_judgements",
            // As the file-only grid edits before it (wbx-22): its beats are
            // in the command the intent recorded; Unwind does not reverse it.
            LibraryEdit::Grid(_) => "window_grid",
        }
    }

    /// The stored payload of `edit`, for the kinds that have one.
    #[must_use]
    pub fn store(edit: &LibraryEdit) -> Option<Value> {
        match edit {
            LibraryEdit::Track(edits) => Some(json!({ "edits": track_edits(edits) })),
            LibraryEdit::TrackTags { tags, comments } => Some(json!({
                "tags": tags
                    .iter()
                    .map(|edit| {
                        let parts = edit.parts();
                        json!({"track": parts.track, "before": parts.before, "after": parts.after})
                    })
                    .collect::<Vec<Value>>(),
                "comments": track_edits(comments),
            })),
            LibraryEdit::MyTagOrder(order) => {
                let parts = order.parts();
                let run = |run: &[(String, Option<i64>)]| -> Vec<Value> {
                    run.iter()
                        .map(|(id, seq)| json!({"id": id, "seq": seq}))
                        .collect()
                };
                Some(json!({
                    "parent": parts.parent,
                    "before": run(&parts.before),
                    "after": run(&parts.after),
                }))
            }
            LibraryEdit::ReorderEntries(reorder) => {
                let (playlist, before, after) = reorder.parts();
                let run = |run: &[(String, i64)]| -> Vec<Value> {
                    run.iter()
                        .map(|(entry, number)| json!({"entry": entry, "number": number}))
                        .collect()
                };
                Some(json!({ "playlist": playlist, "before": run(before), "after": run(after) }))
            }
            LibraryEdit::Judgements(judgements) => {
                let value = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
                Some(json!({
                    "changes": judgements
                        .changes
                        .iter()
                        .map(|change| json!({
                            "track": change.track,
                            "kind": change.kind,
                            "before": value(&change.before),
                            "after": value(&change.after),
                        }))
                        .collect::<Vec<Value>>(),
                }))
            }
            _ => None,
        }
    }

    /// The history's edit a stored payload was made from: what an Unwind
    /// checks against the Library and inverts. `None` for a kind with no
    /// stored payload, a format this build does not know, or a payload
    /// that is not of its kind's shape.
    #[must_use]
    pub fn restore(kind: &str, format: u32, payload: &Value) -> Option<LibraryEdit> {
        if format != FORMAT {
            return None;
        }
        match kind {
            "track" => Some(LibraryEdit::Track(track_edits_from(payload.get("edits")?)?)),
            "track_tags" => {
                let tags = payload
                    .get("tags")?
                    .as_array()?
                    .iter()
                    .map(|edit| {
                        Some(TrackTagEdit::from_parts(TrackTagEditParts {
                            track: edit.get("track")?.as_str()?.to_owned(),
                            before: texts(edit.get("before")?)?,
                            after: texts(edit.get("after")?)?,
                        }))
                    })
                    .collect::<Option<Vec<TrackTagEdit>>>()?;
                let comments = track_edits_from(payload.get("comments")?)?;
                Some(LibraryEdit::TrackTags { tags, comments })
            }
            "my_tag_order" => {
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
                            Some((item.get("id")?.as_str()?.to_owned(), seq))
                        })
                        .collect()
                };
                Some(LibraryEdit::MyTagOrder(MyTagOrder::from_parts(
                    MyTagOrderParts {
                        parent: payload.get("parent")?.as_str()?.to_owned(),
                        before: run(payload.get("before")?)?,
                        after: run(payload.get("after")?)?,
                    },
                )))
            }
            "reorder_playlist_entries" => {
                let run = |run: &Value| -> Option<Vec<(String, i64)>> {
                    run.as_array()?
                        .iter()
                        .map(|item| {
                            Some((
                                item.get("entry")?.as_str()?.to_owned(),
                                item.get("number")?.as_i64()?,
                            ))
                        })
                        .collect()
                };
                Some(LibraryEdit::ReorderEntries(
                    rbl_library::entry_order::EntryReorder::from_parts(
                        payload.get("playlist")?.as_str()?.to_owned(),
                        run(payload.get("before")?)?,
                        run(payload.get("after")?)?,
                    ),
                ))
            }
            "dj_judgements" => {
                let changes = payload
                    .get("changes")?
                    .as_array()?
                    .iter()
                    .map(|change| {
                        Some(rbl_library::judgements::JudgementChange {
                            track: change.get("track")?.as_str()?.to_owned(),
                            kind: change.get("kind")?.as_str()?.to_owned(),
                            before: change.get("before")?.to_string(),
                            after: change.get("after")?.to_string(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(LibraryEdit::Judgements(
                    rbl_library::judgements::Judgements { changes },
                ))
            }
            _ => None,
        }
    }

    /// A stored payload's values for a person to read, in the direction
    /// the entry changed them: `undone` swaps before and after.
    #[must_use]
    pub fn values(kind: &str, format: u32, payload: &Value, undone: bool) -> Vec<ValueChange> {
        let mut values = Vec::new();
        if format != FORMAT {
            return values;
        }
        let list = |key: &str| -> Vec<Value> {
            payload
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let track = |edit: &Value| {
            format!(
                "track {}",
                edit.get("track").and_then(Value::as_str).unwrap_or("?")
            )
        };
        let field = |edit: &Value, key: &str| edit.get(key).cloned().unwrap_or(Value::Null);
        let column = |edit: &Value| ValueChange {
            subject: track(edit),
            field: edit
                .get("column")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned(),
            before: shown(&field(edit, "before")),
            after: shown(&field(edit, "after")),
        };
        match kind {
            "track" => values.extend(list("edits").iter().map(column)),
            "track_tags" => {
                values.extend(list("tags").iter().map(|edit| ValueChange {
                    subject: track(edit),
                    field: "MyTags".to_owned(),
                    before: field(edit, "before"),
                    after: field(edit, "after"),
                }));
                values.extend(list("comments").iter().map(column));
            }
            "my_tag_order" => {
                let ids = |key: &str| -> Value {
                    list(key)
                        .iter()
                        .map(|item| field(item, "id"))
                        .collect::<Vec<Value>>()
                        .into()
                };
                let parent = payload.get("parent").and_then(Value::as_str).unwrap_or("?");
                values.push(ValueChange {
                    subject: if parent == rbl_db::write::ROOT {
                        "MyTag categories".to_owned()
                    } else {
                        format!("MyTags of category {parent}")
                    },
                    field: "order".to_owned(),
                    before: ids("before"),
                    after: ids("after"),
                });
            }
            "dj_judgements" => values.extend(list("changes").iter().map(|change| {
                ValueChange {
                    subject: track(change),
                    field: change
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    before: field(change, "before"),
                    after: field(change, "after"),
                }
            })),
            _ => {}
        }
        if undone {
            for value in &mut values {
                std::mem::swap(&mut value.before, &mut value.after);
            }
        }
        values
    }

    fn track_edits(edits: &[TrackEdit]) -> Vec<Value> {
        edits
            .iter()
            .map(|edit| {
                let parts = edit.parts();
                json!({
                    "track": parts.track,
                    "column": parts.column,
                    "before": sql_json(&parts.before),
                    "after": sql_json(&parts.after),
                })
            })
            .collect()
    }

    fn track_edits_from(edits: &Value) -> Option<Vec<TrackEdit>> {
        edits
            .as_array()?
            .iter()
            .map(|edit| {
                TrackEdit::from_parts(TrackEditParts {
                    track: edit.get("track")?.as_str()?.to_owned(),
                    column: edit.get("column")?.as_str()?.to_owned(),
                    before: json_sql(edit.get("before")?)?,
                    after: json_sql(edit.get("after")?)?,
                })
            })
            .collect()
    }

    fn texts(list: &Value) -> Option<Vec<String>> {
        list.as_array()?
            .iter()
            .map(|text| text.as_str().map(str::to_owned))
            .collect()
    }

    /// A SQLite value, exactly: an integer and a real of the same amount
    /// stay apart.
    fn sql_json(value: &Sql) -> Value {
        match value {
            Sql::Null => Value::Null,
            Sql::Integer(whole) => json!(whole),
            Sql::Real(real) => json!({ "real": real }),
            Sql::Text(text) => json!(text),
            Sql::Blob(bytes) => {
                use std::fmt::Write as _;
                let hex = bytes.iter().fold(String::new(), |mut hex, byte| {
                    let _ = write!(hex, "{byte:02x}");
                    hex
                });
                json!({ "blob": hex })
            }
        }
    }

    fn json_sql(value: &Value) -> Option<Sql> {
        match value {
            Value::Null => Some(Sql::Null),
            Value::Number(number) => number.as_i64().map(Sql::Integer),
            Value::String(text) => Some(Sql::Text(text.clone())),
            Value::Object(tagged) => {
                if let Some(real) = tagged.get("real") {
                    return real.as_f64().map(Sql::Real);
                }
                let hex = tagged.get("blob")?.as_str()?;
                if hex.len() % 2 != 0 {
                    return None;
                }
                (0..hex.len())
                    .step_by(2)
                    .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
                    .collect::<Option<Vec<u8>>>()
                    .map(Sql::Blob)
            }
            _ => None,
        }
    }

    /// A stored value as a person reads it.
    fn shown(value: &Value) -> Value {
        match value {
            Value::Object(tagged) => match (tagged.get("real"), tagged.get("blob")) {
                (Some(real), _) => real.clone(),
                (_, Some(Value::String(hex))) => json!(format!("<{} bytes>", hex.len() / 2)),
                _ => value.clone(),
            },
            other => other.clone(),
        }
    }
}

/// Whether `command` changes the Library, and so takes a Run on its
/// request and answers with the Run its changes are in.
pub(crate) fn takes_run(command: &str) -> bool {
    command == crate::protocol::commands::TRACKS_RATE
        || command == crate::intake::commands::TRACKS_SET_PREP_COLOUR
        || command == crate::tag_panel::commands::INTAKE_SET_TAGS
        || crate::mytag_edits::answers(command)
        || crate::playlist_edits::answers(command)
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::RUNS_LIST | commands::RUNS_SHOW)
}

/// Answers `runs.list` and `runs.show`. Reading never makes a Sidecar:
/// with none there are no Runs.
pub(crate) fn answer(
    keeper: &Keeper,
    stems: &Stems,
    request: &Request,
) -> Result<Value, ServiceError> {
    let recovery_error = keeper.state().recovery_error.clone();
    if request.command == commands::RUNS_LIST {
        let index = |name: &str, default| {
            request.args[name]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(default)
        };
        let mut pending = bulk_spill::pending(
            &stems.bulk_recovery_dir(),
            None,
            index("offset", 0),
            index("limit", DEFAULT_PAGE_LIMIT).min(1000),
        )
        .map_err(|why| ServiceError::new(crate::stems::codes::SIDECAR_FAILED, why))?;
        if pending["total"].as_u64().unwrap_or(0) != 0 {
            if let Some(records) = pending["records"].as_array_mut() {
                for record in records {
                    record["result"] = json!({"track":record["result"].get("track"),"target":record["result"].get("target")});
                }
            }
            return Err(ServiceError::new(crate::stems::codes::SIDECAR_FAILED,
                format!("Accepted writes are waiting for a Ledger Run because recording was unavailable. Pending targets (page with offset/limit): {pending}")));
        }
    }
    stems.with_sidecar_wait(|sidecar| {
        if sidecar.is_none() && (stems.sidecar_dir().join("sidecar.db").exists() || bulk_spill::exists(&stems.bulk_recovery_dir())) {
            if let Some(error) = recovery_error { return Err(error); }
            return Err(ServiceError::new(crate::stems::codes::SIDECAR_FAILED,
                "The Sidecar is not open. Check service startup diagnostics and storage access; a Ledger query cannot open or recover it."));
        }
        let mut answer = read_ledger(sidecar, request)?;
        if request.command == commands::RUNS_SHOW {
            if let Some(run) = request.args["id"].as_i64() {
                answer["pending_recording"] = bulk_spill::pending(&stems.bulk_recovery_dir(), Some(run), answer["offset"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(0), answer["limit"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(DEFAULT_PAGE_LIMIT))
                    .map_err(|why|ServiceError::new(crate::stems::codes::SIDECAR_FAILED,why))?;
            }
        }
        Ok(answer)
    })?
}

fn read_ledger(sidecar: Option<&Sidecar>, request: &Request) -> Result<Value, ServiceError> {
    let bad = |e: serde_json::Error| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    };
    // Startup and write admission recover. A query never settles active work.
    let answer = if request.command == commands::RUNS_LIST {
        let args: ListRuns = if request.args.is_null() {
            ListRuns::default()
        } else {
            serde_json::from_value(request.args.clone()).map_err(bad)?
        };
        let limit = args.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        let offset = args.offset.unwrap_or(0);
        let (runs, total) = match &sidecar {
            Some(sidecar) => sidecar.runs(limit, offset).map_err(|e| failed(&e))?,
            None => (Vec::new(), 0),
        };
        let runs: Vec<RunSummary> = runs.into_iter().map(summary).collect();
        serde_json::to_value(RunList {
            next_offset: next_offset(offset, runs.len(), total),
            runs,
            total,
            offset,
            limit,
        })
    } else {
        let args: ShowRun = serde_json::from_value(request.args.clone()).map_err(bad)?;
        let not_found = || {
            ServiceError::new(
                crate::facts::codes::RUN_NOT_FOUND,
                format!("there is no Run {}", args.id),
            )
        };
        let sidecar = sidecar.ok_or_else(not_found)?;
        let run = sidecar
            .run_summary(RunId(args.id))
            .map_err(|e| failed(&e))?
            .ok_or_else(not_found)?;
        let limit = args.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        let offset = args.offset.unwrap_or(0);
        let (entries, total) = sidecar
            .run_entries(RunId(args.id), limit, offset)
            .map_err(|e| failed(&e))?;
        let entries: Vec<LedgerEntry> = entries.into_iter().map(entry).collect();
        serde_json::to_value(RunShown {
            pending_recording: Value::Null,
            run: summary(run),
            next_offset: next_offset(offset, entries.len(), total),
            entries,
            total,
            offset,
            limit,
        })
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

fn next_offset(offset: usize, shown: usize, total: usize) -> Option<usize> {
    let next = offset.saturating_add(shown);
    (shown > 0 && next < total).then_some(next)
}

fn summary(run: wreckordbox_sidecar::ledger::RunSummary) -> RunSummary {
    RunSummary {
        changes: run.counts.iter().map(|count| count.count).sum(),
        run: run.run,
        counts: run.counts,
    }
}

fn entry(entry: wreckordbox_sidecar::ledger::Entry) -> LedgerEntry {
    let values = match (&entry.fact, &entry.payload) {
        (Some(fact), _) => vec![ValueChange {
            subject: format!("track {}", fact.track_id),
            field: fact.kind.clone(),
            before: entry
                .replaced
                .as_ref()
                .map_or(Value::Null, |replaced| replaced.value.clone()),
            after: fact.value.clone(),
        }],
        (None, Some(payload)) if entry.store == Store::Library => stored::values(
            &entry.kind,
            entry.format,
            payload,
            entry.action == Action::Undo,
        ),
        _ => Vec::new(),
    };
    LedgerEntry {
        store: entry.store,
        id: entry.id,
        run: entry.run,
        at: entry.at,
        source: entry.source,
        action: entry.action,
        kind: entry.kind,
        label: entry.label,
        command: entry.command,
        reverses: entry.reverses,
        part_of: entry.part_of,
        reversible: entry.reversible,
        format: entry.format,
        facts_not_recorded: entry
            .payload
            .as_ref()
            .and_then(|p| p.get("facts_not_recorded"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        payload: entry.payload,
        values,
        fact: entry.fact.map(Into::into),
        replaced: entry.replaced.map(Into::into),
    }
}

fn failed(error: &wreckordbox_sidecar::SidecarError) -> ServiceError {
    ServiceError::new(
        crate::stems::codes::SIDECAR_FAILED,
        format!("the Sidecar failed: {error}"),
    )
}

/// Who a command's changes are by, and which Run they go in.
#[derive(Debug, Clone)]
enum Target {
    /// The DJ, in the window's Run.
    Window,
    /// A Run that was open before the command and stays open. The command
    /// is admitted to it ([`Keeper::admit`]): the Run cannot end until the
    /// command is over.
    Joined { run: RunId, source: Source },
    /// A Run of the command's own, made when its first change is recorded.
    Own {
        source: Source,
        description: String,
        made: Option<RunId>,
    },
}

/// The command this thread is answering, for the entries its edits make.
struct Scope {
    target: Target,
    /// `{"name", "args"}`.
    command: Value,
    name: String,
    /// How many entries it has made.
    entries: usize,
}

thread_local! {
    /// Set for the length of one command, on the thread answering it,
    /// which is the thread its edits are made and recorded on. A thread
    /// with none is not answering a command: the window's own.
    static CURRENT: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// An entry that could not be written when it was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Spilled {
    /// The identifier reserved for it before the Library was changed;
    /// writing it again under the same one changes nothing.
    #[serde(default)]
    id: Option<i64>,
    /// The Run, when one could be found or made.
    run: Option<i64>,
    source: Source,
    /// What to call the Run that has to be made for it.
    description: String,
    at: i64,
    action: Action,
    kind: String,
    label: String,
    command: Option<Value>,
    reverses: Option<i64>,
    #[serde(default)]
    part_of: Option<i64>,
    reversible: bool,
    format: u32,
    payload: Option<Value>,
    #[serde(default)]
    dj_snapshot: Option<Value>,
    #[serde(default)]
    dj_facts: Option<wreckordbox_sidecar::dj_facts::NativeFacts>,
    /// The Mutants' consequences of the change (wbx-80), applied with it.
    #[serde(default)]
    follow: wreckordbox_sidecar::mutant_follow::Follow,
    /// The consequences could not be worked out when the entry was made:
    /// they are worked out from the Library and `mutant_plan` when it is
    /// written.
    #[serde(default)]
    recover_mutants: bool,
    #[serde(default)]
    mutant_plan: Option<Value>,
}

struct WindowRun {
    run: RunId,
    last_edit: Instant,
}

/// The Library write begun last ([`Keeper::begin`]): its intent is in the
/// Sidecar until an entry settles it.
struct Op {
    /// The intent's identifier, which its entry takes.
    id: i64,
    /// Whether it changed any row, once told.
    wrote: Option<bool>,
    /// Whether its entry was made (written or kept for later).
    recorded: bool,
    /// An unnamed write outside the history: deliberately not recorded
    /// here (bead wbx-14.11).
    unnamed: bool,
    window: bool,
    run: Option<RunId>,
    source: Source,
    description: String,
    command: Option<Value>,
    dj_snapshot: Option<Value>,
    /// What the write planned for Mutants before it was made (wbx-80).
    mutant_plan: Option<Value>,
}

#[derive(Default)]
struct State {
    player: Option<WindowRun>,
    window: Option<WindowRun>,
    /// Whether what an earlier service left behind has been dealt with:
    /// its kept entries written, its intents settled, its Runs ended.
    settled: bool,
    recovery_error: Option<ServiceError>,
    /// Entries made and not yet in the Ledger, oldest first. Also in the
    /// spill file, when that could be written.
    unwritten: Vec<Spilled>,
    /// Completed bulk spills only: live operation intents must never be settled.
    bulk_waiting: Vec<std::path::PathBuf>,
    op: Option<Op>,
    /// Intents that needed no entry, removed with the next one.
    done: Vec<i64>,
    /// Runs with a command in them still being answered, and how many.
    admitted: HashMap<i64, usize>,
    /// Runs being ended: no command is admitted to them.
    finishing: HashSet<i64>,
    /// The service is stopping: no write is begun.
    closed: bool,
}

/// One entry on its way into the Ledger.
struct Draft<'a> {
    action: Action,
    kind: &'a str,
    label: &'a str,
    reverses: Option<i64>,
    part_of: Option<i64>,
    payload: Option<Value>,
    /// What the change means for the Mutants of its entries (wbx-80), or
    /// why that could not be worked out.
    follow: Result<wreckordbox_sidecar::mutant_follow::Follow, String>,
    /// The plan its write left with the intent, for working it out again.
    plan: Option<Value>,
}

/// Failures a test can inject into a service with [`FAULTS_ENV`], honoured
/// only under `RBXPORT_TEST`: a comma-separated list of
/// `abort_after_write` (the process dies once a Library write is made,
/// before its entry), `fail_append=N` (the next N entries are refused by
/// the Sidecar), `abort_in_replay` (the process dies once the first kept
/// entry is written back, before the rest) and `delay_after_begin_ms=N`
/// (every Library write waits N ms once begun).
pub const FAULTS_ENV: &str = "WRECKORDBOX_LEDGER_FAULTS";

#[derive(Default)]
struct Faults {
    abort_after_write: bool,
    abort_before_file_publish: bool,
    fail_append: AtomicUsize,
    abort_in_replay: bool,
    delay_after_begin: Option<Duration>,
}

impl Faults {
    fn from_env() -> Self {
        let mut faults = Self::default();
        if std::env::var("RBXPORT_TEST").as_deref() != Ok("1") {
            return faults;
        }
        let Ok(list) = std::env::var(FAULTS_ENV) else {
            return faults;
        };
        for fault in list.split(',').map(str::trim) {
            let (name, value) = fault.split_once('=').unwrap_or((fault, ""));
            let number = value.parse::<u64>().unwrap_or(1);
            match name {
                "abort_before_file_publish" => faults.abort_before_file_publish = true,
                "abort_after_write" => faults.abort_after_write = true,
                "fail_append" => {
                    faults.fail_append = AtomicUsize::new(usize::try_from(number).unwrap_or(1));
                }
                "abort_in_replay" => faults.abort_in_replay = true,
                "delay_after_begin_ms" => {
                    faults.delay_after_begin = Some(Duration::from_millis(number));
                }
                _ => tracing::warn!(fault, "unknown Ledger fault ignored"),
            }
        }
        faults
    }

    fn refuse_append(&self) -> bool {
        let mut left = self.fail_append.load(Ordering::Acquire);
        while left > 0 {
            match self.fail_append.compare_exchange(
                left,
                left - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(now) => left = now,
            }
        }
        false
    }
}

/// The label of an entry for a change whose outcome is not known.
pub const UNCONFIRMED: &str = "unconfirmed";
const BEGUN_NOT_CONFIRMED: &str =
    "A change to the Library was begun and not confirmed: wreckordbox stopped before recording it";
const MADE_NOT_RECORDED: &str = "A change to the Library was made and its entry was not recorded";

/// Keeps the Ledger for one service: the `Ledger` its Library core tells.
pub(crate) struct Keeper {
    weak: std::sync::Weak<Keeper>,
    stopping: std::sync::atomic::AtomicBool,
    stems: Arc<Stems>,
    location: rbl_db::LibraryLocation,
    pause: Duration,
    state: Mutex<State>,
    /// Signalled when a command leaves a Run.
    left: Condvar,
    faults: Faults,
}

impl Keeper {
    pub(crate) fn new(stems: Arc<Stems>, location: rbl_db::LibraryLocation) -> Arc<Self> {
        let pause = std::env::var(WINDOW_PAUSE_ENV)
            .ok()
            .and_then(|secs| secs.parse::<u64>().ok())
            .map_or(WINDOW_PAUSE, Duration::from_secs);
        Arc::new_cyclic(|weak| Self {
            weak: weak.clone(),
            stopping: std::sync::atomic::AtomicBool::new(false),
            stems,
            location,
            pause,
            state: Mutex::new(State::default()),
            left: Condvar::new(),
            faults: Faults::from_env(),
        })
    }

    /// Answers one command with its changes attributed: runs `answer` on
    /// this thread and gives back its result beside the Run the command's
    /// changes are in, when it changed anything or joined a Run.
    ///
    /// `from_client` is whether a client asked (a connection) or the
    /// process hosting the service did. A Run named on the request is
    /// checked before anything is done, and cannot end until the command
    /// is over.
    pub(crate) fn attributed<T>(
        &self,
        from_client: bool,
        request: &Request,
        answer: impl FnOnce() -> Result<T, ServiceError>,
    ) -> (Result<T, ServiceError>, Option<Run>) {
        let target = match self.target(from_client, request) {
            Ok(Some(target)) => target,
            Ok(None) => return (answer(), None),
            Err(refused) => return (Err(refused), None),
        };
        let admitted = match &target {
            Target::Joined { run, .. } => Some(*run),
            _ => None,
        };
        let scope = Scope {
            target,
            command: json!({"name": request.command, "args": request.args}),
            name: request.command.clone(),
            entries: 0,
        };
        CURRENT.with(|current| *current.borrow_mut() = Some(scope));
        let result = answer();
        let scope = CURRENT.with(|current| current.borrow_mut().take());
        if let Some(run) = admitted {
            self.leave(run);
        }
        let Some(scope) = scope else {
            return (result, None);
        };
        let run = match scope.target {
            Target::Window => None,
            Target::Joined { run, .. } => Some(run),
            Target::Own { made, .. } => {
                // Do not open a writer for a refused or unchanged command.
                if let Some((run, sidecar)) =
                    made.and_then(|run| self.stems.sidecar().ok().map(|sidecar| (run, sidecar)))
                {
                    let outcome = match scope.entries {
                        1 => "1 change".to_owned(),
                        count => format!("{count} changes"),
                    };
                    if let Err(error) = sidecar.finish_run(run, &outcome) {
                        tracing::warn!(%error, run = run.0, "could not end a command's Run");
                    }
                }
                made
            }
        };
        let run = run.and_then(|run| self.stems.sidecar().ok()?.run(run).ok()?);
        (result, run)
    }

    /// Where `request`'s changes go. `None` for a command that changes
    /// nothing in the Library: it takes no Run.
    fn target(&self, from_client: bool, request: &Request) -> Result<Option<Target>, ServiceError> {
        if !takes_run(&request.command) {
            return match &request.run {
                Some(_) => Err(ServiceError::new(
                    service_codes::BAD_REQUEST,
                    format!(
                        "{} takes no Run on its request: it is not a command that edits the Library",
                        request.command
                    ),
                )),
                None => Ok(None),
            };
        }
        let named = request.run.clone().unwrap_or_default();
        let source = named
            .source
            .as_ref()
            .map(crate::facts::source)
            .transpose()?;
        if let Some(id) = named.id {
            if named.description.is_some() {
                return Err(ServiceError::new(
                    service_codes::BAD_REQUEST,
                    "a Run that exists has its description already: name the Run or describe a new one, not both",
                ));
            }
            let run = self.admit(id)?;
            return Ok(Some(Target::Joined {
                run: run.id,
                source: source.unwrap_or(run.source),
            }));
        }
        if !from_client && request.run.is_none() {
            return Ok(Some(Target::Window));
        }
        let description =
            match named.description.as_deref().map(str::trim) {
                Some("") => return Err(ServiceError::new(
                    service_codes::BAD_REQUEST,
                    "a Run's description cannot be empty: say what the work is, or leave it out",
                )),
                Some(description) => description.to_owned(),
                None => described(request),
            };
        Ok(Some(Target::Own {
            source: source.unwrap_or_else(|| Source::new(SourceKind::Tool, UNNAMED_SOURCE)),
            description,
            made: None,
        }))
    }

    /// Admits a command to open Run `id`: checked open and counted in one
    /// step under the Keeper's lock, so a Run being ended
    /// ([`Self::finishing`]) admits nothing and is not ended while a
    /// command is in it.
    fn admit(&self, id: i64) -> Result<Run, ServiceError> {
        let not_found = || {
            ServiceError::new(
                crate::facts::codes::RUN_NOT_FOUND,
                format!("there is no Run {id}"),
            )
        };
        let ended = || {
            ServiceError::new(
                crate::facts::codes::RUN_FINISHED,
                format!("Run {id} has ended; start another to change more"),
            )
        };
        let sidecar = self.stems.sidecar_if_made()?.ok_or_else(not_found)?;
        let mut state = self.state();
        if state.finishing.contains(&id) {
            return Err(ended());
        }
        let run = sidecar
            .run(RunId(id))
            .map_err(|e| failed(&e))?
            .ok_or_else(not_found)?;
        if run.is_finished() {
            return Err(ended());
        }
        *state.admitted.entry(id).or_default() += 1;
        Ok(run)
    }

    fn leave(&self, run: RunId) {
        let mut state = self.state();
        if let Some(count) = state.admitted.get_mut(&run.0) {
            *count -= 1;
            if *count == 0 {
                state.admitted.remove(&run.0);
            }
        }
        drop(state);
        self.left.notify_all();
    }

    /// Ends a Run with `finish` once no command is in it: commands already
    /// admitted are waited for, and no other is admitted meanwhile.
    /// `args` are `runs.finish`'s.
    pub(crate) fn finishing<T>(
        &self,
        args: &Value,
        finish: impl FnOnce() -> Result<T, ServiceError>,
    ) -> Result<T, ServiceError> {
        let Some(id) = args.get("id").and_then(Value::as_i64) else {
            return finish();
        };
        let mut state = self.state();
        state.finishing.insert(id);
        while state.admitted.get(&id).is_some_and(|count| *count > 0) {
            state = self
                .left
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let finished = finish();
        state.finishing.remove(&id);
        finished
    }

    /// Settles what an earlier service left, when there is a Sidecar:
    /// asked as the service starts. Queries never call this or open a writer.
    pub(crate) fn recover(&self) -> Result<(), ServiceError> {
        let result = (|| {
            let sidecar = if bulk_spill::exists(&self.stems.bulk_recovery_dir()) {
                Some(self.stems.sidecar()?)
            } else {
                self.stems.sidecar_if_made()?
            };
            let Some(sidecar) = sidecar else {
                self.state().settled = true;
                return Ok(());
            };
            let mut state = self.state();
            self.settle(&sidecar, &mut state).map_err(|why| {
                ServiceError::new(
                    crate::stems::codes::SIDECAR_FAILED,
                    format!("the Ledger could not be brought up to date: {why}"),
                )
            })
        })();
        self.state().recovery_error = result.as_ref().err().cloned();
        result
    }

    /// Bulk starts share recovery/catch-up admission. New recording failures
    /// remain separate from accepted writes; earlier unresolved work does not.
    fn admit_bulk(&self) -> Result<(), String> {
        let mut state = self.state();
        if state.closed {
            return Err("wreckordbox is stopping".into());
        }
        if !state.settled
            || !state.unwritten.is_empty()
            || !state.bulk_waiting.is_empty()
            || state.op.is_some()
        {
            let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
            self.settle(&sidecar, &mut state)?;
            self.write_waiting(&sidecar, &mut state)?;
            self.close_op(&sidecar, &mut state);
        }
        Ok(())
    }

    /// The service is stopping: no further write is begun. Writes already
    /// begun finish and are recorded.
    pub(crate) fn shut(&self) {
        self.stopping.store(true, Ordering::Release);
        self.state().closed = true;
    }

    /// Ends the window's Run and settles the last write's intent: the
    /// service has stopped and every write it saw is over. Entries still
    /// waiting stay in the spill file for the next service.
    pub(crate) fn close(&self) {
        self.stopping.store(true, Ordering::Release);
        let mut state = self.state();
        state.closed = true;
        let Ok(Some(sidecar)) = self.stems.sidecar_if_made() else {
            return;
        };
        self.close_op(&sidecar, &mut state);
        let done = std::mem::take(&mut state.done);
        if let Err(error) = sidecar.settle_intents(&done) {
            tracing::warn!(%error, "could not settle the Ledger's last intents");
        }
        for run in [state.window.take(), state.player.take()]
            .into_iter()
            .flatten()
        {
            let _ = sidecar.finish_run(run.run, "wreckordbox stopped");
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Deals once with what an earlier service left: its kept entries are
    /// written (each once, under its own identifier), every intent no entry
    /// settled becomes an entry saying its change was begun and not
    /// confirmed, and the Runs it owned and never ended are ended. A
    /// damaged spill file stops here, untouched, until the DJ deals with
    /// it.
    fn settle(&self, sidecar: &Sidecar, state: &mut State) -> Result<(), String> {
        if state.settled {
            return Ok(());
        }
        let file = spill_file(sidecar);
        if file.is_file() && state.unwritten.is_empty() {
            state.unwritten = read_spill(&file)?;
        }
        self.write_waiting(sidecar, state)?;
        bulk_spill::replay_all(&self.stems.bulk_recovery_dir(), sidecar)?;
        for open in sidecar.open_intents().map_err(|e| e.to_string())? {
            // What the unconfirmed write did to entries: the Library as it
            // is, and what it planned. A failed read keeps it unsettled.
            let follow =
                crate::mutant_follow::recover(&self.location, sidecar, open.mutant_plan.as_ref())
                    .map_err(|why| {
                    format!("the Mutants of an unconfirmed change could not be recovered: {why}")
                })?;
            let run = match open.run {
                Some(run) => run,
                None => late_run(sidecar, &open.source, &open.description)?,
            };
            let facts =
                crate::dj_facts::recover(&self.location, &self.stems, open.dj_snapshot.as_ref());
            let (_, followed) = sidecar
                .append_following(
                    &NewEntry {
                        id: Some(open.id),
                        run,
                        store: Store::Library,
                        source: &open.source,
                        at: Some(open.at),
                        action: Action::Write,
                        kind: UNCONFIRMED,
                        label: &files::recovery_label(
                            &self.location,
                            open.command.as_ref(),
                            BEGUN_NOT_CONFIRMED,
                        ),
                        command: open.command.as_ref(),
                        reverses: None,
                        part_of: None,
                        reversible: false,
                        format: stored::FORMAT,
                        payload: None,
                    },
                    &facts,
                    true,
                    &follow,
                )
                .map_err(|e| e.to_string())?;
            crate::mutant_follow::announce(&self.stems, &followed);
        }
        sidecar
            .finish_owned_runs("wreckordbox stopped without ending it")
            .map_err(|e| e.to_string())?;
        state.settled = true;
        Ok(())
    }

    /// Writes every entry still waiting into the Ledger, oldest first, and
    /// removes the spill file. An entry already there under its identifier
    /// (a replay cut short) is not written twice.
    fn write_waiting(&self, sidecar: &Sidecar, state: &mut State) -> Result<(), String> {
        while let Some(dir) = state.bulk_waiting.first() {
            bulk_spill::replay(dir, sidecar, || Ok(()))?;
            state.bulk_waiting.remove(0);
        }
        while let Some(waiting) = state.unwritten.first() {
            let run = match waiting.run {
                Some(run) => RunId(run),
                None => late_run(sidecar, &waiting.source, &waiting.description)?,
            };
            let reconciled = waiting.dj_facts.is_none();
            let facts = match &waiting.dj_facts {
                Some(facts) => facts.clone(),
                None => crate::dj_facts::recover(
                    &self.location,
                    &self.stems,
                    waiting.dj_snapshot.as_ref(),
                ),
            };
            let recovered;
            let follow = if waiting.recover_mutants {
                recovered = crate::mutant_follow::recover(
                    &self.location,
                    sidecar,
                    waiting.mutant_plan.as_ref(),
                )?;
                &recovered
            } else {
                &waiting.follow
            };
            let (_, followed) = sidecar
                .append_following(
                    &NewEntry {
                        id: waiting.id,
                        run,
                        store: Store::Library,
                        source: &waiting.source,
                        at: Some(waiting.at),
                        action: waiting.action,
                        kind: &waiting.kind,
                        label: &waiting.label,
                        command: waiting.command.as_ref(),
                        reverses: waiting.reverses,
                        part_of: waiting.part_of,
                        reversible: waiting.reversible,
                        format: waiting.format,
                        payload: waiting.payload.as_ref(),
                    },
                    &facts,
                    reconciled,
                    follow,
                )
                .map_err(|e| e.to_string())?;
            crate::mutant_follow::announce(&self.stems, &followed);
            state.unwritten.remove(0);
            if self.faults.abort_in_replay {
                std::process::abort();
            }
        }
        let file = spill_file(sidecar);
        if file.is_file() {
            std::fs::remove_file(&file)
                .map_err(|e| format!("{} could not be removed: {e}", file.display()))?;
        }
        Ok(())
    }

    /// The last write begun is over and the next is about to begin: a
    /// write that changed the Library and was neither recorded nor
    /// deliberately left out gets an entry saying so, under its intent's
    /// identifier; one that needed no entry has its intent removed with
    /// the next. When even that entry cannot be written, the intent stays,
    /// and the next service reports the change as begun and not confirmed.
    fn close_op(&self, sidecar: &Sidecar, state: &mut State) {
        let Some(op) = state.op.take() else {
            return;
        };
        if op.recorded {
            return;
        }
        if op.unnamed || op.wrote != Some(true) {
            state.done.push(op.id);
            return;
        }
        let follow = match crate::mutant_follow::recover(
            &self.location,
            sidecar,
            op.mutant_plan.as_ref(),
        ) {
            Ok(follow) => follow,
            Err(error) => {
                // Left as an open intent: the next service recovers it.
                tracing::error!(%error, intent = op.id, "a change's Mutants could not be recovered; the next service tries again");
                return;
            }
        };
        let run = if op.window {
            self.window_run(sidecar, state)
        } else {
            op.run
                .map_or_else(|| late_run(sidecar, &op.source, &op.description), Ok)
        };
        let appended = run.and_then(|run| {
            let facts =
                crate::dj_facts::recover(&self.location, &self.stems, op.dj_snapshot.as_ref());
            sidecar
                .append_following(
                    &NewEntry {
                        id: Some(op.id),
                        run,
                        store: Store::Library,
                        source: &op.source,
                        at: None,
                        action: Action::Write,
                        kind: UNCONFIRMED,
                        label: &files::recovery_label(
                            &self.location,
                            op.command.as_ref(),
                            MADE_NOT_RECORDED,
                        ),
                        command: op.command.as_ref(),
                        reverses: None,
                        part_of: None,
                        reversible: false,
                        format: stored::FORMAT,
                        payload: None,
                    },
                    &facts,
                    true,
                    &follow,
                )
                .map(|(_, followed)| crate::mutant_follow::announce(&self.stems, &followed))
                .map_err(|e| e.to_string())
        });
        if let Err(error) = appended {
            tracing::error!(%error, intent = op.id, "a change to the Library was made and could not be recorded; the next service reports it");
        }
    }

    /// Before a write that replaces or copies playlist entries: what its
    /// Mutants need should the write never be confirmed, kept with the
    /// write's intent (wbx-80). Best effort, as recovery evidence is: it
    /// never refuses a write, and a write it could not plan is recovered
    /// by detaching, without carrying.
    fn plan_mutants(&self, db: &rbl_db::Library, targets: &rbl_library::ledger::Preparation) {
        let Ok(sidecar) = self.stems.sidecar() else {
            return;
        };
        let plan = match crate::mutant_follow::plan_before(db, &sidecar, targets.replaces_entries) {
            Ok(Some(plan)) => plan,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "the Mutants of a replacement or copy could not be planned");
                return;
            }
        };
        let id = {
            let mut state = self.state();
            let Some(op) = state.op.as_mut() else {
                return;
            };
            op.mutant_plan = Some(plan.clone());
            op.id
        };
        if let Err(error) = sidecar.capture_mutant_plan(id, &plan) {
            tracing::warn!(%error, "a Mutant plan could not be kept with its intent");
        }
    }

    /// The window's Run: the one open, or a new one when there is none or
    /// the DJ has paused since its last edit.
    fn window_run(&self, sidecar: &Sidecar, state: &mut State) -> Result<RunId, String> {
        if let Some(window) = &mut state.window {
            if window.last_edit.elapsed() < self.pause {
                window.last_edit = Instant::now();
                return Ok(window.run);
            }
            let _ = sidecar.finish_run(window.run, "the DJ paused");
            state.window = None;
        }
        let run = sidecar
            .begin_owned_run(&Source::new(SourceKind::Dj, "dj"), WINDOW_RUN)
            .map_err(|e| e.to_string())?;
        state.window = Some(WindowRun {
            run,
            last_edit: Instant::now(),
        });
        Ok(run)
    }

    fn player_run(&self, sidecar: &Sidecar, state: &mut State) -> Result<RunId, String> {
        if let Some(player) = &mut state.player {
            if player.last_edit.elapsed() < self.pause {
                player.last_edit = Instant::now();
                return Ok(player.run);
            }
            let _ = sidecar.finish_run(player.run, "the player paused");
        }
        let run = sidecar
            .begin_owned_run(&Source::new(SourceKind::Tool, "link"), files::PLAYER_RUN)
            .map_err(|e| e.to_string())?;
        state.player = Some(WindowRun {
            run,
            last_edit: Instant::now(),
        });
        Ok(run)
    }

    /// Appends one entry for a change already made, attributed to the
    /// command this thread is answering or else to the DJ in the window.
    /// Answers the entry's identifier: the one reserved for the write it
    /// records, which it keeps even when it has to be written later.
    fn append(&self, draft: Draft) -> Option<i64> {
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            let mut state = self.state();
            let sidecar = self.stems.sidecar();
            let dj = Source::new(SourceKind::Dj, "dj");
            // An operation descriptor and evidence belong only to the entry
            // claiming that operation's reserved identifier.
            let fresh = state.op.as_ref().filter(|op| !op.recorded);
            let command = fresh.and_then(|op| op.command.clone())
                .or_else(|| current.as_ref().map(|scope| scope.command.clone()));
            let dj_snapshot = fresh.and_then(|op| op.dj_snapshot.clone());
            let player = fresh.is_some_and(|op| files::is_link(op.command.as_ref()));
            // Which Run and whose change, as far as can be told even when
            // the Sidecar cannot be written.
            let (run, source, description) = if player {
                let run = match &sidecar {
                    Ok(sidecar) => self.player_run(sidecar, &mut state),
                    Err(error) => Err(error.message.clone()),
                };
                (run, Source::new(SourceKind::Tool, "link"), files::PLAYER_RUN.to_owned())
            } else { match (current.as_mut().map(|scope| &mut scope.target), &sidecar) {
                (Some(Target::Joined { run, source }), _) => {
                    (Ok(*run), source.clone(), String::new())
                }
                (Some(Target::Own { source, description, made }), sidecar) => {
                    let run = match (*made, sidecar) {
                        (Some(run), _) => Ok(run),
                        (None, Ok(sidecar)) => sidecar
                            .begin_owned_run(source, description)
                            .map_err(|e| e.to_string())
                            .inspect(|run| *made = Some(*run)),
                        (None, Err(error)) => Err(error.message.clone()),
                    };
                    (run, source.clone(), description.clone())
                }
                (Some(Target::Window) | None, Ok(sidecar)) => (
                    self.window_run(sidecar, &mut state),
                    dj,
                    WINDOW_RUN.to_owned(),
                ),
                (Some(Target::Window) | None, Err(error)) => {
                    (Err(error.message.clone()), dj, WINDOW_RUN.to_owned())
                }
            }};
            if let Some(scope) = current.as_mut() {
                scope.entries += 1;
            }
            // The identifier its write reserved, for the first entry of
            // that write; another (there should be none) takes the next.
            let id = match state.op.as_mut() {
                Some(op) if !op.recorded => {
                    op.recorded = true;
                    Some(op.id)
                }
                _ => None,
            };
            let mut dj_facts = if source.kind == SourceKind::Dj && draft.action != Action::Undo {
                crate::dj_facts::derive(&self.location, &self.stems, draft.kind, draft.payload.as_ref())
            } else { wreckordbox_sidecar::dj_facts::NativeFacts::default() };
            if source.kind == SourceKind::Dj {
                if draft.action == Action::Undo { dj_facts.undo_of = draft.reverses; }
                if draft.action == Action::Redo { dj_facts.redo_of = draft.reverses; }
            }
            let at = now();
            let reversible = draft.payload.is_some();
            let (follow, unworked) = match draft.follow {
                Ok(follow) => (follow, None),
                Err(why) => (wreckordbox_sidecar::mutant_follow::Follow::default(), Some(why)),
            };
            let written = match (&sidecar, &run) {
                (Ok(_), Ok(_)) if unworked.is_some() => Err(format!(
                    "the Mutants' consequences could not be worked out: {}",
                    unworked.clone().unwrap_or_default()
                )),
                (Ok(_), Ok(_)) if self.faults.refuse_append() => {
                    Err("the Sidecar refused the entry (injected)".to_owned())
                }
                (Ok(sidecar), Ok(run)) if state.unwritten.is_empty() => sidecar
                    .append_following(&NewEntry {
                        id,
                        run: *run,
                        store: Store::Library,
                        source: &source,
                        at: Some(at),
                        action: draft.action,
                        kind: draft.kind,
                        label: draft.label,
                        command: command.as_ref(),
                        reverses: draft.reverses,
                        part_of: draft.part_of,
                        reversible,
                        format: stored::FORMAT,
                        payload: draft.payload.as_ref(),
                    }, &dj_facts, false, &follow)
                    .map(|(id, followed)| {
                        crate::mutant_follow::announce(&self.stems, &followed);
                        id
                    })
                    .map_err(|e| e.to_string()),
                (Ok(_), Ok(_)) => Err("earlier entries are still waiting to be written".to_owned()),
                (Err(error), _) => Err(error.message.clone()),
                (_, Err(error)) => Err(error.clone()),
            };
            match written {
                Ok(id) => Some(id),
                Err(error) => {
                    // The Library has changed. Keep the entry: here, and on
                    // disk in case this process ends before it is written.
                    // Its intent stays in the Sidecar until it is.
                    let spilled = Spilled {
                        id,
                        run: run.ok().map(|run| run.0),
                        source,
                        description,
                        at,
                        action: draft.action,
                        kind: draft.kind.to_owned(),
                        label: draft.label.to_owned(),
                        command,
                        reverses: draft.reverses,
                        part_of: draft.part_of,
                        reversible,
                        format: stored::FORMAT,
                        payload: draft.payload,
                        dj_snapshot,
                        dj_facts: Some(dj_facts),
                        recover_mutants: unworked.is_some(),
                        mutant_plan: draft.plan,
                        follow,
                    };
                    let on_disk = sidecar
                        .as_ref()
                        .map_err(|e| e.message.clone())
                        .and_then(|sidecar| spill(&spill_file(sidecar), &spilled));
                    tracing::error!(
                        %error,
                        kept_on_disk = on_disk.is_ok(),
                        kind = %spilled.kind,
                        "a change to the Library could not be written to the Ledger; it is kept and no further change is made until it is"
                    );
                    state.unwritten.push(spilled);
                    id
                }
            }
        })
    }
}

impl Keeper {
    fn begin_operation(&self, file: Option<Value>) -> Result<(), String> {
        let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
        let mut state = self.state();
        if state.closed {
            return Err("wreckordbox is stopping".to_owned());
        }
        self.settle(&sidecar, &mut state)?;
        if !state.unwritten.is_empty() || !state.bulk_waiting.is_empty() {
            self.write_waiting(&sidecar, &mut state)
                .map_err(|e| format!("an earlier change is still not recorded ({e})"))?;
        }
        self.close_op(&sidecar, &mut state);
        // Whose change it will be, as far as is known before it is made.
        let (window, run, source, description, command) = CURRENT.with(|current| {
            let current = current.borrow();
            let command = file.or_else(|| current.as_ref().map(|scope| scope.command.clone()));
            if files::is_link(command.as_ref()) {
                return (
                    false,
                    state.player.as_ref().map(|player| player.run),
                    Source::new(SourceKind::Tool, "link"),
                    files::PLAYER_RUN.to_owned(),
                    command,
                );
            }
            match current.as_ref().map(|scope| &scope.target) {
                Some(Target::Joined { run, source }) => {
                    (false, Some(*run), source.clone(), String::new(), command)
                }
                Some(Target::Own {
                    source,
                    description,
                    made,
                }) => (false, *made, source.clone(), description.clone(), command),
                Some(Target::Window) | None => (
                    true,
                    state.window.as_ref().map(|window| window.run),
                    Source::new(SourceKind::Dj, "dj"),
                    WINDOW_RUN.to_owned(),
                    command,
                ),
            }
        });
        let done = std::mem::take(&mut state.done);
        let dj_snapshot = None;
        let id = sidecar.begin_intent_with_snapshot(
            &Intent {
                source: &source,
                run,
                description: &description,
                command: command.as_ref(),
                at: now(),
            },
            &done,
            dj_snapshot.as_ref(),
        );
        let id = match id {
            Ok(id) => id,
            Err(error) => {
                // Still to be removed with the next intent.
                state.done = done;
                return Err(error.to_string());
            }
        };
        state.op = Some(Op {
            id,
            wrote: None,
            recorded: false,
            unnamed: false,
            window,
            run,
            source,
            description,
            command,
            dj_snapshot,
            mutant_plan: None,
        });
        drop(state);
        if let Some(delay) = self.faults.delay_after_begin {
            std::thread::sleep(delay);
        }
        Ok(())
    }
}

impl rbl_library::ledger::Ledger for Keeper {
    fn bulk(&self, kind: rbl_library::bulk::Kind) -> Option<Arc<dyn rbl_library::bulk::Recorder>> {
        Some(bulk::make(self, kind))
    }

    fn begin(&self) -> Result<(), String> {
        if let Some(active) = bulk::active(self) {
            active.set_bulk(false);
        }
        self.begin_operation(None)
    }

    fn begin_unkept(&self) -> Result<(), String> {
        if let Some(active) = bulk::active(self) {
            active.set_bulk(true);
            return active.begin();
        }
        self.begin_operation(None)
    }

    fn begin_file(&self, kind: &str, track: &str, values: &Value) -> Result<(), String> {
        if let Some(active) = bulk::active(self) {
            active.set_bulk(true);
            return active.begin();
        }
        let mut args = values.as_object().cloned().unwrap_or_default();
        args.insert("kind".to_owned(), Value::from(kind));
        args.insert("track".to_owned(), Value::from(track));
        self.begin_operation(Some(
            serde_json::json!({"name": "files.edit", "args": args}),
        ))?;
        if self.faults.abort_before_file_publish {
            std::process::abort();
        }
        Ok(())
    }

    fn file_written(&self, kind: &str, track: &str) {
        if bulk::active(self).is_some_and(|active| active.is_bulk()) {
            // FileRecord has already called written(true).
            return;
        }
        let command = self.state().op.as_ref().and_then(|op| op.command.clone());
        let args = command.as_ref().and_then(|command| command.get("args"));
        let reason =
            if let Some(reason) = args.and_then(|args| args["not_reversible_reason"].as_str()) {
                reason
            } else if args.is_some_and(|args| args.get("before").is_some()) {
                "Unwind does not yet reverse analysis file edits"
            } else {
                "previous analysis bytes were not retained"
            };
        let label = format!("{kind}: track {track}. Cannot reverse: {reason}.");
        let action = match args.and_then(|args| args["step"].as_str()) {
            Some("undo") => Action::Undo,
            Some("redo") => Action::Redo,
            _ => Action::Write,
        };
        self.append(Draft {
            action,
            kind,
            label: &label,
            reverses: None,
            part_of: None,
            payload: None,
            // A file edit moves no playlist entry.
            follow: Ok(wreckordbox_sidecar::mutant_follow::Follow::default()),
            plan: None,
        });
    }

    fn prepare(&self, db: &rbl_db::Library, targets: &rbl_library::ledger::Preparation) {
        self.plan_mutants(db, targets);
        let mut state = self.state();
        let Some(op) = state
            .op
            .as_mut()
            .filter(|op| op.source.kind == SourceKind::Dj)
        else {
            return;
        };
        let snapshot = crate::dj_facts::snapshot(db, targets, op.command.as_ref());
        if snapshot.is_null() {
            return;
        }
        op.dj_snapshot = Some(snapshot.clone());
        if let Ok(sidecar) = self.stems.sidecar() {
            if let Err(error) = sidecar.capture_dj_intent(op.id, &snapshot) {
                tracing::warn!(%error, "bounded DJ recovery evidence could not be saved; normal recording continues");
            }
        }
    }

    fn written(&self, wrote: bool) {
        if let Some(active) = bulk::active(self).filter(|active| active.is_bulk()) {
            active.written(wrote);
            if wrote && self.faults.abort_after_write {
                std::process::abort();
            }
            return;
        }
        if let Some(op) = self.state().op.as_mut() {
            op.wrote = Some(wrote);
            if !wrote {
                if let Ok(sidecar) = self.stems.sidecar() {
                    if let Err(error) = sidecar.settle_intents(&[op.id]) {
                        tracing::error!(%error, "could not clear a failed Library write's intent");
                    }
                }
            }
        }
        if wrote && self.faults.abort_after_write {
            std::process::abort();
        }
    }

    fn record(&self, step: Step, edit: &LibraryEdit, label: &str, link: Link) -> Option<i64> {
        let sidecar = self.stems.sidecar().ok();
        // The plan the write left with its intent, before it was made.
        let plan = self
            .state()
            .op
            .as_ref()
            .filter(|op| !op.recorded)
            .and_then(|op| op.mutant_plan.clone());
        let follow = crate::mutant_follow::of_edit(
            &self.location,
            sidecar.as_deref(),
            step,
            edit,
            plan.as_ref(),
        );
        self.append(Draft {
            action: match step {
                Step::Edit => Action::Edit,
                Step::Undo => Action::Undo,
                Step::Redo => Action::Redo,
            },
            kind: stored::kind_of(edit),
            label,
            reverses: link.reverses,
            part_of: link.part_of,
            payload: stored::store(edit),
            follow,
            plan,
        })
    }

    fn note(&self, kept: Unkept, name: Option<&str>) {
        if bulk::active(self).is_some_and(|active| active.is_bulk()) {
            return;
        }
        // Named by the caller, else by the command being answered. A write
        // with neither is not an edit (analysis, an import, a player): not
        // recorded here (bead wbx-14.11), and its intent needs no entry.
        let command =
            CURRENT.with(|current| current.borrow().as_ref().map(|scope| scope.name.clone()));
        let Some(label) = name.map(str::to_owned).or(command) else {
            if let Some(op) = self.state().op.as_mut() {
                op.unnamed = true;
                if let Ok(sidecar) = self.stems.sidecar() {
                    if let Err(error) = sidecar.settle_intents(&[op.id]) {
                        tracing::error!(%error, "could not settle unnamed write");
                    }
                }
            }
            return;
        };
        // Rows removed for good (Remove from Collection) name no entries:
        // the Mutants are brought into line with the Library instead.
        let follow = match kept {
            Unkept::Permanent => self
                .stems
                .sidecar()
                .map_err(|error| error.message)
                .and_then(|sidecar| crate::mutant_follow::reconcile(&self.location, &sidecar)),
            Unkept::Unrecorded => Ok(wreckordbox_sidecar::mutant_follow::Follow::default()),
        };
        self.append(Draft {
            action: Action::Edit,
            kind: match kept {
                Unkept::Unrecorded => UNRECORDED,
                Unkept::Permanent => PERMANENT,
            },
            label: &label,
            reverses: None,
            part_of: None,
            payload: None,
            follow,
            plan: None,
        });
    }
}

/// A Run, ended at once, for an entry written after the Run it was made
/// in could be known.
fn late_run(sidecar: &Sidecar, source: &Source, description: &str) -> Result<RunId, String> {
    let description = if description.is_empty() {
        "Changes recorded late"
    } else {
        description
    };
    let run = sidecar
        .begin_run(source, description)
        .map_err(|e| e.to_string())?;
    sidecar
        .finish_run(run, "recorded late")
        .map_err(|e| e.to_string())?;
    Ok(run)
}

/// Reads every kept entry in `file`, or none: a line that is not a whole
/// entry stops recovery with the file left as it is.
fn read_spill(file: &std::path::Path) -> Result<Vec<Spilled>, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("{} could not be read: {e}", file.display()))?;
    let mut read = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let spilled: Spilled = serde_json::from_str(line).map_err(|e| {
            format!(
                "{} holds, at line {}, something that is not a kept entry ({e}). It is left as it is and nothing is changed until it is dealt with: once the changes it records are known, move it aside",
                file.display(),
                number + 1
            )
        })?;
        read.push(spilled);
    }
    Ok(read)
}

/// Where entries the Ledger could not take wait.
fn spill_file(sidecar: &Sidecar) -> PathBuf {
    sidecar.dir().join(SPILL_FILE)
}

/// What a command's own Run is called when the caller did not say.
fn described(request: &Request) -> String {
    const LONGEST: usize = 160;
    let mut text = format!("{} {}", request.command, request.args);
    if text.chars().count() > LONGEST {
        text = text.chars().take(LONGEST - 1).collect();
        text.push('…');
    }
    text
}

/// Adds one entry to the spill file and waits for it to reach the disk.
fn spill(file: &std::path::Path, entry: &Spilled) -> Result<(), String> {
    let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| e.to_string())?;
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    out.sync_all().map_err(|e| e.to_string())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}
