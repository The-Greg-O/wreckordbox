# The window's theme file

[Documentation](../README.md)

On Linux the window follows your Omarchy theme: its grounds, text, accent and
the system monospace font change when you switch themes. A theme file of its
own lets you change that look without touching the desktop's theme, the way
other Omarchy apps are configured.

## Where it goes

`~/.config/wreckordbox/theme.toml` (or `$XDG_CONFIG_HOME/wreckordbox/theme.toml`
when that is set). Neither the folder nor the file exists until you make them.
Start from the commented example shipped in the repository,
[`theme.example.toml`](theme.example.toml): every line of it is a comment and
documents one key.

```
mkdir -p ~/.config/wreckordbox
cp docs/user/theme.example.toml ~/.config/wreckordbox/theme.toml
```

Saving, creating or deleting the file changes every open window at once; no
restart is needed. The file may be a link into a dotfiles repository: saving
the file it leads to, in place or by an editor's rename, is followed too, and
so is pointing the link elsewhere.

## How it is laid over the Omarchy theme

The look starts from one of two bases:

- `follow = "omarchy"` (the default): the active Omarchy theme and the system
  monospace font, live, exactly as with no file. A theme that lacks some
  colours has them derived as usual.
- `follow = "none"`: the built-in look (dark greys, a blue accent, Arial),
  the colours a machine without Omarchy has. The Omarchy theme and the system
  font are then neither read nor watched. A file with nothing but
  `follow = "none"` gives exactly the built-in look.

Each colour key in the file then replaces exactly the role it names, and
nothing else: a file with only `accent = "#ff8800"` changes the accent and
leaves every other colour on the base, following an Omarchy switch live. Two
things are chosen again when a key they depend on changes: the text written
on accent-filled controls (readable on whatever accent results; chosen from
the deepest ground and the brightest text) and, unless the file sets
`selection` too, a selected row's colour if the text could not be read on
it. Where the base has no tab-line colours of its own (the built-in look,
or a theme without them), the active tab's line is the accent's colour and
follows a new accent.

Colour keys have the same names as an Omarchy theme's `colors.toml`, so you
can paste a theme's palette into the file and edit it from there. Some keys
only stand in for a missing one (`blue` or `color4` for `accent`, and so on)
and some are accepted only so a pasted palette reports nothing; the example
file lists each group.

Colours that carry meaning are never themeable: Energy, Prep colours, key
lights, cue and phrase colours and the waveforms keep their own colours under
every theme and every theme file.

## Waveform colours

The waveform colour theme is chosen in Preferences (View, Color, "Waveform
color") or in this file with `waveform = "3band"`, `"rgb"`, `"blue"` or
`"theme"`. **The file wins:** while it sets `waveform`, every window draws
that theme and Preferences shows it, greyed, with a note that the theme file
sets it. Remove the line to choose in Preferences again; the choice made
there before is kept.

3Band (the default), RGB and Blue are rekordbox's own colours and never
change. Theme takes the active Omarchy theme's `blue`, `magenta` and
`bright_foreground` (or `color4`, `color5`, `color15`), and follows a theme
switch live. It never takes this file's colours: a file's `blue` or
`bright_foreground` changes the window, not the waveform. Under
`follow = "none"`, or with no usable Omarchy theme, Theme draws in Tokyo
Night's blue, magenta and bright foreground. Cue, playhead, beat and phrase
colours are the same under all four.

## Type

`font` and `font_size` set the interface's face and base size (12 px by
default; titles, labels and the status line keep their proportion to it).
`mono_font` and `mono_font_size` set the face for text whose digits line up:
number, BPM, key, time, bitrate and date added in the Library list. Sizes
are 8 to 24 px at 100% interface scale; Ctrl+- and Ctrl+= still scale the
whole interface.

Rows, column headings, list columns, the tree, the top bar and the status
line grow with the sizes, so values still fit at 24 px; a list column you
widened or narrowed keeps that proportion. The old (upstream) layout takes
the face and scales its own measured type sizes and its track list's rows,
headings and columns by `font_size` over 12.

## Mistakes

A mistake in the file never breaks the window. An invalid colour, an unknown
key, a value of the wrong kind, a line that is not valid TOML, or a file that
cannot be read (no permission, not a regular file, larger than 256 KB, not
UTF-8) is left out, the rest of the file applies, and the status line says
"Theme file" with the first problem and its line; the tooltip lists every
one. The same lines are written to the app's log. The note can be dismissed;
it stays away while the same mistakes remain (saving a valid colour or
switching the Omarchy theme does not bring it back), returns when the
mistakes change, and is forgotten once the file is fixed. Making an
unreadable file readable again (for example `chmod`) is picked up at once.

## For tests and screenshots

`WRECKORDBOX_THEME_FILE=/path/to/theme.toml` makes the app read that file
instead of yours, and `WRECKORDBOX_THEME_COLORS=/path/to/colors.toml` stands in
for the Omarchy palette. While only the second is set, no theme file is read.
