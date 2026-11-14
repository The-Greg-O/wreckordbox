//! Explicit local Ledger spill inspection and recovery.
use clap::Subcommand;
use serde_json::{json, Value};
use wreckordbox_service::{Client, ServiceError, facts::SourceArg};

#[derive(Subcommand)]
pub enum Commands {
    /// List retained accepted-write spills. Does not recover or delete anything.
    List {
        #[arg(long)] limit: Option<usize>,
        #[arg(long)] offset: Option<usize>,
    },
    /// Inspect original attribution, files and a page of accepted results.
    Show {
        id: String,
        #[arg(long)] limit: Option<usize>,
        #[arg(long)] offset: Option<usize>,
    },
    /// Replay accepted records. For a lost manifest, supply its original JSON.
    Replay {
        id: String,
        #[arg(long, value_name="JSON_FILE")] manifest: Option<std::path::PathBuf>,
    },
    /// Explicitly discard accepted-write records, recording the DJ's decision.
    Discard {
        id: String,
        #[arg(long,value_parser=crate::facts::parse_source)] source: SourceArg,
        #[arg(long)] reason: String,
    },
}
pub fn run(client: &mut Client, command: &Commands) -> Result<String, ServiceError> {
    let (name,args) = match command {
        Commands::List {limit,offset} => ("runs.spills.list",json!({"limit":limit,"offset":offset})),
        Commands::Show {id,limit,offset} => ("runs.spills.show",json!({"id":id,"limit":limit,"offset":offset})),
        Commands::Replay {id,manifest} => {
            let manifest:Option<Value> = manifest.as_ref().map(|path| {
                let bytes = std::fs::read(path).map_err(|e|bad(format!("{}: {e}",path.display())))?;
                serde_json::from_slice(&bytes).map_err(bad)
            }).transpose()?;
            ("runs.spills.replay",json!({"id":id,"manifest":manifest}))
        },
        Commands::Discard {id,source,reason} => ("runs.spills.discard",json!({"id":id,"source":source,"reason":reason})),
    };
    let result:Value = client.call(name,args)?;
    serde_json::to_string_pretty(&result).map_err(bad)
}
fn bad(error: impl ToString) -> ServiceError {
    ServiceError::new("bad_request", error.to_string())
}
