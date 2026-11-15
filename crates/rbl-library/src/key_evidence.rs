//! Optional writer-only evidence publication, prepared without the edit gate.
use rbl_db::key_tag::prepared::Prepared;

/// A verified key writer's observation, never a query or a recovery operation.
pub trait KeyEvidence: Send + Sync {
    /// Called after this exact prepared file was published. Failure leaves the
    /// accepted key edit intact and its evidence stale; the caller shows it.
    fn record_key_evidence(&self, published: &Prepared) -> Result<(), String>;
}
