//! The Sidecar (ADR 0001, ADR 0004): wreckordbox's own store beside the
//! Library, holding what rekordbox has no place for. It is a folder with a
//! small database of its own and the audio of the Stems; nothing here ever
//! touches a rekordbox-format file.
//!
//! The store is general: it holds [`Fact`]s. A Fact attaches to the
//! Library's track, with the [`Fingerprint`] of the track's audio recorded
//! alongside, and has a kind, a value, a [`Source`], a confidence, the Run
//! that made it and a time. Each [`Kind`] is declared a Judgement (follows
//! the music) or a Measurement (tied to the exact audio, stale once it
//! changes). [`stems`] is the first kind, a Measurement.
//!
//! - [`Sidecar::open`] opens (or makes) the store in a folder.
//! - [`Sidecar::begin_run`], [`Sidecar::record`], [`Sidecar::facts`] and
//!   [`Sidecar::mark_stale`] are the general interface a new kind uses.
//! - [`Kinds`] is the declared kinds: the built-in ones and the DJ's own
//!   in the Sidecar folder's [`KINDS_FILE`], each with the type of its
//!   value. [`Sidecar::record`] itself takes any kind; whoever records a
//!   Fact for someone else checks it against [`Kinds`] first.
//! - [`TagSchema`] is the DJ's vocabulary file in the same folder: the
//!   genre tree, the other dimensions and the deck view. Each dimension is
//!   a kind of Fact ([`Kinds::with_schema`]).
//! - [`Sidecar::record_stems`] and [`Sidecar::stems`] are the Stems kind.
//! - [`decisions`] is the Decisions: judgements between fixed candidates
//!   and the answers Deciders gave them. Not Facts, in tables of their own.
//! - [`mutants`] is the Mutants: a playlist entry's own edited version of a
//!   track. Not Facts, in tables of their own, kept by the same rules.
//! - [`fingerprint_file`] fingerprints a track's audio.
//!
//! - [`ledger`] is the Ledger's table: the append-only record of every
//!   change to the Library and to the Sidecar, read back by Run.
//!
//! There is no Unwind or Projection here. A Fact is never changed or
//! deleted once recorded, only marked stale or superseded, and carries
//! its Run; a Ledger entry is never changed at all.
//!
//! No dependency on the desktop framework, the Library service or any
//! rekordbox-format crate.

mod active_runs;
pub mod attachments;
mod audio;
pub mod decisions;
mod facts_snapshot;
mod fingerprint;
mod kinds;
pub mod ledger;
pub mod mutant_follow;
pub mod mutants;
pub mod precedence;
pub mod projection;
pub mod projection_barriers;
pub mod stems;
mod store;
pub mod tag_schema;

use std::path::{Path, PathBuf};

pub use active_runs::{ActiveRun, ActiveRunsReader};
pub use audio::{audio_info, AudioInfo};
pub use facts_snapshot::{FactsReader, FactsSnapshot};
pub use fingerprint::{fingerprint_cached, fingerprint_file, known_fingerprint, Fingerprint};
pub use kinds::{Declaration, Kinds, ValueType, KINDS_FILE};
pub use store::{
    Fact, Kind, Nature, NewFact, Run, RunId, Sidecar, Source, SourceKind, SCHEMA_VERSION,
};
pub use tag_schema::{TagSchema, TAG_SCHEMA_FILE};

/// The Sidecar's folder when nothing says otherwise: beside the Library's
/// `master.db`, in the same folder.
pub const FOLDER_NAME: &str = "wreckordbox-sidecar";

/// Where the Sidecar of the Library at `master_db` lives by default:
/// [`FOLDER_NAME`] in the folder that holds `master.db`.
#[must_use]
pub fn default_dir(master_db: &Path) -> PathBuf {
    master_db
        .parent()
        .map_or_else(|| PathBuf::from(FOLDER_NAME), |dir| dir.join(FOLDER_NAME))
}

/// Why the Sidecar refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("{what} {}: {source}", path.display())]
    Io {
        what: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the Sidecar's database failed: {0}")]
    Db(#[from] rusqlite::Error),
    /// The store was written by a newer wreckordbox.
    #[error("the Sidecar at {} is version {found}; this wreckordbox reads up to version {supported}", path.display())]
    Newer {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    /// A kind already declared the other way.
    #[error("{kind} is already declared a {declared}, so it cannot be recorded as a {asked}")]
    KindConflict {
        kind: String,
        declared: Nature,
        asked: Nature,
    },
    /// The kinds file does not parse, or declares a kind badly.
    #[error("the kinds file {} is refused: {why}", path.display())]
    KindsFile { path: PathBuf, why: String },
    /// The Tag schema does not parse, or has a mistake.
    #[error("the Tag schema {} is refused: {why}", path.display())]
    TagSchema { path: PathBuf, why: String },
    /// A Fact that cannot be recorded as it stands.
    #[error("{0}")]
    Invalid(String),
    /// An audio file whose length or format cannot be read.
    #[error("cannot read {} as audio: {why}", path.display())]
    Audio { path: PathBuf, why: String },
    /// A Stem that is not exactly as long as its parent.
    #[error("the {role} Stem is {samples} samples long and its parent is {parent}; a Stem must be exactly as long as its parent")]
    StemLength {
        role: stems::Role,
        samples: u64,
        parent: u64,
    },
    /// A set that is not exactly one Stem for each of the four roles.
    #[error("a Stem set is one Stem each for vocals, other, bass and drums: {0}")]
    StemSet(String),
    /// No Decision with that identifier.
    #[error("there is no Decision {0}")]
    NoDecision(i64),
    /// A Decision that cannot be asked as it stands: no question, or a
    /// context that is not an object.
    #[error("{0}")]
    Decision(String),
    /// Candidates a Decision cannot be asked with.
    #[error("{0}")]
    Candidates(String),
    /// An answer that names none of its Decision's candidates.
    #[error("\"{choice}\" is not a candidate of Decision {decision}; its candidates are: {}", candidates.join(", "))]
    NotACandidate {
        decision: i64,
        choice: String,
        candidates: Vec<String>,
    },
    /// A probability that is not one, or probabilities that do not agree.
    #[error("{0}")]
    Probability(String),
    /// A playlist entry that already has a Mutant.
    #[error("playlist entry {entry_id} is already a Mutant (Mutant {mutant})")]
    AlreadyMutant { entry_id: String, mutant: i64 },
    /// No live Mutant with that identifier.
    #[error("there is no Mutant {0}")]
    NoMutant(i64),
    /// A change named a Run the store does not have.
    #[error("there is no Run {}", .0 .0)]
    NoRun(RunId),
    /// A change named a Run that has ended.
    #[error("Run {} has ended", .0 .0)]
    RunFinished(RunId),
}

pub type Result<T> = std::result::Result<T, SidecarError>;

pub(crate) fn io<'a>(
    what: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> SidecarError + 'a {
    move |source| SidecarError::Io {
        what,
        path: path.to_path_buf(),
        source,
    }
}

pub mod dj_facts;

pub mod bulk;
