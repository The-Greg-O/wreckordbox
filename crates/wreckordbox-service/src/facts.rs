//! Facts: what is known about a track, kept in the Sidecar with who says
//! so and how sure they are, and the Runs that record them.
//!
//! A Fact is recorded for a track with a kind, a value, a source and a
//! confidence, as part of a Run. Nothing here writes to the Library: no
//! `library_changed` is sent and the Library's generation does not move.
//! A change to the Sidecar is announced as
//! [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED), the event
//! the Stems commands send.
//!
//! A kind must be declared before a Fact of it is accepted, as a Judgement
//! or a Measurement and with the type of its value: built in, or in the
//! kinds file in the Sidecar's folder
//! ([`KINDS_FILE`](wreckordbox_sidecar::KINDS_FILE)), which is read on
//! every command so a kind is usable as soon as the file is saved.
//!
//! Facts from different sources about the same track and kind stand side
//! by side; which one "stands" is not decided here. A source that records
//! the same kind for the same track again supersedes its own earlier Fact,
//! which is kept.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use wreckordbox_sidecar::{
    Declaration, Fingerprint, Nature, Run, Source, SourceKind, ValueType,
};
use wreckordbox_sidecar::{Kinds, NewFact, RunId, Sidecar, SidecarError};

use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Parent, ParentAudio, SidecarChanged, Stems};

/// The names of the Facts and Runs commands.
pub mod commands {
    /// Records one Fact or many, in one Run. Takes a
    /// [`RecordFacts`](super::RecordFacts), answers a
    /// [`Recorded`](super::Recorded). All are recorded or none.
    pub const FACTS_RECORD: &str = "facts.record";
    /// The Facts the Sidecar holds for one track. Takes a
    /// [`FactsOfTrack`](super::FactsOfTrack), answers a
    /// [`TrackFacts`](super::TrackFacts).
    pub const FACTS_OF_TRACK: &str = "facts.of_track";
    /// The declared kinds. No arguments, answers a
    /// [`DeclaredKinds`](super::DeclaredKinds).
    pub const FACTS_KINDS: &str = "facts.kinds";
    /// Starts a Run. Takes a [`StartRun`](super::StartRun), answers a
    /// [`Run`](super::Run).
    pub const RUNS_START: &str = "runs.start";
    /// Ends a Run. Takes a [`FinishRun`](super::FinishRun), answers a
    /// [`Run`](super::Run).
    pub const RUNS_FINISH: &str = "runs.finish";
}

/// The error codes of the Facts and Runs commands, beside the service's
/// own and [`stems::codes`]. Nothing is recorded when any is answered.
pub mod codes {
    /// The kind is not declared: not built in and not in the kinds file.
    pub const KIND_NOT_DECLARED: &str = "kind_not_declared";
    /// The value is not of the type the kind is declared with.
    pub const WRONG_VALUE_TYPE: &str = "wrong_value_type";
    /// The kind is recorded only by its own command (Stems).
    pub const KIND_RESERVED: &str = "kind_reserved";
    /// The kinds file does not parse or declares a kind badly.
    pub const KINDS_FILE_INVALID: &str = "kinds_file_invalid";
    /// No confidence was given, or it is not from 0 to 1.
    pub const INVALID_CONFIDENCE: &str = "invalid_confidence";
    /// No source was given, or it has no name.
    pub const INVALID_SOURCE: &str = "invalid_source";
    /// There is no Run with that identifier.
    pub const RUN_NOT_FOUND: &str = "run_not_found";
    /// The Run has ended; nothing more is recorded in it.
    pub const RUN_FINISHED: &str = "run_finished";
}

/// A source as a command takes it: `{"kind": "agent", "name": "tagger"}`.
/// The DJ needs no name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceArg {
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// The arguments of `facts.record`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordFacts {
    /// The Run to record in, from `runs.start`. Left out, a Run is made for
    /// this command alone and ended with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// What the Run made for this command is called. Not with `run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Who the Facts are from, for those that do not say. Left out, the
    /// source of `run`. Needed when a Run is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    /// The confidence of the Facts that do not give their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub facts: Vec<FactToRecord>,
}

/// One Fact to record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactToRecord {
    /// The track's identifier, as every track listing gives it.
    #[serde(alias = "track_id")]
    pub track: String,
    pub kind: String,
    pub value: Value,
    /// 0 to 1. Left out: the command's, or 1 when the source is the DJ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Left out: the command's, or the Run's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
}

/// A Fact as the commands answer it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: i64,
    pub track_id: String,
    pub kind: String,
    pub nature: Nature,
    pub value: Value,
    pub source: Source,
    pub confidence: f64,
    /// The identifier of the Run that recorded it.
    pub run: i64,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
    /// The fingerprint of the track's audio when this was recorded; `null`
    /// for a Judgement about a track whose file was not at hand.
    pub fingerprint: Option<Fingerprint>,
    /// A Measurement whose track's audio has changed since.
    pub stale: bool,
    /// The same source has since recorded this kind for the track again.
    pub superseded: bool,
    /// The Fact that took this one's place.
    pub superseded_by: Option<i64>,
}

impl From<wreckordbox_sidecar::Fact> for Fact {
    fn from(fact: wreckordbox_sidecar::Fact) -> Self {
        Self {
            stale: fact.is_stale(),
            superseded: !fact.is_live(),
            id: fact.id,
            track_id: fact.track_id,
            kind: fact.kind,
            nature: fact.nature,
            value: fact.value,
            source: fact.source,
            confidence: fact.confidence,
            run: fact.run.0,
            recorded_at: fact.recorded_at,
            fingerprint: fact.fingerprint,
            superseded_by: fact.superseded_by,
        }
    }
}

/// What `facts.record` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recorded {
    /// The Run the Facts were recorded in.
    pub run: Run,
    /// True when the Run was made for this command (and ended with it).
    pub created_run: bool,
    /// The Facts, in the order they were given.
    pub facts: Vec<Fact>,
    /// Measurements found stale on the way: their track's audio is no
    /// longer the audio they were made from.
    pub marked_stale: Vec<i64>,
}

/// The arguments of `facts.of_track`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactsOfTrack {
    /// The track's identifier.
    pub id: String,
    /// Only Facts of this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Also stale, unverified and superseded Facts.
    #[serde(default)]
    pub all: bool,
    /// Retained for compatibility. Audio is now checked on every read.
    #[serde(default)]
    pub check_audio: bool,
}

/// What `facts.of_track` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackFacts {
    pub track: Parent,
    /// Newest first.
    pub facts: Vec<Fact>,
    /// Measurements from different audio, kept for inspection only.
    pub stale: Vec<Fact>,
    /// Measurements whose audio could not be read, excluded from standing Facts.
    pub unverified: Vec<Fact>,
    pub reattached: Vec<wreckordbox_sidecar::attachments::Attachment>,
    pub review: Vec<wreckordbox_sidecar::attachments::Review>,
}

/// What `facts.kinds` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclaredKinds {
    /// The built-in kinds, then the kinds file's by name.
    pub kinds: Vec<Declaration>,
    /// Where the kinds file is, or would be.
    pub file: std::path::PathBuf,
}

/// The arguments of `runs.start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRun {
    pub source: SourceArg,
    pub description: String,
}

/// The arguments of `runs.finish`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishRun {
    pub id: i64,
    /// How it went, in a word or a sentence. Left out: `finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::FACTS_RECORD
            | commands::FACTS_OF_TRACK
            | commands::FACTS_KINDS
            | commands::RUNS_START
            | commands::RUNS_FINISH
    )
}

/// Answers one of this module's commands. `lookup` finds a track in the
/// Library; it is the only thing here that takes the Library's lock, and
/// only to read.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let facts = Facts {
        sidecar: stems.sidecar()?,
        stems,
    };
    let answer = match request.command.as_str() {
        commands::FACTS_RECORD => serde_json::to_value(facts.record(arguments(request)?, lookup)?),
        commands::FACTS_OF_TRACK => {
            serde_json::to_value(facts.of_track(&arguments(request)?, lookup)?)
        }
        commands::FACTS_KINDS => {
            let kinds = facts.kinds()?;
            serde_json::to_value(DeclaredKinds {
                kinds: kinds.all().to_vec(),
                file: kinds.file().to_path_buf(),
            })
        }
        commands::RUNS_START => serde_json::to_value(facts.start_run(&arguments(request)?)?),
        _ => serde_json::to_value(facts.finish_run(&arguments(request)?)?),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// One Fact that has passed every check `facts.record` makes, for a
/// command of another module that writes it in a transaction of its own
/// (a Decision's answer, with [`Sidecar::answer_decision_with_fact`]).
pub(crate) struct CheckedFact {
    pub(crate) track_id: String,
    pub(crate) kind: Declaration,
    pub(crate) value: Value,
    /// Of the track's audio now, when its file is there.
    pub(crate) fingerprint: Option<Fingerprint>,
}

/// Checks one Fact as `facts.record` would (its kind's declaration, its
/// value, its track and the audio a Measurement needs), writing nothing.
pub(crate) fn checked(
    stems: &Arc<Stems>,
    fact: FactToRecord,
    source: &Source,
    confidence: f64,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<CheckedFact, ServiceError> {
    let facts = Facts {
        sidecar: stems.sidecar()?,
        stems,
    };
    let mut tracks = HashMap::new();
    let one = check(
        fact,
        &facts.kinds()?,
        Some(source),
        Some(confidence),
        &mut tracks,
        lookup,
    )?;
    Ok(CheckedFact {
        track_id: one.track_id,
        kind: one.kind,
        value: one.value,
        fingerprint: tracks
            .into_values()
            .next()
            .and_then(|(_, fingerprint)| fingerprint),
    })
}

/// What `facts.record` does once a Fact is written, for a Fact another
/// module wrote: tells clients, and marks stale (and says so) the track's
/// Measurements made from other audio.
pub(crate) fn recorded(
    stems: &Arc<Stems>,
    fact: &Fact,
    fingerprint: Option<&Fingerprint>,
) -> Result<(), ServiceError> {
    let facts = Facts {
        sidecar: stems.sidecar()?,
        stems,
    };
    facts.changed(&fact.track_id, &fact.kind, Some(fact.id), &[]);
    if let Some(fingerprint) = fingerprint {
        facts.mark_stale(&fact.track_id, fingerprint)?;
    }
    Ok(())
}

/// The Run with that identifier, if it can still be recorded in.
pub(crate) fn open_run(stems: &Arc<Stems>, id: i64) -> Result<Run, ServiceError> {
    Facts {
        sidecar: stems.sidecar()?,
        stems,
    }
    .open_run(id)
}

pub(crate) fn arguments<T: serde::de::DeserializeOwned>(
    request: &Request,
) -> Result<T, ServiceError> {
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

struct Facts<'a> {
    sidecar: Arc<Sidecar>,
    stems: &'a Stems,
}

/// A Fact that has passed every check and waits to be written.
struct Checked {
    track_id: String,
    kind: Declaration,
    value: Value,
    source: Source,
    confidence: f64,
}

impl Facts<'_> {
    fn kinds(&self) -> Result<Kinds, ServiceError> {
        Kinds::load(self.sidecar.dir()).map_err(|e| sidecar_error(&e))
    }

    fn existing_run(&self, id: i64) -> Result<Run, ServiceError> {
        self.sidecar
            .run(RunId(id))
            .map_err(|e| sidecar_error(&e))?
            .ok_or_else(|| ServiceError::new(codes::RUN_NOT_FOUND, format!("there is no Run {id}")))
    }

    /// A Run that can still be recorded in.
    fn open_run(&self, id: i64) -> Result<Run, ServiceError> {
        let run = self.existing_run(id)?;
        if run.is_finished() {
            return Err(ServiceError::new(
                codes::RUN_FINISHED,
                format!("Run {id} has ended; start another to record more"),
            ));
        }
        Ok(run)
    }

    fn start_run(&self, start: &StartRun) -> Result<Run, ServiceError> {
        let source = source(&start.source)?;
        let description = start.description.trim();
        if description.is_empty() {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "a Run needs a description: what the work is, for whoever reviews it later",
            ));
        }
        let id = self
            .sidecar
            .begin_run(&source, description)
            .map_err(|e| sidecar_error(&e))?;
        self.existing_run(id.0)
    }

    fn finish_run(&self, finish: &FinishRun) -> Result<Run, ServiceError> {
        let run = self.open_run(finish.id)?;
        self.sidecar
            .finish_run(run.id, finish.outcome.as_deref().unwrap_or("finished"))
            .map_err(|e| sidecar_error(&e))?;
        self.existing_run(finish.id)
    }

    fn record(
        &self,
        record: RecordFacts,
        lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
    ) -> Result<Recorded, ServiceError> {
        if record.facts.is_empty() {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "there are no Facts to record",
            ));
        }
        if record.run.is_some() && record.description.is_some() {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "give a Run to record in or a description for a new one, not both",
            ));
        }
        let kinds = self.kinds()?;
        let given_source = record.source.as_ref().map(source).transpose()?;
        let run = record.run.map(|id| self.open_run(id)).transpose()?;
        let default_source = given_source
            .clone()
            .or_else(|| run.as_ref().map(|run| run.source.clone()));
        if run.is_none() && given_source.is_none() {
            return Err(ServiceError::new(
                codes::INVALID_SOURCE,
                "say who the Facts are from: a source (dj, agent, tool or web, with a name), or a Run to record in",
            ));
        }

        // Every check, for every Fact, before anything is written.
        let many = record.facts.len() > 1;
        let mut tracks: HashMap<String, (ParentAudio, Option<Fingerprint>)> = HashMap::new();
        let mut checked = Vec::with_capacity(record.facts.len());
        for (at, fact) in record.facts.into_iter().enumerate() {
            let one = check(
                fact,
                &kinds,
                default_source.as_ref(),
                record.confidence,
                &mut tracks,
                lookup,
            )
            .map_err(|error| {
                if many {
                    ServiceError::new(&error.code, format!("Fact {}: {}", at + 1, error.message))
                } else {
                    error
                }
            })?;
            checked.push(one);
        }

        let (run, created_run) = if let Some(run) = run {
            (run, false)
        } else {
            let description = record
                .description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map_or_else(|| default_description(&checked), str::to_owned);
            let source = given_source.as_ref().unwrap_or(&checked[0].source);
            let id = self
                .sidecar
                .begin_run(source, &description)
                .map_err(|e| sidecar_error(&e))?;
            (self.existing_run(id.0)?, true)
        };

        // With the audio at hand, Measurements made from other audio are
        // found out here, as the Stems commands find them out.
        let mut marked_stale = Vec::new();
        let mut seen: Vec<&String> = tracks.keys().collect();
        seen.sort();
        for track_id in seen {
            let (found, fingerprint) = &tracks[track_id];
            let (_, stale) = self.stems.observe_audio(found, fingerprint.as_ref())?;
            marked_stale.extend(stale);
        }

        let new: Vec<NewFact> = checked
            .iter()
            .map(|fact| NewFact {
                track_id: &fact.track_id,
                fingerprint: tracks
                    .get(&fact.track_id)
                    .and_then(|(_, fingerprint)| fingerprint.as_ref()),
                kind: fact.kind.kind(),
                value: fact.value.clone(),
                source: &fact.source,
                confidence: fact.confidence,
                run: run.id,
            })
            .collect();
        let written = self.sidecar.record_many(&new);
        let run = if created_run {
            let outcome = match &written {
                Ok(facts) if facts.len() == 1 => "recorded 1 Fact".to_owned(),
                Ok(facts) => format!("recorded {} Facts", facts.len()),
                Err(error) => format!("nothing recorded: {error}"),
            };
            let _ = self.sidecar.finish_run(run.id, &outcome);
            self.existing_run(run.id.0)?
        } else {
            run
        };
        let facts: Vec<Fact> = written
            .map_err(|e| sidecar_error(&e))?
            .into_iter()
            .map(Fact::from)
            .collect();
        for fact in &facts {
            self.changed(&fact.track_id, &fact.kind, Some(fact.id), &[]);
        }
        Ok(Recorded {
            run,
            created_run,
            facts,
            marked_stale,
        })
    }

    fn of_track(
        &self,
        wanted: &FactsOfTrack,
        lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
    ) -> Result<TrackFacts, ServiceError> {
        let found = lookup(&wanted.id)?;
        let kinds = self.kinds();
        if let Some(kind) = &wanted.kind {
            let kinds = kinds.as_ref().map_err(Clone::clone)?;
            // While the Tag schema has a mistake its dimensions are not
            // declared, and their Facts are still to be read.
            if kinds.get(kind).is_none() && !kinds.schema_is_broken() {
                return Err(not_declared(kind, kinds));
            }
        }
        // What is stored is read whatever state the files are in; a value
        // from when its kind took another shape is given as the kind takes
        // it now.
        let as_now = |mut fact: Fact| {
            if let Some(kind) = kinds.as_ref().ok().and_then(|kinds| kinds.get(&fact.kind)) {
                fact.value = kind.normalise(std::mem::take(&mut fact.value));
            }
            fact
        };
        let track = found.parent().clone();
        let fingerprint = stems::fingerprint(&track, found.path()).ok();
        let (attachments, _) = self.stems.observe_audio(&found, fingerprint.as_ref())?;
        let facts = match &wanted.kind {
            Some(kind) => self.sidecar.facts(&track.id, kind),
            None => self.sidecar.facts_of_track(&track.id),
        }
        .map_err(|e| sidecar_error(&e))?;
        let facts: Vec<Fact> = facts.into_iter().map(Fact::from).map(as_now).collect();
        let stale = facts.iter().filter(|fact| fact.stale).cloned().collect();
        let unverified = facts
            .iter()
            .filter(|fact| {
                !fact.stale && fact.nature == Nature::Measurement && fingerprint.is_none()
            })
            .cloned()
            .collect();
        Ok(TrackFacts {
            track,
            stale,
            unverified,
            reattached: attachments.reattached,
            review: attachments.review,
            facts: facts
                .into_iter()
                .filter(|fact| {
                    wanted.all
                        || (!fact.superseded
                            && !fact.stale
                            && (fact.nature == Nature::Judgement || fingerprint.is_some()))
                })
                .collect(),
        })
    }

    fn changed(&self, track_id: &str, kind: &str, fact_id: Option<i64>, stale: &[i64]) {
        self.stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(SidecarChanged {
                track_id: track_id.to_owned(),
                kind: kind.to_owned(),
                fact_id,
                stale: stale.to_vec(),
            }),
        );
    }
}

/// Checks one Fact against its kind's declaration, its source, its
/// confidence and the Library, writing nothing.
fn check(
    fact: FactToRecord,
    kinds: &Kinds,
    default_source: Option<&Source>,
    default_confidence: Option<f64>,
    tracks: &mut HashMap<String, (ParentAudio, Option<Fingerprint>)>,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Checked, ServiceError> {
    // A Tag schema with a mistake holds back the kinds it governs, and
    // only those.
    if let Some(error) = kinds.refused_by_schema(&fact.kind) {
        return Err(sidecar_error(&error));
    }
    let Some(kind) = kinds.get(&fact.kind) else {
        return Err(not_declared(&fact.kind, kinds));
    };
    let mut fact = fact;
    fact.value = kind.normalise(std::mem::take(&mut fact.value));
    if kind.reserved {
        return Err(ServiceError::new(
            codes::KIND_RESERVED,
            format!(
                "{} is recorded only by the command that makes it, not as a general Fact",
                kind.name
            ),
        ));
    }
    kind.check(&fact.value)
        .map_err(|why| ServiceError::new(codes::WRONG_VALUE_TYPE, why))?;
    let source = match &fact.source {
        Some(given) => source(given)?,
        None => default_source
            .cloned()
            .ok_or_else(|| ServiceError::new(codes::INVALID_SOURCE, "the Fact has no source"))?,
    };
    let confidence = fact
        .confidence
        .or(default_confidence)
        .or((source.kind == SourceKind::Dj).then_some(1.0))
        .ok_or_else(|| {
            ServiceError::new(
                codes::INVALID_CONFIDENCE,
                "say how sure the source is: a confidence from 0 to 1",
            )
        })?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(ServiceError::new(
            codes::INVALID_CONFIDENCE,
            format!("a confidence is from 0 to 1, not {confidence}"),
        ));
    }
    if !tracks.contains_key(&fact.track) {
        let found = lookup(&fact.track)?;
        // The fingerprint goes with every Fact when the audio is at
        // hand; only a Measurement cannot do without it.
        let fingerprint = if found.path().is_file() {
            Some(stems::fingerprint(found.parent(), found.path())?)
        } else {
            None
        };
        tracks.insert(fact.track.clone(), (found, fingerprint));
    }
    let (found, fingerprint) = &tracks[&fact.track];
    let parent = found.parent();
    if kind.nature == Nature::Measurement && fingerprint.is_none() {
        return Err(ServiceError::new(
            stems::codes::TRACK_FILE_MISSING,
            format!(
                "{} is a Measurement, tied to the audio it was made from, and the audio file of track {} \"{}\" is missing",
                kind.name, parent.id, parent.title
            ),
        ));
    }
    Ok(Checked {
        track_id: parent.id.clone(),
        kind: kind.clone(),
        value: fact.value,
        source,
        confidence,
    })
}

/// A source as given, checked: every source but the DJ has a name.
pub(crate) fn source(given: &SourceArg) -> Result<Source, ServiceError> {
    let name = given.name.trim();
    if !name.is_empty() {
        return Ok(Source::new(given.kind, name));
    }
    if given.kind == SourceKind::Dj {
        return Ok(Source::new(SourceKind::Dj, "dj"));
    }
    Err(ServiceError::new(
        codes::INVALID_SOURCE,
        "a source that is not the DJ needs a name: which agent, tool or web address it is",
    ))
}

fn not_declared(kind: &str, kinds: &Kinds) -> ServiceError {
    let known: Vec<&str> = kinds.all().iter().map(|kind| kind.name.as_str()).collect();
    ServiceError::new(
        codes::KIND_NOT_DECLARED,
        format!(
            "{kind} is not a declared kind of Fact. Declared: {}. Declare another in {}",
            known.join(", "),
            kinds.file().display()
        ),
    )
}

/// What a Run made for one command is called when nobody said.
fn default_description(facts: &[Checked]) -> String {
    if let [fact] = facts {
        return format!("Record {} for track {}", fact.kind.name, fact.track_id);
    }
    let mut tracks: Vec<&str> = facts.iter().map(|fact| fact.track_id.as_str()).collect();
    tracks.sort_unstable();
    tracks.dedup();
    format!(
        "Record {} Facts about {} {}",
        facts.len(),
        tracks.len(),
        if tracks.len() == 1 { "track" } else { "tracks" }
    )
}

pub(crate) fn sidecar_error(error: &SidecarError) -> ServiceError {
    match error {
        SidecarError::KindsFile { .. } => {
            ServiceError::new(codes::KINDS_FILE_INVALID, error.to_string())
        }
        SidecarError::TagSchema { .. } => {
            ServiceError::new(crate::schema::codes::TAG_SCHEMA_INVALID, error.to_string())
        }
        _ => ServiceError::new(
            stems::codes::SIDECAR_FAILED,
            format!("the Sidecar failed: {error}"),
        ),
    }
}
