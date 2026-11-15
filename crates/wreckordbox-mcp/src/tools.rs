//! The tools: each one service command, with a typed input schema whose
//! properties are the command's own arguments.
//!
//! A read tool's name starts `read_` and its description `Read only.`; a
//! changing tool's name starts `change_` and its description says what it
//! changes. The arguments a tool takes are passed to the service as they
//! are, with the fixed ones of [`Tool::fixed`] added, so it is the service
//! that checks them and says what is wrong.

use serde_json::{json, Map, Value};

/// What a tool's command can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing: a read.
    Read,
    /// The Library, through the service's edit path.
    Library,
    /// The Sidecar beside the Library (Facts, Runs, Decisions, Mutants).
    Sidecar,
    /// The Library Protection switch.
    Protection,
}

/// How a tool says whose work a change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// It takes no Run and no source.
    Nobody,
    /// A Library edit: `run`, `source` and `description` are sent with the
    /// request as its Run, as the CLI's `--run`, `--source` and
    /// `--description` are.
    Request,
    /// A Sidecar write whose arguments carry `run`, `source` and
    /// `description` themselves.
    InArgs,
    /// `decisions.answer`: the source is the Decider.
    Decider,
    /// `decisions.create`: a Run and the asker's source, no description.
    RunAndSource,
    /// `runs.start`: the source is the Run's.
    Starts,
}

/// One tool.
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: &'static str,
    /// The service command it sends.
    pub command: &'static str,
    pub effect: Effect,
    pub who: Who,
    pub description: String,
    /// The command's own arguments: JSON-schema properties.
    pub properties: Map<String, Value>,
    pub required: Vec<&'static str>,
    /// Arguments always sent, whatever the tool was given.
    pub fixed: Map<String, Value>,
}

impl Tool {
    /// Whether the tool changes anything.
    #[must_use]
    pub fn changes(&self) -> bool {
        self.effect != Effect::Read
    }

    /// The input schema: the command's arguments, and the Run arguments
    /// its kind of attribution takes. Nothing else is accepted.
    #[must_use]
    pub fn input_schema(&self) -> Value {
        let mut properties = self.properties.clone();
        for (name, schema) in run_properties(self.who) {
            properties.insert(name.to_owned(), schema);
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": self.required,
            "additionalProperties": false,
        })
    }

    /// The tool as `tools/list` lists it.
    #[must_use]
    pub fn listed(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema(),
            "annotations": {
                "readOnlyHint": !self.changes(),
                "destructiveHint": false,
                "idempotentHint": !self.changes(),
                "openWorldHint": false,
            },
        })
    }
}

/// The source argument: never the DJ.
fn source_property() -> Value {
    json!({
        "type": "string",
        "pattern": "^(agent|tool|web):.+$",
        "description": "Who the work is by: `agent:NAME`, `tool:NAME` or `web:ADDRESS`. Never the DJ. \
                        Left out (and with no `run`): this agent, as `agent:<the MCP client's name>`."
    })
}

fn run_properties(who: Who) -> Vec<(&'static str, Value)> {
    let run = json!({"type": "integer", "description": "The open Run this belongs to, from `change_run_start`. Left out: a Run of its own."});
    let description = json!({"type": "string", "description": "What the work is, for whoever reviews the Run later (only without `run`)."});
    match who {
        Who::Nobody => vec![],
        Who::Request | Who::InArgs | Who::Decider => vec![
            ("run", run),
            ("source", source_property()),
            ("description", description),
        ],
        Who::RunAndSource => vec![("run", run), ("source", source_property())],
        Who::Starts => vec![("source", source_property())],
    }
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn integer(description: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": description})
}

fn number(description: &str) -> Value {
    json!({"type": "number", "description": description})
}

fn boolean(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn texts(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

const TRACK_ID: &str = "The track's identifier: `id` in `read_tracks` or `read_tracks_query`.";
const LIMIT: &str = "The most rows to answer.";
const OFFSET: &str = "How many rows to skip; each page's `next_offset` fetches the next.";
const LIBRARY_EDIT: &str =
    "Changes the Library through its single edit path: one step of the window's Undo, \
     recorded in the Ledger in a Run, seen at once by the window and every client, and refused \
     (`library_protected`) while Library Protection is on.";
const SIDECAR_WRITE: &str = "Changes the Sidecar, never the Library.";

struct Builder {
    tools: Vec<Tool>,
}

impl Builder {
    #[allow(clippy::too_many_arguments, reason = "one row of the catalogue")]
    fn add(
        &mut self,
        name: &'static str,
        command: &'static str,
        effect: Effect,
        who: Who,
        description: &str,
        properties: Vec<(&str, Value)>,
        required: Vec<&'static str>,
        fixed: Value,
    ) {
        let lead = match effect {
            Effect::Read => "Read only. Changes nothing.",
            Effect::Library => LIBRARY_EDIT,
            Effect::Sidecar => SIDECAR_WRITE,
            Effect::Protection => "Changes Library Protection, not the Library.",
        };
        self.tools.push(Tool {
            name,
            command,
            effect,
            who,
            description: format!("{lead} {description}"),
            properties: properties
                .into_iter()
                .map(|(name, schema)| (name.to_owned(), schema))
                .collect(),
            required,
            fixed: match fixed {
                Value::Object(fixed) => fixed,
                _ => Map::new(),
            },
        });
    }

    fn read(
        &mut self,
        name: &'static str,
        command: &'static str,
        description: &str,
        properties: Vec<(&str, Value)>,
        required: Vec<&'static str>,
    ) {
        self.add(
            name,
            command,
            Effect::Read,
            Who::Nobody,
            description,
            properties,
            required,
            json!({}),
        );
    }
}

/// Every tool, in the order `tools/list` gives them.
#[allow(clippy::too_many_lines, reason = "the catalogue: one entry per tool")]
#[must_use]
pub fn catalogue() -> Vec<Tool> {
    let mut b = Builder { tools: Vec::new() };

    // Reads.
    b.read(
        "read_status",
        "status",
        "Whether the Library service is running and which Library it has open (`wreckordbox status`).",
        vec![],
        vec![],
    );
    b.read(
        "read_tracks",
        "tracks.list",
        "Tracks in the window's order, six fields each, matching the window's search box (`wreckordbox tracks list`).",
        vec![
            ("filter", text("Every word must appear in the track's details, in any case.")),
            ("limit", integer(LIMIT)),
        ],
        vec![],
    );
    b.read(
        "read_track",
        "tracks.get",
        "Everything the Library holds about one track (`wreckordbox tracks show`).",
        vec![("id", text(TRACK_ID))],
        vec!["id"],
    );
    b.read(
        "read_tracks_query",
        "tracks.query",
        "Tracks by any field, MyTag or playlist, a page at a time; every argument given must hold \
         (`wreckordbox tracks query`). A number or date is one value (`2021`) or an inclusive range \
         (`2019..2021`, `2019..`, `..2021`).",
        vec![
            ("search", text("As the window's search box.")),
            ("title", text("Title contains this, any case.")),
            ("artist", text("Artist contains this.")),
            ("album", text("Album contains this.")),
            ("genre", text("Genre contains this.")),
            ("label", text("Label contains this.")),
            ("comment", text("Comment contains this.")),
            ("location", text("The audio file's path contains this.")),
            ("year", text("A year or a range of years.")),
            ("bpm", text("`128` (as the Track Filter lists 128), `128.03` exactly, or `124..128.5`.")),
            ("bpm_ranges", texts("Tempo ranges to the hundredth (`123.50..128.49`): any of them.")),
            ("keys", texts("Keys as the Library spells them (`8A`): any of them.")),
            ("energies", texts("Energy (the star rating) 1 to 5 or a range, `0` for none: any of them.")),
            ("prep_colours", texts("Prep colours (colour labels) or `none`: any of them.")),
            ("duration", text("Seconds or M:SS, or a range.")),
            ("date_added", text("A day (YYYY-MM-DD) or a range of days.")),
            ("play_count", text("A count or a range.")),
            ("analysed", boolean("Analysed, or not.")),
            ("my_tags", texts("MyTag names: the track carries every one.")),
            (
                "my_tag_picks",
                json!({
                    "type": "array",
                    "description": "MyTags picked per category, as the Track Filter picks them.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "category": {"type": "string"},
                            "tags": {"type": "array", "items": {"type": "string"}},
                            "any": {"type": "boolean", "description": "Any of the tags passes, not only all."}
                        },
                        "required": ["category", "tags", "any"],
                        "additionalProperties": false
                    }
                }),
            ),
            ("playlist", text("Only this playlist's tracks, by name or identifier; a folder is everything beneath it.")),
            ("limit", integer(LIMIT)),
            ("offset", integer(OFFSET)),
        ],
        vec![],
    );
    b.read(
        "read_mytags",
        "mytags.list",
        "The MyTag categories and their tags, with identifiers and how many tracks carry each (`wreckordbox mytags list`).",
        vec![],
        vec![],
    );
    b.read(
        "read_track_mytags",
        "mytags.of_track",
        "The MyTags one track carries (`wreckordbox mytags show`).",
        vec![("id", text(TRACK_ID))],
        vec!["id"],
    );
    b.read(
        "read_mytag_tracks",
        "mytags.tracks",
        "The tracks carrying one MyTag (`wreckordbox mytags tracks`).",
        vec![
            ("id", text("The tag's identifier, from `read_mytags`.")),
            ("limit", integer(LIMIT)),
        ],
        vec!["id"],
    );
    b.read(
        "read_playlists",
        "playlists.tree",
        "The playlist tree: folders, playlists and intelligent playlists (`wreckordbox playlists tree`).",
        vec![],
        vec![],
    );
    b.read(
        "read_playlist_tracks",
        "playlists.tracks",
        "A playlist's tracks in order, each with its `entry_id` (`wreckordbox playlists tracks`).",
        vec![
            (
                "id",
                text("The playlist's identifier, from `read_playlists`."),
            ),
            ("limit", integer(LIMIT)),
        ],
        vec!["id"],
    );
    b.read(
        "read_history",
        "history.list",
        "The occasions play history was recorded on (`wreckordbox history list`).",
        vec![],
        vec![],
    );
    b.read(
        "read_history_tracks",
        "history.tracks",
        "What was played on one occasion, in order (`wreckordbox history tracks`).",
        vec![
            (
                "id",
                text("The occasion's identifier, from `read_history`."),
            ),
            ("limit", integer(LIMIT)),
        ],
        vec!["id"],
    );
    b.read(
        "read_protection",
        "protection.get",
        "Whether Library Protection is on: while it is, every Library change is refused (`wreckordbox protection`).",
        vec![],
        vec![],
    );
    b.read(
        "read_intake",
        "intake.list",
        "Tracks waiting for Intake (no Prep colour), each with what it still needs (`wreckordbox intake list`).",
        vec![("limit", integer(LIMIT)), ("offset", integer(OFFSET))],
        vec![],
    );
    b.read(
        "read_problems",
        "intake.problems",
        "Tracks marked Problem, each with what it still needs (`wreckordbox intake problems`).",
        vec![
            ("limit", integer(LIMIT)),
            ("offset", integer(OFFSET)),
            (
                "label",
                text("The colour label meaning Problem, when it is not called Problem."),
            ),
        ],
        vec![],
    );
    b.read(
        "read_intake_counts",
        "intake.counts",
        "How many tracks wait in Intake and in Problems (`wreckordbox intake count`).",
        vec![(
            "label",
            text("The colour label meaning Problem, when it is not called Problem."),
        )],
        vec![],
    );
    b.read(
        "read_prep_colours",
        "intake.colours",
        "The eight colour labels with the DJ's names, and which mean Clean and Problem (`wreckordbox intake colours`).",
        vec![],
        vec![],
    );
    b.read(
        "read_track_tag_panel",
        "intake.tag_panel",
        "A track's tags as Intake's tag panel shows them: every dimension and what the track has (`wreckordbox intake tags`).",
        vec![("id", text(TRACK_ID))],
        vec!["id"],
    );
    b.read(
        "read_fact_kinds",
        "facts.kinds",
        "The declared kinds of Fact, each a Judgement or a Measurement, and its value type (`wreckordbox facts kinds`).",
        vec![],
        vec![],
    );
    b.read(
        "read_track_facts",
        "facts.of_track",
        "The Facts held for one track, newest first (`wreckordbox facts show`).",
        vec![
            ("id", text(TRACK_ID)),
            ("kind", text("Only Facts of this kind.")),
            ("all", boolean("Also stale, unverified and replaced Facts.")),
            (
                "check_audio",
                boolean("Verify Measurements against the audio (records nothing)."),
            ),
        ],
        vec!["id"],
    );
    b.read(
        "read_standing_fact",
        "facts.standing",
        "Which Fact stands for a track and kind, why, and what it superseded (`wreckordbox facts standing`).",
        vec![("id", text(TRACK_ID)), ("kind", text("The kind of Fact."))],
        vec!["id", "kind"],
    );
    b.read(
        "read_runs",
        "runs.list",
        "Runs, newest first, with how many changes each made (`wreckordbox runs list`).",
        vec![("limit", integer(LIMIT)), ("offset", integer(OFFSET))],
        vec![],
    );
    b.read(
        "read_run",
        "runs.show",
        "One Run and every change it made, with the value before and after (`wreckordbox runs show`).",
        vec![
            ("id", integer("The Run's identifier.")),
            ("limit", integer(LIMIT)),
            ("offset", integer(OFFSET)),
        ],
        vec!["id"],
    );
    b.read(
        "read_decisions",
        "decisions.list",
        "Decisions, oldest first (`wreckordbox decisions list|pending|answered`).",
        vec![
            (
                "state",
                json!({"type": "string", "enum": ["pending", "answered", "all"], "description": "Which Decisions; `all` when left out."}),
            ),
            ("track", text("Only Decisions about this track.")),
            ("topic", text("Only Decisions of this topic.")),
            ("limit", integer(LIMIT)),
            ("offset", integer(OFFSET)),
        ],
        vec![],
    );
    b.read(
        "read_decision",
        "decisions.get",
        "One Decision with every answer it has been given (`wreckordbox decisions show`).",
        vec![("id", integer("The Decision's identifier."))],
        vec!["id"],
    );
    b.read(
        "read_mutants",
        "mutants.list",
        "The Mutants, or those of one playlist or track (`wreckordbox mutants list`).",
        vec![
            ("playlist", text("Only this playlist's Mutants.")),
            ("track", text("Only Mutants of this track.")),
            (
                "check_audio",
                boolean("Verify audio where its cached verdict is missing."),
            ),
        ],
        vec![],
    );
    b.read(
        "read_mutant",
        "mutants.get",
        "One Mutant: its entry, parent track, edits and history (`wreckordbox mutants show`).",
        vec![
            ("id", integer("The Mutant's identifier.")),
            ("entry", text("Or the playlist entry it belongs to.")),
            (
                "check_audio",
                boolean("Verify audio where its cached verdict is missing."),
            ),
        ],
        vec![],
    );
    b.read(
        "read_review_waiting",
        "review.waiting",
        "The Facts Projection held back for the DJ to decide (`wreckordbox review waiting`). Show them to the \
         DJ: accepting or rejecting them is the DJ's decision and is not a tool.",
        vec![
            ("tracks", texts("Only these tracks.")),
            ("field", json!({"type": "string", "enum": ["energy", "prep_colour"]})),
            ("at_least", number("Only at or above this confidence, 0 to 1.")),
        ],
        vec![],
    );
    b.read(
        "read_review_rejected",
        "review.rejected",
        "The suggestions the DJ rejected (`wreckordbox review rejected`).",
        vec![
            ("tracks", texts("Only these tracks.")),
            (
                "field",
                json!({"type": "string", "enum": ["energy", "prep_colour"]}),
            ),
        ],
        vec![],
    );
    b.read(
        "read_projection_barriers",
        "projection.barriers",
        "Writes the Ledger cannot account for, which stop Projection filling fields (`wreckordbox projection barriers`).",
        vec![],
        vec![],
    );
    b.read(
        "read_projection_plan",
        "projection.plan",
        "What Projection would write from standing Facts into Energy and Prep colour, and what it holds \
         for the DJ, with the `plan` that `change_project` takes (`wreckordbox project --dry-run`).",
        vec![("tracks", texts("Only these tracks; every track with Facts when left out."))],
        vec![],
    );
    b.read(
        "read_sticks",
        "sticks.list",
        "The mounted Sticks: kind, whether protected, last load and room (`wreckordbox stick list`).",
        vec![],
        vec![],
    );
    b.read(
        "read_stick_load_preview",
        "sticks.dry_run",
        "What loading playlists onto a Stick would do: tracks and playlists added, changed and removed, \
         and the room before and after (`wreckordbox stick dry-run`). Writes nothing. Loading is CLI-only.",
        vec![
            ("stick", text("Where the Stick is mounted, as an absolute path: `path` in `read_sticks`.")),
            ("playlists", texts("The playlists' identifiers; a folder stands for everything in it.")),
        ],
        vec!["stick", "playlists"],
    );
    b.read(
        "read_stems",
        "stems.show",
        "The Stems the Sidecar holds for one track (`wreckordbox stems show`).",
        vec![("id", text(TRACK_ID))],
        vec!["id"],
    );
    b.read(
        "read_intensity",
        "intensity.show",
        "The Intensity the Sidecar holds for one track (`wreckordbox intensity show`).",
        vec![("id", text(TRACK_ID))],
        vec!["id"],
    );
    b.read(
        "read_schema_check",
        "schema.check",
        "Checks the Library's Tag schema and shows the MyTag categories its deck view makes (`wreckordbox schema check`).",
        vec![],
        vec![],
    );

    // Library edits.
    let library = |b: &mut Builder,
                   name: &'static str,
                   command: &'static str,
                   description: &str,
                   properties: Vec<(&str, Value)>,
                   required: Vec<&'static str>,
                   fixed: Value| {
        b.add(
            name,
            command,
            Effect::Library,
            Who::Request,
            description,
            properties,
            required,
            fixed,
        );
    };
    let comments = boolean(
        "Also rewrite the tracks' comments with their MyTags: when the DJ's rekordbox adds My Tags to Comments.",
    );
    library(
        &mut b,
        "change_track_rating",
        "tracks.rate",
        "Sets a track's star rating (its Energy), 0 to 5; 0 clears it (`wreckordbox tracks rate`).",
        vec![
            ("id", text(TRACK_ID)),
            (
                "rating",
                json!({"type": "integer", "minimum": 0, "maximum": 5}),
            ),
        ],
        vec!["id", "rating"],
        json!({"assert": true}),
    );
    library(
        &mut b,
        "change_prep_colour",
        "tracks.set_prep_colour",
        "Sets a track's Prep colour (its colour label): a colour takes it out of Intake, the one named \
         Problem puts it in Problems (`wreckordbox intake colour`).",
        vec![
            ("id", text(TRACK_ID)),
            ("prep_colour", text("The DJ's name for a label (`Clean`), the colour (`Green`), or `none`.")),
            ("colour_id", json!({"type": "integer", "minimum": 0, "maximum": 8, "description": "Or the label's number, 0 for none."})),
        ],
        vec!["id"],
        json!({"assert": true}),
    );
    library(
        &mut b,
        "change_mytag_assign",
        "mytags.assign",
        "Puts a MyTag on tracks; a track that has it is left as it is (`wreckordbox mytags assign`).",
        vec![
            ("id", text("The tag's identifier, from `read_mytags`.")),
            ("tracks", texts("The tracks' identifiers.")),
            ("comments", comments.clone()),
        ],
        vec!["id", "tracks"],
        json!({"assert": true}),
    );
    library(
        &mut b,
        "change_mytag_unassign",
        "mytags.unassign",
        "Takes a MyTag off tracks; the tag stays (`wreckordbox mytags unassign`).",
        vec![
            ("id", text("The tag's identifier, from `read_mytags`.")),
            ("tracks", texts("The tracks' identifiers.")),
            ("comments", comments.clone()),
        ],
        vec!["id", "tracks"],
        json!({"assert": true}),
    );
    library(
        &mut b,
        "change_mytag_create",
        "mytags.create",
        "Adds a MyTag at the end of a category (`wreckordbox mytags create`).",
        vec![
            (
                "category",
                text("The category's identifier, from `read_mytags`."),
            ),
            ("name", text("The new tag's name, unique in its category.")),
        ],
        vec!["category", "name"],
        json!({}),
    );
    library(
        &mut b,
        "change_mytag_rename",
        "mytags.rename",
        "Renames a MyTag or a category; tracks keep the tag (`wreckordbox mytags rename`).",
        vec![
            ("id", text("The tag's or category's identifier.")),
            ("name", text("The new name.")),
            ("comments", comments.clone()),
        ],
        vec!["id", "name"],
        json!({}),
    );
    library(
        &mut b,
        "change_mytag_move",
        "mytags.move",
        "Moves a MyTag within its category, or a category among categories (`wreckordbox mytags move`).",
        vec![
            ("id", text("The tag's or category's identifier.")),
            ("position", json!({"type": "integer", "minimum": 1, "description": "The place, from 1; past the end is the end."})),
        ],
        vec!["id", "position"],
        json!({}),
    );
    library(
        &mut b,
        "change_track_tags",
        "intake.set_tags",
        "Sets one dimension of a track to exactly these tags, none clearing it, as a click in Intake's tag \
         panel does: a deck-facing dimension as MyTags, any other as a Fact (`wreckordbox intake tag`).",
        vec![
            ("id", text(TRACK_ID)),
            ("dimension", text("The dimension's `id` in `read_track_tag_panel`.")),
            ("terms", texts("The tags' identifiers; in the genre tree the most specific (a parent is implied).")),
            ("drop_unmapped", texts("Values no tag stands for, to take off too.")),
            ("comments", comments),
        ],
        vec!["id", "dimension", "terms"],
        json!({"assert": true}),
    );
    library(
        &mut b,
        "change_playlist_copy",
        "playlists.copy",
        "Copies a playlist beside it, with its tracks in order and a copy of each Mutant (`wreckordbox playlists copy`).",
        vec![
            ("playlist", text("The playlist's identifier.")),
            ("name", text("The copy's name; left out, the name and \"copy\".")),
        ],
        vec!["playlist"],
        json!({}),
    );
    library(
        &mut b,
        "change_playlist_remove_tracks",
        "playlists.remove",
        "Takes every entry of these tracks out of a playlist, with their Mutants (`wreckordbox playlists remove`).",
        vec![
            ("playlist", text("The playlist's identifier.")),
            ("tracks", texts("The tracks' identifiers.")),
        ],
        vec!["playlist", "tracks"],
        json!({}),
    );
    library(
        &mut b,
        "change_playlist_reorder",
        "playlists.reorder",
        "Puts a playlist's entries in a new order; Mutants stay on their entries (`wreckordbox playlists reorder`).",
        vec![
            ("playlist", text("The playlist's identifier.")),
            ("entries", texts("Every `entry_id` of the playlist once, in the new order.")),
        ],
        vec!["playlist", "entries"],
        json!({}),
    );
    library(
        &mut b,
        "change_project",
        "projection.apply",
        "Applies Projection: standing Facts into Energy and Prep colour, exactly as the plan from \
         `read_projection_plan` listed, else refused with nothing written; never over a value the DJ set \
         by hand (`wreckordbox project --expect`).",
        vec![
            ("expect", text("The `plan` that `read_projection_plan` answered.")),
            ("tracks", texts("The same tracks given to `read_projection_plan`, if any.")),
        ],
        vec!["expect"],
        json!({}),
    );
    b.add(
        "change_protection_on",
        "protection.set",
        Effect::Protection,
        Who::Nobody,
        "Turns Library Protection on: every Library change is refused, here and in the window, until the \
         DJ turns it off (`wreckordbox protection on`). Turning it off is the DJ's decision and not a tool.",
        vec![],
        vec![],
        json!({"protect_library": true}),
    );

    // Sidecar writes.
    b.add(
        "change_run_start",
        "runs.start",
        Effect::Sidecar,
        Who::Starts,
        "Starts a Run: one piece of work whose changes belong together. Pass its `id` as `run` to every \
         change of the work, then `change_run_finish` (`wreckordbox runs start`).",
        vec![("description", text("What the work is, for the DJ."))],
        vec!["description"],
        json!({}),
    );
    b.add(
        "change_run_finish",
        "runs.finish",
        Effect::Sidecar,
        Who::Nobody,
        "Ends a Run; nothing more can be recorded in it (`wreckordbox runs finish`).",
        vec![
            ("id", integer("The Run's identifier.")),
            ("outcome", text("How it went, in a word or a sentence.")),
        ],
        vec!["id"],
        json!({}),
    );
    b.add(
        "change_record_facts",
        "facts.record",
        Effect::Sidecar,
        Who::InArgs,
        "Records Facts about tracks, all or none, in one Run (`wreckordbox facts record`, `facts record-many`). \
         A kind must be declared (`read_fact_kinds`) and its value of the kind's type.",
        vec![
            (
                "facts",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "track": {"type": "string"},
                            "kind": {"type": "string"},
                            "value": {"description": "Of the kind's type: text, a list of texts, a number, ..."},
                            "confidence": {"type": "number", "minimum": 0, "maximum": 1}
                        },
                        "required": ["track", "kind", "value"],
                        "additionalProperties": false
                    }
                }),
            ),
            ("confidence", json!({"type": "number", "minimum": 0, "maximum": 1, "description": "How sure, 0 to 1, for every Fact that does not say."})),
        ],
        vec!["facts"],
        json!({}),
    );
    b.add(
        "change_decision_create",
        "decisions.create",
        Effect::Sidecar,
        Who::RunAndSource,
        "Asks a Decision: a question between fixed candidates, pending until a Decider answers (`wreckordbox decisions create`).",
        vec![
            ("question_type", json!({"type": "string", "enum": ["choice", "yes_no"]})),
            ("question", text("The question; for yes_no, the statement that is true or not.")),
            ("candidates", texts("The candidates' names: two or more for `choice`, none for `yes_no`.")),
            ("context", json!({"type": "object", "description": "What a Decider answers from."})),
            ("topic", text("What kind of question, in lower-case letters, digits and underscores.")),
            ("track", text("The track the question is about.")),
        ],
        vec!["question_type", "question"],
        json!({}),
    );
    b.add(
        "change_decision_answer",
        "decisions.answer",
        Effect::Sidecar,
        Who::Decider,
        "Answers a Decision as a Decider, this agent unless `source` says otherwise (`wreckordbox decisions answer`).",
        vec![
            ("id", integer("The Decision's identifier.")),
            ("choice", text("A candidate's name, exactly.")),
            ("probability", json!({"type": "number", "minimum": 0, "maximum": 1, "description": "The probability that the choice is right."})),
            ("reason", text("Why, in a sentence.")),
        ],
        vec!["id", "choice"],
        json!({}),
    );
    b.add(
        "change_mutant_create",
        "mutants.create",
        Effect::Sidecar,
        Who::InArgs,
        "Makes a Mutant of one playlist entry: its own version of its track. Nothing audible changes until \
         it is edited (`wreckordbox mutants create`).",
        vec![
            ("entry", text("The entry's `entry_id`, from `read_playlist_tracks`.")),
            ("playlist", text("Or the playlist ...")),
            ("position", json!({"type": "integer", "minimum": 1, "description": "... and the entry's place in it, from 1."})),
        ],
        vec![],
        json!({}),
    );
    b.add(
        "change_mutant_edit",
        "mutants.edit",
        Effect::Sidecar,
        Who::InArgs,
        "Sets edits on a Mutant together: `key_shift`, `tempo`, `stems` (vocals, melody, bass, drums), \
         `cues` with `kind` and `position_ms` (`wreckordbox mutants edit`).",
        vec![
            ("id", integer("The Mutant's identifier.")),
            ("entry", text("Or its playlist entry.")),
            ("edits", json!({"type": "object", "description": "Omitted fields keep their current edits."})),
        ],
        vec!["edits"],
        json!({}),
    );
    b.add(
        "change_mutant_remove",
        "mutants.remove",
        Effect::Sidecar,
        Who::InArgs,
        "Removes a Mutant: its entry is the track again (`wreckordbox mutants remove`).",
        vec![
            ("id", integer("The Mutant's identifier.")),
            ("entry", text("Or its playlist entry.")),
        ],
        vec![],
        json!({}),
    );
    b.tools
}
