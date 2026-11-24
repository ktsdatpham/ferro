# From plugin to built-in feature

A plugin is yours: it runs with your rights and nobody reviews it (ADR 0009).
When a plugin is useful to everyone, it can become part of Ferro. Only then
does it go through the repository gate, the same as any other change.

## 1. Open a pull request

Follow [`AGENTS.md`](../../AGENTS.md) and
[`CONTRIBUTING.md`](../../CONTRIBUTING.md):

- Pick or add a task in [`docs/tasks/TREE.md`](../tasks/TREE.md) and claim it.
- Describe what the plugin does, who uses it, and link the plugin's source.
- Keep the PR small: one feature (one plugin command or one concept).

## 2. Choose its form

- **Port it to commands and MCP tools** (the usual path) when it changes the
  model in a way agents and other tools should also be able to do: write
  `Command`s in `ferro-core` (pure, deterministic, mm and radians, failures
  with `ferro.<area>.<problem>` codes), one MCP tool per command with the
  same `verb_noun` name, and UI in `ferro-app`. Python then gets the tools
  automatically (`python3 scripts/gen_python.py --refresh`).
- **Keep it as a bundled plugin** when it is a workflow built from existing
  tools (a sequence of tool calls, a report, an import helper). It stays a
  plugin folder shipped with Ferro; its Python must only call SDK functions,
  and its manifest's ids, labels and keys follow the glossary.

Either way: use the glossary terms exactly (`docs/foundation/05-naming-conventions.md`),
propose an ADR for any new concept, verb, unit or dependency, and never
rename a released stable string (tool names, parameter keys, failure codes,
plugin ids and command ids once shipped).

## 3. Pass the checks

- `scripts/ferro-check` passes (lints, formatting, clippy, all tests,
  the headless sample house).
- The corpus passes: `cargo test -p ferro-cli --test corpus`; add a corpus
  script and fingerprint for the new behaviour.
- New algorithms have basic, degenerate and tolerance-edge tests; a bundled
  plugin has a headless test running its command
  (`ferro_mcp::plugins::run_command_blocking`).
- Public items have doc comments (what, units, failure cases).

## 4. Get it verified

Someone other than the author re-runs the checks and the task's acceptance
line, tries it on screen, and marks the task verified as the rules at the
top of the task tree say.
Only then is the feature merged.
