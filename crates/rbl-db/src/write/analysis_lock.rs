//! Recording adapter for the existing native setter; no analysis/audio reads.
use super::{TrackEdit, Writer};
use crate::Result;

impl Writer {
    /// Set the native analysis lock, retaining the exact flag values for the
    /// ordinary edit/assertion path. A repeated value writes no row/revision.
    pub fn set_analysis_lock_with_undo(
        &mut self,
        content: &str,
        on: bool,
    ) -> Result<Option<TrackEdit>> {
        let locked: bool = self.library().connection().query_row(
            "SELECT (COALESCE(Analysed,0) & 128) != 0 FROM djmdContent WHERE ID=?1 AND rb_local_deleted=0",
            [content],
            |row| row.get(0),
        )?;
        if locked == on {
            return Ok(None);
        }
        let before = self.track_value(content, "Analysed")?;
        self.set_analysis_lock(content, on)?;
        let after = self.track_value(content, "Analysed")?;
        Ok(Some(TrackEdit {
            content: content.to_owned(),
            column: "Analysed",
            before,
            after,
        }))
    }
}
