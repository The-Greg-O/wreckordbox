//! Mutant edit transactions and live parent/readiness projection.
use super::{
    actions, again, mutant_edits, place, sidecar_error, Committed, CreateMutant, EditOutcome,
    Edits, Mutant, MutantUpdate, Mutants, OpenLibrary, PlaylistName, RemoveMutant, Renderability,
    ServiceError, Value,
};
use crate::mutant_edits::{ClearMutant, Cue, EditMutant};
use crate::tracks::TrackRecord;

pub(super) fn parse_edits(value: Value) -> Result<Edits, ServiceError> {
    let failed = |message: String| {
        ServiceError::new(
            "mutant_edits_invalid",
            format!("invalid stored Mutant edits: {message}"),
        )
    };
    let mut edits: Edits = serde_json::from_value(value).map_err(|e| failed(e.to_string()))?;
    edits.validate().map_err(|e| failed(e.message))?;
    Ok(edits)
}

pub(super) fn metadata(
    library: &OpenLibrary,
    id: &str,
) -> Result<(Option<TrackRecord>, Vec<Cue>), ServiceError> {
    let index = library.index()?;
    let Some(row) = index.row_of(id) else {
        return Ok((None, Vec::new()));
    };
    let comments = library.with_db(|db| {
        rbl_index::reload_cues_of(db, &index, id)
            .map_err(|e| ServiceError::new("library_failed", e.to_string()))?;
        rbl_db::details::cue_comments(db.connection(), id)
            .map_err(|e| ServiceError::new("library_failed", e.to_string()))
    })?;
    let cues = index
        .cues_of(row)
        .iter()
        .map(|cue| Cue {
            kind: cue.kind,
            position_ms: cue.position_ms,
            out_ms: cue.out_ms,
            colour: cue.colour,
            memory_colour: cue.memory_colour,
            comment: comments
                .get(&cue.id.to_string())
                .cloned()
                .unwrap_or_default(),
        })
        .collect();
    Ok((Some(crate::tracks::record(&index, row)), cues))
}

pub(super) fn source_readiness(parent: Option<&TrackRecord>) -> Renderability {
    match parent {
        None => Renderability::blocked(ServiceError::new(
            "track_not_found",
            "the Mutant's parent is no longer in the Library",
        )),
        Some(p) if !std::path::Path::new(&p.location).is_file() => Renderability::blocked(
            ServiceError::new("track_file_missing", "the parent's audio is missing"),
        ),
        Some(_) => Renderability::ready(),
    }
}
impl Mutants<'_> {
    pub(super) fn edit(&self, args: &EditMutant) -> Result<EditOutcome, ServiceError> {
        self.change_edits(
            &RemoveMutant {
                id: args.id,
                entry: args.entry.clone(),
                run: args.run,
                source: args.source.clone(),
                description: args.description.clone(),
            },
            |edits| {
                if let Ok(Value::Object(patch)) = serde_json::to_value(&args.edits) {
                    edits.extend(patch);
                }
            },
            args.edits.stems.is_some(),
        )
    }
    pub(super) fn clear(&self, args: &ClearMutant) -> Result<EditOutcome, ServiceError> {
        self.change_edits(
            &RemoveMutant {
                id: args.id,
                entry: args.entry.clone(),
                run: args.run,
                source: args.source.clone(),
                description: args.description.clone(),
            },
            |edits| {
                let key = match args.edit {
                    crate::mutant_edits::Aspect::Stems => "stems",
                    crate::mutant_edits::Aspect::KeyShift => "key_shift",
                    crate::mutant_edits::Aspect::Tempo => "tempo",
                    crate::mutant_edits::Aspect::Cues => "cues",
                };
                edits.remove(key);
            },
            false,
        )
    }

    fn change_edits(
        &self,
        args: &RemoveMutant,
        patch: impl FnOnce(&mut serde_json::Map<String, Value>),
        check_stems: bool,
    ) -> Result<EditOutcome, ServiceError> {
        let provenance =
            self.provenance(args.run, args.source.as_ref(), args.description.as_deref())?;
        let first = self.stored(args.id, args.entry.as_deref(), false)?;
        let mutex = self.stems.mutant_lock(first.id);
        let _serial = mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stored = self.stored(args.id, args.entry.as_deref(), false)?;
        // Parse, then patch, then validate: an older stored value can be repaired.
        let mut raw = stored.edits.as_object().cloned().ok_or_else(|| {
            ServiceError::new(
                "mutant_edits_invalid",
                format!(
                    "Mutant {}: stored edits are not an object; inspect or remove this Mutant",
                    stored.id
                ),
            )
        })?;
        patch(&mut raw);
        let mut edits: Edits = serde_json::from_value(Value::Object(raw)).map_err(|e| {
            ServiceError::new("mutant_edits_invalid", format!("Mutant {}: {e}", stored.id))
        })?;
        edits.validate()?;
        let (found, parent, cues) = self.read(|library| {
            library.steady(|| {
                let found = place(
                    library,
                    &CreateMutant {
                        entry: Some(stored.entry_id.clone()),
                        ..Default::default()
                    },
                )?;
                let (parent, cues) = metadata(library, &stored.track_id)?;
                Ok((found, parent, cues))
            })
        })?;
        edits.validate_cues(parent.as_ref(), &cues)?;
        let value = serde_json::to_value(&edits)
            .map_err(|e| ServiceError::new("bad_request", e.to_string()))?;
        if value == stored.edits {
            let mutant = self
                .described(vec![stored], false)?
                .pop()
                .ok_or_else(|| ServiceError::new("service_failed", "no Mutant"))?;
            return Ok(EditOutcome {
                mutant,
                run: None,
                created_run: false,
                changed: false,
            });
        }
        let audio_stamp = stamp(&found.audio);
        if check_stems && edits.stems.is_some() {
            self.check_stems(
                parent
                    .as_ref()
                    .ok_or_else(|| ServiceError::new("track_not_found", "the parent is missing"))?,
                true,
            )?;
        }
        let fingerprint = wreckordbox_sidecar::known_fingerprint(&found.audio);
        let renderability = self.readiness(&edits, parent.as_ref(), false)?;
        let (mutant, run) = self.read(|library| {
            library.steady(|| {
                let current = self.stored(args.id, args.entry.as_deref(), false)?;
                if current != stored {
                    return Err(ServiceError::new(
                        "mutant_changed",
                        "the Mutant changed; read it again and retry",
                    ));
                }
                let fresh = again(library, &found)?;
                if fresh.audio != found.audio || stamp(&fresh.audio) != audio_stamp {
                    return Err(ServiceError::new(
                        "mutant_parent_changed",
                        "the parent's audio changed while checking the edit; try again",
                    ));
                }
                let found = fresh;
                let (inherited, cues) = metadata(library, &stored.track_id)?;
                let derived =
                    mutant_edits::derive(&edits, inherited.as_ref(), &cues, &stored.track_id);
                let value = serde_json::to_value(&edits)
                    .map_err(|e| ServiceError::new("bad_request", e.to_string()))?;
                let description = provenance.description.clone().unwrap_or_else(|| {
                    format!("Edit Mutant {}: {}", stored.id, derived.description)
                });
                let Committed { mutant: made, run } = self
                    .writer()?
                    .edit_mutant(
                        stored.id,
                        &value,
                        &MutantUpdate {
                            fingerprint: fingerprint.as_ref(),
                            source: &provenance.source,
                            run: provenance.in_run(&description),
                        },
                    )
                    .map_err(|e| sidecar_error(&e))?;
                self.changed(&made, actions::EDITED);
                Ok((
                    Mutant {
                        id: made.id,
                        playlist: PlaylistName {
                            id: found.entry.playlist_id,
                            name: found.playlist_name,
                        },
                        entry_id: found.entry.id,
                        position: Some(found.entry.position),
                        parent: found.parent,
                        fingerprint: made.fingerprint,
                        edits: made.edits,
                        inherited,
                        derived: Some(derived),
                        renderability,
                        source: made.source,
                        run: made.run.0,
                        created_at: made.created_at,
                        changed_at: made.changed_at,
                        removed: false,
                    },
                    run,
                ))
            })
        })?;
        Ok(EditOutcome {
            mutant,
            run: Some(run),
            changed: true,
            created_run: provenance.run.is_none(),
        })
    }
}

// A quick recheck under the gate; the expensive content reads happen outside it.
fn stamp(path: &std::path::Path) -> Option<wreckordbox_sidecar::stems::FileStamp> {
    wreckordbox_sidecar::stems::file_stamp(path)
}
