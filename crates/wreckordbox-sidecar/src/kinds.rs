//! The declared kinds of Fact: for each, whether it is a Judgement or a
//! Measurement, and the type of its value.
//!
//! A kind is declared in one of two places:
//!
//! - here, for the kinds wreckordbox itself knows ([`Kinds::built_in`]);
//! - in [`KINDS_FILE`], a plain-text file in the Sidecar's folder that the
//!   DJ edits and versions (ADR 0006), for any other:
//!
//! ```toml
//! [kinds.danceability]
//! nature = "measurement"
//! value = "number"
//! min = 0
//! max = 1
//! description = "How steady the pulse is."
//!
//! [kinds.era]
//! nature = "judgement"
//! value = "text"
//! choices = ["90s", "00s", "10s", "20s"]
//! ```
//!
//! The file adds kinds; it cannot declare a built-in one again. It is read
//! each time it is needed, so a new kind is usable as soon as it is saved.
//!
//! The value types are few and plain ([`ValueType`]); `choices` narrows
//! text, and `min` and `max` narrow a number.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{Kind, Nature};
use crate::tag_schema::{Problem, TagSchema};
use crate::{Result, SidecarError};

/// The file in the Sidecar's folder that declares kinds beyond the
/// built-in ones.
pub const KINDS_FILE: &str = "kinds.toml";

/// The type of a kind's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    /// One piece of text, not empty: `"Tech House"`.
    Text,
    /// A list of texts, for a question with several answers at once:
    /// `["Dark", "Hypnotic"]`. An empty list says "none of them".
    Texts,
    /// A whole number: `4`.
    Integer,
    /// Any number: `0.62`.
    Number,
    /// `true` or `false`.
    Boolean,
    /// A JSON object whose shape is the kind's own business (Stems,
    /// Intensity).
    Object,
}

impl ValueType {
    /// The word the kinds file and the command line use for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Texts => "texts",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Object => "object",
        }
    }
}

/// One declared kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Declaration {
    pub name: String,
    pub nature: Nature,
    /// The type of its value.
    pub value: ValueType,
    /// For text and texts: the only values allowed. Empty allows any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// The choices are the whole vocabulary even when there are none: a
    /// dimension of the Tag schema, whose values are its terms and nothing
    /// else. A dimension with no terms takes no term.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    /// For a number: the least and the most it may be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub description: String,
    /// Declared by wreckordbox itself, not in the kinds file.
    #[serde(default)]
    pub built_in: bool,
    /// Recorded only by the command that makes it (Stems by a separation),
    /// never as a general Fact.
    #[serde(default)]
    pub reserved: bool,
}

impl Declaration {
    /// The kind as the store records it.
    #[must_use]
    pub fn kind(&self) -> Kind<'_> {
        Kind {
            name: &self.name,
            nature: self.nature,
        }
    }

    /// Whether `value` is a value of this kind; if not, why not, as a
    /// sentence that starts with the kind's name.
    pub fn check(&self, value: &Value) -> std::result::Result<(), String> {
        let name = &self.name;
        let wrong = || {
            Err(format!(
                "{name} takes {}, not {}",
                self.takes(),
                describe(value)
            ))
        };
        match self.value {
            ValueType::Text => match value.as_str() {
                Some(text) => self.check_text(text),
                None => wrong(),
            },
            ValueType::Texts => {
                let Some(items) = value.as_array() else {
                    return wrong();
                };
                for item in items {
                    match item.as_str() {
                        Some(text) => self.check_text(text)?,
                        None => return wrong(),
                    }
                }
                Ok(())
            }
            ValueType::Integer => {
                if value.is_i64() || value.is_u64() {
                    self.check_range(value.as_f64().unwrap_or(f64::NAN))
                } else {
                    wrong()
                }
            }
            ValueType::Number => match value.as_f64() {
                Some(number) if value.is_number() && number.is_finite() => self.check_range(number),
                _ => wrong(),
            },
            ValueType::Boolean if value.is_boolean() => Ok(()),
            ValueType::Object if value.is_object() => Ok(()),
            ValueType::Boolean | ValueType::Object => wrong(),
        }
    }

    /// `value` as this kind holds it now. A kind that takes a list of texts
    /// may have Facts from when it took one text (genre did): that text is
    /// a list of one. Anything else is as it is.
    #[must_use]
    pub fn normalise(&self, value: Value) -> Value {
        match value {
            Value::String(text) if self.value == ValueType::Texts => {
                Value::Array(vec![Value::String(text)])
            }
            value => value,
        }
    }

    /// Whether the Tag schema may make this kind one of its dimensions: a
    /// Judgement that takes text or a list of texts and has no fixed
    /// values of its own.
    #[must_use]
    pub fn may_be_dimension(&self) -> bool {
        self.nature == Nature::Judgement
            && matches!(self.value, ValueType::Text | ValueType::Texts)
            && !self.reserved
            && (self.closed || self.choices.is_empty())
    }

    /// What a value of this kind is, in words: `text`, `a whole number
    /// from 1 to 5`.
    #[must_use]
    pub fn takes(&self) -> String {
        let range = match (self.min, self.max) {
            (Some(min), Some(max)) => format!(" from {min} to {max}"),
            (Some(min), None) => format!(" of at least {min}"),
            (None, Some(max)) => format!(" of at most {max}"),
            (None, None) => String::new(),
        };
        let choices = if self.choices.is_empty() && self.closed {
            " (the Tag schema gives it no terms)".to_owned()
        } else if self.choices.is_empty() {
            String::new()
        } else {
            format!(" (one of: {})", self.choices.join(", "))
        };
        match self.value {
            ValueType::Text => format!("text{choices}"),
            ValueType::Texts => format!("a list of texts{choices}"),
            ValueType::Integer => format!("a whole number{range}"),
            ValueType::Number => format!("a number{range}"),
            ValueType::Boolean => "true or false".to_owned(),
            ValueType::Object => "a JSON object".to_owned(),
        }
    }

    fn check_text(&self, text: &str) -> std::result::Result<(), String> {
        let name = &self.name;
        if text.trim().is_empty() {
            return Err(format!("{name} takes text, and this is empty"));
        }
        if self.closed && self.choices.is_empty() {
            return Err(format!(
                "{name} has no terms in the Tag schema, so it takes none; not \"{text}\""
            ));
        }
        if !self.choices.is_empty() && !self.choices.iter().any(|choice| choice == text) {
            return Err(format!(
                "{name} is one of: {}; not \"{text}\"",
                self.choices.join(", ")
            ));
        }
        Ok(())
    }

    fn check_range(&self, number: f64) -> std::result::Result<(), String> {
        let below = self.min.is_some_and(|min| number < min);
        let above = self.max.is_some_and(|max| number > max);
        if below || above {
            return Err(format!(
                "{} takes {}, not {number}",
                self.name,
                self.takes()
            ));
        }
        Ok(())
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "true or false",
        Value::Number(number) if number.is_i64() || number.is_u64() => "a whole number",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "a JSON object",
    }
}

/// Every declared kind: the built-in ones, then the kinds file's by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Kinds {
    declared: Vec<Declaration>,
    file: PathBuf,
    /// The Tag schema, when it is there and cannot be used.
    broken_schema: Option<BrokenSchema>,
}

/// A Tag schema that has a mistake.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokenSchema {
    path: PathBuf,
    why: String,
    /// The dimensions it names, when it can be read that far.
    dimensions: Option<Vec<String>>,
}

impl Kinds {
    /// The kinds wreckordbox itself declares.
    #[must_use]
    pub fn built_in() -> Vec<Declaration> {
        let colours = [
            "Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple",
        ];
        let declare = |name: &str, nature, value, description: &str| Declaration {
            name: name.to_owned(),
            nature,
            value,
            choices: Vec::new(),
            closed: false,
            min: None,
            max: None,
            description: description.to_owned(),
            built_in: true,
            reserved: false,
        };
        let judgement =
            |name, value, description| declare(name, Nature::Judgement, value, description);
        vec![
            Declaration {
                reserved: true,
                ..declare(
                    crate::stems::STEMS.name,
                    Nature::Measurement,
                    ValueType::Object,
                    "The track's four Stems. Recorded by a separation.",
                )
            },
            judgement(
                "genre",
                ValueType::Texts,
                "The track's genres: the most specific terms of the genre tree, each implying its parents.",
            ),
            judgement("mood", ValueType::Texts, "The moods the track has."),
            judgement(
                "situation",
                ValueType::Texts,
                "The Situations the track suits, such as Opener or Peak time.",
            ),
            judgement(
                "sound",
                ValueType::Texts,
                "The sound elements that stand out in the track.",
            ),
            judgement("vocals", ValueType::Text, "What vocals the track has."),
            judgement(
                "version",
                ValueType::Text,
                "Which version this is, such as Original Mix or Dub.",
            ),
            Declaration {
                min: Some(1.0),
                max: Some(5.0),
                ..judgement(
                    "energy",
                    ValueType::Integer,
                    "Energy: how intense the track is, 1 to 5. The star rating once projected.",
                )
            },
            Declaration {
                choices: colours.map(str::to_owned).to_vec(),
                ..judgement(
                    "prep_colour",
                    ValueType::Text,
                    "Prep colour: where the track is in preparation, as the colour label that says so.",
                )
            },
            declare(
                "intensity",
                Nature::Measurement,
                ValueType::Object,
                "Intensity: how hard the track hits and how that changes over its length.",
            ),
        ]
    }

    /// Every declared kind: the built-in ones, those of the Sidecar
    /// folder's [`KINDS_FILE`], and the dimensions of its Tag schema
    /// ([`TAG_SCHEMA_FILE`](crate::TAG_SCHEMA_FILE)), if there are such
    /// files.
    ///
    /// A kinds file with a mistake is refused whole
    /// ([`SidecarError::KindsFile`]). A Tag schema with a mistake is not an
    /// error here: its dimensions are left out, and
    /// [`refused_by_schema`](Self::refused_by_schema) says which kinds may
    /// not be recorded until it is fixed. What is stored stays readable.
    pub fn load(sidecar_dir: &Path) -> Result<Self> {
        let mut declared = Self::load_declared(sidecar_dir)?;
        let file = sidecar_dir.join(crate::TAG_SCHEMA_FILE);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(declared),
            Err(e) => {
                declared.broken_schema = Some(BrokenSchema {
                    path: file,
                    why: format!("cannot be read: {e}"),
                    dimensions: None,
                });
                return Ok(declared);
            }
        };
        let read = TagSchema::parse(&text).and_then(|schema| declared.clone().with_schema(&schema));
        Ok(match read {
            Ok(kinds) => kinds,
            Err(problems) => {
                declared.broken_schema = Some(BrokenSchema {
                    path: file,
                    why: problems
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                    dimensions: crate::tag_schema::dimension_names(&text),
                });
                declared
            }
        })
    }

    /// Why a Fact of the kind called `name` may not be recorded just now:
    /// the Tag schema has a mistake and governs that kind. `None` when the
    /// schema is sound or has nothing to say of the kind.
    ///
    /// A schema that can be read as far as its dimensions governs those. A
    /// file that cannot be read that far might govern any kind a schema
    /// could: one not declared elsewhere, or a text Judgement with no fixed
    /// values of its own.
    #[must_use]
    pub fn refused_by_schema(&self, name: &str) -> Option<SidecarError> {
        let broken = self.broken_schema.as_ref()?;
        let governed = match &broken.dimensions {
            Some(dimensions) => dimensions.iter().any(|dimension| dimension == name),
            None => self.get(name).is_none_or(Declaration::may_be_dimension),
        };
        governed.then(|| SidecarError::TagSchema {
            path: broken.path.clone(),
            why: format!(
                "{}. Until it is fixed, no Fact of the kind {name} can be recorded",
                broken.why
            ),
        })
    }

    /// Whether there is a Tag schema that cannot be used.
    #[must_use]
    pub fn schema_is_broken(&self) -> bool {
        self.broken_schema.is_some()
    }

    /// These kinds with the Tag schema's dimensions bound to theirs. A
    /// dimension's name is the name of its kind of Fact:
    ///
    /// - a dimension named after a kind already declared (`genre`, `mood`)
    ///   uses it, and that kind must be a Judgement whose value is text or
    ///   a list of texts, with no fixed values of its own (Prep colour has
    ///   its colours, Energy its range: neither can be a dimension);
    /// - any other dimension declares its kind: a Judgement, a list of
    ///   texts.
    ///
    /// Either way the kind's values are then the identifiers of the
    /// dimension's terms and nothing else; a dimension with no terms takes
    /// none.
    pub fn with_schema(mut self, schema: &TagSchema) -> std::result::Result<Self, Vec<Problem>> {
        let mut problems = Vec::new();
        for dimension in &schema.dimensions {
            let choices: Vec<String> = dimension.terms.iter().map(|term| term.id.clone()).collect();
            let Some(kind) = self
                .declared
                .iter_mut()
                .find(|kind| kind.name == dimension.name)
            else {
                self.declared.push(Declaration {
                    name: dimension.name.clone(),
                    nature: Nature::Judgement,
                    value: ValueType::Texts,
                    choices,
                    closed: true,
                    min: None,
                    max: None,
                    description: if dimension.description.is_empty() {
                        format!("The {} dimension of the Tag schema.", dimension.name)
                    } else {
                        dimension.description.clone()
                    },
                    built_in: false,
                    reserved: false,
                });
                continue;
            };
            if !kind.may_be_dimension() {
                problems.push(Problem {
                    line: dimension.line,
                    message: format!(
                        "the dimension {name} would hold Facts of the kind {name}, which is declared a {} taking {}; a dimension's kind is a Judgement taking text or a list of texts, with no fixed values of its own. Call the dimension something else",
                        kind.nature,
                        kind.takes(),
                        name = dimension.name
                    ),
                });
                continue;
            }
            kind.choices = choices;
            kind.closed = true;
        }
        if problems.is_empty() {
            Ok(self)
        } else {
            // In the file's order, as the schema's own problems are.
            problems.sort_by_key(|problem| problem.line);
            Err(problems)
        }
    }

    /// The built-in kinds and those declared in the Sidecar folder's
    /// [`KINDS_FILE`], if there is one, without the Tag schema's. A file
    /// that does not parse, or that declares a kind badly, is refused whole
    /// ([`SidecarError::KindsFile`]).
    pub fn load_declared(sidecar_dir: &Path) -> Result<Self> {
        let file = sidecar_dir.join(KINDS_FILE);
        let mut declared = Self::built_in();
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(refused(&file, e.to_string())),
        };
        let written: KindsFile =
            toml::from_str(&text).map_err(|e| refused(&file, e.to_string()))?;
        for (name, entry) in written.kinds {
            let acceptable = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !acceptable {
                return Err(refused(
                    &file,
                    format!("a kind's name is lower-case letters, digits and underscores, not \"{name}\""),
                ));
            }
            if declared.iter().any(|kind| kind.name == name) {
                return Err(refused(
                    &file,
                    format!("{name} is a built-in kind and cannot be declared again"),
                ));
            }
            let texts = matches!(entry.value, ValueType::Text | ValueType::Texts);
            let numbers = matches!(entry.value, ValueType::Integer | ValueType::Number);
            if !entry.choices.is_empty() && !texts {
                return Err(refused(
                    &file,
                    format!("{name}: choices are for text and texts"),
                ));
            }
            if (entry.min.is_some() || entry.max.is_some()) && !numbers {
                return Err(refused(
                    &file,
                    format!("{name}: min and max are for integer and number"),
                ));
            }
            if let (Some(min), Some(max)) = (entry.min, entry.max) {
                if min > max {
                    return Err(refused(&file, format!("{name}: min is more than max")));
                }
            }
            declared.push(Declaration {
                name,
                nature: entry.nature,
                value: entry.value,
                choices: entry.choices,
                closed: false,
                min: entry.min,
                max: entry.max,
                description: entry.description,
                built_in: false,
                reserved: false,
            });
        }
        Ok(Self {
            declared,
            file,
            broken_schema: None,
        })
    }

    /// The declaration of the kind called `name`, if it is declared.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Declaration> {
        self.declared.iter().find(|kind| kind.name == name)
    }

    /// Every declaration: built-in ones first, then the file's by name.
    #[must_use]
    pub fn all(&self) -> &[Declaration] {
        &self.declared
    }

    /// Where the kinds file is, or would be.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }
}

fn refused(file: &Path, why: String) -> SidecarError {
    SidecarError::KindsFile {
        path: file.to_path_buf(),
        why,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindsFile {
    #[serde(default)]
    kinds: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    nature: Nature,
    value: ValueType,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    description: String,
}
