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

## Implementation note (M5-28, 2026-11-24)

- **Format.** `~/.ferro/plugins/<folder>/plugin.json` (`$FERRO_HOME/plugins`
  when set): `id`, `name`, `version`, `author`, optional `description`, and
  `commands: [{id, label, key?, group?, run, timeout_s?}]`; since ADR 0010
  also optional `publisher`, `price` and `entitlement` (M6-20.5: a paid
  plugin loads, its commands need the entitlement). `run` is
  `module:function` (called with `doc` when it takes a parameter; its
  return value is the run's `result`) or a `.py` file in the folder (run
  like a `run_python` script). `key` is a two-letter code; `group` is
  `panel` (default: also a button in the left panel) or `menu` (menu and
  palette only). JSON, not TOML: no new dependency, and the SDK already
  speaks JSON. A `.disabled` file in the folder disables the plugin.
  Guide: `python/README.md`; promotion to a feature: `docs/plugins/PROMOTE.md`.
- **Loader** (`ferro_mcp::plugins`, outside `ferro-core`): never fails; a
  plugin with a missing or invalid manifest, a missing entry file or an id
  another folder already uses is listed as broken with every problem found.
  Built-in keys always win; a key two plugins want goes to the first by
  folder name; each refusal is shown.
- **Running.** A command is a `run_python` run with generated source
  (`plugins::command_source`) that makes the plugin folder the current
  folder and puts it on `sys.path` after the SDK. Same host, so one undo
  step (named after the label), all or nothing, the Stop button, Esc twice
  and the time limit work as for scripts. The journal actor is the plugin
  (`kind: plugin`, its id and name); it commits like the user. The app
  refuses to start one while another script runs or an AI proposal is open
  (its changes would join the agent's proposal).
- **Selection and input.** New Ferro Link request `get_user_input {prompt,
  default?}` → `{value: string | null}` (SDK `ferro.ask`), only from a
  running script; the time spent in the dialog does not count against the
  time limit. `doc.selection()` returns the selected ids (`doc.selection`
  stays the list of elements). Scripts reach `get_selection` /
  `select_elements` in the app (they were refused before: the run answered
  them with "needs the Ferro app"). Headless: the default, and an empty
  selection. Stable names (chapter 6 §6.2).
- **Hot reload.** The app polls the plugins folder once a second (path,
  size and modification time of every file; no file-watching dependency)
  and reloads on any change; Plugins > Reload plugins does the same.
- **Install.** Plugins > Install from file… (a `.zip`), Install from
  folder…, or a `.zip` dropped on the window: unpacked into a hidden
  staging folder (entry paths that leave it are refused, at most 10 000
  files and 200 MB), then one screen shows the name, version, author and
  what it adds with "This plugin runs with your rights, like any program
  you install". Install replaces a plugin with the same id. Plugins >
  Manage plugins… lists every plugin to enable, disable or remove.
