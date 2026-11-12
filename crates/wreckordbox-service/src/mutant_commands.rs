//! Mutant edit transactions and live parent/readiness projection.
use super::*;
use crate::mutant_edits::{codes, ClearMutant, Cue, EditMutant};
use crate::tracks::TrackRecord;

pub(super) fn parse_edits(value: Value) -> Result<Edits, ServiceError> {
    serde_json::from_value(value)
        .map_err(|e| ServiceError::new("bad_request", format!("invalid Mutant edits: {e}")))
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
    pub(super) fn readiness(
        &self,
        edits: &Edits,
        parent: Option<&TrackRecord>,
    ) -> Result<Renderability, ServiceError> {
        let source = source_readiness(parent);
        if !source.renderable || edits.stems.is_none() {
            return Ok(source);
        }
        let Some(parent) = parent else {
            return Ok(source);
        };
        if let Err(error) = self.check_stems(parent) {
            return if error.code == stems::codes::SIDECAR_FAILED {
                Err(error)
            } else {
                Ok(Renderability::blocked(error))
            };
        }
        let extension = std::path::Path::new(&parent.location)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "mp3" | "m4a") {
            return Ok(Renderability::blocked(ServiceError::new(codes::STEM_ALIGNMENT_UNKNOWN,
                "Stem edits of MP3 and M4A parents cannot be rendered until wbx-49.1 aligns Stems to the parent's timeline")));
        }
        Ok(Renderability::ready())
    }

    fn check_stems(&self, parent: &TrackRecord) -> Result<Fingerprint, ServiceError> {
        let facts = inspect(self.stems, |store| store.stem_facts(&parent.id))?;
        let sets: Vec<_> = facts
            .into_iter()
            .filter(|f| f.is_live())
            .filter_map(|f| {
                wreckordbox_sidecar::stems::StemSet::from_fact(self.stems.sidecar_dir(), f)
            })
            .collect();
        if sets.is_empty() {
            return Err(ServiceError::new(
                codes::STEMS_MISSING,
                "separation is needed: the parent has no Stems",
            ));
        }
        let fingerprint = stems::fingerprint(
            &Parent {
                id: parent.id.clone(),
                title: parent.title.clone(),
            },
            std::path::Path::new(&parent.location),
        )?;
        let matching: Vec<_> = sets
            .iter()
            .filter(|s| s.fingerprint == fingerprint)
            .collect();
        if matching.is_empty() {
            return Err(ServiceError::new(
                codes::STEMS_STALE,
                "separation is needed: the Stems belong to different parent audio",
            ));
        }
        if !matching.iter().any(|s| s.audio_verified()) {
            return Err(ServiceError::new(
                codes::STEMS_MISSING,
                "separation is needed: Stem files are missing or damaged",
            ));
        }
        Ok(fingerprint)
    }

    pub(super) fn edit(&self, args: &EditMutant) -> Result<MutantChanged, ServiceError> {
        self.change_edits(
            &RemoveMutant {
                id: args.id,
                entry: args.entry.clone(),
                run: args.run,
                source: args.source.clone(),
                description: args.description.clone(),
            },
            |edits| edits.patch(args.edits.clone()),
            args.edits.stems.is_some(),
        )
    }
    pub(super) fn clear(&self, args: &ClearMutant) -> Result<MutantChanged, ServiceError> {
        self.change_edits(
            &RemoveMutant {
                id: args.id,
                entry: args.entry.clone(),
                run: args.run,
                source: args.source.clone(),
                description: args.description.clone(),
            },
            |edits| edits.clear(args.edit),
            false,
        )
    }

    fn change_edits(
        &self,
        args: &RemoveMutant,
        patch: impl FnOnce(&mut Edits),
        check_stems: bool,
    ) -> Result<MutantChanged, ServiceError> {
        let provenance =
            self.provenance(args.run, args.source.as_ref(), args.description.as_deref())?;
        // Serialize patches without holding the Library gate while fingerprinting
        // parent audio or hashing Stems. Recheck the entry/path before committing.
        let _serial = self
            .stems
            .mutant_edits
            .lock()
            .map_err(|_| ServiceError::new("service_failed", "the Mutant edit lock failed"))?;
        let stored = self.stored(args.id, args.entry.as_deref(), false)?;
        let mut edits = parse_edits(stored.edits.clone())?;
        patch(&mut edits);
        edits.validate()?;
        let (found, parent) = self.read(|library| {
            library.steady(|| {
                let found = place(
                    library,
                    &CreateMutant {
                        entry: Some(stored.entry_id.clone()),
                        ..Default::default()
                    },
                )?;
                let (parent, _) = metadata(library, &stored.track_id)?;
                Ok((found, parent))
            })
        })?;
        let audio_stamp = stamp(&found.audio);
        let fingerprint =
            if check_stems && edits.stems.is_some() {
                Some(self.check_stems(parent.as_ref().ok_or_else(|| {
                    ServiceError::new("track_not_found", "the parent is missing")
                })?)?)
            } else {
                wreckordbox_sidecar::fingerprint_file(&found.audio).ok()
            };
        let renderability = self.readiness(&edits, parent.as_ref())?;
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
                        derived,
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
        Ok(MutantChanged {
            mutant,
            run,
            created_run: provenance.run.is_none(),
        })
    }
}

// A quick recheck under the gate; the expensive content reads happen outside it.
fn stamp(path: &std::path::Path) -> Option<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}
