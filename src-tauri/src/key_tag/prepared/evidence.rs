//! No plan/state mutex spans audio inspection or evidence publication.
use super::PreparedFiles;
impl PreparedFiles {
    pub(super) fn capture_evidence(&self) -> Result<(), String> {
        let Some(ledger) = &self.ledger else {
            return Ok(());
        };
        let mut files = std::mem::take(&mut *self.files.lock());
        let result = files.iter_mut().try_for_each(|plan| {
            plan.evidence = ledger.prepare_key_evidence(&plan.file, &plan.tracks)?;
            Ok(())
        });
        *self.files.lock() = files;
        result
    }
    pub(super) fn record_evidence(&self) -> Result<(), String> {
        let files = std::mem::take(&mut *self.files.lock());
        let mut errors = Vec::new();
        for plan in files.iter().filter(|p| p.file.is_published()) {
            if let Some(evidence) = &plan.evidence {
                if let Err(why) = evidence.record_key_evidence(&plan.file) {
                    errors.push(format!("{}: {why}", plan.requested.display()));
                }
            }
        }
        *self.files.lock() = files;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
