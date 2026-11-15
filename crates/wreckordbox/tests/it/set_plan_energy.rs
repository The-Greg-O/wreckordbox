//! The DJ's Energy standing in for a missing Intensity (wbx-b59), through
//! the real CLI against a service on a fixture Library: the service passes
//! each pool track's Energy rating to the sequencer.
use crate::common;

use common::Sandbox;

const RISING: &str = r#"version = 1
name = "Rising on Energy"
tracks = 3

[intensity]
shape = [{ at = 0.0, intensity = 0.0 }, { at = 1.0, intensity = 1.0 }]
max_step = 0.6
missing = "energy"
"#;

#[test]
fn a_pool_with_no_intensity_is_planned_on_its_energy_when_the_recipe_asks() {
    let sandbox = Sandbox::new();
    // Rated 5, 1 and 3 stars; none has an Intensity measured.
    for (i, stars) in [5u8, 1, 3].iter().enumerate() {
        sandbox.describe_track(i, &format!("Rated {i}"), "", "8A", *stars);
    }
    let ids: Vec<String> = (0..3).map(rbl_db::fixture::track_id).collect();
    let dir = sandbox.path().join("recipes");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("rising.toml");
    std::fs::write(&file, RISING).unwrap();
    let mut args = vec!["--json", "set-plan", "build", file.to_str().unwrap()];
    args.extend(ids.iter().map(String::as_str));

    let run = sandbox.cli(&args);
    assert!(run.status.success(), "{}", run.stderr);
    let built = run.json();
    assert_eq!(built["built"], true, "{built}");
    let plan = &built["set_plan"]["plan"];
    let order: Vec<&str> = plan["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    // 1, 3, 5 stars are 0, 0.5 and 1: the rising shape exactly.
    assert_eq!(order, [ids[1].as_str(), ids[2].as_str(), ids[0].as_str()]);
    assert!(
        plan["opening"][0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("Energy 1 standing in for Intensity: 0.00"),
        "{plan}"
    );

    // Without the fallback, every track lacks an Intensity and is named.
    std::fs::write(&file, RISING.replace("missing = \"energy\"\n", "")).unwrap();
    let refused = sandbox.cli(&args).json();
    assert_eq!(refused["built"], false, "{refused}");
    assert_eq!(
        refused["unsatisfiable"]["left_out"].as_array().unwrap().len(),
        3,
        "{refused}"
    );
}
