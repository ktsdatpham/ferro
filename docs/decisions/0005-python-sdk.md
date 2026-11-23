# 0005. Python SDK over the same API as MCP

- **Status:** Proposed (2026-11-23). Part 1 (generated client) is within
  D-44 / D-51 / D-65 and proceeds; part 2 (`run_python` in the app) waits for
  the owner's approval because it embeds an interpreter.
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
5. **(Needs approval)** An MCP tool `run_python { code }` runs a script
   against the open document inside one transaction group: all or nothing,
   one Proposal, journal records the script (D-29). The interpreter runs out
   of process (a `python3` the user has, or a bundled one), talking Ferro
   Link, with a time limit. Distributed plugins stay WASM (D-51).

## Alternatives considered

- Hand-written Python bindings (PyO3) to `ferro-core`: fast, but a second
  API surface that drifts from MCP and bypasses Proposals.
- Only MCP: simplest, but agents need many round trips.

## Consequences

- `ferro-mcp` can print its tool list as JSON for the generator; CI checks
  the generated package is current.
- Every new command automatically appears in Python.
