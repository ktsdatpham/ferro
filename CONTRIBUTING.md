# Contributing to Ferro

Thanks for helping! Architects, developers, and architect-developers are all welcome.

## Ways to help

- **Designers / architects:** open an issue describing a workflow that hurts in Revit or SketchUp. Real pain points shape the roadmap.
- **Rust devs:** pick an issue labelled `good first issue`, or a box from [docs/ROADMAP.md](docs/ROADMAP.md).
- **Revit API devs:** the add-in in `revit-addin/` needs DirectShape import (milestone 3).
- **Docs / translation:** English and Vietnamese docs both welcome.

## Dev setup

```bash
rustup update stable
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

CI runs those same checks on Windows, macOS and Linux.

## Ground rules

1. **Geometry and model logic live in `ferro-core` only.** The app, the MCP server and the Revit add-in are thin layers on top.
2. **Every new tool is also an MCP tool.** If a human can do it, an agent should be able to as well.
3. **Units are millimetres internally.** Parse user input with `Length::parse`.
4. **Tests for kernel changes.** Geometry bugs are subtle, so add a test that would have caught yours.
5. Small PRs beat big PRs. Open an issue first for anything large.

## Adding an MCP tool

1. Implement the operation on `Model` in `crates/ferro-core/src/model.rs`, with a test.
2. Describe it in `tools::list()` in `crates/ferro-mcp/src/tools.rs`. Write the description for an AI reader: say what it does and give units.
3. Handle it in `tools::call()`.
4. Add a test in `crates/ferro-mcp/src/main.rs`.

## License

By contributing you agree your work is dual-licensed under MIT or Apache-2.0, like the rest of the project.
