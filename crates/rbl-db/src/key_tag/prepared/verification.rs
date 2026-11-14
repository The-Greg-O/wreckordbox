//! Bind an off-gate observation to this writer's byte-verified staged copy.
use super::{changed, same, Prepared, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Cannot be constructed from a read or deserialized. It belongs to one
/// prepared publication that already passed the key writer's byte comparison.
pub struct Verified<T> {
    temporary: PathBuf,
    observed: T,
}
impl<T> Verified<T> {
    pub fn observed(&self) -> &T {
        &self.observed
    }
}
impl Prepared {
    /// Inspect before/after audio outside the Library gate. Metadata on both
    /// sides must still match the copies the key writer verified byte for byte.
    pub fn inspect_verified<T>(
        &self,
        inspect: impl FnOnce(&Path, &Path) -> Result<T>,
    ) -> Result<Verified<T>> {
        let check = || {
            self.check()?;
            if self.published.is_some()
                || !self
                    .staged
                    .as_ref()
                    .is_some_and(|s| fs::metadata(&self.temp).is_ok_and(|now| same(s, &now)))
            {
                return Err(changed());
            }
            Ok(())
        };
        check()?;
        let observed = inspect(&self.path, &self.temp)?;
        check()?;
        Ok(Verified {
            temporary: self.temp.clone(),
            observed,
        })
    }

    /// Only the publication that was inspected can authorize its evidence.
    /// Failed or rolled-back publication has no such authority.
    pub fn published_verification<'a, T>(&self, verified: &'a Verified<T>) -> Option<&'a T> {
        (self.is_published() && self.temp == verified.temporary).then_some(&verified.observed)
    }
}
