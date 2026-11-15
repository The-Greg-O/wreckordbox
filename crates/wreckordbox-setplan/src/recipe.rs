//! A Recipe: the DJ's written criteria for a journey through a set. A plain
//! TOML file the DJ versions and refines; this module reads and checks it.
//!
//! ```toml
//! version = 1
//! name = "Warm-up to peak"      # optional; for the DJ
//! tracks = 8                    # how many tracks the Set plan holds
//!
//! [intensity]                   # how Intensity should move across the set
//! shape = [                     # where in the set (0 first .. 1 last), the Intensity wanted there
//!   { at = 0.0, intensity = 0.3 },
//!   { at = 0.7, intensity = 0.9 },
//!   { at = 1.0, intensity = 0.5 },
//! ]
//! max_step = 0.3                # optional hard limit on the change between neighbours
//! missing = "leave_out"         # a track with no Intensity: "leave_out" (default) or "allow"
//! weight = 1.0                  # every section has one; default 1
//!
//! [tempo]
//! max_change = 4.0              # optional hard limit, in percent of the outgoing tempo
//!
//! [key]
//! max_moves = 1                 # optional hard limit, in moves on the Camelot wheel
//!
//! [situations]                  # which Situations belong where
//! place = [
//!   { situation = "opener", from = 0.0, to = 0.0, require = true },
//!   { situation = "closer", from = 0.9, to = 1.0 },
//! ]
//!
//! [recency]                     # favour recent additions
//! half_life_days = 90           # a track added this long before `as_of` counts half
//! as_of = "2026-11-01"          # optional; default: the newest addition in the pool
//!
//! [genre]
//! mix = "vary"                  # "vary": neighbours of different genres; "stay": the same
//! ```
//!
//! Positions are fractions of the set: with `tracks = 5` the positions are
//! 0, 0.25, 0.5, 0.75 and 1. A section left out is a criterion the Recipe
//! does not use: it neither scores nor limits. Hard limits (`max_step`,
//! `max_change`, `max_moves`, a place with `require = true`) are never
//! broken in a Set plan; the weights only rank the plans that keep them.
//!
//! There is no clock here: recency is measured to `as_of`, or to the
//! newest date added in the pool, so the same Recipe and pool always give
//! the same plan.
use std::fmt;

use serde::{Deserialize, Serialize};
use toml::Spanned;

/// The only version of the format there is.
pub const VERSION: i64 = 1;

/// One mistake in a Recipe: the line it is on when known, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// A Recipe that has passed every check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recipe {
    pub name: String,
    pub tracks: usize,
    pub intensity: Option<Intensity>,
    pub tempo: Option<Tempo>,
    pub key: Option<Key>,
    pub situations: Option<Situations>,
    pub recency: Option<Recency>,
    pub genre: Option<Genre>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Intensity {
    pub shape: Vec<Point>,
    pub max_step: Option<f64>,
    pub missing: Missing,
    /// How much a step between neighbours that differs from the step the
    /// shape wants costs, against the distance from the shape; default 0.5.
    pub step_weight: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Point {
    pub at: f64,
    pub intensity: f64,
}

/// What a track with no Intensity does when the Recipe has a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// Left out of the plan, and named in its `left_out`.
    #[default]
    LeaveOut,
    /// Used, with no Intensity score; its reasons say so.
    Allow,
    /// The DJ's Energy rating (1 to 5 stars) stands in, as (Energy - 1) / 4;
    /// a track with neither is left out.
    Energy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tempo {
    /// Percent, measured symmetrically: the larger tempo over the smaller.
    pub max_change: Option<f64>,
    /// Half and double time count as the same tempo (70 meets 140).
    pub fold: bool,
    /// The set's tempo range: a track outside it (folded, with `fold`) is
    /// left out.
    pub min_bpm: Option<f64>,
    pub max_bpm: Option<f64>,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Key {
    /// The most a move may cost (see [`crate::KeyStep::cost`]).
    pub max_moves: Option<u32>,
    /// The DJ plays with key lock (Master Tempo) on: a tempo change keeps
    /// the key. Off, the incoming key is shifted by the tempo change.
    pub key_lock: bool,
    /// Without key lock, the most detuning (cents) left after the shift.
    pub max_detune: Option<f64>,
    /// How much a same-key transition is marked down (0 to 1), so a set
    /// does not sit in one key when the pool allows otherwise.
    pub variety: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Situations {
    pub place: Vec<Place>,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Place {
    /// The Situation's term identifier, as its Facts hold it.
    pub situation: String,
    pub from: f64,
    pub to: f64,
    /// Every position from `from` to `to` must be a track of this Situation.
    pub require: bool,
}

impl Place {
    #[must_use]
    pub fn covers(&self, at: f64) -> bool {
        at >= self.from - 1e-9 && at <= self.to + 1e-9
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recency {
    pub half_life_days: f64,
    pub as_of: Option<String>,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Genre {
    pub mix: Mix,
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mix {
    /// Neighbours of different genres score.
    Vary,
    /// Neighbours sharing a genre score.
    Stay,
}

impl Intensity {
    /// The Intensity the shape wants at `at` (0..1): straight lines between
    /// the points, flat before the first and after the last.
    #[must_use]
    pub fn target(&self, at: f64) -> f64 {
        let first = self.shape[0];
        if at <= first.at {
            return first.intensity;
        }
        for pair in self.shape.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if at <= b.at {
                if (b.at - a.at).abs() < f64::EPSILON {
                    return b.intensity;
                }
                return a.intensity + (b.intensity - a.intensity) * (at - a.at) / (b.at - a.at);
            }
        }
        self.shape[self.shape.len() - 1].intensity
    }
}

impl Recipe {
    /// Reads and checks a Recipe. Every mistake found is reported, sorted
    /// by line.
    ///
    /// # Errors
    /// The text is not a valid Recipe; at least one [`Problem`].
    pub fn parse(text: &str) -> Result<Self, Vec<Problem>> {
        let raw: Raw = toml::from_str(text).map_err(|e| {
            let line = e.span().map(|span| line_of(text, span.start));
            vec![Problem {
                line,
                message: e.message().trim().to_owned(),
            }]
        })?;
        let mut check = Check {
            text,
            problems: Vec::new(),
        };
        let recipe = check.recipe(raw);
        let mut problems = check.problems;
        if problems.is_empty() {
            if let Some(recipe) = recipe {
                return Ok(recipe);
            }
        }
        problems.sort_by_key(|p| p.line.unwrap_or(0));
        Err(problems)
    }

    /// Every Situation the Recipe names, sorted, each once.
    #[must_use]
    pub fn situation_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .situations
            .iter()
            .flat_map(|s| s.place.iter().map(|p| p.situation.clone()))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Where position `index` of the set lies, 0 (first) to 1 (last).
    #[must_use]
    pub fn at(&self, index: usize) -> f64 {
        if self.tracks <= 1 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss, reason = "a set is far shorter than 2^52")]
            let at = index as f64 / (self.tracks - 1) as f64;
            at
        }
    }
}

// What the file says, before the checks.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: Option<Spanned<i64>>,
    #[serde(default)]
    name: String,
    tracks: Option<Spanned<i64>>,
    intensity: Option<Spanned<RawIntensity>>,
    tempo: Option<Spanned<RawTempo>>,
    key: Option<Spanned<RawKey>>,
    situations: Option<Spanned<RawSituations>>,
    recency: Option<Spanned<RawRecency>>,
    genre: Option<Spanned<RawGenre>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIntensity {
    shape: Option<Spanned<Vec<Spanned<RawPoint>>>>,
    max_step: Option<Spanned<f64>>,
    #[serde(default)]
    missing: Missing,
    step_weight: Option<Spanned<f64>>,
    weight: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPoint {
    at: f64,
    intensity: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTempo {
    max_change: Option<Spanned<f64>>,
    #[serde(default)]
    fold: bool,
    min_bpm: Option<Spanned<f64>>,
    max_bpm: Option<Spanned<f64>>,
    weight: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKey {
    max_moves: Option<Spanned<i64>>,
    key_lock: Option<bool>,
    max_detune: Option<Spanned<f64>>,
    variety: Option<Spanned<f64>>,
    weight: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSituations {
    #[serde(default)]
    place: Vec<Spanned<RawPlace>>,
    weight: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlace {
    situation: String,
    from: f64,
    to: f64,
    #[serde(default)]
    require: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecency {
    half_life_days: Option<Spanned<f64>>,
    as_of: Option<Spanned<String>>,
    weight: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGenre {
    mix: Option<Mix>,
    weight: Option<Spanned<f64>>,
}

struct Check<'a> {
    text: &'a str,
    problems: Vec<Problem>,
}

/// The longest set a Recipe may ask for; the sequencer's work grows with it.
pub const MOST_TRACKS: i64 = 200;

impl Check<'_> {
    fn at<T>(&mut self, spanned: &Spanned<T>, message: String) {
        let line = Some(line_of(self.text, spanned.span().start));
        self.problems.push(Problem { line, message });
    }

    fn nowhere(&mut self, message: String) {
        self.problems.push(Problem {
            line: None,
            message,
        });
    }

    fn fraction<T>(&mut self, spanned: &Spanned<T>, what: &str, value: f64) -> bool {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            true
        } else {
            self.at(
                spanned,
                format!("{what} is {value}; it must be from 0 to 1"),
            );
            false
        }
    }

    fn weight(&mut self, section: &str, weight: Option<&Spanned<f64>>) -> f64 {
        match weight {
            None => 1.0,
            Some(spanned) => {
                let value = *spanned.get_ref();
                if value.is_finite() && value >= 0.0 {
                    value
                } else {
                    self.at(
                        spanned,
                        format!("{section}.weight is {value}; a weight is 0 or more"),
                    );
                    1.0
                }
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one block per section of the file, in the file's order"
    )]
    fn recipe(&mut self, raw: Raw) -> Option<Recipe> {
        match &raw.version {
            None => self.nowhere(format!(
                "the Recipe has no `version`; write `version = {VERSION}` at the top"
            )),
            Some(version) if *version.get_ref() != VERSION => {
                let found = *version.get_ref();
                self.at(
                    version,
                    format!("this is a version {found} Recipe; only version {VERSION} is read"),
                );
            }
            Some(_) => {}
        }
        let tracks = match &raw.tracks {
            None => {
                self.nowhere(
                    "the Recipe does not say how many tracks the set holds; write `tracks = N`"
                        .to_owned(),
                );
                None
            }
            Some(spanned) => {
                let value = *spanned.get_ref();
                if (2..=MOST_TRACKS).contains(&value) {
                    usize::try_from(value).ok()
                } else {
                    self.at(
                        spanned,
                        format!(
                            "tracks is {value}; a set holds at least 2 tracks and at most {MOST_TRACKS}"
                        ),
                    );
                    None
                }
            }
        };
        let intensity = raw.intensity.map(|section| self.intensity(&section));
        let tempo = raw.tempo.map(|section| {
            let section = section.into_inner();
            let max_change = section.max_change.and_then(|limit| {
                let value = *limit.get_ref();
                if value.is_finite() && value > 0.0 {
                    Some(value)
                } else {
                    self.at(
                        &limit,
                        format!("tempo.max_change is {value}; it is a percentage above 0"),
                    );
                    None
                }
            });
            let mut bpm = |limit: Option<Spanned<f64>>, name: &str| {
                limit.and_then(|limit| {
                    let value = *limit.get_ref();
                    if value.is_finite() && value > 0.0 {
                        Some((value, limit))
                    } else {
                        self.at(&limit, format!("tempo.{name} is {value}; it is a tempo in BPM above 0"));
                        None
                    }
                })
            };
            let min_bpm = bpm(section.min_bpm, "min_bpm");
            let max_bpm = bpm(section.max_bpm, "max_bpm");
            if let (Some((low, _)), Some((high, spanned))) = (&min_bpm, &max_bpm) {
                if low > high {
                    self.at(
                        spanned,
                        format!("tempo.max_bpm is {high}, below tempo.min_bpm {low}"),
                    );
                }
            }
            Tempo {
                max_change,
                fold: section.fold,
                min_bpm: min_bpm.map(|(v, _)| v),
                max_bpm: max_bpm.map(|(v, _)| v),
                weight: self.weight("tempo", section.weight.as_ref()),
            }
        });
        let key = raw.key.map(|section| {
            let section = section.into_inner();
            let max_moves = section.max_moves.and_then(|limit| {
                let value = *limit.get_ref();
                match u32::try_from(value) {
                    Ok(moves) if moves <= 7 => Some(moves),
                    _ => {
                        self.at(
                            &limit,
                            format!(
                                "key.max_moves is {value}; it is from 0 (the same key) to 7 (any key)"
                            ),
                        );
                        None
                    }
                }
            });
            let key_lock = section.key_lock.unwrap_or(true);
            let max_detune = section.max_detune.and_then(|limit| {
                let value = *limit.get_ref();
                if key_lock {
                    self.at(
                        &limit,
                        "key.max_detune needs key.key_lock = false: with key lock a tempo change does not detune".to_owned(),
                    );
                    None
                } else if value.is_finite() && value > 0.0 && value <= 50.0 {
                    Some(value)
                } else {
                    self.at(
                        &limit,
                        format!("key.max_detune is {value}; it is in cents, above 0 and at most 50"),
                    );
                    None
                }
            });
            let variety = match section.variety {
                None => 0.0,
                Some(spanned) => {
                    let value = *spanned.get_ref();
                    if value.is_finite() && (0.0..=1.0).contains(&value) {
                        value
                    } else {
                        self.at(&spanned, format!("key.variety is {value}; it is from 0 to 1"));
                        0.0
                    }
                }
            };
            Key {
                max_moves,
                key_lock,
                max_detune,
                variety,
                weight: self.weight("key", section.weight.as_ref()),
            }
        });
        let situations = raw.situations.map(|section| {
            let section = section.into_inner();
            let mut place = Vec::new();
            for spanned in &section.place {
                let raw = spanned.get_ref();
                let mut fine = true;
                if raw.situation.trim().is_empty() {
                    self.at(
                        spanned,
                        "a place in situations.place has an empty situation name".to_owned(),
                    );
                    fine = false;
                }
                fine &= self.fraction(spanned, "situations.place from", raw.from);
                fine &= self.fraction(spanned, "situations.place to", raw.to);
                if fine && raw.from > raw.to {
                    self.at(
                        spanned,
                        format!(
                            "the place for {:?} runs from {} to {}; from must not be after to",
                            raw.situation, raw.from, raw.to
                        ),
                    );
                    fine = false;
                }
                if fine {
                    place.push(Place {
                        situation: raw.situation.clone(),
                        from: raw.from,
                        to: raw.to,
                        require: raw.require,
                    });
                }
            }
            Situations {
                place,
                weight: self.weight("situations", section.weight.as_ref()),
            }
        });
        let recency = raw.recency.map(|section| {
            let section = section.into_inner();
            let half_life_days = match &section.half_life_days {
                None => 90.0,
                Some(spanned) => {
                    let value = *spanned.get_ref();
                    if value.is_finite() && value > 0.0 {
                        value
                    } else {
                        self.at(
                            spanned,
                            format!(
                                "recency.half_life_days is {value}; it is a number of days above 0"
                            ),
                        );
                        90.0
                    }
                }
            };
            let as_of = section.as_of.and_then(|spanned| {
                if day_number(spanned.get_ref()).is_some() {
                    Some(spanned.into_inner())
                } else {
                    let found = spanned.get_ref().clone();
                    self.at(
                        &spanned,
                        format!("recency.as_of is {found:?}; write a date as YYYY-MM-DD"),
                    );
                    None
                }
            });
            Recency {
                half_life_days,
                as_of,
                weight: self.weight("recency", section.weight.as_ref()),
            }
        });
        let genre = raw.genre.map(|section| {
            let section = section.into_inner();
            Genre {
                mix: section.mix.unwrap_or(Mix::Vary),
                weight: self.weight("genre", section.weight.as_ref()),
            }
        });
        // A shape with mistakes has its problems reported; no Recipe then.
        let intensity = match intensity {
            None => None,
            Some(checked) => Some(checked?),
        };
        Some(Recipe {
            name: raw.name,
            tracks: tracks?,
            intensity,
            tempo,
            key,
            situations,
            recency,
            genre,
        })
    }

    fn intensity(&mut self, section: &Spanned<RawIntensity>) -> Option<Intensity> {
        let raw = section.get_ref();
        let mut shape = Vec::new();
        match &raw.shape {
            None => self.at(
                section,
                "[intensity] has no shape; give at least one point { at, intensity }".to_owned(),
            ),
            Some(points) if points.get_ref().is_empty() => self.at(
                points,
                "intensity.shape is empty; give at least one point { at, intensity }".to_owned(),
            ),
            Some(points) => {
                let mut last: Option<f64> = None;
                for spanned in points.get_ref() {
                    let point = spanned.get_ref();
                    let at_fine = self.fraction(spanned, "intensity.shape at", point.at);
                    let value_fine =
                        self.fraction(spanned, "intensity.shape intensity", point.intensity);
                    if at_fine {
                        if let Some(before) = last {
                            if point.at < before {
                                self.at(
                                    spanned,
                                    format!(
                                        "intensity.shape points go in order of `at`; {} comes after {before}",
                                        point.at
                                    ),
                                );
                            }
                        }
                        last = Some(point.at);
                    }
                    if at_fine && value_fine {
                        shape.push(Point {
                            at: point.at,
                            intensity: point.intensity,
                        });
                    }
                }
            }
        }
        let max_step = raw.max_step.as_ref().and_then(|limit| {
            let value = *limit.get_ref();
            if value.is_finite() && value > 0.0 && value <= 1.0 {
                Some(value)
            } else {
                self.at(
                    limit,
                    format!("intensity.max_step is {value}; it is above 0 and at most 1"),
                );
                None
            }
        });
        if raw.missing == Missing::Allow && raw.max_step.is_some() {
            self.at(
                section,
                "intensity.missing = \"allow\" cannot be used with intensity.max_step: a step to or from a track with no Intensity cannot be checked".to_owned(),
            );
        }
        let step_weight = match &raw.step_weight {
            None => 0.5,
            Some(spanned) => {
                let value = *spanned.get_ref();
                if value.is_finite() && value >= 0.0 {
                    value
                } else {
                    self.at(
                        spanned,
                        format!("intensity.step_weight is {value}; it is 0 or more"),
                    );
                    0.5
                }
            }
        };
        let weight = self.weight("intensity", raw.weight.as_ref());
        (!shape.is_empty()).then_some(Intensity {
            shape,
            max_step,
            missing: raw.missing,
            step_weight,
            weight,
        })
    }
}

/// The line (from 1) of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset).map_or(1, |before| {
        before.bytes().filter(|&b| b == b'\n').count() + 1
    })
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date (also the first ten
/// characters of a longer timestamp), or `None` when it is not a date.
#[must_use]
pub fn day_number(text: &str) -> Option<i64> {
    let date = text.get(..10)?;
    let bytes = date.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i64 = date[..4].parse().ok()?;
    let month: i64 = date[5..7].parse().ok()?;
    let day: i64 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days-from-civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}
