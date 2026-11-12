//! `wreckordbox intake`: the Intake and Problems queues, how many wait in
//! each, and setting a track's Prep colour.
#![allow(
    clippy::doc_markdown,
    reason = "these comments are the command's help, where MyTag is a word, not code"
)]

use std::fmt::Write as _;

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::intake::{
    commands, Colours, Counts, CountsOf, GridState, Page, PrepColour, PrepColourSet, ProblemsPage,
    Queue, QueuedTrack, SetPrepColour,
};
use wreckordbox_service::tracks::DEFAULT_PAGE_LIMIT;
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum IntakeCommands {
    /// List the tracks waiting for Intake: those with no Prep colour, each
    /// with what it still needs.
    ///
    /// NEEDS names what is still empty: `title` or `artist` (a waiting Fact
    /// proposes another), `energy`, `tags` (with the MyTag categories the
    /// track has no tag in), `hot_cues`, and `grid` (not analysed, or
    /// analysed with its analysis not locked). Rows are each track's full
    /// record with `--json`.
    List(PageArgs),
    /// List the tracks marked Problem: those whose Prep colour is the
    /// colour label named Problem in rekordbox, each with what it still
    /// needs.
    Problems {
        #[command(flatten)]
        page: PageArgs,
        #[command(flatten)]
        label: LabelArgs,
    },
    /// Say how many tracks wait in Intake and in Problems, without listing
    /// them.
    Count(LabelArgs),
    /// List the Library's eight colour labels with your names for them,
    /// and which one means Clean (the label named Clean) and which means
    /// Problem: the Prep colours `colour` takes.
    #[command(visible_alias = "colors")]
    Colours,
    /// Set a track's Prep colour and report what it was and what it is
    /// now.
    ///
    /// Giving a track a Prep colour takes it out of Intake; the colour
    /// named Problem puts it in Problems. The window and every other
    /// client of the Library service see the change at once.
    #[command(visible_alias = "color")]
    Colour {
        /// The track's identifier: the ID column of a queue.
        #[arg(value_name = "TRACK_ID")]
        id: String,
        /// The Prep colour, in any case: your name for a colour label
        /// (`Clean`), the colour itself (`Green`), or `none` to clear it.
        #[arg(value_name = "PREP_COLOUR", required_unless_present = "colour_id")]
        prep_colour: Option<String>,
        /// The colour label by its number instead (1 to 8, 0 for none), as
        /// `intake colours` lists them: for a label that is also another
        /// colour's name.
        #[arg(long, value_name = "N", conflicts_with = "prep_colour")]
        colour_id: Option<u8>,
    },
}

#[derive(Args)]
pub struct LabelArgs {
    /// The name of the colour label that marks a Problem, when it is not
    /// called Problem. `WRECKORDBOX_PROBLEM_LABEL` sets it for a service.
    #[arg(long, value_name = "NAME")]
    label: Option<String>,
}

#[derive(Args)]
pub struct PageArgs {
    /// The most tracks to print.
    #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_PAGE_LIMIT)]
    limit: usize,
    /// How many of the queue's tracks to skip; the last line of a page
    /// says what fetches the next.
    #[arg(long, value_name = "ROWS", default_value_t = 0)]
    offset: usize,
}

/// Runs one `intake` subcommand and answers what to print.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &IntakeCommands,
) -> Result<String, ServiceError> {
    match command {
        IntakeCommands::List(page) => {
            let queue: Queue = client.call(
                commands::INTAKE_LIST,
                json!(Page {
                    limit: Some(page.limit),
                    offset: page.offset,
                }),
            )?;
            Ok(if as_json {
                json!(queue).to_string()
            } else {
                queue_text(&queue, "Intake is empty: every track has a Prep colour.")
            })
        }
        IntakeCommands::Problems { page, label } => {
            let queue: Queue = client.call(
                commands::INTAKE_PROBLEMS,
                json!(ProblemsPage {
                    limit: Some(page.limit),
                    offset: page.offset,
                    label: label.label.clone(),
                }),
            )?;
            Ok(if as_json {
                json!(queue).to_string()
            } else {
                queue_text(&queue, "No tracks are marked Problem.")
            })
        }
        IntakeCommands::Count(label) => {
            let counts: Counts = client.call(
                commands::INTAKE_COUNTS,
                json!(CountsOf {
                    label: label.label.clone(),
                }),
            )?;
            Ok(if as_json {
                json!(counts).to_string()
            } else {
                counts_text(&counts)
            })
        }
        IntakeCommands::Colours => {
            let colours: Colours = client.call(commands::INTAKE_COLOURS, json!({}))?;
            Ok(if as_json {
                json!(colours).to_string()
            } else {
                colours_text(&colours)
            })
        }
        IntakeCommands::Colour {
            id,
            prep_colour,
            colour_id,
        } => {
            let set: PrepColourSet = client.call(
                commands::TRACKS_SET_PREP_COLOUR,
                json!(SetPrepColour {
                    id: id.clone(),
                    prep_colour: prep_colour.clone(),
                    colour_id: *colour_id,
                }),
            )?;
            Ok(if as_json {
                json!(set).to_string()
            } else {
                colour_text(&set)
            })
        }
    }
}

/// The two counts, one to a line; Problems says which Prep colour it
/// counted, or why it could not.
fn counts_text(counts: &Counts) -> String {
    let problems = match (&counts.problems, &counts.problem_colour) {
        (Some(problems), Some(colour)) => format!(
            "{problems}  (Prep colour {}, {})",
            colour.label, colour.colour
        ),
        _ => format!(
            "-  ({})",
            counts
                .problems_unavailable
                .as_ref()
                .map_or("not counted", |why| why.message.as_str())
        ),
    };
    format!("Intake    {}\nProblems  {problems}", counts.intake)
}

/// The eight labels as a table; MEANS says which is Clean and which is
/// Problem.
fn colours_text(colours: &Colours) -> String {
    let means = |colour: &PrepColour| {
        if colours.clean.as_ref() == Some(colour) {
            "Clean"
        } else if colours.problem.as_ref() == Some(colour) {
            "Problem"
        } else {
            ""
        }
    };
    let mut rows = vec![["ID", "COLOUR", "LABEL", "MEANS"].map(str::to_owned)];
    rows.extend(colours.colours.iter().map(|colour| {
        [
            colour.id.to_string(),
            colour.colour.clone(),
            colour.label.clone(),
            means(colour).to_owned(),
        ]
    }));
    table(&rows)
}

/// One line saying what a Prep colour edit did.
fn colour_text(set: &PrepColourSet) -> String {
    let name = |colour: &PrepColour| {
        if colour.label == colour.colour {
            colour.colour.clone()
        } else {
            format!("{} ({})", colour.label, colour.colour)
        }
    };
    let track = format!("Track {} \"{}\"", set.id, set.title);
    match (&set.prep_colour, &set.previous) {
        (now, was) if now == was => match now {
            Some(colour) => format!("{track} already has Prep colour {}.", name(colour)),
            None => format!("{track} already has no Prep colour."),
        },
        (now, was) => format!(
            "{track}: {}, was {}.",
            now.as_ref().map_or_else(
                || "no Prep colour".to_owned(),
                |colour| format!("Prep colour {}", name(colour))
            ),
            was.as_ref().map_or_else(|| "none".to_owned(), name),
        ),
    }
}

/// A page of a queue as a table, and which tracks of how many it holds.
fn queue_text(queue: &Queue, empty: &str) -> String {
    if queue.total == 0 {
        return empty.to_owned();
    }
    if queue.tracks.is_empty() {
        return format!("No tracks that far in: the queue holds {}.", queue.total);
    }
    let mut rows = vec![["ID", "TITLE", "ARTIST", "BPM", "KEY", "NEEDS"].map(str::to_owned)];
    rows.extend(queue.tracks.iter().map(|queued| {
        let track = &queued.track;
        [
            track.id.clone(),
            or_dash(&track.title),
            or_dash(&track.artist),
            track
                .bpm
                .map_or_else(|| "-".to_owned(), |bpm| format!("{bpm:.2}")),
            or_dash(&track.key),
            or_dash(&needs_text(queued)),
        ]
    }));
    let mut text = table(&rows);
    let shown = queue.tracks.len();
    if shown < queue.total {
        let _ = write!(
            text,
            "\nTracks {} to {} of {}",
            queue.offset + 1,
            queue.offset + shown,
            queue.total
        );
        match queue.next_offset {
            Some(next) => {
                let _ = write!(text, "; next page: --offset {next}");
            }
            None => text.push('.'),
        }
    }
    text
}

/// What a track still needs, by name, with the detail a name has.
fn needs_text(queued: &QueuedTrack) -> String {
    use wreckordbox_service::intake::needs;
    queued
        .needs
        .iter()
        .map(|need| match need.as_str() {
            needs::TAGS => format!(
                "{need} ({})",
                queued
                    .empty_dimensions
                    .iter()
                    .map(|dimension| dimension.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            needs::GRID => format!(
                "{need} ({})",
                match queued.grid {
                    GridState::NotAnalysed => "not analysed",
                    GridState::NotConfirmed => "not confirmed",
                    GridState::Confirmed => "confirmed",
                }
            ),
            needs::TITLE | needs::ARTIST => {
                let proposed = queued
                    .suggestions
                    .iter()
                    .find(|suggestion| suggestion.field == *need)
                    .map_or("", |suggestion| suggestion.value.as_str());
                format!("{need} ({proposed:?} proposed)")
            }
            _ => need.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rows as lines, columns two spaces apart and as wide as their longest
/// cell.
fn table<const N: usize>(rows: &[[String; N]]) -> String {
    let widths: [usize; N] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    rows.iter()
        .map(|row| {
            let line: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            line.join("  ").trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A cell with nothing in it would shift the ones after it when read back.
fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}
