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
}

/// What the interface is told: the roles, if a usable theme is present, and
/// the system monospace family, if the system has one to name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub roles: Option<ThemeRoles>,
    pub font: Option<String>,
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
    let colours = parse_colors(text);
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
    })
}

/// The roles in the colour file at `path`; `None` when it is missing,
/// unreadable or unusable.
#[must_use]
pub fn roles_from_file(path: &Path) -> Option<ThemeRoles> {
    // A palette is a few hundred bytes; anything large is not one.
    if std::fs::metadata(path).ok()?.len() > 256 * 1024 {
        return None;
    }
    roles_from_colors(&std::fs::read_to_string(path).ok()?)
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
fn css_family(answer: &str) -> Option<String> {
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
        }
    }

    /// The theme as these sources give it now.
    #[must_use]
    pub fn read(&self) -> Theme {
        Theme {
            roles: self.colors.as_deref().and_then(roles_from_file),
            font: system_monospace_font(),
        }
    }

    /// The folders whose entries change on a switch. They need not exist.
    fn folders(&self) -> Vec<PathBuf> {
        // The folder the colour file is in, for a file rewritten where it
        // is; and for an Omarchy theme also the folder holding that one,
        // where a switch removes the theme folder and puts another in.
        let theme = self.colors.as_deref().and_then(Path::parent);
        let holding = theme
            .filter(|_| self.theme_folder_is_replaced)
            .and_then(Path::parent);
        holding
            .into_iter()
            .chain(theme)
            .chain(self.font_folder.as_deref())
            .filter(|folder| folder.is_absolute())
            .map(Path::to_path_buf)
            .collect()
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
                    &sources.folders(),
                    || sources.read(),
                    |theme| {
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

    use super::Theme;

    /// A switch is several writes over a moment; read once they stop.
    const SETTLE: Duration = Duration::from_millis(250);

    /// One mask for every watch: a folder can be watched both for itself and
    /// as the nearest existing folder above another, and inotify keeps one
    /// mask per folder.
    fn mask() -> WatchMask {
        WatchMask::CREATE
            | WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE
            | WatchMask::DELETE_SELF
            | WatchMask::MOVE_SELF
    }

    /// What a watched folder is watched for.
    #[derive(Default)]
    struct Interest {
        /// Anything in it: it is one of the folders a switch writes into.
        everything: bool,
        /// Entries of these names: the next step down to a folder that does
        /// not exist yet.
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

        /// Watches `folder`, or while it does not exist the nearest folder
        /// above it that does, for the entry that leads down to it.
        fn reach(&mut self, folder: &Path) {
            // Each pass watches one level; a path is not deeper than this.
            for _ in 0..64 {
                if folder.is_dir() {
                    if let Some(interest) = self.interest(folder) {
                        interest.everything = true;
                        return;
                    }
                }
                let Some((above, name)) = nearest_existing(folder) else {
                    return;
                };
                let Some(interest) = self.interest(&above) else {
                    return;
                };
                if !interest.names.contains(&name) {
                    interest.names.push(name.clone());
                }
                // It may have appeared between the look and the watch; if
                // so go round again and step down into it.
                if !above.join(&name).is_dir() {
                    return;
                }
            }
        }

        /// Starts again from nothing: folders come and go, and a watch on a
        /// removed folder is dead.
        fn register(&mut self, folders: &[PathBuf]) {
            for (watch, _) in self.interests.drain() {
                let _ = self.inotify.watches().remove(watch);
            }
            for folder in folders {
                self.reach(folder);
            }
        }

        /// Whether `event` could mean a switch. An event for a watch that
        /// has since been dropped is not one.
        fn concerns(&self, event: &Event<&std::ffi::OsStr>) -> bool {
            let Some(interest) = self.interests.get(&event.wd) else {
                return false;
            };
            let gone = EventMask::DELETE_SELF | EventMask::MOVE_SELF | EventMask::IGNORED;
            interest.everything
                || event.mask.intersects(gone)
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

    /// Calls `tell` with the theme each time what `read` returns changes
    /// after something happened in `folders`. Returns only if inotify fails.
    pub(super) fn run(folders: &[PathBuf], read: impl Fn() -> Theme, mut tell: impl FnMut(&Theme)) {
        if folders.is_empty() {
            return;
        }
        let Ok(inotify) = Inotify::init() else { return };
        let mut watches = Watches {
            inotify,
            interests: HashMap::new(),
        };
        watches.register(folders);
        let mut shown = read();
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
            // Watch first, read second: a write that lands after the read is
            // then already watched for.
            watches.register(folders);
            let now = read();
            if now != shown {
                tell(&now);
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
