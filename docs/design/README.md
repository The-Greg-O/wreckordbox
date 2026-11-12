# Window design

The design for the wreckordbox desktop window, worked out with the DJ on 2026-11-11. It covers the window only; what the deck shows is fixed by rekordbox's formats.

- `principles.md`: the rules the interface follows.
- `requirements.md`: what each view must let the DJ do.
- `tokens.md`: type, colour, spacing and density, and how they relate to the Omarchy theme and to upstream's tokens.
- `screens/`: one folder per view, each with a render and a note on behaviour, states and sizes. `screens/mock.html` is the source of every render (open it with `#library-1280`, `#intake-640` and so on); `screens/render.cjs` regenerates the images.
- Decisions that are hard to reverse: `docs/adr/0007` (the window's structure and look) and `docs/adr/0008` (Mutants).

## Status

The DJ has seen every render in `screens/` and agreed the frame, the four views, the look and the density. The last round of changes they asked for (full-resolution waveforms, phrases, waveform colour themes, click-to-toggle tags, more colour) is drawn in the renders here but has not yet been reviewed by them.

## Decided

- The window is organised as four views on keys 1 to 4 (Library, Intake, Review, Stick) that share one tree, one player and one status line.
- Design for a mostly maximised window first. The agent runs in a terminal in another window; there is no agent panel in the app. Half width stays fully usable and narrower widths degrade without breaking.
- Intake is a view of its own, driven by Prep colour, replacing the "no colour" intelligent playlist. Problems is the same view on a different queue.
- Tags are click-to-toggle chips drawn from the Tag schema, every dimension, with the four that reach the deck marked. No per-tag keys, because the schema will change.
- Keys cover the frequent actions; the pointer can do everything; nothing is keys-only. A command palette replaces the native menu row on Linux.
- The player is compact by default and large in Intake and when editing. Beat jump of 4, 8, 16 or 32 beats is always present.
- Keys are shown in Camelot notation and lit against the loaded track the way rekordbox's traffic light does.
- A Track Filter bar over every list emulates the XDJ-AZ's Track Filter.
- A second list can be shown beside the first, as a compact target for building a playlist.
- A Mutant is a playlist's own edited version of a track (see `CONTEXT.md` and ADR 0008).
- Loading a Stick is a summary, a press-and-hold button and a deliberately over-the-top progress display that can shrink to the status line.
- Chrome colours and fonts follow the active Omarchy theme. Colours that carry meaning stay fixed.
- Waveforms are drawn at full device resolution, show phrases as rekordbox does, and have selectable colour themes.

## Rejected

- **A reskin of upstream's layout.** It keeps cherry-picks cheap, but the deck takes too much of the window, tagging stays buried, and the new views have nowhere to live.
- **A keyboard-first, tree-less interface.** The DJ wants to see the playlist tree and move tracks between two visible lists.
- **An agent panel inside the window.** A terminal beside the window does the job and the window updates live either way.
- **A fixed key per tag.** The Tag schema is going to be reorganised, so keys would move.
- **A step-by-step intake wizard.** The DJ works in whatever order the track needs.

## Still open

- The hot cue convention (what each pad letter means). The pads show letters until it is defined; role labels and "which roles are unset" follow from it.
- Whether a Stick's playlist entry can carry a Mutant without a second audio file. The design assumes it cannot and renders a file at export; this needs checking against the export crates.
- What "other things" belong in the per-track tools beyond Stems, key shift and tempo.
- Memory cue list and waveform zoom in the large player: agreed as needed, not yet drawn.
- The Camelot wheel popover: agreed, not yet drawn.
- The command palette, the Track Filter's pickers, the "show every change" dry-run sheet and the Unwind confirmation: not yet drawn.
- Narrow (about 436 px) layouts for Review, Stick and Set plan.
- The end-of-load victory screen and the beat-synced motion are described, not drawn.
- How far the theme-following goes with a light Omarchy theme.
- Whether the Set plan view allows editing the Recipe's target curve by dragging (currently: no, Recipes are files).
