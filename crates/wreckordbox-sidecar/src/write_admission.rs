//! One check every write of a [`Sidecar`](crate::Sidecar) handle passes
//! (wbx-8yh).
//!
//! Whoever owns the handle may install a write admission: a function that
//! says whether a write may be kept now, and if not, why. It is asked at
//! the moment a write would be kept, not when the handle was obtained:
//!
//! - every database write, through SQLite's commit hook on the handle's
//!   connection. A refused commit is rolled back by SQLite itself, so a
//!   statement or transaction prepared, cached or begun while writing was
//!   allowed keeps nothing once it is not;
//! - every file the handle itself makes in the Sidecar's folder (a Run's
//!   workspace, Stems moved in), checked before the file is touched.
//!
//! The refusal comes back as [`SidecarError::Refused`](crate::SidecarError)
//! with the admission's own words. The admission is called with the
//! connection's lock held: it must not use the Sidecar.

use std::cell::RefCell;
use std::sync::{Arc, PoisonError, RwLock};

/// Says whether a write may be kept now: `Err` with the reason when not.
pub type WriteAdmission = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// The admission a handle carries; none means every write is admitted.
#[derive(Default, Clone)]
pub(crate) struct Slot(Arc<RwLock<Option<WriteAdmission>>>);

thread_local! {
    /// Why the last commit on this thread was refused, for the error that
    /// SQLite answers in its place.
    static REFUSED: RefCell<Option<String>> = const { RefCell::new(None) };
}

impl Slot {
    pub(crate) fn set(&self, admission: WriteAdmission) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(admission);
    }

    /// Asks the admission, if there is one.
    pub(crate) fn admit(&self) -> Result<(), String> {
        let admission = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        admission.map_or(Ok(()), |admit| admit())
    }

    /// Installs the commit hook that asks the admission on `db`.
    pub(crate) fn guard(&self, db: &rusqlite::Connection) -> rusqlite::Result<()> {
        let slot = self.clone();
        db.commit_hook(Some(move || match slot.admit() {
            Ok(()) => false,
            Err(why) => {
                REFUSED.with(|refused| *refused.borrow_mut() = Some(why));
                // True turns the commit into a rollback.
                true
            }
        }))
    }
}

/// The refusal behind an SQLite error, when the error is a commit the
/// admission refused.
pub(crate) fn refusal_of(error: &rusqlite::Error) -> Option<String> {
    let rusqlite::Error::SqliteFailure(failure, _) = error else {
        return None;
    };
    if failure.extended_code != rusqlite::ffi::SQLITE_CONSTRAINT_COMMITHOOK {
        return None;
    }
    Some(
        REFUSED
            .with(|refused| refused.borrow_mut().take())
            .unwrap_or_else(|| "the Sidecar refused this write".to_owned()),
    )
}
