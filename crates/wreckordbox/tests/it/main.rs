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
mod fact_producers_hold;
mod facts;
mod intake;
mod intake_tags;
mod intensity;
mod ledger;
mod large_run_backup;
mod ledger_failures;
mod library_protection;
mod library_reading;
mod mutant_edits;
mod mutants;
mod mutants_follow;
mod mytag_edits;
mod mytag_projection;
mod projection;
mod projection_barriers;
mod projection_failures;
mod projection_marks;
mod review;
mod review_races;
mod review_view_reads;
mod run_status;
mod sidecar_on_stick;
mod standing_facts;
mod status;
mod stems_separate;
mod stems_show_read_only;
mod stems_write;
mod stick;
mod stick_library;
mod stick_load;
mod stick_preview_fingerprints;
mod tag_schema;
mod track_filter;
mod tracks_list;
mod tracks_query;
mod tracks_rate;
mod tracks_show;
mod unsigned_mytag_ids;
