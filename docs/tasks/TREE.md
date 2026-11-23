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
- [~] **M5-10 `ferro-scene`** (ADR 0004 §6) @agent-scene
  - [x] M5-10.1 Crate `ferro-scene`: `DisplayScene` with per-element `DrawRecord`s (mesh, offset, colour, bounds, revision), incremental `sync` by structural sharing, view state kept out @agent-scene
    - accept: `cargo test -p ferro-scene` passes (box, empty doc, element without geometry, open body, one-element change re-meshes only it, determinism, section cut)
  - [x] M5-10.2 Instancing of translated copies: meshes stored in local coordinates, shared by quantised content hash + verified match (`INSTANCE_TOL`) @agent-scene
    - accept: test `array_copies_share_one_mesh` (array of 5 → 1 mesh, 5 users; moved copy stays; edited copy leaves) and `rotated_copy_is_not_an_instance` pass
  - [x] M5-10.3 `ferro-app` builds its vertex lists from `ferro-scene`: per-element vertex chunks, rebuilt only on record / selection / style change; hidden and section never re-mesh @agent-scene
    - accept: `cargo test -p ferro-app scene` passes; in the app selection colours, X-ray, Wireframe, Hidden line, hidden elements / tags, section cut lines, hover face highlight, window / crossing selection all look as before
  - [x] M5-10.4 Benchmarks (ignored tests): 2,000 boxes full build, one element changed, selection change @agent-scene
    - accept: `cargo test -p ferro-scene --release -- --ignored --nocapture` and `cargo test -p ferro-app --release scene -- --ignored --nocapture` print timings
  - [ ] M5-10.5 GPU instancing: upload each shared mesh once plus per-record transforms / colours (renderer API change)
  - [ ] M5-10.6 Picking and snapping read the scene (`SnapScene` from `DisplayScene` instead of the document)
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
- [x] **M5-18 `ferro-cli`**: run commands and scripts without the UI (agents, CI, benchmarks) · branch `m5/cli` @agent-cli
  - [x] M5-18.1 Crate `ferro-cli` reusing the MCP tool host (`ferro-mcp` gains a lib target: `tools::Server::headless`, file errors carry `ferro.io.*` codes); `run` scripts with `$ref`, `--keep-going`, `--atomic`, `--in` / `--out` / `--ifc` / `--obj` / `--glb` @agent-cli
    - accept: `cargo run -q -p ferro-cli -- run crates/ferro-cli/tests/scripts/house.json --out /tmp/h.fro --ifc /tmp/h.ifc` prints 5 call lines + 2 output lines + `{"summary":{…,"elements":10,"ok":true}}` and exits 0; `cargo test -p ferro-cli` passes (`failing_script_exits_nonzero_with_the_failure_code`, `atomic_leaves_nothing_behind_on_failure`)
  - [x] M5-18.2 `call`, `tools [--names]`, `info` (counts by category, types, levels, bounding box, problems) @agent-cli
    - accept: `cargo run -q -p ferro-cli -- info /tmp/h.fro` equals `crates/ferro-cli/tests/scripts/house.info.json` (test `info_output_is_stable`); `tools --names` lists every MCP tool
  - [x] M5-18.3 `check`: open, regenerate (stale geometry), validate every body, `.fro.json` and in-memory `.fro` round trip, IFC / OBJ / GLB exported twice identical; exit 1 on any problem @agent-cli
    - accept: `cargo run -q -p ferro-cli -- check /tmp/h.fro` prints `"ok": true` and exits 0; test `check_finds_a_broken_body` (a moved vertex → `ferro.topo.non_planar_face`, exit 1)
  - [x] M5-18.4 `bench <script> --repeat N`: median / max ms per call (D-62) @agent-cli
    - accept: `cargo run -q -p ferro-cli -- bench crates/ferro-cli/tests/scripts/house.json --repeat 3` prints 5 calls with `median_ms` / `max_ms`
  - [x] M5-18.5 Clean errors (exit 2, `ferro.cli.*` / `ferro.io.*` code, never a panic); house + `check` in `scripts/ferro-check` and CI (ubuntu); README, AGENTS.md line, architecture table @agent-cli
    - accept: test `bad_input_gives_a_clean_error_never_a_panic` passes; `scripts/ferro-check` ends with `ferro-cli check: ok`
- [x] **M5-21 File versions and migrations** @lead: `format_version` bumped whenever saved data gains a shape older Ferro cannot read (quantities, material assets, definitions did); a migration chain from every old version, tested on saved samples of each milestone; newer files refused with a clear message
  - accept: `cargo test -p ferro-io migrate` (4 tests): an M4 house saved as format 1 (`crates/ferro-io/tests/fixtures/m4-house.v1.fro.json`, and the same as a format-1 `.fro`) opens unchanged and saves as format 2; format 0 / future versions fail with `ferro.io.version` and a clear message; version table in `crates/ferro-io/src/migrate.rs`
- [ ] **M5-22 Views as elements** (D-33): `View3D` / `PlanView` / `SectionView` carry camera, section box / plane, style (X-ray, wireframe, hidden line), hidden elements and tag filters — today's view state (section, hide, styles) moves into them; saved, switchable, usable by agents
- [ ] **M5-23 Design options and phases** (D-32, ADR 0006 accepted): phases Revit-style data (ordered list, created / demolished) with an ArchiCAD-style simple mode, status derived per view, phased cuts, quantities by status; design options as layers of changes over the shared model (add / remove / override without copying), combinations per view, accept = merge, options kept
- [~] **M5-24 Project settings** @lead: display units per project (mm / m, imperial), precision, Rsi / Rse choices, currency, location (latitude / longitude, time zone, EPW) — one place that UI, MCP and exports read
  - [x] M5-24.1 Data (`ProjectSettings` in `ProjectInfo`: `ProjectUnits`, `SiteLocation`, currency; omitted when default) and commands `set_project_units`, `set_site_location`, `set_project_currency` with MCP tools (degrees / lengths with units at the edge) @lead
    - accept: `cargo test -p ferro-core project_` (10 tests: formatting, decimals, `-0`, bad units, ranges incl. the poles, partial updates, undo, currency codes)
  - [ ] M5-24.2 App: Project settings window; lengths / areas / volumes in the properties panel and value box shown in project units
  - [ ] M5-24.3 Exports read settings: IFC `IfcSite` RefLatitude / RefLongitude / RefElevation, Revit SiteLocation, cost currency
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

- [~] **M5-15 Scenario test audit** @agent-tests
  - [x] M5-15.1 Every command through `from_tool` + `execute`: basic, degenerate, missing / wrong-kind ids, tolerance edges, rollback with the documented code, bad JSON arguments; fixes: offset past the middle → `offset_too_large`, no edge below `SHORT_CURVE_TOL` from any Form command @agent-tests
    - accept: `cargo test -p ferro-core scenario_tests` passes (20 tests, 2 ignored = M5-15.10); `valid_calls_cover_every_command` lists all 37 commands
  - [x] M5-15.2 Undo / redo of every command restores the exact serialised document; failures keep redo @agent-tests
    - accept: `cargo test -p ferro-core undo_and_redo_restore_exact_documents_for_every_command full_history_undo_redo_roundtrip failures_do_not_touch_redo` passes
  - [x] M5-15.3 `.fro` / `.fro.json` round trip of every element kind (with and without id gaps); IFC / OBJ / GLB exports deterministic @agent-tests
    - accept: `cargo test -p ferro-io --test roundtrip` passes (7 tests, 1 ignored = M5-15.9)
  - [x] M5-15.4 Push/pull + imprint stress at 0° / 30° / 45° and 5 km from the origin, bodies validated and volumes checked every step @agent-tests
    - accept: `cargo test -p ferro-core stress_tests` passes (6 tests, 2 ignored = M5-15.7, M5-15.8)
  - [x] M5-15.5 Seeded xorshift fuzzing of Form edits (3 × 200 steps) and random push/pull volume bookkeeping @agent-tests
    - accept: `cargo test -p ferro-core fuzz_tests` passes in under 10 s (debug)
  - [x] M5-15.6 Snapping: priority, occlusion, parallel / perpendicular, pixel tolerance, degenerate input; fix: coincident top / bottom corners prefer the nearer @agent-tests
    - accept: `cargo test -p ferro-snap --test scenarios` passes (13 tests)
  - [x] M5-15.7 Bug (medium): pulling a pocket floor back to flush or beyond fails with `ferro.algo.collapse` (SketchUp removes the pocket / makes a boss). Repro: box 4000×3000×2000, imprint 1000 mm square on top, push −800, push the same face +800 → collapse. Test `stress_tests::pocket_back_to_flush_and_beyond` (ignored) @agent-kernelfix · branch `m5/kernel-fixes`
    - fix: stretch merges moved corners that land within `VERTEX_TOL` of the far corners of the stretched faces, drops collapsed faces, and when the pulled face then fills a hole of a coplanar face with the same normal removes that hole and the face (one top face again; lineage: the pocket floor and walls in `deleted`, the top `kept`); past flush the walls turn and it is a boss. Also fixes a regression of the previous commit: half of a split top pushed down (prism + heal) now drops the spike corners instead of failing the exact simplicity test
    - accept: `cargo test -p ferro-core pocket_back_to_flush_and_beyond step_down_and_back_flush pocket_pulled step_pulled_back_flush` passes (+800 / +799.995 / +800.005 → healed, 6 faces, merged face `ferro.topo.not_found` afterwards; +1000 / +1200 → boss of 200 / 400; +799.98 → `ferro.geom.short_curve`; at 0° / 30° / 45° and 5 km out)
    - limits: two coplanar faces separated by an imprinted line stay two faces (the line is kept, only filled holes heal)
  - [x] M5-15.8 Bug (high): a pocket pushed deeper than the solid is committed as a self-intersecting body with a wrong volume (`validate_solid` passes). Repro: same box, imprint square on top, push −2100 → accepted. Push/pull needs a check that the moved face stays inside / outside the solid. Test `stress_tests::pocket_deeper_than_the_box_is_refused` (ignored) @agent-kernelfix · branch `m5/kernel-fixes`
    - fix: `Body::validate()` (so `validate_solid`, every Transaction on changed Forms, `ferro-cli check` and the fuzz invariants) rejects faces that share no vertex but touch, with exact `predicates::triangles_intersect` (orient3d, D-53) on `Body::face_triangles`, after an x-sweep over face boxes and per-triangle boxes → `ferro.topo.self_intersecting`; push / pull keeps that code (other validation failures stay `ferro.algo.collapse`); push-through within `VERTEX_TOL` of the bottom unchanged; stretch drops spike-tip vertices left by a step pulled back below an old corner (found by the stricter exact `is_simple`)
    - accept: `cargo test -p ferro-core pocket_deeper_than_the_box_is_refused pocket_depth_tolerance_edges_against_the_bottom moving_a_pocket_floor_through_a_side_is_refused topo::body` passes (−1999.995 / −2000 / −2000.005 → `through` with exact volume, −2000.02 → `ferro.topo.self_intersecting`, −1999.4 → prism, at 0° / 30° / 45° and 5 km out)
    - limits: faces that share a vertex are not compared (a pocket wall folding through its own top face is not caught here); curved (`Cylinder`) faces are skipped
  - [ ] M5-15.9 Bug (low): IFC `Name` of unnamed elements is `"walls #<local id>"`; ids are renumbered when a file with gaps is loaded, so the same model exports different names after save / load. Use a uid-based or stable label. Test `roundtrip::ifc_export_is_stable_across_a_round_trip` (ignored)
  - [x] M5-15.10 Bug (high): self-intersecting (bow-tie) outlines are accepted. Repro: `create_polygon` points (0,0) (3000,1000) (3000,0) (0,2000), or `create_floor` with the same outline → committed. Needs a simple-polygon check (`push_pull::is_simple` exists, private) for faces, floor outlines and holes. Tests `scenario_tests::self_intersecting_*` (ignored) @agent-kernelfix · branch `m5/kernel-fixes`
    - fix: exact `geom::predicates::{polygon_is_simple, segments_intersect, polygons_touch, points_are_collinear}` (D-53); `create_polygon`, `Profile::from_loops` (floors, wall profiles) and closed `imprint_face` curves fail with `ferro.geom.self_intersecting_profile` (the code named in 04-operations §4.5); holes outside / touching the outline / touching each other → `ferro.geom.invalid_profile`
    - accept: `cargo test -p ferro-core self_intersecting imprinting_a_bow_tie_is_refused collinear_outlines profile predicates` passes (bow-tie face, vertical bow-tie, figure eight, notch 1 mm short vs touching, bow-tie / outside / touching / overlapping floor holes)
  - [ ] M5-15.11 Spec mismatch: D-60 orders snaps "points > edges / faces > directions > grid", `ferro-snap` uses points > edges > directions > faces > ground. Decide and align spec or code
    - note (@agent-kernelfix, for the lead; behaviour unchanged): recommend keeping the code and amending the D-60 wording to "points > edges > directions > faces > grid / ground". That is SketchUp's behaviour: a point or edge inference beats an axis lock, but an axis / parallel / perpendicular inference beats "On Face" — drawing along the red axis across a face stays on the axis. Faces cover most of the screen, so with faces above directions a direction inference could almost never win while hovering geometry. The spec's "edges / faces" grouping reads as a typo for this order; a one-line spec fix plus the existing `ferro-snap` scenario tests (priority cases) close it
  - [ ] M5-15.12 Low: `set_name` without `name` clears the name (schema says required; serde treats a missing `Option` as `null`); a journal loaded from a file with id gaps still names the old ids
    - [x] M5-15.12.1 `set_name` requires `name` (`null` still clears): `#[serde(deserialize_with = "Option::deserialize")]`, so a missing `name` is bad arguments @agent-kernelfix · branch `m5/kernel-fixes`
      - accept: `cargo test -p ferro-core bad_tool_arguments_are_clean_errors` passes with `{"element": 1}` and `{"element": 1, "name": 5}` rejected by `AnyCommand::from_tool`; the `"name": null` undo case still passes
    - [x] M5-15.12.2 A collinear outline gives `ferro.geom.degenerate` from both `create_polygon` and `create_floor` (was `ferro.topo.degenerate_loop` for floors) @agent-kernelfix · branch `m5/kernel-fixes`
      - accept: `cargo test -p ferro-core collinear_outlines_fail_with_the_same_code every_command_rejects_degenerate` passes
    - [ ] M5-15.12.3 A journal loaded from a file with id gaps still names the old ids (not kernel work; open)

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
