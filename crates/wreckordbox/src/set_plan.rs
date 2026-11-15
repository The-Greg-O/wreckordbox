//! `wreckordbox set-plan`: check a Recipe, build a Set plan from one and a
//! pool of tracks, and read the plans kept in the Sidecar (wbx-39).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde_json::json;
use wreckordbox_service::facts::SourceArg;
use wreckordbox_service::set_plans::{commands, Build, Built, Check, Checked, List, SetPlan, Show};
use wreckordbox_service::tracks::TrackSelection;
use wreckordbox_service::{codes, Client, ServiceError};

#[derive(Subcommand)]
pub enum SetPlanCommands {
    /// Check a Recipe file, naming each mistake and its line. Fails when
    /// the Recipe cannot be used.
    Check {
        #[arg(value_name = "RECIPE")]
        file: PathBuf,
    },
    /// Build a Set plan from a Recipe and a pool of tracks, and keep it in
    /// the Sidecar.
    ///
    /// The plan is the ordered tracks and, for each transition, its score
    /// and each criterion's part of it with the reason. The same Recipe,
    /// pool and Facts always give the same plan. Tracks the Recipe cannot
    /// use (no Intensity, or no tempo or key under a limit) are named.
    /// When no plan keeps the Recipe's limits, it says which criterion is
    /// in the way, keeps nothing and fails. The Library is not changed.
    Build(BuildArgs),
    /// Show a kept Set plan; with `--json`, in full.
    Show {
        #[arg(value_name = "SET_PLAN_ID")]
        id: i64,
    },
    /// List the kept Set plans, newest first.
    List,
    /// Print the set planning guide for agents: the musical basis of
    /// harmonic mixing, tempo and energy flow, and how to turn it into a
    /// Recipe. Read it before writing a Recipe. With `--json`, in sections.
    /// Needs no service and no Library.
    Guide,
}

#[derive(Args)]
pub struct BuildArgs {
    /// The Recipe file.
    #[arg(value_name = "RECIPE")]
    file: PathBuf,
    /// Tracks of the pool by identifier: the ID column of `tracks list`.
    #[arg(value_name = "TRACK_ID")]
    ids: Vec<String>,
    /// Every track in this playlist, by identifier or by name.
    #[arg(long, value_name = "PLAYLIST")]
    playlist: Option<String>,
    /// Every track matching this text, as the window's search box would.
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,
    /// Every track whose genre contains this text.
    #[arg(long, value_name = "TEXT")]
    genre: Option<String>,
    /// Every track with this Energy (the star rating), or a range `3..5`.
    #[arg(long, value_name = "ENERGY")]
    energy: Option<String>,
    /// Every track in the Library.
    #[arg(long)]
    all: bool,
    /// Who builds it: `dj`, `agent:NAME` or `tool:NAME`.
    #[arg(long, value_name = "SOURCE", default_value = "dj", value_parser = crate::facts::parse_source)]
    source: SourceArg,
}

fn read(file: &Path) -> Result<(String, String), ServiceError> {
    let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let text = std::fs::read_to_string(&absolute).map_err(|e| {
        ServiceError::new(
            codes::BAD_REQUEST,
            format!("cannot read the Recipe {}: {e}", absolute.display()),
        )
    })?;
    Ok((text, absolute.display().to_string()))
}

/// Runs one `set-plan` subcommand: what to print, and whether it went well.
pub fn run(
    client: &mut Client,
    as_json: bool,
    command: &SetPlanCommands,
) -> Result<(String, bool), ServiceError> {
    match command {
        SetPlanCommands::Guide => Ok((crate::set_plan_guide::render(as_json), true)),
        SetPlanCommands::Check { file } => {
            let (recipe, _) = read(file)?;
            let checked: Checked =
                client.call(commands::SETPLANS_CHECK, json!(Check { recipe }))?;
            let text = if as_json {
                json!(checked).to_string()
            } else if checked.valid {
                format!("The Recipe {} is valid.", file.display())
            } else {
                let mut out = format!("The Recipe {} has mistakes:", file.display());
                for problem in &checked.problems {
                    let _ = write!(out, "\n  {problem}");
                }
                out
            };
            Ok((text, checked.valid))
        }
        SetPlanCommands::Build(args) => {
            let (recipe, recipe_file) = read(&args.file)?;
            let selects = args.all
                || args.playlist.is_some()
                || args.search.is_some()
                || args.genre.is_some()
                || args.energy.is_some();
            if args.ids.is_empty() && !selects {
                return Err(ServiceError::new(
                    codes::BAD_REQUEST,
                    "name the pool: track ids, --playlist, --search, --genre, --energy, or --all",
                ));
            }
            let asked = Build {
                recipe,
                recipe_file: Some(recipe_file),
                ids: args.ids.clone(),
                selection: selects.then(|| TrackSelection {
                    search: args.search.clone().unwrap_or_default(),
                    playlist: args.playlist.clone(),
                    genre: args.genre.clone(),
                    energy: args.energy.clone(),
                    ..TrackSelection::default()
                }),
                source: args.source.clone(),
            };
            let built: Built = client.call(commands::SETPLANS_BUILD, json!(asked))?;
            let text = if as_json {
                json!(built).to_string()
            } else if let Some(plan) = &built.set_plan {
                plan_text(plan)
            } else if let Some(why) = &built.unsatisfiable {
                let mut out = why.message.clone();
                for out_track in &why.left_out {
                    let _ = write!(out, "\n  {}", out_track.message);
                }
                out
            } else {
                String::new()
            };
            Ok((text, built.built))
        }
        SetPlanCommands::Show { id } => {
            let plan: SetPlan = client.call(commands::SETPLANS_SHOW, json!(Show { id: *id }))?;
            Ok((
                if as_json {
                    json!(plan).to_string()
                } else {
                    plan_text(&plan)
                },
                true,
            ))
        }
        SetPlanCommands::List => {
            let list: List = client.call(commands::SETPLANS_LIST, json!({}))?;
            let text = if as_json {
                json!(list).to_string()
            } else if list.set_plans.is_empty() {
                "No Set plans.".to_owned()
            } else {
                let mut out = "PLAN  TRACKS  SCORE  RUN  NAME".to_owned();
                for plan in &list.set_plans {
                    let _ = write!(
                        out,
                        "\n{:<5} {:<7} {:<6} {:<4} {}",
                        plan.id,
                        plan.tracks,
                        plan.score.map_or_else(String::new, |s| format!("{s:.2}")),
                        plan.run,
                        plan.name
                    );
                }
                out
            };
            Ok((text, true))
        }
    }
}

/// A kept plan in words: the tracks in order, each transition's score and
/// reasons beneath the track it leads to, and what was left out.
fn plan_text(kept: &SetPlan) -> String {
    let plan = &kept.plan;
    let mut out = format!(
        "Set plan {} from the Recipe \"{}\" (Run {}): {} tracks, score {}.",
        kept.id,
        kept.name,
        kept.run,
        plan["tracks"].as_array().map_or(0, Vec::len),
        plan["score"]
    );
    let reasons = |criteria: &serde_json::Value, out: &mut String| {
        for c in criteria.as_array().into_iter().flatten() {
            let _ = write!(
                out,
                "\n      {:<10} {:>7}  {}",
                c["criterion"].as_str().unwrap_or_default(),
                c["score"],
                c["reason"].as_str().unwrap_or_default()
            );
        }
    };
    let transitions = plan["transitions"].as_array().cloned().unwrap_or_default();
    for (i, track) in plan["tracks"].as_array().into_iter().flatten().enumerate() {
        let bpm = track["bpm"]
            .as_f64()
            .map_or_else(String::new, |b| format!("{b:.2}"));
        let _ = write!(
            out,
            "\n{:>3}. {} \"{}\"  {}  {}",
            track["position"],
            track["id"].as_str().unwrap_or_default(),
            track["title"].as_str().unwrap_or_default(),
            bpm,
            track["key"].as_str().unwrap_or_default()
        );
        if i == 0 {
            reasons(&plan["opening"], &mut out);
        } else if let Some(t) = transitions.get(i - 1) {
            let _ = write!(out, "\n      transition score {}", t["score"]);
            reasons(&t["criteria"], &mut out);
        }
    }
    for left in plan["left_out"].as_array().into_iter().flatten() {
        let _ = write!(
            out,
            "\nLeft out: {}",
            left["message"].as_str().unwrap_or_default()
        );
    }
    out
}
