//! Environment-independent service integration tests in one binary.
//! Process-isolation exceptions are documented in docs/development/integration-tests.md.

mod decisions;
mod in_process;
mod intake;
mod ledger_entry_reorder;
mod one_edit_gate;
mod stop_closes_sidecar;
mod tag_panel;
