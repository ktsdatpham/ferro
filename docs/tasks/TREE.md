# Ferro task tree

The single list of work, as a tree: milestones → steps → tasks. Humans and
agents pick a **leaf**, work on it, and tick it here in the same PR.

## Rules for whoever picks a task (humans and agents)

1. Read `AGENTS.md`, `docs/foundation/README.md`, the ADR named on the parent
   step, and the API map (`docs/api/TREE.md`) before writing code.
2. Claim a leaf: change `[ ]` to `[~]` and add `@owner` (agent or person) in
   the PR that starts it. One owner per leaf.
3. Done means: code + tests (basic, degenerate, tolerance edge) + doc
   comments + spec / glossary updated if a concept changed, and
   `scripts/ferro-check` passes. Then tick `[x]` and add the commit or PR.
4. **Verified** means someone other than the author re-ran the checks and
   the acceptance line written under the task: `✓ verified @who`.
   A parent is done only when all its children are verified.
5. Never rename a task id. New work is added as a new child with the next id.

Legend: `[ ]` open · `[~]` in progress · `[x]` done · `✓` verified ·
`(blocked: …)` waiting on another task.

---

## M5 Foundation 2 (ADR 0004)

- [~] **M5-1 Geometry source** @lead
  - [x] M5-1.1 `GeometrySource` (Parametric / Direct) on `Element` · PR #11 ✓ verified @lead
  - [x] M5-1.2 `set_geometry_source`: freeze walls (with openings) and floors; recognise plain walls / slabs · PR #11 ✓ verified @lead
  - [x] M5-1.3 `set_category` keeps fitting types; `set_element_type` on direct walls / floors · PR #11 ✓ verified @lead
  - [x] M5-1.4 Properties panel switch (Edit freely / Make parametric) · PR #11 ✓ verified @lead
  - [x] M5-1.5 Type selector and type data shown for direct walls / floors in the properties panel @lead
    - accept: select a frozen wall: the Properties panel shows a Type combo listing wall types; picking one runs `set_element_type`
- [ ] **M5-2 Representations and quantities** @lead
  - [ ] M5-2.1 Quantities from the body for every category: volume, surface area, footprint area, height (read-only computed parameters)
  - [ ] M5-2.2 Wall faces by side (exterior / interior / top / ends) for direct walls, by comparing with the type's layers when possible
  - [ ] M5-2.3 Derived representations: Axis (walls), Footprint (all), Box; cached, not saved
- [ ] **M5-3 Quantity-typed parameters** (ADR 0004 §2)
  - [ ] M5-3.1 Spec table: id, SI unit, Revit `SpecTypeId`, IFC measure, display units
  - [ ] M5-3.2 `ParamValue::Quantity { spec, value }`; old files still load
  - [ ] M5-3.3 Parameter definitions with `Uid` (key, spec, group, instance / type, Revit / IFC Pset mapping); built-in keys registered
  - [ ] M5-3.4 `set_parameter` takes quantities with units (`"0.8 W/mK"`), UI shows them in display units
- [ ] **M5-4 Material assets** (ADR 0004 §3)
  - [ ] M5-4.1 `Material` assets: appearance (OpenPBR subset), physical, thermal, environmental, cost; old files load
  - [ ] M5-4.2 Type quantities from layers: U-value, mass per m², GWP per m², cost per m²
  - [ ] M5-4.3 `create_material` / `set_material` tools and a material editor in the app
  - [ ] M5-4.4 IFC material property sets; Revit material op carries thermal / physical values
- [ ] **M5-5 Relationships** (ADR 0004 §5) @lead
  - [ ] M5-5.1 Relationship table in the document (`Uid`, kind, elements), saved, undoable
  - [ ] M5-5.2 `Join`: inferred L / T joins become explicit; joins survive freezing
  - [ ] M5-5.3 `Cut`: a void element cuts walls / floors, non-destructive
  - [ ] M5-5.4 `Host` moves from `host_id` into the table (kept as a fast index)
  - [ ] M5-5.5 IFC: `IfcRelVoidsElement`, `IfcRelConnectsPathElements`
- [ ] **M5-6 Revit: free-form elements as loadable families** (default; DirectShape as option)
  - [ ] M5-6.1 `RevitOp::FamilyInstance` with family geometry and category
  - [ ] M5-6.2 Add-in: build the family document (FreeFormElement), load, place, keep `Uid`
- [ ] **M5-7 IFC body for direct elements** (`IfcFacetedBrep`)
- [ ] **M5-8 Body kinds and radial-edge topology** (ADR 0004 §4)
  - [ ] M5-8.1 `BodyKind` Solid / Sheet / Wire / General
  - [ ] M5-8.2 Radial-edge topology: loose edges, an edge shared by 3+ faces
- [ ] **M5-9 Curved geometry**
  - [ ] M5-9.1 Surface / curve types incl. Cone, Sphere, Torus, NURBS in data, file and writers
  - [ ] M5-9.2 Kernel spike: truck / monstertruck vs OCCT, report with recommendation
- [ ] **M5-10 `ferro-scene`** (ADR 0004 §6)
- [x] **M5-11 Commands and keymaps** (SketchUp / Revit / Ferro presets, command palette) @lead
  - accept: `cargo test -p ferro-app keymap` passes; in the app (Ferro preset) `S` `B` picks Floor, `W` `A` Wall, `L` Line, Ctrl+K opens the palette and Enter runs the first match; Edit > Keyboard shortcuts switches preset and writes `~/.ferro/keymap.json`
- [ ] **M5-12 Tooling for many contributors** (ADR 0003)
  - [x] M5-12.1 `scripts/ferro-check`: fmt, clippy, tests, glossary lint, layer lint, new-dependency check, task-tree check @lead
    - accept: `python3 scripts/lint.py` prints `lint: ok`; injecting `use crate::model` into `geom/` or a `pub struct WallManager` makes it fail
  - [x] M5-12.2 API map `docs/api/TREE.md`, generated, checked in CI @lead
    - accept: `python3 scripts/api_tree.py --check` exits 0; adding a pub item without regenerating fails `lint.py`
  - [x] M5-12.3 PR template; `AGENTS.md` points to the tree, the map and the check @lead
    - accept: `.github/pull_request_template.md` exists; `AGENTS.md` step 0 and "Every PR" mention them
  - [ ] M5-12.4 UI languages (`tr!`, `en.ftl`, `vi.ftl`)
  - [ ] M5-12.5 Problem reports (local log, prefilled issue)
- [ ] **M5-13 Code layout by discipline** (after M5-3 / M5-4 land): `model::bim` split into `model::architecture::{wall, floor, opening, …}`, room for `structure`, `mep`, `analysis`, `site`, like Revit's `DB.Architecture` / `DB.Structure`

- [ ] **M5-14 Python SDK** (ADR 0005)
  - [ ] M5-14.1 `ferro-mcp --tools-json` prints the tool list (schemas) for generators
  - [ ] M5-14.2 Generated package `python/ferro/_tools.py`: one typed function per tool; generator script; CI freshness check
  - [ ] M5-14.3 Ferro Link client and object layer (`connect`, `Document`, element wrappers, `transaction`), tests against a fake link server
  - [ ] M5-14.4 `run_python` MCP tool (waits for ADR 0005 approval)

## M6 SketchUp parity, rest (on Foundation 2)

- [ ] M6-1 Reference lines and points (Tape guides), snapping to them (ADR 0003)
- [ ] M6-2 Dimensions (ADR 0003)
- [ ] M6-3 Paint individual faces
- [ ] M6-4 Components (Form types with instances, D-71)
- [ ] M6-5 Zoom window, Shift+Z
- [ ] M6-6 Section fill (cut faces filled)

## Later (from the foundation review)

- [ ] Reality data: point clouds (E57, LAS / LAZ → COPC), splats, scan-to-BIM (F7)
- [ ] Rooms, space boundaries, energy model export (Honeybee / OpenStudio, gbXML) (F8)
- [ ] Time axis, physics, `AnalysisResult` (F9)
- [ ] Layered opinions: options as variants, overrides, sync (F10)
