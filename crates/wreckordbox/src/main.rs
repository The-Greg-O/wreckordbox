//! The `wreckordbox` command: a thin client of the Library service.
//!
//! Every subcommand talks to the service that owns the Library, starting one
//! when none is running. Results go to standard output, as text or with
//! `--json` as JSON; errors go to standard error with a non-zero status.

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use wreckordbox_service::{Client, Config, Launcher, Service, ServiceError, Status};

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
    match cli.command {
        Commands::Status => {
            let status = connect(&config)?.status()?;
            if cli.json {
                print(&json!(status).to_string());
            } else {
                print(&status_text(&status));
            }
        }
        Commands::Serve { spawned: true } => {
            return Ok(if wreckordbox_service::serve_spawned(config) { ExitCode::SUCCESS } else { ExitCode::FAILURE });
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
        ServiceError::new(wreckordbox_service::codes::SERVICE_FAILED, format!("could not find this program: {e}"))
    })?;
    let launcher = Launcher { program, args: vec!["serve".into(), "--spawned".into()] };
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

fn print(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{text}");
    let _ = stdout.flush();
}
