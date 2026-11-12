//! The Tag schema (ADR 0006): checking the DJ's vocabulary file, and
//! writing a first one from the MyTags the Library already has.
//!
//! The file is `tag-schema.toml` in the Sidecar's folder, beside the kinds
//! file ([`wreckordbox_sidecar::tag_schema`] describes its format). Each of
//! its dimensions is a kind of Fact with the same name, and a Fact of that
//! kind holds terms' identifiers.
//!
//! Neither command writes to the Library: no `library_changed` is sent and
//! the Library's generation does not move. `schema.convert` records Facts,
//! announced as [`SIDECAR_CHANGED`](crate::stems::events::SIDECAR_CHANGED).
//!
//! The conversion itself is in [`convert`](self::convert).

#![allow(
    clippy::doc_markdown,
    reason = "MyTag is rekordbox's word for the thing, written as a word, not code"
)]

mod convert;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use wreckordbox_sidecar::tag_schema::{DeckTag, DeckView, Problem};
use wreckordbox_sidecar::{Kinds, SidecarError, TagSchema, TAG_SCHEMA_FILE};

pub(crate) use self::convert::my_tags;
pub use self::convert::{
    Conflict, Convert, Converted, ConvertedCategory, Disambiguated, FileState, ImpliedParents,
    OddEntry, Tree, RUN_DESCRIPTION,
};
use crate::protocol::{codes as service_codes, Request, ServiceError};
use crate::stems::{self, Stems};

/// The names of the Tag schema commands.
pub mod commands {
    /// Checks a Tag schema file. Takes a [`Check`](super::Check), answers a
    /// [`Checked`](super::Checked); a file with mistakes is an answer, not
    /// an error.
    pub const SCHEMA_CHECK: &str = "schema.check";
    /// Writes a first Tag schema from the Library's MyTags and records
    /// every tag on a track as a Fact from the DJ. Takes a
    /// [`Convert`](super::Convert), answers a
    /// [`Converted`](super::Converted).
    pub const SCHEMA_CONVERT: &str = "schema.convert";
}

/// The error codes of the Tag schema commands.
pub mod codes {
    /// There is no Tag schema file where one was looked for.
    pub const TAG_SCHEMA_MISSING: &str = "tag_schema_missing";
    /// The Tag schema has a mistake; nothing uses it until it is fixed.
    pub const TAG_SCHEMA_INVALID: &str = "tag_schema_invalid";
    /// There is already a Tag schema, and the conversion was not told to
    /// replace it.
    pub const TAG_SCHEMA_EXISTS: &str = "tag_schema_exists";
    /// There is a Tag schema file and it cannot be read; it is left as it
    /// is.
    pub const TAG_SCHEMA_UNREADABLE: &str = "tag_schema_unreadable";
    /// The conversion was told of a MyTag category the Library does not
    /// have.
    pub const MY_TAG_CATEGORY_NOT_FOUND: &str = "my_tag_category_not_found";
}

/// The arguments of `schema.check`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// The file to check. Left out: the Library's own Tag schema, in the
    /// Sidecar's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
}

/// One dimension, as `schema.check` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionSummary {
    pub name: String,
    /// The kind of Fact that holds its terms: the same name.
    pub kind: String,
    /// How many terms it has.
    pub terms: usize,
    /// The genre tree.
    pub tree: bool,
}

/// What `schema.check` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub file: PathBuf,
    /// True when nothing is wrong with it.
    pub valid: bool,
    /// Everything wrong with it, in the file's order.
    pub problems: Vec<Problem>,
    /// The genre tree first, then the others by name. Empty when the file
    /// is not valid.
    pub dimensions: Vec<DimensionSummary>,
    /// The MyTag categories and MyTags its deck view makes, in the deck's
    /// order. Empty when the file is not valid.
    pub deck: Vec<DeckView>,
}

/// Whether `command` is one of this module's.
pub(crate) fn answers(command: &str) -> bool {
    matches!(command, commands::SCHEMA_CHECK | commands::SCHEMA_CONVERT)
}

/// Answers one of this module's commands. `read` reads the Library's
/// MyTags; it is the only thing here that takes the Library's lock.
pub(crate) fn answer(
    stems: &Arc<Stems>,
    request: &Request,
    read: &dyn Fn() -> Result<convert::MyTags, ServiceError>,
) -> Result<Value, ServiceError> {
    let answer = match request.command.as_str() {
        commands::SCHEMA_CHECK => serde_json::to_value(check(stems, &arguments(request)?)?),
        _ => serde_json::to_value(convert::convert(stems, &arguments(request)?, read)?),
    };
    answer.map_err(|e| ServiceError::new(service_codes::SERVICE_FAILED, e.to_string()))
}

fn arguments<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, ServiceError> {
    let args = if request.args.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        request.args.clone()
    };
    serde_json::from_value(args).map_err(|e| {
        ServiceError::new(
            service_codes::BAD_REQUEST,
            format!("bad arguments to {}: {e}", request.command),
        )
    })
}

/// Where the Library's own Tag schema is, or would be.
pub(crate) fn file(stems: &Stems) -> Result<PathBuf, ServiceError> {
    Ok(stems.sidecar()?.dir().join(TAG_SCHEMA_FILE))
}

fn check(stems: &Stems, check: &Check) -> Result<Checked, ServiceError> {
    let own = file(stems)?;
    let file = check.file.clone().unwrap_or_else(|| own.clone());
    if !file.is_file() {
        let hint = if file == own {
            ". Write one, or make a first one from the Library's MyTags with `wreckordbox schema convert`"
        } else {
            ""
        };
        return Err(ServiceError::new(
            codes::TAG_SCHEMA_MISSING,
            format!("there is no Tag schema at {}{hint}", file.display()),
        ));
    }
    let declared = declared_kinds(stems)?;
    let read = TagSchema::load(&file).and_then(|schema| {
        declared.with_schema(&schema)?;
        Ok(schema)
    });
    Ok(match read {
        Ok(schema) => Checked {
            file,
            valid: true,
            problems: Vec::new(),
            dimensions: schema
                .dimensions
                .iter()
                .map(|dimension| DimensionSummary {
                    name: dimension.name.clone(),
                    kind: dimension.name.clone(),
                    terms: dimension.terms.len(),
                    tree: dimension.tree,
                })
                .collect(),
            deck: schema.deck_view(),
        },
        Err(problems) => Checked {
            file,
            valid: false,
            problems,
            dimensions: Vec::new(),
            deck: Vec::new(),
        },
    })
}

/// The kinds declared without the Tag schema: the built-in ones and the
/// kinds file's.
pub(crate) fn declared_kinds(stems: &Stems) -> Result<Kinds, ServiceError> {
    Kinds::load_declared(stems.sidecar()?.dir()).map_err(|e| sidecar_error(&e))
}

pub(crate) fn sidecar_error(error: &SidecarError) -> ServiceError {
    match error {
        SidecarError::KindsFile { .. } => {
            ServiceError::new(crate::facts::codes::KINDS_FILE_INVALID, error.to_string())
        }
        SidecarError::TagSchema { .. } => {
            ServiceError::new(codes::TAG_SCHEMA_INVALID, error.to_string())
        }
        _ => ServiceError::new(
            stems::codes::SIDECAR_FAILED,
            format!("the Sidecar failed: {error}"),
        ),
    }
}
