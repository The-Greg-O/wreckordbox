//! Background Fact producers hold the shared Fact coordination through
//! their commit (wbx-j8k).
//!
//! Review's accept checks a track's Facts and writes under
//! [`Stems::fact_writes`](crate::stems::Stems); a producer that observed the
//! audio ([`Stems::before_record`](crate::stems::Stems)) and then recorded
//! after letting go could land its Fact in the middle of an accept. Every
//! producer that records later than its command's dispatch (a separation's
//! thread, Intensity's workers, a Tag schema
//! conversion) therefore takes [`Stems::hold_facts`] before its last
//! observation and keeps it until the Fact is committed.
//!
//! Lock order, as Review's (wbx-27): Fact coordination, then the Library,
//! then the edit gate. The hold is never taken while the Library is held,
//! and a producer holding it takes neither the Library nor the edit gate,
//! so it cannot wait for an accept, a Projection apply or a Stick load that
//! waits for it. Long work (separating, decoding) runs without it.

use crate::stems::Stems;

/// The shared Fact coordination, held until dropped. Re-entrant on its
/// thread, so a holder may still pass `before_record`.
pub(crate) struct FactHold<'a> {
    _guard: parking_lot::ReentrantMutexGuard<'a, ()>,
}

impl Stems {
    /// Takes the shared Fact coordination: waits for an accept in
    /// progress. Take it before any Library lookup the commit depends on,
    /// never while holding the Library.
    pub(crate) fn hold_facts(&self) -> FactHold<'_> {
        FactHold {
            _guard: crate::review::exclude(self),
        }
    }
}

/// A producer holding the coordination, between its observation and its
/// commit: the test hold `producer-<name>` of Review's gate folder (test
/// mode only).
pub(crate) fn about_to_commit(producer: &str) {
    crate::review::hold(&format!("producer-{producer}"));
}
