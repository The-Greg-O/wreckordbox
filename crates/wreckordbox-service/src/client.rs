//! A connection to the service, and starting one on demand.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rbl_db::LibraryLocation;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::protocol::{
    codes, commands, Event, Outcome, RateTrack, RatedTrack, Request, Response, ServiceError,
    Status, TrackList, TrackQuery,
};
use crate::{Config, IDLE_EXIT_ENV, RUNTIME_DIR_ENV};

/// How long a started service has to answer on its socket.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the `wreckordbox` command waits for a window that is still
/// reading the Library before it gives up with [`codes::LIBRARY_LOADING`],
/// unless [`LOADING_WAIT_ENV`](crate::LOADING_WAIT_ENV) says otherwise.
pub const DEFAULT_LOADING_WAIT: Duration = Duration::from_secs(15);

/// How often a command is asked again while the Library is being read.
pub const LOADING_RETRY: Duration = Duration::from_millis(200);

/// The program that runs a service when none is running: it must call
/// [`serve_spawned`](crate::serve_spawned) when run with `args`.
#[derive(Debug, Clone)]
pub struct Launcher {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// One connection to a Library service.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    /// A line read so far when a wait for an event timed out part-way.
    partial: String,
    /// Events that arrived while waiting for a response.
    events: VecDeque<Event>,
    /// How long a command is asked again while the answer is
    /// [`codes::LIBRARY_LOADING`].
    loading_wait: Duration,
}

impl Client {
    /// Connects to the service listening on `socket`
    /// ([`Config::socket_path`]); [`codes::SERVICE_NOT_RUNNING`] when none is.
    pub fn connect(socket: &Path) -> Result<Self, ServiceError> {
        let stream = UnixStream::connect(socket).map_err(|e| {
            ServiceError::new(
                codes::SERVICE_NOT_RUNNING,
                format!("no Library service is running at {}: {e}", socket.display()),
            )
        })?;
        let writer = stream.try_clone().map_err(|e| lost(&e))?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
            partial: String::new(),
            events: VecDeque::new(),
            loading_wait: Duration::ZERO,
        })
    }

    /// Connects to the service for `library`, starting one with `launcher`
    /// when none is running and waiting for its socket.
    pub fn connect_or_start(
        config: &Config,
        library: &LibraryLocation,
        launcher: &Launcher,
    ) -> Result<Self, ServiceError> {
        let socket = config.socket_path(library);
        if let Ok(client) = Self::connect(&socket) {
            return Ok(client);
        }
        match start(config, launcher) {
            Ok(()) => {}
            // Another client started one in the same moment; use that.
            Err(error) if error.code == codes::SERVICE_ALREADY_RUNNING => {}
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match Self::connect(&socket) {
                Ok(client) => return Ok(client),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    /// The service and the Library it has open.
    pub fn status(&mut self) -> Result<Status, ServiceError> {
        self.call(commands::STATUS, json!({}))
    }

    /// Tracks in the Library, as the window lists and searches them.
    pub fn tracks(&mut self, query: &TrackQuery) -> Result<TrackList, ServiceError> {
        self.call(commands::TRACKS_LIST, json!(query))
    }

    /// Sets a track's rating, 0 to 5 stars, by its identifier. Every
    /// subscribed client is told the Library changed.
    pub fn rate_track(&mut self, id: &str, rating: u8) -> Result<RatedTrack, ServiceError> {
        let edit = RateTrack {
            id: id.to_owned(),
            rating,
        };
        self.call(commands::TRACKS_RATE, json!(edit))
    }

    /// Asks a service running by itself to stop and let go of the Library.
    /// A service the window hosts answers [`codes::STOP_REFUSED`].
    pub fn stop(&mut self) -> Result<(), ServiceError> {
        self.call::<Value>(commands::STOP, json!({})).map(|_| ())
    }

    /// Asks the service to send this connection its events; read them with
    /// [`next_event`](Self::next_event).
    pub fn subscribe(&mut self) -> Result<(), ServiceError> {
        self.call::<Value>(commands::SUBSCRIBE, json!({}))
            .map(|_| ())
    }

    /// Has every command wait, for up to `wait`, for a window that is still
    /// reading the Library: while the answer is [`codes::LIBRARY_LOADING`]
    /// the command is asked again every [`LOADING_RETRY`], and that error
    /// is the answer only once `wait` has passed. Nothing has been done by
    /// a command answered so, so asking again cannot do it twice.
    ///
    /// A new connection does not wait at all; the `wreckordbox` command
    /// waits [`DEFAULT_LOADING_WAIT`].
    pub fn wait_while_loading(&mut self, wait: Duration) {
        self.loading_wait = wait;
    }

    /// Sends one command and waits for its answer. Events that arrive
    /// meanwhile are kept for [`next_event`](Self::next_event).
    pub fn call<T: DeserializeOwned>(
        &mut self,
        command: &str,
        args: Value,
    ) -> Result<T, ServiceError> {
        let mut kept = Vec::new();
        let answer = self.call_watching(command, args, |event| kept.push(event));
        self.events.extend(kept);
        answer
    }

    /// Sends one command and waits for its answer, handing each event that
    /// arrives meanwhile to `on_event` as it arrives: how a client follows
    /// a long command's progress. [`subscribe`](Self::subscribe) first.
    ///
    /// While the answer is [`codes::LIBRARY_LOADING`] the command is asked
    /// again, for as long as [`wait_while_loading`](Self::wait_while_loading)
    /// allows.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "callers build the arguments for the call and hand them over"
    )]
    pub fn call_watching<T: DeserializeOwned>(
        &mut self,
        command: &str,
        args: Value,
        mut on_event: impl FnMut(Event),
    ) -> Result<T, ServiceError> {
        let deadline = Instant::now() + self.loading_wait;
        loop {
            match self.call_once(command, &args, &mut on_event) {
                Err(error) if error.code == codes::LIBRARY_LOADING => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(error);
                    }
                    std::thread::sleep(left.min(LOADING_RETRY));
                }
                answer => return answer,
            }
        }
    }

    fn call_once<T: DeserializeOwned>(
        &mut self,
        command: &str,
        args: &Value,
        on_event: &mut dyn FnMut(Event),
    ) -> Result<T, ServiceError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            id,
            command: command.to_owned(),
            args: args.clone(),
        };
        let mut line = serde_json::to_string(&request).map_err(|e| lost(&e))?;
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .map_err(|e| lost(&e))?;
        loop {
            match self.read_message(None)? {
                Some(Message::Response(response)) if response.id == id => {
                    return match response.outcome {
                        Outcome::Result(result) => serde_json::from_value(result).map_err(|e| {
                            ServiceError::new(
                                codes::SERVICE_FAILED,
                                format!("unexpected answer to {command}: {e}"),
                            )
                        }),
                        Outcome::Error(error) => Err(error),
                    };
                }
                Some(Message::Response(_)) | None => {}
                Some(Message::Event(event)) => on_event(event),
            }
        }
    }

    /// The next event, waiting up to `timeout` for one. `None` when none
    /// came in time.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Event>, ServiceError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.read_message(Some(left))? {
                Some(Message::Event(event)) => return Ok(Some(event)),
                Some(Message::Response(_)) => {}
                None => return Ok(None),
            }
        }
    }

    /// One message from the service; `None` when `timeout` passed first.
    fn read_message(&mut self, timeout: Option<Duration>) -> Result<Option<Message>, ServiceError> {
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(|e| lost(&e))?;
        match self.reader.read_line(&mut self.partial) {
            Ok(0) => Err(lost(&"it closed the connection")),
            Ok(_) => {
                let line = std::mem::take(&mut self.partial);
                let value: Value = serde_json::from_str(&line).map_err(|e| lost(&e))?;
                let message = if value.get("event").is_some() {
                    serde_json::from_value(value).map(Message::Event)
                } else {
                    serde_json::from_value(value).map(Message::Response)
                };
                message.map(Some).map_err(|e| lost(&e))
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(lost(&e)),
        }
    }
}

enum Message {
    Response(Response),
    Event(Event),
}

fn lost(why: &dyn std::fmt::Display) -> ServiceError {
    ServiceError::new(
        codes::SERVICE_FAILED,
        format!("lost the Library service: {why}"),
    )
}

/// Runs the launcher as a service of its own, detached from this process,
/// and waits for it to say whether it came up.
fn start(config: &Config, launcher: &Launcher) -> Result<(), ServiceError> {
    let failed = |why: &dyn std::fmt::Display| {
        ServiceError::new(
            codes::SERVICE_FAILED,
            format!("could not start the Library service: {why}"),
        )
    };
    crate::server::private_dir(&config.runtime_dir).map_err(|e| {
        ServiceError::new(
            codes::RUNTIME_DIR_UNUSABLE,
            format!(
                "could not make the runtime directory {}: {e}",
                config.runtime_dir.display()
            ),
        )
    })?;
    let log_path = config.runtime_dir.join("service.log");
    let log = std::fs::File::options()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| failed(&e))?;

    let mut command = Command::new(&launcher.program);
    command
        .args(&launcher.args)
        .env(RUNTIME_DIR_ENV, &config.runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(log)
        // Its own process group: Ctrl-C in the terminal that ran the client
        // must not stop a service other clients are using.
        .process_group(0);
    if let Some(idle_exit) = config.idle_exit {
        command.env(IDLE_EXIT_ENV, idle_exit.as_secs().max(1).to_string());
    }
    let mut child = command.spawn().map_err(|e| failed(&e))?;
    let Some(stdout) = child.stdout.take() else {
        return Err(failed(&"it has no standard output"));
    };

    // One line, then the service leaves its standard output alone.
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .map_err(|e| failed(&e))?;
    let said: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    if said.get("ready").is_some() {
        return Ok(());
    }
    let _ = child.wait();
    Err(said
        .get("error")
        .and_then(|error| serde_json::from_value::<ServiceError>(error.clone()).ok())
        .unwrap_or_else(|| {
            failed(&format_args!(
                "it stopped without saying why; see {}",
                log_path.display()
            ))
        }))
}
