//! One audio observation path for Fact and Stems clients.

use serde_json::json;
use wreckordbox_sidecar::{attachments::Attachments, Fingerprint};

use crate::{
    stems::{events, ParentAudio, SidecarChanged, Stems},
    ServiceError,
};

impl Stems {
    pub(crate) fn observe_audio(
        &self,
        found: &ParentAudio,
        fingerprint: Option<&Fingerprint>,
    ) -> Result<(Attachments, Vec<i64>), ServiceError> {
        let sidecar = self.sidecar()?;
        let track = &found.parent().id;
        let attachments = sidecar
            .observe_audio(track, fingerprint, &found.identity, |id| {
                found.index.row_of(id).is_some()
            })
            .map_err(|e| super::stems::sidecar_error(&e))?;
        for attached in &attachments.reattached {
            self.publish(
                events::SIDECAR_CHANGED,
                json!({
                    "track_id": track, "kind": "attachment", "fact_id": null, "stale": [],
                    "from_track": attached.from_track, "fact_ids": attached.fact_ids,
                }),
            );
        }
        let stale = fingerprint
            .map(|fingerprint| self.mark_stale(track, fingerprint))
            .transpose()?
            .unwrap_or_default();
        Ok((attachments, stale))
    }

    /// Also used by producers that commit a Fact in their own transaction.
    pub(crate) fn mark_stale(
        &self,
        track: &str,
        fingerprint: &Fingerprint,
    ) -> Result<Vec<i64>, ServiceError> {
        let sidecar = self.sidecar()?;
        let stale = sidecar
            .mark_stale(track, fingerprint)
            .map_err(|e| super::stems::sidecar_error(&e))?;
        let mut by_kind = std::collections::BTreeMap::<String, Vec<i64>>::new();
        for id in &stale {
            if let Some(fact) = sidecar
                .fact(*id)
                .map_err(|e| super::stems::sidecar_error(&e))?
            {
                by_kind.entry(fact.kind).or_default().push(*id);
            }
        }
        for (kind, ids) in by_kind {
            self.publish(
                events::SIDECAR_CHANGED,
                json!(SidecarChanged {
                    track_id: track.to_owned(),
                    kind,
                    fact_id: None,
                    stale: ids,
                }),
            );
        }
        Ok(stale)
    }
}
