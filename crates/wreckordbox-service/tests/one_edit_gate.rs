//! Library Protection is enforced in one place, `OpenLibrary::change`,
//! because that is the service's only way to write the Library. These read
//! the crate's own source to keep it so: a change that opens a second way
//! fails here, before any DJ's protected Library is written.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;

/// Every source file of the crate, with its comment lines taken out.
fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let code: String = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .map(|line| format!("{line}\n"))
                .collect();
            files.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                code,
            ));
        }
    }
    assert!(files.iter().any(|(name, _)| name == "library.rs"));
    files
}

#[test]
fn only_the_library_module_holds_the_library_core() {
    for (name, code) in sources() {
        if name == "library.rs" {
            // It never hands the core out: no function answers one.
            assert!(
                !code.contains("-> Arc<LibraryCore>") && !code.contains("-> &LibraryCore"),
                "library.rs hands the Library core out, past Library Protection"
            );
            continue;
        }
        // Elsewhere the core is only ever passed in (by a hosting front
        // end) and on to `OpenLibrary::shared`; nothing calls a method on
        // one.
        for writing in [
            ".write_then(",
            ".write_history_then(",
            ".step_history(",
            ".refresh_after_edit(",
            "core.edit(",
            "core.recorded_edit(",
            "core.permanent_edit(",
            "Writer::open",
        ] {
            assert!(
                !code.contains(writing),
                "{name} writes the Library with `{writing}` instead of through OpenLibrary::edit, \
                 recorded_edit or permanent_edit, so Library Protection would not stop it"
            );
        }
    }
}

#[test]
fn the_host_every_edit_needs_is_lent_only_by_the_protected_gate() {
    let (_, library) = sources()
        .into_iter()
        .find(|(name, _)| name == "library.rs")
        .unwrap();

    // Every writing method of the core takes the host. It is read once, in
    // `change`, after the protection check.
    assert_eq!(
        library.matches("self.host").count(),
        1,
        "library.rs reads `self.host` outside `OpenLibrary::change`"
    );
    let gate = &library[library.find("fn change<").expect("OpenLibrary::change")..];
    let gate = &gate[..gate.find("\n    }\n").unwrap()];
    assert!(gate.contains("self.host"));
    assert!(
        gate.find("self.protected()").unwrap() < gate.find("self.host").unwrap(),
        "the gate must check Library Protection before it lends the host"
    );
    // And no edit is made with a host of its own.
    assert_eq!(
        library.matches("&NoHost").count(),
        1,
        "only loading the Library (`set_library`) may use `&NoHost`"
    );
}
