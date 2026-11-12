# Requirements

What each part of the window must let the DJ do, in the glossary's terms. "Must" items were agreed in the design session; "later" items were agreed in principle and not drawn.

## The frame (every view)

- Switch between Library, Intake, Review and Stick with keys 1 to 4 or a click. Intake and Review show a count of what is waiting.
- See the tree at all times at full width: the Library (All Tracks, Intake, Problems), playlists with every folder and descendant, Set plans, and any mounted Stick with its playlists.
- Hear the loaded track from a compact player: cue, play, overview waveform with hot cues and phrases, position, tempo and key.
- Beat jump forward and back by 4, 8, 16 or 32 beats, with the size on a selector.
- See in the status line whether the Library service is connected, any Run in progress and any Stick load in progress.
- Open a command palette that lists every action with its key. There is no native menu row on Linux.
- Scale the whole interface up and down with Ctrl+-, Ctrl+= and reset it, as a browser does.
- Choose the waveform colour theme (3Band, RGB, Blue, Theme).

## Library

- Show a playlist or All Tracks as a list with these columns by default, in this order: Prep colour, title, artist, BPM, key, Energy, MyTags, time, date added. Artwork, album, label, bitrate and a per-row preview waveform are optional columns, off by default.
- Show keys in Camelot notation, with classic notation as a setting.
- Light the key of every row that mixes with the loaded track: same key, one step either way, the A/B switch (green), and energy-boost moves (yellow).
- Filter any list with a Track Filter bar offering the same fields as the XDJ-AZ's Track Filter: MyTags by category, Energy range, Prep colour, BPM range and key, plus text search. Save a filter as an intelligent playlist.
- Show a second list beside the first as a target: title, BPM, key and Energy, with its track count and length. Add the selected track with one key or by dragging, reorder within it, and swap source and target.
- Mutate a track in a playlist (see Mutants).
- Later: a Camelot wheel that opens on a key, shows the loaded track's compatible keys and filters the list when a key is clicked.

## Intake

- List the tracks that have no Prep colour as a queue, each showing what it still needs. Problems is the same view over tracks whose Prep colour is Problem.
- Load the selected track into a large player: detail waveform with beat grid and phrases, overview, eight hot cue pads, beat jump, grid nudge and tap tempo.
- Edit title and artist in place. Show an agent's proposed value beside the field, accepted with one key.
- Set Energy 1 to 5 and Prep colour.
- Toggle tags by clicking, for every dimension in the Tag schema. Mark the dimensions that are projected to the deck as MyTag categories and list them first. Show a suggested tag distinctly from a set one.
- Use per-track tools: split Stems and mute or solo each Stem, shift the key, change the tempo. Each long-running tool is a Run the DJ can see.
- Work in any order. Show what is still empty for the track.
- Mark the track Clean and load the next one in a single action; mark Problem or skip with one key each.
- Later: a memory cue list, waveform zoom, and pads labelled by role once the DJ's cue convention is defined.

## Mutants

- From a playlist row, choose "mutate". From then on, edits made in the normal editor (Stems muted, key shifted, tempo changed, cues moved) belong to that playlist's version only.
- Show unmistakably, in the player and above the list, that a Mutant is being edited and not the core track, with a way back to the core track.
- Badge a Mutant's row with what differs.
- Grid, Energy and tags always follow the parent. Cues start as the parent's and may diverge.
- A Mutant does not appear in All Tracks. It reaches a Stick as an ordinary track rendered at load time.
- Later: promote a Mutant to a track of its own.

## Review

- List the Facts waiting for the DJ, grouped by Run, as rows of track, field, current value, proposed value and confidence.
- Accept or reject one row, a selection, or everything in a Run above a chosen confidence.
- Load any row's track to listen before deciding.
- List past Runs from the Ledger with their source, date and size. Show a Run's changes and unwind it, or part of it. Show when values changed since would be flagged.
- Show Facts held back for a reason (for example a lookup whose duration does not match).

## Stick

- Show the mounted Stick, whether it is a Gig stick or a Spare stick, when it was last loaded, and its space before and after.
- Summarise what a load would do: tracks added, changed (by field) and removed, Mutants rendered, Native stems written, playlists created and reordered. Offer the full dry run.
- Start the load only by pressing and holding one large button (or holding a key). A Gig stick needs a longer hold. Releasing early cancels.
- While loading, show progress as a full-view display driven by the tracks being written. Shrink it to the status line to keep working in any other view, and bring it back.
- End in an unmistakable "safe to remove" state. Later: a short summary screen of the load.
- A setting reduces the display to a plain progress bar.

## Set plan

- Show a Set plan as a list in order, with the Intensity of each track drawn as a curve across the set against the curve its Recipe asks for.
- Between each pair of rows show why the transition was chosen (tempo change, key move, Intensity step) and where to mix out and in. Mark transitions that break the Recipe.
- Reorder or swap tracks; the difference from the generated plan is recorded.
- Recipes are edited as files, not in the window.

## Sizes

- Main case: mostly maximised on a 1280 x 800 logical-pixel screen.
- Half width (about 640): the tree and the second list give way; the Track Filter bar and key lights remain; Intake's queue becomes a counter and tags reflow.
- Narrow (about 436): lists show title, BPM, key and Energy only. Not yet drawn for Review, Stick and Set plan.
