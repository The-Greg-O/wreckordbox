//! One durable operation intent, one lazy Run and bounded confirmed batches.
//! No Library reads, audio reads, fingerprints, or per-track transactions.
use super::{Keeper, Target, CURRENT};
use rbl_library::bulk::{Kind, Recorder, Step};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use wreckordbox_sidecar::ledger::{Action, Intent, NewEntry, Store};
use wreckordbox_sidecar::{RunId, Source, SourceKind};

const RECORDS: usize = 128;
const BYTES: usize = 64 * 1024;
const REASON: &str = "Previous analysis artifacts and imported native rows were not retained; Unwind cannot reverse this operation";

thread_local! { static ACTIVE: RefCell<Option<Rc<Active>>> = const { RefCell::new(None) }; }
pub(super) struct Active {
    owner: usize,
    session: Arc<Session>,
    began: Cell<bool>,
    wrote: Cell<bool>,
    bulk_write: Cell<bool>,
    admitted: bool,
}
pub(super) fn active(keeper: &Keeper) -> Option<Rc<Active>> {
    ACTIVE.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|a| a.owner == Arc::as_ptr(&keeper.stems) as usize)
            .cloned()
    })
}
impl Active {
    pub(super) fn set_bulk(&self, bulk: bool) {
        self.bulk_write.set(bulk);
    }
    pub(super) fn is_bulk(&self) -> bool {
        self.bulk_write.get()
    }
    pub(super) fn begin(&self) -> Result<(), String> {
        if !self.admitted {
            return Err("That Ledger operation has ended".into());
        }
        self.session.begin()?;
        self.began.set(true);
        Ok(())
    }
    pub(super) fn written(&self, wrote: bool) {
        self.wrote.set(self.wrote.get() || wrote);
    }
}
struct Handle(Arc<Session>);
struct Scope {
    active: Rc<Active>,
    previous: Option<Rc<Active>>,
    target: Value,
    completed: bool,
}
impl Recorder for Handle {
    fn enter(&self, target: Value) -> Box<dyn Step> {
        let admitted = {
            let mut state = self.0.state();
            let admitted = !state.ended && !state.finish_requested;
            if admitted {
                state.active += 1;
            }
            admitted
        };
        let active = Rc::new(Active {
            owner: self.0.owner,
            session: self.0.clone(),
            began: Cell::new(false),
            wrote: Cell::new(false),
            bulk_write: Cell::new(false),
            admitted,
        });
        let previous = ACTIVE.with(|slot| slot.replace(Some(active.clone())));
        Box::new(Scope {
            active,
            previous,
            target,
            completed: false,
        })
    }
    fn finish(&self) -> Option<String> {
        self.0.finish()
    }
}
impl Step for Scope {
    fn complete(&mut self, records: Vec<Value>) {
        self.completed = true;
        let partial = records.iter().any(|r| r["status"] == "partial_or_failed");
        if self.active.wrote.get() || (partial && self.active.began.get()) {
            for record in records {
                self.active.session.record(record);
            }
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.completed && self.active.began.get() {
            self.active
                .session
                .record(json!({"target":self.target,"status":"unconfirmed"}));
        }
        ACTIVE.with(|slot| slot.replace(self.previous.take()));
        let finish = {
            let mut state = self.active.session.state();
            if self.active.admitted {
                state.active -= 1;
            }
            state.active == 0 && state.finish_requested
        };
        if finish {
            self.active.session.finish();
        }
    }
}
#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "admission, completion and degraded recording are independent states"
)]
struct State {
    began: bool,
    intent: Option<i64>,
    run: Option<RunId>,
    pending: Vec<Value>,
    bytes: usize,
    warning: Option<String>,
    lost: bool,
    ended: bool,
    failed_flush: bool,
    active: usize,
    finish_requested: bool,
}
struct Session {
    owner: usize,
    stems: Arc<crate::stems::Stems>,
    source: Source,
    kind: &'static str,
    description: &'static str,
    state: Mutex<State>,
    faults: super::Faults,
    begun: AtomicBool,
}
pub(super) fn make(keeper: &Keeper, kind: Kind) -> Arc<dyn Recorder> {
    let (kind, description, source) = match kind {
        Kind::Analysis => (
            "analysis",
            "Analyze tracks",
            Source::new(SourceKind::Tool, "wreckordbox-analysis"),
        ),
        Kind::Import => (
            "import",
            "Import files",
            CURRENT.with(
                |current| match current.borrow().as_ref().map(|s| &s.target) {
                    Some(Target::Joined { source, .. } | Target::Own { source, .. }) => {
                        source.clone()
                    }
                    _ => Source::new(SourceKind::Dj, "dj"),
                },
            ),
        ),
    };
    Arc::new(Handle(Arc::new(Session {
        owner: Arc::as_ptr(&keeper.stems) as usize,
        stems: keeper.stems.clone(),
        source,
        kind,
        description,
        state: Mutex::new(State::default()),
        faults: super::Faults::from_env(),
        begun: AtomicBool::new(false),
    })))
}
impl Session {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn command(&self) -> Value {
        json!({"name":"bulk.write","args":{"kind":self.kind,"not_reversible_reason":REASON,"recovery":"Operation did not finish; confirmed batches are recorded separately. Further changes may be unrecorded."}})
    }
    fn warning(&self, state: &mut State, error: impl std::fmt::Display) {
        let message = format!(
            "{}: Ledger recording is incomplete; any accepted Library changes remain: {error}",
            self.description
        );
        tracing::error!(%message);
        state.warning = Some(message);
    }
    fn begin(&self) -> Result<(), String> {
        // After admission, a row writer never waits on the buffer's Sidecar
        // flush while holding the Library edit gate.
        if self.begun.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut state = self.state();
        if state.ended {
            return Err("That Ledger operation has ended".into());
        }
        if state.began {
            return Ok(());
        }
        state.began = true;
        let command = self.command();
        let result = self
            .stems
            .sidecar()
            .map_err(|e| e.message)
            .and_then(|sidecar| {
                sidecar
                    .begin_intent(
                        &Intent {
                            run: None,
                            at: super::now(),
                            source: &self.source,
                            description: self.description,
                            command: Some(&command),
                        },
                        &[],
                    )
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(id) => state.intent = Some(id),
            Err(error) => self.warning(&mut state, error),
        }
        self.begun.store(true, Ordering::Release);
        Ok(())
    }
    fn record(&self, mut record: Value) {
        let mut state = self.state();
        if state.ended {
            return;
        }
        let mut bytes = record.to_string().len();
        if bytes > BYTES {
            record = json!({"status":"metadata_omitted","reason":"Result metadata exceeded the 64 KiB recording limit"});
            bytes = record.to_string().len();
        }
        if state.pending.len() >= RECORDS || state.bytes + bytes > BYTES {
            self.flush(&mut state);
        }
        // A persistent recording failure must neither grow memory without bound
        // nor stop the remaining Library writes. The operation intent stays open.
        if state.pending.len() >= RECORDS || state.bytes + bytes > BYTES {
            state.lost = true;
            return;
        }
        state.bytes += bytes;
        state.pending.push(record);
        if state.pending.len() == RECORDS {
            self.flush(&mut state);
        }
    }
    fn flush(&self, state: &mut State) {
        if state.pending.is_empty() || state.failed_flush {
            return;
        }
        let result = (|| -> Result<(), String> {
            let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
            let run = if let Some(run) = state.run {
                run
            } else {
                let run = sidecar
                    .begin_bulk_run(state.intent, &self.source, self.description)
                    .map_err(|e| e.to_string())?;
                state.run = Some(run);
                run
            };
            if self.faults.refuse_append() {
                return Err("injected bulk append failure".into());
            }
            let payload = json!({"records":state.pending,"not_reversible_reason":REASON});
            let command = self.command();
            sidecar
                .append(&NewEntry {
                    id: None,
                    run,
                    store: Store::Library,
                    source: &self.source,
                    at: None,
                    action: Action::Write,
                    kind: self.kind,
                    label: &format!(
                        "{}: {} results. Cannot reverse: {REASON}",
                        self.description,
                        state.pending.len()
                    ),
                    command: Some(&command),
                    reverses: None,
                    part_of: None,
                    reversible: false,
                    format: 1,
                    payload: Some(&payload),
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                state.pending.clear();
                state.bytes = 0;
            }
            Err(error) => {
                state.failed_flush = true;
                self.warning(state, error);
            }
        }
    }
    fn finish(&self) -> Option<String> {
        let mut state = self.state();
        if state.ended {
            return state.warning.clone();
        }
        state.finish_requested = true;
        if state.active != 0 {
            return state.warning.clone();
        }
        if !state.began {
            state.ended = true;
            return None;
        }
        state.failed_flush = false;
        self.flush(&mut state);
        if state.pending.is_empty() && !state.lost {
            state.warning = None;
        }
        match self.stems.sidecar_if_made() {
            Ok(Some(sidecar)) => {
                if state.pending.is_empty() && !state.lost {
                    if let Some(intent) = state.intent {
                        if let Err(error) = sidecar.settle_intents(&[intent]) {
                            self.warning(&mut state, error);
                        }
                    }
                }
                if let Some(run) = state.run {
                    if let Err(error) = sidecar.finish_run(
                        run,
                        if state.warning.is_some() {
                            "recording incomplete"
                        } else {
                            "completed"
                        },
                    ) {
                        self.warning(&mut state, error);
                    }
                }
            }
            Ok(None) => self.warning(&mut state, "the Sidecar is no longer available"),
            Err(error) => self.warning(&mut state, error.message),
        }
        state.ended = true;
        state.warning.clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use rbl_library::ledger::Ledger;

    #[test]
    fn service_caller_source_is_retained_by_import_but_analysis_is_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        let location = rbl_db::fixture::build(
            dir.path(),
            rbl_db::fixture::Shape {
                tracks: 1,
                ..rbl_db::fixture::Shape::default()
            },
        )
        .unwrap();
        let (events, _received) = std::sync::mpsc::channel();
        let stems =
            crate::stems::Stems::from_env(&location.master_db, &dir.path().join("state"), events);
        let keeper = Keeper::new(stems.clone(), location);
        let request: crate::protocol::Request = serde_json::from_value(json!({"id":1,"command":"tracks.rate","args":{"id":"10000","stars":3},"run":{"source":{"kind":"agent","name":"fixture-importer"}}})).unwrap();
        for kind in [Kind::Import, Kind::Analysis] {
            let (result, _) = keeper.attributed(true, &request, || {
                let recorder = make(&keeper, kind);
                let mut step = recorder.enter(json!({"track":"fixture"}));
                keeper.begin_unkept().unwrap();
                keeper.written(true);
                step.complete(vec![json!({"track":"fixture"})]);
                drop(step);
                assert_eq!(recorder.finish(), None);
                Ok(())
            });
            result.unwrap();
        }
        let sidecar = stems.sidecar().unwrap();
        let (runs, _) = sidecar.runs(10, 0).unwrap();
        assert_eq!(runs.len(), 2);
        for run in runs {
            let (entries, _) = sidecar.run_entries(run.run.id, 10, 0).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].source,
                if entries[0].kind == "import" {
                    Source::new(SourceKind::Agent, "fixture-importer")
                } else {
                    Source::new(SourceKind::Tool, "wreckordbox-analysis")
                }
            );
        }
    }
}
