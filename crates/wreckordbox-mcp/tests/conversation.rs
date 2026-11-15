//! A real client conversation with `wreckordbox-mcp` over stdio, against a
//! Library service on a fixture Library: the server is a thin client of
//! that service, as the `wreckordbox` command is.
//!
//! The service is started on demand by the `wreckordbox` binary beside this
//! one, which must be built first (`cargo build -p wreckordbox`, through the
//! machine's queue); tests never start a nested Cargo build.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_db::LibraryLocation;
use serde_json::{json, Value};
use wreckordbox_service::{Client, Config};

/// A fixture Library in a temporary folder, its state and runtime folders.
struct Sandbox {
    dir: tempfile::TempDir,
    location: LibraryLocation,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let location = fixture::build(&library, Shape::default()).unwrap();
        fixture::write_options_json(
            &dir.path().join("options.json"),
            location.master_db.to_str().unwrap(),
            &location.passphrase,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let sandbox = Self { dir, location };
        sandbox.protect(false);
        sandbox
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn protect(&self, on: bool) {
        wreckordbox_service::protection::store(&self.path("state"), on).unwrap();
    }

    fn config(&self) -> Config {
        Config {
            runtime_dir: self.path("run"),
            idle_exit: None,
            recovery_dir: self.path("state"),
        }
    }

    /// Another client of the same service, as the window or the CLI is.
    fn client(&self) -> Client {
        Client::connect(&self.config().socket_path(&self.location)).unwrap()
    }

    /// The MCP server, with the environment a test's `wreckordbox` gets.
    fn server(&self) -> Server {
        let program = PathBuf::from(env!("CARGO_BIN_EXE_wreckordbox-mcp"));
        let cli = program.with_file_name("wreckordbox");
        assert!(
            cli.exists(),
            "{} is not built: run `cargo build -p wreckordbox` first",
            cli.display()
        );
        let mut child = Command::new(program)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.path("home"))
            .env("RBXPORT_TEST", "1")
            .env("RBXPORT_STATE_DIR", self.path("state"))
            .env("RBXPORT_OPTIONS", self.path("options.json"))
            .env("RBXPORT_TEST_FIXTURE", self.path("library"))
            .env("WRECKORDBOX_RUNTIME_DIR", self.path("run"))
            .env("WRECKORDBOX_SIDECAR_DIR", self.path("sidecar"))
            .env("WRECKORDBOX_IDLE_EXIT_SECS", "5")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Server {
            child,
            stdin,
            lines,
            next_id: 1,
            socket: self.config().socket_path(&self.location),
        }
    }
}

/// One MCP client conversation.
struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: i64,
    socket: PathBuf,
}

impl Server {
    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and answers the response with its id.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| panic!("no answer to {method}"));
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
            assert_eq!(message["jsonrpc"], "2.0", "{message}");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// `initialize` and the `initialized` notification.
    fn start(&mut self) -> Value {
        let answer = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test-agent", "version": "1"}
            }),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        answer
    }

    /// `tools/call`, answering its result (a tool error is a result too).
    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let answer = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert!(answer.get("error").is_none(), "{tool}: {answer}");
        answer["result"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Ok(mut client) = Client::connect(&self.socket) {
            let _ = client.stop();
        }
    }
}

/// The JSON a successful tool call answered.
fn structured(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"], "{result}");
    &result["structuredContent"]
}

/// The service error a failed tool call answered.
fn refused(result: &Value) -> Value {
    assert_eq!(result["isError"], true, "{result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    text["error"].clone()
}

fn rating_of(client: &mut Client, id: &str) -> u64 {
    let track: Value = client.call("tracks.get", json!({"id": id})).unwrap();
    track["rating"].as_u64().unwrap()
}

fn tool_names(server: &mut Server) -> Vec<String> {
    let listed = server.request("tools/list", json!({}));
    listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn initialize_names_the_server_and_offers_tools() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    let answer = server.start();
    let result = &answer["result"];
    assert_eq!(result["serverInfo"]["name"], "wreckordbox-mcp", "{answer}");
    assert_eq!(result["protocolVersion"], "2025-06-18", "{answer}");
    assert!(result["capabilities"]["tools"].is_object(), "{answer}");
}

#[test]
fn tools_are_named_and_marked_as_reads_or_changes_and_leave_out_the_djs_decisions() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let listed = server.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(tools.len() > 20, "{listed}");
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let read_only = tool["annotations"]["readOnlyHint"].as_bool().unwrap();
        let description = tool["description"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        if name.starts_with("read_") {
            assert!(read_only, "{tool}");
            assert!(description.starts_with("Read only."), "{tool}");
        } else {
            assert!(name.starts_with("change_"), "{tool}");
            assert!(!read_only, "{tool}");
            assert!(description.starts_with("Changes "), "{tool}");
        }
    }
    let names = tool_names(&mut server);
    for wanted in [
        "read_tracks_query",
        "read_stick_load_preview",
        "change_track_rating",
        "change_run_start",
        "change_record_facts",
    ] {
        assert!(
            names.iter().any(|n| n == wanted),
            "{wanted} not in {names:?}"
        );
    }
    // No tool makes one of the DJ's own decisions, or turns protection off.
    for forbidden in [
        "unlock",
        "acknowledge",
        "accept",
        "reject",
        "reopen",
        "discard",
        "restore",
        "protection_off",
    ] {
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("change_") && n.contains(forbidden)),
            "a DJ-only command is a tool ({forbidden}): {names:?}"
        );
    }
}

#[test]
fn a_read_tool_answers_what_the_service_answers() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let result = server.call(
        "read_tracks_query",
        json!({"bpm": "128..128.05", "limit": 2}),
    );
    let page = structured(&result);
    assert_eq!(page["total"], 6, "{page}");
    assert_eq!(page["tracks"][0]["id"], "10000", "{page}");
    assert_eq!(page["next_offset"], 2, "{page}");
}

#[test]
fn a_change_is_seen_by_another_client_and_recorded_in_the_agents_run() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    // The first call starts the service; then a second client listens.
    structured(&server.call("read_status", json!({})));
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    assert_eq!(rating_of(&mut watcher, "10001"), 0);

    let run = structured(&server.call(
        "change_run_start",
        json!({"description": "Rate the warm-up selection"}),
    ))
    .clone();
    assert_eq!(
        run["source"],
        json!({"kind": "agent", "name": "test-agent"}),
        "{run}"
    );
    let run_id = run["id"].as_i64().unwrap();
    let rated = structured(&server.call(
        "change_track_rating",
        json!({"id": "10001", "rating": 4, "run": run_id}),
    ))
    .clone();
    assert_eq!(rated["previous_rating"], 0, "{rated}");
    assert_eq!(rated["run"]["id"], run_id, "{rated}");

    let event = watcher
        .next_event(Duration::from_secs(10))
        .unwrap()
        .expect("the second client is told the Library changed");
    assert_eq!(event.event, "library_changed", "{event:?}");
    assert_eq!(rating_of(&mut watcher, "10001"), 4);

    // In the Ledger, by the agent, with the value before and after.
    let shown: Value = watcher.call("runs.show", json!({"id": run_id})).unwrap();
    let values = &shown["entries"][0]["values"][0];
    assert_eq!(values["before"], 0, "{shown}");
    assert_eq!(values["after"], 4, "{shown}");
    assert_eq!(
        shown["entries"][0]["source"]["name"], "test-agent",
        "{shown}"
    );
}

#[test]
fn a_change_without_a_run_is_the_agents_own_run_never_the_djs() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let rated =
        structured(&server.call("change_track_rating", json!({"id": "10002", "rating": 2})))
            .clone();
    assert_eq!(
        rated["run"]["source"],
        json!({"kind": "agent", "name": "test-agent"}),
        "{rated}"
    );

    let error = refused(&server.call(
        "change_track_rating",
        json!({"id": "10002", "rating": 5, "source": "dj"}),
    ));
    assert_eq!(error["code"], "invalid_source", "{error}");
    assert_eq!(rating_of(&mut sandbox.client(), "10002"), 2);
}

#[test]
fn a_change_under_library_protection_is_refused_and_changes_nothing() {
    let sandbox = Sandbox::new();
    sandbox.protect(true);
    let mut server = sandbox.server();
    server.start();
    let error = refused(&server.call("change_track_rating", json!({"id": "10003", "rating": 5})));
    assert_eq!(error["code"], "library_protected", "{error}");
    let error = refused(&server.call(
        "change_mytag_assign",
        json!({"id": fixture::MY_TAG_PEAK, "tracks": ["10003"]}),
    ));
    assert_eq!(error["code"], "library_protected", "{error}");
    let mut client = sandbox.client();
    assert_eq!(rating_of(&mut client, "10003"), 0);
    let tags: Value = client
        .call("mytags.of_track", json!({"id": "10003"}))
        .unwrap();
    assert_eq!(tags["my_tags"], json!([]), "{tags}");
    // A read is never refused.
    structured(&server.call("read_protection", json!({})));
}

#[test]
fn a_sidecar_change_takes_the_agent_as_its_source() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let recorded = structured(&server.call(
        "change_record_facts",
        json!({"facts": [{"track": "10004", "kind": "mood", "value": ["Dark"]}], "confidence": 0.7}),
    ))
    .clone();
    assert_eq!(
        recorded["facts"][0]["source"]["name"], "test-agent",
        "{recorded}"
    );
    let shown = structured(&server.call("read_track_facts", json!({"id": "10004"}))).clone();
    assert_eq!(shown["facts"][0]["value"], json!(["Dark"]), "{shown}");
}

#[test]
fn an_unknown_tool_or_method_is_a_protocol_error() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let answer = server.request(
        "tools/call",
        json!({"name": "stick_unlock", "arguments": {}}),
    );
    assert_eq!(answer["error"]["code"], -32602, "{answer}");
    let answer = server.request("resources/list", json!({}));
    assert_eq!(answer["error"]["code"], -32601, "{answer}");
}

#[test]
fn every_tool_is_named_in_the_agent_guide() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let guide = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/agents/guide.md"),
    )
    .unwrap();
    let missing: Vec<String> = tool_names(&mut server)
        .into_iter()
        .filter(|name| !guide.contains(&format!("`{name}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "docs/agents/guide.md does not name: {missing:?}"
    );
}

#[test]
fn a_nested_argument_the_schema_does_not_advertise_is_refused_so_a_fact_is_never_the_djs() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    // The reviewer's call: a DJ source smuggled inside one Fact.
    let error = refused(&server.call(
        "change_record_facts",
        json!({"facts":[{"track":"10001","kind":"energy","value":3,"source":{"kind":"dj","name":"dj"}}]}),
    ));
    assert_eq!(error["code"], "bad_request", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("facts[0].source"),
        "{error}"
    );
    // A wrong type deep inside is refused the same way, before the service.
    let error = refused(&server.call(
        "read_tracks_query",
        json!({"my_tag_picks": [{"category": "Situation", "tags": "Peak", "any": false}]}),
    ));
    assert_eq!(error["code"], "bad_request", "{error}");
    let shown = structured(&server.call("read_track_facts", json!({"id": "10001"}))).clone();
    assert_eq!(shown["facts"], json!([]), "{shown}");
}

#[test]
fn a_change_never_joins_the_djs_run_so_it_never_inherits_the_djs_attribution() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    structured(&server.call("read_status", json!({})));
    let mut dj = sandbox.client();
    let run: Value = dj
        .call(
            "runs.start",
            json!({"source": {"kind": "dj"}, "description": "The DJ's own work"}),
        )
        .unwrap();
    let run = run["id"].as_i64().unwrap();
    for (tool, arguments) in [
        (
            "change_record_facts",
            json!({"facts": [{"track": "10001", "kind": "energy", "value": 3}], "run": run}),
        ),
        (
            "change_track_rating",
            json!({"id": "10001", "rating": 5, "run": run}),
        ),
        (
            "change_track_rating",
            json!({"id": "10001", "rating": 5, "run": run, "source": "agent:test-agent"}),
        ),
    ] {
        let error = refused(&server.call(tool, arguments));
        assert_eq!(error["code"], "invalid_source", "{tool}: {error}");
    }
    let mut client = sandbox.client();
    assert_eq!(rating_of(&mut client, "10001"), 0);
    let facts: Value = client
        .call("facts.of_track", json!({"id": "10001"}))
        .unwrap();
    assert_eq!(facts["facts"], json!([]), "{facts}");
}

#[test]
fn a_decision_asked_without_a_run_or_source_is_the_agents() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let asked = structured(&server.call(
        "change_decision_create",
        json!({"question_type": "yes_no", "question": "Is it a dub?", "track": "10001"}),
    ))
    .clone();
    // The asker is the source of the Run the Decision was asked in.
    let run = asked["run"].as_i64().expect("asked in a Run");
    let shown = structured(&server.call("read_run", json!({"id": run}))).clone();
    assert_eq!(
        shown["run"]["source"],
        json!({"kind": "agent", "name": "test-agent"}),
        "{shown}"
    );
}

#[test]
fn a_playlist_is_made_with_tracks_filled_and_arranged_through_the_tools_as_the_agents() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let folder = structured(&server.call("change_playlist_create_folder", json!({"name": "Gigs"})))
        ["playlist"]
        .as_str()
        .unwrap()
        .to_owned();
    let made = structured(&server.call(
        "change_playlist_create",
        json!({"name": "Warm-up", "parent": folder, "tracks": ["10003", "10001"]}),
    ))
    .clone();
    assert_eq!(
        made["run"]["source"],
        json!({"kind": "agent", "name": "test-agent"}),
        "{made}"
    );
    let playlist = made["playlist"].as_str().unwrap().to_owned();
    structured(&server.call(
        "change_playlist_add_tracks",
        json!({"playlist": playlist, "tracks": ["10002"], "at": 1}),
    ));
    structured(&server.call(
        "change_playlist_rename",
        json!({"playlist": playlist, "name": "Opening"}),
    ));
    let moved =
        structured(&server.call("change_playlist_move", json!({"playlist": playlist}))).clone();
    assert_eq!(moved["parent"], "root", "{moved}");
    assert_eq!(moved["name"], "Opening", "{moved}");

    let listed =
        structured(&server.call("read_playlist_tracks", json!({"id": playlist, "limit": 10})))
            .clone();
    let ids: Vec<&str> = listed["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["10002", "10003", "10001"], "{listed}");

    // Under Library Protection, nothing more is made.
    structured(&server.call("change_protection_on", json!({})));
    let error = refused(&server.call("change_playlist_create", json!({"name": "Late"})));
    assert_eq!(error["code"], "library_protected", "{error}");
}

#[test]
fn a_set_plan_is_built_through_tools_as_the_agents_own_and_read_back() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let checked = structured(&server.call(
        "read_set_plan_check",
        json!({"recipe": "version = 1\ntracks = 3\n[tempo]\nmax_chnge = 2\n"}),
    ))
    .clone();
    assert_eq!(checked["valid"], false, "{checked}");
    assert_eq!(checked["problems"][0]["line"], 4, "{checked}");

    let recipe = "version = 1\nname = \"Agent's warm-up\"\ntracks = 3\n[tempo]\nmax_change = 2\n";
    let built = structured(&server.call(
        "change_set_plan_build",
        json!({"recipe": recipe, "ids": ["10000", "10001", "10002", "10003"]}),
    ))
    .clone();
    assert_eq!(built["built"], true, "{built}");
    let plan = &built["set_plan"];
    assert_eq!(
        plan["plan"]["tracks"].as_array().unwrap().len(),
        3,
        "{built}"
    );
    let run = structured(&server.call("read_run", json!({"id": plan["run"]}))).clone();
    assert_eq!(run["run"]["source"]["kind"], "agent", "{run}");
    assert_eq!(run["run"]["source"]["name"], "test-agent", "{run}");

    let listed = structured(&server.call("read_set_plans", json!({}))).clone();
    assert_eq!(listed["set_plans"][0]["id"], plan["id"], "{listed}");
    let shown = structured(&server.call("read_set_plan", json!({"id": plan["id"]}))).clone();
    assert_eq!(shown["plan"], plan["plan"]);

    // The DJ is never the source of an agent's plan.
    let refused = refused(&server.call(
        "change_set_plan_build",
        json!({"recipe": recipe, "ids": ["10000", "10001", "10002"], "source": "dj"}),
    ));
    assert_eq!(refused["code"], "invalid_source", "{refused}");
}

#[test]
fn a_lookup_is_described_checked_as_a_read_and_recorded_only_by_the_change_tool() {
    let sandbox = Sandbox::new();
    let mut server = sandbox.server();
    server.start();
    let described =
        structured(&server.call("read_lookup_description", json!({"id": "10004"}))).clone();
    assert_eq!(described["track"], "10004", "{described}");
    let candidate = json!({
        // The fixture's tracks may credit no artist; a candidate always does.
        "artist": match described["artist"].as_str() {
            Some("") | None => "Someone Else",
            Some(artist) => artist,
        },
        "title": described["title"],
        "duration": described["duration_seconds"],
    });
    let asked = json!({
        "id": "10004", "candidate": candidate,
        "address": "https://shop.example.test/1", "date": "2026-11-10",
    });

    let dry = structured(&server.call("read_lookup_check", asked.clone())).clone();
    assert_eq!(dry["dry_run"], true, "{dry}");
    assert_eq!(dry["decision"], Value::Null, "{dry}");
    let none = structured(&server.call(
        "read_decisions",
        json!({"topic": "lookup_match", "state": "all"}),
    ))
    .clone();
    assert_eq!(none["total"], 0, "a read records nothing: {none}");

    // The check's attribution is the service's: no source can be given.
    let mut spoofed = asked.clone();
    spoofed["source"] = json!("agent:someone-else");
    let error = refused(&server.call("change_lookup_check", spoofed));
    assert_eq!(error["code"], "bad_request", "{error}");

    let checked = structured(&server.call("change_lookup_check", asked)).clone();
    assert_eq!(checked["verdict"], dry["verdict"], "{checked}");
    assert_eq!(
        checked["decision"]["answers"][0]["decider"],
        json!({"kind": "tool", "name": "lookup-check"}),
        "{checked}"
    );
    let recorded = structured(&server.call(
        "read_decisions",
        json!({"topic": "lookup_match", "state": "all"}),
    ))
    .clone();
    assert_eq!(recorded["total"], 1, "{recorded}");
}
