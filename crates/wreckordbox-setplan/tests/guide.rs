//! The agents' set planning guide (`docs/agents/set-planning.md`) against
//! the arithmetic: every table and number in it is recomputed here from
//! first principles (pitch-class sets, equal temperament) and from this
//! crate's own key arithmetic and scoring, so the document cannot drift.
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

fn key_score(moves: u32) -> f64 {
    (1.0 - f64::from(moves) / 4.0).max(0.0)
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

#[test]
fn camelot_table_is_the_circle_of_fifths_and_the_apps_parser() {
    let rows = table("camelot");
    assert_eq!(rows.len(), 12);
    for (i, row) in rows.iter().enumerate() {
        let n = i32::try_from(i + 1).unwrap();
        for (base, major) in [(0, false), (4, true)] {
            let wanted = (n, major);
            assert_eq!(code(&row[base]), wanted, "row {n}");
            assert_eq!(
                num(&row[base + 2]),
                f64::from(tonic(n, major)),
                "{}",
                row[base]
            );
            // Every spelling, the musical names and rekordbox's, is that key
            // both by note arithmetic and by the app's own parser.
            let names = row[base + 1]
                .split('/')
                .map(str::trim)
                .chain([row[base + 3].as_str()]);
            for name in names {
                let (pc, is_major) = pitch_class(name);
                assert_eq!((pc, is_major), (tonic(n, major), major), "{name}");
                assert_eq!(code(name), wanted, "{name} in the app's parser");
            }
        }
    }
}

#[test]
fn moves_from_8a() {
    let from = (8, false);
    let rows = table("moves");
    assert_eq!(rows.len(), 11);
    for row in rows {
        let to = code(&row[1]);
        // The arithmetic column: `n`, `n + k`, `n - k`, `, A to B`.
        let (steps, ring) = row[2]
            .split_once(',')
            .map_or((row[2].as_str(), ""), |(a, b)| (a, b));
        let steps = steps.trim();
        let k: i32 = if steps == "n" {
            0
        } else if let Some(k) = steps.strip_prefix("n + ") {
            k.parse().unwrap()
        } else {
            -steps.strip_prefix("n - ").unwrap().parse::<i32>().unwrap()
        };
        assert_eq!(to, (wrap(8 + k), ring.trim() == "A to B"), "{}", row[0]);
        assert_eq!(num(&row[3]), shared(from, to) as f64, "{}", row[0]);
        let moves = camelot(from).moves(camelot(to));
        assert_eq!(num(&row[4]), f64::from(moves), "{}", row[0]);
        assert_eq!(num(&row[5]), key_score(moves), "{}", row[0]);
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
    assert!(rows.len() >= 8);
    for row in rows {
        let p = num(&row[0]);
        assert_eq!(num(&row[1]), round(semitones(p), 2), "{p}%");
        assert_eq!(num(&row[2]), round(semitones(p) * 100.0, 0), "{p}%");
    }
}

#[test]
fn numbers_in_the_text() {
    let up = (2f64.powf(1.0 / 12.0) - 1.0) * 100.0;
    let down = (2f64.powf(-1.0 / 12.0) - 1.0) * 100.0;
    let two = (2f64.powf(2.0 / 12.0) - 1.0) * 100.0;
    let said = [
        format!("+{up:.2}% (2^(1/12) - 1)"),
        format!("{down:.2}% (2^(-1/12) - 1)"),
        format!("{two:.2}%"),
        format!("{:.2}% at 120", 100.0 / 120.0),
        format!("{:.2}% at 128", 100.0 / 128.0),
        format!("+{:.2}% of the outgoing", (126.0 - 120.0) / 120.0 * 100.0),
        format!("{:.2}%", (120.0 / 126.0 - 1.0) * 100.0),
        format!("1.5% ({:.0} cents)", semitones(1.5) * 100.0),
        format!(
            "3% (half a semitone, about {:.0} cents)",
            (semitones(3.0) * 100.0 / 10.0).round() * 10.0
        ),
        format!("2^(1/12) (about {:.6})", 2f64.powf(1.0 / 12.0)),
    ];
    let guide = flat();
    for text in said {
        assert!(guide.contains(&text), "the guide should say {text:?}");
    }
    // A semitone is ~5.95%, so a 6% change is just over one.
    assert!(semitones(up) - 1.0 < 1e-12);
}

#[test]
fn key_score_table_is_the_sequencers() {
    for row in table("key-score") {
        let moves: u32 = row[0].parse().unwrap();
        assert_eq!(num(&row[1]), key_score(moves));
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
    for (name, block) in [
        ("Warm-up", 0),
        ("Peak with waves", 1),
        ("Release / closing", 2),
    ] {
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
