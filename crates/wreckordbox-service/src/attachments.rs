//! One audio observation path for Fact and Stems clients.

use serde::Serialize;
use serde_json::json;

/// Attachment notifications are distinct from Fact-kind changes.
#[derive(Serialize)]
struct AttachmentChanged<'a> {
    #[serde(flatten)]
    attachment: &'a Attachment,
    run: Option<RunId>,
}
use wreckordbox_sidecar::{
    attachments::{Attachment, Attachments},
    Fingerprint, RunId,
};

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
        self.before_record(found, fingerprint, None)
    }

    /// Shared pre-write observation for EVERY Fact producer, including
    /// producers which own the transaction that records the Fact.
    pub(crate) fn before_record(
        &self,
        found: &ParentAudio,
        fingerprint: Option<&Fingerprint>,
        run: Option<RunId>,
    ) -> Result<(Attachments, Vec<i64>), ServiceError> {
        let sidecar = self.sidecar()?;
        let track = &found.parent().id;
        let attachments = sidecar
            .observe_audio_in_run(
                track,
                fingerprint,
                &found.identity,
                |id| found.index.row_of(id).is_some(),
                run,
            )
            .map_err(|e| super::stems::sidecar_error(&e))?;
        for attached in &attachments.reattached {
            self.publish(
                "facts_reattached",
                json!(AttachmentChanged {
                    attachment: attached,
                    run
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
