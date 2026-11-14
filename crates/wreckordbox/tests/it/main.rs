//! Shared integration-test binary. See docs/development/integration-tests.md.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

// Keep the helper at its original path so new top-level tests can still use
// `mod common;`. This binary compiles it once for all of its modules.
#[path = "../common/mod.rs"]
mod common;

mod backup;
mod decisions;
mod dj_facts;
mod dj_facts_review;
mod fact_attachment;
mod facts;
mod intake;
mod intake_tags;
mod intensity;
mod ledger;
mod ledger_failures;
mod library_protection;
mod library_reading;
mod mutant_edits;
mod mutants;
mod mutants_follow;
mod mytag_edits;
mod projection;
mod run_status;
mod standing_facts;
mod status;
mod stems_separate;
mod stems_write;
mod stick;
mod stick_library;
mod stick_load;
mod tag_schema;
mod track_filter;
mod tracks_list;
mod tracks_query;
mod tracks_rate;
mod tracks_show;
mod unsigned_mytag_ids;
