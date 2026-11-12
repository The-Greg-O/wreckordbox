//! Recovery observations for file edits. Called during recovery, never by a
//! query or on the write path. Evidence is metadata, never audio or fingerprints.
use serde_json::Value;

pub(super) const PLAYER_RUN: &str = "Changes from a player";

pub(super) fn is_link(command: Option<&Value>) -> bool {
    command.is_some_and(|c| c["name"] == "files.edit" && c["args"]["kind"] == "link_grid_offset")
}

pub(super) fn recovery_label(
    location: &rbl_db::LibraryLocation,
    command: Option<&Value>,
    fallback: &str,
) -> String {
    let Some(command) = command.filter(|c| c["name"] == "files.edit") else {
        return fallback.to_owned();
    };
    let prefix = "An analysis file change was begun; completion is unconfirmed";
    let state = observed(location, &command["args"]);
    match state {
        Some("before") => {
            format!("{prefix}. File is at its before state (not published or rolled back).")
        }
        Some("after") => format!("{prefix}. File is at its after state (publication observed)."),
        _ => format!("{prefix}. The file's outcome is unknown."),
    }
}

fn observed(location: &rbl_db::LibraryLocation, args: &Value) -> Option<&'static str> {
    let relative = args["analysis"].as_str()?;
    let path = rbl_anlz::resolve(&location.share_root, relative)
        .canonicalize()
        .ok()?;
    if !path.starts_with(location.share_root.canonicalize().ok()?)
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("DAT"))
        || std::fs::metadata(&path).ok()?.len() > 4 * 1024 * 1024
    {
        return None;
    }
    let file = rbl_anlz::Anlz::read(&path).ok()?;
    let value = match args["kind"].as_str()? {
        "link_grid_offset" => Value::from(file.grid_offset()?),
        "window_grid" => serde_json::json!(file
            .beat_grid()?
            .iter()
            .map(|b| (b.time_ms, b.beat_number, b.tempo_x100))
            .collect::<Vec<_>>()),
        _ => return None,
    };
    if value == args["before"] {
        Some("before")
    } else if value == args["after"] {
        Some("after")
    } else {
        None
    }
}
