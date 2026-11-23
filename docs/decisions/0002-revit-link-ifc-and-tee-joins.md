# 0002. Revit link, IFC4 export and T-joins (milestone 3)

- **Status:** Accepted
- **Date:** 2026-11-23
- **Affects:** D-41, D-65, D-75; chapters 4, 6, 7

## Context

Milestone 3 connects Ferro to Revit both ways and exports IFC. The spec
requires that the C# add-in contain no geometry logic (§7.2) and that unit
conversion happen only at edges (D-01).

## Decision

**Revit link**

1. New crate `ferro-revit` (depends on `ferro-core` only) is the Revit edge:
   feet, Revit parameter names and categories live here.
2. `plan(doc, state) -> Vec<RevitOp>` (Ferro → Revit). `state` lists the Revit
   elements tagged with a Ferro uid and the signature of the operation that
   last wrote them. Operations: `delete`, `material`, `level`, `wall_type`,
   `floor_type`, `wall`, `floor`, `opening`, `direct_shape`, in that order
   (deletes first). Each upsert carries a signature (FNV-1a of its content,
   lengths rounded to 1e-9 ft); unchanged elements produce no operation.
   A wall whose top is attached to a level carries no height, so moving the
   level updates only the level.
3. Elements without a level are placed on a synthetic level
   `ferro:level:0` ("Ferro Level 0"); importing it back means "no level".
4. `import(session, ctx, snapshot)` (Revit → Ferro) applies a snapshot as
   ordinary commands: creates (returning the new uids for the add-in to
   store), updates (`set_wall`, `set_floor`, `set_opening`,
   `set_level_elevation`, `set_name`, `set_element_type`) and deletes of
   elements in `known_ferro_uids` that are gone from Revit. Levels and types
   with the same name are adopted instead of duplicated. Types are not
   deleted from the Revit side.
5. Transport: Ferro Link tools `get_document_version`, `revit_plan`,
   `revit_import`; or, without the app, `ferro_ffi` (`ferro_revit_plan`,
   `ferro_revit_import`) on a `.fro.json` file. Imports in the app are
   proposals like any agent change; an import that changes nothing is not a
   step.
6. The add-in (Revit 2025+, .NET 8) stores the uid and signature in
   Extensible Storage (`FerroElement`) and the uids known after the last sync
   on Project Information (`FerroProject`). Buttons: Pull, Push, Live (polls
   the document version while Revit is idle and pulls on change). Floors are
   replaced rather than sketch-edited; doors and windows use the first loaded
   family of their category with a type per size.
7. New commands `set_wall` and `set_floor` (also useful to agents).

**IFC4 export** (`ferro_io::export_ifc`): Reference View style STEP file; see
§6.4. GlobalIds are element uids; IFC-only objects get ids derived from
stable names, so re-exports are identical. Validated with IfcOpenShell
(schema + geometry) during development.

**T-joins**: a straight wall whose end lies on the location line of another
straight wall, away from its ends, stops at that wall's face on its own side.
A collinear neighbour at the same point does not prevent it. The crossed wall
is not cut.

## Alternatives considered

- *Revit shared parameter for the uid:* visible and editable by users, needs a
  shared parameter file; Extensible Storage is invisible and robust.
- *Mapping logic in C#:* faster to write, but duplicates the kernel's rules in
  a second language and breaks §7.2.
- *Diffing in the add-in:* signatures computed by the planner keep the add-in
  stateless.

## Consequences

- The add-in compiles in CI against Revit 2025 reference assemblies; it has
  not been run inside Revit by CI. Older Revit versions (.NET Framework 4.8)
  need a second target.
- Curved walls with openings, roofs, rooms and loadable families beyond
  doors / windows are not synced yet.
