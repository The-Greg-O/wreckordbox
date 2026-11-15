//! The agent guide (`docs/agents/guide.md`) cannot drift from the CLI:
//! every command the real `wreckordbox` binary has, found by walking its
//! own `--help`, must have a section of its own in the guide.

use std::path::Path;
use std::process::Command;

/// The subcommands `help` lists for `path`, in its order. A name is a line
/// of the `Commands:` section indented by exactly two spaces; a description
/// that runs over a line is indented further and is not a name.
fn subcommands(path: &[String]) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_wreckordbox"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("RBXPORT_TEST", "1")
        .args(path)
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success(), "{path:?} --help failed");
    let help = String::from_utf8(output.stdout).unwrap();
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if line.is_empty() || !line.starts_with(' ') {
            break;
        }
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.starts_with(' ') {
            continue;
        }
        let name = rest.split_whitespace().next().unwrap();
        if name != "help" {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every command of the CLI as it is typed, groups and leaves alike:
/// `wreckordbox tracks`, `wreckordbox tracks rate`, ...
fn every_command() -> Vec<String> {
    let mut found = Vec::new();
    let mut queue = vec![Vec::<String>::new()];
    while let Some(path) = queue.pop() {
        for name in subcommands(&path) {
            let mut child = path.clone();
            child.push(name);
            found.push(format!("wreckordbox {}", child.join(" ")));
            queue.push(child);
        }
    }
    found.sort();
    found
}

fn guide() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/agents/guide.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_walk_finds_the_commands_it_should() {
    // Guards the walk itself: if help's layout changed and nothing was
    // found, the guide check below would pass vacuously.
    let commands = every_command();
    for known in [
        "wreckordbox status",
        "wreckordbox tracks rate",
        "wreckordbox facts record-many",
        "wreckordbox stick dry-run",
        "wreckordbox runs finish",
    ] {
        assert!(
            commands.iter().any(|c| c == known),
            "{known} not found in {commands:?}"
        );
    }
    assert!(
        !commands.iter().any(|c| c.ends_with(" help")),
        "{commands:?}"
    );
}

#[test]
fn every_cli_command_has_a_section_in_the_agent_guide() {
    let guide = guide();
    let missing: Vec<String> = every_command()
        .into_iter()
        .filter(|command| {
            let heading = format!("`{command}`");
            !guide
                .lines()
                .any(|line| line.starts_with('#') && line.contains(&heading))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "docs/agents/guide.md has no heading naming these commands (a line \
         starting with `#` that contains the command in backticks): {missing:#?}"
    );
}
