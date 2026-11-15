//! A Library hosted on a Stick is read-only for everything the Sidecar
//! records (wbx-45.14, generalised by wbx-8yh: the DJ's decision of
//! 2026-11-14, option 3).
//!
//! "Hosted on a Stick": the Library's folder or the Sidecar's folder is on
//! a Stick. Refused there, locked or unlocked, with one named refusal
//! (`sidecar_on_stick`) that says why and what the DJ can do: Projection
//! applies, Review accept, reject and reopen, Fact recording by anyone
//! (Decisions included), Mutants, the Tag schema's conversion, stem
//! separation and Intensity. Reading (every query, dry runs, Review's
//! waiting list) and ordinary Library edits stay allowed.
//!
//! Enforced in one place every Sidecar write passes: the write admission
//! installed on the service's Sidecar handle ([`admission`]), asked by the
//! handle at the moment a write would be kept, so a handle obtained
//! earlier cannot write later. The only Sidecar writes it lets through on
//! a Stick are an ordinary Library edit's own record (its Ledger entry, its
//! Run, its Assertion), made inside [`ledger_scope`] and only while the
//! Stick is unlocked at that moment. [`refuse_records`] answers the same
//! refusal before a recording command starts, so a command that would
//! also change the Library (a Projection, a Review accept) changes nothing.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::protocol::{codes, ServiceError};

/// Every service command that records in the Sidecar: refused for a
/// Library hosted on a Stick before it starts.
pub(crate) const RECORDS: &[&str] = &[
    "decisions.answer",
    "decisions.create",
    "decisions.decide",
    "facts.record",
    "intensity.measure",
    "mutants.clear",
    "mutants.create",
    "mutants.edit",
    "mutants.remove",
    "projection.acknowledge",
    "projection.apply",
    "projection.mytags.apply",
    "review.accept",
    "review.reject",
    "review.reopen",
    "runs.start",
    "schema.convert",
    "stems.separate",
];

/// The Stick a Library (at `library_dir`) or its Sidecar (at
/// `sidecar_dir`) is on, if either is.
pub(crate) fn stick_of(library_dir: &Path, sidecar_dir: &Path) -> Option<PathBuf> {
    wreckordbox_stick_protection::stick_root(sidecar_dir)
        .or_else(|| wreckordbox_stick_protection::stick_root(library_dir))
}

fn name_of(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The one refusal's words: what was refused, why, and what the DJ can do.
pub(crate) fn words(root: &Path, what: &str) -> String {
    format!(
        "{what} is not recorded for a Library on the Stick {}: a Library hosted on a Stick is read-only for \
         everything the Sidecar records (Facts, Review, Projection, Decisions, Mutants, Stems, Intensity), \
         so the Stick stays free to lock and eject. Nothing was written. Reading and ordinary Library edits \
         still work; to record, copy the Library to this computer and open it from there.",
        name_of(root)
    )
}

fn refusal(root: &Path, what: &str) -> ServiceError {
    ServiceError::new(codes::SIDECAR_ON_STICK, words(root, what))
}

/// Refuses `command` before it starts when it records in the Sidecar of a
/// Library hosted on a Stick.
pub(crate) fn refuse_records(
    library_dir: &Path,
    sidecar_dir: &Path,
    command: &str,
) -> Result<(), ServiceError> {
    if !RECORDS.contains(&command) {
        return Ok(());
    }
    match stick_of(library_dir, sidecar_dir) {
        Some(root) => Err(refusal(&root, &format!("`{command}`"))),
        None => Ok(()),
    }
}

/// Refuses `what` (a separation, a measurement, a tag that is only a DJ
/// Fact) for a Library hosted on a Stick, wherever it is asked from.
pub(crate) fn refuse_background_work(
    library_dir: &Path,
    sidecar_dir: &Path,
    what: &str,
) -> Result<(), ServiceError> {
    match stick_of(library_dir, sidecar_dir) {
        Some(root) => Err(refusal(&root, what)),
        None => Ok(()),
    }
}

/// Refuses, with Stick Protection's words, when the Library or its
/// Sidecar is on a Stick that is locked now: the Ledger's own housekeeping
/// (bringing it up to date at a start) waits for the unlock.
pub(crate) fn refuse_while_locked(stems: &crate::stems::Stems) -> Result<(), ServiceError> {
    let Some(root) = stick_of(stems.library_dir(), stems.sidecar_dir()) else {
        return Ok(());
    };
    if wreckordbox_stick_protection::status(stems.state_dir(), &root).protected {
        return Err(ServiceError::new(
            codes::STICK_PROTECTED,
            format!(
                "The Ledger is brought up to date once the Stick is unlocked; until then its queries wait. {}",
                wreckordbox_stick_protection::refusal(&root)
            ),
        ));
    }
    Ok(())
}

thread_local! {
    /// How deep this thread is in an ordinary Library edit's own record.
    static LEDGER: Cell<u32> = const { Cell::new(0) };
}

/// While held, this thread's Sidecar writes are an ordinary Library edit's
/// own record (the Ledger, its Runs and Assertions): kept on a Stick while
/// the Stick is unlocked. Nests.
#[must_use = "the scope ends when the guard is dropped"]
pub(crate) struct LedgerScope(());

pub(crate) fn ledger_scope() -> LedgerScope {
    LEDGER.with(|depth| depth.set(depth.get() + 1));
    LedgerScope(())
}

impl Drop for LedgerScope {
    fn drop(&mut self) {
        LEDGER.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// The admission the service installs on its Sidecar handle. Each folder's
/// volume is read once per handle (an open file stays on the filesystem it
/// was opened on); a mount table that cannot be read counts as a Stick.
pub(crate) fn admission(
    library_dir: &Path,
    sidecar_dir: &Path,
    state_dir: PathBuf,
) -> wreckordbox_sidecar::WriteAdmission {
    let sidecar = wreckordbox_stick_protection::HeldVolume::new(sidecar_dir);
    let library = wreckordbox_stick_protection::HeldVolume::new(library_dir);
    std::sync::Arc::new(move || {
        let Some(root) = sidecar.stick_root().or_else(|| library.stick_root()) else {
            return Ok(());
        };
        if LEDGER.with(Cell::get) == 0 {
            return Err(words(&root, "A Sidecar write"));
        }
        if wreckordbox_stick_protection::status(&state_dir, &root).protected {
            return Err(wreckordbox_stick_protection::refusal(&root));
        }
        Ok(())
    })
}
