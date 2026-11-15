//! `wreckordbox-mcp`: the Model Context Protocol server, over stdio.
//!
//! An MCP client (such as Claude Code) starts it and talks JSON-RPC on its
//! standard input and output. Nothing else is ever written to standard
//! output.

fn main() {
    let stdin = std::io::stdin();
    wreckordbox_mcp::serve(stdin.lock(), std::io::stdout());
}
