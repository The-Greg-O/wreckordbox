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
            .map(|fingerprint| sidecar.mark_stale(track, fingerprint))
            .transpose()
            .map_err(|e| super::stems::sidecar_error(&e))?
            .unwrap_or_default();
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
                    track_id: track.clone(),
                    kind,
                    fact_id: None,
                    stale: ids,
                }),
            );
        }
        Ok((attachments, stale))
    }
}
