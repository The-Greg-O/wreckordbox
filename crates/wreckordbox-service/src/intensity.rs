//! Intensity: measuring how hard a track hits and keeping the result in
//! the Sidecar.
//!
//! Intensity is a Measurement: computed from a track's audio by
//! [`wreckordbox_intensity`], recorded as a Fact of the built-in kind
//! `intensity` from the tool [`SOURCE_NAME`], and tied to the fingerprint
//! of the audio it was made from. Nothing here writes to the Library: no
//! `library_changed` is sent and the Library's generation does not move.
//!
//! Measuring one track or a thousand is the same command:
//!
//! - the tracks are found in the Library first, which is the only moment
//!   its lock is held; the measuring then runs with the Library free, so
//!   every other command is answered meanwhile;
//! - a track already measured from the audio it has now, by this version
//!   of the method, is not decoded again: its stored Measurement is the
//!   answer;
//! - everything measured by one command belongs to one Run, made when the
//!   first track needs measuring (a command that reuses everything makes
//!   no Run);
//! - each track is decoded once, a packet at a time, and its Measurement
//!   is recorded in a transaction of its own as soon as it is done. So
//!   memory does not grow with the number of tracks, and stopping part-way
//!   ([`commands::INTENSITY_CANCEL`], the service stopping, or the process
//!   being killed) loses only the tracks not finished: the Sidecar holds
//!   whole Measurements or none;
//! - a few tracks are measured at once ([`JOBS_ENV`]), and
//!   [`events::INTENSITY_PROGRESS`] says how far it has got;
//! - a track that cannot be measured (its file is missing or unreadable)
//!   is reported and the rest carry on;
//! - a client that goes away stops nothing, as with Stems: the measuring
//!   finishes and asking again answers from the Sidecar. One measuring
//!   runs at a time; a second is answered [`codes::INTENSITY_BUSY`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use wreckordbox_intensity::{
    BarCurve, Features, Intensity, Method, Scale, Summary, TimeCurve, METHOD_NAME, METHOD_VERSION,
};
use wreckordbox_sidecar::{Fact, Kind, Kinds, NewFact, Sidecar, SidecarError};

use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, SidecarChanged, Stems};
pub use crate::stems::{Fingerprint, Parent, RunId, Source, SourceKind};
use crate::tracks::TrackSelection;

/// The kind of Fact an Intensity is.
pub const INTENSITY: Kind<'static> = Kind::measurement("intensity");
/// The tool that is the source of every Intensity measured here.
pub const SOURCE_NAME: &str = METHOD_NAME;
/// How many tracks are measured at once, when set to a whole number above
/// zero. Unset, half the machine's cores, at most four.
pub const JOBS_ENV: &str = "WRECKORDBOX_INTENSITY_JOBS";

/// The names of the Intensity commands.
pub mod commands {
    /// Measures the Intensity of the tracks named or selected, reusing
    /// what is already measured. Takes a [`Measure`](super::Measure),
    /// answers a [`Measured`](super::Measured) when all are done.
    pub const INTENSITY_MEASURE: &str = "intensity.measure";
    /// The Intensity the Sidecar holds for one track, in full. Takes a
    /// [`TrackId`](crate::stems::TrackId), answers a
    /// [`TrackIntensity`](super::TrackIntensity).
    pub const INTENSITY_SHOW: &str = "intensity.show";
    /// Stops the measuring in progress after the tracks it has finished.
    /// Takes nothing, answers a [`Cancelled`](super::Cancelled).
    pub const INTENSITY_CANCEL: &str = "intensity.cancel";
}

/// The names of the events these commands send, beside
/// [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED).
pub mod events {
    /// How far a measuring has got: `data` is an
    /// [`IntensityProgress`](super::IntensityProgress).
    pub const INTENSITY_PROGRESS: &str = "intensity_progress";
}

/// The error codes of the Intensity commands, beside the service's own and
/// `track_file_missing` and `sidecar_failed` of the Stems commands.
pub mod codes {
    /// A measuring is already running; one at a time.
    pub const INTENSITY_BUSY: &str = "intensity_busy";
    /// The track's audio could not be decoded.
    pub const INTENSITY_FAILED: &str = "intensity_failed";
}

/// The arguments of `intensity.measure`: tracks by identifier, tracks by
/// selection, or both (the named ones first). Naming neither is refused;
/// an empty `selection` is the whole Library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Measure {
    /// Tracks by identifier, measured in this order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// Tracks as `tracks.query` selects them: by any field, by `MyTag`, by
    /// playlist. Its `limit` and `offset` are not applied: every match is
    /// measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<TrackSelection>,
    /// Measure again even where a current Measurement exists.
    pub force: bool,
}

/// What became of one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Decoded, measured and recorded.
    Measured,
    /// Already measured from this audio; nothing was decoded.
    Reused,
    /// Could not be measured; `error` says why. Nothing was recorded.
    Failed,
    /// Not reached, or abandoned part-way, because the measuring was
    /// cancelled. Nothing was recorded.
    Cancelled,
}

/// One track of a [`Measured`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackOutcome {
    pub track: Parent,
    pub status: Status,
    /// The Fact holding the track's Intensity now.
    pub fact_id: Option<i64>,
    /// The fingerprint of the track's audio.
    pub fingerprint: Option<Fingerprint>,
    /// The Intensity's summary; the curves are read with `intensity.show`.
    pub summary: Option<Summary>,
    /// How many points the time curve has.
    pub points: Option<usize>,
    /// How many bars the per-bar curve has; `null` for a track with no
    /// beat grid.
    pub bars: Option<usize>,
    /// Facts found stale because the track's audio is no longer the audio
    /// they were made from. They are kept, marked.
    pub marked_stale: Vec<i64>,
    pub error: Option<ServiceError>,
}

/// What `intensity.measure` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    /// The Run everything measured belongs to; `null` when nothing needed
    /// measuring.
    pub run: Option<RunId>,
    pub total: usize,
    pub measured: usize,
    pub reused: usize,
    pub failed: usize,
    /// True when the measuring was stopped before every track was done.
    pub cancelled: bool,
    /// How many audio files were decoded.
    pub decoded: usize,
    /// One per track, in the order asked.
    pub tracks: Vec<TrackOutcome>,
}

/// A stored Intensity: the Fact, with its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub fact_id: i64,
    pub track_id: String,
    /// The fingerprint of the audio it was measured from.
    pub fingerprint: Option<Fingerprint>,
    /// The Intensity as stored: see [`Measurement::intensity`].
    pub value: Value,
    pub source: Source,
    pub confidence: f64,
    pub run: RunId,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
    /// The track's audio has changed since.
    pub stale: bool,
    /// A later measuring took its place.
    pub superseded: bool,
}

impl Measurement {
    /// The value as this version of the method shapes it.
    ///
    /// # Errors
    ///
    /// When the value was stored by a version whose shape differs.
    pub fn intensity(&self) -> Result<Intensity, serde_json::Error> {
        serde_json::from_value(self.value.clone())
    }
}

impl From<Fact> for Measurement {
    fn from(fact: Fact) -> Self {
        Self {
            stale: fact.is_stale(),
            superseded: !fact.is_live(),
            fact_id: fact.id,
            track_id: fact.track_id,
            fingerprint: fact.fingerprint,
            value: fact.value,
            source: fact.source,
            confidence: fact.confidence,
            run: fact.run,
            recorded_at: fact.recorded_at,
        }
    }
}

/// What `intensity.show` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackIntensity {
    pub track: Parent,
    /// The track's Intensity now: measured from the audio it has, and not
    /// replaced. `null` when it has none.
    pub intensity: Option<Measurement>,
    /// Earlier ones that went stale or were replaced, newest first.
    pub previous: Vec<Measurement>,
}

/// What `intensity.cancel` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancelled {
    /// Whether a measuring was running to be stopped.
    pub cancelled: bool,
}

/// The data of an [`events::INTENSITY_PROGRESS`] event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntensityProgress {
    pub track_id: String,
    pub title: String,
    /// `measuring` when the track's audio starts being decoded, then what
    /// became of it: `measured`, `reused` or `failed`.
    pub status: String,
    /// How many tracks are settled.
    pub done: usize,
    /// How many tracks the command is measuring.
    pub total: usize,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::INTENSITY_MEASURE | commands::INTENSITY_SHOW | commands::INTENSITY_CANCEL
    )
}

/// A track to measure, as the Library has it.
pub(crate) struct Target {
    parent: Parent,
    /// Where the Library says the audio file is.
    path: PathBuf,
    /// Where the Library says the track's analysis file is, when it has
    /// been analysed: the beat grid is in it.
    analysis: Option<PathBuf>,
}

/// Finds the tracks a command is about. The only thing here that reads
/// the Library.
pub(crate) fn targets(
    library: &OpenLibrary,
    wanted: &Measure,
) -> Result<Vec<Target>, ServiceError> {
    let index = library.index()?;
    let share_root = library.share_root();
    let mut rows = Vec::new();
    for id in &wanted.ids {
        let Some(row) = index.row_of(id) else {
            return Err(ServiceError::new(
                service_codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ));
        };
        rows.push(row);
    }
    if let Some(selection) = &wanted.selection {
        rows.extend(crate::tracks::matching(&index, selection)?);
    }
    let mut seen = std::collections::HashSet::new();
    rows.retain(|row| seen.insert(*row));
    Ok(rows
        .into_iter()
        .map(|row| {
            let at = row as usize;
            let analysis = index.analysis_path.get(at);
            Target {
                parent: Parent {
                    id: index.ids.get(at).copied().unwrap_or(0).to_string(),
                    title: index.title.get(at).to_owned(),
                },
                path: PathBuf::from(index.folder_path.get(at)),
                analysis: (!analysis.is_empty()).then(|| rbl_anlz::resolve(&share_root, analysis)),
            }
        })
        .collect())
}

/// Answers one of this module's commands. `lookup` finds the tracks in
/// the Library; it lets go of the Library before any measuring starts.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&Measure) -> Result<Vec<Target>, ServiceError>,
) -> Result<Value, ServiceError> {
    let bad = |e: serde_json::Error| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    };
    let answer = match request.command.as_str() {
        commands::INTENSITY_MEASURE => {
            let wanted: Measure = serde_json::from_value(request.args.clone()).map_err(bad)?;
            if wanted.ids.is_empty() && wanted.selection.is_none() {
                return Err(ServiceError::new(
                    service_codes::BAD_REQUEST,
                    "name the tracks to measure, by identifier or by selection; an empty selection is the whole Library",
                ));
            }
            let targets = lookup(&wanted)?;
            serde_json::to_value(measure(stems, &targets, wanted.force)?)
        }
        commands::INTENSITY_SHOW => {
            let track: stems::TrackId =
                serde_json::from_value(request.args.clone()).map_err(bad)?;
            let found = lookup(&Measure {
                ids: vec![track.id],
                ..Measure::default()
            })?;
            let Some(target) = found.into_iter().next() else {
                return Err(ServiceError::new(
                    service_codes::TRACK_NOT_FOUND,
                    "there is no such track in the Library",
                ));
            };
            serde_json::to_value(show(stems, target)?)
        }
        _ => serde_json::to_value(Cancelled {
            cancelled: cancel(stems),
        }),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// The measuring in progress for each Sidecar, by its folder. Kept here
/// and not on the service so nothing else has to carry it.
static RUNNING: Mutex<Vec<(PathBuf, Arc<AtomicBool>)>> = Mutex::new(Vec::new());

/// Stops the measuring in progress on this service's Sidecar, if there is
/// one: tracks already recorded are kept, the one being decoded is
/// abandoned. Answers whether there was one.
pub(crate) fn cancel(stems: &Stems) -> bool {
    // A service that never opened a Sidecar never measured.
    let Ok(Some(sidecar)) = stems.sidecar_if_made() else {
        return false;
    };
    let running = lock(&RUNNING);
    running
        .iter()
        .filter(|(dir, _)| dir == sidecar.dir())
        .map(|(_, stop)| stop.store(true, Ordering::Release))
        .count()
        > 0
}

/// The claim on a Sidecar's one measuring; let go when dropped.
struct Claim {
    dir: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Claim {
    fn take(dir: &Path) -> Result<Self, ServiceError> {
        let mut running = lock(&RUNNING);
        if running.iter().any(|(claimed, _)| claimed == dir) {
            return Err(ServiceError::new(
                codes::INTENSITY_BUSY,
                "Intensity is already being measured; one measuring at a time. Try again when it is done, or stop it with `intensity cancel`.",
            ));
        }
        let stop = Arc::new(AtomicBool::new(false));
        running.push((dir.to_path_buf(), Arc::clone(&stop)));
        Ok(Self {
            dir: dir.to_path_buf(),
            stop,
        })
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&RUNNING).retain(|(dir, _)| *dir != self.dir);
    }
}

/// One measuring, start to finish.
struct Measuring<'a> {
    stems: &'a Stems,
    sidecar: &'a Sidecar,
    claim: &'a Claim,
    source: Source,
    force: bool,
    total: usize,
    description: String,
    /// The Run, made when the first track needs measuring.
    run: Mutex<Option<RunId>>,
    done: AtomicUsize,
    decoded: AtomicUsize,
}

fn measure(stems: &Arc<Stems>, targets: &[Target], force: bool) -> Result<Measured, ServiceError> {
    let total = targets.len();
    if total == 0 {
        return Ok(Measured {
            run: None,
            total,
            measured: 0,
            reused: 0,
            failed: 0,
            cancelled: false,
            decoded: 0,
            tracks: Vec::new(),
        });
    }
    let sidecar = stems.sidecar()?;
    let claim = Claim::take(sidecar.dir())?;
    let measuring = Measuring {
        stems,
        sidecar: &sidecar,
        claim: &claim,
        source: Source::new(SourceKind::Tool, SOURCE_NAME),
        force,
        total,
        description: match targets {
            [only] => format!(
                "Measure the Intensity of track {} \"{}\"",
                only.parent.id, only.parent.title
            ),
            _ => format!("Measure the Intensity of {total} tracks"),
        },
        run: Mutex::new(None),
        done: AtomicUsize::new(0),
        decoded: AtomicUsize::new(0),
    };

    // Each worker takes the next track not yet taken, so the tracks are
    // settled a few at a time and nothing is held but their outcomes.
    let next = AtomicUsize::new(0);
    let settled: Mutex<Vec<Option<TrackOutcome>>> = Mutex::new(vec![None; total]);
    std::thread::scope(|scope| {
        for _ in 0..jobs().min(total) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, Ordering::AcqRel);
                let Some(target) = targets.get(at) else { break };
                let outcome = measuring.settle(target);
                if let Some(slot) = lock(&settled).get_mut(at) {
                    *slot = Some(outcome);
                }
            });
        }
    });

    let tracks: Vec<TrackOutcome> = lock(&settled)
        .drain(..)
        .zip(targets)
        // A worker that died left its track unsettled: it was not measured.
        .map(|(outcome, target)| {
            outcome.unwrap_or_else(|| {
                TrackOutcome::failed(
                    &target.parent,
                    None,
                    Vec::new(),
                    ServiceError::new(
                        codes::INTENSITY_FAILED,
                        "the measuring stopped unexpectedly",
                    ),
                )
            })
        })
        .collect();
    let count = |status: Status| tracks.iter().filter(|track| track.status == status).count();
    let measured = Measured {
        run: *lock(&measuring.run),
        total,
        measured: count(Status::Measured),
        reused: count(Status::Reused),
        failed: count(Status::Failed),
        cancelled: count(Status::Cancelled) > 0,
        decoded: measuring.decoded.load(Ordering::Acquire),
        tracks,
    };
    if let Some(run) = measured.run {
        let _ = sidecar.finish_run(
            run,
            &format!(
                "measured {}, reused {}, failed {} of {} tracks{}",
                measured.measured,
                measured.reused,
                measured.failed,
                total,
                if measured.cancelled {
                    "; cancelled"
                } else {
                    ""
                }
            ),
        );
    }
    Ok(measured)
}

impl Measuring<'_> {
    /// Settles one track and says so.
    fn settle(&self, target: &Target) -> TrackOutcome {
        if self.claim.stopped() {
            return TrackOutcome::cancelled(&target.parent);
        }
        let outcome = self.one(target);
        if outcome.status != Status::Cancelled {
            let done = self.done.fetch_add(1, Ordering::AcqRel) + 1;
            self.progress(&target.parent, outcome.status.name(), done);
        }
        outcome
    }

    fn one(&self, target: &Target) -> TrackOutcome {
        let parent = &target.parent;
        let fingerprint = match stems::fingerprint(parent, &target.path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return TrackOutcome::failed(parent, None, Vec::new(), error),
        };
        let failed = |marked_stale: Vec<i64>, error: ServiceError| {
            TrackOutcome::failed(parent, Some(fingerprint.clone()), marked_stale, error)
        };
        // Measurements made from other audio are found out first, so one
        // of them is never taken for current.
        let marked_stale = match mark_stale(self.stems, self.sidecar, &parent.id, &fingerprint) {
            Ok(marked_stale) => marked_stale,
            Err(error) => return failed(Vec::new(), error),
        };
        if !self.force {
            match current(self.sidecar, &parent.id, Some(&fingerprint)) {
                Ok(Some(fact)) => {
                    return TrackOutcome::recorded(parent, Status::Reused, &fact, marked_stale)
                }
                Ok(None) => {}
                Err(error) => return failed(marked_stale, error),
            }
        }

        self.progress(parent, "measuring", self.done.load(Ordering::Acquire));
        self.decoded.fetch_add(1, Ordering::AcqRel);
        let bars = bar_starts(target.analysis.as_deref());
        let intensity = match wreckordbox_intensity::measure_file(&target.path, &bars, &|| {
            self.claim.stopped()
        }) {
            Ok(Some(intensity)) => intensity,
            Ok(None) => {
                return TrackOutcome {
                    fingerprint: Some(fingerprint),
                    marked_stale,
                    ..TrackOutcome::cancelled(parent)
                }
            }
            Err(error) => {
                return failed(
                    marked_stale,
                    ServiceError::new(
                        codes::INTENSITY_FAILED,
                        format!(
                            "the audio of track {} \"{}\" could not be measured: {error}",
                            parent.id, parent.title
                        ),
                    ),
                )
            }
        };
        match self.record(parent, &fingerprint, &intensity) {
            Ok(fact) => {
                changed(self.stems, &parent.id, Some(fact.id), &[]);
                TrackOutcome::recorded(parent, Status::Measured, &fact, marked_stale)
            }
            Err(error) => failed(marked_stale, error),
        }
    }

    /// Records one track's Intensity, in the command's Run.
    fn record(
        &self,
        parent: &Parent,
        fingerprint: &Fingerprint,
        intensity: &Intensity,
    ) -> Result<Fact, ServiceError> {
        let value = serde_json::to_value(intensity)
            .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))?;
        // The store takes any value; the kind's declaration is what says
        // an Intensity is an object.
        if let Some(declared) = Kinds::built_in()
            .iter()
            .find(|declared| declared.name == INTENSITY.name)
        {
            declared
                .check(&value)
                .map_err(|message| ServiceError::new(service_codes::SERVICE_FAILED, message))?;
        }
        let run = {
            let mut run = lock(&self.run);
            if let Some(run) = *run {
                run
            } else {
                let begun = self
                    .sidecar
                    .begin_run(&self.source, &self.description)
                    .map_err(|e| sidecar_error(&e))?;
                *run = Some(begun);
                begun
            }
        };
        self.sidecar
            .record(&NewFact {
                track_id: &parent.id,
                fingerprint: Some(fingerprint),
                kind: INTENSITY,
                value,
                source: &self.source,
                confidence: 1.0,
                run,
            })
            .map_err(|e| sidecar_error(&e))
    }

    fn progress(&self, parent: &Parent, status: &str, done: usize) {
        self.stems.publish(
            events::INTENSITY_PROGRESS,
            json!(IntensityProgress {
                track_id: parent.id.clone(),
                title: parent.title.clone(),
                status: status.to_owned(),
                done,
                total: self.total,
            }),
        );
    }
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Reused => "reused",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl TrackOutcome {
    fn cancelled(parent: &Parent) -> Self {
        Self {
            track: parent.clone(),
            status: Status::Cancelled,
            fact_id: None,
            fingerprint: None,
            summary: None,
            points: None,
            bars: None,
            marked_stale: Vec::new(),
            error: None,
        }
    }

    fn failed(
        parent: &Parent,
        fingerprint: Option<Fingerprint>,
        marked_stale: Vec<i64>,
        error: ServiceError,
    ) -> Self {
        Self {
            status: Status::Failed,
            fingerprint,
            marked_stale,
            error: Some(error),
            ..Self::cancelled(parent)
        }
    }

    fn recorded(parent: &Parent, status: Status, fact: &Fact, marked_stale: Vec<i64>) -> Self {
        let intensity = serde_json::from_value::<Intensity>(fact.value.clone()).ok();
        Self {
            status,
            fact_id: Some(fact.id),
            fingerprint: fact.fingerprint.clone(),
            summary: intensity.as_ref().map(|intensity| intensity.summary),
            points: intensity
                .as_ref()
                .map(|intensity| intensity.curve.intensity.len()),
            bars: intensity
                .as_ref()
                .and_then(|intensity| intensity.bars.as_ref())
                .map(|bars| bars.intensity.len()),
            marked_stale,
            ..Self::cancelled(parent)
        }
    }
}

fn show(stems: &Stems, target: Target) -> Result<TrackIntensity, ServiceError> {
    let Target { parent, path, .. } = target;
    let sidecar = stems.sidecar()?;
    // With the audio at hand, a Measurement made from other audio is
    // found out here too, so nothing reads a stale one as current.
    if path.is_file() {
        let fingerprint = stems::fingerprint(&parent, &path)?;
        mark_stale(stems, &sidecar, &parent.id, &fingerprint)?;
    }
    let (current, previous): (Vec<Fact>, Vec<Fact>) = ours(&sidecar, &parent.id)?
        .into_iter()
        .partition(|fact| fact.is_live() && !fact.is_stale());
    Ok(TrackIntensity {
        track: parent,
        intensity: current.into_iter().next().map(Measurement::from),
        previous: previous.into_iter().map(Measurement::from).collect(),
    })
}

/// Every Intensity this tool recorded for a track, newest first. Another
/// source's Fact of the same kind is its own affair.
fn ours(sidecar: &Sidecar, track_id: &str) -> Result<Vec<Fact>, ServiceError> {
    Ok(sidecar
        .facts(track_id, INTENSITY.name)
        .map_err(|e| sidecar_error(&e))?
        .into_iter()
        .filter(|fact| fact.source.kind == SourceKind::Tool && fact.source.name == SOURCE_NAME)
        .collect())
}

/// The track's Intensity that can stand for a new measuring: not replaced,
/// not stale, made from this audio by this version of the method.
fn current(
    sidecar: &Sidecar,
    track_id: &str,
    fingerprint: Option<&Fingerprint>,
) -> Result<Option<Fact>, ServiceError> {
    Ok(ours(sidecar, track_id)?.into_iter().find(|fact| {
        fact.is_live()
            && !fact.is_stale()
            && fact.fingerprint.as_ref() == fingerprint
            && fact.value["method"]["version"] == json!(METHOD_VERSION)
    }))
}

/// Where each bar of the track's beat grid begins, in seconds; empty when
/// the track has no analysis file or no grid in it.
fn bar_starts(analysis: Option<&Path>) -> Vec<f64> {
    analysis
        .filter(|path| path.is_file())
        .and_then(|path| rbl_anlz::Anlz::read(path).ok())
        .and_then(|analysis| analysis.beat_grid())
        .map(|beats| {
            beats
                .iter()
                .filter(|beat| beat.beat_number == 1)
                .map(|beat| f64::from(beat.time_ms) / 1000.0)
                .collect()
        })
        .unwrap_or_default()
}

/// Marks the track's Measurements made from other audio stale and says
/// so, one event per kind.
fn mark_stale(
    stems: &Stems,
    sidecar: &Sidecar,
    track_id: &str,
    fingerprint: &Fingerprint,
) -> Result<Vec<i64>, ServiceError> {
    let stale = sidecar
        .mark_stale(track_id, fingerprint)
        .map_err(|e| sidecar_error(&e))?;
    let mut by_kind: Vec<(String, Vec<i64>)> = Vec::new();
    for id in &stale {
        let Some(fact) = sidecar.fact(*id).map_err(|e| sidecar_error(&e))? else {
            continue;
        };
        match by_kind.iter_mut().find(|(kind, _)| *kind == fact.kind) {
            Some((_, ids)) => ids.push(*id),
            None => by_kind.push((fact.kind, vec![*id])),
        }
    }
    for (kind, ids) in by_kind {
        stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: track_id.to_owned(),
                kind,
                fact_id: None,
                stale: ids,
            }),
        );
    }
    Ok(stale)
}

fn changed(stems: &Stems, track_id: &str, fact_id: Option<i64>, stale: &[i64]) {
    stems.publish(
        stems::events::SIDECAR_CHANGED,
        json!(SidecarChanged {
            track_id: track_id.to_owned(),
            kind: INTENSITY.name.to_owned(),
            fact_id,
            stale: stale.to_vec(),
        }),
    );
}

/// How many tracks to measure at once.
fn jobs() -> usize {
    std::env::var(JOBS_ENV)
        .ok()
        .and_then(|jobs| jobs.trim().parse::<usize>().ok())
        .filter(|jobs| *jobs > 0)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
            (cores / 2).clamp(1, 4)
        })
}

fn sidecar_error(error: &SidecarError) -> ServiceError {
    ServiceError::new(
        stems::codes::SIDECAR_FAILED,
        format!("the Sidecar failed: {error}"),
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
