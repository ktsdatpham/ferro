# ferro (Python SDK)

Script the Ferro app open on this computer from Python (ADR 0005). The SDK
talks [Ferro Link](../docs/decisions/0001-bim-elements-and-ferro-link.md) and
offers exactly the tools MCP offers: no Python-only capabilities.
Pure Python >= 3.9, standard library only; numpy is optional (`as_array`,
`bounding_box(as_numpy=True)`).

```python
import ferro

doc = ferro.connect()                                  # the open Ferro app
l1 = doc.create_level("L1", elevation=0)
walls = doc.create_walls([(0, 0), (0, "6m"), ("8m", "6m"), ("8m", 0)],
                         closed=True, height="3m", level=l1)  # clockwise
door = walls[0].add_door(position="2m")
for w in doc.walls:
    print(w.id, w.length_m, w.refresh().volume_m3)     # m, m³ from list_elements
```

Units: numbers are millimetres, strings may carry units (`"3m"`, `"300cm"`);
points are `(x, y)` or `(x, y, z)`; angles are degrees. Z is up.

## Layers

- `ferro.tools` — generated: one keyword-only, typed function per tool,
  named exactly as the tool (`ferro.tools.create_wall(start=..., end=...)`).
  Arguments left as `None` are not sent. Returns the tool's JSON body.
- `ferro.Document` / `Element` / `Wall` / `Floor` / `Opening` / `Level` — a
  small object layer on top (`doc.walls`, `doc.get(id)`, `doc.selection`,
  `wall.set(height="3.5m")`, `element.set_parameter(key, value)`,
  `element.delete()`).
- `ferro.link.Link` — the Ferro Link client.

Changes made through the SDK are Commands like any other: each tool call is
one Transaction (one undo step), recorded in the journal with the `agent`
name given to `connect()`. With the app's default autonomy they collect into
one Proposal the user accepts or rejects. `doc.transaction(name)` does **not**
make a block atomic in such a connected script.

## Atomic scripts: `run_python`

Send the script to Ferro instead (MCP tool `run_python {code, timeout_s?}`,
or `ferro-cli call run_python '{"code": "..."}'`). Ferro runs it in your
`python3 -I` with this SDK, `doc`, `tools` and `ferro` predefined, as **one
transaction group**: all or nothing, one undo step, one Proposal for agents.
Set `result = ...` to return JSON; an uncaught exception or a non-zero exit
rolls everything back (`ferro.python.failed`, traceback in `stderr`); past
`timeout_s` (default 30, max 300) the process tree is killed and rolled back
(`ferro.python.timeout`). Inside it, `ferro.connect()` returns the run's
document and `with doc.transaction(name):` is a savepoint: an exception
leaving the block undoes the block. `FERRO_PYTHON` picks the interpreter.

## Errors

Every failure raises `ferro.FerroError(code, message, failures)`. `code` is
the server's first failure code (e.g. `ferro.opening.outside_host`,
`ferro.model.not_found`) or a client code:

| Code | Meaning |
|---|---|
| `ferro.link.not_open` | no `link.json` (the app is not running) or not connected |
| `ferro.link.unreachable` | `link.json` exists but nothing answers on its port |
| `ferro.link.invalid_token` | the app refused the token (stale `link.json`) |
| `ferro.link.timeout` | no answer within the timeout |
| `ferro.link.protocol` | the answer was not one JSON line `{ok, body}` |
| `ferro.tool.failed` | the tool refused without a failure code (see `message`) |

## Development

```sh
python3 scripts/gen_python.py            # regenerate ferro/_tools.py from ferro/tools.json
python3 scripts/gen_python.py --refresh  # first refresh tools.json from `ferro-mcp --tools-json`
python3 scripts/gen_python.py --check    # exit 1 if stale (run by scripts/lint.py)
python3 -m unittest discover -s python/tests
```

`cargo test -p ferro-mcp` fails when `ferro/tools.json` no longer matches the
Rust tool list, so a new command cannot land without its Python function.
