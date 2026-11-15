//! `wreckordbox library rebuild-from-stick` (wbx-b1d): rebuild the Library
//! to match a Stick's library, each track linked to its file in a music
//! folder on this computer, never copied from the Stick. The Stick is only
//! read. `--dry-run` writes nothing anywhere.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::stick_rebuild::{commands, Rebuilt};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum LibraryCommands {
    /// Rebuild the Library from a Stick: its playlists and folders, and
    /// each matched track's rating, colour, comment and My Tags, linking
    /// each track to its file in the music folder.
    ///
    /// A track is matched by the size of its copy on the Stick, the start of
    /// the file's name agreeing (rekordbox shortens names on a Stick). One
    /// with no file, several, or a size match whose name does not agree is
    /// reported by name and left out; one whose file has its name but
    /// another size is linked only with `--accept-retagged`. A track already in the Library at that file is
    /// not added again; the Stick's values replace its own. Applying is one
    /// import Run, refused while Library Protection is on.
    RebuildFromStick(RebuildArgs),
}

#[derive(Args)]
pub struct RebuildArgs {
    /// Where the Stick is mounted. It is only read.
    pub stick: PathBuf,
    /// The folder holding the audio files (searched with its subfolders).
    #[arg(long)]
    pub music: PathBuf,
    /// Say what would be done, and write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Link a track whose file has its name but another size (retagged
    /// since the export) to that file, when no file has its size.
    #[arg(long)]
    pub accept_retagged: bool,
}

impl LibraryCommands {
    pub fn dry_run(&self) -> bool {
        match self {
            Self::RebuildFromStick(args) => args.dry_run,
        }
    }
}

pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &LibraryCommands,
) -> Result<String, ServiceError> {
    let LibraryCommands::RebuildFromStick(args) = command;
    let answer: Rebuilt = client.call(
        commands::LIBRARY_REBUILD_FROM_STICK,
        json!({"stick": args.stick, "music": args.music, "dry_run": args.dry_run, "accept_retagged": args.accept_retagged}),
    )?;
    if as_json {
        return Ok(json!(answer).to_string());
    }
    Ok(text(&answer))
}

fn text(answer: &Rebuilt) -> String {
    let mut out = String::new();
    let verb = if answer.dry_run {
        "Would rebuild"
    } else {
        "Rebuilt"
    };
    let _ = writeln!(
        out,
        "{verb} the Library from {} ({}), linking to {}.",
        answer.stick, answer.format, answer.music
    );
    let _ = writeln!(out, "Matched {} tracks:", answer.matched.len());
    for track in &answer.matched {
        let mut state = if track.existing.is_some() {
            "already in the Library"
        } else {
            "new"
        }
        .to_owned();
        if track.retagged {
            state.push_str(", by name: its size differs from the Stick's copy");
        }
        let _ = writeln!(
            out,
            "  {} - {} -> {} ({state})",
            track.artist, track.title, track.file
        );
        for value in &track.values {
            let from = value
                .from
                .as_ref()
                .map(|f| format!(" from {f}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "    {}: {}{from} -> {}",
                value.value, value.outcome, value.to
            );
        }
    }
    let _ = writeln!(out, "Not matched {} tracks:", answer.unmatched.len());
    for track in &answer.unmatched {
        let reason = match track.reason.as_str() {
            "no_file" => "no file of its size or name in the music folder".to_owned(),
            "ambiguous" => format!("several files it could be: {}", track.candidates.join(", ")),
            "different_size" => format!(
                "a file of that name, of another size (retagged? --accept-retagged links it): {}",
                track.candidates.join(", ")
            ),
            "name_disagrees" => format!(
                "a file of its size whose name does not agree: {}",
                track.candidates.join(", ")
            ),
            other => other.to_owned(),
        };
        let _ = writeln!(
            out,
            "  {} - {} ({}): {reason}",
            track.artist, track.title, track.stick_path
        );
    }
    let _ = writeln!(out, "Playlists:");
    for list in &answer.playlists {
        let kind = if list.folder { "folder" } else { "playlist" };
        let _ = write!(out, "  {} ({kind}): {}", list.path, list.action);
        if !list.folder {
            let _ = write!(out, ", {} tracks", list.tracks);
            if !list.left_out.is_empty() {
                let _ = write!(out, ", left out: {}", list.left_out.join(", "));
            }
        }
        out.push('\n');
    }
    let _ = writeln!(out, "MyTags:");
    for category in &answer.my_tags {
        let here = category
            .library_category
            .as_deref()
            .unwrap_or("no category here");
        let _ = writeln!(out, "  {} ({here})", category.name);
        for tag in &category.tags {
            let _ = writeln!(
                out,
                "    {}: {}, {} tracks",
                tag.name, tag.action, tag.tracks
            );
        }
    }
    let _ = writeln!(out, "Values carried:");
    for (value, tracks) in &answer.values {
        let _ = writeln!(out, "  {value}: {tracks} tracks");
    }
    let _ = writeln!(out, "Not carried:");
    for what in &answer.not_carried {
        let _ = writeln!(out, "  {what}");
    }
    if let Some(warning) = &answer.recording_warning {
        let _ = writeln!(out, "Recording warning: {warning}");
    }
    out.trim_end().to_owned()
}
