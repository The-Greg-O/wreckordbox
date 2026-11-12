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
}

/// Implemented by the service's Ledger. Enter is thread-local and its guard
/// must not cross an await. Completion reports only values already in hand.
pub trait Recorder: Send + Sync {
    fn enter(&self, target: Value) -> Box<dyn Step>;
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
        let mut step = self
            .0
            .as_ref()
            .map(|recorder| recorder.enter(target.clone()));
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
    pub fn finish(&self) -> Option<String> {
        self.0.as_ref().and_then(|recorder| recorder.finish())
    }
}
