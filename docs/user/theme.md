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
restart is needed.

## How it is laid over the Omarchy theme

The look is built in three layers, the later winning key by key:

1. The built-in look (dark greys, a blue accent, Arial): what the window has
   on a machine without Omarchy.
2. The active Omarchy theme's palette and the system monospace font, while
   `follow = "omarchy"` (the default).
3. The theme file's keys.

So a file with only `accent = "#ff8800"` changes the accent and leaves every
other colour following the Omarchy theme, live. With `follow = "none"` the
Omarchy theme is not read at all and the file's keys are laid over the
built-in look; a file with nothing but `follow = "none"` gives exactly the
built-in look.

Colour keys have the same names as an Omarchy theme's `colors.toml`, so you can
paste a theme's palette into the file and edit it from there. From the
combined palette the window derives its roles exactly as it does for an
Omarchy theme; anything missing is derived from `background` and
`foreground`, and the text written on accent-filled controls is always chosen
to be readable on whatever accent results.

Colours that carry meaning are never themeable: Energy, Prep colours, key
lights, cue and phrase colours and the waveforms keep their own colours under
every theme and every theme file. Waveform colour themes are a separate
setting.

## Type

`font` and `font_size` set the interface's face and base size (12 px by
default; titles, labels and the status line keep their proportion to it).
`mono_font` and `mono_font_size` set the face for text whose digits line up:
BPM, key, time, bitrate and dates in the list. Sizes are 8 to 24 px at 100%
interface scale; Ctrl+- and Ctrl+= still scale the whole interface.

The sizes apply to the new view shell. The old (upstream) layout takes the
`font` face but keeps its own measured sizes until it is removed.

## Mistakes

A mistake in the file never breaks the window. An invalid colour, an unknown
key, a value of the wrong kind or a line that is not valid TOML is left out,
the rest of the file applies, and a notice at the bottom of the window lists
each problem with its line number. The same lines are written to the app's
log. The notice can be dismissed; it comes back if the file still has a
problem the next time it is saved.

## For tests and screenshots

`WRECKORDBOX_THEME_FILE=/path/to/theme.toml` makes the app read that file
instead of yours, and `WRECKORDBOX_THEME_COLORS=/path/to/colors.toml` stands in
for the Omarchy palette. While only the second is set, no theme file is read.
