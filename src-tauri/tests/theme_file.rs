//! The window's own theme file reaches the windows: a change to a theme file
//! under a temporary folder, through the real inotify watcher, out as the
//! `theme:changed` event of a mock app, laid over an Omarchy-shaped theme in
//! the same temporary folder. Nothing here reads or touches the DJ's own
//! theme or theme file.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use rbxport_lib::omarchy_theme::{
    follow, follow_sources, Sources, Watch, CHANGED_EVENT, COLORS_FILE_ENV,
};
use rbxport_lib::theme_file::THEME_FILE_ENV;
use tauri::Listener;

fn palette(accent: &str) -> String {
    format!("background = \"#1a1b26\"\nforeground = \"#a9b1d6\"\naccent = \"{accent}\"\n")
}

/// Every theme the app announces.
fn announced(app: &tauri::App<tauri::test::MockRuntime>) -> Receiver<serde_json::Value> {
    let (send, receive) = channel();
    app.listen(CHANGED_EVENT, move |event| {
        let _ = send.send(serde_json::from_str(event.payload()).unwrap());
    });
    receive
}

fn started() {
    std::thread::sleep(Duration::from_millis(600));
}

fn next(themes: &Receiver<serde_json::Value>) -> serde_json::Value {
    themes
        .recv_timeout(Duration::from_secs(20))
        .expect("the app should announce the changed theme")
}

fn accent(theme: &serde_json::Value) -> Option<&str> {
    theme["roles"]["accent"].as_str()
}

/// An Omarchy-shaped state folder with a theme of `accent`, and a theme file
/// at `<root>/config/wreckordbox/theme.toml` that does not exist yet.
fn sources(root: &Path, accent: &str) -> Sources {
    let theme = root.join("state/current/theme");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::write(theme.join("colors.toml"), palette(accent)).unwrap();
    Sources {
        colors: Some(theme.join("colors.toml")),
        theme_folder_is_replaced: true,
        font_folder: None,
        theme_file: Some(root.join("config/wreckordbox/theme.toml")),
    }
}

#[test]
fn a_theme_file_whose_folder_did_not_exist_is_followed_once_made_then_saved_and_deleted() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();

    // Created, folder and all: one role changes, the others stay Omarchy's.
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "selection = \"#445566\"\n").unwrap();
    let theme = next(&themes);
    assert_eq!(theme["roles"]["selection"], "#445566");
    assert_eq!(accent(&theme), Some("#7aa2f7"));
    assert_eq!(theme["roles"]["working"], "#1a1b26");

    // Saved in place.
    std::fs::write(&file, "accent = \"#ff8800\"\n").unwrap();
    assert_eq!(accent(&next(&themes)), Some("#ff8800"));

    // Saved as editors do: a new file renamed over the old one.
    let swap = file.with_extension("toml.swp");
    std::fs::write(&swap, "accent = \"#00cc66\"\n").unwrap();
    std::fs::rename(&swap, &file).unwrap();
    assert_eq!(accent(&next(&themes)), Some("#00cc66"));

    // Deleted: back to the Omarchy theme.
    std::fs::remove_file(&file).unwrap();
    assert_eq!(accent(&next(&themes)), Some("#7aa2f7"));
}

#[test]
fn the_omarchy_theme_is_still_followed_under_a_file_and_not_under_follow_none() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "selection = \"#445566\"\n").unwrap();
    let current = root.path().join("state/current");
    let switch = |accent: &str| {
        std::fs::create_dir_all(current.join("next-theme")).unwrap();
        std::fs::write(current.join("next-theme/colors.toml"), palette(accent)).unwrap();
        std::fs::remove_dir_all(current.join("theme")).unwrap();
        std::fs::rename(current.join("next-theme"), current.join("theme")).unwrap();
    };
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();

    // An Omarchy switch under a file that overrides another role.
    switch("#7daea3");
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#7daea3"));
    assert_eq!(theme["roles"]["selection"], "#445566");

    // follow = "none": the built-in look with the file's key over it.
    std::fs::write(&file, "follow = \"none\"\nselection = \"#445566\"\n").unwrap();
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#1373eb"));
    assert_eq!(theme["roles"]["working"], "#1e1e1e");
    assert_eq!(theme["font"], serde_json::Value::Null);

    // Now an Omarchy switch changes nothing in the window.
    switch("#b9f24b");
    assert!(themes.recv_timeout(Duration::from_secs(2)).is_err());

    // Back to following: the switch that was ignored shows.
    std::fs::write(&file, "follow = \"omarchy\"\n").unwrap();
    assert_eq!(accent(&next(&themes)), Some("#b9f24b"));
}

#[test]
fn a_mistake_reaches_the_window_with_its_line_and_the_rest_applies() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();

    std::fs::write(
        &file,
        "accent = \"#ff8800\"\nacent = \"#00ff00\"\nmuted = \"grey\"\nnot toml at all\n",
    )
    .unwrap();
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#ff8800"));
    let lines: Vec<u64> = theme["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| problem["line"].as_u64().unwrap())
        .collect();
    assert_eq!(lines, [2, 3, 4]);

    // Fixed: the problems go.
    std::fs::write(&file, "accent = \"#ff8800\"\n").unwrap();
    assert_eq!(next(&themes)["problems"], serde_json::json!([]));
}

#[test]
fn the_override_variable_names_the_theme_file_the_app_follows() {
    // The one test here that goes through the environment, as the
    // screenshot script does.
    let root = tempfile::tempdir().unwrap();
    let colours = root.path().join("colors.toml");
    std::fs::write(&colours, palette("#7aa2f7")).unwrap();
    let file = root.path().join("not-yet/theme.toml");
    std::env::set_var(COLORS_FILE_ENV, &colours);
    std::env::set_var(THEME_FILE_ENV, &file);
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow(app.handle());
    started();

    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "font = \"DejaVu Sans\"\nfont_size = 14\n").unwrap();
    let theme = next(&themes);
    assert_eq!(theme["interfaceFont"], "DejaVu Sans");
    assert_eq!(theme["interfaceFontSize"], 14.0);
    assert_eq!(theme["font"], "DejaVu Sans");
    assert_eq!(accent(&theme), Some("#7aa2f7"));
}

#[test]
fn a_theme_file_linked_into_a_dotfiles_folder_follows_saves_of_its_target() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let link = sources.theme_file.clone().unwrap();
    let dotfiles = root.path().join("dotfiles/wreckordbox");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    let target = dotfiles.join("theme.toml");
    std::fs::write(&target, "accent = \"#ff8800\"\n").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();

    // Saved in place, in the dotfiles folder.
    std::fs::write(&target, "accent = \"#00cc66\"\n").unwrap();
    assert_eq!(accent(&next(&themes)), Some("#00cc66"));

    // Saved as editors do, a new file renamed over the target.
    let swap = dotfiles.join(".theme.toml.swp");
    std::fs::write(&swap, "accent = \"#cc00cc\"\n").unwrap();
    std::fs::rename(&swap, &target).unwrap();
    assert_eq!(accent(&next(&themes)), Some("#cc00cc"));

    // The link pointed at another file, and that file then saved.
    let other = root.path().join("dotfiles/other.toml");
    std::fs::write(&other, "accent = \"#123456\"\n").unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    assert_eq!(accent(&next(&themes)), Some("#123456"));
    std::fs::write(&other, "accent = \"#654321\"\n").unwrap();
    assert_eq!(accent(&next(&themes)), Some("#654321"));
}

/// Sets `path`'s permission bits.
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_file_that_may_not_be_read_is_reported_and_reloaded_when_it_may_again() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    let folder = file.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(&file, "accent = \"#ff8800\"\n").unwrap();
    chmod(&file, 0o000);
    if std::fs::read(&file).is_ok() {
        chmod(&file, 0o644);
        return; // Permissions do not stop this user (root): nothing to test.
    }
    chmod(&file, 0o644);
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();

    chmod(&file, 0o000);
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#7aa2f7"));
    assert!(
        theme["problems"][0]["message"].as_str().unwrap().contains("ermission"),
        "{theme}"
    );
    chmod(&file, 0o644);
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#ff8800"));
    assert_eq!(theme["problems"], serde_json::json!([]));

    // The folder itself closed and opened again.
    chmod(&folder, 0o000);
    let theme = next(&themes);
    assert_eq!(accent(&theme), Some("#7aa2f7"));
    chmod(&folder, 0o755);
    assert_eq!(accent(&next(&themes)), Some("#ff8800"));
}

#[test]
fn only_the_theme_file_is_watched_in_its_folder_and_omarchy_not_at_all_under_follow_none() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    let folder = file.parent().unwrap().to_path_buf();
    let entry = |name: &str| Some(std::ffi::OsString::from(name));
    // Following Omarchy: its folders whole, and the theme file by name.
    let following = sources.watches(true);
    assert!(following.contains(&Watch {
        folder: folder.clone(),
        entry: entry("theme.toml"),
    }));
    assert!(following.iter().any(|watch| watch.folder == root.path().join("state/current") && watch.entry.is_none()));
    // Under follow = "none": only the theme file.
    assert_eq!(
        sources.watches(false),
        [Watch {
            folder,
            entry: entry("theme.toml"),
        }]
    );
}

#[test]
fn other_files_beside_the_theme_file_are_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let sources = sources(root.path(), "#7aa2f7");
    let file = sources.theme_file.clone().unwrap();
    let folder = file.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(&file, "follow = \"none\"\nacent = \"#00ff00\"\n").unwrap();
    let app = tauri::test::mock_app();
    let themes = announced(&app);
    follow_sources(app.handle(), sources);
    started();
    // Nothing to announce: another file in the folder, and an Omarchy switch
    // while not following it.
    std::fs::write(folder.join("notes.txt"), "hello").unwrap();
    std::fs::write(root.path().join("state/current/theme/colors.toml"), palette("#b9f24b")).unwrap();
    assert!(themes.recv_timeout(Duration::from_secs(2)).is_err());
    std::fs::write(&file, "follow = \"none\"\n").unwrap();
    assert_eq!(next(&themes)["problems"], serde_json::json!([]));
}
