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

- [x] **M5-1 Geometry source** @lead
  - [x] M5-1.1 `GeometrySource` (Parametric / Direct) on `Element` · PR #11 @lead
    - accept: `cargo test -p ferro-core geometry_source` (7 tests) passes
  - [x] M5-1.2 `set_geometry_source`: freeze walls (with openings) and floors; recognise plain walls / slabs · PR #11 @lead
  - [x] M5-1.3 `set_category` keeps fitting types; `set_element_type` on direct walls / floors · PR #11 @lead
  - [x] M5-1.4 Properties panel switch (Edit freely / Make parametric) · PR #11 @lead
    - accept: select a wall, "Edit freely" → Geometry shows Direct, Push/Pull on its faces edits the solid; "Make parametric" on a plain box wall returns it
  - [x] M5-1.5 Type selector and type data shown for direct walls / floors in the properties panel @lead
    - accept: select a frozen wall: the Properties panel shows a Type combo listing wall types; picking one runs `set_element_type`
- [~] **M5-2 Representations and quantities** @lead
  - [x] M5-2.1 Quantities from the body for every category: volume, surface area, footprint area, height (read-only computed parameters) @lead
    - accept: test `direct_walls_and_floors_report_quantities_from_their_solid`; a frozen wall shows Volume / Height / Thickness and no Length
  - [ ] M5-2.2 Wall faces by side (exterior / interior / top / ends) for direct walls, by comparing with the type's layers when possible
  - [ ] M5-2.3 Derived representations: Axis (walls), Footprint (all), Box; cached, not saved
- [ ] **M5-3 Quantity-typed parameters** (ADR 0004 §2)
  - [x] M5-3.1 Spec table: id, SI unit, Revit `SpecTypeId`, IFC measure, display units @agent-quantities · branch `m5/quantities`
    - Acceptance: `cargo test -p ferro-core math::quantity_spec` passes; `QuantitySpec::ALL` has 23 specs with unique ids, and `QuantitySpec::parse("35 MPa", None)` gives `(Stress, 35e6)`. — ✓ tests re-run @lead after merge
  - [x] M5-3.2 `ParamValue::Quantity { spec, value }`; old files still load @agent-quantities · branch `m5/quantities`
    - Acceptance: `cargo test -p ferro-core model::parameter` passes, including `old_variants_still_load_and_report_their_spec` (M1–M4 JSON shapes load). — ✓ tests re-run @lead after merge
  - [x] M5-3.3 Parameter definitions with `Uid` (key, spec, group, instance / type, Revit / IFC Pset mapping); built-in keys registered @agent-quantities · branch `m5/quantities`
    - Acceptance: `cargo test -p ferro-core parameter` passes (covers `parameter_definition` and `parameter_commands`); all 12 `keys::*` have a definition with `Uid = ParameterDefinition::uid_for_key(key)`; `set_parameter_definition` persists in `ProjectInfo` and undoes. — ✓ tests re-run @lead after merge
  - [x] M5-3.4 `set_parameter` takes quantities with units (`"0.8 W/mK"`), UI shows them in display units @agent-quantities · branch `m5/quantities`
    - Acceptance: `cargo test -p ferro-tools` passes; via MCP `set_parameter` with `"value": "1200 kg/m3"` stores `{"type":"quantity","value":{"spec":"density","value":1200.0}}`; the properties panel lists non-`ferro.*` quantity parameters as e.g. `0.8 W/(m·K)`. — ✓ tests re-run @lead after merge ; app ✓ verified @lead (wall param `0.8 W/mK` shown in display units)
- [ ] **M5-4 Material assets** (ADR 0004 §3)
  - [x] M5-4.1 `Material` assets: appearance (OpenPBR subset), physical, thermal, environmental, cost; old files load · branch `m5/materials` @agent-materials
    - Acceptance: `cargo test -p ferro-core model::material` passes; `old_material_json_loads_and_saves_unchanged` shows `{"color":[..],"roughness":..}` re-serialises byte-identical. — ✓ tests re-run @lead after merge
  - [x] M5-4.2 Type quantities from layers: U-value, mass per m², GWP per m², cost per m² · branch `m5/materials` @agent-materials
    - Acceptance: `cargo test -p ferro-core structure_quantities` passes; 200 mm brick (λ 0.77) + 100 mm wool (λ 0.035) gives U = 0.30424 W/m²K (Rsi 0.13, Rse 0.04, ISO 6946), and a layer without λ gives `u_value_w_m2k: null` with a `no_conductivity` gap in `get_element` on the type. — ✓ tests re-run @lead after merge
  - [x] M5-4.3 `create_material` / `set_material` tools and a material editor in the app · branch `m5/materials` @agent-materials
    - Acceptance: `cargo test -p ferro-core material_tests` and `cargo test -p ferro-mcp agent_sets_material_data` pass; in the app, ✎ next to a material opens an editor whose Apply changes the wall type's U-value shown in Properties. — ✓ tests re-run @lead after merge ; app ✓ verified @lead (Wool λ 0.035 via the editor → type U-value 0.304 W/(m²·K), mass 363 kg/m²)
  - [x] M5-4.4 IFC material property sets; Revit material op carries thermal / physical values · branch `m5/materials` @agent-materials
    - Acceptance: `cargo test -p ferro-io ifc` and `cargo test -p ferro-revit material_op` pass; IFC of a material with data contains `IFCMATERIALPROPERTIES('Pset_MaterialThermal'`; the C# add-in (`Applier.cs`, thermal / structural assets) builds in CI. — ✓ tests re-run @lead after merge ; C# ✓ compiled in CI (revit-addin job on 7141d1f)
  - [ ] M5-4.5 Acoustic asset (absorption per octave band) and texture rendering of `ImageMapping`
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
  - [x] M5-12.5 Problem reports (local log, prefilled issue) @lead
    - accept: `cargo test -p ferro-app report` passes; Help > Report a problem shows the log (command names and codes only), Open GitHub issue opens a prefilled URL; a panic writes `~/.ferro/reports/crash-*.md` that is offered on the next start
- [ ] **M5-13 Code layout by discipline** (after M5-3 / M5-4 land): `model::bim` split into `model::architecture::{wall, floor, opening, …}`, room for `structure`, `mep`, `analysis`, `site`, like Revit's `DB.Architecture` / `DB.Structure`

- [ ] **M5-16 Kernel seam crate `ferro-kernel`**: every Form geometry operation goes through one crate so the kernel can change without touching what is above (lesson from SolidCraft, `docs/review/2026-11-solidcraft-solidworks.md`)
- [ ] **M5-17 Model corpus**: sample models rebuilt in CI, checking topology ids, lineage, volumes and areas after every kernel change (SolidCraft's naming corpus), plus hand-computed oracle cases and a robustness fuzz with boolean identities
- [ ] **M5-19 Problems panel** ("What's Wrong"): regeneration warnings and broken references listed, click selects, fix hints; references carry `exact` / `unverified`, and a lost reference is reported, never silently re-attached
- [ ] **M5-20 Model cache in `.fro`**: display meshes (and B-rep) cached so files open without regenerating; linked models load light (mesh + faces needed to snap / host) and resolve on edit
- [ ] **M5-18 `ferro-cli`**: run commands and scripts without the UI (agents, CI, benchmarks)
- [ ] **M5-21 File versions and migrations**: `format_version` bumped whenever saved data gains a shape older Ferro cannot read (quantities, material assets, definitions did); a migration chain from every old version, tested on saved samples of each milestone; newer files refused with a clear message
- [ ] **M5-22 Views as elements** (D-33): `View3D` / `PlanView` / `SectionView` carry camera, section box / plane, style (X-ray, wireframe, hidden line), hidden elements and tag filters — today's view state (section, hide, styles) moves into them; saved, switchable, usable by agents
- [ ] **M5-23 Design options and phases** (D-32): option sets / options and phase created / demolished made real (filtering, views, IFC / Revit mapping) before collaboration builds on them
- [ ] **M5-24 Project settings**: display units per project (mm / m, imperial), precision, Rsi / Rse choices, currency, location (latitude / longitude, time zone, EPW) — one place that UI, MCP and exports read
- [ ] **M5-25 Family and component definition format** (D-43, D-71): `.ffa` = parameters + formulas + geometry (Forms / profiles / sweeps) + type catalog; components are instances of a definition; what M6-4, M6-8–M6-10 build on
- [ ] **M5-26 Change format for sync and branches** (D-34, D-55, F10): every property has a stable path; a change is a list of path edits; diff / merge / sync use it
- [ ] **M5-14 Python SDK** (ADR 0005)
  - [x] M5-14.1 `ferro-mcp --tools-json` prints the tool list (schemas) for generators @agent-python
    - accept: `cargo run -q -p ferro-mcp -- --tools-json` prints `{"tools": [...]}` equal to MCP `tools/list`; `cargo test -p ferro-mcp` passes (incl. `tools_json_matches_tools_list`, `python_sdk_snapshot_is_current`) ✓ verified @lead (52 tools, 8 tests)
  - [x] M5-14.2 Generated package `python/ferro/_tools.py`: one typed function per tool; generator script; CI freshness check @agent-python
    - accept: `python3 scripts/gen_python.py --check` exits 0 and `python3 scripts/lint.py` prints `lint: ok`; editing `python/ferro/tools.json` makes both fail ✓ verified @lead
  - [x] M5-14.3 Ferro Link client and object layer (`connect`, `Document`, element wrappers, `transaction`), tests against a fake link server @agent-python
    - accept: `python3 -m unittest discover -s python/tests` passes (14 tests) on Python ≥ 3.9 with no third-party packages ✓ verified @lead (14 OK; also live: created 4 walls in the running app, read volumes, error code mapping)
  - [ ] M5-14.4 `run_python` MCP tool (waits for ADR 0005 approval)
  - [ ] M5-14.5 Register the SDK's client error codes (`ferro.link.not_open`, `unreachable`, `invalid_token`, `timeout`, `protocol`, `ferro.tool.failed`) in the spec's failure-code list
  - [ ] M5-14.6 `get_element` summary with a body kind field, so the SDK picks wrappers without guessing and `get(id)` needs one call

## M6 SketchUp parity, rest (on Foundation 2)

- [ ] M6-1 Reference lines and points (Tape guides), snapping to them (ADR 0003)
- [ ] M6-2 Dimensions (ADR 0003)
- [ ] M6-3 Paint individual faces
- [ ] M6-4 Components (Form types with instances, D-71)
- [ ] M6-5 Zoom window, Shift+Z
- [ ] M6-6 Section fill (cut faces filled)
- [ ] M6-7 Temporary dimensions on selection, type to change (Revit / SolidWorks Instant3D)
- [ ] M6-8 Type catalog as a table, CSV import / export (SolidWorks design tables)
- [ ] M6-9 Parameter formulas with units (SolidWorks equations, Revit family formulas)
- [ ] M6-10 Structural members: profile swept along a path, corner trims, cut list (SolidWorks weldments)
- [ ] M6-11 Catalog wizards: doors / windows by TCVN sizes, stairs by code (Hole Wizard idea)
- [ ] M6-12 Instant cost and carbon estimate from material assets × quantities
- [ ] M6-13 Tool property panels generated from the command schema (xCAD property pages)

## Later (from the foundation review)

- [ ] Reality data: point clouds (E57, LAS / LAZ → COPC), splats, scan-to-BIM (F7)
- [ ] Rooms, space boundaries, energy model export (Honeybee / OpenStudio, gbXML) (F8)
- [ ] Time axis, physics, `AnalysisResult` (F9)
- [ ] Layered opinions: options as variants, overrides, sync (F10)
