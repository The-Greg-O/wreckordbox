//! A Model Context Protocol server for an agent outside wreckordbox: a thin
//! client of the Library service, exactly as the `wreckordbox` command is.
//!
//! It has no Library access of its own and uses no network. Each tool is
//! one service command: a tool call connects to the service for this
//! machine's Library (starting one with the `wreckordbox` command beside
//! this program when none is running), sends that command with the tool's
//! arguments, and answers the service's result. So every change goes
//! through the service's single edit path, Library Protection, the Ledger
//! and the hosting window's live update, as a CLI change does.
//!
//! The JSON-RPC subset MCP needs (`initialize`, `ping`, `tools/list`,
//! `tools/call`, notifications) is implemented here by hand, over
//! newline-delimited JSON on stdio: the service client is synchronous and
//! three methods do not justify an async runtime and an SDK.
//!
//! The commands that are the DJ's own decision (unlocking a Stick,
//! clearing a Projection barrier, accepting, rejecting or reopening a
//! waiting Fact, discarding a Ledger spill) are not tools; see
//! `docs/agents/guide.md`.

mod rpc;
mod schema;
pub mod tools;

pub use rpc::serve;
