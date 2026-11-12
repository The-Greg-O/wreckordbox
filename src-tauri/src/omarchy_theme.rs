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
    Some(ThemeRoles {
        mode: if light { "light" } else { "dark" }.to_owned(),
        ground: hex(get(&["darker_background"]).unwrap_or_else(|| mix(background, black, darker))),
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
        text_bright: hex(get(&["bright_foreground", "color15"]).unwrap_or(foreground)),
        accent: hex(accent),
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

/// The theme as it stands now.
#[must_use]
pub fn current() -> Theme {
    Theme {
        roles: colors_file().and_then(|path| roles_from_file(&path)),
        font: system_monospace_font(),
    }
}

/// The theme, for a window that has just opened.
#[tauri::command]
pub fn theme_roles() -> Theme {
    current()
}

/// Tells every window when the theme changes, for as long as the app runs.
///
/// Waits on the folder a switch writes into (inotify, no polling) and reads
/// the theme again once the writes have settled. Does nothing where there is
/// no such folder: a machine without Omarchy has no switch to follow.
pub fn follow(app: &tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    watch::start(app.clone());
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

#[cfg(target_os = "linux")]
mod watch {
    use std::path::PathBuf;
    use std::time::Duration;

    use inotify::{Inotify, WatchMask};

    use super::{colors_file, current, CHANGED_EVENT, COLORS_FILE_ENV};

    /// A switch is several writes over a moment; read once they stop.
    const SETTLE: Duration = Duration::from_millis(250);

    /// The folders whose entries change on a theme or font switch.
    fn folders() -> Vec<PathBuf> {
        let mut folders = Vec::new();
        if let Some(colors) = colors_file() {
            let overridden = std::env::var_os(COLORS_FILE_ENV).is_some_and(|path| !path.is_empty());
            // The override is one file, rewritten in place. The active theme
            // is a folder that is replaced whole, so its parent is watched.
            let folder = if overridden {
                colors.parent()
            } else {
                colors.parent().and_then(|theme| theme.parent())
            };
            folders.extend(folder.map(PathBuf::from));
        }
        // `omarchy font set` rewrites fonts.conf here.
        folders.extend(dirs::config_dir().map(|config| config.join("fontconfig")));
        folders.retain(|folder| folder.is_dir());
        folders
    }

    pub(super) fn start(app: tauri::AppHandle) {
        let folders = folders();
        if folders.is_empty() {
            return;
        }
        let spawned = std::thread::Builder::new().name("theme-watch".into()).spawn(move || {
            let Ok(mut inotify) = Inotify::init() else { return };
            let mask = WatchMask::CREATE | WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE;
            let watched = folders.iter().filter(|folder| inotify.watches().add(folder, mask).is_ok()).count();
            if watched == 0 {
                return;
            }
            let mut shown = current();
            let mut buffer = [0_u8; 4096];
            // Blocks until something in a watched folder changes.
            while inotify.read_events_blocking(&mut buffer).is_ok() {
                loop {
                    std::thread::sleep(SETTLE);
                    let more = inotify.read_events(&mut buffer).is_ok_and(|events| events.count() > 0);
                    if !more {
                        break;
                    }
                }
                let now = current();
                if now != shown {
                    tracing::info!(themed = now.roles.is_some(), font = ?now.font, "theme changed");
                    let _ = tauri::Emitter::emit(&app, CHANGED_EVENT, &now);
                    shown = now;
                }
            }
        });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start the theme watcher");
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
