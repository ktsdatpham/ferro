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
  small object layer on top (`doc.walls`, `doc.get(id)`, `doc.selection` (ids: `doc.selection()`),
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

## Plugins: your own buttons in a minute

A plugin (ADR 0009) adds commands to Ferro: a button in the left panel, an
item in the *Plugins* menu, an entry in the command palette (Ctrl+K) and,
if you like, a two-letter key. It is a folder in `~/.ferro/plugins/`
(`$FERRO_HOME/plugins` when set) holding `plugin.json` and Python code.

**Plugins > New plugin…** asks for a name, creates the folder from a
template and opens it. Then:

1. edit `main.py`,
2. save (Ferro reloads plugins within a second; *Plugins > Reload plugins*
   does it at once),
3. click the command.

`plugin.json`:

```json
{
  "id": "my-tools",
  "name": "My tools",
  "version": "0.1.0",
  "author": "Your name",
  "description": "What it does.",
  "commands": [
    {"id": "box_at_origin", "label": "Box at origin", "key": "BX",
     "group": "panel", "run": "main:box_at_origin"}
  ]
}
```

- `id`: lower-case letters, digits, `-`, `_`; unique among your plugins.
- `commands[].run`: `module:function` in the plugin folder, or a `.py`
  script file (run like `run_python`, with `doc`, `tools`, `ferro`
  predefined and `result` returned).
- `key` (optional): a two-letter code typed in sequence, as in Revit. A key
  Ferro already uses stays Ferro's; the app shows a warning.
- `group` (optional): `panel` (default; also a button in the left panel)
  or `menu` (menu and palette only). `timeout_s` (optional): time limit,
  30 s by default, 300 s at most.

`main.py`:

```python
import ferro

def box_at_origin(doc):
    height = ferro.ask("Box height", default="3m")   # a dialog; None = Cancel
    if height is None:
        return None
    ids = doc.selection()                            # ids of the selected elements
    return doc.call("create_box", origin=[0, 0, 0], width="2m", depth="2m", height=height)
```

The function gets `doc` if it takes a parameter; what it returns is the
run's result, and its last printed line shows in the status bar. Each click
is **one undo step**, all or nothing: an exception undoes everything the
command did. The Stop button (or Esc twice) stops it. The current folder is
the plugin folder, and modules next to `main.py` can be imported. Outside
the app (headless `ferro-mcp` / `ferro-cli`), `ferro.ask` returns the
default and `doc.selection()` is empty.

A plugin runs with your rights, like a Revit add-in or a SketchUp
extension: it can call every tool, read and write files, use the network.
Install plugins from others only if you trust them: *Plugins > Install
from file…* (a `.zip`; dropping it on the window works too) shows who made
it and what it adds once. *Plugins > Manage plugins…* enables, disables and
removes plugins. To make a plugin part of Ferro, see
[`docs/plugins/PROMOTE.md`](../docs/plugins/PROMOTE.md).

**For agents** writing a plugin from a description: create the folder with
the three files above (copy the template from *New plugin…* or from
`TEMPLATE_MAIN` in `crates/ferro-mcp/src/plugins.rs`), keep one function
per command, use only `doc`, `ferro.tools` and `ferro.ask`, and check the
manifest by opening *Plugins > Manage plugins…* (a broken plugin shows its
reason).

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
