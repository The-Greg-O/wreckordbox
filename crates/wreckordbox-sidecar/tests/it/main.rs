//! Sidecar integration tests sharing one binary and one fixture helper.

#[path = "../common/mod.rs"]
mod common;

mod active_runs;
mod attachment_index;
mod attachment_migration;
mod attachment_scale;
mod decisions;
mod facts;
mod fingerprint;
mod ledger;
mod mutants;
mod stems;
