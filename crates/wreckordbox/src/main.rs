//! The `wreckordbox` command: a thin client of the Library service.
//!
//! Every subcommand talks to the service that owns the Library, starting one
//! when none is running. Results go to standard output, as text or with
//! `--json` as JSON; errors go to standard error with a non-zero status.

mod facts;
mod intake;
mod intensity;
mod mytag_edits;
mod reading;
mod stems;
mod tracks;

use std::fmt::Write as _;
use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use wreckordbox_service::{
    Client, Config, Launcher, RatedTrack, Service, ServiceError, Status, TrackList, TrackQuery,
    DEFAULT_TRACK_LIMIT,
};

/// Work on the wreckordbox Library from a terminal.
///
/// One background Library service owns the Library; this command and the
/// window are both clients of it. A service is started when none is running
/// and stops by itself when nothing is using it.
#[derive(Parser)]
#[command(name = "wreckordbox", version, propagate_version = true)]
struct Cli {
    /// Print results as JSON on standard output, and errors as JSON on
    /// standard error.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Report whether the Library service is running and which Library it
    /// has open, starting the service if it is not running.
    Status,
    /// Find tracks in the Library.
    Tracks {
        #[command(subcommand)]
        command: TrackCommands,
    },
    /// Read and edit the My Tag categories and tags, and which tracks carry
    /// them.
    Mytags {
        #[command(subcommand)]
        command: reading::MyTagCommands,
    },
    /// Read the playlist tree and what is in a playlist.
    Playlists {
        #[command(subcommand)]
        command: reading::PlaylistCommands,
    },
    /// Read play history: what was played on which occasion, in order.
    History {
        #[command(subcommand)]
        command: reading::HistoryCommands,
    },
    /// Separate a track into Stems, kept in the Sidecar beside the Library.
    Stems {
        #[command(subcommand)]
        command: stems::StemCommands,
    },
    /// Record and read Facts: what is known about a track, kept in the
    /// Sidecar with its source and confidence. Never changes the Library.
    Facts {
        #[command(subcommand)]
        command: facts::FactCommands,
    },
    /// Start and end Runs: pieces of work whose changes belong together.
    Runs {
        #[command(subcommand)]
        command: facts::RunCommands,
    },
    /// The Intake and Problems queues: tracks with no Prep colour, tracks
    /// marked Problem, and what each still needs.
    Intake {
        #[command(subcommand)]
        command: intake::IntakeCommands,
    },
    /// Measure Intensity: how hard a track hits and how that changes over
    /// its length, from its audio. Kept in the Sidecar; never changes the
    /// Library.
    Intensity {
        #[command(subcommand)]
        command: intensity::IntensityCommands,
    },
    /// Run the Library service in the foreground until interrupted.
    ///
    /// Not needed in ordinary use: the other subcommands start a service on
    /// demand. Refuses to run when a service already owns the Library.
    Serve {
        /// Run as the background service a client started.
        #[arg(long, hide = true)]
        spawned: bool,
    },
}

#[derive(Subcommand)]
enum TrackCommands {
    /// List tracks with their identifier, title, artist, BPM, key and
    /// rating, in the order the window lists the collection.
    List {
        /// Only tracks matching this text, as the window's search box
        /// would: every word must appear in the track's title, artist,
        /// album or other details, in any case.
        #[arg(long, value_name = "TEXT", default_value = "")]
        filter: String,
        /// The most rows to print. The JSON result's `total` says how many
        /// tracks matched.
        #[arg(long, value_name = "ROWS", default_value_t = DEFAULT_TRACK_LIMIT)]
        limit: usize,
    },
    /// Set a track's rating and report what it was and what it is now.
    ///
    /// The window and every other client of the Library service see the
    /// change at once. Nothing is changed when the track is not in the
    /// Library, or while rekordbox is running.
    Rate {
        /// The track's identifier: the ID column of `tracks list`.
        #[arg(value_name = "ID")]
        id: String,
        /// Stars, 0 to 5. 0 clears the rating.
        #[arg(value_name = "STARS")]
        stars: u8,
    },
    /// Show everything the Library holds about one track.
    Show(tracks::ShowArgs),
    /// Find tracks by any of their fields, by MyTag and by playlist, a page
    /// at a time. Every option given must hold.
    ///
    /// A number or a date is one value (`2021`) or an inclusive range
    /// (`2019..2021`, `2019..`, `..2021`). Rows are each track's full
    /// record with `--json`, as `tracks show` gives it. `tracks list` is
    /// the short form: the same search, six columns.
    #[allow(
        clippy::doc_markdown,
        reason = "this is the command's help, where MyTag is a word, not code"
    )]
    Query(Box<tracks::QueryArgs>),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            let mut stderr = std::io::stderr();
            let _ = if cli.json {
                writeln!(stderr, "{}", json!({ "error": error }))
            } else {
                writeln!(stderr, "wreckordbox: {error}")
            };
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, ServiceError> {
    let config = Config::from_env()?;
    match &cli.command {
        Commands::Status => {
            let status = connect(&config)?.status()?;
            if cli.json {
                print(&json!(status).to_string());
            } else {
                print(&status_text(&status));
            }
        }
        Commands::Tracks {
            command: TrackCommands::List { filter, limit },
        } => {
            let list = connect(&config)?.tracks(&TrackQuery {
                filter: filter.clone(),
                limit: Some(*limit),
            })?;
            if cli.json {
                print(&json!(list).to_string());
            } else {
                print(&tracks_text(&list));
            }
        }
        Commands::Tracks {
            command: TrackCommands::Rate { id, stars },
        } => {
            let rated = connect(&config)?.rate_track(id, *stars)?;
            if cli.json {
                print(&json!(rated).to_string());
            } else {
                print(&rated_text(&rated));
            }
        }
        Commands::Mytags { command } => {
            print(&reading::my_tags(
                &mut connect(&config)?,
                cli.json,
                command,
            )?);
        }
        Commands::Playlists { command } => {
            print(&reading::playlists(
                &mut connect(&config)?,
                cli.json,
                command,
            )?);
        }
        Commands::History { command } => {
            print(&reading::history(
                &mut connect(&config)?,
                cli.json,
                command,
            )?);
        }
        Commands::Tracks {
            command: TrackCommands::Show(args),
        } => print(&tracks::show(&mut connect(&config)?, args, cli.json)?),
        Commands::Tracks {
            command: TrackCommands::Query(args),
        } => print(&tracks::query(&mut connect(&config)?, args, cli.json)?),
        Commands::Stems { command } => {
            print(&stems::run(&mut connect(&config)?, cli.json, command)?);
        }
        Commands::Facts { command } => {
            print(&facts::facts(&mut connect(&config)?, cli.json, command)?);
        }
        Commands::Runs { command } => {
            print(&facts::runs(&mut connect(&config)?, cli.json, command)?);
        }
        Commands::Intake { command } => {
            print(&intake::run(&mut connect(&config)?, cli.json, command)?);
        }
        Commands::Intensity { command } => {
            print(&intensity::run(&mut connect(&config)?, cli.json, command)?);
        }
        Commands::Serve { spawned: true } => {
            return Ok(if wreckordbox_service::serve_spawned(config) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        Commands::Serve { spawned: false } => {
            let service = Service::start(wreckordbox_service::locate_library()?, &config)?;
            let status = service.status()?;
            if cli.json {
                print(&json!(status).to_string());
            } else {
                print(&status_text(&status));
            }
            service.wait();
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A client of the service for this machine's Library, started if need be
/// by running this same program as `wreckordbox serve`.
fn connect(config: &Config) -> Result<Client, ServiceError> {
    let library = wreckordbox_service::locate_library()?;
    let program = std::env::current_exe().map_err(|e| {
        ServiceError::new(
            wreckordbox_service::codes::SERVICE_FAILED,
            format!("could not find this program: {e}"),
        )
    })?;
    let launcher = Launcher {
        program,
        args: vec!["serve".into(), "--spawned".into()],
    };
    Client::connect_or_start(config, &library, &launcher)
}

fn status_text(status: &Status) -> String {
    let tracks = status.library.tracks;
    format!(
        "Library service is running (pid {})\nLibrary: {} ({tracks} {})\nSocket:  {}",
        status.pid,
        status.library.path.display(),
        if tracks == 1 { "track" } else { "tracks" },
        status.socket.display(),
    )
}

/// The tracks as a table: one line each under a header, columns two spaces
/// apart and as wide as their longest cell.
fn tracks_text(list: &TrackList) -> String {
    if list.total == 0 {
        return "No tracks.".to_owned();
    }
    let mut rows = vec![["ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"].map(str::to_owned)];
    rows.extend(list.tracks.iter().map(|track| {
        [
            track.id.clone(),
            or_dash(&track.title),
            or_dash(&track.artist),
            track
                .bpm
                .map_or_else(|| "-".to_owned(), |bpm| format!("{bpm:.2}")),
            or_dash(&track.key),
            track.rating.to_string(),
        ]
    }));
    let widths: [usize; 6] = std::array::from_fn(|column| {
        rows.iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    });
    let lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let line: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            line.join("  ").trim_end().to_owned()
        })
        .collect();
    let mut text = lines.join("\n");
    if list.tracks.len() < list.total {
        let _ = write!(
            text,
            "\nShowing {} of {} tracks; raise --limit to see more.",
            list.tracks.len(),
            list.total
        );
    }
    text
}

/// One line saying what a rating edit did.
fn rated_text(rated: &RatedTrack) -> String {
    let stars = |count: u8| {
        if count == 1 {
            "1 star".to_owned()
        } else {
            format!("{count} stars")
        }
    };
    if rated.rating == rated.previous_rating {
        return format!(
            "Track {} \"{}\" already has {}.",
            rated.id,
            rated.title,
            stars(rated.rating)
        );
    }
    format!(
        "Rated track {} \"{}\": {}, was {}.",
        rated.id,
        rated.title,
        stars(rated.rating),
        stars(rated.previous_rating)
    )
}

/// A cell with nothing in it would shift the ones after it when read back.
fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}

fn print(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{text}");
    let _ = stdout.flush();
}
