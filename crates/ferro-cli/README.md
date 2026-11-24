# ferro-cli

Ferro without the UI: run tool scripts, inspect, check and benchmark models.
For agents, CI and performance budgets (D-62). Every call goes through the
same tool host as MCP (`ferro_mcp::tools::Server`, headless, never linked to
the app), so a script is exactly what an agent would send (D-17, D-44).

```sh
cargo run -p ferro-cli -- <command> ...
```

| Command | What it does |
|---|---|
| `run <script.json\|-> [--in F] [--out F] [--ifc F] [--obj F] [--glb F] [--keep-going] [--atomic]` | Run a script; one JSON line per call, then a summary line |
| `call <tool> ['<json args>'] [--in F] [--out F] [--ifc F] [--obj F] [--glb F]` | One call |
| `tools [--names]` | The tool list (`{"tools": [...]}` as MCP `tools/list`, or names only) |
| `info <model>` | Summary: element counts by category, types, levels, bounding box (mm), problems |
| `check <model>` | Self-acceptance report; exit 1 on any problem |
| `bench <script.json\|-> [--repeat N] [--in F]` | Median / max ms per call over N fresh runs (default 10) |

`<model>` and `--in` / `--out` are `.fro` (SQLite) or `.fro.json`.

**Agents verify their work with `ferro-cli check`.**

## Scripts

A JSON array of calls with the same tool names and arguments as MCP
(units as strings allowed: `"3m"`, `"300cm"`):

```json
[
  {"id": "l1", "tool": "create_level", "args": {"name": "Ground", "elevation": 0}},
  {"id": "walls", "tool": "create_walls", "args": {
    "points": [[0, 0], [0, "6m"], ["8m", "6m"], ["8m", 0]], "closed": true,
    "level": {"$ref": "l1.result.element"}}},
  {"tool": "create_opening", "args": {
    "wall": {"$ref": "walls.result.elements.0"}, "kind": "door", "position": "3m"}}
]
```

- `id` (optional): a name for the step. Not numeric, no dots, unique.
- `{"$ref": "<step>.<path>"}` as an argument value is replaced by a value from
  the result of an earlier **successful** step: `<step>` is its `id` or 0-based
  index; `<path>` walks keys and array indices of that step's result.
- Unknown keys in a step are an error (typos are not ignored).

Example: [`tests/scripts/house.json`](tests/scripts/house.json).
[`tests/scripts/subtract.json`](tests/scripts/subtract.json) cuts an opening
with `subtract_forms`: the CLI session has the geometry kernel attached
(`ferro-kernel`, ADR 0007).

### `run` output

```text
{"ok":true,"result":{...},"step":0,"tool":"create_level"}
{"error":"command failed; nothing was changed","failures":[{"code":"ferro.opening.outside_host",...}],"ok":false,"step":1,"tool":"create_opening"}
{"ok":true,"result":{"saved":"house.fro"},"tool":"save_document"}
{"summary":{"calls":2,"elements":10,"failed":1,"ok":false,"rolled_back":false}}
```

- Stops at the first failed call unless `--keep-going`.
- Outputs (`--out`, then `--ifc`, `--obj`, `--glb`) are written only when no
  call failed, or always with `--keep-going`.
- `--atomic` runs the script as one transaction group (one undo step, one
  journal entry). If any call fails, everything is rolled back; with
  `--keep-going` the outputs then hold the model as it was before the script.
  `open_document` / `new_document` inside an atomic script replace the
  document and end the group; `undo` / `redo` fail inside it.
- Keys are sorted, so output is deterministic for the same input, except
  `uid`s the model generates (random UUID v7) and file paths you pass.

## `check`

Opens the model and runs, in this order:

| Check | Problem codes |
|---|---|
| `open` | `ferro.io.*` |
| `regenerate`: rebuild walls, floors, openings; stored geometry must equal it | generation codes, `ferro.cli.stale_geometry` |
| `bodies`: `validate_solid` for closed bodies, walls and floors; `validate` for others | `ferro.topo.*` |
| `roundtrip_json`: save to `.fro.json`, reopen, save again, compare | `ferro.cli.roundtrip`, `ferro.io.*` |
| `roundtrip_fro`: save to an in-memory `.fro`, reopen, compare | `ferro.cli.roundtrip`, `ferro.io.*` |
| `deterministic_export`: IFC, OBJ and GLB exported twice are identical | `ferro.cli.nondeterministic`, `ferro.io.*` |

Prints `{"file", "ok", "elements", "checks": [{"check", "ok", "problems"}]}`.

## Benchmarks

`ferro-cli bench tests/scripts/cuts-bench.json --repeat 3` times the cut
cache (M5-5.7): 50 walls each cut by its own box, then an edit of an
unrelated wall (every wall regenerates, no cut row is recomputed), then
moving one cutter and moving it back (one row recomputed, then a cache
hit). Compare the `set_wall_height` / `move_elements` lines with a
`create_relationship` line (one kernel call).

## `fingerprint` and the model corpus (M5-17)

`ferro-cli fingerprint m.fro` prints a canonical description of the model:
every element by local id (deterministic for a script) with its category,
body kind, references, and for geometry each face by persistent id with
vertex count, area (mm²) and normal, plus counts, volume (mm³), bounding box
and relationships. `--expect F` compares with a saved fingerprint (numbers
within 1e-6 relative) and exits 1 listing the differing paths.

`tests/corpus/*.json` are sample models; `cargo test -p ferro-cli --test
corpus` rebuilds each, runs `check` and compares it with
`<name>.fingerprint.json`, plus hand-computed oracle quantities. A kernel
change that renames faces, loses lineage or changes a quantity fails there.
After an intended change: `FERRO_BLESS=1 cargo test -p ferro-cli --test
corpus`, then review the fingerprint diff in the PR. Add a script to the
corpus for every new modelling feature.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | A tool call failed, or `check` found a problem |
| 2 | The command line, a script, arguments or a model file could not be read. Stdout: `{"ok": false, "error": {"code", "message"}}` |

## Failure codes added by the CLI

Stable once released (AGENTS.md).

| Code | Meaning |
|---|---|
| `ferro.cli.usage` | Unknown command or option, missing value |
| `ferro.cli.script` | Script or argument JSON is not JSON or has the wrong shape |
| `ferro.cli.bad_ref` | A `$ref` names no earlier successful step, or no such path in its result |
| `ferro.cli.tool_error` | A tool reported an error without a kernel failure code (bad units, missing argument, unknown tool) |
| `ferro.cli.stale_geometry` | Saved geometry differs from regenerated geometry |
| `ferro.cli.roundtrip` | Saving and reopening changes the document |
| `ferro.cli.nondeterministic` | Two exports of the same document differ |

File errors keep their `ferro.io.*` codes (`ferro.io.file`, `ferro.io.json`,
`ferro.io.sqlite`); command failures keep the kernel's codes.
