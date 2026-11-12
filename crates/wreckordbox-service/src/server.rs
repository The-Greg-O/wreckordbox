//! The service: the Library's lock, the open Library, and the socket.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rbl_db::{DbError, LibraryLocation};
use rbl_library::{Host, LibraryCore};
use serde_json::{json, Value};

use crate::client::Client;
use crate::library::OpenLibrary;
use crate::protocol::{
    codes, commands, Event, LibraryChanged, LibraryStatus, Outcome, RateTrack, Request, Response,
    ServiceError, Status, TrackQuery, PROTOCOL_VERSION,
};
use crate::Config;

/// How long a service started on demand stays up with no client, unless
/// [`IDLE_EXIT_ENV`](crate::IDLE_EXIT_ENV) says otherwise.
const SPAWNED_IDLE_EXIT: Duration = Duration::from_secs(300);

/// How long a front end waits for a service running by itself to hand the
/// Library over.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one client may leave an event unread before it is cut off. An
/// event is sent by whoever changed the Library, which may be the window; a
/// client that stopped reading must not hold it up.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);

/// `sun_path` is 108 bytes on Linux and 104 on macOS, with a terminator.
const SOCKET_PATH_MAX: usize = 100;

/// A running Library service. Dropping it stops it.
pub struct Service {
    shared: Arc<Shared>,
}

struct Shared {
    /// The open Library, and the lock that says this service has it. Both
    /// are let go, in that order, when the service stops.
    library: Mutex<Option<Owned>>,
    master_db: PathBuf,
    socket: PathBuf,
    /// Whether a front end hosts this service on its own Library core. Such
    /// a service does not stop for a client: its front end would go on
    /// writing with nothing holding the Library's lock.
    hosted: bool,
    /// A client asked a service running by itself to stop; it does once that
    /// client has its answer.
    stop_requested: AtomicBool,
    stopping: AtomicBool,
    /// Set, and signalled, once the service has stopped.
    stopped: (Mutex<bool>, Condvar),
    connections: Mutex<HashMap<u64, Arc<Connection>>>,
    /// The hosting process's own subscriptions ([`Service::events`]).
    listeners: Mutex<Vec<Sender<Event>>>,
    /// Where the Library core's changes wait to be sent to clients, off the
    /// thread that made them. Closed when the service stops.
    changes: Mutex<Option<Sender<Event>>>,
    /// When the last client left; `None` while one is connected.
    idle_since: Mutex<Option<Instant>>,
    /// The Sidecar and the separation engine (`stems.*` commands).
    stems: Arc<crate::stems::Stems>,
    /// Keeps the Ledger: told of every edit by the Library core.
    ledger: Arc<crate::ledger::Keeper>,
}

struct Owned {
    library: OpenLibrary,
    /// The kernel drops it with the process, so a crashed service never
    /// blocks the next one.
    _lock: File,
}

struct Connection {
    writer: Mutex<UnixStream>,
    subscribed: AtomicBool,
}

impl Service {
    /// Starts the one service for `library`: takes its lock, opens it, and
    /// listens on [`Config::socket_path`].
    ///
    /// Fails with [`codes::SERVICE_ALREADY_RUNNING`] when another service
    /// has the Library, and with a `library_*` code when it cannot be opened.
    pub fn start(library: LibraryLocation, config: &Config) -> Result<Self, ServiceError> {
        let recovery_dir = config.recovery_dir.clone();
        Self::begin(library, config, false, |library| {
            OpenLibrary::open(library, recovery_dir)
        })
    }

    /// Starts the one service for `library` inside a front end, on the
    /// front end's own Library core: there is one in-memory Library and one
    /// change generation, so an edit from a client and an edit from the
    /// front end are the same edit. `host` is what the front end lends an
    /// edit (its recovery and its edit history).
    ///
    /// The front end reads the Library into `core` itself, before or after
    /// this; until it has, commands answer [`codes::LIBRARY_LOADING`].
    ///
    /// A service running by itself for the same Library is asked to stop
    /// and this one takes its place, so the front end is the Library's one
    /// writer for as long as it holds the service. A service hosted by
    /// another front end does not stop: this fails with
    /// [`codes::STOP_REFUSED`], and the caller must not write.
    pub fn host(
        library: &LibraryLocation,
        config: &Config,
        core: &Arc<LibraryCore>,
        host: &Arc<dyn Host + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        let socket = config.socket_path(library);
        let deadline = Instant::now() + HANDOVER_TIMEOUT;
        loop {
            let (core, host) = (Arc::clone(core), Arc::clone(host));
            let started = Self::begin(library.clone(), config, true, |_| {
                Ok(OpenLibrary::shared(core, host))
            });
            let busy = match started {
                Err(error) if error.code == codes::SERVICE_ALREADY_RUNNING => error,
                other => return other,
            };
            // Nothing answers while the other service is still opening the
            // Library, or once it has begun to stop: ask again.
            if let Ok(mut running) = Client::connect(&socket) {
                match running.stop() {
                    Err(refused) if refused.code == codes::STOP_REFUSED => return Err(refused),
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                return Err(busy);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn begin(
        library: LibraryLocation,
        config: &Config,
        hosted: bool,
        open: impl FnOnce(LibraryLocation) -> Result<OpenLibrary, ServiceError>,
    ) -> Result<Self, ServiceError> {
        if !library.master_db.is_file() {
            return Err(ServiceError::new(
                codes::LIBRARY_MISSING,
                format!(
                    "the Library is missing: there is no {}",
                    library.master_db.display()
                ),
            ));
        }
        let socket = config.socket_path(&library);
        let library_lock = take_lock(&library.master_db, &socket)?;
        let master_db = library.master_db.clone();
        let open = open(library)?;
        let listener = listen(&config.runtime_dir, &socket)?;
        let (queue, queued) = std::sync::mpsc::channel();
        let (sidecar_events, sidecar_queued) = std::sync::mpsc::channel();
        let stems = crate::stems::Stems::from_env(&master_db, sidecar_events);
        let ledger = crate::ledger::Keeper::new(Arc::clone(&stems));

        let shared = Arc::new(Shared {
            library: Mutex::new(Some(Owned {
                library: open,
                _lock: library_lock,
            })),
            master_db: master_db.clone(),
            socket,
            hosted,
            stop_requested: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            stopped: (Mutex::new(false), Condvar::new()),
            connections: Mutex::new(HashMap::new()),
            listeners: Mutex::new(Vec::new()),
            changes: Mutex::new(Some(queue)),
            idle_since: Mutex::new(Some(Instant::now())),
            stems,
            ledger,
        });
        // What the Stems commands have to say (progress, a change to the
        // Sidecar), sent off the thread that said it. Ends with the service.
        {
            let shared = Arc::downgrade(&shared);
            std::thread::spawn(move || {
                for event in sidecar_queued {
                    let Some(shared) = shared.upgrade() else {
                        break;
                    };
                    shared.publish(&event);
                }
            });
        }
        // Every change to the Library, whoever made it: a client's command,
        // or the front end this service shares the core with. The core calls
        // this under its own lock, so the change is only queued here.
        {
            let watching = Arc::downgrade(&shared);
            if let Some(owned) = lock(&shared.library).as_ref() {
                owned.library.watch(move |change| {
                    let Some(shared) = watching.upgrade() else {
                        return false;
                    };
                    let queue = lock(&shared.changes);
                    queue.as_ref().is_some_and(|queue| {
                        queue.send(LibraryChanged::from(change).event()).is_ok()
                    })
                });
                // Every turn of Library Protection, likewise in order, on
                // the same queue. A hosting window hears of it from the core.
                let watching = Arc::downgrade(&shared);
                owned.library.watch_protection(move |state| {
                    let Some(shared) = watching.upgrade() else {
                        return false;
                    };
                    let queue = lock(&shared.changes);
                    queue.as_ref().is_some_and(|queue| {
                        queue
                            .send(Event {
                                event: crate::protection::PROTECTION_CHANGED.to_owned(),
                                data: json!(crate::protection::Protection::from(state)),
                            })
                            .is_ok()
                    })
                });
            }
        }
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                // Ends when the service stops and closes the queue.
                for event in queued {
                    shared.send_to_clients(&event);
                }
            });
        }
        if let Some(idle_exit) = config.idle_exit {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || stop_when_idle(&shared, idle_exit));
        }
        // From here on every edit through the core, a client's or the
        // hosting front end's, is recorded in the Ledger: installed, and
        // what an earlier service left settled, before any client is
        // accepted. A Ledger that cannot be settled now refuses every write
        // and every read of it, saying why, until it can.
        if let Some(owned) = lock(&shared.library).as_ref() {
            owned.library.keep_ledger(shared.ledger.clone());
        }
        if let Err(error) = shared.ledger.recover() {
            tracing::error!(%error, "the Ledger could not be brought up to date");
        }
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || accept_clients(&shared, &listener));
        }
        Ok(Self { shared })
    }

    /// What the `status` command answers.
    pub fn status(&self) -> Result<Status, ServiceError> {
        self.shared.status()
    }

    /// Sends `event` to every subscribed client and to
    /// [`events`](Self::events). A change to the Library needs no call: the
    /// service sends `library_changed` itself.
    pub fn publish(&self, event: &Event) {
        self.shared.publish(event);
    }

    /// Runs one command for the process hosting the service, without the
    /// socket: the same dispatch a client's request goes through, so there
    /// is one path to the Library. `subscribe` is a connection's command;
    /// the host calls [`events`](Self::events) instead.
    pub fn call(&self, command: &str, args: Value) -> Result<Value, ServiceError> {
        let request = Request {
            id: 0,
            command: command.to_owned(),
            args,
            run: None,
        };
        self.shared.answer(None, &request).0
    }

    /// What the process hosting the service has not seen for itself, from
    /// now on: each change a client's command (or [`call`](Self::call))
    /// made, as the `library_changed` a subscribed client is sent, and
    /// whatever is [`publish`](Self::publish)ed. A change the host makes
    /// through the Library core it shares with the service is not repeated
    /// here; the host made it and already knows. Dropping the receiver ends
    /// it.
    pub fn events(&self) -> Receiver<Event> {
        let (sender, receiver) = std::sync::mpsc::channel();
        lock(&self.shared.listeners).push(sender);
        receiver
    }

    /// Stops accepting clients and disconnects the ones there are.
    pub fn stop(&self) {
        self.shared.stop();
    }

    /// Blocks until the service has stopped: by [`stop`](Self::stop), or by
    /// itself after [`Config::idle_exit`] with no client.
    pub fn wait(self) {
        self.stopped().wait();
    }

    /// Something to wait on, from another thread, for the service to stop:
    /// how a hosting front end learns its service is gone.
    #[must_use]
    pub fn stopped(&self) -> Stopped {
        Stopped {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Waits for a [`Service`] to stop, without keeping it running.
pub struct Stopped {
    shared: Arc<Shared>,
}

impl Stopped {
    /// Blocks until the service has stopped, for whatever reason.
    pub fn wait(&self) {
        let (stopped, signal) = &self.shared.stopped;
        let mut stopped = lock(stopped);
        while !*stopped {
            stopped = signal.wait(stopped).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

/// Runs a service as the child a client started: says on standard output
/// whether it came up, as one line the starting client reads, then serves
/// until idle. Returns whether it started.
pub fn serve_spawned(mut config: Config) -> bool {
    config.idle_exit = config.idle_exit.or(Some(SPAWNED_IDLE_EXIT));
    let started = crate::locate_library().and_then(|library| Service::start(library, &config));
    let line = match &started {
        Ok(_) => json!({ "ready": true }),
        Err(error) => json!({ "error": error }),
    };
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
    match started {
        Ok(service) => {
            service.wait();
            true
        }
        Err(_) => false,
    }
}

impl Shared {
    fn status(&self) -> Result<Status, ServiceError> {
        let tracks = self.with_library(OpenLibrary::track_count)??;
        Ok(Status {
            running: true,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol: PROTOCOL_VERSION,
            library: LibraryStatus {
                path: self.master_db.clone(),
                tracks,
            },
            socket: self.socket.clone(),
        })
    }

    /// Runs `use_it` on the open Library, one command at a time.
    fn with_library<T>(&self, use_it: impl FnOnce(&OpenLibrary) -> T) -> Result<T, ServiceError> {
        match lock(&self.library).as_ref() {
            Some(owned) => Ok(use_it(&owned.library)),
            None => Err(ServiceError::new(
                codes::SERVICE_FAILED,
                "the Library service is stopping",
            )),
        }
    }

    /// Answers one request with its edits attributed in the Ledger: the
    /// result, and the Run the command's changes are in.
    fn answer(
        &self,
        connection: Option<&Connection>,
        request: &Request,
    ) -> (Result<Value, ServiceError>, Option<Value>) {
        let (result, run) = self.ledger.attributed(connection.is_some(), request, || {
            self.dispatch(connection, request)
        });
        (result, run.and_then(|run| serde_json::to_value(run).ok()))
    }

    /// The one place a command name becomes behaviour.
    fn dispatch(
        &self,
        connection: Option<&Connection>,
        request: &Request,
    ) -> Result<Value, ServiceError> {
        match request.command.as_str() {
            commands::STATUS => to_value(&self.status()?),
            commands::SUBSCRIBE => {
                let Some(connection) = connection else {
                    return Err(ServiceError::new(
                        codes::BAD_REQUEST,
                        "subscribe is for a connection; the hosting process reads Service::events",
                    ));
                };
                connection.subscribed.store(true, Ordering::Release);
                Ok(json!({}))
            }
            commands::TRACKS_LIST => {
                let query: TrackQuery = arguments(request)?;
                to_value(&self.with_library(|library| library.tracks(&query))??)
            }
            commands::TRACKS_RATE => {
                let edit: RateTrack = required_arguments(request)?;
                let (rated, changed) = self.with_library(|library| library.rate(&edit))??;
                // Clients hear of it from the core, as they hear of every
                // change; the hosting process is told here, of the changes
                // it did not make.
                self.tell_host(&changed.event());
                to_value(&rated)
            }
            commands::STOP => {
                if self.hosted || connection.is_none() {
                    return Err(ServiceError::new(
                        codes::STOP_REFUSED,
                        "the wreckordbox window is hosting the Library service; it stops when the window is closed",
                    ));
                }
                // Stopped by the connection's own thread, once this answer
                // has been sent.
                self.stop_requested.store(true, Ordering::Release);
                Ok(json!({}))
            }
            crate::protection::commands::GET => {
                to_value(&self.with_library(OpenLibrary::protection)?)
            }
            crate::protection::commands::SET => {
                // Turned through the core, which tells the clients (the
                // watcher below) and a hosting window (its own watcher) in
                // order, with the change's revision.
                let set: crate::protection::SetProtection = required_arguments(request)?;
                to_value(&self.with_library(|library| library.protect(set.protect_library))??)
            }
            reading if crate::reading::answers(reading) => self
                .with_library(|library| crate::reading::answer(library, reading, &request.args))?,
            edit if crate::mytag_edits::answers(edit) => {
                let (result, changed) = self.with_library(|library| {
                    crate::mytag_edits::answer(library, edit, &request.args)
                })??;
                // As for a rating: clients hear of it from the core, the
                // hosting process here.
                self.tell_host(&changed.event());
                Ok(result)
            }
            crate::tracks::TRACKS_GET | crate::tracks::TRACKS_QUERY => {
                self.with_library(|library| crate::tracks::answer(library, request))?
            }
            crate::native_stems::commands::STEMS_WRITE => {
                // To a Stick, not the Library: only the lookup takes it.
                crate::native_stems::answer(&self.stems, request, &|id| {
                    self.with_library(|library| crate::native_stems::parent(library, id))?
                })
            }
            stems if crate::stems::answers(stems) => {
                // Only the lookup takes the Library; a separation runs
                // with it free.
                crate::stems::answer(&self.stems, request, &|id| {
                    self.with_library(|library| crate::stems::parent(library, id))?
                })
            }
            crate::run_status::RUNS_ACTIVE => crate::run_status::active(&self.stems),
            runs if crate::ledger::answers(runs) => {
                crate::ledger::answer(&self.ledger, &self.stems, request)
            }
            crate::facts::commands::RUNS_FINISH => {
                // Not while a command admitted to the Run is still in it.
                self.ledger.finishing(&request.args, || {
                    crate::facts::answer(&self.stems, request, &|id| {
                        self.with_library(|library| crate::stems::parent(library, id))?
                    })
                })
            }
            facts if crate::facts::answers(facts) => {
                // The Sidecar's, not the Library's: the Library is only
                // read, to find the track.
                crate::facts::answer(&self.stems, request, &|id| {
                    self.with_library(|library| crate::stems::parent(library, id))?
                })
            }
            decisions if crate::decisions::answers(decisions) => {
                // The Sidecar's too: the Library is only read, to find a
                // track a Decision or its Fact is about.
                crate::decisions::answer(&self.stems, request, &|id| {
                    self.with_library(|library| crate::stems::parent(library, id))?
                })
            }
            mutants if crate::mutants::answers(mutants) => {
                // The Sidecar's, not the Library's: the Library is only
                // read, to find the entry, its playlist and its track.
                crate::mutants::answer(&self.stems, request, &|read| {
                    self.with_library(|library| read(library))?
                })
            }
            schema if crate::schema::answers(schema) => {
                // Reads the Library's MyTags and writes only beside the
                // Sidecar: the Library does not change.
                crate::schema::answer(
                    &self.stems,
                    request,
                    &|| self.with_library(crate::schema::my_tags)?,
                    &|id| self.with_library(|library| crate::stems::parent(library, id))?,
                )
            }
            intake if crate::intake::answers(intake) => {
                let (result, changed) = self.with_library(|library| {
                    crate::intake::answer(library, &self.stems, request)
                })??;
                // Only its Prep colour edit changes the Library; the
                // hosting process is told of it here, as of a rating.
                if let Some(changed) = changed {
                    self.tell_host(&changed.event());
                }
                Ok(result)
            }
            unknown => Err(ServiceError::new(
                codes::UNKNOWN_COMMAND,
                format!("unknown command: {unknown}"),
            )),
        }
    }

    fn publish(&self, event: &Event) {
        self.send_to_clients(event);
        self.tell_host(event);
    }

    fn tell_host(&self, event: &Event) {
        // A host that dropped its receiver is forgotten here.
        lock(&self.listeners).retain(|listener| listener.send(event.clone()).is_ok());
    }

    fn send_to_clients(&self, event: &Event) {
        let Ok(line) = serde_json::to_string(event) else {
            return;
        };
        let subscribers: Vec<Arc<Connection>> = lock(&self.connections)
            .values()
            .filter(|connection| connection.subscribed.load(Ordering::Acquire))
            .cloned()
            .collect();
        for connection in subscribers {
            // A client that has gone is dropped by its own reader; one that
            // stopped reading is cut off, since half a line may have gone.
            if connection.send(&line).is_err() {
                let _ = lock(&connection.writer).shutdown(std::net::Shutdown::Both);
            }
        }
    }

    fn stop(&self) {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return;
        }
        for connection in lock(&self.connections).values() {
            let _ = lock(&connection.writer).shutdown(std::net::Shutdown::Both);
        }
        // Wakes the accept loop, which then sees `stopping` and closes the
        // listener. Best effort: if the socket file has been removed the
        // loop stays asleep, accepting nobody, until the process ends.
        let _ = UnixStream::connect(&self.socket);
        // While the lock is still held, so this can only be our own socket.
        let _ = std::fs::remove_file(&self.socket);
        // A separation in progress is abandoned; nothing of it is kept.
        self.stems.cancel();
        // No more changes to send; the sending thread ends.
        drop(lock(&self.changes).take());
        // No write is begun from now on; one already begun is recorded.
        self.ledger.shut();
        // Waits for a command in progress, then closes the Library before
        // letting another service have it. The core, which a hosting front
        // end keeps, is no longer this service's to record: its edit gate
        // is taken first, so an edit the window is making is over, and
        // recorded, before the Ledger is taken off it.
        if let Some(owned) = lock(&self.library).take() {
            owned.library.let_go_of_ledger(|| self.ledger.close());
        } else {
            self.ledger.close();
        }
        *lock(&self.stopped.0) = true;
        self.stopped.1.notify_all();
    }
}

impl Connection {
    fn send(&self, line: &str) -> std::io::Result<()> {
        let mut writer = lock(&self.writer);
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")
    }
}

fn accept_clients(shared: &Arc<Shared>, listener: &UnixListener) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::Acquire) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let Ok(writer) = stream.try_clone() else {
            continue;
        };
        let _ = writer.set_write_timeout(Some(SEND_TIMEOUT));
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection {
            writer: Mutex::new(writer),
            subscribed: AtomicBool::new(false),
        });
        {
            let mut connections = lock(&shared.connections);
            connections.insert(id, Arc::clone(&connection));
            *lock(&shared.idle_since) = None;
        }
        let shared = Arc::clone(shared);
        std::thread::spawn(move || {
            serve_client(&shared, &connection, stream);
            let mut connections = lock(&shared.connections);
            connections.remove(&id);
            if connections.is_empty() {
                *lock(&shared.idle_since) = Some(Instant::now());
            }
        });
    }
}

fn serve_client(shared: &Shared, connection: &Connection, stream: UnixStream) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                let (result, run) = shared.answer(Some(connection), &request);
                Response {
                    id: request.id,
                    outcome: match result {
                        Ok(result) => Outcome::Result(result),
                        Err(error) => Outcome::Error(error),
                    },
                    run,
                }
            }
            // Nothing to answer to by id; 0 is never a client's id.
            Err(e) => Response {
                id: 0,
                outcome: Outcome::Error(ServiceError::new(
                    codes::BAD_REQUEST,
                    format!("not a request: {e}"),
                )),
                run: None,
            },
        };
        let Ok(line) = serde_json::to_string(&response) else {
            break;
        };
        if connection.send(&line).is_err() {
            break;
        }
        if shared.stop_requested.load(Ordering::Acquire) {
            shared.stop();
            break;
        }
    }
}

fn stop_when_idle(shared: &Shared, idle_exit: Duration) {
    let tick = (idle_exit / 4).clamp(Duration::from_millis(20), Duration::from_millis(500));
    while !shared.stopping.load(Ordering::Acquire) {
        std::thread::sleep(tick);
        let idle_for = lock(&shared.idle_since).map(|since| since.elapsed());
        if idle_for.is_some_and(|idle| idle >= idle_exit) {
            shared.stop();
        }
    }
}

/// Takes the Library's lock, or says which service has it.
fn take_lock(master_db: &Path, socket: &Path) -> Result<File, ServiceError> {
    let path = crate::lock_path(master_db);
    let failed = |e: &dyn std::fmt::Display| {
        ServiceError::new(
            codes::SERVICE_FAILED,
            format!(
                "could not take the Library's lock at {}: {e}",
                path.display()
            ),
        )
    };
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| failed(&e))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let mut holder = String::new();
            let _ = file.read_to_string(&mut holder);
            let holder = holder
                .lines()
                .next()
                .map(|pid| format!(" (pid {pid})"))
                .unwrap_or_default();
            return Err(ServiceError::new(
                codes::SERVICE_ALREADY_RUNNING,
                format!(
                    "a Library service is already running for {}{holder}; only one may own a Library",
                    master_db.display()
                ),
            ));
        }
        Err(TryLockError::Error(e)) => return Err(failed(&e)),
    }
    // For the message above, and for a person looking: who holds it, where.
    let note = format!("{}\n{}\n", std::process::id(), socket.display());
    file.set_len(0)
        .and_then(|()| file.rewind())
        .and_then(|()| file.write_all(note.as_bytes()))
        .map_err(|e| failed(&e))?;
    Ok(file)
}

/// Binds the socket in a directory only this user can enter.
fn listen(runtime_dir: &Path, socket: &Path) -> Result<UnixListener, ServiceError> {
    let unusable = |what: &str, e: &dyn std::fmt::Display| {
        ServiceError::new(
            codes::RUNTIME_DIR_UNUSABLE,
            format!("{what} {}: {e}", socket.display()),
        )
    };
    if socket.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(unusable(
            "the path is too long for a socket:",
            &"choose a shorter runtime directory",
        ));
    }
    private_dir(runtime_dir)
        .map_err(|e| unusable("could not make the runtime directory for", &e))?;
    // This process holds the Library's lock, so a socket already here was
    // left by a service that is gone.
    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(unusable("could not replace the stale socket", &e)),
    }
    let listener = UnixListener::bind(socket).map_err(|e| unusable("could not listen on", &e))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| unusable("could not restrict", &e))?;
    Ok(listener)
}

/// Makes `dir` if needed and closes it to every other user.
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Says which of missing, locked or damaged a failure to open is.
pub(crate) fn open_error(error: &DbError) -> ServiceError {
    use rusqlite::ErrorCode::{DatabaseBusy, DatabaseLocked};
    let locked = || {
        ServiceError::new(
            codes::LIBRARY_LOCKED,
            "the Library is locked: another program is writing to it. Close that program, or try again in a moment.",
        )
    };
    match error {
        DbError::NotInstalled(why) => ServiceError::new(
            codes::LIBRARY_MISSING,
            format!("the Library is missing: {why}"),
        ),
        DbError::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(failure.code, DatabaseBusy | DatabaseLocked) =>
        {
            locked()
        }
        // `rbl-db` folds the failure of its first read into a message, so
        // SQLite's own wording for SQLITE_BUSY is all there is to go on.
        DbError::Open(why) if why.contains("database is locked") => locked(),
        DbError::WriteRefused(why) => ServiceError::new(
            codes::LIBRARY_LOCKED,
            format!("the Library is locked: {why}"),
        ),
        other => ServiceError::new(
            codes::LIBRARY_DAMAGED,
            format!("the Library is damaged or is not a rekordbox library: {other}"),
        ),
    }
}

/// A command's arguments; leaving them out altogether means the defaults.
fn arguments<T: serde::de::DeserializeOwned + Default>(
    request: &Request,
) -> Result<T, ServiceError> {
    if request.args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

/// A command's arguments when it has ones that cannot be left out.
fn required_arguments<T: serde::de::DeserializeOwned>(
    request: &Request,
) -> Result<T, ServiceError> {
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value).map_err(|e| ServiceError::new(codes::SERVICE_FAILED, e.to_string()))
}

/// A panic in one client's thread must not take the others down with a
/// poisoned lock; nothing guarded here is left half-written.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
