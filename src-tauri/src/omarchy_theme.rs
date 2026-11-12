//! The window's chrome follows the DJ's active Omarchy theme.
//!
//! Omarchy keeps the applied theme as a folder of generated files at
//! `~/.local/state/omarchy/current/theme/`; its palette is `colors.toml`, a
//! flat list of `key = "#rrggbb"` lines. `omarchy theme set` builds the next
//! theme beside it, removes that folder and renames the new one into place,
//! then rewrites `current/theme.name` and `current/theme.generation`. So the
//! folder to watch is `current/`, never the theme folder itself, which does
//! not survive a switch. The system monospace font is whatever fontconfig
//! answers for `monospace` (`omarchy font set` writes
//! `~/.config/fontconfig/fonts.conf`).
//!
//! This module turns that palette into the roles of `docs/design/tokens.md`
//! and hands them to the interface, which sets them as CSS custom properties
//! (`src/lib/themeRoles.ts`). With no theme there are no roles and the
//! interface keeps the defaults in `src/styles/tokens.css`, which are
//! upstream's colours; that is also what macOS and Windows get. Colours that
//! carry meaning (waveform bands, cues, phrases, Prep colours, meters) are
//! not roles and never pass through here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::theme_file::{Follow, ThemeFile, ThemeProblem};

/// Points the module at a `colors.toml` of the caller's choosing instead of
/// the active Omarchy theme's. For tests and screenshots: the DJ's real theme
/// is never switched to look at another one.
pub const COLORS_FILE_ENV: &str = "WRECKORDBOX_THEME_COLORS";

/// Sent to every window when the theme or the font changes.
pub const CHANGED_EVENT: &str = "theme:changed";

/// The colours of the chrome, by what they are for. Every value is
/// `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeRoles {
    /// `dark` or `light`.
    pub mode: String,
    /// Deepest ground: top bar, status line, waveform wells.
    pub ground: String,
    /// Panels: tree, player, target list.
    pub panel: String,
    /// The working area.
    pub working: String,
    /// Hairlines and unset pips.
    pub hairline: String,
    /// A selected row.
    pub selection: String,
    /// Control borders.
    pub border: String,
    pub text: String,
    /// Secondary text and labels.
    pub text_dim: String,
    /// Emphasised text.
    pub text_bright: String,
    /// Focus, the active filter, the deck marker.
    pub accent: String,
    /// Text and glyphs on a control filled with the accent.
    pub on_accent: String,
    /// The two ends of the active view tab's line.
    pub tab_from: String,
    pub tab_to: String,
    /// A control's fill, where it is not the hairline colour: only the
    /// built-in look tells the two apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    /// A lit control under the pointer, where the look gives its own rather
    /// than the one the window derives from the accent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_hover: Option<String>,
    /// The built-in look (with the theme file's keys over it), not a theme:
    /// upstream's layout keeps its own lit colours.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub built_in: bool,
}

/// What the interface is told: the roles, if a usable theme is present, and
/// the interface's face, if the theme file or the system names one. The
/// sizes and the monospace face come from the theme file only, and so do
/// the problems.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub roles: Option<ThemeRoles>,
    /// The face for upstream's layout (`--role-font`): the file's `font`, or
    /// the system monospace font while following Omarchy.
    pub font: Option<String>,
    /// The theme file's `font`, `font_size`, `mono_font` and
    /// `mono_font_size`, for the view shell; none of them come from Omarchy.
    pub interface_font: Option<String>,
    pub interface_font_size: Option<f64>,
    pub mono_font: Option<String>,
    pub mono_font_size: Option<f64>,
    /// The theme file's mistakes, each left out of what is shown.
    pub problems: Vec<ThemeProblem>,
    /// The Omarchy theme's colours for the Theme waveform, if the look
    /// follows a usable Omarchy theme. Never the theme file's: waveform
    /// colours are not themeable by it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wave: Option<WaveColours>,
    /// The theme file's waveform colour theme, which wins over Preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waveform: Option<String>,
}

/// The three band colours of the Theme waveform, each `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaveColours {
    /// The low band (outer): the theme's blue.
    pub low: String,
    /// The mid band: the theme's magenta.
    pub mid: String,
    /// The high band (core): the theme's bright foreground.
    pub high: String,
}

/// The Theme waveform's colours in an Omarchy palette, or `None` unless the
/// palette is usable (as for the roles) and gives all three, so the window
/// never mixes the theme's colours with its fallbacks.
fn wave_from_colors(text: &str) -> Option<WaveColours> {
    let colours = parse_colors(text);
    roles_from_palette(&colours)?;
    let get = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| colours.get(*key).and_then(|value| rgb(value)))
            .map(hex)
    };
    Some(WaveColours {
        low: get(&["blue", "color4"])?,
        mid: get(&["magenta", "color5"])?,
        high: get(&["bright_foreground", "color15"])?,
    })
}

/// The theme the window shows. The base roles are resolved first: the
/// Omarchy theme's (where the file follows it and its palette is usable), or
/// the built-in look's. The theme file's keys then replace the roles they
/// name, and only those; what depends on contrast with a replaced role (the
/// label on the accent, a selection the text could not be read on) is
/// chosen again. With no file this is exactly the Omarchy theme.
/// `system_font` is asked only when the look follows Omarchy and the file
/// names no face.
#[must_use]
pub fn layered(
    omarchy: Option<&str>,
    file: Option<&ThemeFile>,
    system_font: impl FnOnce() -> Option<String>,
) -> Theme {
    let follows = file.is_none_or(|file| file.follow == Follow::Omarchy);
    let base = omarchy.filter(|_| follows).and_then(roles_from_colors);
    let over = file
        .map(|file| &file.colours)
        .filter(|colours| !colours.is_empty());
    let roles = match over {
        None => base,
        Some(over) => Some(overlay(&base.unwrap_or_else(built_in_roles), over)),
    };
    let interface_font = file.and_then(|file| file.font.clone());
    let font = match &interface_font {
        Some(face) => Some(face.clone()),
        None if follows => system_font(),
        None => None,
    };
    Theme {
        roles,
        font,
        interface_font,
        interface_font_size: file.and_then(|file| file.font_size),
        mono_font: file.and_then(|file| file.mono_font.clone()),
        mono_font_size: file.and_then(|file| file.mono_font_size),
        problems: file.map(|file| file.problems.clone()).unwrap_or_default(),
        wave: omarchy.filter(|_| follows).and_then(wave_from_colors),
        waveform: file.and_then(|file| file.waveform.clone()),
    }
}

/// The built-in look as roles: exactly the fallbacks of the `--role-*`
/// properties in `src/styles/tokens.css`, upstream's colours.
fn built_in_roles() -> ThemeRoles {
    let colour = |value: &str| value.to_owned();
    ThemeRoles {
        mode: colour("dark"),
        ground: colour("#000000"),
        panel: colour("#0f0f0f"),
        working: colour("#1e1e1e"),
        hairline: colour("#323232"),
        selection: colour("#314b67"),
        border: colour("#646464"),
        text: colour("#e6e6e6"),
        text_dim: colour("#9a9a9a"),
        text_bright: colour("#ffffff"),
        accent: colour("#1373eb"),
        on_accent: colour("#f0f0f0"),
        tab_from: colour("#1373eb"),
        tab_to: colour("#1373eb"),
        control: Some(colour("#2b2b2b")),
        accent_hover: Some(colour("#4a82ed")),
        built_in: true,
    }
}

/// The theme file's keys that name each role, first wins: the key itself,
/// then the ones that stand in for it in an Omarchy palette.
pub(crate) const ROLE_KEYS: &[(&str, &[&str])] = &[
    ("ground", &["darker_background"]),
    ("panel", &["dark_background"]),
    ("working", &["background"]),
    ("hairline", &["lighter_background"]),
    ("selection", &["selection"]),
    ("border", &["muted"]),
    ("text", &["foreground"]),
    ("text_dim", &["dark_foreground"]),
    ("text_bright", &["bright_foreground", "color15"]),
    ("accent", &["accent", "blue", "color4"]),
    (
        "tab_from",
        &["bright_magenta", "color13", "magenta", "color5"],
    ),
    ("tab_to", &["bright_yellow", "color11", "yellow", "color3"]),
];

/// `base` with the roles `file` names replaced (see `layered`).
fn overlay(base: &ThemeRoles, file: &BTreeMap<String, String>) -> ThemeRoles {
    let given = |role: &str| {
        ROLE_KEYS
            .iter()
            .find(|(name, _)| *name == role)
            .and_then(|(_, keys)| keys.iter().find_map(|key| file.get(*key)))
            .filter(|value| rgb(value).is_some())
            .cloned()
    };
    let mut roles = base.clone();
    let set = |role: &str, slot: &mut String| {
        if let Some(value) = given(role) {
            *slot = value;
        }
    };
    set("ground", &mut roles.ground);
    set("panel", &mut roles.panel);
    set("working", &mut roles.working);
    set("selection", &mut roles.selection);
    set("border", &mut roles.border);
    set("text", &mut roles.text);
    set("text_dim", &mut roles.text_dim);
    set("text_bright", &mut roles.text_bright);
    set("accent", &mut roles.accent);
    set("tab_from", &mut roles.tab_from);
    set("tab_to", &mut roles.tab_to);
    if let Some(hairline) = given("hairline") {
        // A control's fill is the hairline colour in a theme.
        roles.hairline = hairline;
        roles.control = None;
    }
    if let Some(mode) = file
        .get("mode")
        .filter(|mode| matches!(mode.as_str(), "dark" | "light"))
    {
        roles.mode.clone_from(mode);
    }
    // A tab end the base took from its accent (it had none of its own)
    // stays the accent's colour.
    if roles.accent != base.accent {
        for (role, slot) in [
            ("tab_from", &mut roles.tab_from),
            ("tab_to", &mut roles.tab_to),
        ] {
            if given(role).is_none() && *slot == base.accent {
                slot.clone_from(&roles.accent);
            }
        }
    }
    let colour = |value: &str| rgb(value).unwrap_or_default();
    // The label on the accent is chosen from the accent, the deepest ground
    // and the brightest text; choose again only if one of them changed.
    if (
        roles.accent.as_str(),
        roles.ground.as_str(),
        roles.text_bright.as_str(),
    ) != (
        base.accent.as_str(),
        base.ground.as_str(),
        base.text_bright.as_str(),
    ) {
        let label = on_accent(
            colour(&roles.accent),
            [colour(&roles.ground), colour(&roles.text_bright)],
        );
        roles.on_accent = hex(label);
        roles.accent_hover = None;
    }
    // A selection the text cannot be read on is replaced, as for a theme.
    if given("selection").is_none()
        && contrast(colour(&roles.selection), colour(&roles.text)) < SELECTION_CONTRAST
    {
        roles.selection = hex(mix(colour(&roles.working), colour(&roles.text), 0.18));
    }
    roles
}

type Rgb = [u8; 3];

/// The `key = "#rrggbb"` lines of a `colors.toml`, and `mode`. Anything else
/// in the file (a table, a value that is not a colour, a broken line) is
/// passed over rather than failing the whole theme.
fn parse_colors(text: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.starts_with('#') || key.starts_with('[') {
            continue;
        }
        let value = value.trim();
        let value = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or_default(),
            _ => value.split_whitespace().next().unwrap_or_default(),
        };
        if key == "mode" || rgb(value).is_some() {
            found
                .entry(key.to_owned())
                .or_insert_with(|| value.to_ascii_lowercase());
        }
    }
    found
}

/// Whether `value` is a `#rrggbb` colour.
pub(crate) fn is_colour(value: &str) -> bool {
    rgb(value).is_some()
}

fn rgb(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn hex(colour: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2])
}

/// `from` moved `share` of the way to `to`.
fn mix(from: Rgb, to: Rgb, share: f64) -> Rgb {
    let channel = |at: usize| {
        let value = f64::from(from[at]) + (f64::from(to[at]) - f64::from(from[at])) * share;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a byte first"
        )]
        let byte = value.round().clamp(0.0, 255.0) as u8;
        byte
    };
    [channel(0), channel(1), channel(2)]
}

/// WCAG relative luminance.
fn luminance(colour: Rgb) -> f64 {
    let linear = |byte: u8| {
        let value = f64::from(byte) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(colour[0]) + 0.7152 * linear(colour[1]) + 0.0722 * linear(colour[2])
}

/// WCAG contrast ratio, 1 (none) to 21.
fn contrast(one: Rgb, other: Rgb) -> f64 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// Below this a selected row's text cannot be read on its ground.
const SELECTION_CONTRAST: f64 = 2.0;

/// What a label on an accent-filled control needs against the accent.
const ON_ACCENT_CONTRAST: f64 = 4.5;

/// The colour for text on a control filled with `accent`: the theme's own
/// darkest ground or brightest text, whichever stands out more, and plain
/// black or white where neither of the theme's stands out enough. An accent
/// can be any colour at all, the text colour included, so nothing fixed is
/// safe on it.
fn on_accent(accent: Rgb, candidates: [Rgb; 2]) -> Rgb {
    let best = |colours: [Rgb; 2]| {
        if contrast(colours[0], accent) >= contrast(colours[1], accent) {
            colours[0]
        } else {
            colours[1]
        }
    };
    let own = best(candidates);
    if contrast(own, accent) >= ON_ACCENT_CONTRAST {
        own
    } else {
        best([[0, 0, 0], [255, 255, 255]])
    }
}

/// The roles for a `colors.toml`, or `None` when it does not give both a
/// `background` and a `foreground`, in which case the interface keeps its
/// built-in colours whole rather than mixing two palettes.
///
/// A key the file lacks is derived from the ones it has, so a partial theme
/// stays a coherent one. Themes older than `colors.toml` get theirs generated
/// from their terminal palette with only `accent`, `selection`, `background`,
/// `foreground` and `color0` to `color15`; that is the partial file met in
/// practice, and the numbered colours stand in for the named ones.
#[must_use]
pub fn roles_from_colors(text: &str) -> Option<ThemeRoles> {
    roles_from_palette(&parse_colors(text))
}

/// The roles for a palette already read: `colors.toml`'s keys to their
/// `#rrggbb` values, and `mode`.
fn roles_from_palette(colours: &BTreeMap<String, String>) -> Option<ThemeRoles> {
    let get = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| colours.get(*key).and_then(|value| rgb(value)))
    };
    let background = get(&["background"])?;
    let foreground = get(&["foreground"])?;
    let light = match colours.get("mode").map(String::as_str) {
        Some("light") => true,
        Some("dark") => false,
        _ => luminance(background) > luminance(foreground),
    };
    // Omarchy's darker grounds go towards black in light themes too, only by
    // less.
    let (dark, darker) = if light { (0.05, 0.10) } else { (0.25, 0.45) };
    let black = [0, 0, 0];
    let accent = get(&["accent", "blue", "color4"]).unwrap_or(foreground);
    // A theme generated from a terminal palette with no selection colour is
    // given the foreground as its selection, which would hide a selected
    // row's text.
    let selection = get(&["selection"])
        .filter(|selection| contrast(*selection, foreground) >= SELECTION_CONTRAST)
        .unwrap_or_else(|| mix(background, foreground, 0.18));
    let ground = get(&["darker_background"]).unwrap_or_else(|| mix(background, black, darker));
    let text_bright = get(&["bright_foreground", "color15"]).unwrap_or(foreground);
    Some(ThemeRoles {
        mode: if light { "light" } else { "dark" }.to_owned(),
        ground: hex(ground),
        panel: hex(get(&["dark_background"]).unwrap_or_else(|| mix(background, black, dark))),
        working: hex(background),
        hairline: hex(
            get(&["lighter_background"]).unwrap_or_else(|| mix(background, foreground, 0.10))
        ),
        selection: hex(selection),
        border: hex(get(&["muted"]).unwrap_or_else(|| mix(background, foreground, 0.30))),
        text: hex(foreground),
        text_dim: hex(
            get(&["dark_foreground"]).unwrap_or_else(|| mix(foreground, background, 0.45))
        ),
        text_bright: hex(text_bright),
        accent: hex(accent),
        on_accent: hex(on_accent(accent, [ground, text_bright])),
        tab_from: hex(get(&["bright_magenta", "color13", "magenta", "color5"]).unwrap_or(accent)),
        tab_to: hex(get(&["bright_yellow", "color11", "yellow", "color3"]).unwrap_or(accent)),
        control: None,
        accent_hover: None,
        built_in: false,
    })
}

/// The roles in the colour file at `path`; `None` when it is missing,
/// unreadable or unusable.
#[must_use]
pub fn roles_from_file(path: &Path) -> Option<ThemeRoles> {
    roles_from_colors(&palette_text(path)?)
}

/// The text of the colour file at `path`, if it is one.
fn palette_text(path: &Path) -> Option<String> {
    // A palette is a few hundred bytes; anything large is not one.
    let bytes = crate::theme_file::read_small(path, 256 * 1024).ok()?;
    String::from_utf8(bytes).ok()
}

/// The colour file to read: the override, or the active Omarchy theme's on
/// Linux. Other systems have none and keep upstream's colours.
#[must_use]
pub fn colors_file() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(COLORS_FILE_ENV).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "linux") {
        // Omarchy writes under `$HOME/.local/state` whatever XDG_STATE_HOME is.
        return Some(dirs::home_dir()?.join(".local/state/omarchy/current/theme/colors.toml"));
    }
    None
}

/// A family name as it may be written into a CSS `font-family`: the first of
/// fontconfig's comma-separated aliases, refused if it holds anything that
/// could end the quoted string it will be put in.
pub(crate) fn css_family(answer: &str) -> Option<String> {
    let family = answer.lines().next()?.split(',').next()?.trim();
    let safe = !family.is_empty()
        && family.len() <= 100
        && family
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '+' | '(' | ')'));
    safe.then(|| family.to_owned())
}

/// The system monospace family, asked of fontconfig as `omarchy font
/// current` asks it. Linux only: elsewhere the interface keeps its own face.
#[must_use]
pub fn system_monospace_font() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let answer = std::process::Command::new("fc-match")
        .args(["monospace", "-f", "%{family}\n"])
        .output()
        .ok()?;
    if !answer.status.success() {
        return None;
    }
    css_family(&String::from_utf8_lossy(&answer.stdout))
}

/// Where the theme and the font are read from, and so what a switch of
/// either writes to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sources {
    /// The colour file, if this system has a place for one.
    pub colors: Option<PathBuf>,
    /// True for an Omarchy theme: `colors` sits in a theme folder that a
    /// switch replaces whole (or a link that is pointed elsewhere), so it is
    /// the folder holding the theme folder that has to be watched. False for
    /// a file that is rewritten where it is.
    pub theme_folder_is_replaced: bool,
    /// The folder `omarchy font set` writes `fonts.conf` into.
    pub font_folder: Option<PathBuf>,
    /// The window's own theme file (`theme_file.rs`), laid over the theme.
    /// It and its folder need not exist.
    pub theme_file: Option<PathBuf>,
}

impl Sources {
    /// The override where it is set, else the active Omarchy theme on Linux.
    /// Other systems have nothing to read and keep upstream's colours.
    #[must_use]
    pub fn of_this_system() -> Self {
        let overridden = std::env::var_os(COLORS_FILE_ENV).is_some_and(|path| !path.is_empty());
        Self {
            colors: colors_file(),
            theme_folder_is_replaced: !overridden,
            font_folder: cfg!(target_os = "linux")
                .then(dirs::config_dir)
                .flatten()
                .map(|config| config.join("fontconfig")),
            theme_file: crate::theme_file::theme_file(),
        }
    }

    /// The theme as these sources give it now.
    #[must_use]
    pub fn read(&self) -> Theme {
        self.read_following().0
    }

    /// The theme, and whether it follows the Omarchy theme (the theme
    /// file's `follow`). Under `follow = "none"` neither the Omarchy palette
    /// nor the system font is read.
    fn read_following(&self) -> (Theme, bool) {
        let file = self.theme_file.as_deref().and_then(crate::theme_file::read);
        let follows = file
            .as_ref()
            .is_none_or(|file| file.follow == Follow::Omarchy);
        let omarchy = self
            .colors
            .as_deref()
            .filter(|_| follows)
            .and_then(palette_text);
        let theme = layered(omarchy.as_deref(), file.as_ref(), system_monospace_font);
        (theme, follows)
    }

    /// What to watch for a change of the theme. While the look follows
    /// Omarchy: the folders an Omarchy switch or `omarchy font set` writes
    /// into, whole. Always: the theme file, by name, in its folder, and, if
    /// it is a link (a dotfiles repository), the file it leads to, by name,
    /// in that file's folder. None of them need exist.
    #[must_use]
    pub fn watches(&self, follows: bool) -> Vec<Watch> {
        let whole = |folder: &Path| Watch {
            folder: folder.to_path_buf(),
            entry: None,
        };
        let named = |file: &Path| {
            Some(Watch {
                folder: file.parent()?.to_path_buf(),
                entry: Some(file.file_name()?.to_os_string()),
            })
        };
        let mut watches = Vec::new();
        if follows {
            // The folder the colour file is in, for a file rewritten where
            // it is; and for an Omarchy theme also the folder holding that
            // one, where a switch removes the theme folder and puts another
            // in.
            let theme = self.colors.as_deref().and_then(Path::parent);
            let holding = theme
                .filter(|_| self.theme_folder_is_replaced)
                .and_then(Path::parent);
            watches.extend(
                holding
                    .into_iter()
                    .chain(theme)
                    .chain(self.font_folder.as_deref())
                    .map(whole),
            );
        }
        if let Some(file) = self.theme_file.as_deref() {
            watches.extend(named(file));
            let target = std::fs::canonicalize(file).ok().filter(|target| {
                file.parent()
                    .and_then(|folder| std::fs::canonicalize(folder).ok())
                    != target.parent().map(Path::to_path_buf)
            });
            watches.extend(target.as_deref().and_then(named));
        }
        watches.retain(|watch| watch.folder.is_absolute());
        watches
    }
}

/// One thing the theme watcher waits on: a folder, whole or for one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    pub folder: PathBuf,
    /// The one entry of `folder` that matters; `None` for all of them.
    pub entry: Option<std::ffi::OsString>,
}

/// Writes the theme file's mistakes to the log.
fn log_problems(sources: &Sources, theme: &Theme) {
    let file = sources.theme_file.as_deref().unwrap_or(Path::new(""));
    for problem in &theme.problems {
        tracing::warn!(file = %file.display(), line = problem.line, "theme file: {}", problem.message);
    }
}

/// The theme as it stands now.
#[must_use]
pub fn current() -> Theme {
    Sources::of_this_system().read()
}

/// The theme, for a window that has just opened.
#[tauri::command]
pub fn theme_roles() -> Theme {
    current()
}

/// Tells every window when the theme changes, for as long as the app runs.
pub fn follow<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    follow_sources(app, Sources::of_this_system());
}

/// Tells every window when what `sources` name changes, for as long as the
/// app runs.
///
/// Waits on the folders a switch writes into (inotify, no polling) and reads
/// the theme again once the writes have settled. A folder that is not there
/// yet, or that is removed and made again, is picked up when it appears: the
/// nearest folder above it that does exist is watched for it meanwhile.
/// Linux only; elsewhere there is no theme to follow.
pub fn follow_sources<R: tauri::Runtime>(app: &tauri::AppHandle<R>, sources: Sources) {
    #[cfg(target_os = "linux")]
    {
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("theme-watch".into())
            .spawn(move || {
                watch::run(
                    |follows| sources.watches(follows),
                    || sources.read_following(),
                    |theme, announce| {
                        log_problems(&sources, theme);
                        if !announce {
                            return;
                        }
                        tracing::info!(themed = theme.roles.is_some(), font = ?theme.font, "theme changed");
                        let _ = tauri::Emitter::emit(&app, CHANGED_EVENT, theme);
                    },
                );
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start the theme watcher");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (app, sources);
}

#[cfg(target_os = "linux")]
mod watch {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use inotify::{Event, EventMask, Inotify, WatchDescriptor, WatchMask};

    use super::{Theme, Watch};

    /// A switch is several writes over a moment; read once they stop.
    const SETTLE: Duration = Duration::from_millis(250);

    /// One mask for every watch: a folder can be watched both for itself and
    /// as the nearest existing folder above another, and inotify keeps one
    /// mask per folder. `ATTRIB`: a file or folder made readable again.
    fn mask() -> WatchMask {
        WatchMask::CREATE
            | WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE
            | WatchMask::DELETE_SELF
            | WatchMask::MOVE_SELF
            | WatchMask::ATTRIB
    }

    /// What a watched folder is watched for.
    #[derive(Default)]
    struct Interest {
        /// Anything in it: it is one of the folders a switch writes into.
        everything: bool,
        /// Entries of these names: a file that matters, or the next step
        /// down to a folder that does not exist yet.
        names: Vec<OsString>,
    }

    struct Watches {
        inotify: Inotify,
        interests: HashMap<WatchDescriptor, Interest>,
    }

    impl Watches {
        fn interest(&mut self, folder: &Path) -> Option<&mut Interest> {
            let watch = self.inotify.watches().add(folder, mask()).ok()?;
            Some(self.interests.entry(watch).or_default())
        }

        fn want(&mut self, folder: &Path, name: &OsString) -> bool {
            let Some(interest) = self.interest(folder) else {
                return false;
            };
            if !interest.names.contains(name) {
                interest.names.push(name.clone());
            }
            true
        }

        /// Watches `watch.folder` for what `watch` asks, or while it does
        /// not exist (or cannot be watched: no permission) the nearest
        /// folder above it that can be, for the entry that leads down to it.
        fn reach(&mut self, watch: &Watch) {
            let mut folder = watch.folder.clone();
            // Each pass goes one level down; a path is not deeper than this.
            for _ in 0..64 {
                if folder.is_dir() {
                    let watched = match &watch.entry {
                        None => self
                            .interest(&folder)
                            .map(|interest| interest.everything = true)
                            .is_some(),
                        Some(name) => self.want(&folder, name),
                    };
                    if watched {
                        return;
                    }
                    // There, but not to be watched: wait in the folder above
                    // for it to change.
                    if let (Some(above), Some(name)) = (folder.parent(), folder.file_name()) {
                        self.want(above, &name.to_os_string());
                    }
                    return;
                }
                let Some((above, name)) = nearest_existing(&folder) else {
                    return;
                };
                if !self.want(&above, &name) {
                    return;
                }
                // It may have appeared between the look and the watch; if
                // so go round again and step down into it.
                if !above.join(&name).is_dir() {
                    return;
                }
                folder.clone_from(&watch.folder);
            }
        }

        /// Starts again from nothing: folders come and go, and a watch on a
        /// removed folder is dead.
        fn register(&mut self, watches: &[Watch]) {
            for (watch, _) in self.interests.drain() {
                let _ = self.inotify.watches().remove(watch);
            }
            for watch in watches {
                self.reach(watch);
            }
        }

        /// Whether `event` could mean a change of the theme. An event for a
        /// watch that has since been dropped is not one.
        fn concerns(&self, event: &Event<&std::ffi::OsStr>) -> bool {
            let Some(interest) = self.interests.get(&event.wd) else {
                return false;
            };
            let gone = EventMask::DELETE_SELF | EventMask::MOVE_SELF | EventMask::IGNORED;
            interest.everything
                || event.mask.intersects(gone)
                // About the watched folder itself: its permissions changed.
                || event.name.is_none()
                || event
                    .name
                    .is_some_and(|name| interest.names.iter().any(|wanted| wanted == name))
        }
    }

    /// The deepest existing folder above `folder`, and the name of the entry
    /// in it that leads towards `folder`.
    fn nearest_existing(folder: &Path) -> Option<(PathBuf, OsString)> {
        let mut below = folder;
        loop {
            let above = below.parent()?;
            if above.is_dir() {
                return Some((above.to_path_buf(), below.file_name()?.to_os_string()));
            }
            below = above;
        }
    }

    /// Watches what `plan` names for the look's current `follow`, then
    /// reads; if the read finds `follow` changed, plans and reads again.
    /// Watch first, read second: a write that lands after the read is then
    /// already watched for.
    fn refresh(
        watches: &mut Watches,
        plan: &impl Fn(bool) -> Vec<Watch>,
        read: &impl Fn() -> (Theme, bool),
        mut follows: bool,
    ) -> (Theme, bool) {
        for _ in 0..3 {
            watches.register(&plan(follows));
            let (theme, now) = read();
            if now == follows {
                return (theme, now);
            }
            follows = now;
        }
        read()
    }

    /// Calls `tell` with the theme each time what `read` returns changes
    /// after something happened in what `plan` names, with `true`; once at
    /// the start with `false`. Returns only if inotify fails.
    pub(super) fn run(
        plan: impl Fn(bool) -> Vec<Watch>,
        read: impl Fn() -> (Theme, bool),
        mut tell: impl FnMut(&Theme, bool),
    ) {
        let Ok(inotify) = Inotify::init() else { return };
        let mut watches = Watches {
            inotify,
            interests: HashMap::new(),
        };
        let (mut shown, mut follows) = refresh(&mut watches, &plan, &read, true);
        tell(&shown, false);
        let mut buffer = [0_u8; 4096];
        loop {
            if watches.interests.is_empty() {
                return;
            }
            // Blocks until something happens in a watched folder.
            let Ok(mut events) = watches.inotify.read_events_blocking(&mut buffer) else {
                return;
            };
            let interests = &watches;
            if !events.any(|event| interests.concerns(&event)) {
                continue;
            }
            loop {
                std::thread::sleep(SETTLE);
                let Ok(mut events) = watches.inotify.read_events(&mut buffer) else {
                    break;
                };
                let interests = &watches;
                if !events.any(|event| interests.concerns(&event)) {
                    break;
                }
            }
            let now;
            (now, follows) = refresh(&mut watches, &plan, &read, follows);
            if now != shown {
                tell(&now, true);
                shown = now;
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    /// Omarchy's Tokyo Night `colors.toml`, as shipped.
    const TOKYO_NIGHT: &str = r##"mode = "dark"

accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"

background = "#1a1b26"
dark_background = "#13141c"
darker_background = "#0e0e14"
lighter_background = "#24283b"

foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"

red = "#f7768e"
yellow = "#e0af68"
blue = "#7aa2f7"
magenta = "#ad8ee6"

bright_yellow = "#ff9e64"
bright_magenta = "#bb9af7"
"##;

    #[test]
    fn a_full_theme_gives_the_role_table_of_the_tokens_document() {
        let roles =
            roles_from_colors(TOKYO_NIGHT).unwrap_or_else(|| panic!("a full theme has roles"));
        assert_eq!(
            roles,
            ThemeRoles {
                mode: "dark".into(),
                ground: "#0e0e14".into(),
                panel: "#13141c".into(),
                working: "#1a1b26".into(),
                hairline: "#24283b".into(),
                selection: "#292e42".into(),
                border: "#414868".into(),
                text: "#a9b1d6".into(),
                text_dim: "#565f89".into(),
                text_bright: "#c0caf5".into(),
                accent: "#7aa2f7".into(),
                on_accent: "#0e0e14".into(),
                tab_from: "#bb9af7".into(),
                tab_to: "#ff9e64".into(),
                control: None,
                accent_hover: None,
                built_in: false,
            }
        );
    }

    /// What `omarchy-theme-colors-from-alacritty` writes for a theme with no
    /// selection colour of its own: the selection is the foreground.
    const FROM_TERMINAL: &str = r##"accent = "#83a598"
selection = "#d4be98"

background = "#282828"
foreground = "#d4be98"

color0 = "#282828"
color3 = "#d8a657"
color4 = "#83a598"
color5 = "#d3869b"
color11 = "#e9b143"
color13 = "#e396a9"
color15 = "#ddc7a1"
"##;

    #[test]
    fn a_partial_theme_derives_the_missing_roles_from_the_ones_it_has() {
        let roles =
            roles_from_colors(FROM_TERMINAL).unwrap_or_else(|| panic!("a partial theme has roles"));
        assert_eq!(roles.mode, "dark");
        assert_eq!(roles.working, "#282828");
        assert_eq!(roles.text, "#d4be98");
        assert_eq!(roles.accent, "#83a598");
        assert_eq!(roles.text_bright, "#ddc7a1");
        assert_eq!(roles.tab_from, "#e396a9");
        assert_eq!(roles.tab_to, "#e9b143");
        // Derived grounds step down from the background; nothing is left at
        // a default from some other palette.
        let lum = |value: &str| luminance(rgb(value).unwrap_or_default());
        assert!(lum(&roles.ground) < lum(&roles.panel));
        assert!(lum(&roles.panel) < lum(&roles.working));
        assert!(lum(&roles.working) < lum(&roles.hairline));
        assert!(lum(&roles.hairline) < lum(&roles.border));
        assert!(lum(&roles.text_dim) < lum(&roles.text));
    }

    #[test]
    fn a_partial_theme_stays_readable() {
        let roles =
            roles_from_colors(FROM_TERMINAL).unwrap_or_else(|| panic!("a partial theme has roles"));
        let pair = |one: &str, other: &str| {
            contrast(rgb(one).unwrap_or_default(), rgb(other).unwrap_or_default())
        };
        // The file's selection is the text colour; a selected row must still
        // show its text.
        assert_ne!(roles.selection, roles.text);
        assert!(pair(&roles.selection, &roles.text) >= 4.5);
        for ground in [&roles.ground, &roles.panel, &roles.working] {
            assert!(pair(ground, &roles.text) >= 4.5);
            assert!(pair(ground, &roles.text_dim) >= 2.5);
        }
    }

    #[test]
    fn text_on_the_accent_can_be_read_whatever_the_accent_is() {
        let pair = |one: &str, other: &str| {
            contrast(rgb(one).unwrap_or_default(), rgb(other).unwrap_or_default())
        };
        // No accent given: the accent is the text colour itself, and the old
        // fixed near-white label on it could not be seen at all.
        let same = roles_from_colors("background = \"#101010\"\nforeground = \"#f0f0f0\"\n")
            .unwrap_or_else(|| panic!("two colours are enough"));
        assert_eq!(same.accent, "#f0f0f0");
        assert!(pair(&same.on_accent, &same.accent) >= 4.5);
        for theme in [
            TOKYO_NIGHT,
            FROM_TERMINAL,
            // A mid accent on which neither of the theme's own ends is enough.
            "background = \"#777777\"\nforeground = \"#888888\"\naccent = \"#808080\"\n",
            // A light theme with a pale accent.
            "background = \"#eff1f5\"\nforeground = \"#4c4f69\"\naccent = \"#f9e2af\"\n",
            "background = \"#0e1016\"\nforeground = \"#eef0f6\"\naccent = \"#b9f24b\"\n",
        ] {
            let roles = roles_from_colors(theme).unwrap_or_else(|| panic!("has roles"));
            assert!(
                pair(&roles.on_accent, &roles.accent) >= 4.5,
                "{} on {}",
                roles.on_accent,
                roles.accent
            );
        }
    }

    #[test]
    fn a_light_partial_theme_is_recognised_without_a_mode() {
        let roles = roles_from_colors("background = \"#eff1f5\"\nforeground = \"#4c4f69\"\n")
            .unwrap_or_else(|| panic!("two colours are enough"));
        assert_eq!(roles.mode, "light");
        assert_eq!(roles.accent, "#4c4f69");
        let lum = |value: &str| luminance(rgb(value).unwrap_or_default());
        assert!(lum(&roles.ground) < lum(&roles.working));
        assert!(lum(&roles.text_dim) > lum(&roles.text));
    }

    #[test]
    fn a_file_without_a_background_and_a_foreground_gives_no_roles() {
        assert_eq!(roles_from_colors(""), None);
        assert_eq!(
            roles_from_colors("accent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\n"),
            None
        );
        assert_eq!(
            roles_from_colors("background = \"#1a1b26\"\nforeground = \"blue\"\n"),
            None
        );
    }

    #[test]
    fn values_that_are_not_colours_never_reach_the_interface() {
        let text = "background = \"#1a1b26\"\nforeground = \"#a9b1d6\"\n\
                    accent = \"red; } body { display: none\"\nmuted = \"#12345\"\n[table]\nnot a line\n";
        let roles =
            roles_from_colors(text).unwrap_or_else(|| panic!("the two good colours are enough"));
        assert_eq!(roles.accent, "#a9b1d6");
        for value in [
            &roles.accent,
            &roles.border,
            &roles.selection,
            &roles.ground,
        ] {
            assert!(rgb(value).is_some(), "{value} is not #rrggbb");
        }
    }

    #[test]
    fn quoting_case_and_comments_are_tolerated() {
        let roles = roles_from_colors("background = '#1A1B26' # ground\n  foreground=#A9B1D6\n")
            .unwrap_or_else(|| panic!("both colours parse"));
        assert_eq!(
            (roles.working.as_str(), roles.text.as_str()),
            ("#1a1b26", "#a9b1d6")
        );
    }

    #[test]
    fn a_missing_theme_file_gives_no_roles() {
        let folder = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(roles_from_file(&folder.path().join("colors.toml")), None);
        assert_eq!(roles_from_file(folder.path()), None);
    }

    #[test]
    fn a_theme_file_is_read_from_disk() {
        let folder = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = folder.path().join("colors.toml");
        std::fs::write(&file, TOKYO_NIGHT).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            roles_from_file(&file).map(|roles| roles.accent),
            Some("#7aa2f7".into())
        );
    }

    fn file(text: &str) -> ThemeFile {
        crate::theme_file::parse(text)
    }

    fn no_font() -> Option<String> {
        None
    }

    #[test]
    fn with_no_theme_file_the_omarchy_theme_is_shown_as_it_is() {
        let theme = layered(Some(TOKYO_NIGHT), None, || {
            Some("JetBrainsMono Nerd Font".into())
        });
        assert_eq!(theme.roles, roles_from_colors(TOKYO_NIGHT));
        assert_eq!(theme.font.as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(theme.problems, []);
        assert_eq!(layered(None, None, no_font).roles, None);
    }

    #[test]
    fn a_file_overriding_one_colour_changes_only_that_role() {
        let omarchy = roles_from_colors(TOKYO_NIGHT).unwrap();
        let theme = layered(
            Some(TOKYO_NIGHT),
            Some(&file("muted = \"#ff00ff\"\n")),
            no_font,
        );
        assert_eq!(
            theme.roles,
            Some(ThemeRoles {
                border: "#ff00ff".into(),
                ..omarchy
            })
        );
    }
    /// The built-in look as roles: the stylesheet's fallbacks in
    /// `src/styles/tokens.css`.
    fn built_in() -> ThemeRoles {
        ThemeRoles {
            mode: "dark".into(),
            ground: "#000000".into(),
            panel: "#0f0f0f".into(),
            working: "#1e1e1e".into(),
            hairline: "#323232".into(),
            selection: "#314b67".into(),
            border: "#646464".into(),
            text: "#e6e6e6".into(),
            text_dim: "#9a9a9a".into(),
            text_bright: "#ffffff".into(),
            accent: "#1373eb".into(),
            on_accent: "#f0f0f0".into(),
            tab_from: "#1373eb".into(),
            tab_to: "#1373eb".into(),
            control: Some("#2b2b2b".into()),
            accent_hover: Some("#4a82ed".into()),
            built_in: true,
        }
    }

    /// Everything but the on-accent colour and the hover derived from it,
    /// which follow a changed accent.
    fn apart_from_the_accents_label(roles: &ThemeRoles) -> ThemeRoles {
        ThemeRoles {
            on_accent: String::new(),
            accent_hover: None,
            ..roles.clone()
        }
    }

    /// What wbx-45.6's review asked for: one key in the file, one role
    /// changed, over full, partial, light and built-in bases.
    #[test]
    fn an_override_changes_only_the_role_it_names_over_any_base() {
        let partial = "background = \"#282828\"\nforeground = \"#d4be98\"\n";
        let light = "background = \"#eff1f5\"\nforeground = \"#4c4f69\"\n";
        let base = |omarchy: Option<&str>| -> ThemeRoles {
            omarchy.and_then(roles_from_colors).unwrap_or_else(built_in)
        };
        for omarchy in [Some(TOKYO_NIGHT), Some(partial), Some(light), None] {
            let before = base(omarchy);
            let dark = before.mode == "dark";
            for (key, value, role) in [
                (
                    "foreground",
                    if dark { "#ebdbb2" } else { "#202020" },
                    "text",
                ),
                ("muted", "#ff00ff", "border"),
                (
                    "background",
                    if dark { "#202020" } else { "#f8f8f8" },
                    "working",
                ),
                ("dark_foreground", "#777777", "text_dim"),
                (
                    "darker_background",
                    if dark { "#050505" } else { "#d0d0d0" },
                    "ground",
                ),
            ] {
                let theme = layered(
                    omarchy,
                    Some(&file(&format!("{key} = \"{value}\"\n"))),
                    no_font,
                );
                let mut expected = before.clone();
                match role {
                    "text" => expected.text = value.into(),
                    "border" => expected.border = value.into(),
                    "working" => expected.working = value.into(),
                    "text_dim" => expected.text_dim = value.into(),
                    _ => expected.ground = value.into(),
                }
                let got = theme.roles.unwrap();
                if role == "ground" {
                    // The label on the accent is chosen from the ground; it
                    // may change, nothing else may.
                    assert_eq!(
                        apart_from_the_accents_label(&got),
                        apart_from_the_accents_label(&expected),
                        "{key} over {omarchy:?}"
                    );
                } else {
                    assert_eq!(got, expected, "{key} over {omarchy:?}");
                }
            }
        }
    }

    #[test]
    fn over_the_built_in_look_the_stylesheets_own_fallbacks_stand() {
        // Only `muted`: the control fill, the accent's label and its hover
        // stay the built-in look's, not values derived from the palette.
        let theme = layered(None, Some(&file("muted = \"#ff00ff\"\n")), no_font);
        assert_eq!(
            theme.roles,
            Some(ThemeRoles {
                border: "#ff00ff".into(),
                ..built_in()
            })
        );
        // `lighter_background` is both the hairline and the control fill.
        let theme = layered(
            None,
            Some(&file("lighter_background = \"#3a3a3a\"\n")),
            no_font,
        );
        let roles = theme.roles.unwrap();
        assert_eq!((roles.hairline.as_str(), roles.control), ("#3a3a3a", None));
    }

    #[test]
    fn follow_none_lays_the_file_over_the_built_in_look_and_not_over_omarchy() {
        let asked = std::cell::Cell::new(false);
        let theme = layered(
            Some(TOKYO_NIGHT),
            Some(&file("follow = \"none\"\naccent = \"#ff8800\"\n")),
            || {
                asked.set(true);
                Some("JetBrainsMono Nerd Font".into())
            },
        );
        assert_eq!(
            theme.roles,
            Some(ThemeRoles {
                accent: "#ff8800".into(),
                // The built-in tab line is the accent's colour.
                tab_from: "#ff8800".into(),
                tab_to: "#ff8800".into(),
                // A new accent gets a label chosen for it, and the window
                // derives its hover.
                on_accent: "#000000".into(),
                accent_hover: None,
                ..built_in()
            })
        );
        // Not following Omarchy: the face is the built-in one too.
        assert_eq!(theme.font, None);
        assert!(!asked.get());
    }

    #[test]
    fn follow_none_with_no_colours_is_exactly_the_built_in_look() {
        let theme = layered(
            Some(TOKYO_NIGHT),
            Some(&file("follow = \"none\"\n")),
            no_font,
        );
        // No roles: the stylesheet's fallbacks, which are the built-in look.
        assert_eq!(theme.roles, None);
    }

    #[test]
    fn without_an_omarchy_theme_the_file_is_laid_over_the_built_in_look() {
        let theme = layered(None, Some(&file("selection = \"#445566\"\n")), no_font);
        assert_eq!(
            theme.roles,
            Some(ThemeRoles {
                selection: "#445566".into(),
                ..built_in()
            })
        );
        // An Omarchy file that is not a usable palette is not mixed in.
        let theme = layered(
            Some("accent = \"#00ff00\"\n"),
            Some(&file("selection = \"#445566\"\n")),
            no_font,
        );
        assert_eq!(
            theme.roles.map(|roles| roles.accent),
            Some("#1373eb".into())
        );
    }

    #[test]
    fn whatever_accent_the_file_gives_its_label_can_be_read() {
        let pair = |one: &str, other: &str| {
            contrast(rgb(one).unwrap_or_default(), rgb(other).unwrap_or_default())
        };
        for accent in [
            "#a9b1d6", "#808080", "#ffff00", "#000000", "#0e0e14", "#ffffff", "#7f7f7f",
        ] {
            let text = format!("accent = \"{accent}\"\n");
            for omarchy in [Some(TOKYO_NIGHT), None] {
                let roles = layered(omarchy, Some(&file(&text)), no_font).roles.unwrap();
                assert_eq!(roles.accent, accent);
                assert!(
                    pair(&roles.on_accent, &roles.accent) >= 4.5,
                    "{} on {accent}",
                    roles.on_accent
                );
            }
        }
    }

    #[test]
    fn the_files_faces_and_sizes_are_passed_on() {
        let text =
            "font = \"Inter\"\nfont_size = 13\nmono_font = \"Iosevka\"\nmono_font_size = 11.5\n";
        let theme = layered(Some(TOKYO_NIGHT), Some(&file(text)), || {
            Some("JetBrainsMono Nerd Font".into())
        });
        assert_eq!(theme.font.as_deref(), Some("Inter"));
        assert_eq!(theme.interface_font.as_deref(), Some("Inter"));
        assert_eq!(theme.interface_font_size, Some(13.0));
        assert_eq!(theme.mono_font.as_deref(), Some("Iosevka"));
        assert_eq!(theme.mono_font_size, Some(11.5));
        // Fonts alone leave the colours on the Omarchy theme.
        assert_eq!(theme.roles, roles_from_colors(TOKYO_NIGHT));
        // Without them the system face stays upstream's, and the shell's own.
        let theme = layered(Some(TOKYO_NIGHT), Some(&file("")), || {
            Some("JetBrainsMono Nerd Font".into())
        });
        assert_eq!(theme.font.as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(theme.interface_font, None);
        assert_eq!(theme.interface_font_size, None);
    }

    #[test]
    fn the_files_mistakes_are_passed_on_and_the_rest_applies() {
        let theme = layered(
            Some(TOKYO_NIGHT),
            Some(&file("accent = \"#ff8800\"\nmuted = \"grey\"\n")),
            no_font,
        );
        assert_eq!(
            theme.roles.as_ref().map(|roles| roles.accent.as_str()),
            Some("#ff8800")
        );
        assert_eq!(
            theme.roles.map(|roles| roles.border),
            Some("#414868".into())
        );
        assert_eq!(
            theme
                .problems
                .iter()
                .map(|problem| problem.line)
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn the_theme_waveform_comes_from_the_omarchy_palette_alone() {
        let theme = layered(Some(TOKYO_NIGHT), None, no_font);
        let wave = theme.wave.expect("Tokyo Night has blue, magenta and bright_foreground");
        assert_eq!(
            (wave.low.as_str(), wave.mid.as_str(), wave.high.as_str()),
            ("#7aa2f7", "#ad8ee6", "#c0caf5")
        );
        // The theme file's colours are the window's, not the waveform's.
        let over = file("bright_foreground = \"#ff0000\"\nblue = \"#00ff00\"\nmagenta = \"#0000ff\"\n");
        assert_eq!(
            layered(Some(TOKYO_NIGHT), Some(&over), no_font).wave,
            Some(wave.clone())
        );
        // Only the numbered colours: they stand in, as they do for the roles.
        let numbered = "background = \"#101010\"\nforeground = \"#e0e0e0\"\ncolor4 = \"#112233\"\ncolor5 = \"#445566\"\ncolor15 = \"#778899\"\n";
        let wave = layered(Some(numbered), None, no_font).wave.unwrap();
        assert_eq!(
            (wave.low.as_str(), wave.mid.as_str(), wave.high.as_str()),
            ("#112233", "#445566", "#778899")
        );
        // Not following, no Omarchy theme, or an unusable one: none, and the
        // window falls back to its own.
        assert_eq!(layered(Some(TOKYO_NIGHT), Some(&file("follow = \"none\"")), no_font).wave, None);
        assert_eq!(layered(None, None, no_font).wave, None);
        assert_eq!(layered(Some("blue = \"#7aa2f7\""), None, no_font).wave, None);
    }

    #[test]
    fn the_files_waveform_choice_is_passed_on() {
        assert_eq!(
            layered(Some(TOKYO_NIGHT), Some(&file("waveform = \"rgb\"")), no_font)
                .waveform
                .as_deref(),
            Some("rgb")
        );
        assert_eq!(layered(Some(TOKYO_NIGHT), None, no_font).waveform, None);
    }

    #[test]
    fn a_font_family_is_the_first_alias_and_safe_to_quote() {
        assert_eq!(
            css_family("JetBrainsMono Nerd Font,JetBrainsMono NF\n"),
            Some("JetBrainsMono Nerd Font".into())
        );
        assert_eq!(css_family("Evil\"; } body { display: none"), None);
        assert_eq!(css_family("It's Mono"), None);
        assert_eq!(css_family(""), None);
    }
}
