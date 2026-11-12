//! A supplementary lint, not the guarantee. Library Protection is enforced
//! by the Library core itself (`rbl_library::protection`): every write the
//! core makes is refused while it is on, checked under the edit gate, and
//! the behavioural tests (`crates/rbl-library/tests/protection.rs`, the
//! CLI's `library_protection.rs`, the shell's `service_host.rs`) prove it.
//!
//! These only read the crate's own top-level source files for literal
//! spellings, to keep the service's writes going through
//! `OpenLibrary::change` (the one place that lends the core a host and
//! maps its refusals to `library_protected`). A nested module or another
//! spelling would slip past them; the core would still refuse its writes.
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
    // `change`, which maps the core's protection refusal to
    // `library_protected`.
    assert_eq!(
        library.matches("self.host").count(),
        1,
        "library.rs reads `self.host` outside `OpenLibrary::change`"
    );
    let gate = &library[library.find("fn change<").expect("OpenLibrary::change")..];
    let gate = &gate[..gate.find("\n    }\n").unwrap()];
    assert!(gate.contains("self.host"));
    assert!(
        gate.contains("edit_error"),
        "the gate must map the core's refusals, Library Protection among them"
    );
    // And no edit is made with a host of its own.
    assert_eq!(
        library.matches("&NoHost").count(),
        1,
        "only loading the Library (`set_library`) may use `&NoHost`"
    );
}
