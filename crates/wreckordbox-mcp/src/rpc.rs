//! JSON-RPC 2.0 over newline-delimited stdio, the subset MCP needs, and
//! the call of a tool through the Library service.

use std::io::{BufRead, Write};

use serde_json::{json, Map, Value};
use wreckordbox_service::ledger::RunArg;
use wreckordbox_service::{Client, Config, Launcher, ServiceError};

use crate::tools::{catalogue, Tool, Who};

/// The protocol revisions this server speaks, newest first. A client asking
/// for one of them gets it; any other gets the newest.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the server tells the client about itself when it starts.
const INSTRUCTIONS: &str = "wreckordbox: a DJ's rekordbox-format Library. Tools starting read_ change \
nothing; tools starting change_ change the Library or its Sidecar through the same service the window \
and the wreckordbox command use, honour Library Protection and are recorded in the Ledger. Group a \
piece of work in a Run (change_run_start, pass `run`, change_run_finish). Never claim to be the DJ. \
Unlocking a Stick, clearing a Projection barrier, accepting, rejecting or reopening a waiting Fact, \
turning Library Protection off and restoring a backup are the DJ's own decisions: they are not tools. \
Read docs/agents/guide.md in the wreckordbox repository for every command and recipe.";

/// Serves one client: reads requests from `input` until it ends, writing
/// each response to `output` on a line of its own.
pub fn serve(input: impl BufRead, mut output: impl Write) {
    let mut session = Session {
        tools: catalogue(),
        agent: "mcp".to_owned(),
    };
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(answer) = session.answer(&line) else {
            continue;
        };
        if writeln!(output, "{answer}")
            .and_then(|()| output.flush())
            .is_err()
        {
            break;
        }
    }
}

struct Session {
    tools: Vec<Tool>,
    /// The name changes are attributed to, as `agent:<name>`: the client's
    /// own name from `initialize`.
    agent: String,
}

impl Session {
    /// The response to one line, `None` for a notification.
    fn answer(&mut self, line: &str) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(&Value::Null, PARSE_ERROR, "not JSON"));
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to us, or a batch: neither is used.
            return message
                .get("id")
                .map(|id| error(id, INVALID_REQUEST, "not a request"));
        };
        // A notification (no id) is never answered.
        let id = message.get("id")?.clone();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let outcome = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(Tool::listed).collect::<Vec<_>>()
            })),
            "tools/call" => self.call(&params),
            _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
        };
        Some(match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error(&id, code, &message),
        })
    }

    fn initialize(&mut self, params: &Value) -> Value {
        if let Some(name) = params["clientInfo"]["name"].as_str() {
            let name: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            if !name.is_empty() {
                self.agent = name;
            }
        }
        let asked = params["protocolVersion"].as_str().unwrap_or_default();
        let version = PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == asked)
            .unwrap_or(&PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "wreckordbox-mcp", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })
    }

    fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"].as_str().unwrap_or_default();
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| (INVALID_PARAMS, format!("no tool {name}")))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(arguments)) => arguments.clone(),
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_owned())),
        };
        // A refusal is the tool's result, for the agent to read and say.
        Ok(match self.run(tool, arguments) {
            Ok(result) => {
                let result = match result {
                    Value::Object(_) => result,
                    other => json!({ "result": other }),
                };
                json!({
                    "content": [{"type": "text", "text": result.to_string()}],
                    "structuredContent": result,
                    "isError": false,
                })
            }
            Err(failure) => json!({
                "content": [{"type": "text", "text": json!({"error": failure}).to_string()}],
                "isError": true,
            }),
        })
    }

    /// Sends the tool's command to the service and answers its result, with
    /// the Run its changes are in as `run`, as the CLI prints it.
    fn run(&self, tool: &Tool, mut arguments: Map<String, Value>) -> Result<Value, ServiceError> {
        // A source that is not an agent, tool or web lookup (the DJ) is
        // refused as a source, before its form is checked.
        if let Some(text) = arguments.get("source") {
            text.as_str()
                .map_or_else(|| Err(invalid_source()), source)?;
        }
        // Every level, so nothing the schema does not advertise (a `source`
        // inside one Fact) reaches the service.
        crate::schema::check(&tool.input_schema(), &Value::Object(arguments.clone()), "")
            .map_err(|why| bad_request(format!("{}: {why}", tool.name)))?;
        let run = arguments.remove("run");
        // `runs.start` takes its description as its own argument.
        let description = if tool.who == Who::Starts {
            None
        } else {
            arguments.remove("description")
        };
        let source = match arguments.remove("source") {
            Some(Value::String(text)) => Some(source(&text)?),
            Some(_) => return Err(invalid_source()),
            None => None,
        };
        // Without a Run or a source, a change is this agent's own work.
        let by_us = || json!({"kind": "agent", "name": self.agent});
        let source = source.or_else(|| run.is_none().then(by_us));
        let mut args = tool.fixed.clone();
        args.extend(arguments);
        let mut client = connect()?;
        if let Some(id) = run.as_ref().and_then(Value::as_i64) {
            refuse_the_djs_run(&mut client, id)?;
        }
        match tool.who {
            Who::Nobody => {}
            Who::Request | Who::OwnRun => {
                let named = json!({"id": run, "source": source, "description": description});
                let named: RunArg = serde_json::from_value(strip_nulls(named))
                    .map_err(|e| bad_request(format!("run, source or description: {e}")))?;
                client.in_run(Some(named));
            }
            Who::InArgs => {
                insert(&mut args, "run", run);
                insert(&mut args, "source", source);
                insert(&mut args, "description", description);
            }
            Who::Decider => {
                insert(&mut args, "run", run);
                insert(&mut args, "decider", source.or_else(|| Some(by_us())));
                insert(&mut args, "description", description);
            }
            Who::RunAndSource => {
                insert(&mut args, "run", run);
                insert(&mut args, "source", source);
            }
            Who::Starts => insert(&mut args, "source", source.or_else(|| Some(by_us()))),
        }
        let mut result: Value = client.call(tool.command, Value::Object(args))?;
        if let (Some(run), Value::Object(object)) = (client.last_run(), &mut result) {
            object.entry("run").or_insert_with(|| json!(run));
        }
        Ok(result)
    }
}

/// An agent's change never joins the DJ's Run: it would be recorded with
/// the DJ's attribution, or beside the DJ's own work. Asked before the
/// change, on the same connection.
fn refuse_the_djs_run(client: &mut Client, id: i64) -> Result<(), ServiceError> {
    let shown: Value = client.call("runs.show", json!({"id": id, "limit": 1}))?;
    if shown["run"]["source"]["kind"] == "dj" {
        return Err(ServiceError::new(
            "invalid_source",
            format!(
                "Run {id} is the DJ's: an agent's change never joins the DJ's Run. \
                 Start a Run of your own with change_run_start"
            ),
        ));
    }
    Ok(())
}

fn insert(args: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        args.insert(key.to_owned(), value);
    }
}

fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            Value::Object(object.into_iter().filter(|(_, v)| !v.is_null()).collect())
        }
        other => other,
    }
}

/// `agent:NAME`, `tool:NAME` or `web:ADDRESS` as the service takes it. The
/// DJ is never a source here: an agent's change is never the DJ's.
fn source(text: &str) -> Result<Value, ServiceError> {
    match text.split_once(':') {
        Some((kind @ ("agent" | "tool" | "web"), name)) if !name.trim().is_empty() => {
            Ok(json!({"kind": kind, "name": name}))
        }
        _ => Err(invalid_source()),
    }
}

fn invalid_source() -> ServiceError {
    ServiceError::new(
        "invalid_source",
        "a source is `agent:NAME`, `tool:NAME` or `web:ADDRESS`; a tool's change is never the DJ's",
    )
}

fn bad_request(message: String) -> ServiceError {
    ServiceError::new(wreckordbox_service::codes::BAD_REQUEST, message)
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// A client of the service for this machine's Library, as the
/// `wreckordbox` command connects: started on demand by that command, and
/// waiting for a window that is still reading the Library.
fn connect() -> Result<Client, ServiceError> {
    let config = Config::from_env()?;
    let library = wreckordbox_service::locate_library()?;
    let launcher = Launcher {
        program: cli_program(),
        args: vec!["serve".into(), "--spawned".into()],
    };
    let mut client = Client::connect_or_start(&config, &library, &launcher)?;
    client.wait_while_loading(wreckordbox_service::loading_wait_from_env());
    Ok(client)
}

/// The `wreckordbox` command that starts a service: `WRECKORDBOX_CLI`, else
/// the one beside this program, else the one on the path.
fn cli_program() -> std::path::PathBuf {
    if let Some(program) = std::env::var_os("WRECKORDBOX_CLI") {
        return program.into();
    }
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("wreckordbox"))
        .filter(|beside| beside.exists())
        .unwrap_or_else(|| "wreckordbox".into())
}
