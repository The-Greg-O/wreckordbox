//! The native menu, built against a mock app.
//!
//! The packaged window cannot be screenshotted on this machine, so "the menu
//! is there" would otherwise be an assumption. A mock app builds the real
//! menu with no window at all, which settles the part that actually breaks:
//! whether it constructs, and whether the ids the frontend switches on are
//! the ids the items carry.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
// No harness means no test runner to report to; printing is the report.
#![allow(clippy::print_stdout)]

fn main() {
    // Not `#[test]`: on macOS a menu item can only be built on the main
    // thread, and the default harness runs every test on a spawned one.
    #[cfg(target_os = "macos")]
    {
        the_menu_builds_and_carries_the_ids_the_frontend_switches_on();
        the_menu_offers_the_clipboard_items_a_text_field_needs();
        the_menu_rebuilds_with_translated_labels();
        println!("3 menu checks passed");
    }
    // Linux has no menu row at all (wbx-54): the command palette lists what
    // the menus held, and the line goes to the music.
    #[cfg(target_os = "linux")]
    {
        linux_installs_no_menu_row();
        linux_keeps_no_menu_row_when_the_labels_arrive();
        the_palette_reaches_what_only_the_shell_can_do();
        the_shell_items_act_on_the_window();
        the_palette_names_the_webviews_editing_actions();
        println!("5 menu checks passed");
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("skipped: the menu bar is untested on this platform");
}

#[cfg(target_os = "linux")]
fn linux_installs_no_menu_row() {
    let app = tauri::test::mock_app();
    rbxport_lib::menu::install(app.handle()).expect("install succeeds");
    assert!(!rbxport_lib::menu::has_menu_row());
    assert!(app.menu().is_none(), "Linux must have no native menu row");
}

#[cfg(target_os = "linux")]
fn linux_keeps_no_menu_row_when_the_labels_arrive() {
    use std::collections::HashMap;

    // The window sends its translated labels at start and on every language
    // change; that used to rebuild and install the menu.
    let app = tauri::test::mock_app();
    rbxport_lib::menu::install(app.handle()).expect("install succeeds");
    let labels = HashMap::from([("File".to_owned(), "Datei".to_owned())]);
    rbxport_lib::menu::set_menu_labels(app.handle().clone(), labels).expect("labels are accepted");
    rbxport_lib::menu::set_menu_enabled(
        app.handle().clone(),
        HashMap::from([("relocate".to_owned(), true)]),
    )
    .expect("enabled states are accepted");
    rbxport_lib::menu::set_history_menu(app.handle().clone(), Some("Rating".to_owned()), None)
        .expect("history labels are accepted");
    assert!(
        app.menu().is_none(),
        "the labels must not bring the menu row back"
    );
}

#[cfg(target_os = "linux")]
fn the_palette_reaches_what_only_the_shell_can_do() {
    use tauri::Listener;

    // Full screen, Close Window and Quit were native items the window never
    // heard about; the palette asks the shell for them by id. Anything else
    // goes to the window as the same event a menu click sent.
    assert!(rbxport_lib::menu::SHELL_ITEMS.contains(&"fullscreen"));
    assert!(rbxport_lib::menu::SHELL_ITEMS.contains(&"close-window"));
    assert!(rbxport_lib::menu::SHELL_ITEMS.contains(&"quit"));

    let app = tauri::test::mock_app();
    let (sent, heard) = std::sync::mpsc::channel::<String>();
    app.listen(rbxport_lib::menu::EVENT, move |event| {
        let _ = sent.send(event.payload().to_owned());
    });
    // With no main window, Full screen is a no-op and not an error.
    rbxport_lib::menu::run_menu_item(app.handle().clone(), "fullscreen".to_owned())
        .expect("full screen is accepted");
    rbxport_lib::menu::run_menu_item(app.handle().clone(), "missing".to_owned())
        .expect("a window item is accepted");
    let payload = heard
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the window hears the item");
    assert_eq!(payload, "\"missing\"");
    assert!(
        heard.try_recv().is_err(),
        "full screen is the shell's own and must not reach the window"
    );
}

/// A window that remembers what was done to it.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct Watched {
    full: std::cell::Cell<bool>,
    closed: std::cell::Cell<u32>,
}

#[cfg(target_os = "linux")]
impl rbxport_lib::menu::ShellWindow for Watched {
    fn is_fullscreen(&self) -> bool {
        self.full.get()
    }
    fn set_fullscreen(&self, on: bool) -> Result<(), String> {
        self.full.set(on);
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        self.closed.set(self.closed.get() + 1);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn the_shell_items_act_on_the_window() {
    use rbxport_lib::menu::run_shell_item;

    let window = Watched::default();
    // Full screen goes in, and out again.
    assert!(run_shell_item(&window, "fullscreen").unwrap());
    assert!(
        window.full.get(),
        "full screen must make the window full screen"
    );
    assert!(run_shell_item(&window, "fullscreen").unwrap());
    assert!(!window.full.get(), "full screen again must leave it");
    // Close Window and Quit close the main window (which quits).
    assert!(run_shell_item(&window, "close-window").unwrap());
    assert_eq!(window.closed.get(), 1);
    assert!(run_shell_item(&window, "quit").unwrap());
    assert_eq!(window.closed.get(), 2);
    // Anything else is the window's, not the shell's.
    assert!(!run_shell_item(&window, "missing").unwrap());
    assert_eq!(window.closed.get(), 2);
    assert!(!window.full.get());
}

#[cfg(target_os = "linux")]
fn the_palette_names_the_webviews_editing_actions() {
    use rbxport_lib::menu::editing_command;

    // WebKit's own command names (`WEBKIT_EDITING_COMMAND_*`).
    assert_eq!(editing_command("cut"), Some("Cut"));
    assert_eq!(editing_command("copy"), Some("Copy"));
    assert_eq!(editing_command("paste"), Some("Paste"));
    assert_eq!(editing_command("selectAll"), Some("SelectAll"));
    assert_eq!(editing_command("delete-library"), None);

    // Through a window: accepted for an editing action, refused otherwise.
    let app = tauri::test::mock_app();
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("a mock window");
    rbxport_lib::menu::run_edit_command(window.clone(), "paste".to_owned())
        .expect("paste is accepted");
    assert!(rbxport_lib::menu::run_edit_command(window, "format".to_owned()).is_err());
}

/// Every id the menu emits, in the order the menus are laid out.
#[cfg(target_os = "macos")]
fn ids(app: &tauri::AppHandle<tauri::test::MockRuntime>) -> Vec<String> {
    let menu = rbxport_lib::menu::build(app).expect("the menu builds");
    let mut out = Vec::new();
    for item in menu.items().expect("top level") {
        let Some(submenu) = item.as_submenu() else {
            continue;
        };
        for child in submenu.items().expect("submenu items") {
            out.push(child.id().0.clone());
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn the_menu_builds_and_carries_the_ids_the_frontend_switches_on() {
    let app = tauri::test::mock_app();
    let found = ids(&app.handle().clone());

    // These three are the contract with src/lib/menu.ts. A rename on either
    // side turns a menu item into one that silently does nothing.
    for id in [
        "settings",
        "import",
        "import-folder",
        "missing",
        "info",
        "sub",
        "fullscreen",
    ] {
        assert!(
            found.iter().any(|f| f == id),
            "no item with id {id}; got {found:?}"
        );
    }
}

#[cfg(target_os = "macos")]
fn the_menu_offers_the_clipboard_items_a_text_field_needs() {
    // Without a predefined Edit menu the standard shortcuts never reach the
    // webview on macOS, and renaming a playlist stops accepting paste.
    let app = tauri::test::mock_app();
    let menu = rbxport_lib::menu::build(&app.handle().clone()).expect("the menu builds");
    let titles: Vec<String> = menu
        .items()
        .expect("top level")
        .iter()
        .filter_map(|item| item.as_submenu().and_then(|s| s.text().ok()))
        .collect();
    assert!(
        titles.iter().any(|t| t == "Edit"),
        "no Edit menu; got {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "File"),
        "no File menu; got {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "View"),
        "no View menu; got {titles:?}"
    );
}

#[cfg(target_os = "macos")]
fn the_menu_rebuilds_with_translated_labels() {
    use std::collections::HashMap;

    let app = tauri::test::mock_app();
    app.set_menu(rbxport_lib::menu::build(&app.handle().clone()).expect("English menu builds"))
        .expect("English menu installs");
    let labels = HashMap::from([
        ("File".to_owned(), "Datei".to_owned()),
        ("Edit".to_owned(), "Bearbeiten".to_owned()),
        ("View".to_owned(), "Ansicht".to_owned()),
    ]);
    rbxport_lib::menu::set_menu_labels(app.handle().clone(), labels)
        .expect("translated menu installs");

    let menu = app.menu().expect("installed menu");
    let titles: Vec<String> = menu
        .items()
        .expect("top level")
        .iter()
        .filter_map(|item| item.as_submenu().and_then(|submenu| submenu.text().ok()))
        .collect();
    for translated in ["Datei", "Bearbeiten", "Ansicht"] {
        assert!(
            titles.iter().any(|title| title == translated),
            "no {translated} menu; got {titles:?}"
        );
    }
}
