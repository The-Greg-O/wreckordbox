//! Lookups without a network (wbx-37, ADR 0005): wreckordbox describes a
//! track for an agent to search for, and checks a candidate the agent
//! found against it. It never fetches anything; the agent, with its own
//! network access, does the looking up and hands the candidate over.
//!
//! - `lookup.describe` answers a [`TrackDescription`]: what the Library has
//!   (artist, title, mix name, duration, label, release date) and the
//!   audio file's own tags. A read: it writes nothing anywhere and does not
//!   open the Sidecar.
//! - `lookup.check` compares a candidate with the track by the rules of
//!   [`crate::lookup_match`] and answers accepted, held for review, or
//!   rejected, with the reason. Each check is recorded as a Decision
//!   (topic [`TOPIC`], candidates `accepted`, `held`, `rejected`) answered
//!   by [`DECIDER`]. When accepted, the candidate's fields become Facts
//!   whose source is `web:<address>`, waiting like any Fact from a source
//!   that is not the DJ until Projection or Review. A held or rejected
//!   candidate makes no Fact. The Decision, its answer and the Facts are
//!   one Run (described with the address and the date) and one Sidecar
//!   transaction. With `dry_run` the check writes nothing at all.
//!
//! Neither writes to the Library: no edit path, no `library_changed`, no
//! generation change. The Library is only read.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use wreckordbox_sidecar::decisions::{Candidate, NewDecision, QuestionType, RunOfAnswer};
use wreckordbox_sidecar::lookups::{LookupCheck, LookupFact};
use wreckordbox_sidecar::{Kinds, Nature, RunId, Source, SourceKind};

use crate::decisions::Decision;
use crate::facts::{self, Fact, Run};
use crate::library::OpenLibrary;
use crate::lookup_match::{self, Finding, Side, Verdict};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, ParentAudio, Stems};

/// The names of the lookup commands.
pub mod commands {
    /// A track described for a search. Takes a
    /// [`DescribeTrack`](super::DescribeTrack), answers a
    /// [`TrackDescription`](super::TrackDescription).
    pub const LOOKUP_DESCRIBE: &str = "lookup.describe";
    /// A candidate checked against a track. Takes a
    /// [`CheckCandidate`](super::CheckCandidate), answers a
    /// [`CandidateChecked`](super::CandidateChecked).
    pub const LOOKUP_CHECK: &str = "lookup.check";
}

/// The error codes of the lookup commands, beside the service's own.
/// Nothing is recorded when either is answered.
pub mod codes {
    /// The candidate cannot be checked: no artist or no title, a field
    /// that is not text where text is needed, a duration that is not one.
    pub const INVALID_CANDIDATE: &str = "invalid_candidate";
    /// The address or the date of the lookup is missing or malformed.
    pub const INVALID_LOOKUP: &str = "invalid_lookup";
}

/// The topic of a check's Decision.
pub const TOPIC: &str = "lookup_match";

/// The name of the Decider that answers a check's Decision, as a tool.
pub const DECIDER: &str = "lookup-check";

/// The arguments of `lookup.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribeTrack {
    /// The track's identifier, as every track listing gives it.
    pub id: String,
}

/// A track as an agent searches for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackDescription {
    pub track: String,
    /// As the Library credits it. Empty when it has none, as for the other
    /// texts.
    pub artist: String,
    /// The credit split into names (at commas, ampersands, "feat." and the
    /// like), in its order.
    pub artists: Vec<String>,
    pub title: String,
    pub mix_name: String,
    /// Whole seconds; 0 when the Library does not know.
    pub duration_seconds: u32,
    pub label: String,
    /// As the Library has it, usually `YYYY-MM-DD`; `null` when it has
    /// none.
    pub release_date: Option<String>,
    /// The release year; `null` when unknown.
    pub year: Option<u16>,
    pub album: String,
    pub genre: String,
    /// What to type into a search box: `Artist - Title (Mix Name)`.
    pub search: String,
    /// The audio file and its own tags.
    pub file: FileDescription,
}

/// The track's audio file and the tags written in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDescription {
    /// Where the Library says the file is.
    pub path: String,
    /// The tags as the file has them; `null` when they cannot be read.
    pub tags: Option<FileTags>,
    /// Why there are no tags: the file is not there, or cannot be read.
    /// `null` when they were read.
    pub problem: Option<String>,
}

/// An audio file's own tags. Empty text when the file has no such tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub label: String,
    pub comment: String,
    pub year: Option<u16>,
    pub key: String,
    pub duration_seconds: u32,
}

/// The arguments of `lookup.check`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckCandidate {
    /// The track the candidate may be.
    pub id: String,
    /// The candidate's fields, as found: `artist` and `title` (required),
    /// `mix_name`, `duration` (seconds, or `m:ss`), `label`,
    /// `release_date` (`YYYY-MM-DD`, or a year), and any other field named
    /// as a declared kind of Fact (`genre`, ...). If accepted, every field
    /// but `duration` becomes a Fact of the kind of the same name
    /// (`mix_name` is the kind `version`).
    pub candidate: Map<String, Value>,
    /// Where the candidate was found: the source of its Facts.
    pub address: String,
    /// When it was found, `YYYY-MM-DD`.
    pub date: String,
    /// How far apart the durations may be, in seconds. Left out:
    /// [`DEFAULT_TOLERANCE_SECONDS`](crate::lookup_match::DEFAULT_TOLERANCE_SECONDS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance_seconds: Option<f64>,
    /// The open Run the check belongs to. Left out: a Run is made for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// Check and say what would be recorded, recording nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// A field of an accepted candidate as a Fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedFact {
    /// The candidate's field.
    pub field: String,
    pub kind: String,
    pub value: Value,
}

/// A field of an accepted candidate that does not become a Fact, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotRecorded {
    pub field: String,
    pub kind: String,
    /// An error code: `kind_not_declared`, `wrong_value_type`, ...
    pub code: String,
    pub message: String,
}

/// What `lookup.check` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateChecked {
    pub track: String,
    pub verdict: Verdict,
    /// Why, in a sentence or two.
    pub reason: String,
    /// What each field's comparison found.
    pub findings: Vec<Finding>,
    pub address: String,
    pub date: String,
    pub dry_run: bool,
    /// The Decision the check is recorded as, with its answer; `null` on a
    /// dry run.
    pub decision: Option<Decision>,
    /// The Run it belongs to; `null` on a dry run.
    pub run: Option<Run>,
    /// The Facts the candidate's fields became; empty unless accepted, and
    /// on a dry run.
    pub facts: Vec<Fact>,
    /// The fields that become (on a dry run, would become) Facts.
    pub proposed: Vec<ProposedFact>,
    /// The fields that do not, and why.
    pub not_recorded: Vec<NotRecorded>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::LOOKUP_DESCRIBE | commands::LOOKUP_CHECK)
}

/// Answers one of this module's commands. `describe` reads a track from
/// the Library; `lookup` finds it as every Fact producer does. Both only
/// read the Library.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    describe: &dyn Fn(&str) -> Result<TrackDescription, ServiceError>,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<Value, ServiceError> {
    let answer = if request.command == commands::LOOKUP_DESCRIBE {
        let wanted: DescribeTrack = facts::arguments(request)?;
        let mut described = describe(&wanted.id)?;
        described.file = file_of(&described.file.path);
        serde_json::to_value(described)
    } else {
        serde_json::to_value(check(stems, &facts::arguments(request)?, describe, lookup)?)
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// The track as the Library has it, with its file's tags not yet read
/// (the file is read with the Library free: [`file_of`]).
pub(crate) fn described(library: &OpenLibrary, id: &str) -> Result<TrackDescription, ServiceError> {
    let index = library.index()?;
    let Some(row) = index.row_of(id) else {
        return Err(ServiceError::new(
            service_codes::TRACK_NOT_FOUND,
            format!("there is no track {id} in the Library"),
        ));
    };
    let record = crate::tracks::record(&index, row);
    let mix_name = index.mix_name(row).trim().to_owned();
    let release_date = index.release_date.get(row as usize).trim().to_owned();
    let search = search_text(&record.artist, &record.title, &mix_name);
    Ok(TrackDescription {
        track: record.id,
        artists: lookup_match::artists(&record.artist),
        artist: record.artist,
        title: record.title,
        mix_name,
        duration_seconds: record.duration_seconds,
        label: record.label,
        year: record.year.or_else(|| year_of(&release_date)),
        release_date: (!release_date.is_empty()).then_some(release_date),
        album: record.album,
        genre: record.genre,
        search,
        file: FileDescription {
            path: record.location,
            tags: None,
            problem: None,
        },
    })
}

fn search_text(artist: &str, title: &str, mix_name: &str) -> String {
    let (_, in_title) = lookup_match::split_title(title);
    let mut text = if artist.trim().is_empty() {
        title.trim().to_owned()
    } else {
        format!("{} - {}", artist.trim(), title.trim())
    };
    if !mix_name.is_empty() && in_title.is_none() {
        text.push_str(" (");
        text.push_str(mix_name);
        text.push(')');
    }
    text
}

/// The file at `path` and its tags, read only.
fn file_of(path: &str) -> FileDescription {
    let file = Path::new(path);
    let (tags, problem) = if file.is_file() {
        match rbl_db::import::read_tags(file) {
            Ok(tags) => (
                Some(FileTags {
                    title: tags.title,
                    artist: tags.artist,
                    album: tags.album,
                    genre: tags.genre,
                    label: tags.label,
                    comment: tags.comment,
                    year: (tags.year > 0).then_some(tags.year),
                    key: tags.key,
                    duration_seconds: tags.duration_sec,
                }),
                None,
            ),
            Err(e) => (None, Some(format!("the file's tags cannot be read: {e}"))),
        }
    } else {
        (None, Some("the audio file is not there".to_owned()))
    };
    FileDescription {
        path: path.to_owned(),
        tags,
        problem,
    }
}

/// The year a release date starts with, when it starts with one.
fn year_of(date: &str) -> Option<u16> {
    let digits = date.get(..4)?;
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|year| *year > 0)
}

fn invalid_candidate(message: impl Into<String>) -> ServiceError {
    ServiceError::new(codes::INVALID_CANDIDATE, message)
}

/// A text field of the candidate: empty when it is left out or null.
fn text_field(candidate: &Map<String, Value>, field: &str) -> Result<String, ServiceError> {
    match candidate.get(field) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.trim().to_owned()),
        Some(other) => Err(invalid_candidate(format!(
            "the candidate's {field} is text, not {other}"
        ))),
    }
}

/// The candidate as the rules compare it.
fn candidate_side(candidate: &Map<String, Value>) -> Result<Side, ServiceError> {
    let artist = text_field(candidate, "artist")?;
    let title = text_field(candidate, "title")?;
    if artist.is_empty() || title.is_empty() {
        return Err(invalid_candidate(
            "a candidate needs at least an artist and a title to be checked",
        ));
    }
    let duration_seconds = match candidate.get("duration") {
        None | Some(Value::Null) => None,
        Some(given) => Some(lookup_match::duration_of(given).ok_or_else(|| {
            invalid_candidate(format!(
                "the candidate's duration is seconds or m:ss, not {given}"
            ))
        })?),
    };
    let year = match candidate.get("release_date") {
        None | Some(Value::Null) => None,
        Some(Value::String(date)) => Some(year_of(date.trim()).ok_or_else(|| {
            invalid_candidate(format!(
                "the candidate's release_date starts with its year (YYYY-MM-DD), not \"{date}\""
            ))
        })?),
        Some(Value::Number(year)) => Some(
            year.as_u64()
                .and_then(|year| u16::try_from(year).ok())
                .filter(|year| *year > 0)
                .ok_or_else(|| {
                    invalid_candidate(format!("the candidate's release_date {year} is not a year"))
                })?,
        ),
        Some(other) => {
            return Err(invalid_candidate(format!(
                "the candidate's release_date is a date, not {other}"
            )))
        }
    };
    Ok(Side {
        artist,
        title,
        mix_name: text_field(candidate, "mix_name")?,
        duration_seconds,
        year,
        label: text_field(candidate, "label")?,
    })
}

fn track_side(track: &TrackDescription) -> Side {
    Side {
        artist: track.artist.clone(),
        title: track.title.clone(),
        mix_name: track.mix_name.clone(),
        duration_seconds: (track.duration_seconds > 0).then(|| f64::from(track.duration_seconds)),
        year: track.year,
        label: track.label.clone(),
    }
}

/// A plain `YYYY-MM-DD`.
fn plain_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let number = |text: &str, length: usize| {
        (text.len() == length && text.chars().all(|c| c.is_ascii_digit()))
            .then(|| text.parse::<u32>().ok())
            .flatten()
    };
    matches!(
        (number(year, 4), number(month, 2), number(day, 2)),
        (Some(1..), Some(1..=12), Some(1..=31))
    )
}

/// The kind of Fact a candidate's field becomes.
fn kind_of(field: &str) -> &str {
    if field == "mix_name" {
        "version"
    } else {
        field
    }
}

/// Splits an accepted candidate's fields into those that become Facts
/// (checked as `facts.record` checks a value) and those that do not.
fn proposed(
    candidate: &Map<String, Value>,
    kinds: &Kinds,
) -> (Vec<ProposedFact>, Vec<NotRecorded>) {
    let mut yes = Vec::new();
    let mut no = Vec::new();
    for (field, value) in candidate {
        if field == "duration"
            || value.is_null()
            || value.as_str().is_some_and(|t| t.trim().is_empty())
        {
            continue;
        }
        let kind = kind_of(field);
        let refuse = |code: &str, message: String| NotRecorded {
            field: field.clone(),
            kind: kind.to_owned(),
            code: code.to_owned(),
            message,
        };
        if let Some(error) = kinds.refused_by_schema(kind) {
            let refused = facts::sidecar_error(&error);
            no.push(refuse(&refused.code, refused.message));
            continue;
        }
        let Some(declared) = kinds.get(kind) else {
            let refused = facts::not_declared(kind, kinds);
            no.push(refuse(&refused.code, refused.message));
            continue;
        };
        if declared.reserved {
            no.push(refuse(
                facts::codes::KIND_RESERVED,
                format!("{kind} is recorded only by the command that makes it"),
            ));
            continue;
        }
        if declared.nature == Nature::Measurement {
            no.push(refuse(
                facts::codes::KIND_RESERVED,
                format!("{kind} is a Measurement, made from the audio, not looked up"),
            ));
            continue;
        }
        let value = declared.normalise(match value {
            Value::String(text) => Value::String(text.trim().to_owned()),
            other => other.clone(),
        });
        if let Err(why) = declared.check(&value) {
            no.push(refuse(facts::codes::WRONG_VALUE_TYPE, why));
            continue;
        }
        yes.push(ProposedFact {
            field: field.clone(),
            kind: declared.name.clone(),
            value,
        });
    }
    (yes, no)
}

fn check(
    stems: &Arc<Stems>,
    given: &CheckCandidate,
    describe: &dyn Fn(&str) -> Result<TrackDescription, ServiceError>,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
) -> Result<CandidateChecked, ServiceError> {
    let address = given.address.trim().to_owned();
    if address.is_empty() || address.chars().any(char::is_whitespace) {
        return Err(ServiceError::new(
            codes::INVALID_LOOKUP,
            format!(
                "the address the candidate came from is one address with no spaces, not \"{}\"",
                given.address
            ),
        ));
    }
    let date = given.date.trim().to_owned();
    if !plain_date(&date) {
        return Err(ServiceError::new(
            codes::INVALID_LOOKUP,
            format!(
                "the date of the lookup is YYYY-MM-DD, not \"{}\"",
                given.date
            ),
        ));
    }
    let tolerance = given
        .tolerance_seconds
        .unwrap_or(lookup_match::DEFAULT_TOLERANCE_SECONDS);
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("a tolerance is a number of seconds, 0 or more, not {tolerance}"),
        ));
    }
    let candidate = candidate_side(&given.candidate)?;
    let track = describe(&given.id)?;
    let checked = lookup_match::check(&track_side(&track), &candidate, tolerance);
    let reason = checked.reason();
    let kinds = Kinds::load(stems.sidecar_dir()).map_err(|e| facts::sidecar_error(&e))?;
    let (proposed, not_recorded) = if checked.verdict == Verdict::Accepted {
        proposed(&given.candidate, &kinds)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut answer = CandidateChecked {
        track: track.track.clone(),
        verdict: checked.verdict,
        reason,
        findings: checked.findings,
        address,
        date,
        dry_run: given.dry_run,
        decision: None,
        run: None,
        facts: Vec::new(),
        proposed,
        not_recorded,
    };
    if given.dry_run {
        return Ok(answer);
    }
    record(stems, given, &track, tolerance, lookup, &mut answer)?;
    Ok(answer)
}

/// Records a check that has been made: its Decision, its answer and its
/// Facts, then tells clients.
fn record(
    stems: &Arc<Stems>,
    given: &CheckCandidate,
    track: &TrackDescription,
    tolerance: f64,
    lookup: &dyn Fn(&str) -> Result<ParentAudio, ServiceError>,
    answer: &mut CandidateChecked,
) -> Result<(), ServiceError> {
    let sidecar = stems.sidecar()?;
    if let Some(run) = given.run {
        facts::open_run(stems, run)?;
    }
    let decider = Source::new(SourceKind::Tool, DECIDER);
    let from = Source::new(SourceKind::Web, answer.address.clone());
    let found = lookup(&track.track)?;
    let fingerprint = if answer.proposed.is_empty() || !found.path().is_file() {
        None
    } else {
        Some(stems::fingerprint(found.parent(), found.path())?)
    };
    if !answer.proposed.is_empty() {
        stems.before_record(&found, fingerprint.as_ref(), given.run.map(RunId))?;
    }
    let kinds = Kinds::load(stems.sidecar_dir()).map_err(|e| facts::sidecar_error(&e))?;
    let declared = answer
        .proposed
        .iter()
        .map(|fact| {
            kinds
                .get(&fact.kind)
                .cloned()
                .ok_or_else(|| facts::not_declared(&fact.kind, &kinds))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let lookup_facts: Vec<LookupFact> = answer
        .proposed
        .iter()
        .zip(&declared)
        .map(|(fact, declaration)| LookupFact {
            kind: declaration.kind(),
            value: fact.value.clone(),
            source: &from,
            confidence: lookup_match::CHECK_PROBABILITY,
        })
        .collect();
    let context = json!({
        "track": {
            "artist": track.artist,
            "title": track.title,
            "mix_name": track.mix_name,
            "duration_seconds": track.duration_seconds,
            "label": track.label,
            "release_date": track.release_date,
            "year": track.year,
        },
        "candidate": Value::Object(given.candidate.clone()),
        "address": answer.address,
        "date": answer.date,
        "tolerance_seconds": tolerance,
    });
    let question = format!(
        "Is the recording at {} the track {}, \"{}\"?",
        answer.address, track.track, track.search
    );
    let candidates = [
        Candidate {
            name: Verdict::Accepted.as_str().to_owned(),
            description: Some("the same recording: its fields become Facts".to_owned()),
        },
        Candidate {
            name: Verdict::Held.as_str().to_owned(),
            description: Some(
                "perhaps the same recording, perhaps another mix, edit or master: for the DJ to review"
                    .to_owned(),
            ),
        },
        Candidate {
            name: Verdict::Rejected.as_str().to_owned(),
            description: Some("another recording".to_owned()),
        },
    ];
    let description = format!(
        "Lookup check of track {}: {}, looked up on {}",
        track.track, answer.address, answer.date
    );
    let recorded = sidecar
        .record_lookup_check(&LookupCheck {
            decision: NewDecision {
                question_type: QuestionType::Choice,
                topic: Some(TOPIC),
                question: &question,
                context: &context,
                candidates: &candidates,
                track_id: Some(&track.track),
                run: None,
            },
            decider: &decider,
            choice: answer.verdict.as_str(),
            choice_probability: lookup_match::CHECK_PROBABILITY,
            reason: &answer.reason,
            track_id: &found.parent().id,
            fingerprint: fingerprint.as_ref(),
            facts: &lookup_facts,
            run: match given.run {
                Some(run) => RunOfAnswer::Open(RunId(run)),
                None => RunOfAnswer::Made {
                    description: &description,
                },
            },
        })
        .map_err(|e| crate::decisions::sidecar_error(&e))?;
    let facts: Vec<Fact> = recorded.facts.into_iter().map(Fact::from).collect();
    for fact in &facts {
        // The check stands whatever becomes of the stale-marking, which the
        // next command about the track's audio does again.
        let _ = facts::recorded(stems, fact, fingerprint.as_ref());
    }
    stems.publish(
        stems::events::SIDECAR_CHANGED,
        json!({
            "track_id": recorded.decision.track_id.clone().unwrap_or_default(),
            "kind": crate::decisions::CHANGED_KIND,
            "fact_id": null,
            "stale": [],
            "decision_id": recorded.decision.id,
            "answer_id": recorded.answer.id,
        }),
    );
    answer.run = sidecar
        .run(recorded.run)
        .map_err(|e| facts::sidecar_error(&e))?;
    answer.decision = Some(recorded.decision);
    answer.facts = facts;
    Ok(())
}
