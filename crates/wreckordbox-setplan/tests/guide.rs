//! The agents' set planning guide (`docs/agents/set-planning.md`) against
//! the arithmetic: every table and number in it is recomputed here from
//! first principles (pitch-class sets, equal temperament) and from this
//! crate's own key arithmetic and scoring, so the document cannot drift.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use std::collections::BTreeSet;

use wreckordbox_setplan::sequence::Candidate;
use wreckordbox_setplan::{plan, Camelot, Recipe};

const GUIDE: &str = include_str!("../../../docs/agents/set-planning.md");

/// The rows (header and separator dropped) of the table after
/// `<!-- table: NAME -->`.
fn table(name: &str) -> Vec<Vec<String>> {
    let marker = format!("<!-- table: {name} -->");
    let start = GUIDE
        .find(&marker)
        .unwrap_or_else(|| panic!("the guide has no table {name}"));
    GUIDE[start + marker.len()..]
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| l.starts_with('|'))
        .skip(2)
        .map(|l| {
            l.trim()
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect()
        })
        .collect()
}

/// The guide with every run of white space made one space, so a phrase
/// broken across lines is still found.
fn flat() -> String {
    GUIDE.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn num(text: &str) -> f64 {
    text.parse()
        .unwrap_or_else(|_| panic!("{text:?} is not a number"))
}

// ---- Theory, independent of the crate ----

const MAJOR_STEPS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

/// The tonic's pitch class (C = 0) of Camelot `n`, from the circle of
/// fifths: 8B is C, each number a fifth (+7) up, `nA` the relative minor.
fn tonic(n: i32, major: bool) -> i32 {
    let t = (7 * (n - 8)).rem_euclid(12);
    if major {
        t
    } else {
        (t - 3).rem_euclid(12)
    }
}

/// The pitch classes of the (natural minor or major) scale of a key.
fn scale(n: i32, major: bool) -> BTreeSet<i32> {
    let root = if major {
        tonic(n, true)
    } else {
        (tonic(n, false) + 3).rem_euclid(12)
    };
    MAJOR_STEPS
        .iter()
        .map(|s| (root + s).rem_euclid(12))
        .collect()
}

fn shared(a: (i32, bool), b: (i32, bool)) -> usize {
    scale(a.0, a.1).intersection(&scale(b.0, b.1)).count()
}

/// A note name (`C`, `F#`, `Bb`, optionally `m`) as its tonic pitch class.
fn pitch_class(name: &str) -> (i32, bool) {
    let mut chars = name.chars();
    let mut p = match chars.next().unwrap() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        other => panic!("not a note: {other}"),
    };
    let rest = chars.as_str();
    let rest = if let Some(r) = rest.strip_prefix('#') {
        p += 1;
        r
    } else if let Some(r) = rest.strip_prefix('b') {
        p -= 1;
        r
    } else {
        rest
    };
    (i32::rem_euclid(p, 12), rest != "m")
}

fn code(text: &str) -> (i32, bool) {
    let c = Camelot::parse(text).unwrap_or_else(|| panic!("not a Camelot code: {text}"));
    (i32::from(c.number), c.major)
}

fn camelot(key: (i32, bool)) -> Camelot {
    Camelot {
        number: u8::try_from(key.0).unwrap(),
        major: key.1,
    }
}

fn wrap(n: i32) -> i32 {
    (n - 1).rem_euclid(12) + 1
}

fn semitones(percent: f64) -> f64 {
    12.0 * (1.0 + percent / 100.0).log2()
}

/// The key score at weight 1 of a move of this cost (section 4).
fn key_score(cost: f64) -> f64 {
    1.0 - cost / 6.0
}

fn round(x: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (x * f).round() / f
}

// ---- The tables ----

#[test]
fn shared_tones_by_distance_on_the_circle() {
    let rows = table("shared-by-distance");
    assert_eq!(rows.len(), 7);
    for row in rows {
        let d: i32 = row[0].parse().unwrap();
        let expected = shared((8, true), (wrap(8 + d), true));
        assert_eq!(num(&row[1]), expected as f64, "distance {d}");
        // The same for the minor ring, and across rings by the relative.
        assert_eq!(shared((8, false), (wrap(8 + d), false)), expected);
        assert_eq!(shared((8, false), (wrap(8 + d), true)), expected);
    }
    // The textbook anchor: C major and G major differ only by F / F#.
    let major_set =
        |t: i32| -> BTreeSet<i32> { MAJOR_STEPS.iter().map(|s| (t + s).rem_euclid(12)).collect() };
    let (c, g) = (major_set(pitch_class("C").0), major_set(pitch_class("G").0));
    assert_eq!(c.difference(&g).copied().collect::<Vec<_>>(), [5]);
    assert_eq!(g.difference(&c).copied().collect::<Vec<_>>(), [6]);
}

/// The tonic triad of a key: root, third (major or minor), fifth.
fn triad(key: (i32, bool)) -> BTreeSet<i32> {
    let t = tonic(key.0, key.1);
    let third = if key.1 { 4 } else { 3 };
    [t, t + third, t + 7]
        .iter()
        .map(|x| x.rem_euclid(12))
        .collect()
}

/// Open Key from the tonic alone: 1m is A minor, 1d C major, one fifth
/// up per number.
fn open_key(key: (i32, bool)) -> String {
    let p = tonic(key.0, key.1);
    let zero = if key.1 { 0 } else { 9 };
    let o = (7 * (p - zero)).rem_euclid(12) + 1;
    format!("{o}{}", if key.1 { 'd' } else { 'm' })
}

#[test]
fn camelot_table_is_the_circle_of_fifths_and_the_apps_parser() {
    let rows = table("camelot");
    assert_eq!(rows.len(), 12);
    for (i, row) in rows.iter().enumerate() {
        let n = i32::try_from(i + 1).unwrap();
        for (base, major) in [(0, false), (5, true)] {
            let wanted = (n, major);
            assert_eq!(code(&row[base]), wanted, "row {n}");
            let p = tonic(n, major);
            assert_eq!(num(&row[base + 2]), f64::from(p), "{}", row[base]);
            // The guide's formulas, both ways.
            let m = i32::from(major);
            assert_eq!(((7 * n + 1 + 3 * m).rem_euclid(12)), p);
            assert_eq!((7 * p + 4 + 3 * m).rem_euclid(12) + 1, n);
            // Open Key: from the tonic, and by the guide's formula.
            assert_eq!(row[base + 4], open_key(wanted), "{}", row[base]);
            let o = (n + 4).rem_euclid(12) + 1;
            assert_eq!(
                row[base + 4],
                format!("{o}{}", if major { 'd' } else { 'm' })
            );
            assert_eq!((o + 6).rem_euclid(12) + 1, n);
            // Every spelling, the musical names and rekordbox's, is that key
            // both by note arithmetic and by the app's own parser.
            let names = row[base + 1]
                .split('/')
                .map(str::trim)
                .chain([row[base + 3].as_str()]);
            for name in names {
                let (pc, is_major) = pitch_class(name);
                assert_eq!((pc, is_major), (p, major), "{name}");
                assert_eq!(code(name), wanted, "{name} in the app's parser");
            }
        }
    }
}

#[test]
fn moves_between_keys() {
    let rows = table("moves");
    assert_eq!(rows.len(), 21);
    for row in rows {
        let (move_, from, to) = (&row[0], code(&row[1]), code(&row[2]));
        // The arithmetic column: `k`, `k + n`, `k - n`, then `, ring`.
        let (steps, ring) = row[3]
            .split_once(',')
            .map_or((row[3].as_str(), ""), |(a, b)| (a, b));
        let steps = steps.trim();
        let k: i32 = if steps == "k" {
            0
        } else if let Some(k) = steps.strip_prefix("k + ") {
            k.parse().unwrap()
        } else {
            -steps.strip_prefix("k - ").unwrap().parse::<i32>().unwrap()
        };
        let flips = ring.trim() == "ring";
        assert_eq!(to, (wrap(from.0 + k), from.1 ^ flips), "{move_}");
        let up = (tonic(to.0, to.1) - tonic(from.0, from.1)).rem_euclid(12);
        assert_eq!(num(&row[4]), f64::from(up), "{move_}: tonic shift");
        assert_eq!(num(&row[5]), shared(from, to) as f64, "{move_}: scale");
        let triads = triad(from).intersection(&triad(to)).count();
        assert_eq!(num(&row[6]), triads as f64, "{move_}: triad");
        let cost = camelot(from).step_to(camelot(to)).cost();
        assert_eq!(num(&row[7]), cost, "{move_}: cost");
        assert_eq!(num(&row[8]), round(key_score(cost), 4), "{move_}");
    }
}

#[test]
fn relative_and_parallel_tone_counts() {
    let set = |steps: &[i32]| -> BTreeSet<i32> { steps.iter().copied().collect() };
    let major = set(&MAJOR_STEPS);
    let natural = set(&[0, 2, 3, 5, 7, 8, 10]);
    let harmonic = set(&[0, 2, 3, 5, 7, 8, 11]);
    let melodic = set(&[0, 2, 3, 5, 7, 9, 11]);
    assert_eq!(
        major.intersection(&natural).copied().collect::<Vec<_>>(),
        [0, 2, 5, 7]
    );
    assert_eq!(major.intersection(&harmonic).count(), 5);
    assert_eq!(major.intersection(&melodic).count(), 6);
    // Harmonic minor against its relative major (3 semitones up): 6 of 7.
    let relative: BTreeSet<i32> = major.iter().map(|x| (x + 3) % 12).collect();
    assert_eq!(relative, natural);
    assert_eq!(relative.intersection(&harmonic).count(), 6);
    // The shared count is max(2, 7 - d).
    for d in 0..=6 {
        let expected = (7 - d).max(2);
        assert_eq!(shared((8, true), (wrap(8 + d), true)), expected as usize);
    }
}

#[test]
fn semitone_moves_on_the_wheel_are_plus_seven() {
    // Transposing every key up one semitone moves it +7, same ring; +2 is a
    // whole tone. From the tonics alone, for all 24 keys.
    for n in 1..=12 {
        for major in [false, true] {
            assert_eq!(
                tonic(wrap(n + 7), major),
                (tonic(n, major) + 1).rem_euclid(12)
            );
            assert_eq!(
                tonic(wrap(n + 2), major),
                (tonic(n, major) + 2).rem_euclid(12)
            );
        }
    }
}

#[test]
fn tempo_to_pitch() {
    let rows = table("tempo-pitch");
    assert_eq!(rows.len(), 8);
    for row in rows {
        let p = num(&row[0]);
        assert_eq!(num(&row[1]), round(semitones(p), 4), "+{p}%");
        assert_eq!(num(&row[2]), round(semitones(p) * 100.0, 1), "+{p}%");
        assert_eq!(num(&row[3]), round(semitones(-p), 4), "-{p}%");
        assert_eq!(num(&row[4]), round(semitones(-p) * 100.0, 1), "-{p}%");
    }
}

#[test]
fn bpm_steps() {
    let rows = table("bpm-steps");
    assert_eq!(rows.len(), 5);
    for row in rows {
        let (x, y) = (num(&row[0]), num(&row[1]));
        let p = (y - x) / x * 100.0;
        assert_eq!(num(&row[2]), round(p, 3), "{x} to {y}");
        assert_eq!(num(&row[3]), round(semitones(p), 4), "{x} to {y}");
        assert_eq!(num(&row[4]), round(semitones(p) * 100.0, 1), "{x} to {y}");
    }
}

fn pct(semis: f64) -> f64 {
    (2f64.powf(semis / 12.0) - 1.0) * 100.0
}

#[test]
fn numbers_in_the_text() {
    let cents = |ratio: f64| 1200.0 * ratio.log2();
    let shift = semitones((127.1 / 120.0 - 1.0) * 100.0);
    let quarter = semitones((129.0 / 125.0 - 1.0) * 100.0);
    let beat = |f: f64| f * (2f64.powf(quarter / 12.0) - 1.0);
    let onsets = |bpm: f64| 4.0 * bpm / 60.0;
    let said = [
        format!("2^(1/12) (about {:.6})", 2f64.powf(1.0 / 12.0)),
        format!("1200 x log2(1.5) = {:.2} cents", cents(1.5)),
        format!(
            "2^(7/12) = {:.6}, 700 cents, {:.2} cents flat",
            2f64.powf(7.0 / 12.0),
            cents(1.5) - 700.0
        ),
        format!(
            "{:.2} cents sharp of 5:4",
            cents(2f64.powf(4.0 / 12.0) / 1.25)
        ),
        format!(
            "{:.2} cents flat of 6:5",
            -cents(2f64.powf(3.0 / 12.0) / 1.2)
        ),
        format!("{:.2}% at 120", 100.0 / 120.0),
        format!("{:.2}% at 128", 100.0 / 128.0),
        format!("120 to 125 is +{:.3}%", 5.0 / 120.0 * 100.0),
        format!("125 to 120 is {:.3}%", -5.0 / 125.0 * 100.0),
        format!("|ln(125/120)| = {:.4}", (125f64 / 120.0).ln()),
        format!("+{:.2}% of the outgoing", 6.0 / 120.0 * 100.0),
        format!("played at {:.2}%", (120.0 / 126.0 - 1.0) * 100.0),
        format!("+{:.4}% (2^(1/12) - 1)", pct(1.0)),
        format!("{:.4}% (2^(-1/12) - 1)", pct(-1.0)),
        format!("+{:.4}% or {:.4}%; two", pct(0.5), pct(-0.5)),
        format!("+{:.4}% or {:.4}%.", pct(2.0), pct(-2.0)),
        format!(
            "(+{:.2}%) rises {shift:.4} semitones",
            (127.1 / 120.0 - 1.0) * 100.0
        ),
        format!("{:.2} cents flat", (1.0 - shift) * 100.0),
        format!("from about {:.2}% to {:.2}%", pct(0.25), pct(0.75)),
        format!("{:.1} cents apart", quarter * 100.0),
        format!(
            "110 Hz beats at {:.1} Hz, at 440 Hz at {:.1} Hz",
            beat(110.0),
            beat(440.0)
        ),
        format!("1.5% ({:.0} cents)", semitones(1.5) * 100.0),
        format!(
            "+{:.1}% for 4% over 16 tracks",
            (1.04f64.powi(15) - 1.0) * 100.0
        ),
        format!("+{:.1}% for 1%", (1.01f64.powi(15) - 1.0) * 100.0),
        format!(
            "({:.2} at 120, {:.2} at 128, {:.2} at 140)",
            onsets(120.0),
            onsets(128.0),
            onsets(140.0)
        ),
        format!(
            "120 to 125 and 125 to 120 are both {:.2}%",
            (125.0 / 120.0 - 1.0) * 100.0
        ),
        format!("is heard as 8A, {:.2} cents off", (1.0 - shift) * 100.0),
        format!("a sixth of S ({:.2}% at S = 4)", 4.0 / 6.0),
        format!(
            "costs 1/6 ({:.4}), the same as an Intensity {:.4} off",
            1.0 - key_score(1.0),
            1.0 - key_score(1.0)
        ),
        format!("0.70 x 0.70 = {:.0}%", 0.7 * 0.7 * 100.0),
        "Only the hard limits".to_owned(),
    ];
    let guide = flat();
    for text in said {
        assert!(guide.contains(&text), "the guide should say {text:?}");
    }
    // The compensatory example of section 4: 1 + 1 + 0 beats 3 x 0.65.
    let (lopsided, even): (f64, f64) = ([1.0, 1.0, 0.0].iter().sum(), 3.0 * 0.65);
    assert!(lopsided > even && (even - 1.95).abs() < 1e-12);
    assert!(guide.contains("totals 2.0 and beats one scoring 0.65 on all three (1.95)"));
    // A basic key move (cost 1) is a sixth of S in tempo.
    assert!((1.0 - key_score(1.0) - 1.0 / 6.0).abs() < 1e-12);
    // The working rule of 1.5% is about 26 cents, so max_detune = 25 fits it.
    assert!(guide.contains("`key.max_detune = 25` (cents)"));
    assert_eq!(round(semitones(1.5) * 100.0, 0), 26.0);
    // 7 is its own inverse mod 12; a semitone is ~5.95%.
    assert_eq!((7 * 7) % 12, 1);
    assert!((semitones(pct(1.0)) - 1.0).abs() < 1e-12);
}

/// Transition scores of `values`, pinned in that order by required
/// Situations of weight 0, round a flat Intensity of 0.6.
fn pinned_scores(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let places: Vec<String> = (0..n)
        .map(|i| {
            let at = i as f64 / (n - 1) as f64;
            format!("{{ situation = \"p{i}\", from = {at}, to = {at}, require = true }}")
        })
        .collect();
    let recipe = Recipe::parse(&format!(
        "version = 1\ntracks = {n}\n[intensity]\nshape = [{{ at = 0.0, intensity = 0.6 }}]\n[situations]\nweight = 0.0\nplace = [{}]\n",
        places.join(", ")
    ))
    .unwrap();
    let pool: Vec<Candidate> = values
        .iter()
        .enumerate()
        .map(|(i, v)| Candidate {
            id: format!("{i}"),
            title: format!("{i}"),
            bpm: None,
            key: None,
            intensity: Some(*v),
            energy: None,
            situations: Some(vec![format!("p{i}")]),
            genres: None,
            added: None,
        })
        .collect();
    let built = serde_json::to_value(plan(&recipe, &pool).unwrap()).unwrap();
    built["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["score"].as_f64().unwrap())
        .collect()
}

#[test]
fn the_zigzag_scores_below_a_steady_run() {
    // Section 5: 0.5, 0.7, 0.5 round a flat 0.6 now scores 0.8 a
    // transition (0.9 off the curve, less 0.5 x the 0.2 step the shape does
    // not take) against 0.9 for a steady 0.5, 0.5, 0.5. By the sequencer.
    assert_eq!(pinned_scores(&[0.5, 0.7, 0.5]), [0.8, 0.8]);
    assert_eq!(pinned_scores(&[0.5, 0.5, 0.5]), [0.9, 0.9]);
    assert_eq!(round(1.0 - 0.1 - 0.5 * 0.2, 4), 0.8);
    assert!(flat().contains("so 0.5, 0.7, 0.5 round a flat 0.6 now scores 0.8 a transition against 0.9 for a steady 0.5, 0.5, 0.5"));
}

#[test]
fn key_score_table_is_the_sequencers() {
    let rows = table("key-score");
    assert_eq!(rows.len(), 8);
    for row in rows {
        assert_eq!(num(&row[1]), round(key_score(num(&row[0])), 4));
    }
    // Every cost in the table is one the moves table uses.
    let costs: BTreeSet<String> = table("moves").iter().map(|r| r[7].clone()).collect();
    for row in table("key-score") {
        assert!(
            costs.contains(&row[0]),
            "cost {} is not in the moves table",
            row[0]
        );
    }
}

/// The worked transition, built by the real sequencer.
#[test]
fn worked_transition_is_what_the_sequencer_scores() {
    let recipe = Recipe::parse(
        r#"
version = 1
tracks = 2
[intensity]
shape = [{ at = 0.0, intensity = 0.6 }]
[tempo]
max_change = 4.0
[key]
[situations]
place = [{ situation = "opener", from = 0.0, to = 0.0, require = true }]
weight = 0.0
"#,
    )
    .unwrap();
    let track = |id: &str, bpm: f64, key: &str, intensity: f64, situations: Option<Vec<String>>| {
        Candidate {
            id: id.into(),
            title: id.into(),
            bpm: Some(bpm),
            key: Camelot::parse(key),
            intensity: Some(intensity),
            energy: None,
            situations,
            genres: None,
            added: None,
        }
    };
    let pool = [
        track("1", 124.0, "8A", 0.60, Some(vec!["opener".into()])),
        track("2", 126.0, "9A", 0.62, Some(vec![])),
    ];
    let built = plan(&recipe, &pool).unwrap();
    let json = serde_json::to_value(&built).unwrap();
    let transition = &json["transitions"][0];
    let rows = table("worked-transition");
    let mut total = 0.0;
    for row in &rows[..3] {
        let got = transition["criteria"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["criterion"] == row[0].as_str())
            .unwrap_or_else(|| panic!("no {} in {transition}", row[0]));
        assert_eq!(got["score"].as_f64().unwrap(), num(&row[2]), "{}", row[0]);
        total += num(&row[2]);
        let reason = got["reason"].as_str().unwrap();
        let shown = reason.split(',').next().unwrap();
        assert!(flat().contains(shown), "the guide should quote {shown:?}");
    }
    assert_eq!(rows[3][0], "total");
    assert!((num(&rows[3][2]) - round(total, 4)).abs() < 1e-9);
    assert!((transition["score"].as_f64().unwrap() - num(&rows[3][2])).abs() < 1e-9);
    // And from first principles.
    assert_eq!(round(1.0 - 0.02 - 0.5 * 0.02, 4), num(&rows[0][2]));
    assert_eq!(round(1.0 - 1.0 / 6.0, 4), num(&rows[1][2]));
    assert_eq!(round(1.0 - 2.0 / 124.0 * 100.0 / 4.0, 4), num(&rows[2][2]));
}

// ---- The example Recipes ----

fn toml_blocks() -> Vec<&'static str> {
    GUIDE
        .split("```toml\n")
        .skip(1)
        .map(|rest| rest.split("```").next().unwrap())
        .collect()
}

/// The steepest Intensity change per track the Recipe's shape asks for.
fn steepest_step(recipe: &Recipe) -> f64 {
    let shape = &recipe.intensity.as_ref().unwrap().shape;
    let slope = shape
        .windows(2)
        .map(|p| ((p[1].intensity - p[0].intensity) / (p[1].at - p[0].at)).abs())
        .fold(0.0, f64::max);
    #[allow(clippy::cast_precision_loss)]
    let per_track = slope / (recipe.tracks - 1) as f64;
    per_track
}

#[test]
fn example_recipes_are_valid_and_their_steps_fit() {
    let blocks = toml_blocks();
    assert_eq!(blocks.len(), 3, "warm-up, peak-time hour, closing");
    let rows = table("steepest-step");
    assert_eq!(rows.len(), 3);
    for (text, row) in blocks.iter().zip(rows) {
        let recipe = Recipe::parse(text).unwrap_or_else(|p| panic!("{p:?}\n{text}"));
        assert_eq!(recipe.name, row[0]);
        assert_eq!(recipe.tracks.to_string(), row[1]);
        let step = steepest_step(&recipe);
        assert_eq!(num(&row[3]), round(step, 4), "{}", row[0]);
        let shape = recipe.intensity.as_ref().unwrap();
        let slope = shape
            .shape
            .windows(2)
            .map(|p| ((p[1].intensity - p[0].intensity) / (p[1].at - p[0].at)).abs())
            .fold(0.0, f64::max);
        assert_eq!(num(&row[2]), round(slope, 4), "{}", row[0]);
        assert_eq!(num(&row[4]), shape.max_step.unwrap());
        assert!(
            step <= shape.max_step.unwrap(),
            "{}: the shape is steeper than max_step",
            row[0]
        );
    }
}

#[test]
fn shapes_table_points_match_the_examples() {
    // The shapes table's warm-up, peak and closing rows are the examples'.
    let blocks = toml_blocks();
    for (name, block) in [("Warm-up", 0), ("Peak-time hour", 1), ("Closing", 2)] {
        let line = GUIDE
            .lines()
            .find(|l| l.starts_with(&format!("| {name} |")))
            .unwrap_or_else(|| panic!("no shape {name}"));
        let recipe = Recipe::parse(blocks[block]).unwrap();
        let points: Vec<String> = recipe
            .intensity
            .unwrap()
            .shape
            .iter()
            .map(|p| format!("({}, {})", p.at, p.intensity))
            .collect();
        assert!(line.contains(&points.join(", ")), "{line} vs {points:?}");
    }
}
