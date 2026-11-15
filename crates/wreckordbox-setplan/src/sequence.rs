//! The sequencer: a [`Recipe`] and a pool of [`Candidate`]s in, a [`Plan`]
//! out, or the [`Unsatisfiable`] criterion in the way.
//!
//! **Deterministic.** The pool is sorted by track identifier first, so its
//! order is not an input; ties are broken by that order; there is no clock,
//! no randomness and no hash-map iteration. The same Recipe and the same
//! Facts give the same plan, byte for byte once serialised.
//!
//! **How.** A beam search over the positions of the set: every partial
//! order that keeps the Recipe's hard limits is extended by each unused
//! track that keeps them too, scored, and the best [`BEAM`] are kept (a
//! partial order ending on the same track with the same tracks used is kept
//! once, the better one). The best whole order wins. A beam can miss a plan
//! that exists when the limits are very tight; it never returns one that
//! breaks a limit.
//!
//! **Scores.** Each criterion the Recipe uses contributes to each
//! transition, with a reason in words: `intensity` (the incoming track
//! against the shape, with the step), `key` (moves on the wheel), `tempo`
//! (the change in percent), `situations` (does the incoming track's
//! Situation belong here), `recency` (how recently it was added) and
//! `genre` (the mix). The first track has the position criteria only, in
//! [`Plan::opening`]. A weight scales a criterion; most contributions run
//! from 0 to the weight, `situations` from minus the weight to the weight.
//!
//! **Missing data is named.** A track the Recipe's hard limits cannot be
//! checked on (no tempo under a tempo limit, no key under a key limit) is
//! left out of the plan and listed in `left_out` with what it lacks, as is
//! a track with no Intensity unless the Recipe says `missing = "allow"`.
//! Where a criterion is scored without data, its reason says so.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::recipe::{day_number, Missing, Mix, Recipe};
use crate::Camelot;

/// How many partial orders are kept at each position.
pub const BEAM: usize = 48;

/// One track of the pool, with what the sequencer needs to know of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    /// `None`, or zero, is no tempo.
    pub bpm: Option<f64>,
    pub key: Option<Camelot>,
    /// The track's Intensity (0..1), or `None` when it has none.
    pub intensity: Option<f64>,
    /// The standing Situation Fact's term identifiers; `None` when there is
    /// no Situation Fact at all (unknown, not "none").
    pub situations: Option<Vec<String>>,
    /// The standing genre Fact's terms (or the Library's genre); `None` unknown.
    pub genres: Option<Vec<String>>,
    /// When the track was added to the Library, `YYYY-MM-DD`.
    pub added: Option<String>,
}

/// A criterion a transition is scored on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Criterion {
    Intensity,
    Key,
    Tempo,
    Situations,
    Recency,
    Genre,
}

/// One criterion's part of a score, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub criterion: Criterion,
    pub score: f64,
    pub reason: String,
}

/// A track in its place in the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    /// From 1.
    pub position: usize,
    pub id: String,
    pub title: String,
    /// Where in the set, 0 (first) to 1 (last).
    pub at: f64,
    pub bpm: Option<f64>,
    /// Camelot, `""` when the track has no key.
    pub key: String,
    pub intensity: Option<f64>,
    /// What the Recipe's shape wants here; `None` without a shape.
    pub target_intensity: Option<f64>,
    pub situations: Vec<String>,
    pub genres: Vec<String>,
}

/// From one track to the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    /// Positions, from 1.
    pub from: usize,
    pub to: usize,
    pub from_track: String,
    pub to_track: String,
    /// The sum of `criteria`.
    pub score: f64,
    pub criteria: Vec<Contribution>,
}

/// What a track lacks for this Recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lack {
    Intensity,
    Tempo,
    Key,
}

impl Lack {
    fn words(self) -> &'static str {
        match self {
            Self::Intensity => "no Intensity",
            Self::Tempo => "no tempo",
            Self::Key => "no key",
        }
    }
}

/// A track of the pool the plan could not use, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftOut {
    pub id: String,
    pub title: String,
    pub lacks: Vec<Lack>,
    pub message: String,
}

/// A Set plan as the sequencer built it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The Recipe's name.
    pub recipe: String,
    pub tracks: Vec<Placed>,
    /// The first track's position criteria.
    pub opening: Vec<Contribution>,
    pub transitions: Vec<Transition>,
    /// The whole set's score: the opening and every transition.
    pub score: f64,
    /// How many distinct tracks the pool held, and how many could be used.
    pub pool: usize,
    pub usable: usize,
    pub left_out: Vec<LeftOut>,
    /// The day recency was measured to, when the Recipe weighs recency.
    pub recency_as_of: Option<String>,
}

/// A hard limit (or the pool's size) that can stand in a plan's way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    /// Fewer usable tracks than the Recipe asks for.
    Tracks,
    IntensityStep,
    KeyMoves,
    TempoChange,
    /// A place with `require = true`.
    Situations,
}

/// How often a limit refused a choice at the position where the search
/// stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub limit: Blocker,
    pub refused: usize,
}

/// Why no Set plan can be built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unsatisfiable {
    /// The criterion in the way: the first of `in_the_way`, else the limit
    /// that refused most at the stopping position.
    pub criterion: Blocker,
    /// Each limit that alone, removed, lets a plan be built.
    pub in_the_way: Vec<Blocker>,
    pub message: String,
    /// The position (from 1) no track could take, when the search ran.
    pub position: Option<usize>,
    pub rejected: Vec<Rejection>,
    pub pool: usize,
    pub usable: usize,
    pub left_out: Vec<LeftOut>,
}

/// Builds the Set plan.
///
/// # Errors
/// The Recipe cannot be satisfied from this pool; [`Unsatisfiable`] names
/// the criterion in the way.
pub fn plan(recipe: &Recipe, pool: &[Candidate]) -> Result<Plan, Box<Unsatisfiable>> {
    let mut sorted: Vec<&Candidate> = pool.iter().collect();
    sorted.sort_by(|a, b| (a.id.len(), &a.id).cmp(&(b.id.len(), &b.id)));
    sorted.dedup_by(|a, b| a.id == b.id);
    let pool_size = sorted.len();
    let mut usable = Vec::new();
    let mut left_out = Vec::new();
    for candidate in sorted {
        let lacks = lacks(recipe, candidate);
        if lacks.is_empty() {
            usable.push(candidate);
        } else {
            let words: Vec<&str> = lacks.iter().map(|l| l.words()).collect();
            left_out.push(LeftOut {
                id: candidate.id.clone(),
                title: candidate.title.clone(),
                message: format!(
                    "track {} {:?} has {}, which this Recipe needs; it is left out",
                    candidate.id,
                    candidate.title,
                    words.join(" and ")
                ),
                lacks,
            });
        }
    }
    let as_of = recipe.recency.as_ref().and_then(|recency| {
        recency.as_of.clone().or_else(|| {
            usable
                .iter()
                .filter_map(|c| c.added.as_deref())
                .filter_map(|d| day_number(d).map(|n| (n, d.get(..10).unwrap_or(d))))
                .max()
                .map(|(_, d)| d.to_owned())
        })
    });
    let scorer = Scorer {
        recipe,
        as_of: as_of.as_deref().and_then(day_number),
        as_of_text: as_of.clone().unwrap_or_default(),
    };
    let unsatisfiable = |criterion, in_the_way, message, position, rejected| {
        Box::new(Unsatisfiable {
            criterion,
            in_the_way,
            message,
            position,
            rejected,
            pool: pool_size,
            usable: usable.len(),
            left_out: left_out.clone(),
        })
    };
    if usable.len() < recipe.tracks {
        let mut counts: BTreeMap<Lack, usize> = BTreeMap::new();
        for out in &left_out {
            for lack in &out.lacks {
                *counts.entry(*lack).or_default() += 1;
            }
        }
        let why: Vec<String> = counts
            .iter()
            .map(|(lack, n)| {
                format!(
                    "{n} {} {}",
                    if *n == 1 { "has" } else { "have" },
                    lack.words()
                )
            })
            .collect();
        let why = if why.is_empty() {
            String::new()
        } else {
            format!(" ({})", why.join(", "))
        };
        return Err(unsatisfiable(
            Blocker::Tracks,
            vec![Blocker::Tracks],
            format!(
                "The Recipe asks for {} tracks; the pool has {pool_size}, {} usable{why}. Add tracks to the pool or ask for fewer.",
                recipe.tracks,
                usable.len()
            ),
            None,
            Vec::new(),
        ));
    }
    let limits = Limits::of(recipe);
    match search(&scorer, &usable, limits) {
        Ok(order) => Ok(scorer.write(&order, &usable, pool_size, left_out, as_of)),
        Err(stopped) => {
            let in_the_way: Vec<Blocker> = limits
                .present()
                .into_iter()
                .filter(|&limit| search(&scorer, &usable, limits.without(limit)).is_ok())
                .collect();
            let criterion = in_the_way.first().copied().unwrap_or_else(|| {
                stopped
                    .rejected
                    .iter()
                    .max_by(|a, b| a.refused.cmp(&b.refused).then(b.limit.cmp(&a.limit)))
                    .map_or(Blocker::Tracks, |r| r.limit)
            });
            let refusals: Vec<String> = stopped
                .rejected
                .iter()
                .map(|r| format!("{} refused {}", scorer.name(r.limit), r.refused))
                .collect();
            let message = if in_the_way.is_empty() {
                format!(
                    "The Recipe cannot be satisfied from this pool of {} usable tracks: no single limit is in the way. At position {} no track could follow ({}). Relax several limits, or add tracks to the pool.",
                    usable.len(),
                    stopped.position,
                    refusals.join(", ")
                )
            } else {
                let names: Vec<String> = in_the_way.iter().map(|&b| scorer.name(b)).collect();
                format!(
                    "The Recipe cannot be satisfied from this pool of {} usable tracks: {} {} in the way. Without {}, a plan of {} tracks can be made; with it, no track could take position {} ({}).",
                    usable.len(),
                    names.join(" or "),
                    if names.len() == 1 { "is" } else { "are each" },
                    if names.len() == 1 { "it" } else { "any one of them" },
                    recipe.tracks,
                    stopped.position,
                    refusals.join(", ")
                )
            };
            Err(unsatisfiable(
                criterion,
                in_the_way,
                message,
                Some(stopped.position),
                stopped.rejected,
            ))
        }
    }
}

fn lacks(recipe: &Recipe, c: &Candidate) -> Vec<Lack> {
    let mut lacks = Vec::new();
    if recipe
        .intensity
        .as_ref()
        .is_some_and(|i| i.missing == Missing::LeaveOut)
        && c.intensity.is_none()
    {
        lacks.push(Lack::Intensity);
    }
    if recipe
        .tempo
        .as_ref()
        .is_some_and(|t| t.max_change.is_some())
        && tempo(c).is_none()
    {
        lacks.push(Lack::Tempo);
    }
    if recipe.key.as_ref().is_some_and(|k| k.max_moves.is_some()) && c.key.is_none() {
        lacks.push(Lack::Key);
    }
    lacks
}

fn tempo(c: &Candidate) -> Option<f64> {
    c.bpm.filter(|b| b.is_finite() && *b > 0.0)
}

fn round4(x: f64) -> f64 {
    let rounded = (x * 10_000.0).round() / 10_000.0;
    // No "-0" in a plan.
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// Which hard limits are switched on for one search.
#[derive(Debug, Clone, Copy)]
struct Limits {
    step: Option<f64>,
    moves: Option<u32>,
    change: Option<f64>,
    situations: bool,
}

impl Limits {
    fn of(recipe: &Recipe) -> Self {
        Self {
            step: recipe.intensity.as_ref().and_then(|i| i.max_step),
            moves: recipe.key.as_ref().and_then(|k| k.max_moves),
            change: recipe.tempo.as_ref().and_then(|t| t.max_change),
            situations: recipe
                .situations
                .as_ref()
                .is_some_and(|s| s.place.iter().any(|p| p.require)),
        }
    }

    fn present(self) -> Vec<Blocker> {
        let mut present = Vec::new();
        if self.step.is_some() {
            present.push(Blocker::IntensityStep);
        }
        if self.moves.is_some() {
            present.push(Blocker::KeyMoves);
        }
        if self.change.is_some() {
            present.push(Blocker::TempoChange);
        }
        if self.situations {
            present.push(Blocker::Situations);
        }
        present
    }

    fn without(mut self, limit: Blocker) -> Self {
        match limit {
            Blocker::IntensityStep => self.step = None,
            Blocker::KeyMoves => self.moves = None,
            Blocker::TempoChange => self.change = None,
            Blocker::Situations => self.situations = false,
            Blocker::Tracks => {}
        }
        self
    }
}

struct Scorer<'a> {
    recipe: &'a Recipe,
    as_of: Option<i64>,
    as_of_text: String,
}

struct Stopped {
    position: usize,
    rejected: Vec<Rejection>,
}

#[derive(Clone)]
struct State {
    order: Vec<usize>,
    used: Vec<u64>,
    score: f64,
}

fn better(a: &State, b: &State) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.order.cmp(&b.order))
}

/// The best order of `recipe.tracks` of `pool` (indexes), or where it stopped.
fn search(scorer: &Scorer<'_>, pool: &[&Candidate], limits: Limits) -> Result<Vec<usize>, Stopped> {
    let n = scorer.recipe.tracks;
    let words = pool.len().div_ceil(64);
    let mut refused: BTreeMap<Blocker, usize> = BTreeMap::new();
    let mut beam: Vec<State> = Vec::new();
    for (index, candidate) in pool.iter().enumerate() {
        if limits.situations && !scorer.required_here(candidate, 0) {
            *refused.entry(Blocker::Situations).or_default() += 1;
            continue;
        }
        let mut used = vec![0u64; words];
        used[index / 64] |= 1 << (index % 64);
        beam.push(State {
            order: vec![index],
            used,
            score: scorer
                .at_position(candidate, 0, None)
                .iter()
                .map(|c| c.score)
                .sum(),
        });
    }
    beam.sort_by(better);
    beam.truncate(BEAM);
    for position in 1..n {
        if beam.is_empty() {
            return Err(stopped(position, &refused));
        }
        refused.clear();
        let mut next: BTreeMap<(usize, Vec<u64>), State> = BTreeMap::new();
        for state in &beam {
            let last = pool[state.order[state.order.len() - 1]];
            for (index, candidate) in pool.iter().enumerate() {
                if state.used[index / 64] & (1 << (index % 64)) != 0 {
                    continue;
                }
                let broken = scorer.broken(last, candidate, position, limits);
                if !broken.is_empty() {
                    for limit in broken {
                        *refused.entry(limit).or_default() += 1;
                    }
                    continue;
                }
                let gained: f64 = scorer
                    .transition(last, candidate, position)
                    .iter()
                    .map(|c| c.score)
                    .sum();
                let mut order = state.order.clone();
                order.push(index);
                let mut used = state.used.clone();
                used[index / 64] |= 1 << (index % 64);
                let grown = State {
                    order,
                    used: used.clone(),
                    score: state.score + gained,
                };
                match next.entry((index, used)) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(grown);
                    }
                    std::collections::btree_map::Entry::Occupied(mut slot) => {
                        if better(&grown, slot.get()) == std::cmp::Ordering::Less {
                            slot.insert(grown);
                        }
                    }
                }
            }
        }
        beam = next.into_values().collect();
        beam.sort_by(better);
        beam.truncate(BEAM);
    }
    beam.into_iter()
        .next()
        .map(|state| state.order)
        .ok_or_else(|| stopped(n, &refused))
}

fn stopped(position: usize, refused: &BTreeMap<Blocker, usize>) -> Stopped {
    Stopped {
        // From 1.
        position: position + 1,
        rejected: refused
            .iter()
            .map(|(&limit, &refused)| Rejection { limit, refused })
            .collect(),
    }
}

impl Scorer<'_> {
    fn name(&self, limit: Blocker) -> String {
        let r = self.recipe;
        match limit {
            Blocker::Tracks => "the number of tracks".to_owned(),
            Blocker::IntensityStep => format!(
                "the Intensity step limit (intensity.max_step {})",
                r.intensity
                    .as_ref()
                    .and_then(|i| i.max_step)
                    .unwrap_or_default()
            ),
            Blocker::KeyMoves => format!(
                "the key limit (key.max_moves {})",
                r.key.as_ref().and_then(|k| k.max_moves).unwrap_or_default()
            ),
            Blocker::TempoChange => format!(
                "the tempo limit (tempo.max_change {}%)",
                r.tempo
                    .as_ref()
                    .and_then(|t| t.max_change)
                    .unwrap_or_default()
            ),
            Blocker::Situations => {
                let required: Vec<String> = r
                    .situations
                    .iter()
                    .flat_map(|s| s.place.iter())
                    .filter(|p| p.require)
                    .map(|p| format!("{} from {} to {}", p.situation, p.from, p.to))
                    .collect();
                format!("the required Situations ({})", required.join("; "))
            }
        }
    }

    /// Whether `c` carries every Situation required at `position`.
    fn required_here(&self, c: &Candidate, position: usize) -> bool {
        let at = self.recipe.at(position);
        self.recipe
            .situations
            .iter()
            .flat_map(|s| s.place.iter())
            .all(|p| {
                !p.require
                    || !p.covers(at)
                    || c.situations
                        .as_ref()
                        .is_some_and(|carried| carried.contains(&p.situation))
            })
    }

    /// The hard limits going from `a` to `b` at `position` would break.
    fn broken(
        &self,
        a: &Candidate,
        b: &Candidate,
        position: usize,
        limits: Limits,
    ) -> Vec<Blocker> {
        let mut broken = Vec::new();
        if let (Some(step), Some(x), Some(y)) = (limits.step, a.intensity, b.intensity) {
            if (y - x).abs() > step + 1e-9 {
                broken.push(Blocker::IntensityStep);
            }
        }
        if let (Some(moves), Some(x), Some(y)) = (limits.moves, a.key, b.key) {
            if x.moves(y) > moves {
                broken.push(Blocker::KeyMoves);
            }
        }
        if let (Some(change), Some(x), Some(y)) = (limits.change, tempo(a), tempo(b)) {
            if ((y - x) / x * 100.0).abs() > change + 1e-9 {
                broken.push(Blocker::TempoChange);
            }
        }
        if limits.situations && !self.required_here(b, position) {
            broken.push(Blocker::Situations);
        }
        broken
    }

    /// The criteria of the track at `position` alone: Intensity against the
    /// shape, Situations, recency. `from` is the track before, for the step.
    fn at_position(
        &self,
        c: &Candidate,
        position: usize,
        from: Option<&Candidate>,
    ) -> Vec<Contribution> {
        let r = self.recipe;
        let at = r.at(position);
        let mut out = Vec::new();
        if let Some(shape) = &r.intensity {
            let target = shape.target(at);
            out.push(match c.intensity {
                Some(value) => {
                    let step = match from.and_then(|f| f.intensity) {
                        Some(before) => format!(", a step of {:+.2} from {before:.2}", value - before),
                        None => String::new(),
                    };
                    Contribution {
                        criterion: Criterion::Intensity,
                        score: round4(shape.weight * (1.0 - (value - target).abs())),
                        reason: format!("Intensity {value:.2} where the shape wants {target:.2}{step}"),
                    }
                }
                None => Contribution {
                    criterion: Criterion::Intensity,
                    score: 0.0,
                    reason: format!(
                        "track {} has no Intensity, so it is not scored against the shape ({target:.2} here)",
                        c.id
                    ),
                },
            });
        }
        if let Some(situations) = &r.situations {
            let w = situations.weight;
            let here: Vec<&str> = situations
                .place
                .iter()
                .filter(|p| p.covers(at))
                .map(|p| p.situation.as_str())
                .collect();
            let (score, reason) = match &c.situations {
                None => (0.0, format!("track {} has no Situation recorded", c.id)),
                Some(carried) => {
                    if let Some(fits) = carried.iter().find(|s| here.contains(&s.as_str())) {
                        (w, format!("{fits} belongs here"))
                    } else if let Some(elsewhere) = situations
                        .place
                        .iter()
                        .find(|p| carried.contains(&p.situation))
                    {
                        (
                            -w,
                            format!(
                                "{} belongs from {} to {}, not here at {at:.2}",
                                elsewhere.situation, elsewhere.from, elsewhere.to
                            ),
                        )
                    } else if here.is_empty() {
                        (0.0, format!("no Situation is placed here at {at:.2}"))
                    } else {
                        (
                            0.0,
                            format!("the track is not {} as wanted here", here.join(" or ")),
                        )
                    }
                }
            };
            out.push(Contribution {
                criterion: Criterion::Situations,
                score: round4(score),
                reason,
            });
        }
        if let Some(recency) = &r.recency {
            let added = c
                .added
                .as_deref()
                .and_then(|d| day_number(d).map(|n| (n, d)));
            out.push(match (added, self.as_of) {
                (Some((day, text)), Some(as_of)) => {
                    let age = (as_of - day).max(0);
                    #[allow(clippy::cast_precision_loss, reason = "days")]
                    let score = recency.weight * 0.5f64.powf(age as f64 / recency.half_life_days);
                    Contribution {
                        criterion: Criterion::Recency,
                        score: round4(score),
                        reason: format!(
                            "added {}, {age} days before {}",
                            text.get(..10).unwrap_or(text),
                            self.as_of_text
                        ),
                    }
                }
                _ => Contribution {
                    criterion: Criterion::Recency,
                    score: 0.0,
                    reason: format!("track {} has no date added", c.id),
                },
            });
        }
        out
    }

    /// Every criterion of the transition from `a` to `b` at `position`, in
    /// the order of [`Criterion`].
    #[allow(
        clippy::many_single_char_names,
        clippy::single_match_else,
        reason = "a and b are the transition's two tracks, x and y their values; each criterion is known or not"
    )]
    fn transition(&self, a: &Candidate, b: &Candidate, position: usize) -> Vec<Contribution> {
        let r = self.recipe;
        let mut out = self.at_position(b, position, Some(a));
        if let Some(key) = &r.key {
            out.push(match (a.key, b.key) {
                (Some(x), Some(y)) => {
                    let moves = x.moves(y);
                    let score = key.weight * (1.0 - f64::from(moves) / 4.0).max(0.0);
                    Contribution {
                        criterion: Criterion::Key,
                        score: round4(score),
                        reason: format!(
                            "{x} to {y}, {moves} move{}",
                            if moves == 1 { "" } else { "s" }
                        ),
                    }
                }
                _ => {
                    let without = if a.key.is_none() { &a.id } else { &b.id };
                    Contribution {
                        criterion: Criterion::Key,
                        score: 0.0,
                        reason: format!(
                            "track {without} has no key, so the key move is not scored"
                        ),
                    }
                }
            });
        }
        if let Some(t) = &r.tempo {
            out.push(match (tempo(a), tempo(b)) {
                (Some(x), Some(y)) => {
                    let change = (y - x) / x * 100.0;
                    let scale = t.max_change.unwrap_or(8.0);
                    Contribution {
                        criterion: Criterion::Tempo,
                        score: round4(t.weight * (1.0 - change.abs() / scale).max(0.0)),
                        reason: format!("{x:.2} to {y:.2} BPM, {change:+.1}%"),
                    }
                }
                _ => {
                    let without = if tempo(a).is_none() { &a.id } else { &b.id };
                    Contribution {
                        criterion: Criterion::Tempo,
                        score: 0.0,
                        reason: format!(
                            "track {without} has no tempo, so the change is not scored"
                        ),
                    }
                }
            });
        }
        if let Some(genre) = &r.genre {
            out.push(match (&a.genres, &b.genres) {
                (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() => {
                    let shared: Vec<&String> = y.iter().filter(|g| x.contains(g)).collect();
                    let same = !shared.is_empty();
                    let scores = match genre.mix {
                        Mix::Vary => !same,
                        Mix::Stay => same,
                    };
                    let reason = if same {
                        format!(
                            "both {}",
                            shared
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    } else {
                        format!("{} to {}", x.join(", "), y.join(", "))
                    };
                    let wanted = match genre.mix {
                        Mix::Vary => "the Recipe wants variety",
                        Mix::Stay => "the Recipe wants the genre kept",
                    };
                    Contribution {
                        criterion: Criterion::Genre,
                        score: if scores { round4(genre.weight) } else { 0.0 },
                        reason: format!("{reason}; {wanted}"),
                    }
                }
                _ => {
                    let without = if a.genres.as_ref().is_none_or(Vec::is_empty) {
                        &a.id
                    } else {
                        &b.id
                    };
                    Contribution {
                        criterion: Criterion::Genre,
                        score: 0.0,
                        reason: format!("track {without} has no genre, so the mix is not scored"),
                    }
                }
            });
        }
        out.sort_by_key(|c| c.criterion);
        out
    }

    fn write(
        &self,
        order: &[usize],
        pool: &[&Candidate],
        pool_size: usize,
        left_out: Vec<LeftOut>,
        recency_as_of: Option<String>,
    ) -> Plan {
        let r = self.recipe;
        let tracks: Vec<Placed> = order
            .iter()
            .enumerate()
            .map(|(position, &index)| {
                let c = pool[index];
                let at = r.at(position);
                Placed {
                    position: position + 1,
                    id: c.id.clone(),
                    title: c.title.clone(),
                    at: round4(at),
                    bpm: tempo(c),
                    key: c.key.map(|k| k.to_string()).unwrap_or_default(),
                    intensity: c.intensity,
                    target_intensity: r.intensity.as_ref().map(|i| round4(i.target(at))),
                    situations: c.situations.clone().unwrap_or_default(),
                    genres: c.genres.clone().unwrap_or_default(),
                }
            })
            .collect();
        let opening = self.at_position(pool[order[0]], 0, None);
        let transitions: Vec<Transition> = order
            .windows(2)
            .enumerate()
            .map(|(i, pair)| {
                let (a, b) = (pool[pair[0]], pool[pair[1]]);
                let criteria = self.transition(a, b, i + 1);
                Transition {
                    from: i + 1,
                    to: i + 2,
                    from_track: a.id.clone(),
                    to_track: b.id.clone(),
                    score: round4(criteria.iter().map(|c| c.score).sum()),
                    criteria,
                }
            })
            .collect();
        let score = round4(
            opening.iter().map(|c| c.score).sum::<f64>()
                + transitions.iter().map(|t| t.score).sum::<f64>(),
        );
        Plan {
            recipe: r.name.clone(),
            tracks,
            opening,
            transitions,
            score,
            pool: pool_size,
            usable: pool.len(),
            left_out,
            recency_as_of,
        }
    }
}
