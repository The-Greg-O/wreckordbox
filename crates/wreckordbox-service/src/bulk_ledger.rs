//! One operation Run, acceptance-time order, bounded recording transactions.
//! Reservation/admission never takes the flush mutex under the Library gate.
use super::{
    bulk_spill::{self as spill, Manifest, Record},
    Keeper, Target, CURRENT,
};
use rbl_library::bulk::{GenerateRecords, Kind, Recorder, Step};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
};
use wreckordbox_sidecar::{bulk::OrderLease, ledger::Intent, RunId, Sidecar, Source, SourceKind};

static NEXT: AtomicU64 = AtomicU64::new(0);
const RECORDS: usize = 128;
const BYTES: usize = 64 * 1024;
pub(super) const REASON:&str="Previous analysis artifacts and imported native rows were not retained; Unwind cannot reverse this operation";
thread_local! {static ACTIVE:RefCell<Option<Rc<Active>>>=const {RefCell::new(None)};}
pub(super) struct Active {
    owner: usize,
    session: Arc<Session>,
    uncertain: Cell<bool>,
    asserted: Cell<bool>,
    bulk_write: Cell<bool>,
    admitted: bool,
    capacity: usize,
    stamps: RefCell<Vec<Stamp>>,
}
struct Stamp {
    at: i64,
    ids: Vec<i64>,
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
    pub(super) fn imports(&self) -> bool {
        self.session.kind == "import"
    }
    /// Kept replacement history uses the same operation owner/Run as the
    /// imported rows. Its native intent and Mutant preparation remain intact.
    pub(super) fn import_target(&self) -> (Result<RunId, String>, Source, String) {
        let mut admission = self.session.admission();
        let run = if let Some(run) = admission.run {
            Ok(run)
        } else {
            self.session
                .stems
                .sidecar()
                .map_err(|e| e.message)
                .and_then(|sidecar| {
                    sidecar
                        .begin_bulk_run(
                            admission.intent,
                            &self.session.source,
                            self.session.description,
                        )
                        .map_err(|e| e.to_string())
                })
                .inspect(|run| admission.run = Some(*run))
        };
        (
            run,
            self.session.source.clone(),
            self.session.description.into(),
        )
    }
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
        self.uncertain.set(true);
        Ok(())
    }
    pub(super) fn written(&self, wrote: bool) {
        self.uncertain.set(false);
        let asserted = self.asserted.replace(false);
        if wrote || asserted {
            self.stamps
                .borrow_mut()
                .push(self.session.accept(self.capacity));
        }
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
        self.enter_many(target, 1)
    }
    fn enter_many(&self, target: Value, capacity: usize) -> Box<dyn Step> {
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
            uncertain: Cell::new(false),
            asserted: Cell::new(false),
            bulk_write: Cell::new(false),
            admitted,
            capacity,
            stamps: RefCell::new(Vec::new()),
        });
        let previous = ACTIVE.with(|slot| slot.replace(Some(active.clone())));
        Box::new(Scope {
            active,
            previous,
            target,
            completed: false,
        })
    }
    fn assert_player_fields(&self) {
        if matches!(self.0.kind, "link" | "round_trip") {
            ACTIVE.with(|slot| {
                if let Some(active) = slot.borrow().as_ref() {
                    if Arc::ptr_eq(&active.session, &self.0) && active.is_bulk() {
                        // written() reserves order under the native writer gate,
                        // including when the fields were already at these values.
                        active.asserted.set(true);
                    }
                }
            });
        }
    }
    fn idle(&self) -> bool {
        self.0.state().active == 0
    }
    fn finish(&self) -> Option<String> {
        self.0.finish()
    }
    fn finish_wait(&self) -> Option<String> {
        {
            let mut state = self.0.state();
            state.finish_requested = true;
            while state.active != 0 {
                state = self
                    .0
                    .left
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        self.0.finish()
    }
}
impl Step for Scope {
    fn complete_generated(&mut self, generate: &mut GenerateRecords<'_>) {
        self.completed = true;
        let mut stamps = self.active.stamps.take();
        let Some(last) = stamps.pop() else { return };
        for stamp in stamps {
            self.active.session.record(Record { id: stamp.ids.first().copied(), at: stamp.at,
                value: json!({"target":self.target,"status":"accepted_native_step","not_reversible_reason":"Intermediate native write; its values were not retained"}) });
        }
        let mut index = 0;
        generate(&mut |value| {
            self.active.session.record(Record {
                id: last.ids.get(index).copied(),
                at: last.at,
                value,
            });
            index += 1;
        });
        if index == 0 {
            self.active.session.record(Record { id: last.ids.first().copied(), at: last.at,
                value: json!({"target":self.target,"status":"accepted_native_step","not_reversible_reason":"The writer returned no result values"}) });
        }
    }
    fn complete(&mut self, mut records: Vec<Value>) {
        self.completed = true;
        let mut stamps = self.active.stamps.take();
        if stamps.is_empty() {
            if self.active.uncertain.get()
                && records.iter().any(|r| r["status"] == "partial_or_failed")
            {
                stamps.push(self.active.session.accept(records.len().max(1)));
            } else {
                return;
            }
        }
        // A bounded file slice accepts several individual publications. Pair
        // each result with its own acceptance stamp, after releasing its gates.
        if stamps.len() > 1 && stamps.len() == records.len() {
            for (stamp, value) in stamps.into_iter().zip(records) {
                self.active.session.record(Record {
                    id: stamp.ids.first().copied(),
                    at: stamp.at,
                    value,
                });
            }
            return;
        }
        let Some(last) = stamps.pop() else {
            return;
        };
        if records.is_empty() {
            records.push(json!({"track":self.target.get("track"),"target":self.target,"status":"accepted_native_step","not_reversible_reason":"The writer returned no result values"}));
        }
        for stamp in stamps {
            self.active.session.record(Record{id:stamp.ids.first().copied(),at:stamp.at,value:json!({"track":self.target.get("track"),"target":self.target,"status":"accepted_native_step","not_reversible_reason":"Intermediate native write; its values were not retained"})});
        }
        for (index, value) in records.into_iter().enumerate() {
            self.active.session.record(Record {
                id: last.ids.get(index).copied(),
                at: last.at,
                value,
            });
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.completed
            && (self.active.uncertain.get() || !self.active.stamps.borrow().is_empty())
        {
            self.complete(vec![json!({"track":self.target.get("track"),"target":self.target,"status":"partial_or_failed","error":"write unwound before its result"})]);
        }
        ACTIVE.with(|slot| slot.replace(self.previous.take()));
        let finish = {
            let mut state = self.active.session.state();
            if self.active.admitted {
                state.active -= 1;
            }
            state.active == 0 && state.finish_requested
        };
        self.active.session.left.notify_all();
        if finish {
            self.active.session.finish();
        }
    }
}
#[derive(Default)]
struct Admission {
    sidecar: Option<Arc<Sidecar>>,
    lease: OrderLease,
    order_failed: bool,
    intent: Option<i64>,
    run: Option<RunId>,
    warning: Option<String>,
}
#[derive(Default)]
struct State {
    pending: Vec<Record>,
    bytes: usize,
    warning: Option<String>,
    ended: bool,
    active: usize,
    finish_requested: bool,
    manifest: Option<Manifest>,
    spill_index: u64,
    // Only used if BOTH the Sidecar and spill filesystem fail: keep compact
    // named outcomes, not unbounded result payloads, and expose them in warning.
    unspilled: Vec<Record>,
}
struct Session {
    owner: usize,
    keeper: Weak<Keeper>,
    stems: Arc<crate::stems::Stems>,
    source: Source,
    kind: &'static str,
    description: &'static str,
    admission: Mutex<Admission>,
    state: Mutex<State>,
    left: Condvar,
    faults: super::Faults,
    begun: AtomicBool,
    spill: std::path::PathBuf,
}
pub(super) fn make(keeper: &Keeper, kind: Kind) -> Arc<dyn Recorder> {
    let (kind, description, source) = match kind {
        Kind::Analysis => (
            "analysis",
            "Analyze tracks",
            Source::new(SourceKind::Tool, "wreckordbox-analysis"),
        ),
        Kind::Link => (
            "link",
            "Player edits over LINK",
            Source::new(SourceKind::Tool, "link"),
        ),
        Kind::RoundTrip => (
            "round_trip",
            "Player edits from Round-trip",
            Source::new(SourceKind::Tool, "round-trip"),
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
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let spill = keeper
        .stems
        .bulk_recovery_dir()
        .join("ledger-bulk")
        .join(format!("{}-{}-{serial}", super::now(), std::process::id()));
    Arc::new(Handle(Arc::new(Session {
        owner: Arc::as_ptr(&keeper.stems) as usize,
        keeper: keeper.weak.clone(),
        stems: keeper.stems.clone(),
        source,
        kind,
        description,
        admission: Mutex::new(Admission::default()),
        state: Mutex::new(State::default()),
        left: Condvar::new(),
        faults: super::Faults::from_env(),
        begun: AtomicBool::new(false),
        spill,
    })))
}
impl Session {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn admission(&self) -> std::sync::MutexGuard<'_, Admission> {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn command(&self) -> Value {
        json!({"name":"bulk.write","args":{"kind":self.kind,"not_reversible_reason":REASON,"recovery":"Operation did not finish; confirmed or spilled results are recorded separately. Further writes may be unconfirmed."}})
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
        let keeper = self.keeper.upgrade().ok_or("wreckordbox has stopped")?;
        if keeper.stopping.load(Ordering::Acquire) {
            return Err("wreckordbox is stopping".into());
        }
        if self.begun.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut admission = self.admission();
        if self.begun.load(Ordering::Acquire) {
            return Ok(());
        }
        keeper.admit_bulk()?;
        let command = self.command();
        match self.stems.sidecar() {
            Ok(sidecar) => {
                match sidecar.begin_intent(
                    &Intent {
                        run: None,
                        at: super::now(),
                        source: &self.source,
                        description: self.description,
                        command: Some(&command),
                    },
                    &[],
                ) {
                    Ok(id) => admission.intent = Some(id),
                    Err(error) => admission.warning = Some(error.to_string()),
                }
                admission.sidecar = Some(sidecar);
            }
            Err(error) => admission.warning = Some(error.message),
        }
        self.begun.store(true, Ordering::Release);
        Ok(())
    }
    fn accept(&self, capacity: usize) -> Stamp {
        let at = super::now();
        let mut admission = self.admission();
        let ids = if let Some(sidecar) = admission.sidecar.clone() {
            match sidecar.reserve_bulk_ids(&mut admission.lease, capacity) {
                Ok(ids) => ids,
                Err(error) => {
                    admission.warning = Some(error.to_string());
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if ids.len() != capacity {
            admission.order_failed = true;
        }
        Stamp { at, ids }
    }
    fn manifest(&self, state: &mut State) -> Manifest {
        let (intent, run) = {
            let admission = self.admission();
            (admission.intent, admission.run)
        };
        state
            .manifest
            .get_or_insert_with(|| Manifest {
                intent,
                run,
                source: self.source.clone(),
                description: self.description.into(),
                kind: self.kind.into(),
                complete: false,
            })
            .clone()
    }
    fn record(&self, mut record: Record) {
        let mut state = self.state();
        if state.ended {
            return;
        }
        if !record.value.is_object() {
            record.value = json!({"result":record.value});
        }
        if record.id.is_none() {
            record.value["order_unavailable"] = json!("The Sidecar could not reserve an id at acceptance; this entry retains its write time but its id was assigned later");
        }
        let mut bytes = record.value.to_string().len();
        if bytes > BYTES {
            record.value = json!({"track":record.value.get("track"),"target":record.value.get("target"),"status":"metadata_omitted","reason":"Result metadata exceeded 64 KiB"});
            bytes = record.value.to_string().len();
        }
        if !state.pending.is_empty()
            && (state.pending.len() >= RECORDS || state.bytes + bytes > BYTES)
        {
            self.flush(&mut state);
        }
        state.bytes += bytes;
        state.pending.push(record);
        if state.pending.len() == RECORDS {
            self.flush(&mut state);
        }
    }
    fn before_append(&self) -> Result<(), String> {
        if self.faults.refuse_append() {
            Err("injected bulk append failure".into())
        } else {
            Ok(())
        }
    }
    fn flush(&self, state: &mut State) {
        if state.pending.is_empty() {
            return;
        }
        let mut manifest = self.manifest(state);
        let result = (|| {
            let _access = self.stems.bulk_access()?;
            let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
            if manifest.run.is_none() {
                let mut admission = self.admission();
                let run = match admission.run {
                    Some(run) => run,
                    None => sidecar
                        .begin_bulk_run(manifest.intent, &manifest.source, &manifest.description)
                        .map_err(|e| e.to_string())?,
                };
                admission.run = Some(run);
                manifest.run = Some(run);
            }
            state.manifest = Some(manifest.clone());
            spill::bind_run(&self.spill, &manifest)?;
            self.before_append()?;
            manifest.append(&sidecar, &state.pending)
        })();
        if let Err(error) = result {
            self.warning(state, error);
            state.spill_index += 1;
            if let Err(error) =
                spill::save(&self.spill, &manifest, state.spill_index, &state.pending)
            {
                self.warning(
                    state,
                    format!("spill failed ({error}); named results retained in memory"),
                );
                for record in &state.pending {
                    state.unspilled.push(Record{id:record.id,at:record.at,value:json!({"track":record.value.get("track"),"target":record.value.get("target"),"status":"accepted_but_details_not_recorded"})});
                }
            }
        }
        state.pending.clear();
        state.bytes = 0;
    }
    fn finish(&self) -> Option<String> {
        let mut state = self.state();
        if state.ended {
            return state.warning.clone();
        }
        state.finish_requested = true;
        if state.active != 0 {
            return Some(
                "Ledger operation is still finishing; its accepted writes are pending recording"
                    .into(),
            );
        }
        if !self.begun.load(Ordering::Acquire) {
            state.ended = true;
            return None;
        }
        self.flush(&mut state);
        let unspilled = std::mem::take(&mut state.unspilled);
        for chunk in unspilled.chunks(RECORDS) {
            state.pending = chunk.to_vec();
            self.flush(&mut state);
        }
        let mut manifest = self.manifest(&mut state);
        manifest.complete = true;
        let result = (|| {
            spill::mark_complete(&self.spill)?;
            let _access = self.stems.bulk_access()?;
            let sidecar = self.stems.sidecar().map_err(|e| e.message)?;
            spill::replay(&self.spill, &sidecar, || self.before_append())?;
            if !state.unspilled.is_empty() {
                return Err(format!(
                    "named writes could not be stored: {}",
                    serde_json::to_string(&state.unspilled).unwrap_or_default()
                ));
            }
            if let Some(intent) = manifest.intent {
                sidecar
                    .settle_intents(&[intent])
                    .map_err(|e| e.to_string())?;
            }
            if let Some(run) = manifest.run {
                sidecar
                    .finish_run(run, "completed")
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                state.warning = if self.admission().order_failed {
                    Some("All accepted results were recorded, but some Ledger ids could not be reserved at write time; see order_unavailable on those results.".into())
                } else {
                    None
                }
            }
            Err(error) => {
                self.warning(&mut state, error);
                if self.spill.exists() {
                    if let Some(keeper) = self.keeper.upgrade() {
                        keeper.state().bulk_waiting.push(self.spill.clone());
                    }
                }
            }
        }
        // Kept replacement history uses the ordinary durable spill (and its
        // transactional Mutant Follow), but is still part of this operation.
        // A successful bulk append must not conceal a waiting history entry.
        if self.kind == "import"
            && self.keeper.upgrade().is_some_and(|keeper| {
                keeper.state().unwritten.iter().any(|entry| {
                    entry.run == manifest.run.map(|run| run.0) && entry.source == self.source
                })
            })
        {
            self.warning(
                &mut state,
                "kept replacement history is waiting to be recorded",
            );
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
        for kind in [Kind::Import, Kind::Analysis, Kind::Link, Kind::RoundTrip] {
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
        assert_eq!(runs.len(), 4);
        for run in runs {
            let (entries, _) = sidecar.run_entries(run.run.id, 10, 0).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].source,
                if entries[0].kind == "import" {
                    Source::new(SourceKind::Agent, "fixture-importer")
                } else {
                    Source::new(
                        SourceKind::Tool,
                        match entries[0].kind.as_str() {
                            "link" => "link",
                            "round_trip" => "round-trip",
                            _ => "wreckordbox-analysis",
                        },
                    )
                }
            );
        }
    }
    #[test]
    fn stopping_refuses_an_already_open_bulk_scope() {
        let dir = tempfile::tempdir().unwrap();
        let location =
            rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
        let (events, _) = std::sync::mpsc::channel();
        let stems =
            crate::stems::Stems::from_env(&location.master_db, &dir.path().join("state"), events);
        let keeper = Keeper::new(stems, location);
        let recorder = make(&keeper, Kind::Analysis);
        let _scope = recorder.enter(json!({"track":"10000"}));
        keeper.begin_unkept().unwrap();
        keeper.shut();
        assert!(
            keeper.begin_unkept().is_err(),
            "stopping must refuse another write in the same operation"
        );
    }
}
