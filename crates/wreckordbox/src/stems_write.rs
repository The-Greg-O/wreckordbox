//! `stems write`: a track's Stems written to a Spare stick as Native stems.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::json;
use wreckordbox_service::native_stems::{commands, NativeStems, WriteStems};
use wreckordbox_service::{Client, ServiceError};

/// Asks the service to write Native stems for track `id` to the Stick at
/// `stick`, and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    id: &str,
    stick: &Path,
) -> Result<String, ServiceError> {
    // The service does not run where this command was typed, so a Stick
    // named relative to here is named in full for it.
    let stick = std::path::absolute(stick).unwrap_or_else(|_| stick.to_path_buf());
    let written: NativeStems = client.call(
        commands::STEMS_WRITE,
        json!(WriteStems {
            id: id.to_owned(),
            stick,
        }),
    )?;
    Ok(if as_json {
        json!(written).to_string()
    } else {
        written_text(&written)
    })
}

fn written_text(written: &NativeStems) -> String {
    let track = &written.track;
    if let Some(skipped) = &written.skipped {
        return format!(
            "Skipped track {} \"{}\": no Native stems were written, because {}.\n\
             Track on the Stick  {}",
            track.id, track.title, skipped.reason, written.track_path
        );
    }
    let folder = written.folder.as_deref().map_or_else(
        || written.stick.clone(),
        |folder| written.stick.join(folder),
    );
    let mut text = if written.replaced {
        format!(
            "Replaced the Native stems for track {} \"{}\" at {}",
            track.id,
            track.title,
            folder.display()
        )
    } else {
        format!(
            "Wrote Native stems for track {} \"{}\" to {}",
            track.id,
            track.title,
            folder.display()
        )
    };
    let _ = write!(text, "\nTrack on the Stick  {}", written.track_path);
    let _ = write!(text, "\nStems               {}", written.files.join(", "));
    if let (Some(samples), Some(rate)) = (written.samples, written.sample_rate) {
        let _ = write!(text, " ({samples} samples at {rate} Hz)");
    }
    for warning in &written.warnings {
        let _ = write!(text, "\nWarning: {warning}");
    }
    text
}
