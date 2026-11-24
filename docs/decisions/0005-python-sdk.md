# 0005. Python SDK over the same API as MCP

- **Status:** Accepted (2026-11-24, owner approval, both parts). Part 1
  (generated client) is built; part 2 (`run_python`) is built in M5-14.4
  (see the implementation note); cancelling and user priority in M5-14.7,
  one target rule for design options in M5-14.8 (owner's changes to the
  busy policy, 2026-11-24).
- **Affects:** D-44, D-46, D-65, D-80 (D-51 superseded); chapter 7

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
   Link, with a time limit. Plugins may use it too (ADR 0009 supersedes D-51).

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
  too); calls naming their own option keep it. **One rule (M5-14.8)** for
  scripts and single agent commands: omitted (or `null`), the target is the
  option the user is editing in the app, else the shared model; the string
  `"shared"` names the shared model explicitly (as in `diff_documents`);
  an id names that option. The rule lives in one function,
  `ferro_tools::with_default_design_option`, used by the app for agent
  commands and by the run for script calls. The Proposal bar names where
  the changes land ("into option: X" or "shared model").
- **Security.** A script has the user's own rights: it is the user's
  Python. Only a caller allowed to edit may start one: the user, or an agent,
  whose script then becomes (part of) its Proposal unless the user raised
  autonomy; plugins run scripts like the user (ADR 0009; before it, D-51 refused them;
  plugin commands and `get_user_input`: ADR 0009 implementation note). The
  script reaches Ferro only with a **one-time token** (random UUID v4) given
  in environment variables (`FERRO_LINK_PORT`, `FERRO_LINK_TOKEN`), never in
  argv; the host accepts it only while that run lasts and drops it when the
  run ends. A script cannot start another (`ferro.python.nested`), call
  `undo` / `redo`, or use file tools.
- **Other clients during a run.** Read-only tools are answered; anything
  that changes the document or its history (commands, `undo`, `redo`,
  `save_document`, `revit_import`, another `run_python`) is refused with
  `ferro.python.busy` rather than queued, so a client never waits for an
  unknown time and nothing slips into the script's group.
- **The user has priority (M5-14.7).** While a script runs the status bar
  shows "Script running · N s" with a Stop button; Esc twice within a
  second also stops it. An edit the user tries meanwhile (a tool's command,
  the palette, undo / redo, save, open, new, accept / reject, a typed value)
  is held and a dialog asks "A script is running (N s). Stop it and do your
  action?" [Stop script and continue] [Keep waiting]. Stopping rolls the
  script back, then does the action; keeping waiting does it when the
  script ends. A held action is never dropped.
- **Cancel (M5-14.7).** `cancel_python` (new standard verb `cancel`,
  chapter 5 §5.5; a stable name) stops the running script: the host kills
  its process group with `SIGKILL` (Windows: `taskkill /T /F`), the same
  path as a timeout, so a script ignoring `SIGTERM` or running child
  processes is stopped too, and the whole group (or savepoint) rolls back;
  the run answers `ferro.python.cancelled` with the output so far. Allowed
  for the user and the client that started the run (same agent name, any
  version); others get `ferro.python.not_allowed`. Clients name themselves
  and all hold the link token, so this prevents mistakes between
  cooperating clients, not a client that lies about its name. Headless
  hosts expose a `RunControl` another thread cancels through; `ferro-mcp`
  reads stdin on its own thread so `cancel_python` (and the MCP
  `notifications/cancelled` for the waiting `run_python`) work while the
  script runs. The SDK's `Link.call` sends `cancel_python` when Ctrl+C
  interrupts a `run_python` call. The time limit (30 s by default, 300 s at
  most) stays as the hard stop for a forgotten script. A child that
  leaves the process group (`setsid`) is not reached.
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
