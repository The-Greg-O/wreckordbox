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
//! # When the Sidecar cannot be written
//!
//! Before any write to the Library the core asks [`Keeper::ready`]: the
//! Sidecar must open and take a write, or the Library write is refused
//! and nothing changes. An entry is appended after the Library write it
//! records (they are two databases). If that append fails, the entry is
//! kept in memory and in [`SPILL_FILE`] in the Sidecar's folder, and is
//! written to the Ledger before the next Library write is let through.
//!
//! # Locks
//!
//! The Keeper is called under the Library's edit gate, and for an entry
//! under the edit history's lock. It takes only its own state lock and
//! the Sidecar's, both leaves: it never takes the service's Library lock,
//! the edit gate or the history, and never calls a host or sends an
//! event.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rbl_library::history::LibraryEdit;
use rbl_library::ledger::{Step, Unkept};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::ledger::{Action, Count, NewEntry, Store};
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
    /// The service command that made it: `{"name", "args"}`.
    pub command: Option<Value>,
    /// The entry an Undo or Redo steps.
    pub reverses: Option<i64>,
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
}

/// What `runs.show` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunShown {
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
            LibraryEdit::Track(_) => "track",
            LibraryEdit::KeyEdit { .. } => "key_edit",
            LibraryEdit::TrackTags { .. } => "track_tags",
            LibraryEdit::MyTagOrder(_) => "my_tag_order",
            LibraryEdit::Cues(_) => "cues",
            LibraryEdit::CreatePlaylist(_) => "create_playlist",
            LibraryEdit::PlaylistEdit(_) => "playlist_edit",
            LibraryEdit::Relocate(_) => "relocate",
            LibraryEdit::ListRemoval(_) => "list_removal",
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
        || crate::mytag_edits::answers(command)
}

pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::RUNS_LIST | commands::RUNS_SHOW)
}

/// Answers `runs.list` and `runs.show`. Reading never makes a Sidecar:
/// with none there are no Runs.
pub(crate) fn answer(stems: &Stems, request: &Request) -> Result<Value, ServiceError> {
    let bad = |e: serde_json::Error| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    };
    let sidecar = stems.sidecar_if_made()?;
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
        reversible: entry.reversible,
        format: entry.format,
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
    /// A Run that was open before the command and stays open.
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
    reversible: bool,
    format: u32,
    payload: Option<Value>,
}

struct WindowRun {
    run: RunId,
    last_edit: Instant,
}

#[derive(Default)]
struct State {
    window: Option<WindowRun>,
    /// Whether what an earlier service left behind has been dealt with:
    /// its spill file read, its window Run ended.
    settled: bool,
    /// Entries made and not yet in the Ledger, oldest first. Also in the
    /// spill file, when that could be written.
    unwritten: Vec<Spilled>,
}

/// One entry on its way into the Ledger.
struct Draft<'a> {
    action: Action,
    kind: &'a str,
    label: &'a str,
    reverses: Option<i64>,
    payload: Option<Value>,
}

/// Keeps the Ledger for one service: the `Ledger` its Library core tells.
pub(crate) struct Keeper {
    stems: Arc<Stems>,
    pause: Duration,
    state: Mutex<State>,
}

impl Keeper {
    pub(crate) fn new(stems: Arc<Stems>) -> Arc<Self> {
        let pause = std::env::var(WINDOW_PAUSE_ENV)
            .ok()
            .and_then(|secs| secs.parse::<u64>().ok())
            .map_or(WINDOW_PAUSE, Duration::from_secs);
        Arc::new(Self {
            stems,
            pause,
            state: Mutex::new(State::default()),
        })
    }

    /// Answers one command with its changes attributed: runs `answer` on
    /// this thread and gives back its result beside the Run the command's
    /// changes are in, when it changed anything or joined a Run.
    ///
    /// `from_client` is whether a client asked (a connection) or the
    /// process hosting the service did. A Run named on the request is
    /// checked before anything is done.
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
        let scope = Scope {
            target,
            command: json!({"name": request.command, "args": request.args}),
            name: request.command.clone(),
            entries: 0,
        };
        CURRENT.with(|current| *current.borrow_mut() = Some(scope));
        let result = answer();
        let Some(scope) = CURRENT.with(|current| current.borrow_mut().take()) else {
            return (result, None);
        };
        let run = match scope.target {
            Target::Window => None,
            Target::Joined { run, .. } => Some(run),
            Target::Own { made, .. } => {
                if let (Some(run), Ok(sidecar)) = (made, self.stems.sidecar()) {
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
            let not_found = || {
                ServiceError::new(
                    crate::facts::codes::RUN_NOT_FOUND,
                    format!("there is no Run {id}"),
                )
            };
            let run = self
                .stems
                .sidecar_if_made()?
                .ok_or_else(not_found)?
                .run(RunId(id))
                .map_err(|e| failed(&e))?
                .ok_or_else(not_found)?;
            if run.is_finished() {
                return Err(ServiceError::new(
                    crate::facts::codes::RUN_FINISHED,
                    format!("Run {id} has ended; start another to change more"),
                ));
            }
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

    /// Ends the window's Run, when one is open: the service is stopping.
    pub(crate) fn close(&self) {
        let window = self.state().window.take();
        if let (Some(window), Ok(Some(sidecar))) = (window, self.stems.sidecar_if_made()) {
            let _ = sidecar.finish_run(window.run, "wreckordbox stopped");
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Deals once with what an earlier service left: entries it spilled
    /// and a window Run it never ended.
    fn settle(sidecar: &Sidecar, state: &mut State) -> Result<(), String> {
        if state.settled {
            return Ok(());
        }
        let file = spill_file(sidecar);
        if file.is_file() && state.unwritten.is_empty() {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("{} could not be read: {e}", file.display()))?;
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                let spilled: Spilled = serde_json::from_str(line).map_err(|e| {
                    format!("{} holds a line that is not an entry: {e}", file.display())
                })?;
                state.unwritten.push(spilled);
            }
        }
        sidecar
            .finish_abandoned_runs(
                &Source::new(SourceKind::Dj, "dj"),
                WINDOW_RUN,
                "wreckordbox stopped without ending it",
            )
            .map_err(|e| e.to_string())?;
        state.settled = true;
        Ok(())
    }

    /// Writes every entry still waiting into the Ledger, oldest first, and
    /// removes the spill file.
    fn write_waiting(sidecar: &Sidecar, state: &mut State) -> Result<(), String> {
        while let Some(waiting) = state.unwritten.first() {
            let run = if let Some(run) = waiting.run {
                RunId(run)
            } else {
                let run = sidecar
                    .begin_run(&waiting.source, &waiting.description)
                    .map_err(|e| e.to_string())?;
                sidecar
                    .finish_run(run, "recorded late")
                    .map_err(|e| e.to_string())?;
                run
            };
            sidecar
                .append(&NewEntry {
                    run,
                    store: Store::Library,
                    source: &waiting.source,
                    at: Some(waiting.at),
                    action: waiting.action,
                    kind: &waiting.kind,
                    label: &waiting.label,
                    command: waiting.command.as_ref(),
                    reverses: waiting.reverses,
                    reversible: waiting.reversible,
                    format: waiting.format,
                    payload: waiting.payload.as_ref(),
                })
                .map_err(|e| e.to_string())?;
            state.unwritten.remove(0);
        }
        let file = spill_file(sidecar);
        if file.is_file() {
            std::fs::remove_file(&file)
                .map_err(|e| format!("{} could not be removed: {e}", file.display()))?;
        }
        Ok(())
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
            .begin_run(&Source::new(SourceKind::Dj, "dj"), WINDOW_RUN)
            .map_err(|e| e.to_string())?;
        state.window = Some(WindowRun {
            run,
            last_edit: Instant::now(),
        });
        Ok(run)
    }

    /// Appends one entry for a change already made, attributed to the
    /// command this thread is answering or else to the DJ in the window.
    /// Answers the entry's identifier; `None` when it had to be kept for
    /// later.
    fn append(&self, draft: Draft) -> Option<i64> {
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            let mut state = self.state();
            let sidecar = self.stems.sidecar();
            let dj = Source::new(SourceKind::Dj, "dj");
            // Which Run and whose change, as far as can be told even when
            // the Sidecar cannot be written.
            let (run, source, description) = match (current.as_mut().map(|scope| &mut scope.target), &sidecar) {
                (Some(Target::Joined { run, source }), _) => {
                    (Ok(*run), source.clone(), String::new())
                }
                (Some(Target::Own { source, description, made }), sidecar) => {
                    let run = match (*made, sidecar) {
                        (Some(run), _) => Ok(run),
                        (None, Ok(sidecar)) => sidecar
                            .begin_run(source, description)
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
            };
            let command = current.as_ref().map(|scope| scope.command.clone());
            if let Some(scope) = current.as_mut() {
                scope.entries += 1;
            }
            let at = now();
            let reversible = draft.payload.is_some();
            let written = match (&sidecar, &run) {
                (Ok(sidecar), Ok(run)) if state.unwritten.is_empty() => sidecar
                    .append(&NewEntry {
                        run: *run,
                        store: Store::Library,
                        source: &source,
                        at: Some(at),
                        action: draft.action,
                        kind: draft.kind,
                        label: draft.label,
                        command: command.as_ref(),
                        reverses: draft.reverses,
                        reversible,
                        format: stored::FORMAT,
                        payload: draft.payload.as_ref(),
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
                    let spilled = Spilled {
                        run: run.ok().map(|run| run.0),
                        source,
                        description,
                        at,
                        action: draft.action,
                        kind: draft.kind.to_owned(),
                        label: draft.label.to_owned(),
                        command,
                        reverses: draft.reverses,
                        reversible,
                        format: stored::FORMAT,
                        payload: draft.payload,
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
                    None
                }
            }
        })
    }
}

impl rbl_library::ledger::Ledger for Keeper {
    fn ready(&self) -> Result<(), String> {
        let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
        let mut state = self.state();
        Self::settle(&sidecar, &mut state)?;
        if !state.unwritten.is_empty() {
            Self::write_waiting(&sidecar, &mut state)
                .map_err(|e| format!("an earlier change is still not recorded ({e})"))?;
        }
        sidecar.ledger_ready().map_err(|e| e.to_string())
    }

    fn record(
        &self,
        step: Step,
        edit: &LibraryEdit,
        label: &str,
        reverses: Option<i64>,
    ) -> Option<i64> {
        self.append(Draft {
            action: match step {
                Step::Edit => Action::Edit,
                Step::Undo => Action::Undo,
                Step::Redo => Action::Redo,
            },
            kind: stored::kind_of(edit),
            label,
            reverses,
            payload: stored::store(edit),
        })
    }

    fn note(&self, kept: Unkept, name: Option<&str>) {
        // Named by the caller, else by the command being answered. A write
        // with neither is not an edit (analysis, an import, a player): not
        // recorded here.
        let command =
            CURRENT.with(|current| current.borrow().as_ref().map(|scope| scope.name.clone()));
        let Some(label) = name.map(str::to_owned).or(command) else {
            return;
        };
        self.append(Draft {
            action: Action::Edit,
            kind: match kept {
                Unkept::Unrecorded => UNRECORDED,
                Unkept::Permanent => PERMANENT,
            },
            label: &label,
            reverses: None,
            payload: None,
        });
    }
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
