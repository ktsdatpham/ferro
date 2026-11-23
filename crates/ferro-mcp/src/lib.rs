//! ferro-mcp library: the MCP tool host (a headless [`tools::Server`] with
//! file tools, forwarding to the open app over Ferro Link when linked).
//! The `ferro-mcp` binary serves it over stdio; `ferro-cli` reuses it so
//! scripts run exactly the same dispatch as agents (D-44).

pub mod tools;
