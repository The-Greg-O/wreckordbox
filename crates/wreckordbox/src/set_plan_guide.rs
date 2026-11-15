//! `wreckordbox set-plan guide` (wbx-wjl): the set planning guide an agent
//! reads before it writes a Recipe or builds a Set plan
//! (`wreckordbox_setplan::guide`). Printing it needs no service and no
//! Library, and writes nothing.

/// The guide as text (Markdown), or with `as_json` in sections.
#[must_use]
pub fn render(as_json: bool) -> String {
    if as_json {
        wreckordbox_setplan::guide::sections().to_string()
    } else {
        wreckordbox_setplan::guide::GUIDE.trim_end().to_owned()
    }
}
