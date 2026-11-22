# Contributing to Ferro

Thanks for helping! Architects, developers, and architect-developers are all welcome.

**Read [docs/foundation/](docs/foundation/README.md) first.** It fixes the units,
coordinate systems, data model, naming and the way the model changes. AI agents:
see [AGENTS.md](AGENTS.md).

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

## Adding a command (and its MCP tool)

1. Add a struct implementing `Command` in `crates/ferro-core/src/ops/commands.rs`
   (`NAME` = `verb_noun`), register it in `any_command!`, and test it.
2. Describe it for AI readers in `tools::list()` in `crates/ferro-mcp/src/tools.rs`
   (the test `every_command_has_a_tool_with_the_same_name` enforces this).
3. If the app needs a tool for it, add a state in `crates/ferro-app/src/tools.rs`
   that previews and commits that command.

## License

Ferro is licensed under MPL-2.0. By contributing you agree your work is licensed under MPL-2.0 too.
Sign off every commit (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/).
