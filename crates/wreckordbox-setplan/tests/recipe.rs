//! A Recipe file is validated, with clear messages for mistakes.
use wreckordbox_setplan::recipe::{Recipe, Problem};

fn problems(text: &str) -> Vec<Problem> {
    Recipe::parse(text).expect_err("the Recipe should be refused")
}

fn messages(text: &str) -> Vec<String> {
    problems(text).into_iter().map(|p| p.to_string()).collect()
}

const GOOD: &str = r#"
version = 1
name = "Warm-up to peak"
tracks = 6

[intensity]
shape = [
  { at = 0.0, intensity = 0.3 },
  { at = 0.7, intensity = 0.9 },
  { at = 1.0, intensity = 0.5 },
]
max_step = 0.3

[tempo]
max_change = 4.0

[key]
max_moves = 1

[situations]
place = [
  { situation = "opener", from = 0.0, to = 0.0, require = true },
  { situation = "closer", from = 0.9, to = 1.0 },
]

[recency]
weight = 0.5
half_life_days = 90
as_of = "2026-11-01"

[genre]
weight = 0.5
mix = "vary"
"#;

#[test]
fn a_whole_recipe_is_read() {
    let recipe = Recipe::parse(GOOD).expect("valid");
    assert_eq!(recipe.name, "Warm-up to peak");
    assert_eq!(recipe.tracks, 6);
    let intensity = recipe.intensity.as_ref().unwrap();
    assert_eq!(intensity.shape.len(), 3);
    assert_eq!(intensity.max_step, Some(0.3));
    assert!((intensity.target(0.35) - 0.6).abs() < 1e-9, "halfway between 0.3 and 0.9");
    assert!((intensity.target(0.85) - 0.7).abs() < 1e-9);
    assert_eq!(recipe.tempo.as_ref().unwrap().max_change, Some(4.0));
    assert_eq!(recipe.key.as_ref().unwrap().max_moves, Some(1));
    let places = &recipe.situations.as_ref().unwrap().place;
    assert_eq!(places[0].situation, "opener");
    assert!(places[0].require);
    assert!(!places[1].require);
    assert_eq!(recipe.recency.as_ref().unwrap().as_of.as_deref(), Some("2026-11-01"));
    assert_eq!(recipe.situation_names(), vec!["closer".to_owned(), "opener".to_owned()]);
}

#[test]
fn a_toml_mistake_names_its_line() {
    let found = problems("version = 1\ntracks = 4\n[tempo\nmax_change = 3\n");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, Some(3), "{found:?}");
}

#[test]
fn an_unknown_key_is_refused_with_its_name_and_line() {
    let found = problems("version = 1\ntracks = 4\n\n[tempo]\nmax_chnage = 3\n");
    assert_eq!(found[0].line, Some(5), "{found:?}");
    assert!(found[0].message.contains("max_chnage"), "{found:?}");
}

#[test]
fn version_and_length_are_required() {
    let found = messages("name = \"x\"\n");
    assert!(found.iter().any(|m| m.contains("version")), "{found:?}");
    let found = messages("version = 2\ntracks = 3\n");
    assert!(found.iter().any(|m| m.contains("version 2")), "{found:?}");
    let found = messages("version = 1\ntracks = 0\n");
    assert!(found.iter().any(|m| m.contains("tracks") && m.contains("at least 2")), "{found:?}");
}

#[test]
fn every_semantic_mistake_is_reported_together_with_its_line() {
    let text = r#"version = 1
tracks = 5

[intensity]
shape = [
  { at = 0.5, intensity = 0.4 },
  { at = 0.2, intensity = 1.4 },
]
max_step = 0

[tempo]
max_change = -2

[key]
max_moves = 8

[situations]
place = [
  { situation = "", from = 0.0, to = 0.1 },
  { situation = "closer", from = 0.9, to = 0.5 },
]

[recency]
half_life_days = 0
as_of = "yesterday"
"#;
    let found = problems(text);
    let text_of = |needle: &str| {
        found
            .iter()
            .find(|p| p.message.contains(needle))
            .unwrap_or_else(|| panic!("no problem mentioning {needle:?} in {found:#?}"))
    };
    assert_eq!(text_of("intensity.shape").line, Some(7), "out of order point");
    assert!(found.iter().any(|p| p.message.contains("1.4")), "{found:#?}");
    text_of("intensity.max_step");
    text_of("tempo.max_change");
    text_of("key.max_moves");
    assert_eq!(text_of("situation name").line, Some(19));
    assert_eq!(text_of("from 0.9").line, Some(20));
    text_of("recency.half_life_days");
    text_of("recency.as_of");
    // `mix` is an enum: serde refuses it, alone, before the checks.
    let found = messages("version = 1\ntracks = 3\n[genre]\nmix = \"shuffle\"\n");
    assert!(found[0].contains("shuffle") && found[0].contains("vary"), "{found:?}");
}

#[test]
fn problems_are_sorted_by_line() {
    let text = "version = 1\ntracks = 1\n[key]\nmax_moves = 9\n";
    let found = problems(text);
    let lines: Vec<_> = found.iter().map(|p| p.line).collect();
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(lines, sorted);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn allowing_missing_intensity_with_a_step_limit_is_refused() {
    let text = "version = 1\ntracks = 3\n[intensity]\nshape = [{ at = 0, intensity = 0.5 }]\nmax_step = 0.2\nmissing = \"allow\"\n";
    let found = messages(text);
    assert!(found.iter().any(|m| m.contains("missing") && m.contains("max_step")), "{found:?}");
}

#[test]
fn a_recipe_needs_at_least_one_shape_point() {
    let found = messages("version = 1\ntracks = 3\n[intensity]\nshape = []\n");
    assert!(found.iter().any(|m| m.contains("intensity.shape") && m.contains("at least one")), "{found:?}");
}
