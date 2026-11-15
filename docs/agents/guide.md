# Driving wreckordbox from an agent

This guide is for an agent outside the app (Claude Code, Codex, a script) that
works on a DJ's Library for them through the `wreckordbox` command, or through
the `wreckordbox-mcp` server that wraps the same service (see
[The MCP server](#the-mcp-server)). There is no agent inside wreckordbox, and
the Library service never uses the network (ADR 0005): everything an agent
does, it does through these commands.

Every command below was taken from the built binary's own `--help` and run
against a fixture Library. A test (`crates/wreckordbox/tests/it/agent_guide.rs`)
fails when the CLI has a command this guide has no section for, so if you add a
command, add its section here.

## Rules that bind you

These come from the repository's `AGENTS.md` and from how the Library is
protected. They are not suggestions.

- **Never unlock a Stick unless the DJ asked for that Stick, by name, to be
  unlocked.** Stick Protection is the only guarantee that nothing writes to a
  Stick; the window's hold-to-confirm is just an interface.
- **Never clear a Projection barrier** (`projection acknowledge`) **unless the
  DJ asked for that barrier to be cleared.**
- **Never accept, reject or reopen a waiting Fact** (`review accept|reject|reopen`)
  **unless the DJ asked for that decision.** These record the act as the DJ's.
- **Never discard a Ledger spill** (`runs spills discard`), and never replay
  one with a manifest you made up: a replay manifest must repeat the source,
  Run and kind the Ledger already recorded; never substitute the DJ or guess.
- **Never turn Library Protection off** (`protection off`) unless the DJ asked
  you to. While it is on, every change you try is refused; that is the DJ
  saying "not now". Turning it back on (`protection on`) is always safe.
- **Never claim to be the DJ.** `--source dj` records a change as the DJ's own
  hand. Use `--source agent:<your-name>` for your work. The DJ-only commands
  above take `--source dj` only because the DJ told you, in this conversation,
  to make that decision for them.
- **Never run a real stem separation** (`stems separate`) without the DJ asking:
  it takes over twenty minutes a track and keeps the machine busy.
- **Never restore a backup** (`backup restore --yes`) or **remove tracks**
  (`tracks remove --confirm`) without the DJ asking: both are destructive.
- **Preview before you write to a Stick.** `stick load` writes only exactly
  what a `stick dry-run` of the same load reported in the last ten minutes,
  and only to an unlocked Stick. Show the DJ the dry run first.
- **Group your work in a Run** (`runs start` ... `runs finish`) so the DJ can
  see in one place what you changed, with every value before and after.
- **Read the JSON, not the text.** Pass `--json` and parse standard output.
  The text form is for people and may change.
- When something fails, tell the DJ the error's `message`; do not retry the
  same change in another way to get round a refusal.

### Commands that are the DJ's own decision (CLI only)

These are deliberately **not** tools of the MCP server. An agent may run them
from the CLI only when the DJ has asked, in so many words, for that exact
decision:

| Command | What it decides |
|---|---|
| `wreckordbox stick unlock <STICK> [--name NAME]` | that a Stick may be written to |
| `wreckordbox projection acknowledge <BARRIER> --source dj` | that Projection may fill fields a barrier blocked |
| `wreckordbox review accept ... --token T --source dj` | that held-back Facts are written to the Library as the DJ's |
| `wreckordbox review reject ... --source dj` | that suggestions are never offered again |
| `wreckordbox review reopen ... --source dj` | that rejected suggestions wait again |
| `wreckordbox runs spills discard <ID> --source dj --reason TEXT` | that set-aside Ledger records are given up |
| `wreckordbox backup restore <BACKUP> --yes` | that the Library and Sidecar are replaced (also not a tool: no service) |
| `wreckordbox protection off` | that the Library may be changed at all |

The last two are not the DJ's *judgement* about content, but they are as
destructive, so the MCP server leaves them out too. `protection on` and
reading protection are tools.

## How the command works

- **One service owns the Library.** The first command starts a background
  Library service (or talks to the window, which hosts it while open); it
  stops by itself after five idle minutes. The window and every other client
  see each change at once.
- **`--json`** (anywhere on the line) prints the result as one JSON object on
  standard output. Without it, a table or sentence for people.
- **Failure**: exit status 1, and with `--json` one line on standard error:
  `{"error":{"code":"<stable code>","message":"<for the DJ>"}}`. Without
  `--json`: `wreckordbox: <message>`, sometimes followed by a hint line.
  A few commands (`stick load`, `schema check`) also exit 1 after printing a
  result that says what went wrong.
- **Common error codes**, any command: `library_missing`, `library_locked`,
  `library_damaged`, `library_loading` (a window that has just opened is still
  reading; the command already waited 15 s, `WRECKORDBOX_LOADING_WAIT_SECS`
  changes that), `service_failed`, `bad_request` (wrong or missing
  arguments; the message says which), `track_not_found`. Any command that
  changes the Library can also fail with `library_protected` (Library
  Protection is on) and `write_refused` (another guard said no, such as
  rekordbox running; the message is the reason).
- **Reads never write.** No reading command changes the Library, the Sidecar
  or a Stick.
- **What "changes the Library" means.** Ratings, colour labels, MyTags,
  playlists and removals change the rekordbox Library (`master.db`). Facts,
  Runs, Decisions, Mutants, Stems and Intensity live in the **Sidecar** beside
  it and never change the Library. Every Library change goes through one edit
  path, is recorded in the Ledger, and is refused while Library Protection is
  on. Most are a step the window's Undo takes back; some are not (see
  [Undo](#undo)).

### Runs and sources

Every command that changes the Library (`tracks`, `mytags`, `playlists`,
`intake`, `project`) takes three options anywhere after its group name:

- `--run <RUN_ID>`: put the change in an open Run from `runs start`.
- `--source <SOURCE>`: who the change is by: `dj`, `agent:NAME`, `tool:NAME`
  or `web:ADDRESS`. Without it and without `--run`, the change is recorded as
  `tool:wreckordbox`, never as the DJ's.
- `--description <TEXT>`: what the work is (only without `--run`).

Without `--run`, the command is a Run of its own, opened and finished for it.
The result's `run` key names the Run either way:

```json
"run": {"id": 1, "description": "Rate the warm-up selection", "source": {"kind": "agent", "name": "claude"},
        "started_at": 1794766051209, "finished_at": null, "outcome": null}
```

Times are milliseconds since 1970 (UTC). Facts, Decisions and Mutants take
their own `--run`, `--source` and `--description` options with the same
meaning (see each command).

### Undo

Most Library changes are one step of the window's Undo history, so the DJ can
take back your most recent changes from the window. **Not all of them:**

- **Permanent, and they empty the window's Undo and Redo history:**
  `mytags merge`, `mytags delete` and `tracks remove --confirm`. Nothing in the
  window brings them back; a restore from a backup is the only way. Ask the DJ
  first, every time.
- **Outside the Undo history** (they are not Undo steps, and they end the
  window's Redo): `mytags create` and `mytags rename`. To take one back, delete
  the new tag or rename it again.

Every other Library change (ratings, Prep colours, MyTag assign and unassign,
`mytags move`, `intake tag`, playlist edits, Projection) is an Undo step.
**The CLI has no undo command.** Reversing a whole Run later and out of order (Unwind) is planned
(beads `wbx-23`, `wbx-24`) and not built yet; until then `runs show` lists
every change with its value before and after, which is what you, or the DJ,
need to put things back by hand.

### Where things are

Which Library: the one rekordbox's own settings name (upstream's
`RBXPORT_OPTIONS` overrides it). The Sidecar is a folder beside it
(`WRECKORDBOX_SIDECAR_DIR` overrides). `wreckordbox status` names the Library.

## Recipes

Each recipe assumes `--json` and that you parse the result. Track identifiers
are the `id` of `tracks list` / `tracks query`. Replace `agent:claude` with your
own name.

### Find tracks by tag, key or BPM

```sh
wreckordbox --json tracks query --key 8A --key 9A --bpm 124..128 --limit 100
wreckordbox --json tracks query --mytag "Peak" --energy 4..5
wreckordbox --json tracks query --mytag-in Situation "Peak time" --prep-colour none
wreckordbox --json tracks query --playlist "Friday" --analysed yes
```

Every option must hold. Page with `--offset`: each page's `next_offset` is
the offset of the next page, `null` on the last. `tracks list --filter TEXT` is
the window's search box, six columns only. `mytags list` gives the tag
identifiers and names to filter by.

### Rate and colour a selection, as one Run

```sh
wreckordbox --json runs start --source agent:claude --description "Energy and Prep colour for the new arrivals"
# -> {"id": 7, ...}
wreckordbox --json tracks rate 10001 4 --run 7
wreckordbox --json tracks rate 10002 3 --run 7
wreckordbox --json intake colour 10001 Green --run 7
wreckordbox --json runs finish 7 --outcome "2 tracks rated, 1 coloured"
```

The star rating is Energy. A Prep colour is a colour label; `intake colours`
lists them with the DJ's names for them.

### Set MyTags

```sh
wreckordbox --json mytags list                          # tag and category identifiers
wreckordbox --json mytags assign 700003 10001 10002 --run 7
wreckordbox --json mytags unassign 700003 10002 --run 7
wreckordbox --json intake tag 10003 700001 700002 --run 7   # set one dimension exactly
```

If the DJ has rekordbox's *Add "My Tag" to the "Comments"* preference on, pass
`--comments` so each track's comment is rewritten with its tags.

### Build a playlist

The CLI cannot create an empty playlist or add tracks to one yet; it can copy,
trim, reorder and delete. To build a set from an existing playlist:

```sh
wreckordbox --json playlists tree
wreckordbox --json playlists copy 900000 --name "Warm-up set" --run 7   # -> "playlist": "<new id>"
wreckordbox --json playlists remove <new id> 10004 10005 --run 7
wreckordbox --json playlists tracks <new id> --limit 500                # entry_id of each entry
wreckordbox --json playlists reorder <new id> <entry_id> <entry_id> ... --run 7
```

A playlist built from nothing needs the DJ to make it in the window first.

### Work through Intake

```sh
wreckordbox --json intake count
wreckordbox --json intake list --limit 20       # each track's "needs": energy, tags, hot_cues, grid, title, artist
wreckordbox --json intake tags 10001            # the tag panel: dimensions and what the track has
wreckordbox --json intake tag 10001 700001 700002 --run 7
wreckordbox --json tracks rate 10001 3 --run 7
wreckordbox --json intake colour 10001 Clean --run 7   # a Prep colour takes it out of Intake
```

A track with no Prep colour is in Intake; the colour label named *Problem*
puts it in Problems (`intake problems`). Hot cues and the beat grid are set in
the window.

### Record a Fact

When you know something rekordbox cannot hold, or you are not sure enough to
change the Library, record a Fact. It never changes the Library.

```sh
wreckordbox --json facts kinds                         # what kinds exist and their value types
wreckordbox --json facts record 10001 mood Dark Hypnotic --source agent:claude --confidence 0.7
wreckordbox --json facts record 10002 energy 3 --source agent:claude --confidence 0.9 --run 7
printf '%s\n' '{"track":"10003","kind":"genre","value":["Tech House"],"confidence":0.8}' \
  | wreckordbox --json facts record-many --source agent:claude --description "Genres from the tagger"
```

`project --dry-run` then shows what Projection would write into the Library
from standing Facts, and what it holds back for the DJ (`review waiting`).

### Start and finish a Run so changes are grouped and undoable

```sh
wreckordbox --json runs start --source agent:claude --description "What you are doing, for the DJ"
# pass --run <id> to every changing command and to facts record
wreckordbox --json runs finish <id> --outcome "how it went"
```

A Run left open can still be recorded in; finish it when the work is done.
Each Library change inside it is still its own step of the window's Undo,
apart from the permanent and outside-history ones named under [Undo](#undo).

### List a Run, and take it back

```sh
wreckordbox --json runs list --limit 10
wreckordbox --json runs show 7     # every change: "values": [{"subject","field","before","after"}]
```

To reverse it now: the DJ can Undo in the window (newest first), except for
`mytags merge`, `mytags delete` and `tracks remove`, which are permanent, and
`mytags create` and `rename`, which are outside the Undo history (see
[Undo](#undo)). Or you can set each `before` value again in a new Run, after
asking. Unwind is not built.

### Preview a Stick load

```sh
wreckordbox --json stick list                              # mounted Sticks, kind, protected, free space
wreckordbox --json stick dry-run /run/media/dj/SPARE 900000 900001
```

The dry run writes nothing and answers `plan`, the counts of tracks added,
changed and removed, playlists created and removed, and the room before and
after. Show it to the DJ. Loading it (`stick load --plan <plan>`) needs the
Stick unlocked, which is the DJ's decision.

## Command reference

Each section says what the command reads or changes, its arguments, the JSON
it prints (abridged; keys shown are the ones you need), an example, and how it
fails. Options common to a group (`--run`, `--source`, `--description`,
`--json`) are described above and not repeated. `--help` on any command is
authoritative.

### `wreckordbox status`

Reads: the service. Changes nothing (starts the service if none is running).
Arguments: none.

```sh
wreckordbox --json status
```
```json
{"running": true, "pid": 1763103, "version": "1.10.0", "protocol": 1,
 "library": {"path": "/.../master.db", "tracks": 40}, "socket": "/run/user/1000/wreckordbox/03f4....sock"}
```

Fails: `library_missing`, `library_locked`, `library_damaged`, `service_failed`.

### `wreckordbox tracks`

The group for finding, rating and removing tracks. Takes the Run options.

### `wreckordbox tracks list`

Reads the Library. `--filter TEXT` (the window's search box: every word must
appear), `--limit ROWS` (default 50).

```sh
wreckordbox --json tracks list --filter "deep house" --limit 2
```
```json
{"total": 40, "tracks": [{"id": "10000", "title": "Fixture Track 0", "artist": "", "bpm": 128.0, "key": "", "rating": 0}]}
```

`total` counts every match before the limit. `bpm` is `null` for a track with
none. Fails: only the common errors.

### `wreckordbox tracks show`

Reads one track in full. Argument: `<ID>`.

```sh
wreckordbox --json tracks show 10001
```
```json
{"id": "10001", "title": "Fixture Track 1", "artist": "", "album": "", "genre": "", "label": "",
 "bpm": 128.01, "key": "", "rating": 0, "energy": null, "colour_id": 0, "colour": null, "prep_colour": null,
 "comment": "", "year": null, "date_added": null, "duration_seconds": 2, "play_count": 0, "analysed": true,
 "file_name": "Fixture Track 1.wav", "location": "/.../Fixture Track 1.wav",
 "my_tags": [], "playlists": [{"id": "900000", "name": "Playlist 0"}]}
```

Fails: `track_not_found`.

### `wreckordbox tracks query`

Reads the Library: tracks by any field, a page at a time; every option must
hold. Options: `--search`, `--title`, `--artist`, `--album`, `--genre`,
`--label`, `--comment`, `--location` (text, contains, any case); `--year`,
`--bpm`, `--duration`, `--date-added`, `--play-count` (one value or a range
`a..b`, `a..`, `..b`); `--key` (repeatable: any of), `--energy`/`--rating`
(1 to 5, 0 for none, repeatable), `--bpm-range` (repeatable), `--prep-colour`
(repeatable, or `none`), `--analysed yes|no`, `--mytag NAME` (repeatable: all
of), `--mytag-in CATEGORY TAG` with `--mytag-any CATEGORY`, `--playlist NAME|ID`,
`--limit` (50), `--offset` (0).

```sh
wreckordbox --json tracks query --bpm 128..128.05 --limit 2
```
```json
{"total": 6, "offset": 0, "limit": 2, "next_offset": 2, "tracks": [ /* full records, as tracks show */ ]}
```

Fails: `bad_request` for a range or value it cannot read; a MyTag or category
the Library no longer has is left out and said so.

### `wreckordbox tracks rate`

**Changes the Library**: a track's star rating (its Energy). Arguments:
`<ID> <STARS>` (0 to 5; 0 clears). A rating typed here is a deliberate hand
set by the source given.

```sh
wreckordbox --json tracks rate 10001 4 --run 1
```
```json
{"id": "10001", "title": "Fixture Track 1", "rating": 4, "previous_rating": 0, "generation": 3, "run": {"id": 1, ...}}
```

Fails: `track_not_found`, `invalid_rating` (over 5), `library_protected`,
`write_refused`.

### `wreckordbox tracks remove`

**Changes the Library** only with `--confirm`: removes tracks from the
collection and every playlist (the audio files stay). Without `--confirm` it
says which tracks and Mutants would go and removes nothing. Arguments:
`<TRACK_ID>...`. **Permanent:** no Undo takes it back, and it empties the
window's Undo history. Ask the DJ before `--confirm`.

```sh
wreckordbox --json tracks remove 10039
```
```json
{"removed": false, "tracks": ["10039"], "mutants": [], "generation": null}
```

Fails: `track_not_found`, `library_protected`, `write_refused`.

### `wreckordbox mytags`

The group for MyTag categories and tags. Takes the Run options. The changing
commands take `--comments` to also rewrite the tracks' comments with their
MyTags (pass it when the DJ's rekordbox puts MyTags into comments).

### `wreckordbox mytags list`

Reads the categories and tags in stored order, with how many tracks carry each.

```sh
wreckordbox --json mytags list
```
```json
{"categories": [{"id": "700001", "name": "Situation",
  "tags": [{"id": "700002", "name": "Peak", "tracks": 0}, {"id": "700003", "name": "Warm-up", "tracks": 0}]}]}
```

### `wreckordbox mytags show`

Reads the MyTags one track carries. Argument: `<TRACK_ID>`.

```json
{"id": "10001", "title": "Fixture Track 1", "my_tags": [{"id": "700003", "name": "Warm-up", "category": "Situation", "category_id": "700001"}]}
```

Fails: `track_not_found`.

### `wreckordbox mytags tracks`

Reads the tracks carrying a tag, in collection order. Arguments: `<TAG_ID>`,
`--limit` (50).

```json
{"my_tag": {"id": "700003", "name": "Warm up", "category": "Situation", "category_id": "700001"},
 "total": 2, "tracks": [{"id": "10001", "title": "Fixture Track 1", "artist": "", "bpm": 128.01, "key": "", "rating": 4}]}
```

Fails: an unknown tag identifier.

### `wreckordbox mytags create`

**Changes the Library**: adds a tag at the end of a category. Arguments:
`<CATEGORY_ID> <NAME>` (unique in the category). Answers the new tag and the
Run. Not an Undo step (delete the tag to take it back). Fails: a duplicate
name, an unknown category, `library_protected`.

### `wreckordbox mytags create-category`

Always refused (`my_tag_category_limit`): rekordbox fixes the categories at
four. Rename one with `mytags rename` instead. Exists so the refusal is
explained.

### `wreckordbox mytags rename`

**Changes the Library**: renames a tag or a category; tracks keep the tag.
Arguments: `<ID> <NAME>`, `--comments`. Not an Undo step: rename it back to
take it back.

```sh
wreckordbox --json mytags rename 700003 "Warm up" --run 1
```
```json
{"id": "700003", "kind": "tag", "name": "Warm up", "previous_name": "Warm-up", "generation": 8, "run": {...}}
```

### `wreckordbox mytags move`

**Changes the Library**: moves a tag within its category, or a category among
categories. Arguments: `<ID> <POSITION>` (from 1; past the end is the end).

### `wreckordbox mytags reorder`

**Changes the Library**: puts every tag of one category, or every category, in
the order given. Arguments: `<ID>...`, each exactly once. Fails with
`bad_request` when one is missing or repeated.

### `wreckordbox mytags merge`

**Changes the Library**: every track carrying `<FROM_TAG_ID>` is given
`<INTO_TAG_ID>`, then the first tag is deleted. `--comments`. This empties the
window's Undo history, as the window's own merge does; ask first.

### `wreckordbox mytags delete`

**Changes the Library**: deletes a tag and takes it off every track. Argument:
`<TAG_ID>`, `--comments`. Empties the window's Undo history; ask first.

### `wreckordbox mytags assign`

**Changes the Library**: puts a tag on tracks; a track that has it is left as
it is. Arguments: `<TAG_ID> <TRACK_ID>...`, `--comments`.

```sh
wreckordbox --json mytags assign 700003 10001 10002 --run 1
```
```json
{"changed": 2, "generation": 4, "my_tag": {"id": "700003", "name": "Warm-up", ...},
 "tracks": [{"id": "10001", "title": "Fixture Track 1", "changed": true}, ...], "run": {...}}
```

Fails: unknown tag, `track_not_found`, `library_protected`.

### `wreckordbox mytags unassign`

**Changes the Library**: takes a tag off tracks; the tag stays. Same arguments
and result as `mytags assign`.

### `wreckordbox playlists`

The group for the playlist tree. Takes the Run options.

### `wreckordbox playlists tree`

Reads folders, playlists and intelligent playlists (with their rule).

```json
{"playlists": [{"id": "900000", "name": "Playlist 0", "kind": "playlist", "parent_id": null, "depth": 1, "tracks": 5}]}
```

### `wreckordbox playlists tracks`

Reads a playlist's tracks in order (an intelligent playlist: what its rule
admits now; a folder: every playlist in it). Arguments: `<ID>`, `--limit` (50).

```json
{"playlist": {"id": "900000", "name": "Playlist 0", "kind": "playlist"}, "total": 5,
 "tracks": [{"position": 1, "entry_id": "fixture-song-...", "id": "10000", "title": "Fixture Track 0", ...}]}
```

`entry_id` names one entry of the playlist: what `playlists reorder` and
`mutants create` take.

### `wreckordbox playlists remove`

**Changes the Library**: takes every entry of each track out of a playlist; a
Mutant of an entry goes with it. Arguments: `<PLAYLIST_ID> <TRACK_ID>...`.

### `wreckordbox playlists delete`

**Changes the Library**: deletes a playlist, or a folder and everything in it,
with its Mutants. Argument: `<PLAYLIST_ID>`. Ask first.

### `wreckordbox playlists reorder`

**Changes the Library**: puts the entries in a new order; Mutants stay on
their entries. Arguments: `<PLAYLIST_ID> <ENTRY_ID>...`, every entry once.

### `wreckordbox playlists copy`

**Changes the Library**: copies a playlist beside it, with its tracks in order
and a copy of each Mutant. Arguments: `<PLAYLIST_ID>`, `--name NAME`
(default: the name and "copy").

```sh
wreckordbox --json playlists copy 900000 --name "Warm-up set" --source agent:claude
```
```json
{"from": "900000", "playlist": "2578147643", "name": "Warm-up set", "tracks": 5, "mutants": [], "generation": 7, "run": {...}}
```

### `wreckordbox history`

The group for play history. Reads only.

### `wreckordbox history list`

Reads the occasions history was recorded on, oldest first.

```json
{"occasions": [{"id": "800000", "name": "HISTORY 2026-09-01", "folder": "2026/September", "tracks": 5}]}
```

### `wreckordbox history tracks`

Reads what was played on one occasion, in order. Arguments: `<ID>`, `--limit`.

```json
{"occasion": {"id": "800000", ...}, "total": 5, "tracks": [{"position": 1, "id": "10000", "title": "Fixture Track 0", ...}]}
```

### `wreckordbox protection`

Without a subcommand: reads whether Library Protection is on. It counts as on
until someone turns it off, including on a machine where the window never ran.

```json
{"protect_library": true, "revision": 0}
```

### `wreckordbox protection on`

Changes the protection switch (not the Library): every Library change is
refused, here and in the window, until it is turned off. Always safe to run.

```json
{"protect_library": true, "previous": false, "revision": 2}
```

### `wreckordbox protection off`

Changes the protection switch: lets the Library be changed. **Only when the
DJ asks.** Unlike the window, it does not ask whether a backup exists. Not an
MCP tool.

### `wreckordbox stick`

The group for Sticks (USB drives for the decks). Every Stick is protected
until unlocked; an unlock ends when it is ejected, removed or put back. A
Spare stick carries an empty `.rbxport-disposable` file at its root; any other
is a Gig stick.

### `wreckordbox stick status`

Reads whether one Stick is protected. Argument: `<STICK>` (its mount folder).

```json
{"name": "SPARE", "protected": true, "spare": true, "revision": 0}
```

### `wreckordbox stick unlock`

Changes Stick Protection: lets the Stick be written to. **The DJ's decision
only; never unlock a Stick the DJ did not name.** A Gig stick needs
`--name NAME`, its name typed. Not an MCP tool. Fails with `bad_request` for a
Gig stick without its name.

### `wreckordbox stick lock`

Changes Stick Protection: protects the Stick again. Always safe.

### `wreckordbox stick list`

Reads the mounted Sticks: kind, whether protected, last load, room.

```json
{"sticks": [{"name": "SPARE", "path": "/run/media/dj/SPARE", "kind": "spare", "protected": true,
  "free_bytes": 4116152320, "total_bytes": null, "last_loaded": null, "playlists": null, "tracks": null, "unfinished_write": false}]}
```

### `wreckordbox stick dry-run`

Reads the Library and the Stick and writes nothing: what loading these
playlists would do. Arguments: `<STICK> <PLAYLIST_ID>...` (a folder stands for
everything in it).

```sh
wreckordbox --json stick dry-run /run/media/dj/SPARE 900000
```
```json
{"plan": "fc8addee8874454bb1f65299a917cd2c", "playlists": [{"id": "900000", "name": "Playlist 0"}],
 "changes": {"counts": {"tracks_added": 3, "tracks_changed": 0, "tracks_removed": 0, "tracks_missing": 2,
                        "playlists_created": 1, "playlists_removed": 0, "native_stems": 0, ...},
             "tracks": {"added": [...], "missing": [{"id": "10003", "title": "Track 003"}], ...},
             "playlists": {"created": ["Playlist 0"], ...}},
 "space": {"needed_bytes": 3729728, "free_before": 4097490944, "free_after": 4094634848, "shortfall_bytes": null},
 "not_loadable": null, "stick": {...}}
```

`missing` tracks have no audio file to copy. Fails: an unknown playlist, a
path that is not a mounted Stick.

### `wreckordbox stick load`

**Writes to a Stick**: exactly what the dry run reported. Arguments:
`<STICK> <PLAYLIST_ID>... --plan <PLAN_ID>`. Refused with `stick_protected`
unless the DJ unlocked the Stick; refused when the plan is not from a dry run
of this very load in the last ten minutes, or anything changed since; refused
before writing when it would not fit. Exits 1 unless everything planned was
written. Only on the DJ's word.

### `wreckordbox stems`

The group for Stems in the Sidecar and Native stems on a Spare stick.

### `wreckordbox stems separate`

Changes the Sidecar (never the Library): separates a track into four Stems.
**Takes over twenty minutes a track; only when the DJ asks.** A track with
current Stems is not separated again. Argument: `<TRACK_ID>`. Progress on
standard error; interrupting the command does not stop the service's work.

### `wreckordbox stems show`

Reads the Stems the Sidecar holds for a track.

```json
{"track": {"id": "10001", "title": "Fixture Track 1"}, "stems": null, "previous": []}
```

### `wreckordbox stems write`

**Writes to a Stick**: a track's Stems as Native stems under `STEMS/`.
Arguments: `<TRACK_ID> <STICK>`. Refused unless the Stick carries the Spare
stick marker and already holds an export of the track, and the track has
current Stems.

### `wreckordbox facts`

The group for Facts: what is known about a track, with its source and
confidence, in the Sidecar. Never changes the Library.

### `wreckordbox facts record`

Changes the Sidecar: one Fact. Arguments: `<TRACK_ID> <KIND> <VALUE>...`
(text as it is; several values for a list kind; anything else as JSON),
`--source` (required unless `--run`), `--confidence 0..1` (not for `dj`),
`--run`, `--description`. The same source recording the same kind again
replaces its own earlier Fact; another source's stays beside it.

```sh
wreckordbox --json facts record 10001 mood Dark Hypnotic --source agent:claude --confidence 0.7 --run 1
```
```json
{"created_run": false, "facts": [{"id": 1, "track_id": "10001", "kind": "mood", "value": ["Dark", "Hypnotic"],
  "nature": "judgement", "confidence": 0.7, "source": {"kind": "agent", "name": "claude"}, "run": 1, "stale": false, ...}],
 "marked_stale": [], "run": {...}}
```

Fails: `invalid_source` (no source and no Run), an undeclared kind or a value
of the wrong type, `track_not_found`, a Tag schema with a mistake (every
`facts.*` command is refused until it is fixed).

### `wreckordbox facts record-many`

Changes the Sidecar: many Facts in one Run, all or none. Reads one JSON object
per line (or a JSON array) from standard input or `--file PATH`:
`{"track": "10003", "kind": "genre", "value": ["Tech House"], "confidence": 0.8}`.
`--source`, `--confidence`, `--run`, `--description` apply to every line that
does not say them. Fails: `bad_request` ("there are no Facts to record") and
the `facts record` errors, naming the line.

### `wreckordbox facts show`

Reads the Facts of a track, newest first. Arguments: `<TRACK_ID>`, `--kind`,
`--all` (also stale, unverified and replaced ones), `--check-audio` (verify
Measurements against the audio; still records nothing).

```json
{"track": {"id": "10001", "title": "Fixture Track 1"}, "facts": [{"id": 1, "kind": "mood", ...}],
 "stale": [], "unverified": [], "reattached": [], "review": []}
```

### `wreckordbox facts kinds`

Reads the declared kinds: Judgement or Measurement, and the value type.

```json
{"file": "/.../kinds.toml", "kinds": [{"name": "energy", "nature": "judgement", "value": "integer", "min": 1.0, "max": 5.0,
  "built_in": true, "reserved": false, "description": "Energy: how intense the track is, 1 to 5. ..."}, ...]}
```

### `wreckordbox facts standing`

Reads which Fact stands for a track and kind, why, and what it superseded.
Arguments: `<TRACK_ID> <KIND>`.

```json
{"kind": "mood", "standing": {"id": 1, ...}, "reason": {"code": "only_candidate", "message": "..."},
 "superseded": [], "stale": [], "track": {...}, "rules_file": "/.../source-precedence.toml"}
```

### `wreckordbox runs`

The group for Runs: pieces of work whose changes belong together.

### `wreckordbox runs start`

Changes the Sidecar: opens a Run. Requires `--source` and `--description`.

```sh
wreckordbox --json runs start --source agent:claude --description "Rate the warm-up selection"
```
```json
{"id": 1, "description": "Rate the warm-up selection", "source": {"kind": "agent", "name": "claude"},
 "started_at": 1794766051209, "finished_at": null, "outcome": null}
```

### `wreckordbox runs finish`

Changes the Sidecar: ends a Run; nothing more is recorded in it. Arguments:
`<RUN_ID>`, `--outcome TEXT`. Answers the Run with `finished_at` set. Fails:
`run_finished` for a Run already ended, or an unknown Run.

### `wreckordbox runs list`

Reads Runs, newest first, with change counts. `--limit`, `--offset`.

```json
{"total": 7, "offset": 0, "limit": 1, "next_offset": 1,
 "runs": [{"id": 7, "description": "...", "source": {...}, "started_at": ..., "finished_at": ..., "outcome": "...", "changes": 0, "counts": []}]}
```

### `wreckordbox runs show`

Reads one Run and every change it made, in order, with before and after.
Arguments: `<RUN_ID>`, `--limit`, `--offset`.

```json
{"run": {"id": 1, "source": {...}, "description": "...", "changes": 3, "counts": [...], ...},
 "entries": [{"id": 2, "action": "edit", "store": "library", "label": "Track Edit", "reversible": true,
              "command": {"name": "tracks.rate", "args": {...}},
              "values": [{"subject": "track 10001", "field": "Rating", "before": 0, "after": 4}]},
             {"id": 5, "action": "record", "store": "sidecar", "fact": {...}}]}
```

### `wreckordbox runs spills`

The group for Ledger spills: records of changes the Library accepted but the
Ledger could not take at the time, set aside on this machine to be recovered
explicitly. Reads never recover or delete them. See
`docs/development/ledger-failure-paths.md`.

### `wreckordbox runs spills list`

Reads the retained spills. `--limit`, `--offset`. Recovers and deletes
nothing.

### `wreckordbox runs spills show`

Reads one spill: its original Run and source, its files and a page of its
accepted records. Arguments: `<ID>`, `--limit`, `--offset`.

### `wreckordbox runs spills replay`

Changes the Sidecar's Ledger: replays a spill's accepted records with their
original source and Run. Arguments: `<ID>`, `--manifest JSON_FILE` (the
original metadata, only when the spill's own manifest was lost; it must repeat
what the Ledger recorded, never invented or the DJ substituted). Retrying is
safe. Run it when the DJ asks, or when `runs show` reports pending recording
and the DJ agrees; not an MCP tool.

### `wreckordbox runs spills discard`

Changes the Sidecar's Ledger: discards a spill's records, **as the DJ's
decision** (`--source dj --reason TEXT`, nothing else is accepted). The
Library's values stay; what is lost is their record. Only when the DJ asked
for that spill to be discarded. Not an MCP tool.

### `wreckordbox decisions`

The group for Decisions: a question between fixed candidates, answered by
Deciders (you, a rule, the DJ). In the Sidecar; never changes the Library.

### `wreckordbox decisions create`

Changes the Sidecar: asks a Decision. Options: `--type choice|yes-no`,
`--question`, `--candidate NAME[=DESCRIPTION]` (two or more for `choice`),
`--context JSON`, `--topic`, `--track`, `--run`, `--source` (who asks:
`agent:NAME` and so on). The asker is the source of the Run the Decision is
asked in: without `--run` or `--source`, `tool:wreckordbox-decisions`. An
agent or tool cannot ask in the DJ's Run.

```sh
wreckordbox --json decisions create --type choice --question "Genre?" --candidate House --candidate Techno --track 10001 --topic genre
```
```json
{"id": 1, "pending": true, "question": "Genre?", "question_type": "choice", "topic": "genre", "track_id": "10001",
 "candidates": [{"name": "House"}, {"name": "Techno"}], "context": {}, "answers": [], "run": null, "created_at": ...}
```

### `wreckordbox decisions pending`

Reads the Decisions nobody (or `--decider` SOURCE) has answered. `--track`,
`--topic`, `--limit`, `--offset`. Answers `{"decisions": [...], "total", "offset", "limit", "next_offset"}`.

### `wreckordbox decisions answered`

Reads the Decisions that have an answer (by `--decider`, if given). Same
options and result as `decisions pending`.

### `wreckordbox decisions list`

Reads every Decision. Same options and result as `decisions pending`.

### `wreckordbox decisions show`

Reads one Decision with every answer. Argument: `<DECISION_ID>`.

### `wreckordbox decisions answer`

Changes the Sidecar: answers as a Decider under your own name. Arguments:
`<DECISION_ID> <CHOICE>`, `--decider agent:NAME`, `--probability 0..1` or
`--probabilities JSON`, `--reason`, and `--fact KIND` (with `--fact-track`,
`--fact-value`) to also record the choice as a Fact; `--run`, `--description`.

```json
{"answer": {"id": 1, "decision_id": 1, "choice": "House", "choice_probability": 0.8, "decider": {"kind": "agent", "name": "claude"}, ...},
 "decision": {...}, "fact": null, "run": null}
```

Fails: a choice that is not a candidate, exactly as named.

### `wreckordbox decisions decide`

Changes the Sidecar: the rule-based Decider answers as `tool:rule-based`.
`--match FIELD` picks the candidate named by that context field;
`--threshold FIELD --at-least N --at-most N` answers yes/no. When the rule
cannot answer, nothing is recorded. Also `--fact`, `--fact-track`,
`--fact-value`, `--run`, `--description`.

### `wreckordbox mutants`

The group for Mutants: a playlist entry's own edited version of its track,
kept in the Sidecar; never written into the Library. Changing commands take
`--source`, `--run`, `--description`.

### `wreckordbox mutants edit`

Changes the Sidecar: sets edits on a Mutant. Arguments: `[ID]` or
`--entry ENTRY_ID`, `--edits JSON` (`key_shift`, `tempo`, `stems`
(vocals, melody, bass, drums), `cues` with `kind` and `position_ms`).

```sh
wreckordbox --json mutants edit 1 --edits '{"tempo":1.04}' --source agent:claude
```
```json
{"changed": true, "created_run": true, "mutant": {"id": 1, "edits": {"tempo": 1.04},
  "derived": {"bpm": 133.12, "description": "tempo +4.00%", ...}, ...}, "run": {...}}
```

### `wreckordbox mutants clear`

Changes the Sidecar: clears one edit (`stems`, `key_shift`, `tempo`, `cues`),
inheriting it from the track again. Arguments: `[ID]` or `--entry`, `<EDIT>`.

### `wreckordbox mutants create`

Changes the Sidecar: makes a Mutant of one playlist entry. Arguments:
`[ENTRY_ID]` or `--playlist ID --position N`. Nothing audible changes until it
is edited. Answers `{"created_run", "mutant": {...}, "run"}`.

### `wreckordbox mutants show`

Reads one Mutant, its edits and history. Arguments: `[MUTANT_ID]` or
`--entry`, `--check-audio`.

### `wreckordbox mutants list`

Reads the Mutants, of a `--playlist` or a `--track` if given. `--check-audio`.

```json
{"mutants": [], "total": 0}
```

### `wreckordbox mutants remove`

Changes the Sidecar: removes a Mutant; the entry is the track again.
Arguments: `[MUTANT_ID]` or `--entry`.

### `wreckordbox intake`

The group for Intake (tracks with no Prep colour) and Problems (tracks whose
Prep colour is the label named Problem). Takes the Run options.

### `wreckordbox intake list`

Reads the Intake queue, each track with what it still needs. `--limit` (50),
`--offset` (0).

```json
{"total": 40, "offset": 0, "limit": 1, "next_offset": 1, "prep_colour": null,
 "tracks": [{"id": "10000", "title": "...", "needs": ["energy", "tags", "hot_cues", "grid"],
             "empty_dimensions": [{"id": "700001", "kind": "mytag_category", "name": "Situation"}],
             "grid": "not_confirmed", "suggestions": [], ... }]}
```

### `wreckordbox intake problems`

Reads the Problems queue. `--limit`, `--offset`, `--label NAME` (the label
that means Problem, when it is not called Problem). Fails with
`problem_colour_not_found` when no colour label is named Problem.

### `wreckordbox intake count`

Reads how many tracks wait in each queue. `--label NAME`.

```json
{"intake": 40, "problems": null, "problem_colour": null,
 "problems_unavailable": {"code": "problem_colour_not_found", "message": "..."}}
```

### `wreckordbox intake colours`

Reads the eight colour labels with the DJ's names, and which mean Clean and
Problem.

```json
{"colours": [{"id": 1, "colour": "Pink", "label": "Pink"}, ...], "clean": null, "problem": null, "problem_unavailable": {...}}
```

### `wreckordbox intake colour`

**Changes the Library**: a track's Prep colour (colour label). Arguments:
`<TRACK_ID> [PREP_COLOUR]` (the DJ's name for a label, the colour, or `none`),
or `--colour-id N` (0 to 8).

```sh
wreckordbox --json intake colour 10002 Green --run 1
```
```json
{"id": "10002", "title": "Fixture Track 2", "changed": true, "previous": null,
 "prep_colour": {"id": 5, "colour": "Green", "label": "Green"}, "generation": 5, "run": {...}}
```

### `wreckordbox intake tags`

Reads a track's tags as Intake's tag panel shows them. Argument: `<TRACK_ID>`.

```json
{"source": "my_tags", "dimensions": [{"id": "700001", "label": "Situation", "deck": true, "edit": "my_tag", "single": false,
   "tags": [{"id": "700002", "name": "Peak", "my_tag": "700002", "parent": null, "depth": 0}, ...]}],
 "track": {"id": "10001", "set": {"700001": []}, "unmapped": {}, "versions": {...}}, "schema_problem": null}
```

### `wreckordbox intake tag`

**Changes the Library** for a deck-facing dimension (as MyTags) or the
Sidecar for any other (as a Fact): sets one dimension of a track to exactly
the tags given; none clears it. Arguments: `<TRACK_ID> <DIMENSION> [TAG]...`,
`--drop-unmapped VALUE`, `--comments`.

```sh
wreckordbox --json intake tag 10003 700001 700002 --run 1
```
```json
{"changed": true, "facts_not_recorded": [], "panel": { /* as intake tags */ }, "run": {...}}
```

### `wreckordbox intensity`

The group for Intensity: a Measurement from the audio, in the Sidecar.

### `wreckordbox intensity measure`

Changes the Sidecar: measures tracks (`[TRACK_ID]...`, or `--playlist`,
`--search`, `--genre`, `--location`, `--energy`, `--all`), skipping current
Measurements unless `--force`. One Run. Reads audio; slow on many tracks.

### `wreckordbox intensity show`

Reads a track's Intensity. Argument: `<TRACK_ID>`.

```json
{"track": {"id": "10001", "title": "..."}, "intensity": null, "previous": []}
```

### `wreckordbox intensity cancel`

Stops the measuring in progress; finished tracks are kept.

```json
{"cancelled": false}
```

### `wreckordbox schema`

The group for the Tag schema (`tag-schema.toml` in the Sidecar's folder).

### `wreckordbox schema check`

Reads the Tag schema (or `[FILE]`) and shows the MyTag categories its deck view
makes. Exits 1, naming each mistake and its line, when it cannot be used.
Fails with `tag_schema_missing` when there is none.

### `wreckordbox schema convert`

Changes the Sidecar (never the Library): writes a first Tag schema from the
MyTags and records every tag on a track as the DJ's Fact. `--dry-run` writes
nothing; `--tree CATEGORY`, `--name CATEGORY=NAME`, `--replace`, `--reimport`.
Run it only when the DJ asks; the dry run is always safe.

```json
{"file_state": "not_written", "categories": [{"category": "Situation", "dimension": "situation", "terms": 2, ...}],
 "facts_recorded": 3, "conflicts": [], "text": "# The Tag schema: ...", "run": null}
```

### `wreckordbox project`

**Changes the Library** unless `--dry-run`: writes standing Facts into Energy
(the rating) and Prep colour under `projection-policy.toml`; never overwrites a
value the DJ set by hand. `--dry-run`, `--expect PLAN` (apply only exactly
what that dry run listed), `--track ID` (repeatable), `--mytags` (the deck
view's MyTags instead). Takes the Run options; applying is one Run by
`tool:projection` unless `--source` says otherwise.

```sh
wreckordbox --json project --dry-run
```
```json
{"dry_run": true, "plan": "9d1af8d157e34e1f", "changes": [],
 "held": [{"track": "10002", "field": "energy", "before": null, "after": 3, "fact": 2, "reason": "waits_for_dj", ...}],
 "kept": [], "invalid": [], "stale": [], "library_protected": false, "policy": {...}}
```

Fails: `library_protected` when applying; refused with nothing written when
`--expect` no longer matches.

### `wreckordbox projection`

The group for Projection's barriers: writes the Ledger cannot account for.

### `wreckordbox projection barriers`

Reads the barriers and whether the DJ cleared each.

```json
{"barriers": [], "standing": 0, "allows": "A cleared barrier lets Projection fill an empty field ..."}
```

### `wreckordbox projection acknowledge`

Changes the Ledger: clears a barrier **as the DJ's decision**. Only
`--source dj` is accepted (`barrier_not_the_djs` otherwise). Only when the DJ
asked for that barrier. Not an MCP tool.

### `wreckordbox review`

The group for Facts Projection held back for the DJ, and for the titles and
artists agents proposed (never projected: they wait for the DJ, and the
Intake queue shows the same rows). Filters on every subcommand: `--track`,
`--field energy|prep_colour|title|artist`, `--from SOURCE`,
`--at-least CONFIDENCE`.

### `wreckordbox review waiting`

Reads the waiting Facts and a `token` for the listing.

```json
{"count": 0, "waiting": [], "token": "7b7d.09612b07b5ecb5a5", "row_tokens": {}, "runs": [], "library_protected": true}
```

Show these to the DJ; their answer is theirs to give.

### `wreckordbox review accept`

**Changes the Library** as the DJ's own hand set. `--source dj` and
`--token` (from the listing the DJ decided from) are required; `[FACT]...` or
`--all`. Refused with `review_not_the_djs` for any other source. A proposed
title or artist is accepted only by naming its Fact: `--all` and a filter or
confidence never take one, and `--field title|artist` is refused. Two
proposals for the same track and field in one accept are refused
(`review_competing`): the DJ chooses one. Only on the DJ's word. Not an MCP
tool.

### `wreckordbox review reject`

Changes the Ledger: rejects waiting Facts as the DJ's decision; they are never
offered again. `--source dj` only. Not an MCP tool.

### `wreckordbox review rejected`

Reads the rejected suggestions and the rejection each is in.

### `wreckordbox review reopen`

Changes the Ledger: rejected suggestions wait again, as the DJ's decision.
`[FACT]...`, `--all` or `--rejection ENTRY`; `--source dj` only. Not an MCP
tool.

### `wreckordbox backup`

The group for backups of the Library and its Sidecar. Needs no service.

### `wreckordbox backup list`

Reads this Library's backups, newest first.

```json
{"backups": []}
```

### `wreckordbox backup restore`

**Replaces the Library and the Sidecar** with a backup. Without `--yes`,
refused with `restore_refused` and nothing done. Also refused while the window
or a service has the Library (`wreckordbox stop` first) or rekordbox runs.
Only when the DJ asks. Not an MCP tool.

### `wreckordbox stop`

Stops a Library service running by itself (not the window's: `stop_refused`).
Prints `{}`. Fails with `service_not_running` when none is.

### `wreckordbox serve`

Runs the service in the foreground. Not needed: every command starts one on
demand. Refuses (`service_already_running`) when a service owns the Library.

## The MCP server

`wreckordbox-mcp` (crate `crates/wreckordbox-mcp`) speaks the Model Context
Protocol over stdio, so an MCP client such as Claude Code can call the Library
service's commands as tools. It is a thin client of the service, exactly like
the `wreckordbox` command: it has no Library access of its own and no network.
Each tool call connects to the service for this machine's Library (starting
one with the `wreckordbox` command beside it, as the CLI does), sends one
service command, and answers the service's result. A change therefore goes
through the same single edit path, Library Protection, Ledger and live update
of the window as the same change from the CLI.

The JSON-RPC it needs (`initialize`, `ping`, `tools/list`, `tools/call`) is
implemented by hand, newline-delimited on stdin and stdout; no MCP SDK is
used.

### Register it with Claude Code

Build both binaries (`cargo build --release -p wreckordbox -p wreckordbox-mcp`)
and keep them in the same folder. Then, in the project's `.mcp.json` (or with
`claude mcp add wreckordbox -- /path/to/wreckordbox-mcp`):

```json
{
  "mcpServers": {
    "wreckordbox": {
      "type": "stdio",
      "command": "/path/to/target/release/wreckordbox-mcp",
      "args": [],
      "env": {}
    }
  }
}
```

The server uses the environment the CLI does: the Library rekordbox's settings
name (or `RBXPORT_OPTIONS`), `WRECKORDBOX_RUNTIME_DIR`,
`WRECKORDBOX_SIDECAR_DIR`, `WRECKORDBOX_LOADING_WAIT_SECS`. Set
`WRECKORDBOX_CLI` when the `wreckordbox` command is not beside it.

### How tools behave

- A tool whose name starts `read_` changes nothing; its description starts
  `Read only.` and it is marked `readOnlyHint`. A tool whose name starts
  `change_` changes something, and its description starts by saying what:
  the Library (through the edit path, refused while Library Protection is on),
  the Sidecar, or Library Protection.
- Each tool's input is a typed JSON schema whose properties are the service
  command's own arguments. The server checks every call against that schema
  at every level before anything reaches the service: an argument it does not
  advertise, at the top or nested (such as a `source` inside one Fact), or a
  value of the wrong type, is refused with `bad_request` naming its path
  (`facts[0].source`). What passes is sent to the service as it is.
- A result is the service's JSON, in `structuredContent` and as text, with a
  `run` key when the call's changes are in a Run, as the CLI's `--json`. A
  refusal is a tool result with `isError: true` and the text
  `{"error":{"code":"...","message":"..."}}`: tell the DJ the message.
- **Whose work it is.** Changing tools take `run`, `source` and `description`
  as the CLI's options. `source` is `agent:NAME`, `tool:NAME` or
  `web:ADDRESS`, never the DJ (`dj` is refused with `invalid_source`). With
  neither `run` nor `source`, the change is recorded as
  `agent:<the MCP client's name>`, the `clientInfo.name` it sent with
  `initialize` (anything but letters, digits, `-`, `_` and `.` becomes `-`).
  That holds for every changing tool, `change_decision_create` included. A
  `run` the DJ started is refused (`invalid_source`), with or without a
  `source`: an agent's change never joins the DJ's Run, so it never inherits
  the DJ's attribution. Start a Run of your own with `change_run_start`.

### Tools

Reads (each named after the CLI command it mirrors in its description):
`read_status`, `read_tracks`, `read_track`, `read_tracks_query`, `read_mytags`,
`read_track_mytags`, `read_mytag_tracks`, `read_playlists`,
`read_playlist_tracks`, `read_history`, `read_history_tracks`,
`read_protection`, `read_intake`, `read_problems`, `read_intake_counts`,
`read_prep_colours`, `read_track_tag_panel`, `read_fact_kinds`,
`read_track_facts`, `read_standing_fact`, `read_runs`, `read_run`,
`read_decisions`, `read_decision`, `read_mutants`, `read_mutant`,
`read_review_waiting`, `read_review_rejected`, `read_projection_barriers`,
`read_projection_plan`, `read_sticks`, `read_stick_load_preview`,
`read_stems`, `read_intensity`, `read_schema_check`.

Changes to the Library: `change_track_rating`, `change_prep_colour`,
`change_mytag_assign`, `change_mytag_unassign`, `change_mytag_create`,
`change_mytag_rename`, `change_mytag_move`, `change_track_tags`,
`change_playlist_copy`, `change_playlist_remove_tracks`,
`change_playlist_reorder`, `change_project` (needs the `plan` of
`read_projection_plan`).

Changes to the Sidecar: `change_run_start`, `change_run_finish`,
`change_record_facts`, `change_decision_create`, `change_decision_answer`,
`change_mutant_create`, `change_mutant_edit`, `change_mutant_remove`.

Changes to Library Protection: `change_protection_on` (only on; off is the
DJ's).

A recipe in tools, rating a selection in one Run:

```text
change_run_start      {"description": "Energy for the new arrivals"}          -> {"id": 7, ...}
change_track_rating   {"id": "10001", "rating": 4, "run": 7}
change_mytag_assign   {"id": "700003", "tracks": ["10001", "10002"], "run": 7}
change_run_finish     {"id": 7, "outcome": "2 rated, 2 tagged"}
read_run              {"id": 7}
```

### CLI only, in this first version

Not tools, by design: the DJ's own decisions (`stick unlock`,
`projection acknowledge`, `review accept`, `review reject`, `review reopen`,
`protection off`, `runs spills discard`), and what is destructive or long
enough to need a person watching: `backup restore`, `tracks remove`, `playlists delete`,
`mytags merge`, `mytags delete`, `stick load`, `stems separate`,
`stems write`, `intensity measure`, `schema convert`, `runs spills replay`
(Ledger recovery, which the DJ should see). Also left out for now:
`mytags reorder`, `mytags create-category` (always refused),
`decisions decide`, `mutants clear`, `project --mytags`, `intensity cancel`,
`stick lock` and `stick status` (Stick Protection state is kept beside the
window, not in the service), `runs spills list` and `runs spills show`,
`backup list`, `stop`, `serve`.
