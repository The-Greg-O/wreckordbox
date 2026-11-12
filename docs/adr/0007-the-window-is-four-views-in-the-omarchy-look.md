# The window is four views in the Omarchy look, not a match for rekordbox

Upstream's window reproduces rekordbox pixel for pixel. Wreckordbox's window is instead organised around the DJ's jobs: four views (Library, Intake, Review, Stick) that share one tree, one player and one status line, drawn in the system monospace font with colours taken from the active Omarchy theme. We chose this because the agent capabilities need views upstream has no place for, because preparing a track (Intake) is the most frequent and most painful job and deserves a screen of its own, and because a window on a tiling desktop has to work at widths rekordbox's layout does not survive. ADR 0003 already allows the window to be restructured freely; this records that we are doing so.

## Considered Options

- **Reskin upstream's layout** with theme colours and collapsing panels. Rejected: upstream interface fixes would keep applying, but the deck takes a third of a half-width window, tagging stays in menus, and Review, Intake and the Stick load have no natural home.
- **A keyboard-first interface with no tree**, where playlists are filters. Rejected: the DJ wants the playlist tree always visible and wants to move tracks between two visible lists.
- **An agent panel inside the window.** Rejected for now: the agent runs in a terminal in another window and the window updates live either way.

## Consequences

- Upstream fixes to the window's layout and styling no longer cherry-pick; they are ported by hand when wanted. Waveform canvases and the player-format crates are unaffected.
- Tokens that describe the music (waveform bands, phrases, cue colours) stay as upstream measured them; tokens that describe rekordbox's chrome are replaced by roles read from the Omarchy theme.
- The native menu row is dropped on Linux in favour of a command palette, so every action needs a palette entry.
- Tag dimensions are drawn from the Tag schema, so the window cannot assume four categories.
