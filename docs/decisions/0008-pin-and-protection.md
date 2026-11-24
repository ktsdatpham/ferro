# 0008. Pin and protection of important elements

- **Status:** Accepted (2026-11-24, owner request; revised the same day: "like Revit, not extreme")
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
2. **What a pin blocks (as Revit).** Deleting a pinned element and moving
   it (move, rotate, mirror, align, nudge, dragging it or its handles) fail
   with `ferro.model.pinned` and change nothing. Everything else stays
   allowed on a pinned element: parameters, type, name, phase, editing its
   shape or profile, copying it. Deleting a host whose hosted element is
   pinned is refused the same way.
3. **Who may unpin.** Anyone who may edit: the user, an agent under its
   normal autonomy (a proposal when it proposes, direct when it applies),
   scripts and plugins. No extra rule.
4. **Auto-pin (as Revit).** Linked models are created pinned (when links
   exist). Levels, grids and reference planes are not pinned automatically;
   a project setting `auto_pin_datums` (default `false`) pins them on
   creation for teams that want it.
5. **UI.** A pin icon on pinned elements and in Properties; Pin / Unpin in
   the context menu and on the `PN` / `UP` two-letter codes (Revit keys); a
   pinned element is still selectable and its handles show as locked.
6. **Interop.** Revit: `Element.Pinned` round-trips through the plan and
   the add-in. IFC: not exported (no standard property).

## Alternatives considered

- **Per-user ownership (Revit worksharing).** Needed for multi-user sync,
  later; a pin is the single-user part of it and stays compatible.
- **Stricter pins (agents must ask, plugins cannot unpin, auto-pinned
  datums).** Proposed first, dropped on the owner's review: Revit's
  behaviour is enough.

## Consequences

- Every command path checks the pin through the `Transaction`, so MCP,
  scripts, agents and design options obey it alike.
- Scenario tests gain a pinned fixture element: every moving or deleting
  command on it must fail with `ferro.model.pinned` and roll back; other
  edits must pass.
