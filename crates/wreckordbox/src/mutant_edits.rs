//! CLI patch/clear operations; all validation and derivation is the service's.
use super::Who;
use clap::Subcommand;
use serde_json::{json, Value};
use wreckordbox_service::mutants::{commands, MutantChanged};
use wreckordbox_service::{Client, ServiceError};

#[derive(Subcommand)]
pub enum Commands {
    /// Set supplied edits together. Example: --edits '{"key_shift":1,"tempo":1.04}'.
    /// Stems are vocals, melody, bass, drums. Cues use kind and position_ms.
    Edit {
        #[arg(required_unless_present = "entry", conflicts_with = "entry")]
        id: Option<i64>,
        #[arg(long)]
        entry: Option<String>,
        /// JSON object; omitted fields keep their current edits. Use clear to inherit.
        #[arg(long, value_parser = parse_json)]
        edits: Value,
        #[command(flatten)]
        who: Who,
    },
    /// Clear exactly one edit, inheriting that aspect from the parent again.
    #[command(allow_missing_positional = true)]
    Clear {
        #[arg(required_unless_present = "entry", conflicts_with = "entry")]
        id: Option<i64>,
        #[arg(long)]
        entry: Option<String>,
        #[arg(value_parser = ["stems", "key_shift", "tempo", "cues"])]
        edit: String,
        #[command(flatten)]
        who: Who,
    },
}
pub(super) fn run(
    client: &mut Client,
    as_json: bool,
    args: &Commands,
) -> Result<String, ServiceError> {
    let (command, value) = match args {
        Commands::Edit {
            id,
            entry,
            edits,
            who,
        } => (
            commands::MUTANTS_EDIT,
            json!({"id":id,"entry":entry,"edits":edits,"run":who.run,"source":who.source,"description":who.description}),
        ),
        Commands::Clear {
            id,
            entry,
            edit,
            who,
        } => (
            commands::MUTANTS_CLEAR,
            json!({"id":id,"entry":entry,"edit":edit,"run":who.run,"source":who.source,"description":who.description}),
        ),
    };
    let changed: MutantChanged = client.call(command, value)?;
    if as_json {
        Ok(json!(changed).to_string())
    } else {
        Ok(format!(
            "Mutant {}: {}. Renderable: {}. Run {}.",
            changed.mutant.id,
            changed.mutant.derived.description,
            super::readiness_text(&changed.mutant),
            changed.run.id.0
        ))
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}
