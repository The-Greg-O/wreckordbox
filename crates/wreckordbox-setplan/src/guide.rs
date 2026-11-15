//! The set planning guide for agents (wbx-wjl): `docs/agents/set-planning.md`,
//! embedded at build time, so the CLI (`wreckordbox set-plan guide`) and
//! the MCP server (`read_set_planning_guide`) answer the same document
//! with no service, no Library and no writes. `tests/guide.rs` recomputes
//! every table and number in it.

use serde_json::{json, Value};

/// The guide as it is in the repository.
pub const GUIDE: &str = include_str!("../../../docs/agents/set-planning.md");
/// Where it is in the repository.
pub const SOURCE: &str = "docs/agents/set-planning.md";

/// The guide as `{title, source, markdown, introduction, sections: [{id,
/// heading, markdown}]}`: one section per `## ` heading, its `markdown`
/// starting with the heading. The introduction and the sections'
/// `markdown`, joined, are the whole document.
#[must_use]
pub fn sections() -> Value {
    to_json(GUIDE)
}

fn to_json(document: &str) -> Value {
    let mut title = String::new();
    let mut introduction = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut fenced = false;
    for line in document.split_inclusive('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        }
        let bare = line.trim_end();
        if !fenced && title.is_empty() {
            if let Some(text) = bare.strip_prefix("# ") {
                text.clone_into(&mut title);
            }
        }
        if !fenced {
            if let Some(heading) = bare.strip_prefix("## ") {
                sections.push((heading.to_owned(), String::new()));
            }
        }
        match sections.last_mut() {
            Some((_, text)) => text.push_str(line),
            None => introduction.push_str(line),
        }
    }
    json!({
        "title": title,
        "source": SOURCE,
        "markdown": document,
        "introduction": introduction,
        "sections": sections
            .iter()
            .map(|(heading, text)| json!({"id": slug(heading), "heading": heading, "markdown": text}))
            .collect::<Vec<_>>(),
    })
}

/// `1. Keys` is `keys`; `4. How this app sequences: ...` is
/// `how-this-app-sequences-...`: the words, lower case, joined by `-`,
/// without the section number.
fn slug(heading: &str) -> String {
    let words = heading
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !w.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    words.join("-")
}
