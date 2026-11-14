//! The window as the host of the Library service (ADR 0002).
//!
//! While the window is open it is the Library's one writer, and the
//! `wreckordbox` command works on the window's own Library: the window
//! holds the service's lock and socket, and the service runs on the
//! window's [`rbl_library::LibraryCore`], so there is one index, one edit
//! path and one generation whichever side makes a change.
//!
//! - A service already running by itself (started by a command) is asked to
//!   stop and the window takes its place. The window's other commands still
//!   write in this process, so being a client of another process's service
//!   would make two writers.
//! - A change a client makes reaches the interface as the event a change of
//!   the window's own does (`library:changed`, `tag-list:changed`).
//! - If the window cannot be the host, or its service stops while the
//!   window is open, the window stops writing and says why.
//!
//! Lock order: nothing here is called with the core's edit gate or lock
//! held. A client's command holds the service's own lock and then takes the
//! edit gate, so taking them the other way round (starting, stopping or
//! calling the service from inside an edit) could deadlock.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use rbl_db::LibraryLocation;
use rbl_library::Host;
use rbl_library::protection::ProtectionState;
use tauri::Manager as _;
use wreckordbox_service::{codes, events, ChangeKind, Config, LibraryChanged, Service, ServiceError};

use crate::error::AppResult;

use crate::state::AppState;

/// The interface's event for a window that can no longer write: the
/// message to show.
pub const SERVICE_STOPPED: &str = "service:stopped";

/// The interface's event for a Fact changed in the Sidecar:
/// `{ trackId, kind }` (wbx-65).
pub const SIDECAR_CHANGED: &str = "sidecar:changed";

/// The interface's event for a Stick load's progress (wbx-72): the
/// service's `stick_load_progress` as it is, `{ stick, stage, done, total,
/// track }`. Sent for every step; the interface coalesces a burst.
pub const STICK_LOAD_PROGRESS: &str = "stick-load:progress";

/// The interface's event for every turn of Library Protection, by
/// whoever: a [`ProtectionDto`]. Sent in order; one with a lower revision
/// than one already applied is stale.
pub const PROTECTION_CHANGED: &str = "library-protection:changed";

/// What the DJ is told when the window's service stops under it.
pub const STOPPED_MESSAGE: &str =
    "The Library service has stopped, so changes cannot be saved. Restart wreckordbox to carry on editing.";

/// What the hosting window is told. The app emits these to the interface;
/// a test collects them.
pub trait FrontEnd: Send + Sync + 'static {
    /// A client of the service changed the Library.
    fn library_changed(&self, change: LibraryChanged);
    /// The window is not, or is no longer, the Library's writer; `message`
    /// is for the DJ.
    fn cannot_write(&self, message: &str);
    /// A Fact of `kind` on `track` changed in the Sidecar (a client's
    /// `facts.record`, a tag set from Intake): what follows the Sidecar
    /// reads again (wbx-65).
    fn sidecar_changed(&self, _track: &str, _kind: &str) {}
    /// A Stick load moved on (`stick_load_progress`, wbx-72).
    fn stick_load_progress(&self, _progress: serde_json::Value) {}
}

/// The service the window is hosting, and for which Library.
pub(crate) struct Hosted {
    service: Service,
    master_db: PathBuf,
    /// Set before the window stops the service itself, so that stop is not
    /// reported as a failure.
    closing: Arc<AtomicBool>,
}

/// What an edit made through the service borrows from the window: its
/// recovery of interrupted analysis-file edits and its edit history. Weak,
/// so the service the state holds does not keep the state alive.
struct WindowHost(Weak<AppState>);

impl Host for WindowHost {
    fn recover_interrupted_files(&self, location: &LibraryLocation) -> Result<(), String> {
        match self.0.upgrade() {
            Some(state) => state.recover_interrupted_files(location),
            None => Err("wreckordbox is closing.".to_owned()),
        }
    }

    fn history_changed(&self) {
        if let Some(state) = self.0.upgrade() {
            state.history_changed();
        }
    }

    fn library_replacing(&self, next: &rbl_index::Library) {
        if let Some(state) = self.0.upgrade() {
            state.library_replacing(next);
        }
    }

    fn library_replaced(&self, another_library: bool) {
        if let Some(state) = self.0.upgrade() {
            state.library_replaced(another_library);
        }
    }
}

impl AppState {
    /// Makes this window the host of the service for the Library at
    /// `location`, on the window's own Library core. Call it before the
    /// Library is read, and again whenever the window turns to another
    /// Library; hosting the Library it already hosts does nothing.
    ///
    /// On failure nothing can be written through this window until a later
    /// call succeeds, and `front` is told why.
    ///
    /// Blocking: a service running by itself is given a few seconds to stop.
    /// Never call it with the edit gate held.
    pub fn host_service(
        self: &Arc<Self>,
        location: &LibraryLocation,
        config: &Config,
        front: Arc<dyn FrontEnd>,
    ) -> Result<(), ServiceError> {
        let mut slot = self.service.lock();
        let master_db = location.master_db.canonicalize().unwrap_or_else(|_| location.master_db.clone());
        if slot.as_ref().is_some_and(|hosted| hosted.master_db == master_db) {
            return Ok(());
        }
        // Another Library: let go of the one before.
        if let Some(previous) = slot.take() {
            previous.closing.store(true, Ordering::Release);
            drop(previous);
        }
        let core = self.shared_core();
        let host: Arc<dyn Host + Send + Sync> = Arc::new(WindowHost(Arc::downgrade(self)));
        let service = match Service::host(location, config, &core, &host) {
            Ok(service) => service,
            Err(error) => {
                let message = cannot_host(&error);
                tracing::error!(code = %error.code, error = %error.message, "the window is not hosting the Library service");
                core.refuse_writes(Some(message.clone()));
                front.cannot_write(&message);
                return Err(error);
            }
        };
        core.refuse_writes(None);
        tracing::info!(library = %master_db.display(), "hosting the Library service");

        // What clients change, told to the interface. Ends with the service.
        let changes = service.events();
        {
            let front = Arc::clone(&front);
            std::thread::spawn(move || {
                for event in changes {
                    if event.event == events::LIBRARY_CHANGED {
                        if let Ok(change) = serde_json::from_value::<LibraryChanged>(event.data) {
                            front.library_changed(change);
                        }
                    } else if event.event == wreckordbox_service::stems::events::SIDECAR_CHANGED {
                        let field = |name: &str| event.data.get(name).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
                        front.sidecar_changed(&field("track_id"), &field("kind"));
                    } else if event.event == wreckordbox_service::stick_load::events::STICK_LOAD_PROGRESS {
                        front.stick_load_progress(event.data);
                    }
                }
            });
        }
        // The service going away while the window is open.
        let closing = Arc::new(AtomicBool::new(false));
        {
            let (stopped, closing) = (service.stopped(), Arc::clone(&closing));
            std::thread::spawn(move || {
                stopped.wait();
                if !closing.load(Ordering::Acquire) {
                    tracing::error!("the Library service stopped while the window is open");
                    core.refuse_writes(Some(STOPPED_MESSAGE.to_owned()));
                    front.cannot_write(STOPPED_MESSAGE);
                }
            });
        }
        *slot = Some(Hosted { service, master_db, closing });
        Ok(())
    }

    /// Stops the service this window hosts, as the window closes. Never
    /// call it with the edit gate held.
    pub fn stop_service(&self) {
        // Taken out first and the hosting lock let go, so stopping (which
        // waits for a running Stick load to stop where it safely can and
        // send its report) never holds up anything else that asks for it.
        // A drain (wbx-71): completes only once every Stick load command has
        // kept its report; a second stop while one runs waits for it.
        self.shutdown.drain(|| {
            let hosted = self.service.lock().take();
            if let Some(hosted) = hosted {
                hosted.closing.store(true, Ordering::Release);
                drop(hosted);
            }
        });
    }

    /// A handle to the service this window hosts, to call without holding
    /// the hosting lock (a Stick load runs for minutes; closing the window
    /// must reach it). `None` when the window hosts no service.
    pub fn service_caller(&self) -> Option<wreckordbox_service::Caller> {
        self.service.lock().as_ref().map(|hosted| hosted.service.caller())
    }

    /// Runs `use_it` on the service this window hosts, when it hosts one.
    /// Never call it with the edit gate held.
    pub fn with_service<T>(&self, use_it: impl FnOnce(&Service) -> T) -> Option<T> {
        self.service.lock().as_ref().map(|hosted| use_it(&hosted.service))
    }

    /// Why this window cannot change the Library, when it is not the
    /// Library's writer. Kept for a window that asks after the event went.
    pub fn service_problem(&self) -> Option<String> {
        self.core().write_refusal()
    }
}

/// Why the window could not become the host, for the DJ.
fn cannot_host(error: &ServiceError) -> String {
    if error.code == codes::STOP_REFUSED {
        "Another wreckordbox window has this Library open, so this window cannot change it. Close the other window, then restart this one.".to_owned()
    } else if error.code == codes::SERVICE_ALREADY_RUNNING {
        // It holds the Library's lock and did not answer on this socket.
        "Another wreckordbox service has this Library open and did not stop, so this window cannot change it. Stop it, then restart this window.".to_owned()
    } else {
        format!("The Library service could not start, so this window cannot change the Library: {}", error.message)
    }
}

/// The app as the hosting window: events to the interface.
struct Interface<R: tauri::Runtime>(tauri::AppHandle<R>);

impl<R: tauri::Runtime> FrontEnd for Interface<R> {
    fn library_changed(&self, change: LibraryChanged) {
        // The events the window's own edits send (`commands::ChangeEvent`),
        // so the interface cannot tell a client's change from its own.
        let event = match change.kind {
            ChangeKind::Library => "library:changed",
            ChangeKind::TagList => "tag-list:changed",
        };
        let _ = tauri::Emitter::emit(&self.0, event, change.generation);
    }

    fn cannot_write(&self, message: &str) {
        let _ = tauri::Emitter::emit(&self.0, SERVICE_STOPPED, message);
    }

    fn sidecar_changed(&self, track: &str, kind: &str) {
        let _ = tauri::Emitter::emit(&self.0, SIDECAR_CHANGED, serde_json::json!({ "trackId": track, "kind": kind }));
    }

    fn stick_load_progress(&self, progress: serde_json::Value) {
        let _ = tauri::Emitter::emit(&self.0, STICK_LOAD_PROGRESS, progress);
    }
}

/// How long a window that starts while a backup is being restored waits
/// for the restore to finish before it gives up.
pub const RESTORE_WAIT: Duration = Duration::from_secs(30 * 60);

/// What a window may do with the Library it is about to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    /// It hosts the service: it owns the Library, any interrupted restore
    /// has been recovered under its lock, and it may write.
    Owner,
    /// Another window or service owns the Library and nothing is left to
    /// recover: it may read, and cannot write.
    Reader,
    /// It must not read the Library: a restore is running, or one stopped
    /// midway and this window does not own the Library to recover it.
    Refused(String),
}

/// Hosts the service for the Library the app is about to read, using the
/// environment's configuration. See [`own_library`].
pub(crate) fn host<R: tauri::Runtime>(app: &tauri::AppHandle<R>, location: &LibraryLocation) -> Ownership {
    match Config::from_env() {
        Ok(config) => own_library(app, location, &config, RESTORE_WAIT),
        Err(error) => {
            let message = cannot_host(&error);
            tracing::error!(error = %error.message, "the window is not hosting the Library service");
            app.state::<Arc<AppState>>().core().refuse_writes(Some(message.clone()));
            Interface(app.clone()).cannot_write(&message);
            if rbl_backup::journal::pending(app.state::<Arc<AppState>>().backup_dir()) {
                Ownership::Refused(UNFINISHED.to_owned())
            } else {
                Ownership::Reader
            }
        }
    }
}

const UNFINISHED: &str = "A restore of the Library is unfinished, and another program owns the Library. Quit it, then open wreckordbox again.";
const RESTORING: &str = "A backup is being restored into the Library. Open wreckordbox again when the restore has finished.";

/// Makes the window the Library's owner before anything reads it: hosts
/// the service, which recovers an interrupted restore under the Library's
/// lock. Never recovers anything without that lock.
///
/// While a restore owns the Library the window waits, up to `wait`, then
/// hosts; past it, it is refused. When another window or service owns the
/// Library it may only read, and only if no restore is left to recover.
/// A Library whose `master.db` is missing with no restore to recover has
/// nothing to host; the next load tries again.
pub fn own_library<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    location: &LibraryLocation,
    config: &Config,
    wait: Duration,
) -> Ownership {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if wreckordbox_service::backup::restoring(location) {
            if std::time::Instant::now() >= deadline {
                return Ownership::Refused(RESTORING.to_owned());
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let pending = rbl_backup::journal::pending(&config.recovery_dir);
        if !location.master_db.is_file() && !pending {
            return Ownership::Reader;
        }
        match host_with(app, location, config) {
            Ok(()) => return Ownership::Owner,
            // A restore took the Library between the look and the try.
            Err(_) if wreckordbox_service::backup::restoring(location) => {}
            Err(_) if rbl_backup::journal::pending(&config.recovery_dir) => {
                return Ownership::Refused(UNFINISHED.to_owned());
            }
            Err(_) => return Ownership::Reader,
        }
    }
}

/// [`AppState::host_service`] for the app's state, with the app's interface
/// as the front end: a client's change is emitted as the window's own
/// change events, and `service:stopped` says when the window cannot write.
pub fn host_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    location: &LibraryLocation,
    config: &Config,
) -> Result<(), ServiceError> {
    let state = Arc::clone(app.state::<Arc<AppState>>().inner());
    // The window's service lasts as long as the window, clients or not.
    let config = Config { idle_exit: None, ..config.clone() };
    state.host_service(location, &config, Arc::new(Interface(app.clone())))
}

/// Why this window cannot change the Library, or nothing when it can: what
/// `service:stopped` said, for a window that was not listening yet.
#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "a command is handed its state by value")]
pub fn service_problem(state: tauri::State<'_, Arc<AppState>>) -> Option<String> {
    state.service_problem()
}

/// Library Protection as the window is told it. The Library core owns the
/// switch (`rbl_library::protection`); the window's preference only shows
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionDto {
    pub protect_library: bool,
    /// Higher for every later change.
    pub revision: u64,
}

impl From<ProtectionState> for ProtectionDto {
    fn from(state: ProtectionState) -> Self {
        Self { protect_library: state.protect_library, revision: state.revision }
    }
}

/// Tells the interface of every turn of Library Protection, whoever made
/// it (Preferences, the command line, a script), in order, as
/// [`PROTECTION_CHANGED`]. Call once, when the app starts.
pub fn announce_protection<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let (send, turns) = std::sync::mpsc::channel::<ProtectionState>();
    // The core calls this under its edit gate: only queue.
    app.state::<Arc<AppState>>().core().watch_protection(move |state| send.send(state).is_ok());
    let app = app.clone();
    std::thread::spawn(move || {
        for state in turns {
            let _ = tauri::Emitter::emit(&app, PROTECTION_CHANGED, ProtectionDto::from(state));
        }
    });
}

/// Library Protection as it stands.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub fn library_protection(state: tauri::State<'_, Arc<AppState>>) -> ProtectionDto {
    state.core().protection().into()
}

/// The window's stored preference, taken up as Library Protection only the
/// first time, while nothing has ever been recorded; answers what stands.
#[tauri::command]
pub async fn adopt_library_protection(
    state: tauri::State<'_, Arc<AppState>>,
    window_preference: bool,
) -> AppResult<ProtectionDto> {
    let state = Arc::clone(&state);
    crate::commands::blocking("adopt_library_protection", move || {
        state.as_protection_writer(|core| core.adopt_protection(window_preference)).map_or_else(
            // Not the writer: what stands, taken up from nobody.
            |_| Ok(state.core().protection().into()),
            |adopted| adopted.map(ProtectionDto::from).map_err(|error| not_recorded(&error)),
        )
    })
    .await
}

/// Turns Library Protection on or off, as the DJ does in Preferences.
/// Answers once it is recorded; an error when it could not be.
#[tauri::command]
pub async fn set_library_protection(
    state: tauri::State<'_, Arc<AppState>>,
    protect_library: bool,
) -> AppResult<ProtectionDto> {
    let state = Arc::clone(&state);
    crate::commands::blocking("set_library_protection", move || {
        state
            .as_protection_writer(|core| core.set_protection(protect_library))
            .map_err(|reason| not_the_writer(&reason))?
            .map(|turned| turned.now.into())
            .map_err(|error| not_recorded(&error))
    })
    .await
}

impl AppState {
    /// Runs `turn` on this window's core only while this window may be the
    /// Library's writer: holding the hosting slot, so hosting is not part-way
    /// through or changing meanwhile. A window that has found it is not the
    /// writer (another window or service has the Library) gets `Err` with
    /// why, and `turn` is not run: a turn through its own core would not take
    /// the writer's edit gate, and the writer's interface would not hear it.
    fn as_protection_writer<T>(&self, turn: impl FnOnce(&rbl_library::LibraryCore) -> T) -> Result<T, String> {
        let _hosting = self.service.lock();
        if let Some(reason) = self.core().write_refusal() {
            return Err(reason);
        }
        Ok(turn(self.core()))
    }
}

/// What a window that is not the Library's writer is told when the DJ turns
/// Library Protection in it.
fn not_the_writer(reason: &str) -> crate::error::AppError {
    crate::error::AppError::new(
        crate::error::ErrorKind::ReadOnly,
        "This window cannot turn Library Protection: it is not the window changing this Library. \
         Turn it in that window, or with `wreckordbox protection on` or `wreckordbox protection off`.",
    )
    .with_detail(reason.to_owned())
}

/// What the DJ is told when the switch could not be saved: said plainly,
/// since the interface shows it beside the switch.
fn not_recorded(error: &std::io::Error) -> crate::error::AppError {
    tracing::error!(%error, "Library Protection could not be saved");
    crate::error::AppError::new(
        crate::error::ErrorKind::ReadOnly,
        format!("Library Protection could not be saved: {error}"),
    )
}
