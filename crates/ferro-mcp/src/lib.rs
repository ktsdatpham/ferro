//! ferro-mcp library: the MCP tool host (a headless [`tools::Server`] with
//! file tools, forwarding to the open app over Ferro Link when linked).
//! The `ferro-mcp` binary serves it over stdio; `ferro-cli` reuses it so
//! scripts run exactly the same dispatch as agents (D-44). [`python`] runs
//! `run_python` scripts for this host and for the app; [`plugins`] loads
//! plugins (ADR 0009) and runs their commands through it.

pub mod plugins;
pub mod python;
pub mod tools;
