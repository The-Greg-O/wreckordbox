//! Set plans from Recipes (wbx-39): `setplans.check`, `setplans.build`,
//! `setplans.show`, `setplans.list`.
//!
//! The Recipe format and the sequencer are `wreckordbox-setplan`'s, pure;
//! this module gathers what the sequencer needs about each track of the
//! pool and keeps the plan in the Sidecar.
//!
//! - **Tempo, key, title, date added** come from the Library's index (the
//!   Library's lock is held only for that lookup). A key is read with the
//!   index's own spelling rules (`rbl_index::key::camelot_rank`).
//! - **Situations and genre** are the standing Facts (`facts.standing`'s
//!   rule) of the kinds `situation` and `genre`; with no genre Fact, the
//!   Library's genre text. A track with no Situation Fact is unknown, not
//!   "none".
//! - **Intensity** is `summary.intensity` of the track's current Intensity:
//!   a live Fact from `wreckordbox-intensity`, valid for this method
//!   version, of the audio the file holds now (each such track's file is
//!   fingerprinted once; nothing is recorded or marked).
//!
//! Only `setplans.build` writes, and only the Sidecar: one Run and one
//! `set_plan` row, in one transaction. It never touches the Library (no
//! `library_changed`, no generation, no `tell_host`), and like every Sidecar
//! recording it is refused for a Library hosted on a Stick
//! (`sidecar_on_stick`, `sidecar_stick::RECORDS`). The reads only read.
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_setplan::recipe::Problem;
use wreckordbox_setplan::sequence::{self, Candidate, Lack, Plan, Unsatisfiable};
use wreckordbox_setplan::{Camelot, Recipe};
use wreckordbox_sidecar::precedence::Rules;
use wreckordbox_sidecar::set_plans::NewSetPlan;
pub use wreckordbox_sidecar::set_plans::SetPlan;
use wreckordbox_sidecar::{Fact, TagSchema, TAG_SCHEMA_FILE};

use crate::facts::{self, Source};
use crate::library::OpenLibrary;
use crate::mutant_reads::{inspect, Store};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Stems};
use crate::tracks::TrackSelection;

pub mod commands {
    /// Check a Recipe: `{"recipe": text}` → [`Checked`](super::Checked).
    pub const SETPLANS_CHECK: &str = "setplans.check";
    /// Build and keep a Set plan → [`Built`](super::Built).
    pub const SETPLANS_BUILD: &str = "setplans.build";
    /// One kept Set plan: `{"id": N}`.
    pub const SETPLANS_SHOW: &str = "setplans.show";
    /// Every kept Set plan, newest first.
    pub const SETPLANS_LIST: &str = "setplans.list";
}

pub mod codes {
    /// The Recipe has mistakes; the message lists them by line.
    pub const RECIPE_INVALID: &str = "recipe_invalid";
    /// No Set plan with that identifier.
    pub const SET_PLAN_NOT_FOUND: &str = "set_plan_not_found";
}

pub mod events {
    /// `{"id", "run", "name", "tracks", "score"}`: a Set plan was kept.
    pub const SET_PLAN_BUILT: &str = "set_plan_built";
}

/// Arguments of `setplans.check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// The Recipe's text.
    pub recipe: String,
}

/// What `setplans.check` answers. A Recipe with mistakes is an answer
/// (`valid: false`), not an error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checked {
    pub valid: bool,
    pub problems: Vec<Problem>,
    /// The Recipe as read, when valid.
    pub recipe: Option<Value>,
}

/// Arguments of `setplans.build`. At least one of `ids` and `selection`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// The Recipe's text.
    pub recipe: String,
    /// Where it was read from, kept with the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_file: Option<String>,
    /// Tracks of the pool by identifier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// Tracks of the pool as `tracks.query` selects them (its `limit` and
    /// `offset` are not applied).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TrackSelection>,
    /// Who builds it: the Run's source (the DJ needs no name).
    pub source: facts::SourceArg,
}

/// What `setplans.build` answers. An unsatisfiable Recipe is an answer
/// (`built: false`), with nothing kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Built {
    pub built: bool,
    pub set_plan: Option<SetPlan>,
    pub unsatisfiable: Option<Unsatisfiable>,
}

/// Arguments of `setplans.show`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Show {
    pub id: i64,
}

/// One line of `setplans.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listed {
    pub id: i64,
    pub run: i64,
    pub name: String,
    pub recipe_file: Option<String>,
    pub created_at: i64,
    pub tracks: usize,
    pub score: Option<f64>,
}

/// What `setplans.list` answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    pub set_plans: Vec<Listed>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::SETPLANS_CHECK
            | commands::SETPLANS_BUILD
            | commands::SETPLANS_SHOW
            | commands::SETPLANS_LIST
    )
}

/// One track of the pool as the Library has it.
pub(crate) struct PoolTrack {
    id: String,
    title: String,
    bpm: Option<f64>,
    key: Option<Camelot>,
    genre: String,
    added: Option<String>,
    path: PathBuf,
    /// The DJ's Energy rating (stars), for `intensity.missing = "energy"`.
    energy: Option<u8>,
}

/// The pool's tracks, from the Library, in the order asked, each once.
pub(crate) fn pool(library: &OpenLibrary, build: &Build) -> Result<Vec<PoolTrack>, ServiceError> {
    let index = library.index()?;
    let mut rows = Vec::new();
    for id in &build.ids {
        let Some(row) = index.row_of(id) else {
            return Err(ServiceError::new(
                service_codes::TRACK_NOT_FOUND,
                format!("there is no track {id} in the Library"),
            ));
        };
        rows.push(row);
    }
    if let Some(selection) = &build.selection {
        rows.extend(crate::tracks::matching(&index, selection)?);
    }
    let mut seen = std::collections::BTreeSet::new();
    rows.retain(|row| seen.insert(*row));
    Ok(rows
        .into_iter()
        .map(|row| {
            let record = crate::tracks::record(&index, row);
            let found = stems::audio_at(&index, row);
            PoolTrack {
                bpm: record.bpm,
                key: Camelot::from_rank(rbl_index::key::camelot_rank(&record.key)),
                genre: record.genre,
                added: record.date_added,
                energy: record.energy,
                path: found.path().to_path_buf(),
                id: record.id,
                title: record.title,
            }
        })
        .collect())
}

/// Answers one of this module's commands. `lookup` finds the pool in the
/// Library and lets go of it before anything else is done.
pub(crate) fn answer(
    stems: &Stems,
    request: &Request,
    lookup: &dyn Fn(&Build) -> Result<Vec<PoolTrack>, ServiceError>,
) -> Result<Value, ServiceError> {
    let value = match request.command.as_str() {
        commands::SETPLANS_CHECK => {
            let check: Check = facts::arguments(request)?;
            to_value(&checked(stems, &check.recipe))
        }
        commands::SETPLANS_BUILD => {
            let wanted: Build = facts::arguments(request)?;
            to_value(&build(stems, &wanted, lookup)?)
        }
        commands::SETPLANS_SHOW => {
            let show: Show = facts::arguments(request)?;
            let found = inspect(stems, |store| match store {
                Store::Open(sidecar) => sidecar.set_plan(show.id),
                Store::Snapshot(reader) => reader.set_plan(show.id),
                Store::Absent => Ok(None),
            })?;
            match found {
                Some(plan) => to_value(&plan),
                None => {
                    return Err(ServiceError::new(
                        codes::SET_PLAN_NOT_FOUND,
                        format!("there is no Set plan {}", show.id),
                    ))
                }
            }
        }
        _ => {
            let plans = inspect(stems, |store| match store {
                Store::Open(sidecar) => sidecar.set_plans(),
                Store::Snapshot(reader) => reader.set_plans(),
                Store::Absent => Ok(Vec::new()),
            })?;
            to_value(&List {
                set_plans: plans
                    .into_iter()
                    .map(|plan| Listed {
                        id: plan.id,
                        run: plan.run,
                        tracks: plan.plan["tracks"].as_array().map_or(0, Vec::len),
                        score: plan.plan["score"].as_f64(),
                        name: plan.name,
                        recipe_file: plan.recipe_file,
                        created_at: plan.created_at,
                    })
                    .collect(),
            })
        }
    };
    value
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(value)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

/// Reads a Recipe and checks the Situations it names against the Tag
/// schema's `situation` dimension, when a valid schema has one.
fn read_recipe(stems: &Stems, text: &str) -> Result<Recipe, Vec<Problem>> {
    let recipe = Recipe::parse(text)?;
    let path = stems.sidecar_dir().join(TAG_SCHEMA_FILE);
    if !path.is_file() {
        return Ok(recipe);
    }
    // A schema with mistakes is `schema check`'s to report.
    let Ok(schema) = TagSchema::load(&path) else {
        return Ok(recipe);
    };
    let Some(dimension) = schema.dimension("situation") else {
        return Ok(recipe);
    };
    let known: Vec<&str> = dimension.terms.iter().map(|t| t.id.as_str()).collect();
    let mut problems = Vec::new();
    for name in recipe.situation_names() {
        if !known.contains(&name.as_str()) {
            let needle = format!("\"{name}\"");
            let line = text
                .lines()
                .position(|line| line.contains(&needle))
                .map(|at| at + 1);
            problems.push(Problem {
                line,
                message: format!(
                    "the Situation {name:?} is not a term of the Tag schema's situation dimension; it has: {}",
                    known.join(", ")
                ),
            });
        }
    }
    if problems.is_empty() {
        Ok(recipe)
    } else {
        problems.sort_by_key(|p| p.line.unwrap_or(0));
        Err(problems)
    }
}

fn checked(stems: &Stems, text: &str) -> Checked {
    match read_recipe(stems, text) {
        Ok(recipe) => Checked {
            valid: true,
            problems: Vec::new(),
            recipe: serde_json::to_value(&recipe).ok(),
        },
        Err(problems) => Checked {
            valid: false,
            problems,
            recipe: None,
        },
    }
}

fn build(
    stems: &Stems,
    wanted: &Build,
    lookup: &dyn Fn(&Build) -> Result<Vec<PoolTrack>, ServiceError>,
) -> Result<Built, ServiceError> {
    let recipe = read_recipe(stems, &wanted.recipe).map_err(|problems| {
        let listed: Vec<String> = problems.iter().map(ToString::to_string).collect();
        ServiceError::new(
            codes::RECIPE_INVALID,
            format!("the Recipe has mistakes: {}", listed.join("; ")),
        )
    })?;
    if wanted.ids.is_empty() && wanted.selection.is_none() {
        return Err(ServiceError::new(
            service_codes::BAD_REQUEST,
            "name the pool: track ids, a selection, or both",
        ));
    }
    if wanted.source.kind != facts::SourceKind::Dj && wanted.source.name.trim().is_empty() {
        return Err(ServiceError::new(
            facts::codes::INVALID_SOURCE,
            "a source other than the DJ needs a name",
        ));
    }
    let pool = lookup(wanted)?;
    let (candidates, why) = candidates(stems, &recipe, &pool)?;
    let pool_ids: Vec<String> = pool.iter().map(|t| t.id.clone()).collect();
    let explain = |left_out: &mut Vec<sequence::LeftOut>| {
        for out in left_out {
            if out.lacks.contains(&Lack::Intensity) {
                if let Some(why) = why.get(&out.id) {
                    out.message = format!("{} ({why})", out.message);
                }
            }
        }
    };
    match sequence::plan(&recipe, &candidates) {
        Err(mut unsatisfiable) => {
            explain(&mut unsatisfiable.left_out);
            Ok(Built {
                built: false,
                set_plan: None,
                unsatisfiable: Some(*unsatisfiable),
            })
        }
        Ok(mut plan) => {
            explain(&mut plan.left_out);
            let kept = keep(stems, wanted, &recipe, &pool_ids, &plan)?;
            stems.publish(
                events::SET_PLAN_BUILT,
                json!({
                    "id": kept.id,
                    "run": kept.run,
                    "name": kept.name,
                    "tracks": plan.tracks.len(),
                    "score": plan.score,
                }),
            );
            Ok(Built {
                built: true,
                set_plan: Some(kept),
                unsatisfiable: None,
            })
        }
    }
}

fn keep(
    stems: &Stems,
    wanted: &Build,
    recipe: &Recipe,
    pool: &[String],
    plan: &Plan,
) -> Result<SetPlan, ServiceError> {
    let text = serde_json::to_string(plan)
        .map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))?;
    let name = if recipe.name.is_empty() {
        wanted
            .recipe_file
            .as_deref()
            .and_then(|f| std::path::Path::new(f).file_stem())
            .map_or_else(|| "Recipe".to_owned(), |s| s.to_string_lossy().into_owned())
    } else {
        recipe.name.clone()
    };
    let description = format!(
        "Build a Set plan from the Recipe {name:?} ({} tracks from a pool of {})",
        recipe.tracks,
        pool.len()
    );
    let outcome = format!(
        "built a Set plan of {} tracks, score {}",
        plan.tracks.len(),
        plan.score
    );
    let source = Source {
        kind: wanted.source.kind,
        name: if wanted.source.kind == facts::SourceKind::Dj {
            "dj".to_owned()
        } else {
            wanted.source.name.clone()
        },
    };
    let sidecar = stems.sidecar()?;
    sidecar
        .record_set_plan(&NewSetPlan {
            source: &source,
            description: &description,
            name: &name,
            recipe: &wanted.recipe,
            recipe_file: wanted.recipe_file.as_deref(),
            pool,
            plan: &text,
            outcome: &outcome,
        })
        .map_err(|e| stems::sidecar_error(&e))
}

/// The sequencer's view of each pool track, and for each track with no
/// usable Intensity, why.
fn candidates(
    stems: &Stems,
    recipe: &Recipe,
    pool: &[PoolTrack],
) -> Result<(Vec<Candidate>, BTreeMap<String, String>), ServiceError> {
    let rules = Rules::load(stems.sidecar_dir())
        .map_err(|why| ServiceError::new(crate::standing::RULES_INVALID, why))?;
    let wants_intensity = recipe.intensity.is_some();
    // Facts first, through the open writer or a read-only snapshot; the
    // audio is read afterwards, with the Sidecar's slot free.
    let stored: Vec<(Vec<Fact>, Vec<Fact>, Vec<Fact>)> = inspect(stems, |store| {
        pool.iter()
            .map(|track| {
                Ok((
                    store.facts(&track.id, "situation")?,
                    store.facts(&track.id, "genre")?,
                    if wants_intensity {
                        store.facts(&track.id, "intensity")?
                    } else {
                        Vec::new()
                    },
                ))
            })
            .collect()
    })?;
    let mut why = BTreeMap::new();
    let mut out = Vec::with_capacity(pool.len());
    for (track, (situations, genres, intensities)) in pool.iter().zip(stored) {
        let situations = rules.standing(&situations).0.map(|fact| terms(&fact.value));
        let genres = rules
            .standing(&genres)
            .0
            .map(|fact| terms(&fact.value))
            .or_else(|| (!track.genre.trim().is_empty()).then(|| vec![track.genre.clone()]));
        let intensity = if wants_intensity {
            match current_intensity(track, &intensities) {
                Ok(value) => Some(value),
                Err(reason) => {
                    why.insert(track.id.clone(), reason);
                    None
                }
            }
        } else {
            None
        };
        out.push(Candidate {
            id: track.id.clone(),
            title: track.title.clone(),
            bpm: track.bpm,
            key: track.key,
            intensity,
            energy: track.energy,
            situations,
            genres,
            added: track.added.clone(),
        });
    }
    Ok((out, why))
}

/// A Fact's value as a list of terms (a single text is a list of one).
fn terms(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// The track's current Intensity summary, or why it has none.
fn current_intensity(track: &PoolTrack, facts: &[Fact]) -> Result<f64, String> {
    let ours: Vec<&Fact> = facts
        .iter()
        .filter(|fact| crate::intensity::is_ours(fact) && fact.is_live())
        .collect();
    if ours.is_empty() {
        return Err(format!(
            "never measured: `wreckordbox intensity measure {}` measures it",
            track.id
        ));
    }
    if !track.path.is_file() {
        return Err(format!(
            "its audio file is missing: there is no {}",
            track.path.display()
        ));
    }
    let audio = wreckordbox_sidecar::fingerprint_file(&track.path)
        .map_err(|e| format!("its audio file cannot be read: {e}"))?;
    let current: Vec<&Fact> = ours
        .into_iter()
        .filter(|fact| {
            fact.fingerprint
                .as_ref()
                .is_some_and(|stored| audio.matches_stored(stored))
        })
        .collect();
    if current.is_empty() {
        return Err(format!(
            "its audio has changed since it was measured: `wreckordbox intensity measure {}` measures it again",
            track.id
        ));
    }
    current
        .into_iter()
        .find_map(|fact| crate::intensity::valid(fact).map(|value| value.summary.intensity))
        .ok_or_else(|| {
            format!(
                "it was measured by an older method: `wreckordbox intensity measure {}` measures it again",
                track.id
            )
        })
}
