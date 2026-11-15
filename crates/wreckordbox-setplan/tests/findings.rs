//! The research review's findings on the sequencer (wbx-b59), each as the
//! review's own concrete case: directed key moves, the effective key under
//! a tempo change without key lock, a symmetric and foldable tempo measure
//! with set-wide bounds, the Intensity step between neighbours, an Energy
//! fallback, key variety and missing data scored apart from a clash.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use wreckordbox_setplan::sequence::{plan, Blocker, Candidate, Criterion, Lack, Plan};
use wreckordbox_setplan::{Camelot, Recipe};

fn recipe(text: &str) -> Recipe {
    Recipe::parse(text).unwrap_or_else(|p| panic!("{p:?}"))
}

fn k(text: &str) -> Camelot {
    Camelot::parse(text).unwrap()
}

fn track(id: &str, bpm: Option<f64>, key: Option<&str>, intensity: Option<f64>) -> Candidate {
    Candidate {
        id: id.into(),
        title: format!("Track {id}"),
        bpm,
        key: key.map(k),
        intensity,
        energy: None,
        situations: Some(vec![format!("p{id}")]),
        genres: None,
        added: None,
    }
}

/// `extra` plus a `[situations]` section with weight 0 that pins track
/// `p0` to the first position, `p1` to the second, ..., so the order is
/// the pool's and every score is the one under test.
fn pinned(extra: &str, pool: &[Candidate]) -> Plan {
    let n = pool.len();
    let places: Vec<String> = (0..n)
        .map(|i| {
            let at = i as f64 / (n - 1) as f64;
            format!("{{ situation = \"p{i}\", from = {at}, to = {at}, require = true }}")
        })
        .collect();
    let text = format!(
        "version = 1\ntracks = {n}\n{extra}\n[situations]\nweight = 0.0\nplace = [{}]\n",
        places.join(", ")
    );
    plan(&recipe(&text), pool).unwrap_or_else(|u| panic!("{}", u.message))
}

fn criterion<'p>(p: &'p Plan, transition: usize, c: Criterion) -> (f64, &'p str) {
    let found = p.transitions[transition]
        .criteria
        .iter()
        .find(|x| x.criterion == c)
        .unwrap_or_else(|| panic!("no {c:?} in {:?}", p.transitions[transition]));
    (found.score, found.reason.as_str())
}

/// The key contribution (weight 1) of `from` to `to`, two tracks pinned.
fn key_step(extra: &str, from: &str, to: &str) -> (f64, String) {
    let pool = [
        track("0", Some(124.0), Some(from), None),
        track("1", Some(124.0), Some(to), None),
    ];
    let p = pinned(&format!("[key]\n{extra}"), &pool);
    let (score, reason) = criterion(&p, 0, Criterion::Key);
    (score, reason.to_owned())
}

// ---- 1. Directed, musically graded key moves ----

#[test]
fn every_move_has_a_name_and_a_cost_by_direction_and_ring() {
    // (from, to, name, cost): the review's cases, from 8A and from 8B.
    let table = [
        ("8A", "8A", "same key", 0.0),
        ("8A", "9A", "+1, a fifth up", 1.0),
        ("8A", "7A", "-1, a fifth down", 1.0),
        ("8A", "8B", "relative major", 1.0),
        ("8B", "8A", "relative minor", 1.0),
        ("8A", "7B", "consonant diagonal", 1.5),
        ("8B", "9A", "consonant diagonal", 1.5),
        ("8A", "10A", "+2, energy boost", 2.0),
        ("8A", "6A", "-2, wind-down", 2.0),
        ("8A", "11B", "parallel major", 2.0),
        ("8B", "5A", "parallel minor", 2.0),
        ("8A", "9B", "clashing diagonal", 3.0),
        ("8B", "7A", "clashing diagonal", 3.0),
        ("8A", "5A", "-3", 3.0),
        ("8A", "11A", "+3", 3.0),
        ("8A", "12A", "+4", 4.0),
        ("8A", "3A", "+7, a semitone up", 5.0),
        ("8A", "1A", "-7, a semitone down", 5.0),
        ("8A", "2A", "tritone", 6.0),
        ("8A", "5B", "tritone ring flip", 6.0),
        ("8B", "11A", "tritone ring flip", 6.0),
    ];
    for (from, to, name, cost) in table {
        let step = k(from).step_to(k(to));
        assert_eq!(step.name(), name, "{from} to {to}");
        assert_eq!(step.cost(), cost, "{from} to {to}");
    }
    // The wheel wraps: 12A to 1A is +1, 1A to 12A is -1.
    assert_eq!(k("12A").step_to(k("1A")).name(), "+1, a fifth up");
    assert_eq!(k("1A").step_to(k("12A")).name(), "-1, a fifth down");
}

#[test]
fn parallel_keys_score_above_the_tritone_flip_and_the_diagonals_differ() {
    // Before: 8A to 11B and 8A to 5B were both 4 moves, both 0; 8A to 7B and
    // 8A to 9B both 2 moves, both 0.5.
    let (parallel, reason) = key_step("", "8A", "11B");
    assert!(reason.contains("parallel major"), "{reason}");
    let (flip, _) = key_step("", "8A", "5B");
    assert!(parallel > flip, "{parallel} vs {flip}");
    assert_eq!(flip, 0.0, "the tritone ring flip is the worst");
    let (consonant, _) = key_step("", "8A", "7B");
    let (clashing, reason) = key_step("", "8A", "9B");
    assert!(consonant > clashing, "{consonant} vs {clashing}");
    assert!(reason.contains("clashing diagonal"), "{reason}");
    // Score = 1 - cost / 6.
    assert_eq!(key_step("", "8A", "9A").0, 0.8333);
    assert_eq!(key_step("", "8A", "8A").0, 1.0);
}

#[test]
fn max_moves_admits_by_cost() {
    // max_moves 2 admits the consonant diagonal and the parallel key, not
    // the clashing diagonal (which it admitted as 2 moves before).
    let two = "max_moves = 2\n";
    let pool = |to: &str| {
        [
            track("0", Some(124.0), Some("8A"), None),
            track("1", Some(124.0), Some(to), None),
        ]
    };
    let text = |extra: &str| {
        format!(
            "version = 1\ntracks = 2\n[key]\n{extra}\n[situations]\nweight = 0.0\nplace = [{{ situation = \"p0\", from = 0.0, to = 0.0, require = true }}]\n"
        )
    };
    for to in ["7B", "11B", "10A", "6A"] {
        assert!(plan(&recipe(&text(two)), &pool(to)).is_ok(), "8A to {to}");
    }
    let refused = plan(&recipe(&text(two)), &pool("9B")).unwrap_err();
    assert_eq!(refused.criterion, Blocker::KeyMoves);
    assert!(plan(&recipe(&text("max_moves = 3\n")), &pool("9B")).is_ok());
}

#[test]
fn a_boost_with_a_rising_shape_beats_a_wind_down_and_the_reverse() {
    // +2 and -2 both cost 2 against max_moves, but a climbing Intensity
    // shape prefers the energy boost and a falling one the wind-down.
    let climbing = "[intensity]\nshape = [{ at = 0.0, intensity = 0.4 }, { at = 1.0, intensity = 0.6 }]\nstep_weight = 0.0\nmissing = \"allow\"\n";
    let falling = "[intensity]\nshape = [{ at = 0.0, intensity = 0.6 }, { at = 1.0, intensity = 0.4 }]\nstep_weight = 0.0\nmissing = \"allow\"\n";
    let key_of = |shape: &str, to: &str| {
        let pool = [
            track("0", Some(124.0), Some("8A"), None),
            track("1", Some(124.0), Some(to), None),
        ];
        let p = pinned(&format!("{shape}[key]\n"), &pool);
        let (score, reason) = criterion(&p, 0, Criterion::Key);
        (score, reason.to_owned())
    };
    let (boost_up, reason) = key_of(climbing, "10A");
    assert!(reason.contains("energy boost"), "{reason}");
    let (wind_up, _) = key_of(climbing, "6A");
    assert!(boost_up > wind_up, "{boost_up} vs {wind_up}");
    let (boost_down, _) = key_of(falling, "10A");
    let (wind_down, reason) = key_of(falling, "6A");
    assert!(wind_down > boost_down, "{wind_down} vs {boost_down}");
    assert!(reason.contains("wind-down"), "{reason}");
    // Without a shape they are alike.
    assert_eq!(key_step("", "8A", "10A").0, key_step("", "8A", "6A").0);
}

#[test]
fn a_version_one_recipe_still_parses_and_new_fields_default() {
    let r = recipe(
        "version = 1\ntracks = 4\n[intensity]\nshape = [{ at = 0.0, intensity = 0.5 }]\n[tempo]\nmax_change = 4.0\n[key]\nmax_moves = 1\n",
    );
    let key = r.key.unwrap();
    assert!(key.key_lock);
    assert_eq!(key.variety, 0.0);
    assert_eq!(key.max_detune, None);
    let tempo = r.tempo.unwrap();
    assert!(!tempo.fold);
    assert_eq!((tempo.min_bpm, tempo.max_bpm), (None, None));
    assert_eq!(r.intensity.unwrap().step_weight, 0.5);
}

// ---- 2. The effective key without key lock ----

/// Two tracks pinned, `from` then `to`: (bpm, key).
fn pair(extra: &str, from: (f64, &str), to: (f64, &str)) -> Result<Plan, Blocker> {
    let pool = [
        track("0", Some(from.0), Some(from.1), None),
        track("1", Some(to.0), Some(to.1), None),
    ];
    let text = format!(
        "version = 1\ntracks = 2\n{extra}\n[situations]\nweight = 0.0\nplace = [{{ situation = \"p0\", from = 0.0, to = 0.0, require = true }}]\n"
    );
    plan(&recipe(&text), &pool).map_err(|u| u.criterion)
}

#[test]
fn without_key_lock_a_tempo_change_shifts_the_incoming_key() {
    // The guide's case: a 1A track at 120 BPM played at 127.1 (+5.92%) rises
    // 0.9952 semitones and sounds in 8A, 0.48 cents flat.
    let free = pair("[key]\nkey_lock = false\n", (127.1, "8A"), (120.0, "1A")).unwrap();
    let (score, reason) = criterion(&free, 0, Criterion::Key);
    assert!(reason.contains("heard as 8A"), "{reason}");
    assert!(reason.contains("same key"), "{reason}");
    assert!(reason.contains("+1 semitone"), "{reason}");
    assert_eq!(score, 0.9903, "1 x (1 - 0.4845 / 50)");
    // With key lock (the default) it is a semitone down, cost 5.
    let locked = pair("[key]\n", (127.1, "8A"), (120.0, "1A")).unwrap();
    let (score, reason) = criterion(&locked, 0, Criterion::Key);
    assert!(reason.contains("-7, a semitone down"), "{reason}");
    assert_eq!(score, 0.1667);
    // The same change on an 8A track makes it 3A: a semitone clash.
    let clash = pair("[key]\nkey_lock = false\n", (127.1, "8A"), (120.0, "8A")).unwrap();
    let (_, reason) = criterion(&clash, 0, Criterion::Key);
    assert!(reason.contains("heard as 3A"), "{reason}");
    assert!(reason.contains("+7, a semitone up"), "{reason}");
    // And a key limit is checked on the key as heard.
    assert_eq!(
        pair(
            "[key]\nkey_lock = false\nmax_moves = 1\n",
            (127.1, "8A"),
            (120.0, "8A")
        )
        .unwrap_err(),
        Blocker::KeyMoves
    );
    assert!(pair(
        "[key]\nkey_lock = false\nmax_moves = 1\n",
        (127.1, "8A"),
        (120.0, "1A")
    )
    .is_ok());
}

#[test]
fn residual_detuning_beyond_the_limit_is_refused_and_named() {
    // Two 8A tracks at 125 and 129 BPM: 54.5 cents apart, 45.5 cents from
    // the nearest semitone.
    let limited = "[key]\nkey_lock = false\nmax_detune = 25\n";
    assert_eq!(
        pair(limited, (129.0, "8A"), (125.0, "8A")).unwrap_err(),
        Blocker::KeyDetune
    );
    // 124 to 125.5 BPM (1.2%, 20.7 cents) is within it.
    let fine = pair(limited, (124.0, "8A"), (125.5, "8A")).unwrap();
    let (score, reason) = criterion(&fine, 0, Criterion::Key);
    assert!(reason.contains("-21 cents"), "{reason}");
    assert!(score < 1.0 && score > 0.5, "detuning costs: {score}");
    // max_detune means nothing with key lock on.
    let problems = Recipe::parse("version = 1\ntracks = 2\n[key]\nmax_detune = 25\n").unwrap_err();
    assert!(
        problems[0].message.contains("key_lock = false"),
        "{problems:?}"
    );
    assert_eq!(problems[0].line, Some(4));
}

// ---- 3. Tempo: symmetric, folded, bounded ----

#[test]
fn a_tempo_change_is_the_same_up_and_down() {
    // 120 to 125 and 125 to 120 are the same stretch (25/24, 4.17%).
    let limit = "[tempo]\nmax_change = 4.0\n";
    assert_eq!(
        pair(limit, (120.0, "8A"), (125.0, "8A")).unwrap_err(),
        Blocker::TempoChange
    );
    assert_eq!(
        pair(limit, (125.0, "8A"), (120.0, "8A")).unwrap_err(),
        Blocker::TempoChange
    );
    let wider = "[tempo]\nmax_change = 4.2\n";
    let up = pair(wider, (120.0, "8A"), (125.0, "8A")).unwrap();
    let down = pair(wider, (125.0, "8A"), (120.0, "8A")).unwrap();
    let (up_score, up_reason) = criterion(&up, 0, Criterion::Tempo);
    let (down_score, down_reason) = criterion(&down, 0, Criterion::Tempo);
    assert_eq!(up_score, down_score);
    assert!(up_reason.ends_with("+4.2%"), "{up_reason}");
    assert!(down_reason.ends_with("-4.2%"), "{down_reason}");
}

#[test]
fn half_and_double_time_fold_when_asked() {
    let folded = pair(
        "[tempo]\nfold = true\nmax_change = 3\n",
        (170.0, "8A"),
        (85.0, "8A"),
    )
    .unwrap();
    let (score, reason) = criterion(&folded, 0, Criterion::Tempo);
    assert_eq!(score, 1.0);
    assert!(reason.contains("double time"), "{reason}");
    assert_eq!(
        pair("[tempo]\nmax_change = 3\n", (170.0, "8A"), (85.0, "8A")).unwrap_err(),
        Blocker::TempoChange
    );
}

#[test]
fn a_set_wide_tempo_range_leaves_out_and_names_tracks_outside_it() {
    let pool = [
        track("0", Some(122.0), Some("8A"), None),
        track("1", Some(126.0), Some("8A"), None),
        track("2", Some(140.0), Some("8A"), None),
        track("3", Some(63.0), Some("8A"), None),
    ];
    let p = plan(
        &recipe("version = 1\ntracks = 2\n[tempo]\nmin_bpm = 120\nmax_bpm = 130\n"),
        &pool,
    )
    .unwrap();
    let out: Vec<&str> = p.left_out.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(out, ["2", "3"]);
    assert_eq!(p.left_out[0].lacks, [Lack::TempoRange]);
    assert!(
        p.left_out[0]
            .message
            .contains("140.00 BPM, outside the Recipe's tempo range (120 to 130 BPM)"),
        "{}",
        p.left_out[0].message
    );
    // Folded, 63 BPM is 126 at double time.
    let folded = plan(
        &recipe("version = 1\ntracks = 2\n[tempo]\nfold = true\nmin_bpm = 120\nmax_bpm = 130\n"),
        &pool,
    )
    .unwrap();
    let out: Vec<&str> = folded.left_out.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(out, ["2"]);
    assert!(
        Recipe::parse("version = 1\ntracks = 2\n[tempo]\nmin_bpm = 130\nmax_bpm = 120\n").is_err()
    );
}

// ---- 4. Intensity: the step between neighbours; Energy as a fallback ----

const FLAT: &str = "[intensity]\nshape = [{ at = 0.0, intensity = 0.6 }]\n";

fn intensities(values: &[f64]) -> Vec<Candidate> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| track(&i.to_string(), None, None, Some(*v)))
        .collect()
}

#[test]
fn a_zigzag_round_the_shape_scores_below_a_steady_run() {
    // The guide's corrected example: 0.5, 0.7, 0.5 round a flat 0.6 was
    // 0.9 a transition, the same as a steady 0.5, 0.5, 0.5.
    let zigzag = pinned(FLAT, &intensities(&[0.5, 0.7, 0.5]));
    let steady = pinned(FLAT, &intensities(&[0.5, 0.5, 0.5]));
    let scores = |p: &Plan| p.transitions.iter().map(|t| t.score).collect::<Vec<_>>();
    assert_eq!(scores(&steady), [0.9, 0.9]);
    assert_eq!(scores(&zigzag), [0.8, 0.8], "0.9 - 0.5 x 0.2");
    assert!(zigzag.score < steady.score);
    let (_, reason) = criterion(&zigzag, 0, Criterion::Intensity);
    assert!(
        reason.contains("a step of +0.20 from 0.50 where the shape steps +0.00"),
        "{reason}"
    );
    // Free to order them, the sequencer does not zigzag.
    let free = plan(
        &recipe(&format!("version = 1\ntracks = 3\n{FLAT}")),
        &intensities(&[0.5, 0.7, 0.5]),
    )
    .unwrap();
    assert_ne!(free.tracks[1].intensity, Some(0.7), "{:?}", free.tracks);
    // step_weight = 0 scores the curve only, as before.
    let curve_only = pinned(
        &format!("{FLAT}step_weight = 0.0\n"),
        &intensities(&[0.5, 0.7, 0.5]),
    );
    assert_eq!(scores(&curve_only), [0.9, 0.9]);
}

#[test]
fn energy_stands_in_for_a_missing_intensity_when_asked() {
    let mut pool = intensities(&[0.5, 0.5]);
    let mut rated = track("2", None, None, None);
    rated.energy = Some(4);
    let mut neither = track("3", None, None, None);
    neither.energy = None;
    pool.push(rated);
    pool.push(neither);
    let text = "version = 1\ntracks = 3\n[intensity]\nshape = [{ at = 0.0, intensity = 0.6 }]\nmax_step = 0.3\nmissing = \"energy\"\n";
    let p = plan(&recipe(text), &pool).unwrap();
    assert_eq!(p.left_out.len(), 1);
    assert_eq!(p.left_out[0].id, "3");
    assert!(
        p.left_out[0].message.contains("no Intensity and no Energy"),
        "{}",
        p.left_out[0].message
    );
    let reasons: Vec<&str> = p
        .opening
        .iter()
        .chain(p.transitions.iter().flat_map(|t| t.criteria.iter()))
        .filter(|c| c.criterion == Criterion::Intensity)
        .map(|c| c.reason.as_str())
        .collect();
    assert!(
        reasons
            .iter()
            .any(|r| r.starts_with("Energy 4 standing in for Intensity: 0.75")),
        "{reasons:?}"
    );
    // Energy 4 is 0.75, more than 0.3 above 0.5... the step limit holds on it.
    let tight = text.replace("max_step = 0.3", "max_step = 0.2");
    assert_eq!(
        plan(&recipe(&tight), &pool).unwrap_err().criterion,
        Blocker::IntensityStep
    );
}

// ---- 5. Variety, and missing data apart from a clash ----

#[test]
fn variety_keeps_a_set_from_sitting_in_one_key() {
    let pool = [
        track("0", Some(124.0), Some("8A"), None),
        track("1", Some(124.0), Some("8A"), None),
        track("2", Some(124.0), Some("9A"), None),
        track("3", Some(124.0), Some("8A"), None),
    ];
    let keys = |p: &Plan| p.tracks.iter().map(|t| t.key.clone()).collect::<Vec<_>>();
    let monotone = plan(&recipe("version = 1\ntracks = 3\n[key]\n"), &pool).unwrap();
    assert_eq!(keys(&monotone), ["8A", "8A", "8A"]);
    let varied = plan(
        &recipe("version = 1\ntracks = 3\n[key]\nvariety = 0.5\n"),
        &pool,
    )
    .unwrap();
    assert!(
        keys(&varied).contains(&"9A".to_owned()),
        "{:?}",
        keys(&varied)
    );
    let same = varied
        .transitions
        .iter()
        .flat_map(|t| t.criteria.iter())
        .find(|c| c.reason.contains("same key"));
    if let Some(same) = same {
        assert!(
            same.reason.contains("the Recipe wants variety"),
            "{}",
            same.reason
        );
    }
}

#[test]
fn a_missing_key_or_tempo_is_scored_apart_from_the_worst_case_and_named() {
    let tritone = pair("[key]\n[tempo]\n", (124.0, "8A"), (124.0, "2A")).unwrap();
    assert_eq!(criterion(&tritone, 0, Criterion::Key).0, 0.0);
    let pool = [
        track("0", Some(124.0), Some("8A"), None),
        track("1", None, None, None),
    ];
    let p = pinned("[key]\n[tempo]\n", &pool);
    let (key, reason) = criterion(&p, 0, Criterion::Key);
    assert_eq!(key, 0.5);
    assert!(
        reason.contains("track 1 has no key") && reason.contains("not as a clash"),
        "{reason}"
    );
    let (tempo, reason) = criterion(&p, 0, Criterion::Tempo);
    assert_eq!(tempo, 0.5);
    assert!(reason.contains("track 1 has no tempo"), "{reason}");
}
