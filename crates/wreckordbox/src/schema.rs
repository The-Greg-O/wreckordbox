//! `wreckordbox schema`: checking the Tag schema, and writing a first one
//! from the MyTags the Library already has.
#![allow(
    clippy::doc_markdown,
    reason = "these comments are the command's help, where MyTag is a word, not code"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::schema::{commands, Check, Checked, Convert, Converted, FileState};
use wreckordbox_service::{codes, Client, ServiceError};

#[derive(Subcommand)]
pub enum SchemaCommands {
    /// Check a Tag schema and show the MyTag categories its deck view
    /// makes. Fails, naming each mistake and its line, when the file cannot
    /// be used.
    ///
    /// The Tag schema is `tag-schema.toml` in the Sidecar's folder, beside
    /// the Library: the genre tree, the other dimensions, and which of them
    /// the deck shows as MyTag categories.
    Check {
        /// A file to check instead of the Library's own Tag schema.
        #[arg(value_name = "FILE")]
        file: Option<PathBuf>,
    },
    /// Write a first Tag schema from the Library's MyTag categories and
    /// tags, and record every tag on a track as a Fact from the DJ.
    ///
    /// The Library is not changed. The category whose tags are written as
    /// children (`> Name` after their parent) becomes the genre tree; the
    /// others become dimensions as they are. The deck view of the schema
    /// written is exactly the MyTags there are now. Run again, it records
    /// only what has changed, and it leaves a Tag schema that has been
    /// edited alone.
    Convert {
        /// The MyTag category that is the genre tree, by identifier or
        /// name, or `none`. Left out: the one whose tags are written as
        /// children.
        #[arg(long, value_name = "CATEGORY")]
        tree: Option<String>,
        /// Call a category's dimension NAME (lower-case letters, digits and
        /// underscores). The name is the kind of its Facts. Repeatable.
        #[arg(long = "name", value_name = "CATEGORY=NAME")]
        names: Vec<String>,
        /// Write over a Tag schema that is already there.
        #[arg(long)]
        replace: bool,
        /// Show what would be written and recorded; write and record
        /// nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Runs one `schema` subcommand: what to print, and whether it went well.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &SchemaCommands,
) -> Result<(String, bool), ServiceError> {
    match command {
        SchemaCommands::Check { file } => {
            let file = file
                .as_ref()
                .map(|file| std::path::absolute(file).unwrap_or_else(|_| file.clone()));
            let checked: Checked = client.call(commands::SCHEMA_CHECK, json!(Check { file }))?;
            let text = if as_json {
                json!(checked).to_string()
            } else {
                checked_text(&checked)
            };
            Ok((text, checked.valid))
        }
        SchemaCommands::Convert {
            tree,
            names,
            replace,
            dry_run,
        } => {
            let mut named = BTreeMap::new();
            for name in names {
                let Some((category, dimension)) = name.rsplit_once('=') else {
                    return Err(ServiceError::new(
                        codes::BAD_REQUEST,
                        format!("--name takes CATEGORY=NAME, not \"{name}\""),
                    ));
                };
                named.insert(category.to_owned(), dimension.to_owned());
            }
            let converted: Converted = client.call(
                commands::SCHEMA_CONVERT,
                json!(Convert {
                    tree: tree.clone(),
                    names: named,
                    replace: *replace,
                    dry_run: *dry_run,
                }),
            )?;
            let text = if as_json {
                json!(converted).to_string()
            } else {
                converted_text(&converted)
            };
            Ok((text, true))
        }
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn checked_text(checked: &Checked) -> String {
    let file = checked.file.display();
    if !checked.valid {
        let mut text = format!(
            "The Tag schema {file} cannot be used: {}.",
            plural(checked.problems.len(), "problem", "problems")
        );
        for problem in &checked.problems {
            let _ = write!(text, "\n  {problem}");
        }
        return text;
    }
    let mut text =
        format!("The Tag schema {file} is valid.\n\nDimensions (each is a kind of Fact):");
    for dimension in &checked.dimensions {
        let _ = write!(
            text,
            "\n  {}  {}{}",
            dimension.name,
            plural(dimension.terms, "term", "terms"),
            if dimension.tree {
                ", the genre tree"
            } else {
                ""
            }
        );
    }
    if checked.deck.is_empty() {
        text.push_str("\n\nThe deck view shows no dimension: there would be no MyTag categories.");
    }
    for (at, category) in checked.deck.iter().enumerate() {
        let _ = write!(
            text,
            "\n\nMyTag category {}: {} (the dimension {}, {})",
            at + 1,
            category.label,
            category.dimension,
            plural(category.tags.len(), "MyTag", "MyTags")
        );
        for tag in &category.tags {
            let _ = write!(text, "\n  {}", tag.label);
        }
    }
    text
}

fn converted_text(converted: &Converted) -> String {
    let file = converted.file.display();
    let mut text = match converted.file_state {
        FileState::Written => format!("Wrote the Tag schema {file}."),
        FileState::Replaced => format!("Replaced the Tag schema {file}."),
        FileState::Unchanged => {
            format!("The Tag schema {file} is already what this conversion writes.")
        }
        FileState::NotWritten => {
            format!("Dry run: the Tag schema {file} was not written and no Fact was recorded.")
        }
    };
    text.push_str("\n\nMyTag categories and the dimensions they are:");
    for category in &converted.categories {
        let _ = write!(
            text,
            "\n  {}  ->  {} ({}{})",
            category.category,
            category.dimension,
            plural(category.terms, "term", "terms"),
            if category.tree {
                ", the genre tree"
            } else {
                ""
            }
        );
    }
    match &converted.tree {
        Some(tree) => {
            let _ = write!(
                text,
                "\n\nThe genre tree is {}, {}: {} and {}, a child written with \"{}\" before its name.",
                tree.category,
                if tree.told {
                    "as told"
                } else {
                    "the category whose tags are written as children"
                },
                plural(tree.parents, "term at the top", "terms at the top"),
                plural(tree.children, "child", "children"),
                tree.child_prefix
            );
        }
        None => text.push_str("\n\nNo category is a genre tree."),
    }
    if !converted.disambiguated.is_empty() {
        let _ = write!(
            text,
            "\n\n{} lost a marker written to tell it from the same name under another parent. The deck keeps the tag's text:",
            plural(converted.disambiguated.len(), "name", "names")
        );
        for term in &converted.disambiguated {
            let _ = write!(
                text,
                "\n  \"{}\" is {} under {} ({})",
                term.tag, term.name, term.parent, term.id
            );
        }
    }
    if !converted.odd.is_empty() {
        let _ = write!(
            text,
            "\n\n{} kept as a term, to look at:",
            plural(converted.odd.len(), "odd entry", "odd entries")
        );
        for odd in &converted.odd {
            let _ = write!(
                text,
                "\n  {} \"{}\" ({}): {}; on {}",
                odd.category,
                odd.tag,
                odd.id,
                odd.why,
                plural(odd.tracks, "track", "tracks")
            );
        }
    }
    let implied = &converted.implied_parents;
    if implied.with_parent + implied.without_parent > 0 {
        let _ = write!(
            text,
            "\n\nA child implies its parent: {} a track carries a child with its parent's tag, {} without. A Fact names the child only; the deck view says tag_parents = {}.",
            plural(implied.with_parent, "time", "times"),
            plural(implied.without_parent, "time", "times"),
            implied.tag_parents
        );
    }
    let would = converted.file_state == FileState::NotWritten;
    let _ = write!(
        text,
        "\n\n{} on {}.",
        plural(converted.track_tags, "tag", "tags"),
        plural(converted.tracks, "track", "tracks")
    );
    if converted.facts_recorded == 0 {
        let _ = write!(
            text,
            " No Fact to record: {} already there.",
            plural(converted.facts_unchanged, "is", "are")
        );
    } else if would {
        let _ = write!(
            text,
            " Would record {} from the DJ.",
            plural(converted.facts_recorded, "Fact", "Facts")
        );
    } else {
        let _ = write!(
            text,
            " Recorded {} from the DJ",
            plural(converted.facts_recorded, "Fact", "Facts")
        );
        if let Some(run) = &converted.run {
            let _ = write!(text, " in Run {}", run.id.0);
        }
        text.push('.');
        if converted.facts_unchanged > 0 {
            let _ = write!(text, " {} already there.", converted.facts_unchanged);
        }
    }
    if let Some(schema) = &converted.text {
        let _ = write!(text, "\n\n{}", schema.trim_end());
    }
    text
}
