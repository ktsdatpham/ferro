# 0009. Plugins are open: the user's rights, simple to write

- **Status:** Accepted (2026-11-24, owner decision)
- **Supersedes:** D-51 (distributed plugins in a WASM sandbox)
- **Affects:** ADR 0005 (`run_python`), ADR 0008, D-44, D-65

## Context

D-51 put distributed plugins in a WASM sandbox with capability permissions
and refused plugins from `run_python`. The owner decided otherwise: plugins
the user writes or installs should work like Revit and SketchUp plugins,
with no artificial limits, and be simple enough that a non-programmer can
make one. Strict review is needed only when a plugin is submitted to become
a built-in Ferro feature.

## Decision

1. **Rights.** A plugin runs with the user's rights, like a Revit add-in or
   a SketchUp extension: it may call every tool, run Python, read and write
   files and use the network. It still changes the model only through
   commands (undo, journal, pins and proposals work as for anyone).
2. **Simple to make.** A plugin is a folder with a small manifest (name,
   version, author, commands and buttons it adds) and Python code using the
   Ferro SDK (ADR 0005). Dropping the folder into the plugins folder loads
   it; the app can create a new plugin from a template, and an agent can
   write one from a description. Hot reload while developing.
3. **No permission prompts beyond the first install.** Installing a plugin
   from someone else shows who made it and what it adds once, as Revit and
   SketchUp do; there are no per-call permission dialogs.
4. **Promotion to a feature is gated.** A plugin submitted to become part of
   Ferro goes through the repository gate: AGENTS.md rules, `ferro-check`,
   the corpus, review, and verification by someone other than the author.
5. **WASM** stays an option for running plugins inside the web viewer later;
   it is no longer required on the desktop.

## Consequences

- `run_python` no longer refuses plugins (`ferro.python.not_allowed` stays
  for "only the client that started a run may cancel it").
- A plugin is trusted like any program the user installs; the install
  screen says so plainly.
- New task: plugin format, loader, template and hot reload (M5-28).
