# 4. Operations: commands, transactions, regeneration, failures

This chapter defines **how the model changes**. It is the part most likely to
become technical debt if every contributor invents their own approach, so the
rules are strict.

## 4.1 The one rule (D-17)

> **Nothing mutates a `Document` except a `Command` executing inside a
> `Transaction`.**

UI tools, MCP tools, the Revit bridge, scripts and tests all go through the same
path. This gives undo / redo, validation, AI access, macro recording and a future
collaboration log for free.

## 4.2 Commands

A `Command` is a **serialisable description of an intent**:

```rust
pub trait Command: Serialize + DeserializeOwned {
    const NAME: &'static str;               // "create_rectangle", same as the MCP tool name
    type Output: Serialize;                 // e.g. created ElementIds
    fn execute(&self, tx: &mut Transaction) -> Result<Self::Output, CommandError>;
}
```

Rules:

1. A command name is `verb_noun` in snake_case (chapter 5), and is **identical**
   to its MCP tool name. One command = one MCP tool. No MCP-only logic.
2. Command arguments are plain data (numbers with explicit units, ids as
   `ElementId` or `Uid`, enums). No closures, no references.
3. Commands are **deterministic**: same document + same command = same result.
   (Needed for replay, tests and collaboration.)
4. Commands compose: a command may call other commands inside the same
   transaction.

## 4.3 Transactions

Revit's transaction model, simplified:

| Concept | Meaning | Revit |
|---|---|---|
| `Transaction` | An atomic unit of change with a user-visible name. Commit or roll back as a whole. One undo step | `Transaction` |
| `TransactionGroup` | Several transactions merged into one undo step (e.g. a drag gesture) | `TransactionGroup` |
| `SubTransaction` | A nested attempt that can roll back without aborting the parent | `SubTransaction` |

Lifecycle:

```
start("Push/Pull")  →  commands mutate a working copy
                    →  regenerate dependents (4.4)
                    →  collect failures (4.5)
                    →  Error?  roll back everything, report
                    →  else    commit, push undo step, emit DocumentChanged (4.6)
```

Rules:

1. Only one open transaction per document at a time.
2. Reading the document is always allowed; writing only through `&mut Transaction`.
3. Interactive gestures (dragging push-pull) use a `TransactionGroup` so the
   whole drag is **one undo step**, with live preview via repeated
   start / rollback of inner transactions.

## 4.4 Regeneration (D-18)

After commands run and before commit, the document **regenerates**:

1. The dependency graph lists, for each element, what it depends on (level,
   host, type, joined elements, referenced datums).
2. Every element whose inputs changed is rebuilt in dependency order (topological
   sort). Cycles are an Error.
3. Computed parameters (area, volume, length) are updated.
4. Regeneration is part of the transaction: if it fails, the transaction rolls
   back.

Revit equivalent: `Document.Regenerate()` and the internal element dependency
graph.

**Milestone 2 implementation** (ADR 0001): walls, floors and openings store
parameters plus a derived `geometry` cache. When a transaction touches a level,
type, wall, floor or opening, `model::generate::regenerate` rebuilds every BIM
element and writes only those whose geometry changed. Straight walls sharing an
end point are mitred; openings cut their host. The dependency graph replaces
this global pass when models grow.

## 4.5 Failures

Commands and regeneration report problems as `Failure`s, not panics:

| Severity | Effect | Example | Revit |
|---|---|---|---|
| `Warning` | Commit allowed; shown to the user / agent | "Wall overlaps another wall" | Warning |
| `Error` | Transaction rolls back | "Profile is self-intersecting" | Error |

Each failure has a stable `code` (`ferro.geom.self_intersecting_profile`), a
human message, and the affected `ElementId`s. MCP returns failures to the agent
so it can correct itself.

Rules:

1. **No `panic!`, `unwrap()` or `expect()` on user / agent input paths.** Panics
   are for broken invariants only.
2. Failure codes are stable and documented; agents rely on them.

Client codes. Clients of Ferro Link (the Python SDK, ADR 0005) raise these
when the problem is in reaching Ferro, not in a command. They are stable too:

| Code | Meaning |
|---|---|
| `ferro.link.not_open` | No link file: the app is not running |
| `ferro.link.unreachable` | The link file exists but nobody answers on its port |
| `ferro.link.invalid_token` | The app refused the token |
| `ferro.link.timeout` | No answer within the timeout |
| `ferro.link.protocol` | The answer was not one JSON line `{ok, body}` |
| `ferro.tool.failed` | The tool refused without a failure code of its own |

`get_element` on a missing id fails with `ferro.model.not_found`.

Script codes. `run_python` (ADR 0005 part 2, §6.2) reports these as the
first failure; failures of the script's last failed tool call follow:

| Code | Meaning |
|---|---|
| `ferro.python.unavailable` | No Python 3.9+ could be started (not installed, wrong `FERRO_PYTHON`, too old) |
| `ferro.python.timeout` | The script ran past `timeout_s`; its process tree was killed, nothing changed |
| `ferro.python.failed` | Uncaught exception or non-zero exit; nothing changed, traceback in `stderr` |
| `ferro.python.nested` | A script tried to start `run_python` itself |
| `ferro.python.busy` | Another script is running; changes from other clients wait (read-only tools work) |
| `ferro.python.not_allowed` | The caller may not start scripts (plugins, D-51) |

A script's changes are one `TransactionGroup` (§4.3, `Session` groups):
commit = one undo step and one journal entry naming the script; failure =
the whole group rolls back. When a group is already open (an agent's
Proposal), the script is a `SubTransaction` savepoint in it, and its
`doc.transaction(name)` blocks are nested savepoints.

## 4.6 Events

After a successful commit the document emits:

```rust
DocumentChanged { transaction_name, added: Vec<ElementId>, modified: Vec<ElementId>, deleted: Vec<ElementId> }
```

The UI, the renderer, the MCP server and the Revit live link subscribe to this.
Nobody polls the document. Revit equivalent: `DocumentChanged` event.

## 4.7 Undo / redo

- One committed `Transaction` (or `TransactionGroup`) = one undo step.
- Undo restores the previous document state exactly (ids included).
- Implementation (snapshot vs inverse operations) is internal; the contract above
  is not.

## 4.8 Selection is not a mutation

Selection, camera, active tool and hover state live in the **app**, not in the
`Document`, and never go through transactions. (Revit: `UIDocument.Selection`
vs `Document`.)
