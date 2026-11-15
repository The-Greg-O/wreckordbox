//! `wreckordbox set-plan guide` (wbx-wjl): the set planning guide an agent
//! reads before it writes a Recipe or builds a Set plan. The document is
//! `docs/agents/set-planning.md`, embedded at build time; printing it needs
//! no service and no Library, and writes nothing.

use serde_json::{json, Value};

/// The guide as it is in the repository.
pub const GUIDE: &str = include_str!("../../../docs/agents/set-planning.md");
const SOURCE: &str = "docs/agents/set-planning.md";

/// The guide as text (Markdown), or with `as_json` as
/// `{title, source, markdown, introduction, sections: [{id, heading, markdown}]}`,
/// where the introduction and the sections' `markdown`, joined, are the
/// whole document.
#[must_use]
pub fn render(as_json: bool) -> String {
    if as_json {
        to_json(GUIDE).to_string()
    } else {
        GUIDE.trim_end().to_owned()
    }
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
                title = text.to_owned();
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
