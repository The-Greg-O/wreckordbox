//! Additional archive queries on the immutable reader; no recovery or writes.
use crate::attachments::Review;
use crate::{Fact, FactsReader, Result};
impl FactsReader {
    /// Every Fact currently attached to the track, newest first.
    pub fn facts_of_track(&self, track: &str) -> Result<Vec<Fact>> {
        let mut query = self.connection().prepare_cached(&crate::store::select(
            "WHERE fact.effective_track_id = ?1 ORDER BY fact.id DESC",
        ))?;
        let result = query
            .query_map([track], crate::store::read_fact)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }
    /// Previously stored attachment candidates, without observing audio.
    pub fn attachment_reviews(&self, track: &str) -> Result<Vec<Review>> {
        let mut query = self.connection().prepare_cached("SELECT from_track, reason FROM attachment_review WHERE track_id = ?1 ORDER BY from_track")?;
        let result = query
            .query_map([track], |row| {
                Ok(Review {
                    from_track: row.get(0)?,
                    reason: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }
}
