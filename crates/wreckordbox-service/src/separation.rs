//! The separation engine, run as a child process and spoken to over its
//! JSON-lines protocol (`separation-engine/docs/PROTOCOL.md`): one `hello`,
//! one `separate`, `progress` lines, then `done` or `error`, then
//! `shutdown`. The engine is a Python worker; nothing of it is linked in.
//!
//! One engine process is started per separation and ends with it, so the
//! models it loads (several GB of memory) are not held between tracks.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::protocol::ServiceError;
use crate::stems::codes;

/// The engine protocol this speaks; the engine's `hello` must say the same.
const ENGINE_PROTOCOL: u64 = 1;

/// What one separation produced, as the engine's `done` reports it.
#[derive(Debug, Clone)]
pub(crate) struct Separation {
    /// The engine's name for each stem and the file it wrote.
    pub stems: Vec<(String, PathBuf)>,
    /// The source's length in sample frames, as the engine decoded it.
    pub source_samples: u64,
    pub sample_rate: u32,
    /// The model, or pipeline of models, that did the work.
    pub model: String,
    pub engine_version: String,
}

/// Stops a separation from another thread: the engine is killed and
/// [`separate`] answers [`codes::SEPARATION_CANCELLED`].
#[derive(Default)]
pub(crate) struct Cancel {
    cancelled: AtomicBool,
    engine: Mutex<Option<Child>>,
}

impl Cancel {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(engine) = self.engine().as_mut() {
            let _ = engine.kill();
        }
    }

    fn engine(&self) -> std::sync::MutexGuard<'_, Option<Child>> {
        self.engine.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Deserialize)]
struct Done {
    stems: Vec<DoneStem>,
    source_samples: u64,
    sample_rate: u32,
    #[serde(default)]
    model: String,
}

#[derive(Deserialize)]
struct DoneStem {
    name: String,
    path: PathBuf,
}

/// Separates `input` into four stems written to `workspace`, by running
/// `command` as the engine. `progress` is told each stage and how far along
/// the engine thinks it is, 0 to 1. Blocks until the engine is done, which
/// for the real one is several times the track's length.
pub(crate) fn separate(
    command: &[OsString],
    input: &Path,
    workspace: &Path,
    cancel: &Cancel,
    progress: &mut dyn FnMut(&str, f64),
) -> Result<Separation, ServiceError> {
    let Some((program, args)) = command.split_first() else {
        return Err(ServiceError::new(
            codes::SEPARATION_ENGINE_MISSING,
            "no separation engine is configured",
        ));
    };
    let mut engine = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // The engine logs there, and only there; it goes where the
        // service's own does.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            ServiceError::new(
                codes::SEPARATION_ENGINE_MISSING,
                format!(
                    "could not run the separation engine ({}): {e}. Set {} to the command that runs it.",
                    Path::new(program).display(),
                    crate::stems::SEPARATION_COMMAND_ENV
                ),
            )
        })?;
    let (stdin, stdout) = (engine.stdin.take(), engine.stdout.take());
    *cancel.engine() = Some(engine);
    if cancel.cancelled.load(Ordering::Acquire) {
        // Stopped in the moment before there was an engine to stop.
        cancel.cancel();
    }
    let outcome = match (stdin, stdout) {
        (Some(mut stdin), Some(stdout)) => {
            let outcome = converse(&mut stdin, stdout, input, workspace, progress);
            let _ = stdin.write_all(b"{\"cmd\":\"shutdown\"}\n");
            outcome
        }
        _ => Err(failed(&"it has no standard input or output")),
    };
    if let Some(mut engine) = cancel.engine().take() {
        if outcome.is_err() {
            // It may be mid-separation still; it must not work on unheard.
            let _ = engine.kill();
        }
        let _ = engine.wait();
    }
    if cancel.cancelled.load(Ordering::Acquire) {
        return Err(ServiceError::new(
            codes::SEPARATION_CANCELLED,
            "the separation was stopped because the Library service is stopping",
        ));
    }
    outcome
}

fn converse(
    stdin: &mut impl Write,
    stdout: impl std::io::Read,
    input: &Path,
    workspace: &Path,
    progress: &mut dyn FnMut(&str, f64),
) -> Result<Separation, ServiceError> {
    const JOB: &str = "1";
    let mut lines = BufReader::new(stdout).lines();
    let mut next = || -> Result<Value, ServiceError> {
        loop {
            let line = lines
                .next()
                .ok_or_else(|| failed(&"it stopped before it had finished"))?
                .map_err(|e| failed(&e))?;
            if line.trim().is_empty() {
                continue;
            }
            return serde_json::from_str(&line)
                .map_err(|e| failed(&format_args!("it said something that is not JSON: {e}")));
        }
    };

    let hello = next()?;
    if hello["type"] != "hello" || hello["protocol"].as_u64() != Some(ENGINE_PROTOCOL) {
        return Err(failed(&format_args!(
            "it does not speak engine protocol {ENGINE_PROTOCOL}: {hello}"
        )));
    }
    let engine_version = hello["engine_version"].as_str().unwrap_or("").to_owned();

    let request = json!({
        "cmd": "separate",
        "id": JOB,
        "input": input,
        "mode": "four_stem",
        "workspace": workspace,
        "quality": "standard",
    });
    writeln!(stdin, "{request}")
        .and_then(|()| stdin.flush())
        .map_err(|e| failed(&e))?;

    loop {
        let event = next()?;
        match event["type"].as_str() {
            Some("progress") => progress(
                event["stage"].as_str().unwrap_or(""),
                event["fraction"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0),
            ),
            Some("done") => {
                let done: Done = serde_json::from_value(event["result"].clone())
                    .map_err(|e| failed(&format_args!("its result does not read: {e}")))?;
                return Ok(Separation {
                    stems: done
                        .stems
                        .into_iter()
                        .map(|stem| (stem.name, stem.path))
                        .collect(),
                    source_samples: done.source_samples,
                    sample_rate: done.sample_rate,
                    model: done.model,
                    engine_version,
                });
            }
            Some("error") => {
                return Err(failed(&format_args!(
                    "{} ({})",
                    event["message"].as_str().unwrap_or("no reason given"),
                    event["category"].as_str().unwrap_or("unknown")
                )));
            }
            _ => {}
        }
    }
}

fn failed(why: &dyn std::fmt::Display) -> ServiceError {
    ServiceError::new(
        codes::SEPARATION_FAILED,
        format!("the separation engine failed: {why}"),
    )
}
