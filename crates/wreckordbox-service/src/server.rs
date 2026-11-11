//! The service: the Library's lock, the open Library, and the socket.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rbl_db::{DbError, LibraryLocation};
use serde_json::{json, Value};

use crate::protocol::{
    codes, commands, Event, LibraryStatus, Outcome, Request, Response, ServiceError, Status, TrackQuery,
    PROTOCOL_VERSION,
};
use crate::library::OpenLibrary;
use crate::Config;

/// How long a service started on demand stays up with no client, unless
/// [`IDLE_EXIT_ENV`](crate::IDLE_EXIT_ENV) says otherwise.
const SPAWNED_IDLE_EXIT: Duration = Duration::from_secs(300);

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
    stopping: AtomicBool,
    /// Set, and signalled, once the service has stopped.
    stopped: (Mutex<bool>, Condvar),
    connections: Mutex<HashMap<u64, Arc<Connection>>>,
    /// When the last client left; `None` while one is connected.
    idle_since: Mutex<Option<Instant>>,
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
        if !library.master_db.is_file() {
            return Err(ServiceError::new(
                codes::LIBRARY_MISSING,
                format!("the Library is missing: there is no {}", library.master_db.display()),
            ));
        }
        let socket = config.socket_path(&library);
        let lock = take_lock(&library.master_db, &socket)?;
        let master_db = library.master_db.clone();
        let open = OpenLibrary::open(library)?;
        let listener = listen(&config.runtime_dir, &socket)?;

        let shared = Arc::new(Shared {
            library: Mutex::new(Some(Owned { library: open, _lock: lock })),
            master_db,
            socket,
            stopping: AtomicBool::new(false),
            stopped: (Mutex::new(false), Condvar::new()),
            connections: Mutex::new(HashMap::new()),
            idle_since: Mutex::new(Some(Instant::now())),
        });
        if let Some(idle_exit) = config.idle_exit {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || stop_when_idle(&shared, idle_exit));
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

    /// Sends `event` to every subscribed client.
    pub fn publish(&self, event: &Event) {
        self.shared.publish(event);
    }

    /// Stops accepting clients and disconnects the ones there are.
    pub fn stop(&self) {
        self.shared.stop();
    }

    /// Blocks until the service has stopped: by [`stop`](Self::stop), or by
    /// itself after [`Config::idle_exit`] with no client.
    pub fn wait(self) {
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
            library: LibraryStatus { path: self.master_db.clone(), tracks },
            socket: self.socket.clone(),
        })
    }

    /// Runs `use_it` on the open Library, one command at a time.
    fn with_library<T>(&self, use_it: impl FnOnce(&OpenLibrary) -> T) -> Result<T, ServiceError> {
        match lock(&self.library).as_ref() {
            Some(owned) => Ok(use_it(&owned.library)),
            None => Err(ServiceError::new(codes::SERVICE_FAILED, "the Library service is stopping")),
        }
    }

    /// The one place a command name becomes behaviour.
    fn dispatch(&self, connection: &Connection, request: &Request) -> Result<Value, ServiceError> {
        match request.command.as_str() {
            commands::STATUS => to_value(&self.status()?),
            commands::SUBSCRIBE => {
                connection.subscribed.store(true, Ordering::Release);
                Ok(json!({}))
            }
            commands::TRACKS_LIST => {
                let query: TrackQuery = arguments(request)?;
                to_value(&self.with_library(|library| library.tracks(&query))?)
            }
            unknown => Err(ServiceError::new(codes::UNKNOWN_COMMAND, format!("unknown command: {unknown}"))),
        }
    }

    fn publish(&self, event: &Event) {
        let Ok(line) = serde_json::to_string(event) else { return };
        let subscribers: Vec<Arc<Connection>> = lock(&self.connections)
            .values()
            .filter(|connection| connection.subscribed.load(Ordering::Acquire))
            .cloned()
            .collect();
        for connection in subscribers {
            // A client that has gone is dropped by its own reader.
            let _ = connection.send(&line);
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
        // Waits for a command in progress, then closes the Library before
        // letting another service have it.
        drop(lock(&self.library).take());
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
        let Ok(writer) = stream.try_clone() else { continue };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection { writer: Mutex::new(writer), subscribed: AtomicBool::new(false) });
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
            Ok(request) => Response {
                id: request.id,
                outcome: match shared.dispatch(connection, &request) {
                    Ok(result) => Outcome::Result(result),
                    Err(error) => Outcome::Error(error),
                },
            },
            // Nothing to answer to by id; 0 is never a client's id.
            Err(e) => Response {
                id: 0,
                outcome: Outcome::Error(ServiceError::new(codes::BAD_REQUEST, format!("not a request: {e}"))),
            },
        };
        let Ok(line) = serde_json::to_string(&response) else { break };
        if connection.send(&line).is_err() {
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
        ServiceError::new(codes::SERVICE_FAILED, format!("could not take the Library's lock at {}: {e}", path.display()))
    };
    let mut file =
        File::options().read(true).write(true).create(true).truncate(false).open(&path).map_err(|e| failed(&e))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let mut holder = String::new();
            let _ = file.read_to_string(&mut holder);
            let holder = holder.lines().next().map(|pid| format!(" (pid {pid})")).unwrap_or_default();
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
    file.set_len(0).and_then(|()| file.rewind()).and_then(|()| file.write_all(note.as_bytes())).map_err(|e| failed(&e))?;
    Ok(file)
}

/// Binds the socket in a directory only this user can enter.
fn listen(runtime_dir: &Path, socket: &Path) -> Result<UnixListener, ServiceError> {
    let unusable = |what: &str, e: &dyn std::fmt::Display| {
        ServiceError::new(codes::RUNTIME_DIR_UNUSABLE, format!("{what} {}: {e}", socket.display()))
    };
    if socket.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(unusable("the path is too long for a socket:", &"choose a shorter runtime directory"));
    }
    private_dir(runtime_dir).map_err(|e| unusable("could not make the runtime directory for", &e))?;
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
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
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
        DbError::NotInstalled(why) => {
            ServiceError::new(codes::LIBRARY_MISSING, format!("the Library is missing: {why}"))
        }
        DbError::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
            if matches!(failure.code, DatabaseBusy | DatabaseLocked) =>
        {
            locked()
        }
        // `rbl-db` folds the failure of its first read into a message, so
        // SQLite's own wording for SQLITE_BUSY is all there is to go on.
        DbError::Open(why) if why.contains("database is locked") => locked(),
        DbError::WriteRefused(why) => {
            ServiceError::new(codes::LIBRARY_LOCKED, format!("the Library is locked: {why}"))
        }
        other => ServiceError::new(
            codes::LIBRARY_DAMAGED,
            format!("the Library is damaged or is not a rekordbox library: {other}"),
        ),
    }
}

/// A command's arguments; leaving them out altogether means the defaults.
fn arguments<T: serde::de::DeserializeOwned + Default>(request: &Request) -> Result<T, ServiceError> {
    if request.args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(codes::BAD_REQUEST, format!("bad arguments to {}: {e}", request.command))
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
