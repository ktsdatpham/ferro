# 0001. First BIM elements, Ferro Link and AI proposals (milestone 2)

- **Status:** Accepted
- **Date:** 2026-11-23
- **Affects:** D-14, D-16, D-18, D-26, D-41, D-65, D-70, D-73; chapters 3, 4, 5, 7

## Context

Milestone 2 adds real BIM elements (walls, floors, doors, windows) with system
family types and materials, and lets AI agents edit the model that is open in
the app. The spec described the target (Category → Family → Type → Instance,
regeneration, hosting, joins) without fixing how the first elements store their
data or how agents reach a running app.

## Decision

**Elements and types**

1. New `ElementBody` variants: `Material`, `WallType`, `FloorType`, `Wall`,
   `Floor`, `Opening`. Types are elements (they have ids and uids, and they are
   saved, undone and journaled like everything else). A type carries the
   category of its instances (`walls`, `floors`); `Element::is_definition()`
   tells types and materials apart from placed elements.
2. New category `Materials` (`OST_Materials`, `IfcMaterial`). A Form cannot be
   classified as a material.
3. A type holds a `CompoundStructure`: `CompoundLayer`s
   `{function, thickness, material}` from the exterior (wall) or top (floor)
   face inwards. `LayerFunction` = structure | substrate | insulation | finish |
   membrane (Revit `MaterialFunctionAssignment`). "Layer" alone stays banned
   (D-74).
4. `Wall` = location `Curve` (line or arc, horizontal), `base_offset`, `top`
   (`unconnected {height}` or `level {level, offset}`), `location_line`
   (Revit `WALL_KEY_REF_PARAM`). Base level = `Element::level_id`. The
   **exterior face is on the left** of the curve direction.
5. `Floor` = plan outline + holes (`Point2`, mm) and an `offset`; the top sits at
   level + offset and the slab extends down by the type thickness.
6. `Opening` = door or window hosted by a straight wall (`host_id`):
   `position` (distance along the wall to the centre), `width`, `height`,
   `sill`. Category `doors` / `windows`. A sill below 0.5 mm cuts a notch
   instead of a hole. Openings in curved walls are not supported yet.
7. Missing types are created on demand as `Generic - 200mm`.

**Regeneration**

8. Wall, floor and opening geometry is **derived** and stored on the element as
   a cache (`geometry: Body`). `model::generate::regenerate` rebuilds all of it
   whenever a transaction touches a level, type, wall, floor or opening, and
   only writes elements whose geometry actually changed (structural sharing is
   kept). Output is deterministic.
9. **Joins (M2 rule):** two straight walls whose location curves share an end
   point (within 1 mm), and no third wall at that point, are mitred. Three or
   more walls at a point, T-junctions and very sharp angles (mitre point more
   than 5 × thickness away) stay butt-ended. Joins are inferred from the
   geometry, like Revit's automatic joins; explicit join relationships
   (§3.8: allow / disallow join, join order) come with T-joins in a later
   milestone.
10. An opening that does not fit (outside the wall, taller than the wall,
    overlapping another) is an **error** in the transaction that creates it,
    and a **warning** afterwards (e.g. the wall was shortened); a warned
    opening is not cut.
11. Computed, read-only parameters: `ferro.dimensions.length`, `.height`,
    `.thickness`, `.area` (floors, plan area) and `.volume`.

**Editing**

12. `push_pull_face` on a wall, floor or opening edits **parameters**, never the
    derived geometry (D-73): wall top → height (or top offset), bottom → base
    offset, end → length, side → moves the wall; floor top → offset; opening
    edges → width / height / sill. Thickness always comes from the type.
13. Moving or rotating a wall moves its curve; hosted openings follow. Moving
    an opening alone slides it along its host. Copying a wall copies its
    openings. Deleting a host deletes its openings. A type in use cannot be
    deleted (`ferro.model.type_in_use`). Deleting a level keeps walls and
    floors where they are by converting heights and offsets.

**Agents**

14. New crate `ferro-tools`: tool schemas, unit normalisation and dispatch,
    shared by `ferro-mcp` and the app. Every command is still a tool with the
    same name.
15. **Ferro Link** (D-65): the app listens on `127.0.0.1` (random port) and
    writes `~/.ferro/link.json` (`port`, secret `token`, `pid`, `project`,
    `version`; mode 0600). Clients send one JSON request per line
    `{token, tool, args, agent, prompt}` and get `{ok, body}`. `ferro-mcp`
    forwards every call to the app when a link is reachable and otherwise
    works on its own session. `FERRO_HOME` overrides the folder.
16. **Proposals** (D-26): with autonomy "Propose" (default), the first agent
    command opens a transaction group named after the agent; further commands
    join it; the user accepts (one undo step, journaled with the agent as
    actor) or rejects (rolled back). Any user edit accepts the open proposal
    first; Undo rejects it. "Apply directly" skips the review.
17. The in-app chat uses the user's own key (Anthropic Messages API or any
    OpenAI-compatible endpoint, including local ones). Its tool calls go
    through the same Ferro Link queue, so they are proposals too. Settings live
    in `~/.ferro/settings.json` in plain text, which the UI states.

## Alternatives considered

- *Types as plain structs inside each instance:* simpler, but breaks "change
  the type, every instance updates" and Revit round-tripping.
- *Storing only parameters and meshing on the fly in the app:* the kernel would
  no longer be the single source of geometry for snapping, export and MCP.
- *Explicit join elements now:* needed for T-joins and join order; deferred to
  keep M2 small. The inferred rule is a subset of the explicit one.
- *WebSocket or HTTP for Ferro Link:* more moving parts; newline JSON over TCP
  is enough on localhost and needs no dependency.

## Consequences

- `.fro` files from v0.1 open unchanged (new variants only). Files with walls
  cannot be opened by v0.1.
- Every geometry consumer (render, snap, export, MCP detail) uses
  `Element::geometry()` instead of Forms only.
- Regeneration is global for BIM elements; a dependency graph (§4.4) replaces
  it when models grow (budget work in milestone 3).
