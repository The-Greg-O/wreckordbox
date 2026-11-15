//! The sequencer: a Recipe and a pool of tracks in, a Set plan (or the
//! criterion in the way) out. Pure: no Library, no Sidecar, no clock.
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

fn track(id: u32, bpm: f64, key: &str, intensity: f64) -> Candidate {
    Candidate {
        id: id.to_string(),
        title: format!("Track {id}"),
        bpm: Some(bpm),
        key: Camelot::parse(key),
        intensity: Some(intensity),
        situations: None,
        genres: None,
        added: None,
    }
}

/// A made-up pool: tempos 120..132, keys round the wheel, Intensity 0.1..0.95,
/// spread by a fixed rule (no randomness).
fn pool(n: u32) -> Vec<Candidate> {
    (0..n)
        .map(|i| {
            let bpm = 120.0 + f64::from((i * 7) % 13);
            let key = ["8A", "9A", "8B", "7A", "9B", "10A"][((i * 5) % 6) as usize];
            let intensity = 0.1 + f64::from((i * 11) % 18) * 0.05;
            let mut t = track(10_000 + i, bpm, key, intensity);
            t.added = Some(format!("2026-{:02}-{:02}", i % 12 + 1, i % 28 + 1));
            t.genres = Some(vec![
                ["house", "techno", "disco"][(i % 3) as usize].to_owned()
            ]);
            t
        })
        .collect()
}

const JOURNEY: &str = r#"
version = 1
name = "Build and land"
tracks = 8
[intensity]
shape = [{ at = 0, intensity = 0.2 }, { at = 0.7, intensity = 0.9 }, { at = 1, intensity = 0.5 }]
[tempo]
max_change = 3
[key]
max_moves = 1
[recency]
weight = 0.3
[genre]
weight = 0.3
"#;

fn built(text: &str, pool: &[Candidate]) -> Plan {
    plan(&recipe(text), pool).unwrap_or_else(|u| panic!("{}", u.message))
}

#[test]
fn the_same_recipe_and_pool_give_the_same_plan_byte_for_byte_in_any_order() {
    let first = built(JOURNEY, &pool(40));
    let again = built(JOURNEY, &pool(40));
    let mut reversed = pool(40);
    reversed.reverse();
    let shuffled = built(JOURNEY, &reversed);
    let bytes = |p: &Plan| serde_json::to_string(p).unwrap();
    assert_eq!(bytes(&first), bytes(&again));
    assert_eq!(
        bytes(&first),
        bytes(&shuffled),
        "the pool's order is not an input"
    );
    assert_eq!(first.tracks.len(), 8);
}

#[test]
fn tempo_and_key_limits_are_never_broken() {
    for size in [20, 40, 60] {
        let p = built(JOURNEY, &pool(size));
        for pair in p.tracks.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let (ta, tb) = (a.bpm.unwrap(), b.bpm.unwrap());
            assert!((tb - ta).abs() / ta * 100.0 <= 3.0 + 1e-9, "{ta} -> {tb}");
            let (ka, kb) = (
                Camelot::parse(&a.key).unwrap(),
                Camelot::parse(&b.key).unwrap(),
            );
            assert!(ka.moves(kb) <= 1, "{} -> {}", a.key, b.key);
        }
        let mut ids: Vec<_> = p.tracks.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 8, "no track twice");
    }
}

#[test]
fn camelot_moves_go_round_the_wheel() {
    let k = |s: &str| Camelot::parse(s).unwrap();
    assert_eq!(k("8A").moves(k("8A")), 0);
    assert_eq!(k("8A").moves(k("8B")), 1);
    assert_eq!(k("12A").moves(k("1A")), 1);
    assert_eq!(k("8A").moves(k("9B")), 2);
    assert_eq!(k("2A").moves(k("8B")), 7);
    assert_eq!(k("Am").to_string(), "8A", "classic spellings are read");
    assert_eq!(Camelot::parse("nonsense"), None);
}

#[test]
fn every_transition_carries_its_score_and_each_criterions_contribution() {
    let p = built(JOURNEY, &pool(40));
    assert_eq!(p.transitions.len(), 7);
    for t in &p.transitions {
        let criteria: Vec<Criterion> = t.criteria.iter().map(|c| c.criterion).collect();
        assert_eq!(
            criteria,
            [
                Criterion::Intensity,
                Criterion::Key,
                Criterion::Tempo,
                Criterion::Recency,
                Criterion::Genre
            ]
        );
        let sum: f64 = t.criteria.iter().map(|c| c.score).sum();
        assert!((sum - t.score).abs() < 1e-3, "{} vs {sum}", t.score);
        assert!(t.criteria.iter().all(|c| !c.reason.is_empty()));
    }
    let key = &p.transitions[0].criteria[1];
    assert!(
        key.reason.contains(&p.tracks[0].key) && key.reason.contains(&p.tracks[1].key),
        "{}",
        key.reason
    );
    let tempo = &p.transitions[0].criteria[2];
    assert!(tempo.reason.contains("BPM"), "{}", tempo.reason);
    let total: f64 = p.opening.iter().map(|c| c.score).sum::<f64>()
        + p.transitions.iter().map(|t| t.score).sum::<f64>();
    assert!((total - p.score).abs() < 1e-3);
}

#[test]
fn the_plan_follows_the_intensity_shape() {
    let p = built(JOURNEY, &pool(60));
    let first = p.tracks[0].intensity.unwrap();
    let peak = p.tracks[5].intensity.unwrap(); // at 5/7 = 0.71, the shape's top
    let last = p.tracks[7].intensity.unwrap();
    assert!(peak > first + 0.3, "builds: {first} .. {peak}");
    assert!(peak > last, "lands: {peak} .. {last}");
    for placed in &p.tracks {
        assert!(placed.target_intensity.is_some());
    }
}

#[test]
fn a_tempo_limit_too_tight_for_the_pool_is_named() {
    let mut tracks = Vec::new();
    for (i, bpm) in [120.0, 128.0, 136.0, 144.0].into_iter().enumerate() {
        tracks.push(track(10_000 + i as u32, bpm, "8A", 0.5));
    }
    let text = "version = 1\ntracks = 3\n[tempo]\nmax_change = 2\n[key]\nmax_moves = 1\n";
    let refused = plan(&recipe(text), &tracks).expect_err("no two tracks are within 2%");
    assert_eq!(refused.criterion, Blocker::TempoChange);
    assert_eq!(refused.in_the_way, vec![Blocker::TempoChange]);
    assert!(refused.message.contains("tempo"), "{}", refused.message);
    assert!(refused.message.contains("2%"), "{}", refused.message);
}

#[test]
fn a_key_limit_in_the_way_is_named_and_not_the_tempo() {
    let tracks: Vec<_> = ["1A", "4A", "7A", "10A"]
        .iter()
        .enumerate()
        .map(|(i, key)| track(10_000 + i as u32, 124.0, key, 0.5))
        .collect();
    let text = "version = 1\ntracks = 2\n[tempo]\nmax_change = 2\n[key]\nmax_moves = 1\n";
    let refused = plan(&recipe(text), &tracks).expect_err("every key is three moves apart");
    assert_eq!(refused.in_the_way, vec![Blocker::KeyMoves]);
    assert!(refused.message.contains("key"), "{}", refused.message);
}

#[test]
fn a_pool_too_small_says_how_many_are_usable_and_why_the_rest_are_not() {
    let mut tracks = pool(5);
    tracks[0].intensity = None;
    tracks[1].key = None;
    tracks[2].bpm = None;
    let refused = plan(&recipe(JOURNEY), &tracks).expect_err("8 asked, 2 usable");
    assert_eq!(refused.criterion, Blocker::Tracks);
    assert!(refused.message.contains("8 tracks"), "{}", refused.message);
    assert!(refused.message.contains("2 usable"), "{}", refused.message);
    assert_eq!(refused.left_out.len(), 3);
    assert_eq!(refused.left_out[0].lacks, vec![Lack::Intensity]);
    assert_eq!(refused.left_out[1].lacks, vec![Lack::Key]);
    assert_eq!(refused.left_out[2].lacks, vec![Lack::Tempo]);
}

#[test]
fn tracks_with_no_intensity_or_key_are_named_never_dropped_silently() {
    let mut tracks = pool(30);
    tracks[3].intensity = None;
    tracks[4].key = None;
    let p = built(JOURNEY, &tracks);
    let named: Vec<(&str, &[Lack])> = p
        .left_out
        .iter()
        .map(|l| (l.id.as_str(), l.lacks.as_slice()))
        .collect();
    assert_eq!(
        named,
        vec![
            ("10003", &[Lack::Intensity][..]),
            ("10004", &[Lack::Key][..])
        ]
    );
    assert_eq!(p.pool, 30);
    assert_eq!(p.usable, 28);

    // With `missing = "allow"` and no key limit, both are usable and their
    // reasons say what was not known.
    let open = "version = 1\ntracks = 30\n[intensity]\nshape = [{ at = 0, intensity = 0.5 }]\nmissing = \"allow\"\n[key]\n";
    let p = built(open, &tracks);
    assert!(p.left_out.is_empty());
    let unknown: Vec<&str> = p
        .transitions
        .iter()
        .flat_map(|t| t.criteria.iter())
        .chain(p.opening.iter())
        .filter(|c| c.reason.contains("no Intensity") || c.reason.contains("no key"))
        .map(|c| c.reason.as_str())
        .collect();
    assert!(
        unknown.iter().any(|r| r.contains("no Intensity")),
        "{unknown:?}"
    );
    assert!(unknown.iter().any(|r| r.contains("no key")), "{unknown:?}");
}

#[test]
fn situations_are_placed_and_a_required_place_is_kept() {
    let mut tracks = pool(30);
    for (i, t) in tracks.iter_mut().enumerate() {
        t.situations = Some(match i {
            7 => vec!["opener".to_owned()],
            9 | 12 => vec!["peak_time".to_owned()],
            _ => vec![],
        });
    }
    let text = r#"version = 1
tracks = 6
[situations]
place = [
  { situation = "opener", from = 0, to = 0, require = true },
  { situation = "peak_time", from = 0.5, to = 0.9 },
]
"#;
    let p = built(text, &tracks);
    assert_eq!(p.tracks[0].id, "10007", "the only opener opens");
    let first = &p.opening;
    assert!(first.iter().any(|c| c.criterion == Criterion::Situations
        && c.score > 0.0
        && c.reason.contains("opener")));
    for t in &p.transitions {
        assert!(t
            .criteria
            .iter()
            .any(|c| c.criterion == Criterion::Situations));
    }
    // A required opener the pool lacks is named.
    for t in &mut tracks {
        t.situations = Some(vec![]);
    }
    let refused = plan(&recipe(text), &tracks).expect_err("no opener");
    assert_eq!(refused.in_the_way, vec![Blocker::Situations]);
    assert!(refused.message.contains("opener"), "{}", refused.message);
}

#[test]
fn a_step_limit_on_intensity_holds() {
    let text = "version = 1\ntracks = 6\n[intensity]\nshape = [{ at = 0, intensity = 0.1 }, { at = 1, intensity = 0.95 }]\nmax_step = 0.15\n";
    let p = built(text, &pool(40));
    for pair in p.tracks.windows(2) {
        let step = (pair[1].intensity.unwrap() - pair[0].intensity.unwrap()).abs();
        assert!(step <= 0.15 + 1e-9, "{step}");
    }
}

#[test]
fn recency_is_measured_to_the_newest_addition_without_a_clock() {
    let mut tracks = pool(4);
    for (t, day) in tracks
        .iter_mut()
        .zip(["2026-01-01", "2026-03-31", "2026-03-01", "2025-03-31"])
    {
        t.added = Some(day.to_owned());
    }
    let text = "version = 1\ntracks = 2\n[recency]\nhalf_life_days = 30\n";
    let p = built(text, &tracks);
    assert_eq!(p.recency_as_of.as_deref(), Some("2026-03-31"));
    let opening = &p.opening[0];
    assert_eq!(opening.criterion, Criterion::Recency);
    // The newest two lead: the first scores 1, the second about half (30 days).
    assert_eq!(p.tracks[0].id, "10001");
    assert!((opening.score - 1.0).abs() < 1e-3, "{}", opening.score);
    assert_eq!(p.tracks[1].id, "10002");
    assert!((p.transitions[0].criteria[0].score - 0.5).abs() < 0.02);
}
