//! Building a Set plan from a Recipe (wbx-39), through the real CLI against
//! a service on a fixture Library, with a second client watching.
use crate::common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::Sandbox;
use serde_json::Value;

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

fn recipe(sandbox: &Sandbox, name: &str, text: &str) -> PathBuf {
    let path = sandbox.path().join("recipes").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn build(sandbox: &Sandbox, file: &Path, pool: &[&str]) -> common::Run {
    let mut args = vec!["--json", "set-plan", "build", file.to_str().unwrap()];
    args.extend_from_slice(pool);
    sandbox.cli(&args)
}

/// Every table of the Library with its definition and row count, and the
/// bytes of `master.db`.
fn library_state(sandbox: &Sandbox) -> (Vec<(String, String, i64)>, Vec<u8>) {
    let mut shape = Vec::new();
    sandbox.edit(|writer| {
        let db = writer.library().connection();
        let mut tables = db
            .prepare("SELECT name, coalesce(sql, '') FROM sqlite_master ORDER BY name")
            .unwrap();
        let named: Vec<(String, String)> = tables
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (name, sql) in named {
            let rows = if sql.starts_with("CREATE TABLE") {
                db.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            } else {
                0
            };
            shape.push((name, sql, rows));
        }
    });
    (shape, std::fs::read(sandbox.master_db()).unwrap())
}

fn stop_service(sandbox: &Sandbox) {
    if let Ok(mut client) =
        wreckordbox_service::Client::connect(&sandbox.config().socket_path(sandbox.location()))
    {
        let pid = client.status().unwrap().pid;
        let _ = client.stop();
        let gone = Instant::now() + Duration::from_secs(10);
        while common::alive(u64::from(pid)) && Instant::now() < gone {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Moves on the Camelot wheel, worked out here from the key's text.
fn moves(a: &str, b: &str) -> u32 {
    let split = |k: &str| {
        let (n, letter) = k.split_at(k.len() - 1);
        (n.parse::<i32>().unwrap(), letter.to_owned())
    };
    let ((na, la), (nb, lb)) = (split(a), split(b));
    let apart = (na - nb).unsigned_abs();
    apart.min(12 - apart) + u32::from(la != lb)
}

const KEYS: [&str; 8] = ["8A", "9A", "8B", "7A", "9B", "10A", "8A", "9A"];

/// Eight fixture tracks with keys; the fourth is the only opener.
fn keyed(sandbox: &Sandbox) -> Vec<String> {
    for (i, key) in KEYS.iter().enumerate() {
        sandbox.describe_track(i, &format!("Keyed {i}"), "", key, 3);
    }
    (0..KEYS.len()).map(rbl_db::fixture::track_id).collect()
}

const JOURNEY: &str = r#"version = 1
name = "Warm room"
tracks = 6

[tempo]
max_change = 2

[key]
max_moves = 1

[situations]
place = [{ situation = "opener", from = 0, to = 0, require = true }]
"#;

#[test]
fn a_set_plan_is_built_the_same_twice_kept_in_the_sidecar_and_the_library_is_unchanged() {
    let sandbox = Sandbox::new();
    let ids = keyed(&sandbox);
    let before = library_state(&sandbox);
    ok(
        &sandbox,
        &[
            "--json",
            "facts",
            "record",
            &ids[3],
            "situation",
            "opener",
            "--source",
            "dj",
        ],
    );
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let file = recipe(&sandbox, "warm.toml", JOURNEY);
    let pool: Vec<&str> = ids.iter().map(String::as_str).collect();

    let first = build(&sandbox, &file, &pool);
    assert!(first.status.success(), "{}", first.stderr);
    let first = first.json();
    let again = build(&sandbox, &file, &pool).json();
    assert_eq!(first["built"], true);
    assert_eq!(
        first["set_plan"]["plan"].to_string(),
        again["set_plan"]["plan"].to_string(),
        "the same Recipe, pool and Facts give the same plan"
    );
    assert_ne!(
        first["set_plan"]["id"], again["set_plan"]["id"],
        "each build is kept"
    );

    let plan = &first["set_plan"]["plan"];
    let tracks = plan["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 6);
    assert_eq!(
        tracks[0]["id"],
        ids[3].as_str(),
        "the required opener opens"
    );
    for pair in tracks.windows(2) {
        let (a, b) = (
            pair[0]["key"].as_str().unwrap(),
            pair[1]["key"].as_str().unwrap(),
        );
        assert!(moves(a, b) <= 1, "{a} -> {b} breaks the key limit");
        let (ta, tb) = (
            pair[0]["bpm"].as_f64().unwrap(),
            pair[1]["bpm"].as_f64().unwrap(),
        );
        assert!((tb - ta).abs() / ta * 100.0 <= 2.0, "{ta} -> {tb}");
    }
    let transitions = plan["transitions"].as_array().unwrap();
    assert_eq!(transitions.len(), 5);
    for t in transitions {
        let criteria: Vec<&str> = t["criteria"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["criterion"].as_str().unwrap())
            .collect();
        assert_eq!(criteria, ["key", "tempo", "situations"], "{t}");
        let sum: f64 = t["criteria"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["score"].as_f64().unwrap())
            .sum();
        assert!((sum - t["score"].as_f64().unwrap()).abs() < 1e-3);
        assert!(
            t["criteria"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("move"),
            "{t}"
        );
        assert!(
            t["criteria"][1]["reason"].as_str().unwrap().contains("BPM"),
            "{t}"
        );
    }
    assert_eq!(
        first["set_plan"]["recipe"], JOURNEY,
        "the Recipe's text is kept as read"
    );
    assert_eq!(first["set_plan"]["pool"].as_array().unwrap().len(), 8);
    assert!(first["set_plan"]["run"].as_i64().unwrap() > 0);

    // Kept: shown and listed, newest first.
    let id = first["set_plan"]["id"].as_i64().unwrap().to_string();
    let shown = ok(&sandbox, &["--json", "set-plan", "show", &id]);
    assert_eq!(shown["plan"], first["set_plan"]["plan"]);
    let listed = ok(&sandbox, &["--json", "set-plan", "list"]);
    let listed: Vec<i64> = listed["set_plans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        listed,
        [
            again["set_plan"]["id"].as_i64().unwrap(),
            first["set_plan"]["id"].as_i64().unwrap()
        ]
    );
    let text = sandbox.cli(&["set-plan", "show", &id]);
    assert!(
        text.stdout.contains("Warm room") && text.stdout.contains(&ids[3]),
        "{}",
        text.stdout
    );

    // The watcher heard of each plan; nothing about the Library.
    let mut heard = Vec::new();
    let until = Instant::now() + Duration::from_secs(5);
    while heard.len() < 2 && Instant::now() < until {
        if let Ok(Some(event)) = watcher.next_event(Duration::from_millis(200)) {
            assert_ne!(event.event, "library_changed", "{event:?}");
            if event.event == "set_plan_built" {
                heard.push(event.data["id"].clone());
            }
        }
    }
    assert_eq!(
        heard,
        [
            first["set_plan"]["id"].clone(),
            again["set_plan"]["id"].clone()
        ]
    );

    drop(watcher);
    stop_service(&sandbox);
    assert!(
        library_state(&sandbox) == before,
        "building a Set plan changed the Library"
    );
}

#[test]
fn an_unsatisfiable_recipe_names_the_criterion_in_the_way_and_keeps_nothing() {
    let sandbox = Sandbox::new();
    let ids = keyed(&sandbox);
    let file = recipe(
        &sandbox,
        "same-key.toml",
        "version = 1\ntracks = 4\n[tempo]\nmax_change = 2\n[key]\nmax_moves = 0\n",
    );
    let pool: Vec<&str> = ids.iter().map(String::as_str).collect();
    let refused = build(&sandbox, &file, &pool);
    assert!(!refused.status.success(), "no four tracks share a key");
    let answer = refused.json();
    assert_eq!(answer["built"], false);
    assert_eq!(answer["set_plan"], Value::Null);
    assert_eq!(answer["unsatisfiable"]["criterion"], "key_moves");
    assert_eq!(
        answer["unsatisfiable"]["in_the_way"],
        serde_json::json!(["key_moves"])
    );
    let message = answer["unsatisfiable"]["message"].as_str().unwrap();
    assert!(message.contains("key.max_moves 0"), "{message}");
    let text = sandbox.cli(&[
        "set-plan",
        "build",
        file.to_str().unwrap(),
        &ids[0],
        &ids[1],
        &ids[2],
        &ids[3],
        &ids[4],
    ]);
    assert!(!text.status.success());
    assert!(
        text.stderr.contains("key.max_moves") || text.stdout.contains("key.max_moves"),
        "{}{}",
        text.stdout,
        text.stderr
    );

    // Too few tracks for the Recipe is the pool's own criterion.
    let small = build(&sandbox, &file, &pool[..2]).json();
    assert_eq!(small["unsatisfiable"]["criterion"], "tracks");

    let listed = ok(&sandbox, &["--json", "set-plan", "list"]);
    assert_eq!(
        listed["set_plans"],
        serde_json::json!([]),
        "nothing was kept"
    );
}

#[test]
fn a_recipe_with_mistakes_is_reported_by_line_and_never_built() {
    let sandbox = Sandbox::new();
    let ids = keyed(&sandbox);
    let file = recipe(
        &sandbox,
        "bad.toml",
        "version = 1\ntracks = 4\n\n[key]\nmax_moves = 12\n\n[tempo]\nmax_chnge = 3\n",
    );
    let checked = sandbox.cli(&["--json", "set-plan", "check", file.to_str().unwrap()]);
    assert!(!checked.status.success());
    let checked = checked.json();
    assert_eq!(checked["valid"], false);
    let problems = checked["problems"].as_array().unwrap();
    assert_eq!(problems[0]["line"], 8, "{problems:?}");
    assert!(problems[0]["message"]
        .as_str()
        .unwrap()
        .contains("max_chnge"));

    let refused = build(&sandbox, &file, &[&ids[0], &ids[1]]);
    assert!(!refused.status.success());
    let error = refused.error_json()["error"].clone();
    assert_eq!(error["code"], "recipe_invalid", "{error}");
    assert!(
        error["message"].as_str().unwrap().contains("line 8"),
        "{error}"
    );

    // A Situation the Tag schema does not have is a mistake too.
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(
        sandbox.sidecar_dir().join("tag-schema.toml"),
        "version = 1\n\n[dimensions.situation]\nterms = [\n  { id = \"opener\", name = \"Opener\" },\n  { id = \"closer\", name = \"Closer\" },\n]\n",
    )
    .unwrap();
    let file = recipe(
        &sandbox,
        "peak.toml",
        "version = 1\ntracks = 3\n[situations]\nplace = [\n  { situation = \"opener\", from = 0, to = 0 },\n  { situation = \"peak_time\", from = 0.5, to = 1 },\n]\n",
    );
    let checked = sandbox
        .cli(&["--json", "set-plan", "check", file.to_str().unwrap()])
        .json();
    assert_eq!(checked["valid"], false);
    let problem = &checked["problems"][0];
    assert_eq!(problem["line"], 6, "{checked}");
    assert!(
        problem["message"].as_str().unwrap().contains("peak_time"),
        "{checked}"
    );
    assert!(
        problem["message"].as_str().unwrap().contains("closer"),
        "names what there is: {checked}"
    );

    let good = recipe(&sandbox, "fine.toml", JOURNEY);
    let checked = ok(
        &sandbox,
        &["--json", "set-plan", "check", good.to_str().unwrap()],
    );
    assert_eq!(checked["valid"], true);
    let listed = ok(&sandbox, &["--json", "set-plan", "list"]);
    assert_eq!(listed["set_plans"], serde_json::json!([]));
}

/// A mono 16-bit 44.1 kHz WAV file of `seconds` of audio: a quiet low hum,
/// or a loud one with a hit every eighth of a second.
fn write_wav(path: &Path, seconds: u32, loud: bool) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let rate = 44_100_u32;
    let frames = rate * seconds;
    let mut data = Vec::with_capacity(frames as usize * 2);
    for n in 0..frames {
        let t = f64::from(n) / f64::from(rate);
        let hum = (t * 110.0 * std::f64::consts::TAU).sin();
        let sample = if loud {
            let hit = if n % (rate / 8) < 400 { 0.6 } else { 0.0 };
            0.35 * hum + hit * (t * 3_000.0 * std::f64::consts::TAU).sin()
        } else {
            0.003 * hum
        };
        data.extend_from_slice(&((sample * 32_767.0) as i16).to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out).unwrap();
}

#[test]
fn tracks_with_no_intensity_or_key_are_named_and_the_plan_follows_the_shape() {
    let sandbox = Sandbox::new();
    let quiet = sandbox.path().join("audio/quiet.wav");
    let loud = sandbox.path().join("audio/loud.wav");
    write_wav(&quiet, 6, false);
    write_wav(&loud, 6, true);
    let (mut quiet_id, mut loud_id) = (String::new(), String::new());
    sandbox.edit(|writer| {
        quiet_id = writer.import_file(&quiet).unwrap();
        loud_id = writer.import_file(&loud).unwrap();
    });
    let unmeasured = rbl_db::fixture::track_id(0);
    ok(
        &sandbox,
        &["--json", "intensity", "measure", &loud_id, &quiet_id],
    );

    let file = recipe(
        &sandbox,
        "rise.toml",
        "version = 1\nname = \"Rise\"\ntracks = 2\n[intensity]\nshape = [{ at = 0, intensity = 0.1 }, { at = 1, intensity = 0.9 }]\n",
    );
    let built = build(&sandbox, &file, &[&loud_id, &unmeasured, &quiet_id]);
    assert!(built.status.success(), "{}", built.stderr);
    let plan = &built.json()["set_plan"]["plan"];
    let order: Vec<&str> = plan["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        [quiet_id.as_str(), loud_id.as_str()],
        "quiet first, rising"
    );
    assert_eq!(plan["left_out"].as_array().unwrap().len(), 1, "{plan}");
    let left = &plan["left_out"][0];
    assert_eq!(left["id"], unmeasured.as_str());
    assert_eq!(left["lacks"], serde_json::json!(["intensity"]));
    assert!(
        left["message"].as_str().unwrap().contains("never measured"),
        "{left}"
    );
    assert_eq!(plan["pool"], 3);
    assert_eq!(plan["usable"], 2);

    // Under a key limit, tracks with no key (these imports have none) are
    // left out by name, and the plan cannot be built from what remains.
    let keyed = recipe(
        &sandbox,
        "keyed.toml",
        "version = 1\ntracks = 2\n[key]\nmax_moves = 1\n",
    );
    let refused = build(&sandbox, &keyed, &[&loud_id, &quiet_id]).json();
    assert_eq!(refused["unsatisfiable"]["criterion"], "tracks");
    let lacks: Vec<&Value> = refused["unsatisfiable"]["left_out"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| &l["lacks"])
        .collect();
    assert_eq!(
        lacks,
        [&serde_json::json!(["key"]), &serde_json::json!(["key"])]
    );
}

#[test]
fn a_pool_from_a_playlist_and_an_unknown_track_is_refused() {
    let sandbox = Sandbox::new();
    let ids = keyed(&sandbox);
    let playlist = sandbox.create_playlist("Candidates", &[0, 1, 2, 3, 4, 5, 6, 7]);
    let file = recipe(&sandbox, "loose.toml", "version = 1\ntracks = 3\n[key]\n");
    let built = ok(
        &sandbox,
        &[
            "--json",
            "set-plan",
            "build",
            file.to_str().unwrap(),
            "--playlist",
            &playlist,
        ],
    );
    assert_eq!(built["set_plan"]["pool"].as_array().unwrap().len(), 8);
    assert_eq!(
        built["set_plan"]["plan"]["tracks"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let refused = build(&sandbox, &file, &[&ids[0], "99999"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "track_not_found");
    let none = sandbox.cli(&["--json", "set-plan", "build", file.to_str().unwrap()]);
    assert!(!none.status.success(), "a pool must be named");
}
