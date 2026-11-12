//! A theme switch on disk reaches the windows: a change under a temporary
//! folder, through the real inotify watcher, out as the `theme:changed` event
//! of a mock app. Nothing here reads or touches the DJ's own theme.
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use rbxport_lib::omarchy_theme::{follow, follow_sources, Sources, CHANGED_EVENT, COLORS_FILE_ENV};
use tauri::Listener;

fn palette(accent: &str) -> String {
    format!("background = \"#1a1b26\"\nforeground = \"#a9b1d6\"\naccent = \"{accent}\"\n")
}

/// The accents of the themes the app announces, `None` for "no theme".
fn accents(app: &tauri::App<tauri::test::MockRuntime>) -> Receiver<Option<String>> {
    let (send, receive) = channel();
    app.listen(CHANGED_EVENT, move |event| {
        let theme: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        let _ = send.send(theme["roles"]["accent"].as_str().map(str::to_owned));
    });
    receive
}

/// The watcher registers its folders on its own thread; give it a moment
/// before the first change so that change is not the one it starts from.
fn started() {
    std::thread::sleep(Duration::from_millis(600));
}

fn next(announced: &Receiver<Option<String>>) -> Option<String> {
    announced
        .recv_timeout(Duration::from_secs(20))
        .expect("the app should announce the changed theme")
}

/// An Omarchy-shaped state folder: `<root>/current/theme/colors.toml`.
fn omarchy(root: &Path) -> Sources {
    Sources {
        colors: Some(root.join("current/theme/colors.toml")),
        theme_folder_is_replaced: true,
        font_folder: None,
        theme_file: None,
    }
}

fn write_theme(folder: &Path, accent: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join("colors.toml"), palette(accent)).unwrap();
}

#[test]
fn rewriting_the_override_file_reaches_the_windows() {
    let folder = tempfile::tempdir().unwrap();
    let file: PathBuf = folder.path().join("colors.toml");
    std::fs::write(&file, palette("#7aa2f7")).unwrap();
    // The one test that goes through the environment, as the screenshot
    // script does; the others name their folders outright.
    std::env::set_var(COLORS_FILE_ENV, &file);
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow(app.handle());
    started();

    std::fs::write(&file, palette("#7daea3")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));

    std::fs::remove_file(&file).unwrap();
    assert_eq!(next(&announced), None);

    std::fs::write(&file, palette("#b9f24b")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#b9f24b"));
}

#[test]
fn a_theme_folder_replaced_whole_reaches_the_windows() {
    // What `omarchy theme set` does: build the next theme beside the current
    // one, remove the current one, rename the next into place.
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("current");
    write_theme(&current.join("theme"), "#7aa2f7");
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow_sources(app.handle(), omarchy(root.path()));
    started();

    write_theme(&current.join("next-theme"), "#7daea3");
    std::fs::remove_dir_all(current.join("theme")).unwrap();
    std::fs::rename(current.join("next-theme"), current.join("theme")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));
}

#[test]
fn a_linked_theme_folder_pointed_elsewhere_reaches_the_windows() {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("current");
    std::fs::create_dir_all(&current).unwrap();
    write_theme(&root.path().join("themes/one"), "#7aa2f7");
    write_theme(&root.path().join("themes/two"), "#7daea3");
    write_theme(&root.path().join("themes/three"), "#b9f24b");
    std::os::unix::fs::symlink(root.path().join("themes/one"), current.join("theme")).unwrap();
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow_sources(app.handle(), omarchy(root.path()));
    started();

    // Removed and linked again.
    std::fs::remove_file(current.join("theme")).unwrap();
    std::os::unix::fs::symlink(root.path().join("themes/two"), current.join("theme")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));

    // Replaced in one step, as `ln -sfn` does: a new link renamed over it.
    std::os::unix::fs::symlink(root.path().join("themes/three"), current.join("theme.new"))
        .unwrap();
    std::fs::rename(current.join("theme.new"), current.join("theme")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#b9f24b"));
}

#[test]
fn a_folder_that_is_not_there_at_startup_is_followed_once_it_appears() {
    // No Omarchy state at all when the app starts; only `root` exists.
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state/omarchy");
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow_sources(app.handle(), omarchy(&state));
    started();

    write_theme(&state.join("current/theme"), "#7aa2f7");
    assert_eq!(next(&announced).as_deref(), Some("#7aa2f7"));

    // And it goes on being followed, which needs a watch on the new folder
    // itself and not only on the one that was there at startup.
    write_theme(&state.join("current/next-theme"), "#7daea3");
    std::fs::remove_dir_all(state.join("current/theme")).unwrap();
    std::fs::rename(
        state.join("current/next-theme"),
        state.join("current/theme"),
    )
    .unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));
}

#[test]
fn a_folder_removed_and_made_again_is_followed_again() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state/omarchy");
    write_theme(&state.join("current/theme"), "#7aa2f7");
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow_sources(app.handle(), omarchy(&state));
    started();

    std::fs::remove_dir_all(root.path().join("state")).unwrap();
    assert_eq!(next(&announced), None);

    write_theme(&state.join("current/theme"), "#7daea3");
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));

    std::fs::write(state.join("current/theme/colors.toml"), palette("#b9f24b")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#b9f24b"));
}

#[test]
fn a_font_folder_that_appears_later_is_watched_without_a_false_announcement() {
    // `omarchy font set` creates ~/.config/fontconfig when it is not there.
    // The font itself is the system's and does not change here, so nothing
    // may be announced; a theme change afterwards still is.
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("theme/colors.toml");
    write_theme(&root.path().join("theme"), "#7aa2f7");
    let app = tauri::test::mock_app();
    let announced = accents(&app);
    follow_sources(
        app.handle(),
        Sources {
            colors: Some(file.clone()),
            theme_folder_is_replaced: false,
            font_folder: Some(root.path().join("config/fontconfig")),
            theme_file: None,
        },
    );
    started();

    std::fs::create_dir_all(root.path().join("config/fontconfig")).unwrap();
    std::fs::write(
        root.path().join("config/fontconfig/fonts.conf"),
        "<fontconfig/>",
    )
    .unwrap();
    assert!(announced.recv_timeout(Duration::from_secs(2)).is_err());

    std::fs::write(&file, palette("#7daea3")).unwrap();
    assert_eq!(next(&announced).as_deref(), Some("#7daea3"));
}

/// The switch both halves of the app agree on (see the fixture's `about`).
const SWITCH: &str = include_str!("../../src/lib/themeSwitch.fixture.json");

#[test]
fn a_swapped_theme_link_announces_exactly_the_roles_the_window_is_tested_with() {
    let switch: serde_json::Value = serde_json::from_str(SWITCH).unwrap();
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("current");
    std::fs::create_dir_all(&current).unwrap();
    write_theme(&root.path().join("themes/before"), "#7aa2f7");
    let after = root.path().join("themes/after");
    std::fs::create_dir_all(&after).unwrap();
    std::fs::write(
        after.join("colors.toml"),
        switch["palette"].as_str().unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(root.path().join("themes/before"), current.join("theme")).unwrap();

    let app = tauri::test::mock_app();
    let (send, announced) = channel();
    app.listen(CHANGED_EVENT, move |event| {
        let theme: serde_json::Value = serde_json::from_str(event.payload()).unwrap();
        let _ = send.send(theme);
    });
    follow_sources(app.handle(), omarchy(root.path()));
    started();

    // `ln -sfn`: a new link renamed over the old one.
    std::os::unix::fs::symlink(&after, current.join("theme.new")).unwrap();
    std::fs::rename(current.join("theme.new"), current.join("theme")).unwrap();
    let theme = announced
        .recv_timeout(Duration::from_secs(20))
        .expect("the app should announce the changed theme");
    assert_eq!(
        theme["roles"], switch["roles"],
        "announced: {}",
        theme["roles"]
    );
}
