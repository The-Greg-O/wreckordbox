//! Operation-sized recording for analysis and imports. It never owns a Library
//! writer or a lock: callers still use the core's guarded edit path.
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Results of the analysis tool, regardless of who requested the batch.
    Analysis,
    /// Local file imports, attributed to the caller (or the DJ in the window).
    /// Player metadata and Round-trip need their own player source, not this kind.
    Import,
    /// A player message received over LINK; never attributed to its caller.
    Link,
    /// One Round-trip operation bringing player changes back from a Stick.
    RoundTrip,
}

/// Implemented by the service's Ledger. Enter is thread-local and its guard
/// must not cross an await. Completion reports only values already in hand.
pub trait Recorder: Send + Sync {
    fn enter(&self, target: Value) -> Box<dyn Step>;
    fn enter_many(&self, target: Value, _capacity: usize) -> Box<dyn Step> {
        self.enter(target)
    }
    /// A player explicitly sent scalar fields already at their received values.
    /// Call inside the guarded writer; completion supplies the field payloads.
    fn assert_player_fields(&self) {}
    fn idle(&self) -> bool {
        true
    }
    fn finish_wait(&self) -> Option<String> {
        self.finish()
    }
    /// A warning about recording, separate from the accepted writes.
    fn finish(&self) -> Option<String>;
}
pub trait Step {
    fn complete(&mut self, records: Vec<Value>);
}

#[derive(Clone, Default)]
pub struct Operation(Option<Arc<dyn Recorder>>);
impl Operation {
    pub fn new(core: &crate::LibraryCore, kind: Kind) -> Self {
        let ledger = core.history().lock().ledger();
        Self(ledger.and_then(|ledger| ledger.bulk(kind)))
    }
    /// Runs one track or import slice, including all its auxiliary writes.
    /// On error a begun write is retained as partial, never as a success.
    pub fn run<T, E: std::fmt::Display>(
        &self,
        target: Value,
        work: impl FnOnce() -> Result<T, E>,
        values: impl FnOnce(&T) -> Vec<Value>,
    ) -> Result<T, E> {
        self.run_many(target, 1, work, values)
    }
    /// Maximum results of one accepted native write (e.g. an import slice).
    /// Reserves order slots in bounded blocks, before another write can intervene.
    pub fn run_many<T, E: std::fmt::Display>(
        &self,
        target: Value,
        capacity: usize,
        work: impl FnOnce() -> Result<T, E>,
        values: impl FnOnce(&T) -> Vec<Value>,
    ) -> Result<T, E> {
        let mut step = self
            .0
            .as_ref()
            .map(|recorder| recorder.enter_many(target.clone(), capacity.max(1)));
        let result = work();
        if let Some(step) = &mut step {
            step.complete(match &result {
                Ok(value) => values(value),
                Err(error) => {
                    let mut record =
                        serde_json::json!({"status":"partial_or_failed","error":error.to_string()});
                    record["target"] = target;
                    vec![record]
                }
            });
        }
        result
    }
    /// Preserve a player's same-value choice without rewriting its native row.
    /// Must be called inside this operation's guarded native write.
    pub fn assert_player_fields(&self) {
        if let Some(recorder) = &self.0 {
            recorder.assert_player_fields();
        }
    }
    pub fn idle(&self) -> bool {
        self.0.as_ref().is_none_or(|recorder| recorder.idle())
    }
    pub fn finish_wait(&self) -> Option<String> {
        self.0.as_ref().and_then(|recorder| recorder.finish_wait())
    }
    pub fn finish(&self) -> Option<String> {
        self.0.as_ref().and_then(|recorder| recorder.finish())
    }
}
