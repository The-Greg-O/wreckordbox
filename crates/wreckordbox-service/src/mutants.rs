//! Mutants (ADR 0008): a playlist's own edited version of a track.
//!
//! A playlist entry is mutated; from then on that entry has a Mutant, kept
//! in the Sidecar and keyed to the entry's own identifier in the Library
//! (what `playlists.tracks` answers as `entry_id`), not to its position.
//! The same track in another playlist, or elsewhere in the same one, is
//! another entry and is untouched. Removing the Mutant returns the entry to
//! the core track.
//!
//! Nothing here writes to the Library: the playlist's entry there stays the
//! parent track, no `library_changed` is sent, the Library's generation
//! does not move and Library Protection does not apply. A change is
//! announced as [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED)
//! with the kind [`KIND`], the event the Stems and Facts commands send.
//!
//! Every change is made by a source, in a Run, as a Fact is recorded: in a
//! Run started with `runs.start`, or in one made for the command alone.
//!
//! A Mutant's edits are a JSON object, empty until something edits it: an
//! absent key means that aspect is the parent's.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_sidecar::mutants::{actions, Committed, InRun, MutantUpdate, NewMutant};
use wreckordbox_sidecar::{RunId, Sidecar, SidecarError};

use crate::entries::{self, Entry};
use crate::facts::{codes as fact_codes, Fingerprint, Run, Source, SourceArg};
use crate::library::OpenLibrary;
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Parent, Stems};

/// The `kind` of a `sidecar_changed` event about a Mutant.
pub const KIND: &str = "mutant";

/// The names of the Mutant commands.
pub mod commands {
    /// Mutates a playlist entry. Takes a [`CreateMutant`](super::CreateMutant),
    /// answers a [`MutantChanged`](super::MutantChanged).
    pub const MUTANTS_CREATE: &str = "mutants.create";
    /// One Mutant and what has happened to it. Takes a
    /// [`WhichMutant`](super::WhichMutant), answers a
    /// [`MutantShown`](super::MutantShown).
    pub const MUTANTS_GET: &str = "mutants.get";
    /// The Mutants of one playlist, of one track, or of the whole Library.
    /// Takes a [`ListMutants`](super::ListMutants), answers a
    /// [`MutantList`](super::MutantList).
    pub const MUTANTS_LIST: &str = "mutants.list";
    /// Removes a Mutant: its entry is the core track again. Takes a
    /// [`RemoveMutant`](super::RemoveMutant), answers a
    /// [`MutantChanged`](super::MutantChanged).
    pub const MUTANTS_REMOVE: &str = "mutants.remove";
}

/// The error codes of the Mutant commands, beside the service's own, the
/// Run and source codes of [`crate::facts::codes`] and
/// [`stems::codes::SIDECAR_FAILED`]. Nothing is changed when any is
/// answered.
pub mod codes {
    /// There is no such entry in the Library now: no entry with that
    /// identifier, or no entry at that position of the playlist.
    pub const PLAYLIST_ENTRY_NOT_FOUND: &str = "playlist_entry_not_found";
    /// The entry already has a Mutant.
    pub const ALREADY_A_MUTANT: &str = "already_a_mutant";
    /// There is no Mutant with that identifier, or the entry has none.
    pub const MUTANT_NOT_FOUND: &str = "mutant_not_found";
}

/// The arguments of `mutants.create`: the entry by its own identifier, or
/// by its playlist and position.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateMutant {
    /// The entry's identifier: `entry_id` of `playlists.tracks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// With `position`, in place of `entry`: the playlist's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist: Option<String>,
    /// The entry's place in `playlist` now, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    /// The Run to make the change in, from `runs.start`. Left out, a Run
    /// is made for this command alone and ended with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// Who is making the change. Left out, the source of `run`. Needed
    /// when a Run is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    /// What the Run made for this command is called. Not with `run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The arguments of `mutants.get`: exactly one of the two.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhichMutant {
    /// The Mutant's identifier. A removed Mutant is still answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// The identifier of the entry whose Mutant is wanted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

/// The arguments of `mutants.list`. With neither, every Mutant in the
/// Library.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListMutants {
    /// Only the Mutants of this playlist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist: Option<String>,
    /// Only the Mutants whose parent is this track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
}

/// The arguments of `mutants.remove`: the Mutant by its identifier or by
/// its entry's, and who removes it, as [`CreateMutant`] says who makes it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveMutant {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A playlist, named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistName {
    pub id: String,
    /// Empty when the Library no longer has the playlist.
    pub name: String,
}

/// A Mutant as the commands answer it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mutant {
    pub id: i64,
    /// The playlist it belongs to.
    pub playlist: PlaylistName,
    /// The identifier of the playlist entry it belongs to.
    pub entry_id: String,
    /// Where that entry is in the playlist now, from 1; `null` when the
    /// Library no longer has the entry.
    pub position: Option<usize>,
    /// The track it is a version of. The title is empty when the Library
    /// no longer has the track.
    pub parent: Parent,
    /// The fingerprint of the parent's audio when the entry was mutated;
    /// `null` when its file was not at hand.
    pub fingerprint: Option<Fingerprint>,
    /// What differs from the parent: an object, empty when nothing does.
    pub edits: Value,
    /// Who last changed it.
    pub source: Source,
    /// The identifier of the Run it was last changed in.
    pub run: i64,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    pub changed_at: i64,
    /// True once it has been removed: the entry is the core track.
    pub removed: bool,
}

/// One thing that happened to a Mutant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutantChange {
    pub id: i64,
    /// `created`, `edited` or `removed`.
    pub action: String,
    /// The Mutant's edits after the change.
    pub edits: Value,
    /// The fingerprint of the parent's audio then, when it was read.
    pub fingerprint: Option<Fingerprint>,
    pub source: Source,
    pub run: i64,
    /// Milliseconds since the Unix epoch.
    pub recorded_at: i64,
}

/// What `mutants.create` and `mutants.remove` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutantChanged {
    pub mutant: Mutant,
    /// The Run the change was made in.
    pub run: Run,
    /// True when the Run was made for this command (and ended with it).
    pub created_run: bool,
}

/// What `mutants.get` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutantShown {
    pub mutant: Mutant,
    /// Everything that has happened to it, oldest first.
    pub history: Vec<MutantChange>,
}

/// What `mutants.list` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutantList {
    /// By playlist, then in the playlist's order; a Mutant whose entry the
    /// Library no longer has comes after its playlist's others.
    pub mutants: Vec<Mutant>,
    pub total: usize,
}

/// The data of a `sidecar_changed` event about a Mutant: the fields every
/// such event has (`fact_id` is always `null` and `stale` empty here),
/// and which Mutant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutantEvent {
    /// The parent track.
    pub track_id: String,
    /// [`KIND`].
    pub kind: String,
    pub fact_id: Option<i64>,
    pub stale: Vec<i64>,
    pub mutant_id: i64,
    pub playlist_id: String,
    pub entry_id: String,
    /// `created`, `edited` or `removed`.
    pub action: String,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(
        command,
        commands::MUTANTS_CREATE
            | commands::MUTANTS_GET
            | commands::MUTANTS_LIST
            | commands::MUTANTS_REMOVE
    )
}

/// How this module reads the Library: it is handed the open Library for
/// the length of one closure, which is the only time the Library's lock is
/// held. Nothing is fingerprinted and the Sidecar is not written under it.
pub(crate) type WithLibrary<'a> = &'a dyn Fn(
    &mut dyn FnMut(&OpenLibrary) -> Result<(), ServiceError>,
) -> Result<(), ServiceError>;

/// Answers one of this module's commands.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    library: WithLibrary,
) -> Result<Value, ServiceError> {
    let mutants = Mutants {
        sidecar: stems.sidecar()?,
        stems,
        library,
    };
    let answer = match request.command.as_str() {
        commands::MUTANTS_CREATE => serde_json::to_value(mutants.create(&arguments(request)?)?),
        commands::MUTANTS_GET => serde_json::to_value(mutants.get(&arguments(request)?)?),
        commands::MUTANTS_LIST => serde_json::to_value(mutants.list(&arguments(request)?)?),
        _ => serde_json::to_value(mutants.remove(&arguments(request)?)?),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

fn arguments<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, ServiceError> {
    serde_json::from_value(request.args.clone()).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

struct Mutants<'a> {
    sidecar: Arc<Sidecar>,
    stems: &'a Stems,
    library: WithLibrary<'a>,
}

/// An entry found in the Library, with what a Mutant records of it.
struct Place {
    entry: Entry,
    playlist_name: String,
    parent: Parent,
    /// Where the Library says the parent's audio is.
    audio: std::path::PathBuf,
}

/// Who makes a change, and the Run it is made in.
struct Provenance {
    source: Source,
    /// The Run given, still open; `None` when one is to be made.
    run: Option<Run>,
    description: Option<String>,
}

impl Provenance {
    /// The Run the change goes in: the one given, or one made for it with
    /// `description` (the store checks, makes and ends it with the change).
    fn in_run<'a>(&self, description: &'a str) -> InRun<'a> {
        match &self.run {
            Some(run) => InRun::Open(run.id),
            None => InRun::Own { description },
        }
    }
}

impl Mutants<'_> {
    /// Reads the Library: `read` runs with the Library's lock held.
    fn read<T>(
        &self,
        read: impl FnOnce(&OpenLibrary) -> Result<T, ServiceError>,
    ) -> Result<T, ServiceError> {
        let mut read = Some(read);
        let mut answer = None;
        (self.library)(&mut |library| {
            if let Some(read) = read.take() {
                answer = Some(read(library)?);
            }
            Ok(())
        })?;
        answer.ok_or_else(|| {
            ServiceError::new(service_codes::SERVICE_FAILED, "the Library was not read")
        })
    }

    fn create(&self, create: &CreateMutant) -> Result<MutantChanged, ServiceError> {
        // Found with no Library edit under way, so the entry, its position
        // and its track are those of one state of the Library.
        let found = self.read(|library| library.steady(|| place(library, create)))?;
        let provenance = self.provenance(
            create.run,
            create.source.as_ref(),
            create.description.as_deref(),
        )?;
        if let Some(existing) = self
            .sidecar
            .mutant_of_entry(&found.entry.id)
            .map_err(|e| sidecar_error(&e))?
        {
            return Err(already(&found.entry.id, existing.id));
        }
        // With no lock held, as an edit may come meanwhile: one read of the
        // parent's audio. A Mutant is about the music, so a parent whose
        // file is not at hand can still be mutated.
        let fingerprint = wreckordbox_sidecar::fingerprint_file(&found.audio).ok();
        // Again with no edit under way, until the change is committed: the
        // entry, its playlist and its track are looked up afresh, and an
        // edit that removed any of them meanwhile refuses the change.
        let (place, committed) = self.read(|library| {
            library.steady(|| {
                let place = again(library, &found)?;
                let description = provenance.description.clone().unwrap_or_else(|| {
                    format!(
                        "Mutate entry {} of playlist \"{}\" (track {} \"{}\")",
                        place.entry.position,
                        place.playlist_name,
                        place.parent.id,
                        place.parent.title
                    )
                });
                let committed = self
                    .sidecar
                    .create_mutant(&NewMutant {
                        playlist_id: &place.entry.playlist_id,
                        entry_id: &place.entry.id,
                        track_id: &place.entry.track_id,
                        fingerprint: fingerprint.as_ref(),
                        edits: json!({}),
                        source: &provenance.source,
                        run: provenance.in_run(&description),
                    })
                    .map_err(|e| sidecar_error(&e))?;
                Ok((place, committed))
            })
        })?;
        // Committed: nothing below can fail.
        let Committed { mutant: made, run } = committed;
        self.changed(&made, actions::CREATED);
        Ok(MutantChanged {
            mutant: Mutant {
                id: made.id,
                playlist: PlaylistName {
                    id: place.entry.playlist_id,
                    name: place.playlist_name,
                },
                entry_id: place.entry.id,
                position: Some(place.entry.position),
                parent: place.parent,
                fingerprint: made.fingerprint,
                edits: made.edits,
                source: made.source,
                run: made.run.0,
                created_at: made.created_at,
                changed_at: made.changed_at,
                removed: false,
            },
            run,
            created_run: provenance.run.is_none(),
        })
    }

    fn get(&self, which: &WhichMutant) -> Result<MutantShown, ServiceError> {
        let stored = self.stored(which.id, which.entry.as_deref(), true)?;
        let history = self
            .sidecar
            .mutant_changes(stored.id)
            .map_err(|e| sidecar_error(&e))?
            .into_iter()
            .map(|change| MutantChange {
                id: change.id,
                action: change.action,
                edits: change.edits,
                fingerprint: change.fingerprint,
                source: change.source,
                run: change.run.0,
                recorded_at: change.recorded_at,
            })
            .collect();
        let mutant = self
            .described(vec![stored])?
            .pop()
            .ok_or_else(|| ServiceError::new(service_codes::SERVICE_FAILED, "no Mutant"))?;
        Ok(MutantShown { mutant, history })
    }

    fn list(&self, list: &ListMutants) -> Result<MutantList, ServiceError> {
        let mut stored = match &list.track {
            Some(track) => self.sidecar.mutants_of_track(track),
            None => self.sidecar.mutants(list.playlist.as_deref()),
        }
        .map_err(|e| sidecar_error(&e))?;
        if let (Some(playlist), Some(_)) = (&list.playlist, &list.track) {
            stored.retain(|mutant| &mutant.playlist_id == playlist);
        }
        let mut mutants = self.described(stored)?;
        // By playlist as first met (the oldest Mutant's playlist first),
        // then in the playlist's order.
        let mut met: Vec<String> = Vec::new();
        for mutant in &mutants {
            if !met.contains(&mutant.playlist.id) {
                met.push(mutant.playlist.id.clone());
            }
        }
        mutants.sort_by_key(|mutant| {
            (
                met.iter().position(|id| id == &mutant.playlist.id),
                mutant.position.unwrap_or(usize::MAX),
                mutant.id,
            )
        });
        Ok(MutantList {
            total: mutants.len(),
            mutants,
        })
    }

    fn remove(&self, remove: &RemoveMutant) -> Result<MutantChanged, ServiceError> {
        let stored = self.stored(remove.id, remove.entry.as_deref(), false)?;
        let provenance = self.provenance(
            remove.run,
            remove.source.as_ref(),
            remove.description.as_deref(),
        )?;
        // Described before the change, so that once it is committed nothing
        // is left to fail.
        let described = self
            .described(vec![stored.clone()])?
            .pop()
            .ok_or_else(|| ServiceError::new(service_codes::SERVICE_FAILED, "no Mutant"))?;
        let description = provenance
            .description
            .clone()
            .unwrap_or_else(|| format!("Remove Mutant {}", stored.id));
        let Committed { mutant: removed, run } = self
            .sidecar
            .remove_mutant(
                stored.id,
                &MutantUpdate {
                    // The parent's audio is not read to remove a Mutant.
                    fingerprint: None,
                    source: &provenance.source,
                    run: provenance.in_run(&description),
                },
            )
            .map_err(|e| sidecar_error(&e))?;
        self.changed(&removed, actions::REMOVED);
        Ok(MutantChanged {
            mutant: Mutant {
                removed: !removed.is_live(),
                edits: removed.edits,
                source: removed.source,
                run: removed.run.0,
                changed_at: removed.changed_at,
                ..described
            },
            run,
            created_run: provenance.run.is_none(),
        })
    }

    /// The Mutant a command names, by its identifier or its entry's.
    /// `removed` says whether a removed Mutant named by its identifier
    /// counts.
    fn stored(
        &self,
        id: Option<i64>,
        entry: Option<&str>,
        removed: bool,
    ) -> Result<wreckordbox_sidecar::mutants::Mutant, ServiceError> {
        match (id, entry) {
            (Some(id), None) => self
                .sidecar
                .mutant(id)
                .map_err(|e| sidecar_error(&e))?
                .filter(|mutant| removed || mutant.is_live())
                .ok_or_else(|| no_mutant(id)),
            (None, Some(entry)) => self
                .sidecar
                .mutant_of_entry(entry)
                .map_err(|e| sidecar_error(&e))?
                .ok_or_else(|| {
                    ServiceError::new(
                        codes::MUTANT_NOT_FOUND,
                        format!("playlist entry {entry} is not a Mutant"),
                    )
                }),
            _ => Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "name the Mutant by its id or by its entry, one of the two",
            )),
        }
    }

    /// Stored Mutants as the commands answer them: each with its
    /// playlist's name, its entry's position and its parent's title as the
    /// Library has them now. One read of the Library for them all.
    fn described(
        &self,
        stored: Vec<wreckordbox_sidecar::mutants::Mutant>,
    ) -> Result<Vec<Mutant>, ServiceError> {
        if stored.is_empty() {
            return Ok(Vec::new());
        }
        self.read(|library| {
            let index = library.index()?;
            let mut playlists: HashMap<String, Vec<Entry>> = HashMap::new();
            stored
                .into_iter()
                .map(|mutant| {
                    if !playlists.contains_key(&mutant.playlist_id) {
                        playlists.insert(
                            mutant.playlist_id.clone(),
                            entries::of_playlist(library, &index, &mutant.playlist_id)?,
                        );
                    }
                    let name = entries::playlist_name(&index, &mutant.playlist_id);
                    // A playlist that is gone has no entries to be at.
                    let position = name.as_ref().and_then(|_| {
                        playlists[&mutant.playlist_id]
                            .iter()
                            .find(|entry| entry.id == mutant.entry_id)
                            .map(|entry| entry.position)
                    });
                    let title = index
                        .row_of(&mutant.track_id)
                        .map(|row| index.title.get(row as usize).to_owned())
                        .unwrap_or_default();
                    Ok(Mutant {
                        removed: !mutant.is_live(),
                        id: mutant.id,
                        playlist: PlaylistName {
                            id: mutant.playlist_id,
                            name: name.unwrap_or_default(),
                        },
                        entry_id: mutant.entry_id,
                        position,
                        parent: Parent {
                            id: mutant.track_id,
                            title,
                        },
                        fingerprint: mutant.fingerprint,
                        edits: mutant.edits,
                        source: mutant.source,
                        run: mutant.run.0,
                        created_at: mutant.created_at,
                        changed_at: mutant.changed_at,
                    })
                })
                .collect()
        })
    }

    /// Who is making the change and in which Run, checked before anything
    /// is written: a refused command makes no Run.
    fn provenance(
        &self,
        run: Option<i64>,
        source: Option<&SourceArg>,
        description: Option<&str>,
    ) -> Result<Provenance, ServiceError> {
        let run = match run {
            Some(id) => {
                if description.is_some() {
                    return Err(ServiceError::new(
                        service_codes::BAD_REQUEST,
                        "a description names the Run made for this command; with a Run of your own, describe it when you start it",
                    ));
                }
                let run = self
                    .sidecar
                    .run(RunId(id))
                    .map_err(|e| sidecar_error(&e))?
                    .ok_or_else(|| {
                        ServiceError::new(
                            fact_codes::RUN_NOT_FOUND,
                            format!("there is no Run {id}"),
                        )
                    })?;
                if run.is_finished() {
                    return Err(ServiceError::new(
                        fact_codes::RUN_FINISHED,
                        format!("Run {id} has ended; start another to change more"),
                    ));
                }
                Some(run)
            }
            None => None,
        };
        let source = match (source, &run) {
            (Some(given), _) => crate::facts::source(given)?,
            (None, Some(run)) => run.source.clone(),
            (None, None) => {
                return Err(ServiceError::new(
                    fact_codes::INVALID_SOURCE,
                    "say who is making the change: a source, or a Run started with runs.start",
                ))
            }
        };
        Ok(Provenance {
            source,
            run,
            description: description
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
        })
    }

    fn changed(&self, mutant: &wreckordbox_sidecar::mutants::Mutant, action: &str) {
        self.stems.publish(
            stems::events::SIDECAR_CHANGED,
            json!(MutantEvent {
                track_id: mutant.track_id.clone(),
                kind: KIND.to_owned(),
                fact_id: None,
                stale: Vec::new(),
                mutant_id: mutant.id,
                playlist_id: mutant.playlist_id.clone(),
                entry_id: mutant.entry_id.clone(),
                action: action.to_owned(),
            }),
        );
    }
}

/// Finds the entry a `mutants.create` names, with the Library's lock held.
fn place(library: &OpenLibrary, create: &CreateMutant) -> Result<Place, ServiceError> {
    let index = library.index()?;
    let entry = match (&create.entry, &create.playlist, create.position) {
        (Some(entry), None, None) => entries::find(library, &index, entry)?.ok_or_else(|| {
            ServiceError::new(
                codes::PLAYLIST_ENTRY_NOT_FOUND,
                format!("there is no playlist entry {entry} in the Library"),
            )
        })?,
        (None, Some(playlist), Some(position)) => {
            if entries::playlist_name(&index, playlist).is_none() {
                return Err(ServiceError::new(
                    crate::reading::codes::PLAYLIST_NOT_FOUND,
                    format!("there is no playlist {playlist} in the Library"),
                ));
            }
            let all = entries::of_playlist(library, &index, playlist)?;
            let count = all.len();
            all.into_iter()
                .find(|entry| entry.position == position)
                .ok_or_else(|| {
                    ServiceError::new(
                        codes::PLAYLIST_ENTRY_NOT_FOUND,
                        format!(
                            "playlist {playlist} has no entry at position {position}; it has {count} {}",
                            if count == 1 { "entry" } else { "entries" }
                        ),
                    )
                })?
        }
        _ => {
            return Err(ServiceError::new(
                service_codes::BAD_REQUEST,
                "name the entry by its identifier, or by its playlist and position",
            ))
        }
    };
    let found = stems::parent(library, &entry.track_id)?;
    Ok(Place {
        playlist_name: entries::playlist_name(&index, &entry.playlist_id).unwrap_or_default(),
        parent: found.parent().clone(),
        audio: found.path().to_path_buf(),
        entry,
    })
}

/// The place `found` named, as the Library has it now: the same entry, of
/// the same track in the same playlist, perhaps somewhere else in it.
fn again(library: &OpenLibrary, found: &Place) -> Result<Place, ServiceError> {
    let index = library.index()?;
    let entry = entries::find(library, &index, &found.entry.id)?
        .filter(|entry| {
            entry.playlist_id == found.entry.playlist_id && entry.track_id == found.entry.track_id
        })
        .ok_or_else(|| {
            ServiceError::new(
                codes::PLAYLIST_ENTRY_NOT_FOUND,
                format!(
                    "playlist entry {} left the Library while it was being mutated",
                    found.entry.id
                ),
            )
        })?;
    let parent = stems::parent(library, &entry.track_id)?;
    Ok(Place {
        playlist_name: entries::playlist_name(&index, &entry.playlist_id).unwrap_or_default(),
        parent: parent.parent().clone(),
        audio: parent.path().to_path_buf(),
        entry,
    })
}

fn already(entry: &str, mutant: i64) -> ServiceError {
    ServiceError::new(
        codes::ALREADY_A_MUTANT,
        format!("playlist entry {entry} is already a Mutant (Mutant {mutant})"),
    )
}

fn no_mutant(id: i64) -> ServiceError {
    ServiceError::new(codes::MUTANT_NOT_FOUND, format!("there is no Mutant {id}"))
}

fn sidecar_error(error: &SidecarError) -> ServiceError {
    match error {
        SidecarError::AlreadyMutant { entry_id, mutant } => already(entry_id, *mutant),
        SidecarError::NoMutant(id) => no_mutant(*id),
        SidecarError::NoRun(run) => ServiceError::new(
            fact_codes::RUN_NOT_FOUND,
            format!("there is no Run {}", run.0),
        ),
        SidecarError::RunFinished(run) => ServiceError::new(
            fact_codes::RUN_FINISHED,
            format!("Run {} has ended; start another to change more", run.0),
        ),
        _ => ServiceError::new(
            stems::codes::SIDECAR_FAILED,
            format!("the Sidecar failed: {error}"),
        ),
    }
}
