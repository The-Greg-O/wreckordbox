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

use rbl_db::LibraryLocation;
use rbl_library::Host;
use wreckordbox_service::{
    codes, events, protection, ChangeKind, Config, LibraryChanged, Service, ServiceError,
};

use crate::state::AppState;

/// The interface's event for a window that can no longer write: the
/// message to show.
pub const SERVICE_STOPPED: &str = "service:stopped";

/// The interface's event for Library Protection turned from the command
/// line: whether it is now on.
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
    /// A client of the service turned Library Protection on or off. The
    /// interface takes it up as its own preference.
    fn protection_changed(&self, _protect_library: bool) {}
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
                    } else if event.event == protection::PROTECTION_CHANGED {
                        if let Ok(now) =
                            serde_json::from_value::<protection::Protection>(event.data)
                        {
                            front.protection_changed(now.protect_library);
                        }
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
        if let Some(hosted) = self.service.lock().take() {
            hosted.closing.store(true, Ordering::Release);
            drop(hosted);
        }
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

    fn protection_changed(&self, protect_library: bool) {
        let _ = tauri::Emitter::emit(&self.0, PROTECTION_CHANGED, protect_library);
    }
}

/// Hosts the service for the Library the app is about to read. A Library
/// that is not there yet has nothing to host; the next load tries again.
pub(crate) fn host<R: tauri::Runtime>(app: &tauri::AppHandle<R>, location: &LibraryLocation) {
    use tauri::Manager as _;
    if !location.master_db.is_file() {
        return;
    }
    match Config::from_env() {
        Ok(config) => {
            let _ = host_with(app, location, &config);
        }
        Err(error) => {
            let message = cannot_host(&error);
            tracing::error!(error = %error.message, "the window is not hosting the Library service");
            app.state::<Arc<AppState>>().core().refuse_writes(Some(message.clone()));
            Interface(app.clone()).cannot_write(&message);
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
    use tauri::Manager as _;
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

/// Library Protection as it is recorded for the Library service, or nothing
/// when nothing is. The window's preferences live in its own storage, which
/// the service cannot read; this file, in the folder the two share, is
/// what the service obeys (`wreckordbox_service::protection`).
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub fn library_protection(state: tauri::State<'_, Arc<AppState>>) -> Option<bool> {
    protection::stored(state.core().recovery_dir())
}

/// Records Library Protection for the Library service, when the DJ turns
/// it in Preferences.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a command is handed its state by value"
)]
pub fn record_library_protection(
    state: tauri::State<'_, Arc<AppState>>,
    on: bool,
) -> Result<(), crate::error::AppError> {
    protection::store(state.core().recovery_dir(), on).map_err(|error| {
        crate::error::AppError::internal(format!("could not record Library Protection: {error}"))
    })
}
