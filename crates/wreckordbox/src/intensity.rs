//! `intensity measure`, `intensity show` and `intensity cancel`: how hard
//! a track hits, measured from its audio and kept in the Sidecar.

use std::fmt::Write as _;
use std::io::Write as _;

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::intensity::{
    commands, events, Cancelled, IntensityProgress, Measure, Measured, Measurement, Status,
    TrackIntensity, TrackOutcome,
};
use wreckordbox_service::stems::TrackId;
use wreckordbox_service::tracks::TrackSelection;
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum IntensityCommands {
    /// Measure the Intensity of one track, of several, or of every track
    /// a selection matches, and keep each as a Measurement in the Sidecar.
    ///
    /// Intensity is a summary from 0 to 1 and a curve along the track, one
    /// point a second and, for a track with a beat grid, one point a bar.
    /// It is computed from loudness, the weight of the low and high end,
    /// and how much happens rhythmically.
    ///
    /// A track already measured from the audio it has now is not measured
    /// again. If its audio file has changed, the old Measurement is marked
    /// stale and it is measured afresh. Everything one command measures is
    /// one Run. The Library is not changed.
    ///
    /// Progress is printed to standard error. The work is done by the
    /// Library service, so interrupting this command does not stop it:
    /// `intensity cancel` does, keeping the tracks already finished.
    Measure(MeasureArgs),
    /// Show the Intensity the Sidecar holds for one track; with `--json`,
    /// in full, with every curve.
    Show {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "TRACK_ID")]
        id: String,
    },
    /// Stop the measuring in progress. Tracks already measured are kept;
    /// measuring again later carries on with the rest.
    Cancel,
}

#[derive(Args)]
pub struct MeasureArgs {
    /// Tracks by identifier: the ID column of `tracks list`.
    #[arg(value_name = "TRACK_ID")]
    ids: Vec<String>,
    /// Every track in this playlist, by identifier or by name.
    #[arg(long, value_name = "PLAYLIST")]
    playlist: Option<String>,
    /// Every track matching this text, as the window's search box would.
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,
    /// Every track whose genre contains this text.
    #[arg(long, value_name = "TEXT")]
    genre: Option<String>,
    /// Every track whose audio file's path contains this text.
    #[arg(long, value_name = "TEXT")]
    location: Option<String>,
    /// Every track with this Energy (the star rating), 1 to 5, or a range
    /// such as `3..5`; 0 is a track with none.
    #[arg(long, value_name = "ENERGY")]
    energy: Option<String>,
    /// Every track in the Library.
    #[arg(long)]
    all: bool,
    /// Measure again even where a current Measurement exists.
    #[arg(long)]
    force: bool,
}

impl MeasureArgs {
    fn request(&self) -> Measure {
        let selects = self.all
            || self.playlist.is_some()
            || self.search.is_some()
            || self.genre.is_some()
            || self.location.is_some()
            || self.energy.is_some();
        Measure {
            ids: self.ids.clone(),
            selection: selects.then(|| TrackSelection {
                search: self.search.clone().unwrap_or_default(),
                playlist: self.playlist.clone(),
                genre: self.genre.clone(),
                location: self.location.clone(),
                energy: self.energy.clone(),
                ..TrackSelection::default()
            }),
            force: self.force,
        }
    }
}

/// Runs one Intensity command and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &IntensityCommands,
) -> Result<String, ServiceError> {
    match command {
        IntensityCommands::Measure(args) => {
            // Progress comes as events, which a connection must ask for.
            client.subscribe()?;
            let measured: Measured =
                client.call_watching(commands::INTENSITY_MEASURE, json!(args.request()), |event| {
                    if as_json || event.event != events::INTENSITY_PROGRESS {
                        return;
                    }
                    let Ok(progress) = serde_json::from_value::<IntensityProgress>(event.data)
                    else {
                        return;
                    };
                    // One line per track settled.
                    if progress.status != "measuring" {
                        let mut stderr = std::io::stderr();
                        let _ = writeln!(
                            stderr,
                            "[{}/{}] {}: {} \"{}\"",
                            progress.done,
                            progress.total,
                            progress.status,
                            progress.track_id,
                            progress.title
                        );
                    }
                })?;
            Ok(if as_json {
                json!(measured).to_string()
            } else {
                measured_text(&measured)
            })
        }
        IntensityCommands::Show { id } => {
            let shown: TrackIntensity =
                client.call(commands::INTENSITY_SHOW, json!(TrackId { id: id.clone() }))?;
            Ok(if as_json {
                json!(shown).to_string()
            } else {
                shown_text(&shown)
            })
        }
        IntensityCommands::Cancel => {
            let cancelled: Cancelled = client.call(commands::INTENSITY_CANCEL, json!({}))?;
            Ok(if as_json {
                json!(cancelled).to_string()
            } else if cancelled.cancelled {
                "Stopping the measuring; the tracks already measured are kept.".to_owned()
            } else {
                "Nothing is being measured.".to_owned()
            })
        }
    }
}

fn tracks(count: usize) -> String {
    format!("{count} {}", if count == 1 { "track" } else { "tracks" })
}

fn measured_text(measured: &Measured) -> String {
    if measured.total == 0 {
        return "No tracks to measure.".to_owned();
    }
    let mut text = match measured.tracks.as_slice() {
        [only] => one_text(only),
        _ => {
            let mut text = format!("Measured {}", tracks(measured.measured));
            if measured.reused > 0 {
                let _ = write!(text, ", {} already measured", measured.reused);
            }
            if measured.failed > 0 {
                let _ = write!(text, ", {} failed", measured.failed);
            }
            let _ = write!(text, ", of {}.", measured.total);
            for outcome in &measured.tracks {
                let _ = write!(text, "\n{}", line_text(outcome));
            }
            text
        }
    };
    if measured.cancelled {
        text.push_str("\nThe measuring was cancelled before every track was done.");
    }
    if let Some(run) = measured.run {
        let _ = write!(text, "\nRun {}", run.0);
    }
    text
}

/// One track of many: its identifier, what became of it, its Intensity.
fn line_text(outcome: &TrackOutcome) -> String {
    let track = &outcome.track;
    match (&outcome.summary, &outcome.error) {
        (Some(summary), _) => format!(
            "{:<8}  {:<8}  {:.2}  \"{}\"",
            track.id,
            status_word(outcome.status),
            summary.intensity,
            track.title
        ),
        (None, Some(error)) => format!(
            "{:<8}  {:<8}  -     \"{}\": {}",
            track.id,
            status_word(outcome.status),
            track.title,
            error.message
        ),
        (None, None) => format!(
            "{:<8}  {:<8}  -     \"{}\"",
            track.id,
            status_word(outcome.status),
            track.title
        ),
    }
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Measured => "measured",
        Status::Reused => "reused",
        Status::Failed => "failed",
        Status::Cancelled => "not done",
    }
}

fn one_text(outcome: &TrackOutcome) -> String {
    let track = &outcome.track;
    let mut text = match outcome.status {
        Status::Measured => format!(
            "Measured the Intensity of track {} \"{}\".",
            track.id, track.title
        ),
        Status::Reused => format!(
            "Track {} \"{}\" is already measured from this audio; it was not measured again.",
            track.id, track.title
        ),
        Status::Failed => format!(
            "Track {} \"{}\" could not be measured: {}",
            track.id,
            track.title,
            outcome
                .error
                .as_ref()
                .map_or("no reason given", |error| error.message.as_str())
        ),
        Status::Cancelled => format!(
            "Track {} \"{}\" was not measured.",
            track.id, track.title
        ),
    };
    if !outcome.marked_stale.is_empty() {
        text.push_str("\nThe track's audio has changed: its earlier Measurements were marked stale and kept.");
    }
    if let Some(summary) = &outcome.summary {
        let _ = write!(
            text,
            "\nIntensity    {:.2} (lowest quarter {:.2}, highest quarter {:.2})",
            summary.intensity, summary.low, summary.high
        );
        let _ = write!(
            text,
            "\nLoudness     {:.1} LUFS integrated",
            summary.loudness_integrated_lufs
        );
    }
    if let Some(points) = outcome.points {
        let _ = write!(text, "\nCurve        {points} points, one a second");
        match outcome.bars {
            Some(bars) => {
                let _ = write!(text, "; {bars} bars");
            }
            None => text.push_str("; no beat grid, so no bars"),
        }
    }
    if let Some(fingerprint) = &outcome.fingerprint {
        let _ = write!(text, "\nFingerprint  {fingerprint}");
    }
    text
}

fn shown_text(shown: &TrackIntensity) -> String {
    let track = &shown.track;
    let mut text = match &shown.intensity {
        Some(measurement) => format!(
            "Track {} \"{}\" has an Intensity.\n{}",
            track.id,
            track.title,
            measurement_text(measurement)
        ),
        None => format!(
            "Track {} \"{}\" has no Intensity from the audio it has now.",
            track.id, track.title
        ),
    };
    for earlier in &shown.previous {
        let why = match (earlier.stale, earlier.superseded) {
            (true, true) => "stale and replaced",
            (true, false) => "stale",
            _ => "replaced",
        };
        let _ = write!(text, "\nEarlier Intensity ({why}): Fact {}", earlier.fact_id);
        if let Ok(intensity) = earlier.intensity() {
            let _ = write!(text, ", {:.2}", intensity.summary.intensity);
        }
    }
    text
}

fn measurement_text(measurement: &Measurement) -> String {
    let mut text = String::new();
    match measurement.intensity() {
        Ok(intensity) => {
            let summary = &intensity.summary;
            let _ = write!(
                text,
                "Intensity    {:.2} (lowest quarter {:.2}, highest quarter {:.2})",
                summary.intensity, summary.low, summary.high
            );
            let _ = write!(
                text,
                "\nLoudness     {:.1} LUFS integrated, {:.1} LUFS loudest three seconds",
                summary.loudness_integrated_lufs, summary.loudness_short_term_max_lufs
            );
            let _ = write!(
                text,
                "\nLow end      {:.1} dB\nHigh end     {:.1} dB\nOnsets       {:.1} a second",
                summary.features.low_end, summary.features.high_end, summary.features.onset_density
            );
            let _ = write!(
                text,
                "\nCurve        {} points, one every {} s",
                intensity.curve.intensity.len(),
                intensity.curve.step_seconds
            );
            match &intensity.bars {
                Some(bars) => {
                    let _ = write!(text, "; {} bars", bars.intensity.len());
                }
                None => text.push_str("; no beat grid, so no bars"),
            }
            let _ = write!(
                text,
                "\nMethod       {} version {}",
                intensity.method.name, intensity.method.version
            );
        }
        Err(_) => text.push_str("Its value is of a shape this version does not read."),
    }
    let _ = write!(text, "\nFact         {}, Run {}", measurement.fact_id, measurement.run.0);
    if let Some(fingerprint) = &measurement.fingerprint {
        let _ = write!(text, "\nFingerprint  {fingerprint}");
    }
    text
}
