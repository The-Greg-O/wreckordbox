# Tokens

The proposed type, colour, spacing and density system. Values shown are the ones used in `screens/mock.html`, drawn with the Tokyo Night theme.

## Relation to upstream

Upstream's `src/styles/tokens.css` measures every size and colour against rekordbox. This design keeps the tokens that describe the music and replaces the ones that describe rekordbox's chrome.

| Kept from upstream (fixed in every theme) | Replaced (follow the Omarchy theme) |
| --- | --- |
| 3Band waveform colours (`--c-wave-*`) | Backgrounds (`--c-black`, `--c-top`, `--c-panel`, `--c-raised`, `--c-button`) |
| Phrase colours (`--c-phrase-*`) | Text (`--c-text`, `--c-text-dim`) |
| Hot cue and pad colours | Accent and selection (`--c-accent`, `--c-select`) |
| Playhead, beat and downbeat markers | Fonts and every size |
| Meter colours | |

Because the layout and sizes change, upstream interface fixes will not cherry-pick into the window (ADR 0007). The canvases that draw waveforms can stay close to upstream.

## Colour roles from the Omarchy theme

Read from the active theme's `colors.toml`; the window re-reads it when the theme changes.

| Role | Theme key | Tokyo Night |
| --- | --- | --- |
| Deepest ground (top bar, status line, waveform wells) | `darker_background` | `#0e0e14` |
| Panel (tree, player, target list) | `dark_background` | `#13141c` |
| Working area | `background` | `#1a1b26` |
| Hairlines, unset pips | `lighter_background` | `#24283b` |
| Selected row | `selection` | `#292e42` |
| Control borders | `muted` | `#414868` |
| Text | `foreground` | `#a9b1d6` |
| Secondary text, labels | `dark_foreground` | `#565f89` |
| Emphasised text | `bright_foreground` | `#c0caf5` |
| Accent (focus, active filter, deck marker) | `accent` | `#7aa2f7` |
| Text and glyphs on an accent fill | derived: whichever of `darker_background` and `bright_foreground` stands out more from the accent, or black or white where neither reaches 4.5:1 | `#0e0e14` |

The active view tab carries a two-colour line (`bright_magenta` to `bright_yellow`); the Stick load display uses the same pair. These are the only gradients.

The active theme's file is `~/.local/state/omarchy/current/theme/colors.toml`. In CSS each role is a `--role-*` custom property (`src/styles/tokens.css`, set by `src/lib/themeRoles.ts`).

### Fallbacks

- **No theme** (no Omarchy, macOS, Windows, the browser mock, or a file without both `background` and `foreground`): every role keeps upstream's colour, so the window looks as upstream's does.
- **A partial file:** a missing role is derived from the file's own `background` and `foreground` (darker grounds towards black; hairline, selection and borders a step towards the foreground; secondary text a step towards the background), never taken from upstream, so two palettes are not mixed. A selection colour too close to the text colour to read is replaced the same way.
- **A light theme:** the roles are the theme's, for the new views. Upstream's layout, until it is removed (wbx-77), keeps its dark grounds and text and takes only the accent and the font, because its icons are drawn for a dark ground; it stays readable, it does not turn light.
- **Lit controls in upstream's layout:** every lit state of the chrome (the AU, Q and MASTER buttons, the active player tab, the open side-rail box, Preferences' selections, the Track Filter's picked rows, tick boxes) is the accent under a theme, with the on-accent colour written on it. Without a theme they keep upstream's measured blues.
- **Font:** where fontconfig cannot name a monospace family, upstream's face.

## Fixed colours (meaning, not theme)

- **Prep colour:** rekordbox's own label colours, shown as a 3 px stripe at the row's left edge.
- **Energy:** five pips coloured by level, cool to hot (1 blue, 2 cyan, 3 green, 4 orange, 5 red). Unset pips are a fixed slate (`--c-energy-0`). In upstream's layout, until it is removed, Energy is five stars in the same colours: lit stars in the track's level colour, unlit ones in the unset colour, never the theme's text colour.
- **Key lights:** green fill for a key that mixes with the loaded track, yellow fill for an energy-boost move, no fill otherwise.
- **Tag dimensions:** one hue per dimension for set tags; an unset tag is outlined in the hairline colour; a suggested tag has a dashed yellow outline.
- **Mutant:** magenta badge and banner. Magenta is used for nothing else in a list.
- **Waveform themes:** 3Band (upstream's measured colours, the default), RGB, Blue, and Theme (drawn from the Omarchy theme's blue, magenta and bright foreground). Cue, playhead and phrase colours do not change with the waveform theme.

## Type

- One family: the Omarchy system monospace font (JetBrains Mono Nerd Font unless the DJ changes it), read from the system, not bundled.
- Sizes at 100% scale: 12 px for lists and controls, 13 px bold for a list's title, 10.5 px uppercase with 0.08 em tracking for labels and column headings, 11 px for the status line.
- Numbers are tabular everywhere.

## Density and spacing

- List row: 24 px. Column heading row: 22 px. Set plan transition line: 19 px. Intake queue row: three lines, about 58 px.
- Top bar 30 px, compact player about 58 px including the phrase strip, status line 22 px.
- Tree 186 px wide; compact target list 336 px wide.
- Spacing steps: 2, 4, 6, 8, 10, 14 px. Corners 2 px. Borders 1 px.
- All of it multiplies by the whole-interface scale. Canvases redraw at device resolution at every scale.

## Breakpoints

By the window's own width, not the screen's: at 800 px and below the tree, the second list, and the artist, MyTags and date-added columns give way and the tag grid goes from three columns to two.
