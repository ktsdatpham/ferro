# 0008. Pin and protection of important elements

- **Status:** Accepted (2026-11-24, owner request)
- **Affects:** D-26 (proposals), D-33, ADR 0005 (`run_python`), ADR 0006 (design options), M5-27

## Context

Datums (levels, grids, reference planes, lines and points) and big
references (linked models, survey points, site) carry the whole model. A
stray drag, a careless agent or a script must not move or delete them.
Revit has `Element.Pinned` (pinned elements refuse move and delete until
unpinned) and, in worksharing, element ownership. SketchUp has *Lock* on
groups. The owner asked for a pin mechanism that also keeps agents and
scripts away from these elements.

## Decision

1. **Field.** Every element has `pinned: bool` (default `false`, omitted
   from files when `false`; Revit `Element.Pinned`).
2. **What a pin blocks.** A command that would delete a pinned element,
   change its location, geometry, level, type, host, or phase, or override
   it in a design-option layer, fails with `ferro.model.pinned` and changes
   nothing. Allowed on a pinned element: `set_pinned` itself, its name, its
   user parameters and computed values that the transaction regenerates.
   Deleting a host whose hosted element is pinned is refused the same way.
3. **Who may unpin.** The user, directly (one undo step). An agent's or a
   script's `set_pinned {pinned: false}` always becomes a proposal for the
   user, whatever the agent's autonomy level, and a plugin cannot unpin at
   all (`ferro.model.pin_protected`). Pinning is allowed to everyone.
4. **Auto-pin.** New levels (and, when they exist, grids, reference planes,
   lines, points and linked models) are created pinned. A project setting
   `auto_pin_datums` (default `true`) turns this off.
5. **UI.** A pin icon on pinned elements and in Properties; Pin / Unpin in
   the context menu and on the `PN` / `UP` two-letter codes (Revit keys); a
   pinned element is still selectable and its handles show as locked.
6. **Interop.** Revit: `Element.Pinned` round-trips through the plan and
   the add-in. IFC: not exported (no standard property).

## Alternatives considered

- **Per-user ownership (Revit worksharing).** Needed for multi-user sync,
  later; a pin is the single-user part of it and stays compatible.
- **Lock levels beyond a boolean.** Not needed yet; the agent rule (3)
  already separates "the user can unpin" from "agents cannot".

## Consequences

- Every command path checks the pin through the `Transaction`, so MCP,
  scripts, agents and design options obey it alike.
- Scenario tests gain a pinned fixture element: every modifying command
  on it must fail with `ferro.model.pinned` and roll back.
