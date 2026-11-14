//! Arriving while the Library's last service is still going (wbx-1.2).
//!
//! A service stops answering before it lets go of the Library's lock: it
//! closes its socket first, and one that crashed can hold the lock while
//! its threads exit and its core is written. A service started in that
//! moment does not give up at once: it waits for the explicit condition,
//! the lock let go ([`crate::library_released`]), or for the holder to
//! answer on its socket, which means a live service has the Library. The
//! wait is bounded; a holder that never lets go is named in the error.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::protocol::{codes, ServiceError};

/// Seconds a new service waits for a service that no longer answers to let
/// go of the Library, instead of [`DEFAULT_RELEASE_WAIT`].
pub const RELEASE_WAIT_ENV: &str = "WRECKORDBOX_RELEASE_WAIT_SECS";

/// How long a new service waits for a stopping one to let go of the Library.
pub const DEFAULT_RELEASE_WAIT: Duration = Duration::from_secs(10);

/// Test only (honoured under `RBXPORT_TEST=1`): a stopping service, once it
/// has closed its socket and before it lets go of the Library, creates
/// `<path>.held` and waits until `<path>` exists. How a test holds a
/// service in the state a crashing one is in.
pub const HOLD_LOCK_ENV: &str = "WRECKORDBOX_TEST_HOLD_LOCK";

/// How often the two conditions are looked at.
const PROBE: Duration = Duration::from_millis(20);

/// The longest the test hold lasts, so a test that forgets it hangs nothing.
const HOLD_LIMIT: Duration = Duration::from_secs(60);

fn release_wait() -> Duration {
    std::env::var(RELEASE_WAIT_ENV)
        .ok()
        .and_then(|secs| secs.trim().parse::<u64>().ok())
        .map_or(DEFAULT_RELEASE_WAIT, Duration::from_secs)
}

/// The [`HOLD_LOCK_ENV`] fault, called by a stopping service just before it
/// lets go of the Library. Does nothing unless a test asked for it.
pub(crate) fn hold_before_release() {
    if std::env::var("RBXPORT_TEST").as_deref() != Ok("1") {
        return;
    }
    let Some(gate) = std::env::var_os(HOLD_LOCK_ENV).map(PathBuf::from) else {
        return;
    };
    let mut held = gate.clone().into_os_string();
    held.push(".held");
    let _ = std::fs::write(&held, std::process::id().to_string());
    let deadline = Instant::now() + HOLD_LIMIT;
    while !gate.exists() && Instant::now() < deadline {
        std::thread::sleep(PROBE);
    }
}

/// Takes the Library's lock for a service starting by itself. When another
/// process holds it, waits for that holder to let go, or to answer on its
/// socket, whichever comes first, for at most [`RELEASE_WAIT_ENV`] seconds:
///
/// - let go: the lock is taken (or, when another service won it in the
///   same moment, that one is waited for in turn);
/// - answering, or restoring a backup: a live owner, so
///   [`codes::SERVICE_ALREADY_RUNNING`] at once, as before;
/// - neither in time: the same code, naming the holder that did not let go.
///
/// Never takes a lock another process holds; nothing is removed or
/// signalled. The stale socket file a crashed holder left is replaced only
/// once the lock is this process's (`server::listen`).
pub(crate) fn take_lock(master_db: &Path, socket: &Path) -> Result<File, ServiceError> {
    let patience = release_wait();
    let deadline = Instant::now() + patience;
    let mut told = false;
    loop {
        let busy = match crate::server::take_lock(master_db, socket) {
            Err(error) if error.code == codes::SERVICE_ALREADY_RUNNING => error,
            taken => return taken,
        };
        let holder = Holder::read(master_db);
        if holder.restoring() || holder.answers(socket) {
            return Err(busy);
        }
        if !told {
            // To the terminal of `serve`, or the log of a spawned service.
            eprintln!(
                "wreckordbox: the Library service{} no longer answers but still holds {}; waiting up to {} s for it to let go",
                holder.pid_text(),
                master_db.display(),
                patience.as_secs()
            );
            told = true;
        }
        loop {
            if crate::library_released(master_db) {
                break;
            }
            if holder.answers(socket) {
                return Err(busy);
            }
            if Instant::now() >= deadline {
                return Err(ServiceError::new(
                    codes::SERVICE_ALREADY_RUNNING,
                    format!(
                        "a Library service{} still holds {} but did not let go of it within {} s of no longer answering; it may be stuck exiting. Wait for it to end, or end it, and try again",
                        holder.pid_text(),
                        master_db.display(),
                        patience.as_secs()
                    ),
                ));
            }
            std::thread::sleep(PROBE);
        }
    }
}

/// Who holds the lock, from the note the holder wrote in the lock file.
struct Holder {
    pid: Option<String>,
    socket: Option<PathBuf>,
}

impl Holder {
    fn read(master_db: &Path) -> Self {
        let note = std::fs::read_to_string(crate::lock_path(master_db)).unwrap_or_default();
        let mut lines = note.lines();
        Self {
            pid: lines
                .next()
                .filter(|pid| !pid.is_empty())
                .map(str::to_owned),
            socket: lines.next().filter(|s| !s.is_empty()).map(PathBuf::from),
        }
    }

    fn restoring(&self) -> bool {
        self.socket.as_deref() == Some(Path::new(crate::backup::RESTORING))
    }

    /// Whether a service answers where the holder said it listens, or on
    /// this Library's socket here (the note may be a moment old).
    fn answers(&self, ours: &Path) -> bool {
        self.socket
            .iter()
            .map(PathBuf::as_path)
            .chain(std::iter::once(ours))
            .any(listening)
    }

    fn pid_text(&self) -> String {
        self.pid
            .as_ref()
            .map(|pid| format!(" (pid {pid})"))
            .unwrap_or_default()
    }
}

/// Whether something listens on `socket`, without ever blocking: a
/// connection queued, or refused only because the listener's backlog is
/// full (a live holder that is not accepting, wbx-1.2 review), means a
/// live listener. A blocking connect would wait for that backlog with no
/// bound at all.
fn listening(socket: &Path) -> bool {
    use socket2::{Domain, SockAddr, Socket, Type};
    let Ok(address) = SockAddr::unix(socket) else {
        return false;
    };
    let Ok(probe) = Socket::new(Domain::UNIX, Type::STREAM, None) else {
        return false;
    };
    if probe.set_nonblocking(true).is_err() {
        return false;
    }
    match probe.connect(&address) {
        Ok(()) => true,
        Err(error) => {
            error.kind() == std::io::ErrorKind::WouldBlock
                || error.raw_os_error() == Some(EINPROGRESS)
        }
    }
}

/// `connect(2)` on a non-blocking socket that will finish later (Linux).
const EINPROGRESS: i32 = 115;
