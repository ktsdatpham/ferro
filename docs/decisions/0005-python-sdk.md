# 0005. Python SDK over the same API as MCP

- **Status:** Accepted (2026-11-24, owner approval, both parts). Part 1
  (generated client) is built; part 2 (`run_python`) is built in M5-14.4
  (see the implementation note).
- **Affects:** D-44, D-46, D-51, D-65; chapter 7

## Context

AI agents write Python better than any other language, and many architects
already script in Python (pyRevit, Dynamo, Grasshopper). Agents driving
Ferro through dozens of single MCP tool calls are slow and spend tokens; a
short script that calls the same tools does the job in one step.

## Decision

1. **One API, two front ends.** The command / query set exposed by MCP
   (ferro-tools schemas) is the API. A Python package `ferro` is
   *generated* from those schemas: one typed function per tool, with the
   tool's description as docstring and units in parameter names / docs. No
   Python-only capabilities (D-44).
2. **Object layer like the Revit API**, written by hand on top of the
   generated functions: `ferro.connect() -> Document`, `doc.walls`,
   `doc.get(id)`, `wall.set(height=...)`, `doc.transaction("name")` as a
   context manager. Names follow the glossary.
3. **Transport: Ferro Link** (ADR 0001) to the open app; changes become a
   Proposal (D-26) unless the user raised autonomy.
4. Geometry queries can return numpy arrays when numpy is installed.
5. **(Approved 2026-11-24)** An MCP tool `run_python { code }` runs a script
   against the open document inside one transaction group: all or nothing,
   one Proposal, journal records the script (D-29). The interpreter runs out
   of process (a `python3` the user has, or a bundled one), talking Ferro
   Link, with a time limit. Distributed plugins stay WASM (D-51).

## Implementation note: `run_python` (M5-14.4, 2026-11-24)

- **Process.** The host (`ferro_mcp::python`, shared by the app and headless
  `ferro-mcp` / `ferro-cli`) starts the user's interpreter (`$FERRO_PYTHON`,
  else `python3`; `python` on Windows) as `python -I -u -c <bootstrap>`.
  `-I` keeps the current folder, the user site and `PYTHON*` variables out
  of `sys.path`; the bootstrap puts the SDK bundled in the binary (written
  to a fresh private temp folder per run, or `$FERRO_PYTHON_SDK`) first,
  refuses Python < 3.9 (`ferro.python.unavailable`) and execs the source,
  which arrives on stdin. The script runs in its own process group (Windows:
  `taskkill /T`), so a timeout kills the whole tree.
- **Grouping.** The run opens a `Session` group (one undo step, one journal
  entry whose `scripts` holds the source); if a group is already open, as for
  an agent's Proposal or `ferro-cli run --atomic`, the run is a
  `SubTransaction` savepoint in it, so a failed script undoes only its own
  work. `doc.transaction(name)` inside a script maps onto nested savepoints
  through three Ferro Link requests only a running script may send:
  `begin_transaction`, `commit_transaction`, `rollback_transaction`.
- **Design options.** `run_python {in_design_option}` runs every command of
  the script inside that option (`Session::execute_in_option`, which commits
  through the open group, so savepoints and rollback cover option layers
  too); calls naming their own option keep it. In the app the default is
  the option the user is editing, so a script lands where the user's own
  commands would; headless and plain agent calls default to the shared
  model.
- **Security.** A script has the user's own rights: it is the user's
  Python. Only a caller allowed to edit may start one: the user, or an agent,
  whose script then becomes (part of) its Proposal unless the user raised
  autonomy; plugins are refused (`ferro.python.not_allowed`, D-51). The
  script reaches Ferro only with a **one-time token** (random UUID v4) given
  in environment variables (`FERRO_LINK_PORT`, `FERRO_LINK_TOKEN`), never in
  argv; the host accepts it only while that run lasts and drops it when the
  run ends. A script cannot start another (`ferro.python.nested`), call
  `undo` / `redo`, or use file tools.
- **Other clients during a run.** Read-only tools are answered; anything
  that changes the document or its history (commands, `undo`, `redo`,
  `save_document`, `revit_import`, another `run_python`) is refused with
  `ferro.python.busy` rather than queued, so a client never waits for an
  unknown time and nothing slips into the script's group. The user's own
  edits in the app wait the same way (a status message says why).
- **Never blocking the UI.** The app answers the script's calls between
  frames and polls the process once per frame; the `run_python` answer is
  sent when the script ends. Clients wait the script's time limit + 30 s.

## Alternatives considered

- Hand-written Python bindings (PyO3) to `ferro-core`: fast, but a second
  API surface that drifts from MCP and bypasses Proposals.
- Only MCP: simplest, but agents need many round trips.

## Consequences

- `ferro-mcp` can print its tool list as JSON for the generator; CI checks
  the generated package is current.
- Every new command automatically appears in Python.
