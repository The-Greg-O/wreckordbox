//! The window's own theme file: `~/.config/wreckordbox/theme.toml`.
//!
//! It is one more layer over the Omarchy theme (`omarchy_theme.rs`), not a
//! second mechanism: the file's colour keys have the same names as an Omarchy
//! theme's `colors.toml` and are laid over that palette (or over the built-in
//! one, with `follow = "none"`) key by key, before the same roles are derived
//! from the result. Fonts and their base sizes are the file's alone.
//!
//! A mistake in the file never breaks the window: the bad key, or the line
//! that does not parse, is left out, the rest applies, and each mistake is
//! reported with its line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Points the app at a theme file of the caller's choosing instead of the
/// DJ's own. For tests and screenshots.
pub const THEME_FILE_ENV: &str = "WRECKORDBOX_THEME_FILE";

/// What the file says the look starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Follow {
    /// The active Omarchy theme, as when there is no file.
    #[default]
    Omarchy,
    /// The built-in look, upstream's colours and face.
    None,
}

/// A mistake in the file, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeProblem {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

/// What a theme file asks for, with its mistakes left out and listed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeFile {
    pub follow: Follow,
    /// Colour keys (Omarchy's names), each `#rrggbb` in lower case, and
    /// `mode`.
    pub colours: BTreeMap<String, String>,
    /// The interface's face.
    pub font: Option<String>,
    /// The interface's base text size, in CSS pixels at 100% scale.
    pub font_size: Option<f64>,
    /// The face for text whose digits line up: BPM, key, times, dates.
    pub mono_font: Option<String>,
    pub mono_font_size: Option<f64>,
    pub problems: Vec<ThemeProblem>,
}

/// The colour keys of an Omarchy `colors.toml`. All are accepted, so a
/// pasted palette is not reported; the roles read some of them directly and
/// the rest stand in where those are missing.
#[rustfmt::skip]
const COLOUR_KEYS: &[&str] = &[
    "accent", "selection", "muted",
    "background", "dark_background", "darker_background", "lighter_background",
    "foreground", "dark_foreground", "light_foreground", "bright_foreground",
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "orange", "brown",
    "bright_red", "bright_green", "bright_yellow", "bright_blue", "bright_magenta", "bright_cyan",
    "color0", "color1", "color2", "color3", "color4", "color5", "color6", "color7",
    "color8", "color9", "color10", "color11", "color12", "color13", "color14", "color15",
];

/// Every key a theme file may set (`docs/user/theme.example.toml` documents
/// exactly these).
#[must_use]
pub fn accepted_keys() -> Vec<&'static str> {
    ["follow", "mode", "font", "font_size", "mono_font", "mono_font_size"]
        .into_iter()
        .chain(COLOUR_KEYS.iter().copied())
        .collect()
}

/// Settings an Omarchy palette may carry for other programs. Passed over in
/// silence: their values are often not `#rrggbb`.
fn other_programs_key(key: &str) -> bool {
    key.starts_with("hyprland_") || matches!(key, "active_border_color" | "active_tab_background")
}

/// Sizes outside this are a mistake, not a preference (CSS pixels at 100%).
const SIZES: std::ops::RangeInclusive<f64> = 8.0..=24.0;

/// A line that does not parse is removed and the rest tried again, at most
/// this many times.
const MOST_BROKEN_LINES: usize = 64;

/// A theme file is a few kilobytes; anything large is not one.
const LARGEST: u64 = 256 * 1024;

type Table = BTreeMap<String, toml::Spanned<toml::Value>>;

/// The 1-based line `offset` (a byte position in `text`) is on.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset.min(text.len()))
        .map_or(0, |before| before.matches('\n').count())
        + 1
}

/// `text` with line `line` (1-based) blanked: its characters replaced by as
/// many bytes of spaces, so every other byte stays where it was and a span
/// in the result is a span in `text`. `None` if it was already blank.
fn without_line(text: &str, line: usize) -> Option<String> {
    let mut changed = false;
    let kept: Vec<String> = text
        .split('\n')
        .enumerate()
        .map(|(at, content)| {
            if at + 1 == line && !content.trim().is_empty() {
                changed = true;
                " ".repeat(content.len())
            } else {
                content.to_owned()
            }
        })
        .collect();
    changed.then(|| kept.join("\n"))
}

/// The table in `text`, leaving out each line that keeps it from parsing.
fn table(text: &str, problems: &mut Vec<ThemeProblem>) -> Table {
    let mut text = text.to_owned();
    for _ in 0..MOST_BROKEN_LINES {
        let error = match toml::from_str::<Table>(&text) {
            Ok(table) => return table,
            Err(error) => error,
        };
        let line = error.span().map_or(1, |span| line_of(&text, span.start));
        let reason = error
            .message()
            .lines()
            .next()
            .unwrap_or("not valid TOML")
            .trim();
        problems.push(ThemeProblem {
            line,
            message: format!("this line is not valid TOML ({reason}); it is left out"),
        });
        match without_line(&text, line) {
            Some(shorter) => text = shorter,
            None => break,
        }
    }
    Table::new()
}

/// A family name that can be written into a CSS `font-family` as it is.
fn family(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.contains(','))
        .then(|| crate::omarchy_theme::css_family(value))
        .flatten()
}

fn size(value: &toml::Value) -> Option<f64> {
    #[allow(clippy::cast_precision_loss, reason = "a font size is small")]
    let number = match value {
        toml::Value::Integer(whole) => *whole as f64,
        toml::Value::Float(number) => *number,
        _ => return None,
    };
    SIZES.contains(&number).then_some(number)
}

fn quoted(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => format!("\"{text}\""),
        toml::Value::Table(_) => "a table".to_owned(),
        toml::Value::Array(_) => "a list".to_owned(),
        other => other.to_string(),
    }
}

/// Reads `text` as a theme file. Never fails: see the module comment.
#[must_use]
pub fn parse(text: &str) -> ThemeFile {
    let mut file = ThemeFile::default();
    let table = table(text, &mut file.problems);
    for (key, spanned) in table {
        let line = line_of(text, spanned.span().start);
        let value = spanned.get_ref();
        let as_text = value.as_str();
        let mistake = match key.as_str() {
            "follow" => match as_text {
                Some("omarchy") => {
                    file.follow = Follow::Omarchy;
                    None
                }
                Some("none") => {
                    file.follow = Follow::None;
                    None
                }
                _ => Some(format!(
                    "follow is {}; it must be \"omarchy\" or \"none\"",
                    quoted(value)
                )),
            },
            "mode" => match as_text {
                Some(mode @ ("dark" | "light")) => {
                    file.colours.insert(key.clone(), mode.to_owned());
                    None
                }
                _ => Some(format!(
                    "mode is {}; it must be \"dark\" or \"light\"",
                    quoted(value)
                )),
            },
            "font" | "mono_font" => match as_text.and_then(family) {
                Some(name) => {
                    *if key == "font" {
                        &mut file.font
                    } else {
                        &mut file.mono_font
                    } = Some(name);
                    None
                }
                None => Some(format!(
                    "{key} is {}; it must be one font family name, in quotes, with no commas",
                    quoted(value)
                )),
            },
            "font_size" | "mono_font_size" => match size(value) {
                Some(px) => {
                    *if key == "font_size" {
                        &mut file.font_size
                    } else {
                        &mut file.mono_font_size
                    } = Some(px);
                    None
                }
                None => Some(format!(
                    "{key} is {}; it must be a number of pixels from {} to {}",
                    quoted(value),
                    SIZES.start(),
                    SIZES.end()
                )),
            },
            colour if COLOUR_KEYS.contains(&colour) => {
                let hex = as_text
                    .filter(|hex| crate::omarchy_theme::is_colour(hex))
                    .map(str::to_ascii_lowercase);
                match hex {
                    Some(hex) => {
                        file.colours.insert(key.clone(), hex);
                        None
                    }
                    None => Some(format!(
                        "{key} is {}, not a colour; write it as \"#rrggbb\"",
                        quoted(value)
                    )),
                }
            }
            other if other_programs_key(other) => None,
            other => Some(format!("unknown key \"{other}\"; it is ignored")),
        };
        if let Some(message) = mistake {
            file.problems.push(ThemeProblem { line, message });
        }
    }
    file.problems.sort_by_key(|problem| problem.line);
    file
}

/// Why a small text file could not be had.
#[derive(Debug)]
pub(crate) enum Unread {
    /// Nothing at the path (or a parent is not a folder).
    Missing,
    /// Something is there and could not be read; why, for the DJ.
    Failed(String),
}

/// At most `limit` bytes of the regular file at `path`, read through one
/// handle: opened without waiting (a FIFO put in its place is refused, not
/// waited on), checked on that handle, and read no further than `limit + 1`
/// bytes whatever it grows to meanwhile.
pub(crate) fn read_small(path: &Path, limit: u64) -> Result<Vec<u8>, Unread> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => Unread::Missing,
        _ => Unread::Failed(format!("the file could not be read: {error}")),
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| Unread::Failed(format!("the file could not be read: {error}")))?;
    if !metadata.is_file() {
        return Err(Unread::Failed(
            "this is not a regular file, so it is not read".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Unread::Failed(format!("the file could not be read: {error}")))?;
    if bytes.len() as u64 > limit {
        return Err(Unread::Failed(format!(
            "the file is larger than {} KB, so it is not read",
            limit / 1024
        )));
    }
    Ok(bytes)
}

/// The theme file at `path`; `None` when there is none. A file that is there
/// and cannot be read (no permission, not a regular file, too large, not
/// UTF-8) is a file with one problem, on line 1.
#[must_use]
pub fn read(path: &Path) -> Option<ThemeFile> {
    let unreadable = |message: String| ThemeFile {
        problems: vec![ThemeProblem { line: 1, message }],
        ..ThemeFile::default()
    };
    Some(match read_small(path, LARGEST) {
        Err(Unread::Missing) => return None,
        Err(Unread::Failed(message)) => unreadable(message),
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => parse(&text),
            Err(_) => unreadable("the file is not UTF-8 text, so it is not read".to_owned()),
        },
    })
}

/// Where the DJ's theme file is: the override where it is set, else
/// `wreckordbox/theme.toml` in the configuration folder (`XDG_CONFIG_HOME`,
/// else `~/.config`). While only the Omarchy colour override
/// (`WRECKORDBOX_THEME_COLORS`) is set there is none, so a screenshot or test
/// run never picks up the DJ's own file.
#[must_use]
pub fn theme_file() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(path) = set(THEME_FILE_ENV) {
        return Some(PathBuf::from(path));
    }
    if set(crate::omarchy_theme::COLORS_FILE_ENV).is_some() {
        return None;
    }
    Some(dirs::config_dir()?.join("wreckordbox/theme.toml"))
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

    fn lines(file: &ThemeFile) -> Vec<usize> {
        file.problems.iter().map(|problem| problem.line).collect()
    }

    #[test]
    fn a_pasted_omarchy_palette_is_taken_whole_without_a_problem() {
        let pasted = std::fs::read_to_string("/usr/share/omarchy/themes/tokyo-night/colors.toml")
            .unwrap_or_else(|_| {
                "mode = \"dark\"\naccent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\nbright_magenta = \"#bb9af7\"\ncolor4 = \"#7aa2f7\"\nhyprland_active_border = \"rgb(7aa2f7)\"\n".into()
            });
        let file = parse(&pasted);
        assert_eq!(file.problems, []);
        assert_eq!(file.follow, Follow::Omarchy);
        assert_eq!(
            file.colours.get("accent").map(String::as_str),
            Some("#7aa2f7")
        );
        assert_eq!(file.colours.get("mode").map(String::as_str), Some("dark"));
    }

    #[test]
    fn colours_are_kept_in_lower_case() {
        let file = parse("accent = \"#7AA2F7\"\n");
        assert_eq!(
            file.colours.get("accent").map(String::as_str),
            Some("#7aa2f7")
        );
    }

    #[test]
    fn an_invalid_colour_is_reported_with_its_line_and_left_out() {
        let file = parse("# my theme\n\naccent = \"#7aa2f7\"\nmuted = \"#12345\"\nselection = 4\n");
        assert_eq!(lines(&file), [4, 5]);
        assert!(
            file.problems[0].message.contains("muted"),
            "{}",
            file.problems[0].message
        );
        assert!(file.colours.contains_key("accent"));
        assert!(!file.colours.contains_key("muted"));
        assert!(!file.colours.contains_key("selection"));
    }

    #[test]
    fn an_unknown_key_is_reported_with_its_line() {
        let file = parse("acent = \"#7aa2f7\"\n[colors]\naccent = \"#7aa2f7\"\n");
        assert_eq!(lines(&file), [1, 2]);
        assert!(file.problems[0].message.contains("acent"));
        assert!(file.colours.is_empty());
    }

    #[test]
    fn a_line_that_is_not_toml_is_reported_and_the_rest_applies() {
        let file = parse("accent = \"#7aa2f7\"\nmuted = \"#414868\nforeground = \"#a9b1d6\"\nthis is not toml\nbackground = \"#1a1b26\"\n");
        assert_eq!(lines(&file), [2, 4]);
        assert_eq!(
            file.colours.get("accent").map(String::as_str),
            Some("#7aa2f7")
        );
        assert_eq!(
            file.colours.get("foreground").map(String::as_str),
            Some("#a9b1d6")
        );
        assert_eq!(
            file.colours.get("background").map(String::as_str),
            Some("#1a1b26")
        );
    }

    #[test]
    fn a_mistake_after_a_line_that_is_not_toml_keeps_its_own_line() {
        let file = parse("this is not toml\nmuted = \"grey\"\nnot toml either\nacent = \"#ffffff\"\n");
        assert_eq!(lines(&file), [1, 2, 3, 4]);
        assert!(file.problems[1].message.contains("muted"), "{:?}", file.problems);
        assert!(file.problems[3].message.contains("acent"), "{:?}", file.problems);
    }

    #[test]
    fn a_key_given_twice_is_reported_and_the_first_stands() {
        let file = parse("accent = \"#7aa2f7\"\naccent = \"#ff0000\"\n");
        assert_eq!(lines(&file), [2]);
        assert_eq!(
            file.colours.get("accent").map(String::as_str),
            Some("#7aa2f7")
        );
    }

    #[test]
    fn follow_takes_omarchy_or_none_and_nothing_else() {
        assert_eq!(parse("follow = \"none\"").follow, Follow::None);
        assert_eq!(parse("follow = \"omarchy\"").follow, Follow::Omarchy);
        let wrong = parse("follow = \"gnome\"");
        assert_eq!(wrong.follow, Follow::Omarchy);
        assert_eq!(lines(&wrong), [1]);
        assert_eq!(lines(&parse("mode = \"sepia\"")), [1]);
    }

    #[test]
    fn faces_and_sizes_are_checked() {
        let good = parse("font = \"Inter\"\nfont_size = 13\nmono_font = \"JetBrainsMono Nerd Font\"\nmono_font_size = 11.5\n");
        assert_eq!(good.problems, []);
        assert_eq!(good.font.as_deref(), Some("Inter"));
        assert_eq!(good.font_size, Some(13.0));
        assert_eq!(good.mono_font.as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(good.mono_font_size, Some(11.5));
        let bad = parse("font = \"Inter, sans\"\nfont_size = 200\nmono_font = \"Evil\\\"; } body {\"\nmono_font_size = \"12px\"\nfont_size = 3\n");
        assert_eq!(lines(&bad), [1, 2, 3, 4, 5]);
        assert_eq!(
            (bad.font, bad.font_size, bad.mono_font, bad.mono_font_size),
            (None, None, None, None)
        );
    }

    /// Takes read permission away from `path` for the length of `check`,
    /// unless this runs as a user permissions do not stop (root).
    #[cfg(unix)]
    fn without_permission(path: &Path, mode: u32, check: impl FnOnce()) {
        use std::os::unix::fs::PermissionsExt;
        let before = std::fs::metadata(path).unwrap().permissions();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        let stopped = std::fs::read_dir(path).is_err() || std::fs::read(path).is_err();
        if stopped {
            check();
        }
        std::fs::set_permissions(path, before).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_may_not_be_read_is_reported_not_taken_for_no_file() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("theme.toml");
        std::fs::write(&file, "accent = \"#ff8800\"\n").unwrap();
        without_permission(&file, 0o000, || {
            let read = read(&file).expect("a file that is there is not no file");
            assert_eq!(lines(&read), [1]);
            assert!(read.problems[0].message.contains("ermission"), "{:?}", read.problems);
        });
        // Its folder may not be searched: the file cannot even be looked at.
        without_permission(folder.path(), 0o000, || {
            let read = read(&file).expect("a folder that cannot be searched is not no file");
            assert!(read.problems[0].message.contains("ermission"), "{:?}", read.problems);
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_is_reported_without_waiting_on_it() {
        let folder = tempfile::tempdir().unwrap();
        let fifo = folder.path().join("theme.toml");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(made.success());
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(read(&fifo));
        });
        let read = receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("reading a FIFO must not wait for a writer");
        let read = read.expect("a FIFO where the file should be is reported");
        assert!(read.problems[0].message.contains("not a regular file"), "{:?}", read.problems);
    }

    #[test]
    fn no_more_than_the_limit_is_read() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("theme.toml");
        let mut big = "accent = \"#ff8800\"\n".to_owned();
        big.push_str(&"# padding\n".repeat(40_000));
        std::fs::write(&file, big).unwrap();
        let read = read(&file).unwrap();
        assert_eq!(read.colours.len(), 0);
        assert!(read.problems[0].message.contains("larger than"), "{:?}", read.problems);
    }

    #[test]
    fn the_shipped_example_lists_exactly_the_accepted_keys_and_changes_nothing() {
        let example = include_str!("../../docs/user/theme.example.toml");
        // As shipped, all comment: nothing set, nothing reported.
        assert_eq!(parse(example), ThemeFile::default());
        // Each `# key = value` line, uncommented.
        let assignments: Vec<&str> = example
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| {
                line.split_once(" = ").is_some_and(|(key, _)| {
                    !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                })
            })
            .collect();
        let keys: Vec<&str> = assignments.iter().map(|line| line.split_once(" = ").unwrap().0).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "a key is documented twice");
        let mut accepted = accepted_keys();
        accepted.sort_unstable();
        assert_eq!(sorted, accepted);
        // Every documented value is one the parser takes.
        let uncommented = parse(&assignments.join("\n"));
        assert_eq!(uncommented.problems, []);
    }

    #[test]
    fn a_missing_file_is_no_file_and_an_unreadable_one_is_a_problem() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(read(&folder.path().join("theme.toml")), None);
        let file = folder.path().join("theme.toml");
        std::fs::write(&file, [0xff_u8, 0xfe, 0x00]).unwrap();
        assert_eq!(read(&file).map(|file| lines(&file)), Some(vec![1]));
    }
}
