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

/// The share of a criterion's weight a transition gets when the data it
/// needs is missing: between a clash (0) and a perfect match (1), so a
/// missing key or tempo is scored apart from the worst case, and named.
pub const UNKNOWN: f64 = 0.5;

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
    /// The DJ's Energy rating, 1 to 5 stars; `None` unrated. Stands in for
    /// a missing Intensity under `intensity.missing = "energy"`.
    #[serde(default)]
    pub energy: Option<u8>,
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
    /// Not a lack of data: the track's tempo is outside the Recipe's
    /// `tempo.min_bpm` to `tempo.max_bpm`.
    TempoRange,
}

impl Lack {
    fn words(self) -> &'static str {
        match self {
            Self::Intensity => "no Intensity",
            Self::Tempo => "no tempo",
            Self::Key => "no key",
            Self::TempoRange => "a tempo outside the Recipe's range",
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
    /// `key.max_detune`, without key lock.
    KeyDetune,
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
    /// The complete search reached its bound before finding a plan or
    /// proving there is none: no plan was found, which is not proof.
    #[serde(default)]
    pub search_exhausted: bool,
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
    plan_with(recipe, pool, BEAM)
}

/// [`plan`] with a beam of `width` partial orders (tests force a beam miss).
fn plan_with(
    recipe: &Recipe,
    pool: &[Candidate],
    width: usize,
) -> Result<Plan, Box<Unsatisfiable>> {
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
            let energy = recipe
                .intensity
                .as_ref()
                .is_some_and(|i| i.missing == Missing::Energy);
            let words: Vec<&str> = lacks
                .iter()
                .filter(|l| **l != Lack::TempoRange)
                .map(|l| match l {
                    Lack::Intensity if energy => "no Intensity and no Energy",
                    other => other.words(),
                })
                .collect();
            let mut message = format!("track {} {:?}", candidate.id, candidate.title);
            if !words.is_empty() {
                message.push_str(&format!(" has {}, which this Recipe needs;", words.join(" and ")));
            }
            if lacks.contains(&Lack::TempoRange) {
                let t = recipe.tempo.as_ref();
                let bound = |b: Option<f64>| b.map_or_else(|| "any".to_owned(), |v| format!("{v}"));
                message.push_str(&format!(
                    " is at {:.2} BPM, outside the Recipe's tempo range ({} to {} BPM{});",
                    tempo(candidate).unwrap_or_default(),
                    bound(t.and_then(|t| t.min_bpm)),
                    bound(t.and_then(|t| t.max_bpm)),
                    if t.is_some_and(|t| t.fold) { ", half or double time counted" } else { "" }
                ));
            }
            message.push_str(" it is left out");
            left_out.push(LeftOut {
                id: candidate.id.clone(),
                title: candidate.title.clone(),
                message,
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
            search_exhausted: false,
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
    match search(&scorer, &usable, limits, width) {
        Ok(order) => Ok(scorer.write(&order, &usable, pool_size, left_out, as_of)),
        Err(stopped) => {
            let in_the_way: Vec<Blocker> = limits
                .present()
                .into_iter()
                .filter(|&limit| search(&scorer, &usable, limits.without(limit), width).is_ok())
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
            let message = if stopped.exhausted {
                format!(
                    "No plan was found within the search's bound of {COMPLETE_SEARCH_BOUND} checks; the limits may still allow one. {message}"
                )
            } else {
                message
            };
            let mut refused = unsatisfiable(
                criterion,
                in_the_way,
                message,
                Some(stopped.position),
                stopped.rejected,
            );
            refused.search_exhausted = stopped.exhausted;
            Err(refused)
        }
    }
}

fn lacks(recipe: &Recipe, c: &Candidate) -> Vec<Lack> {
    let mut lacks = Vec::new();
    if recipe
        .intensity
        .as_ref()
        .is_some_and(|i| i.missing != Missing::Allow)
        && level(recipe, c).is_none()
    {
        lacks.push(Lack::Intensity);
    }
    let tempo_needed = recipe.tempo.as_ref().is_some_and(|t| {
        t.max_change.is_some() || t.min_bpm.is_some() || t.max_bpm.is_some()
    }) || recipe.key.as_ref().is_some_and(|k| k.max_detune.is_some());
    if tempo_needed && tempo(c).is_none() {
        lacks.push(Lack::Tempo);
    }
    if recipe.key.as_ref().is_some_and(|k| k.max_moves.is_some()) && c.key.is_none() {
        lacks.push(Lack::Key);
    }
    if let (Some(t), Some(bpm)) = (&recipe.tempo, tempo(c)) {
        let low = t.min_bpm.unwrap_or(0.0);
        let high = t.max_bpm.unwrap_or(f64::INFINITY);
        let factors: &[f64] = if t.fold { &[1.0, 0.5, 2.0] } else { &[1.0] };
        let inside = factors
            .iter()
            .any(|f| bpm * f >= low - 1e-9 && bpm * f <= high + 1e-9);
        if !inside {
            lacks.push(Lack::TempoRange);
        }
    }
    lacks
}

fn tempo(c: &Candidate) -> Option<f64> {
    c.bpm.filter(|b| b.is_finite() && *b > 0.0)
}

/// The track's Intensity, or under `missing = "energy"` its Energy as
/// (Energy - 1) / 4 (and the Energy, to say so).
fn level(recipe: &Recipe, c: &Candidate) -> Option<(f64, Option<u8>)> {
    match (c.intensity, recipe.intensity.as_ref().map(|i| i.missing)) {
        (Some(value), _) => Some((value, None)),
        (None, Some(Missing::Energy)) => c
            .energy
            .filter(|e| (1..=5).contains(e))
            .map(|e| ((f64::from(e) - 1.0) / 4.0, Some(e))),
        _ => None,
    }
}

/// The stretch between two tempos in percent, the same both ways: the
/// larger over the smaller. With `fold`, half and double time are tried
/// too. Returns the stretch, its sign (+ when `y` as played is faster) and
/// the factor `y` is played at (1, 2 or 0.5).
fn tempo_step(x: f64, y: f64, fold: bool) -> (f64, f64, f64) {
    let factors: &[f64] = if fold { &[1.0, 2.0, 0.5] } else { &[1.0] };
    let mut best = (f64::INFINITY, 1.0, 1.0);
    for &f in factors {
        let played = y * f;
        let stretch = (x.max(played) / x.min(played) - 1.0) * 100.0;
        if stretch < best.0 - 1e-12 {
            best = (stretch, if played >= x { 1.0 } else { -1.0 }, f);
        }
    }
    best
}

/// The incoming key as heard without key lock: shifted by the whole
/// semitones of the tempo change, and the cents left over.
#[derive(Debug, Clone, Copy)]
struct Heard {
    key: Camelot,
    shift: i32,
    cents: f64,
    percent: f64,
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
    detune: Option<f64>,
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
            detune: recipe.key.as_ref().and_then(|k| k.max_detune),
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
        if self.detune.is_some() {
            present.push(Blocker::KeyDetune);
        }
        present
    }

    fn without(mut self, limit: Blocker) -> Self {
        match limit {
            Blocker::IntensityStep => self.step = None,
            Blocker::KeyMoves => self.moves = None,
            Blocker::TempoChange => self.change = None,
            Blocker::Situations => self.situations = false,
            Blocker::KeyDetune => self.detune = None,
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
    /// The complete search reached its bound: not proven impossible.
    exhausted: bool,
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

/// How many candidate checks the complete search may make after the beam
/// has missed; past it, "no plan found" is said as that, never as proof.
pub const COMPLETE_SEARCH_BOUND: usize = 4_000_000;

fn is_used(used: &[u64], index: usize) -> bool {
    used[index / 64] & (1 << (index % 64)) != 0
}

fn mark(used: &mut [u64], index: usize) {
    used[index / 64] |= 1 << (index % 64);
}

/// The positions a required Situation covers, and which tracks may take
/// each: what every partial order must still be able to fill.
struct Required {
    /// (position, the indexes of the tracks that may take it), in position order.
    positions: Vec<(usize, Vec<usize>)>,
}

impl Required {
    fn of(scorer: &Scorer<'_>, pool: &[&Candidate], limits: Limits) -> Self {
        let mut positions = Vec::new();
        if limits.situations {
            for position in 0..scorer.recipe.tracks {
                let anyone = pool.iter().all(|c| scorer.required_here(c, position));
                if anyone {
                    continue; // no place requires anything here
                }
                let eligible = (0..pool.len())
                    .filter(|&i| scorer.required_here(pool[i], position))
                    .collect();
                positions.push((position, eligible));
            }
        }
        Self { positions }
    }

    /// Whether the required positions from `from` on can each still get a
    /// different unused track (a bipartite matching, Kuhn's, in index order).
    fn fillable(&self, used: &[u64], from: usize) -> bool {
        let wanted: Vec<&Vec<usize>> = self
            .positions
            .iter()
            .filter(|(position, _)| *position >= from)
            .map(|(_, eligible)| eligible)
            .collect();
        if wanted.is_empty() {
            return true;
        }
        let mut owner: BTreeMap<usize, usize> = BTreeMap::new();
        for slot in 0..wanted.len() {
            let mut seen = std::collections::BTreeSet::new();
            if !augment(slot, &wanted, used, &mut owner, &mut seen) {
                return false;
            }
        }
        true
    }
}

fn augment(
    slot: usize,
    wanted: &[&Vec<usize>],
    used: &[u64],
    owner: &mut BTreeMap<usize, usize>,
    seen: &mut std::collections::BTreeSet<usize>,
) -> bool {
    for &track in wanted[slot] {
        if is_used(used, track) || !seen.insert(track) {
            continue;
        }
        let free = match owner.get(&track) {
            None => true,
            Some(&other) => augment(other, wanted, used, owner, seen),
        };
        if free {
            owner.insert(track, slot);
            return true;
        }
    }
    false
}

/// The best order of `recipe.tracks` of `pool` (indexes), or where it stopped.
///
/// The beam first; a partial order that could no longer fill a later
/// required position is dropped before the beam is cut. If the beam still
/// empties, a depth-first search that is complete for the hard limits
/// (bounded by [`COMPLETE_SEARCH_BOUND`], deterministic) looks for any order
/// that keeps them, so a beam miss alone is never reported as unsatisfiable.
fn search(
    scorer: &Scorer<'_>,
    pool: &[&Candidate],
    limits: Limits,
    width: usize,
) -> Result<Vec<usize>, Stopped> {
    let required = Required::of(scorer, pool, limits);
    match beam_search(scorer, pool, limits, &required, width) {
        Ok(order) => Ok(order),
        Err(mut stopped) => match complete_search(scorer, pool, limits, &required) {
            Complete::Found(order) => Ok(order),
            Complete::None => Err(stopped),
            Complete::Bounded => {
                stopped.exhausted = true;
                Err(stopped)
            }
        },
    }
}

fn beam_search(
    scorer: &Scorer<'_>,
    pool: &[&Candidate],
    limits: Limits,
    required: &Required,
    width: usize,
) -> Result<Vec<usize>, Stopped> {
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
        mark(&mut used, index);
        if !required.fillable(&used, 1) {
            *refused.entry(Blocker::Situations).or_default() += 1;
            continue;
        }
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
    beam.truncate(width);
    for position in 1..n {
        if beam.is_empty() {
            // Nothing could take the position before this one.
            return Err(stopped(position - 1, &refused));
        }
        refused.clear();
        let mut next: BTreeMap<(usize, Vec<u64>), State> = BTreeMap::new();
        for state in &beam {
            let last = pool[state.order[state.order.len() - 1]];
            for (index, candidate) in pool.iter().enumerate() {
                if is_used(&state.used, index) {
                    continue;
                }
                let broken = scorer.broken(last, candidate, position, limits);
                if !broken.is_empty() {
                    for limit in broken {
                        *refused.entry(limit).or_default() += 1;
                    }
                    continue;
                }
                let mut used = state.used.clone();
                mark(&mut used, index);
                // Taking this track here would leave a later required
                // position with none to fill it.
                if !required.fillable(&used, position + 1) {
                    *refused.entry(Blocker::Situations).or_default() += 1;
                    continue;
                }
                let gained: f64 = scorer
                    .transition(last, candidate, position)
                    .iter()
                    .map(|c| c.score)
                    .sum();
                let mut order = state.order.clone();
                order.push(index);
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
        beam.truncate(width);
    }
    beam.into_iter()
        .next()
        .map(|state| state.order)
        .ok_or_else(|| stopped(n - 1, &refused))
}

enum Complete {
    Found(Vec<usize>),
    /// Proven: no order keeps the hard limits.
    None,
    /// The bound was reached first.
    Bounded,
}

/// Depth-first over every order that keeps the hard limits, the most
/// promising extension first (by its score, then by index), remembering
/// partial orders proven dead (same last track, same tracks used). Complete
/// within its bound; deterministic.
fn complete_search(
    scorer: &Scorer<'_>,
    pool: &[&Candidate],
    limits: Limits,
    required: &Required,
) -> Complete {
    struct Walk<'s, 'a> {
        scorer: &'s Scorer<'a>,
        pool: &'s [&'s Candidate],
        limits: Limits,
        required: &'s Required,
        dead: std::collections::BTreeSet<(usize, Vec<u64>)>,
        checks: usize,
    }
    impl Walk<'_, '_> {
        /// `Some(true)` found (in `order`), `Some(false)` dead, `None` bounded.
        fn extend(&mut self, order: &mut Vec<usize>, used: &mut Vec<u64>) -> Option<bool> {
            let position = order.len();
            if position == self.scorer.recipe.tracks {
                return Some(true);
            }
            let last = order.last().copied();
            if let Some(last) = last {
                if self.dead.contains(&(last, used.clone())) {
                    return Some(false);
                }
            }
            let mut next: Vec<(f64, usize)> = Vec::new();
            for (index, candidate) in self.pool.iter().enumerate() {
                if is_used(used, index) {
                    continue;
                }
                self.checks += 1;
                if self.checks > COMPLETE_SEARCH_BOUND {
                    return None;
                }
                let gained: f64 = match last {
                    None => {
                        if self.limits.situations && !self.scorer.required_here(candidate, 0) {
                            continue;
                        }
                        self.scorer
                            .at_position(candidate, 0, None)
                            .iter()
                            .map(|c| c.score)
                            .sum()
                    }
                    Some(last) => {
                        let from = self.pool[last];
                        if !self
                            .scorer
                            .broken(from, candidate, position, self.limits)
                            .is_empty()
                        {
                            continue;
                        }
                        self.scorer
                            .transition(from, candidate, position)
                            .iter()
                            .map(|c| c.score)
                            .sum()
                    }
                };
                mark(used, index);
                let fillable = self.required.fillable(used, position + 1);
                used[index / 64] &= !(1 << (index % 64));
                if fillable {
                    next.push((gained, index));
                }
            }
            next.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            for (_, index) in next {
                order.push(index);
                mark(used, index);
                match self.extend(order, used) {
                    Some(true) => return Some(true),
                    None => return None,
                    Some(false) => {}
                }
                order.pop();
                used[index / 64] &= !(1 << (index % 64));
            }
            if let Some(last) = last {
                self.dead.insert((last, used.clone()));
            }
            Some(false)
        }
    }
    let mut walk = Walk {
        scorer,
        pool,
        limits,
        required,
        dead: std::collections::BTreeSet::new(),
        checks: 0,
    };
    let mut order = Vec::new();
    let mut used = vec![0u64; pool.len().div_ceil(64)];
    match walk.extend(&mut order, &mut used) {
        Some(true) => Complete::Found(order),
        Some(false) => Complete::None,
        None => Complete::Bounded,
    }
}

fn stopped(position: usize, refused: &BTreeMap<Blocker, usize>) -> Stopped {
    Stopped {
        // From 1.
        position: position + 1,
        rejected: refused
            .iter()
            .map(|(&limit, &refused)| Rejection { limit, refused })
            .collect(),
        exhausted: false,
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
            Blocker::KeyDetune => format!(
                "the detuning limit without key lock (key.max_detune {} cents)",
                r.key.as_ref().and_then(|k| k.max_detune).unwrap_or_default()
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

    fn fold(&self) -> bool {
        self.recipe.tempo.as_ref().is_some_and(|t| t.fold)
    }

    /// `b`'s key as heard after `a` without key lock: `None` with key lock,
    /// or when either tempo or `b`'s key is unknown.
    fn heard(&self, a: &Candidate, b: &Candidate) -> Option<Heard> {
        let key = self.recipe.key.as_ref()?;
        if key.key_lock {
            return None;
        }
        let (x, y, to) = (tempo(a)?, tempo(b)?, b.key?);
        let (_, _, factor) = tempo_step(x, y, self.fold());
        // `b` is played at `x`: faster by x / (y * factor).
        let semitones = 12.0 * (x / (y * factor)).log2();
        let whole = semitones.round();
        #[allow(clippy::cast_possible_truncation, reason = "a few semitones")]
        let shift = whole as i32;
        Some(Heard {
            key: to.shifted(shift),
            shift,
            cents: (semitones - whole) * 100.0,
            percent: (x / (y * factor) - 1.0) * 100.0,
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
        let r = self.recipe;
        if let (Some(step), Some((x, _)), Some((y, _))) =
            (limits.step, level(r, a), level(r, b))
        {
            if (y - x).abs() > step + 1e-9 {
                broken.push(Blocker::IntensityStep);
            }
        }
        let heard = self.heard(a, b);
        if let (Some(moves), Some(x), Some(y)) = (limits.moves, a.key, b.key) {
            let y = heard.map_or(y, |h| h.key);
            if x.step_to(y).cost() > f64::from(moves) + 1e-9 {
                broken.push(Blocker::KeyMoves);
            }
        }
        if let (Some(detune), Some(h)) = (limits.detune, heard) {
            if h.cents.abs() > detune + 1e-9 {
                broken.push(Blocker::KeyDetune);
            }
        }
        if let (Some(change), Some(x), Some(y)) = (limits.change, tempo(a), tempo(b)) {
            if tempo_step(x, y, self.fold()).0 > change + 1e-9 {
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
            out.push(match level(r, c) {
                Some((value, energy)) => {
                    let mut score = shape.weight * (1.0 - (value - target).abs());
                    let step = match from.and_then(|f| level(r, f)) {
                        Some((before, _)) if position > 0 => {
                            // The step taken against the step the shape takes.
                            let wanted = target - shape.target(r.at(position - 1));
                            let off = (value - before) - wanted;
                            score -= shape.weight * shape.step_weight * off.abs();
                            format!(
                                ", a step of {:+.2} from {before:.2} where the shape steps {wanted:+.2}",
                                value - before
                            )
                        }
                        _ => String::new(),
                    };
                    let what = match energy {
                        Some(e) => format!("Energy {e} standing in for Intensity: {value:.2}"),
                        None => format!("Intensity {value:.2}"),
                    };
                    Contribution {
                        criterion: Criterion::Intensity,
                        score: round4(score),
                        reason: format!("{what} where the shape wants {target:.2}{step}"),
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
                (Some(x), Some(y)) => self.key_move(key, a, b, x, y, position),
                _ => {
                    let without = if a.key.is_none() { &a.id } else { &b.id };
                    Contribution {
                        criterion: Criterion::Key,
                        score: round4(key.weight * UNKNOWN),
                        reason: format!(
                            "track {without} has no key, so the move is unknown: scored at {UNKNOWN} of the weight, not as a clash"
                        ),
                    }
                }
            });
        }
        if let Some(t) = &r.tempo {
            out.push(match (tempo(a), tempo(b)) {
                (Some(x), Some(y)) => {
                    let (stretch, sign, factor) = tempo_step(x, y, t.fold);
                    let scale = t.max_change.unwrap_or(8.0);
                    let folded = if factor > 1.0 {
                        " played at double time,"
                    } else if factor < 1.0 {
                        " played at half time,"
                    } else {
                        ""
                    };
                    Contribution {
                        criterion: Criterion::Tempo,
                        score: round4(t.weight * (1.0 - stretch / scale).max(0.0)),
                        reason: format!("{x:.2} to {y:.2} BPM,{folded} {:+.1}%", sign * stretch),
                    }
                }
                _ => {
                    let without = if tempo(a).is_none() { &a.id } else { &b.id };
                    Contribution {
                        criterion: Criterion::Tempo,
                        score: round4(t.weight * UNKNOWN),
                        reason: format!(
                            "track {without} has no tempo, so the change is unknown: scored at {UNKNOWN} of the weight, not as a jump"
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

    /// The key contribution of a move between two known keys.
    fn key_move(
        &self,
        key: &crate::recipe::Key,
        a: &Candidate,
        b: &Candidate,
        x: Camelot,
        y: Camelot,
        position: usize,
    ) -> Contribution {
        let r = self.recipe;
        let heard = self.heard(a, b);
        let step = x.step_to(heard.map_or(y, |h| h.key));
        let mut cost = step.cost();
        let mut notes = String::new();
        if let Some(h) = heard {
            notes.push_str(&format!(
                ", heard as {} without key lock ({:+} semitone{}, {:+.0} cents, at {:+.1}%)",
                h.key,
                h.shift,
                if h.shift.abs() == 1 { "" } else { "s" },
                h.cents,
                h.percent
            ));
        }
        // An energy boost goes with a rising shape, a wind-down with a
        // falling one: half a move cheaper with it, half dearer against it.
        if let Some(shape) = &r.intensity {
            if step.is_boost() || step.is_wind_down() {
                let rise = shape.target(r.at(position)) - shape.target(r.at(position - 1));
                if rise.abs() > 1e-9 {
                    let with = (rise > 0.0) == step.is_boost();
                    cost += if with { -0.5 } else { 0.5 };
                    notes.push_str(if with {
                        ", with the shape"
                    } else {
                        ", against the shape"
                    });
                }
            }
        }
        let detune = heard.map_or(0.0, |h| h.cents.abs());
        let mut score = key.weight * (1.0 - cost / 6.0).max(0.0) * (1.0 - detune / 50.0);
        if step.cost() == 0.0 && key.variety > 0.0 {
            score -= key.weight * key.variety;
            notes.push_str(&format!("; the Recipe wants variety (-{})", key.variety));
        }
        let to = if heard.is_some() {
            format!("{y}")
        } else {
            format!("{}", step.to)
        };
        Contribution {
            criterion: Criterion::Key,
            score: round4(score),
            reason: format!("{x} to {to}, {}{notes}", step.name()),
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    fn candidate(id: &str, key: &str, intensity: f64) -> Candidate {
        Candidate {
            id: id.to_owned(),
            title: id.to_owned(),
            bpm: Some(120.0),
            key: Camelot::parse(key),
            intensity: Some(intensity),
            energy: None,
            situations: None,
            genres: None,
            added: None,
        }
    }

    /// Two tracks each with one neighbour on the wheel must take both ends;
    /// a one-wide beam starts on a loud middle track and dies, and the
    /// complete search finds the one shape of plan there is.
    #[test]
    fn a_beam_miss_falls_back_to_the_complete_search() {
        let pool = vec![
            candidate("1", "8A", 1.0),
            candidate("2", "8A", 1.0),
            candidate("3", "8A", 1.0),
            candidate("4", "8A", 1.0),
            candidate("5", "9A", 0.0),
            candidate("6", "10A", 0.0),
            candidate("7", "7A", 0.0),
            candidate("8", "6A", 0.0),
        ];
        let recipe = Recipe::parse(
            "version = 1\ntracks = 8\n[intensity]\nshape = [{ at = 0, intensity = 1 }]\nweight = 10\n[key]\nmax_moves = 1\n",
        )
        .unwrap();
        let refs: Vec<&Candidate> = pool.iter().collect();
        let scorer = Scorer {
            recipe: &recipe,
            as_of: None,
            as_of_text: String::new(),
        };
        let limits = Limits::of(&recipe);
        let required = Required::of(&scorer, &refs, limits);
        assert!(
            beam_search(&scorer, &refs, limits, &required, 1).is_err(),
            "the one-wide beam misses"
        );
        let built = plan_with(&recipe, &pool, 1).unwrap_or_else(|u| panic!("{}", u.message));
        let ends = (built.tracks[0].id.as_str(), built.tracks[7].id.as_str());
        assert!(ends == ("6", "8") || ends == ("8", "6"), "{ends:?}");
        let again = plan_with(&recipe, &pool, 1).unwrap();
        assert_eq!(
            serde_json::to_string(&built).unwrap(),
            serde_json::to_string(&again).unwrap()
        );
    }

    /// Proven impossible: the complete search says so, not "exhausted".
    #[test]
    fn a_proven_impossible_recipe_is_not_called_exhausted() {
        let pool = vec![
            candidate("1", "8A", 0.5),
            candidate("2", "10A", 0.5),
            candidate("3", "12A", 0.5),
        ];
        let recipe = Recipe::parse("version = 1\ntracks = 3\n[key]\nmax_moves = 1\n").unwrap();
        let refused = plan_with(&recipe, &pool, 1).unwrap_err();
        assert!(!refused.search_exhausted);
        assert_eq!(refused.criterion, Blocker::KeyMoves);
        assert_eq!(refused.position, Some(2), "{}", refused.message);
    }
}
