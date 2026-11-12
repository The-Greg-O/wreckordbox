//! `stems separate` and `stems show`: a track's Stems in the Sidecar.
//! (`stems write` is in `stems_write.rs`.)

use std::fmt::Write as _;
use std::io::Write as _;

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::stems::{
    commands, events, Separated, StemSet, StemsProgress, TrackId, TrackStems,
};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum StemCommands {
    /// Separate one track into vocals, other (the melody), bass and drums,
    /// and keep the four Stems in the Sidecar, attached to the track.
    ///
    /// A track that already has Stems made from the same audio is not
    /// separated again: the Stems it has are reported. If its audio file
    /// has changed since, the old Stems are marked stale and it is
    /// separated afresh. The Library is not changed.
    ///
    /// Separation is slow: several times the track's length, with most of
    /// the machine busy. Progress is printed to standard error. The work is
    /// done by the Library service, so interrupting this command does not
    /// stop it; run the command again to wait for the result.
    Separate {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
    },
    /// Show the Stems the Sidecar holds for one track.
    Show {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
    },
    /// Write a track's Stems to a Spare stick as Native stems: the form a
    /// modded XDJ-AZ plays on its pads.
    ///
    /// The Stick must already hold an export of the track, and must carry
    /// the Spare stick marker: an empty file named `.rbxport-disposable` at
    /// its root, which says the Stick is safe to overwrite. A Stick without
    /// it is refused and nothing is written; never put one on a Gig stick.
    ///
    /// The track must have Stems made from the audio it has now (see
    /// `stems separate`). They are written as four Ogg Vorbis files in
    /// `STEMS/<the track's file name on the Stick>/`. Players without the
    /// mod ignore that folder, and the Stick's exported library is not
    /// touched. A track whose file name cannot be a folder name there is
    /// reported and skipped, which is not a failure. The Library is not
    /// changed.
    Write {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
        /// Where the Stick is mounted.
        #[arg(value_name = "STICK")]
        stick: std::path::PathBuf,
    },
}

/// Runs one Stems command and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &StemCommands,
) -> Result<String, ServiceError> {
    match command {
        StemCommands::Separate { id } => {
            // Progress comes as events, which a connection must ask for.
            client.subscribe()?;
            let mut shown = None;
            let separated: Separated = client.call_watching(
                commands::STEMS_SEPARATE,
                json!(TrackId { id: id.clone() }),
                |event| {
                    if as_json || event.event != events::STEMS_PROGRESS {
                        return;
                    }
                    let Ok(progress) = serde_json::from_value::<StemsProgress>(event.data) else {
                        return;
                    };
                    if progress.track_id != *id {
                        return;
                    }
                    // One line per stage and per tenth, not per event.
                    let step = (progress.stage.clone(), tenths(progress.fraction));
                    if shown.as_ref() != Some(&step) {
                        let mut stderr = std::io::stderr();
                        let _ = writeln!(
                            stderr,
                            "{}: {}%",
                            progress.stage.replace('_', " "),
                            step.1 * 10
                        );
                        shown = Some(step);
                    }
                },
            )?;
            Ok(if as_json {
                json!(separated).to_string()
            } else {
                separated_text(&separated)
            })
        }
        StemCommands::Write { id, stick } => {
            crate::stems_write::run(client, as_json, id, stick)
        }
        StemCommands::Show { id } => {
            let stems: TrackStems =
                client.call(commands::STEMS_SHOW, json!(TrackId { id: id.clone() }))?;
            Ok(if as_json {
                json!(stems).to_string()
            } else {
                shown_text(&stems)
            })
        }
    }
}

/// How many tenths of the way `fraction` is, 0 to 10.
fn tenths(fraction: f64) -> u8 {
    (0..=10_u8)
        .rev()
        .find(|&tenth| fraction >= f64::from(tenth) / 10.0)
        .unwrap_or(0)
}

fn separated_text(separated: &Separated) -> String {
    let track = &separated.track;
    let mut text = if separated.reused {
        format!(
            "Track {} \"{}\" already has Stems from this audio; they were not made again.",
            track.id, track.title
        )
    } else {
        format!(
            "Separated track {} \"{}\" into {} Stems.",
            track.id,
            track.title,
            separated.stems.stems.len()
        )
    };
    if !separated.marked_stale.is_empty() {
        text.push_str(
            "\nThe track's audio has changed: its earlier Stems were marked stale and kept.",
        );
    }
    let _ = write!(text, "\n{}", set_text(&separated.stems));
    text
}

fn shown_text(stems: &TrackStems) -> String {
    let track = &stems.track;
    let mut text = match &stems.stems {
        Some(set) => format!(
            "Track {} \"{}\" has Stems.\n{}",
            track.id,
            track.title,
            set_text(set)
        ),
        None => format!("Track {} \"{}\" has no Stems.", track.id, track.title),
    };
    for set in &stems.previous {
        let why = match (set.stale, set.superseded) {
            (true, true) => "stale and replaced",
            (true, false) => "stale",
            _ => "replaced",
        };
        let _ = write!(
            text,
            "\nEarlier Stems ({why}): Fact {}, {} samples, fingerprint {}",
            set.fact_id, set.samples, set.fingerprint
        );
    }
    text
}

/// A Stem set as lines: what it is, then one line per Stem.
fn set_text(set: &StemSet) -> String {
    let mut text = format!(
        "{} samples at {} Hz, made by {}",
        set.samples,
        set.sample_rate,
        set.produced_by["model"]
            .as_str()
            .unwrap_or(&set.source.name)
    );
    let _ = write!(text, "\nFingerprint  {}", set.fingerprint);
    for stem in &set.stems {
        let _ = write!(text, "\n{:<11}  {}", stem.role.name(), stem.path.display());
    }
    text
}
