//! Stems: separating a track and keeping the result in the Sidecar.
//!
//! The service owns the Sidecar as it owns the Library, whether it runs by
//! itself or the window hosts it: [`Stems`] opens it on the first command
//! that needs it and is the only thing that writes to it. Nothing here
//! writes to the Library, so no `library_changed` is ever sent; a change to
//! the Sidecar is announced as [`events::SIDECAR_CHANGED`].
//!
//! A separation is long (the real engine takes about four and a half times
//! the track's length), so it does not run inside the command that asked
//! for it:
//!
//! - it runs on a thread of its own, holding no lock on the Library, so
//!   every other command is answered meanwhile;
//! - the command that asked waits for it and is answered when it is done,
//!   with [`events::STEMS_PROGRESS`] sent to subscribed clients on the way;
//! - a client that goes away (an interrupted terminal, an agent's timeout)
//!   stops nothing: the separation finishes and is recorded, and asking
//!   again answers from the Sidecar without running the engine;
//! - asking again for the same track while it is still being separated
//!   joins the wait; asking for another track answers
//!   [`codes::SEPARATION_BUSY`], since one separation uses most of the
//!   machine;
//! - the Sidecar is touched once, at the end, when the finished Stems are
//!   checked and recorded in one step. Until then the engine writes in a
//!   work folder, so a separation that fails, or a service that stops or
//!   dies part-way, leaves the Sidecar as it was.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::stems::NewStems;
pub use wreckordbox_sidecar::stems::{Role, Stem, StemSet};
pub use wreckordbox_sidecar::{Fingerprint, RunId, Source, SourceKind};
use wreckordbox_sidecar::{Sidecar, SidecarError};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, Event, Request, ServiceError};
use crate::separation::{self, Cancel};

/// Names the Sidecar's folder, instead of `wreckordbox-sidecar` beside the
/// Library's `master.db`. What a test points at a temporary folder, and
/// what puts the Stems on another disk.
pub const SIDECAR_DIR_ENV: &str = "WRECKORDBOX_SIDECAR_DIR";
/// The command that runs the separation engine's worker: a JSON array of
/// strings (`["/path/python", "-m", "stemsplitter_engine", "--model-dir",
/// "/path/models"]`), or plain words separated by spaces when none of them
/// holds a space. Unset, the engine in this checkout's
/// `separation-engine/.venv` is run with the model folder
/// `~/.cache/wreckordbox/separation/models`.
pub const SEPARATION_COMMAND_ENV: &str = "WRECKORDBOX_SEPARATION_COMMAND";

/// The names of the Stems commands.
pub mod commands {
    /// Separates one track into its four Stems and records them in the
    /// Sidecar, or answers the Stems it already has when its audio has not
    /// changed. Takes a [`TrackId`](super::TrackId), answers a
    /// [`Separated`](super::Separated). Answers when the separation is
    /// done.
    pub const STEMS_SEPARATE: &str = "stems.separate";
    /// The Stems the Sidecar holds for one track. Takes a
    /// [`TrackId`](super::TrackId), answers a
    /// [`TrackStems`](super::TrackStems).
    pub const STEMS_SHOW: &str = "stems.show";
}

/// The names of the events these commands send.
pub mod events {
    /// How far a separation has got: `data` is a
    /// [`StemsProgress`](super::StemsProgress).
    pub const STEMS_PROGRESS: &str = "stems_progress";
    /// Facts in the Sidecar changed: `data` is a
    /// [`SidecarChanged`](super::SidecarChanged). Not a change to the
    /// Library.
    pub const SIDECAR_CHANGED: &str = "sidecar_changed";
}

/// The error codes of the Stems commands, beside the service's own.
pub mod codes {
    /// The track's audio file is not where the Library says it is.
    pub const TRACK_FILE_MISSING: &str = "track_file_missing";
    /// The separation engine could not be started.
    pub const SEPARATION_ENGINE_MISSING: &str = "separation_engine_missing";
    /// The engine started and did not produce Stems.
    pub const SEPARATION_FAILED: &str = "separation_failed";
    /// Another track is being separated; one at a time.
    pub const SEPARATION_BUSY: &str = "separation_busy";
    /// The service stopped while the track was being separated.
    pub const SEPARATION_CANCELLED: &str = "separation_cancelled";
    /// A Stem is not exactly as long as its parent. Nothing was recorded.
    pub const STEM_LENGTH_MISMATCH: &str = "stem_length_mismatch";
    /// The Sidecar could not be opened, read or written.
    pub const SIDECAR_FAILED: &str = "sidecar_failed";
}

/// The arguments of both commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackId {
    /// The track's identifier, as every track listing gives it.
    pub id: String,
}

/// The parent track, as the answers name it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub id: String,
    pub title: String,
}

/// What `stems.separate` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Separated {
    pub track: Parent,
    /// True when the track already had Stems made from this same audio and
    /// the engine was not run.
    pub reused: bool,
    /// The track's Stems now.
    pub stems: StemSet,
    /// Facts this command found stale because the track's audio is no
    /// longer the audio they were made from. They are kept, marked.
    pub marked_stale: Vec<i64>,
}

/// What `stems.show` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackStems {
    pub track: Parent,
    /// The track's Stems now: made from the audio it has, and not replaced.
    /// `null` when it has none.
    pub stems: Option<StemSet>,
    /// Earlier Stem sets that went stale or were replaced, newest first.
    pub previous: Vec<StemSet>,
}

/// The data of a [`events::STEMS_PROGRESS`] event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StemsProgress {
    pub track_id: String,
    /// The engine's word for what it is doing: `loading_model`,
    /// `separating`, `writing`.
    pub stage: String,
    /// 0 to 1. The engine's estimate from the clock, not a measure.
    pub fraction: f64,
}

/// The data of a [`events::SIDECAR_CHANGED`] event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarChanged {
    pub track_id: String,
    /// The kind of Fact that changed: `stems`, `genre`, ...
    pub kind: String,
    /// The Fact that was recorded, when one was.
    pub fact_id: Option<i64>,
    /// Facts marked stale.
    pub stale: Vec<i64>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::STEMS_SEPARATE | commands::STEMS_SHOW)
}

/// The parent track and where its audio is, from the Library.
pub(crate) struct ParentAudio {
    parent: Parent,
    path: PathBuf,
}

impl ParentAudio {
    pub(crate) fn parent(&self) -> &Parent {
        &self.parent
    }

    /// Where the Library says the track's audio file is.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// Looks the track up in the Library, the way every command does.
pub(crate) fn parent(library: &OpenLibrary, id: &str) -> Result<ParentAudio, ServiceError> {
    let index = library.index()?;
    let Some(row) = index.row_of(id) else {
        return Err(ServiceError::new(
            service_codes::TRACK_NOT_FOUND,
            format!("there is no track {id} in the Library"),
        ));
    };
    let at = row as usize;
    Ok(ParentAudio {
        parent: Parent {
            id: index.ids.get(at).copied().unwrap_or(0).to_string(),
            title: index.title.get(at).to_owned(),
        },
        path: PathBuf::from(index.folder_path.get(at)),
    })
}

/// Answers one of this module's commands. `lookup` finds a track in the
/// Library; it is the only thing here that takes the Library's lock, and it
/// lets go before any separation starts.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let track: TrackId = serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })?;
    let found = lookup(&track.id)?;
    let answer = match request.command.as_str() {
        commands::STEMS_SEPARATE => serde_json::to_value(stems.separate(found)?),
        _ => serde_json::to_value(stems.show(found)?),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// The service's Stems: the Sidecar, the engine, and the one separation
/// that may be running.
pub(crate) struct Stems {
    sidecar_dir: PathBuf,
    command: Vec<OsString>,
    sidecar: Mutex<Option<Arc<Sidecar>>>,
    /// The separation in progress, if any.
    running: Mutex<Option<Arc<Job>>>,
    /// Where events go to reach clients and the host.
    events: Mutex<Sender<Event>>,
}

struct Job {
    track_id: String,
    outcome: Mutex<Option<Result<Separated, ServiceError>>>,
    done: Condvar,
    cancel: Cancel,
}

impl Stems {
    /// For the Library at `master_db`, configured from the environment
    /// ([`SIDECAR_DIR_ENV`], [`SEPARATION_COMMAND_ENV`]). Opens nothing
    /// yet: a service that is never asked about Stems makes no Sidecar.
    pub(crate) fn from_env(master_db: &Path, events: Sender<Event>) -> Arc<Self> {
        let sidecar_dir = std::env::var_os(SIDECAR_DIR_ENV)
            .filter(|dir| !dir.is_empty())
            .map_or_else(
                || wreckordbox_sidecar::default_dir(master_db),
                PathBuf::from,
            );
        let command = std::env::var(SEPARATION_COMMAND_ENV)
            .ok()
            .filter(|command| !command.trim().is_empty())
            .map_or_else(default_command, |command| parse_command(&command));
        Arc::new(Self {
            sidecar_dir,
            command,
            sidecar: Mutex::new(None),
            running: Mutex::new(None),
            events: Mutex::new(events),
        })
    }

    /// Stops the separation in progress, if there is one: the engine is
    /// killed and nothing of it is recorded. For a service that is
    /// stopping.
    pub(crate) fn cancel(&self) {
        if let Some(job) = lock(&self.running).as_ref() {
            job.cancel.cancel();
        }
    }

    /// The Sidecar, opened on first use.
    pub(crate) fn sidecar(&self) -> Result<Arc<Sidecar>, ServiceError> {
        let mut sidecar = lock(&self.sidecar);
        if let Some(open) = sidecar.as_ref() {
            return Ok(Arc::clone(open));
        }
        let open = Arc::new(Sidecar::open(&self.sidecar_dir).map_err(|e| sidecar_error(&e))?);
        *sidecar = Some(Arc::clone(&open));
        Ok(open)
    }

    /// Sends an event to subscribed clients and the host.
    pub(crate) fn publish(&self, event: &str, data: Value) {
        let _ = lock(&self.events).send(Event {
            event: event.to_owned(),
            data,
        });
    }

    fn separate(self: &Arc<Self>, found: ParentAudio) -> Result<Separated, ServiceError> {
        let ParentAudio { parent, path } = found;
        let job = {
            // Held while deciding, so two requests for one track cannot
            // both decide to separate it.
            let mut running = lock(&self.running);
            if let Some(job) = running.as_ref() {
                if job.track_id != parent.id {
                    return Err(ServiceError::new(
                        codes::SEPARATION_BUSY,
                        format!(
                            "track {} is being separated; one track at a time. Try again when it is done.",
                            job.track_id
                        ),
                    ));
                }
                Arc::clone(job)
            } else {
                let sidecar = self.sidecar()?;
                let fingerprint = fingerprint(&parent, &path)?;
                let marked_stale = sidecar
                    .mark_stale(&parent.id, &fingerprint)
                    .map_err(|e| sidecar_error(&e))?;
                if !marked_stale.is_empty() {
                    self.changed(&parent.id, None, &marked_stale);
                }
                let current = sidecar
                    .current_stems(&parent.id, &fingerprint)
                    .map_err(|e| sidecar_error(&e))?
                    // Stems whose files have been removed are not Stems.
                    .filter(|set| set.stems.iter().all(|stem| stem.path.is_file()));
                if let Some(stems) = current {
                    return Ok(Separated {
                        track: parent,
                        reused: true,
                        stems,
                        marked_stale,
                    });
                }
                let job = Arc::new(Job {
                    track_id: parent.id.clone(),
                    outcome: Mutex::new(None),
                    done: Condvar::new(),
                    cancel: Cancel::default(),
                });
                *running = Some(Arc::clone(&job));
                let (stems, work) = (Arc::clone(self), Arc::clone(&job));
                std::thread::spawn(move || {
                    let outcome =
                        stems.run(&sidecar, &work, parent, &path, &fingerprint, marked_stale);
                    *lock(&stems.running) = None;
                    *lock(&work.outcome) = Some(outcome);
                    work.done.notify_all();
                });
                job
            }
        };
        let mut outcome = lock(&job.outcome);
        loop {
            if let Some(outcome) = outcome.as_ref() {
                return outcome.clone();
            }
            outcome = job
                .done
                .wait(outcome)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// One separation, start to finish, on its own thread: a Run, the
    /// engine, then the check and the record.
    fn run(
        &self,
        sidecar: &Sidecar,
        job: &Job,
        parent: Parent,
        path: &Path,
        fingerprint: &Fingerprint,
        marked_stale: Vec<i64>,
    ) -> Result<Separated, ServiceError> {
        let source = Source::new(SourceKind::Tool, "separation-engine");
        let run = sidecar
            .begin_run(
                &source,
                &format!(
                    "Separate track {} \"{}\" into Stems",
                    parent.id, parent.title
                ),
            )
            .map_err(|e| sidecar_error(&e))?;
        let recorded = sidecar
            .workspace(run)
            .map_err(|e| sidecar_error(&e))
            .and_then(|workspace| {
                let mut progress = |stage: &str, fraction: f64| {
                    self.publish(
                        events::STEMS_PROGRESS,
                        json!(StemsProgress {
                            track_id: parent.id.clone(),
                            stage: stage.to_owned(),
                            fraction,
                        }),
                    );
                };
                separation::separate(&self.command, path, &workspace, &job.cancel, &mut progress)
            })
            .and_then(|separation| {
                let files: Vec<(Role, PathBuf)> = separation
                    .stems
                    .iter()
                    .filter_map(|(name, file)| Some((Role::named(name)?, file.clone())))
                    .collect();
                sidecar
                    .record_stems(&NewStems {
                        track_id: &parent.id,
                        fingerprint,
                        parent_samples: separation.source_samples,
                        sample_rate: separation.sample_rate,
                        files: &files,
                        source: &source,
                        produced_by: json!({
                            "engine": "separation-engine",
                            "engine_version": separation.engine_version,
                            "model": separation.model,
                            "mode": "four_stem",
                            "quality": "standard",
                        }),
                        run,
                    })
                    .map_err(|e| sidecar_error(&e))
            });
        sidecar.discard_workspace(run);
        let _ = sidecar.finish_run(
            run,
            &match &recorded {
                Ok(_) => "recorded".to_owned(),
                Err(error) => format!("{}: {}", error.code, error.message),
            },
        );
        let stems = recorded?;
        self.changed(&parent.id, Some(stems.fact_id), &[]);
        Ok(Separated {
            track: parent,
            reused: false,
            stems,
            marked_stale,
        })
    }

    fn show(&self, found: ParentAudio) -> Result<TrackStems, ServiceError> {
        let ParentAudio { parent, path } = found;
        let sidecar = self.sidecar()?;
        // With the audio at hand, Stems made from other audio are found
        // out here too, so nothing reads stale Stems as current.
        if path.is_file() {
            let fingerprint = fingerprint(&parent, &path)?;
            let marked_stale = sidecar
                .mark_stale(&parent.id, &fingerprint)
                .map_err(|e| sidecar_error(&e))?;
            if !marked_stale.is_empty() {
                self.changed(&parent.id, None, &marked_stale);
            }
        }
        let (current, previous): (Vec<StemSet>, Vec<StemSet>) = sidecar
            .stems(&parent.id)
            .map_err(|e| sidecar_error(&e))?
            .into_iter()
            .partition(StemSet::is_current);
        Ok(TrackStems {
            track: parent,
            stems: current.into_iter().next(),
            previous,
        })
    }

    fn changed(&self, track_id: &str, fact_id: Option<i64>, stale: &[i64]) {
        self.publish(
            events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: track_id.to_owned(),
                kind: wreckordbox_sidecar::stems::STEMS.name.to_owned(),
                fact_id,
                stale: stale.to_vec(),
            }),
        );
    }
}

/// The fingerprint of a track's audio, or why its file cannot be read.
pub(crate) fn fingerprint(parent: &Parent, path: &Path) -> Result<Fingerprint, ServiceError> {
    if !path.is_file() {
        return Err(ServiceError::new(
            codes::TRACK_FILE_MISSING,
            format!(
                "the audio file of track {} \"{}\" is missing: there is no {}",
                parent.id,
                parent.title,
                path.display()
            ),
        ));
    }
    wreckordbox_sidecar::fingerprint_file(path).map_err(|e| {
        ServiceError::new(
            codes::TRACK_FILE_MISSING,
            format!(
                "the audio file of track {} \"{}\" cannot be read: {e}",
                parent.id, parent.title
            ),
        )
    })
}

fn sidecar_error(error: &SidecarError) -> ServiceError {
    let code = match error {
        SidecarError::StemLength { .. } => codes::STEM_LENGTH_MISMATCH,
        SidecarError::StemSet(_) | SidecarError::Audio { .. } => codes::SEPARATION_FAILED,
        _ => codes::SIDECAR_FAILED,
    };
    let what = if code == codes::SIDECAR_FAILED {
        "the Sidecar failed"
    } else {
        "the Stems were refused"
    };
    ServiceError::new(code, format!("{what}: {error}"))
}

/// The engine as a checkout has it: the virtual environment
/// `uv sync --extra full` makes in `separation-engine/`, with the models
/// kept in the user's cache folder.
fn default_command() -> Vec<OsString> {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../separation-engine");
    let models = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("wreckordbox/separation/models");
    vec![
        engine.join(".venv/bin/python").into_os_string(),
        "-m".into(),
        "stemsplitter_engine".into(),
        "--model-dir".into(),
        models.into_os_string(),
    ]
}

fn parse_command(text: &str) -> Vec<OsString> {
    let text = text.trim();
    if text.starts_with('[') {
        if let Ok(words) = serde_json::from_str::<Vec<String>>(text) {
            return words.into_iter().map(OsString::from).collect();
        }
    }
    text.split_whitespace().map(OsString::from).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
