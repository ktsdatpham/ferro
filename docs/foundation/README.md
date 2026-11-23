# Ferro Foundation Spec

This folder is the **constitution** of Ferro. It fixes the concepts, words, units,
coordinate systems and rules of operation *before* the codebase grows, so that
humans and AI agents contributing in parallel build one coherent system instead
of technical debt.

> **Rule zero:** if your change introduces a concept, a term, a unit, or a way of
> mutating the model that is not described here, the change must update this
> spec first (or in the same PR), and be approved by a maintainer.

## Why Revit is the reference

Revit is criticised for being hard to *design* in, but its **data model is
rigorous**: every object is an element with a stable identity, a category, a
type, typed parameters, a location, constraints to levels and hosts, and every
change goes through a transaction that can fail cleanly. That rigour is exactly
what turns a sketch into BIM data.

Ferro keeps Revit's rigour **underneath** and puts SketchUp/Rhino-speed
interaction **on top**. Where Revit has a concept that fits, we use the **same
name and the same meaning**, so that mapping to Revit and IFC is mechanical and
contributors who know Revit feel at home.

Where Revit is weak, we deliberately differ, and say so (e.g. `Point3` vs `Vec3`,
millimetres instead of feet, free-form editing inside `Form` elements).

## Reading order

| # | Document | Covers |
|---|---|---|
| 1 | [Coordinates, units and tolerance](01-coordinates-units-tolerance.md) | Axes, origins, north, units, precision, tolerance |
| 2 | [Geometry](02-geometry.md) | Point, vector, plane, transform, curves, loops, profiles, faces, solids, topology |
| 3 | [Elements and data model](03-elements-data-model.md) | Document, Element, ids, category / family / type / instance, parameters, levels, grids, work planes, hosting |
| 4 | [Operations](04-operations.md) | Commands, transactions, regeneration, failures, undo, events |
| 5 | [Naming and code conventions](05-naming-conventions.md) | Glossary, type / function / algorithm naming, errors, tests |
| 6 | [Interop](06-interop.md) | `.fro` file format, MCP, Revit and IFC mapping |
| 7 | [Architecture and layering](07-architecture.md) | Crates, layers, dependency rules |

Open questions still to be decided by the maintainer are worked through, in Vietnamese, in the [Decision Book](../decision-book/README.md) (106 questions). Accepted answers become `D-xx` entries below.

## Decision register

Every foundational choice has an id (`D-xx`). Status is one of
**Proposed** → **Accepted** → (**Superseded by D-yy**). Changing an Accepted
decision requires a new ADR in [`../decisions/`](../decisions/).

| Id | Decision | Status |
|---|---|---|
| D-01 | Internal length unit is the **millimetre**, stored as `f64` | **Accepted** |
| D-02 | Internal angle unit is the **radian**; degrees only at UI / MCP edges | **Accepted** |
| D-03 | Right-handed coordinates, **Z up**, X = project east, Y = project north, counter-clockwise positive | **Accepted** |
| D-04 | Three origins as in Revit: **Internal Origin** (fixed), **Project Base Point**, **Survey Point**, plus true-north angle | **Accepted** |
| D-05 | Model geometry must stay within **10 km** of the Internal Origin; rendering is camera-relative | **Accepted** |
| D-06 | Fixed global tolerances (`VERTEX_TOL` = 0.01 mm, `SHORT_CURVE_TOL` = 0.5 mm, `ANGLE_TOL` = 1e-6 rad); never compare `f64` with `==` | **Accepted** |
| D-07 | **`Point3` and `Vec3` are distinct types** (Revit's `XYZ` conflates them) | **Accepted** |
| D-08 | A `Plane` is a full frame: origin + X axis + Y axis (normal derived) | **Accepted** |
| D-09 | Curves are bounded and parameterised on **[0, 1]** in the public API | **Accepted** |
| D-10 | Profiles: outer `CurveLoop` counter-clockwise about the plane normal, holes clockwise | **Accepted** |
| D-11 | Solid topology is a **half-edge B-rep**; faces may have holes; solids are closed, manifold, outward-facing | **Accepted** |
| D-12 | **All geometry belongs to an element.** Free SketchUp-style editing happens inside a `Form` element; geometry never sticks across elements | **Accepted** |
| D-13 | Identity: `ElementId` (u64, document-local, never reused) **and** `Uid` (UUID v7, stable across save / export / Revit round-trip) | **Accepted** (Q-B7) |
| D-14 | Classification: **Category → Family → Type → Instance**, as in Revit | **Accepted** (with D-40) |
| D-15 | Data lives in **typed parameters** (instance or type), not free-form string tags | **Accepted** |
| D-16 | **Levels** are elements; vertical position is expressed as level + offset | **Accepted** |
| D-17 | Every mutation is a **Command** executed inside a **Transaction**; MCP tools are commands 1:1 | **Accepted** |
| D-18 | After commit, a **dependency graph** regenerates dependents; failures are Warning (commit allowed) or Error (rollback) | **Accepted** |
| D-19 | `ferro-core` is pure: no IO, no UI, no global mutable state, deterministic | **Accepted** |
| D-20 | Use Revit's term whenever the concept matches; never invent synonyms (see glossary) | **Accepted** |
| D-21 | `.fro` files are versioned (`format_version`) with stable keys; **container is SQLite** (incremental save, crash-safe, holds history and assets) with **JSON as the interchange format** for AI, git and tools (Q-B4) | **Accepted** |
| D-22 | **Licence: MPL-2.0, open core.** App, kernel, BIM, MCP, plugin SDK are open; Ferro Cloud (hosted AI, sync, cloud render, premium assets, DWG, enterprise) is the business (Decision Book Q-A3) | **Accepted** |
| D-23 | **Scope: design + presentation** (concept → render → boards → video). Construction documentation stays in Revit; the view system is designed so 2D drawings can be added later (Q-A1) | **Accepted** |
| D-24 | **The document is an immutable, structurally shared tree**: every commit produces a new `DocumentVersion`; readers (render, AI, regeneration) work on snapshots without locks; undo returns to a previous version (Q-B1) | **Accepted** |
| D-25 | **Persistent topology ids + semantic references**: every face / edge / vertex has a persistent id; every topological algorithm returns a lineage map (what came from what); BIM elements are referenced by semantic names (e.g. `WallFace::Exterior`) (Q-C4) | **Accepted** |
| D-26 | **AI changes are proposals by default**: shown as a reviewable draft (accept / reject / refine) built on an uncommitted `TransactionGroup` / branch; users may raise autonomy per action type; an agent never deletes human-created elements without asking (Q-H5) | **Accepted** |
| D-27 | **UI: egui for milestones 1–2**, final toolkit chosen at the end of milestone 2; the UI talks to the core only through commands, queries and events (Q-F1) | **Accepted** |
| D-28 | **Contributions use DCO sign-off**, no CLA; contributors keep copyright and license their work under MPL-2.0 (Q-A4) | **Accepted** |
| D-29 | **State is the source of truth, with a command journal alongside**: every transaction records actor (user / agent + model), time, originating prompt and invoked skills; periodic snapshots; replay is for audit, not required to rebuild the document (Q-B3) | **Accepted** |
| D-30 | **Hybrid geometry kernel**: own Rust half-edge kernel for planar B-rep (with holes) and push-pull; proven libraries for robust booleans and 2D polygon ops behind Ferro APIs; NURBS / curved surfaces plug in later behind a `Surface` trait; must build for WASM and iOS (Q-C1) | **Accepted** |
| D-31 | **The design pipeline is data**: `Brief` and `Program` are core elements; each stage (brief, program, site, massing, layout, facade, images, boards, video, handover) has a status and is checked against earlier stages; stages may be visited in any order (Q-H8) | **Accepted** |
| D-32 | **Design options are core**: every element belongs to the shared model or to one option of an option set; options can be viewed side by side and one is primary; phase fields (existing / demolished / new) are reserved from the start (Q-E7) | **Accepted** |
| D-33 | **Views are elements**: `View3D`, `PlanView`, `SectionView`, `Elevation`, `Scene`, plus view templates; they carry camera, cut, style, filters and active option (Q-E9) | **Accepted** |
| D-34 | **Collaboration target is real-time, Figma-style**: local-first, server-ordered changes, conflicts resolved per element property, soft locks while editing geometry; sync transmits results, not command replays; ships after single-user (Q-I1) | **Accepted** |
| D-35 | **`ferro-core` must build for `wasm32-unknown-unknown`**, enforced in CI; no dependency that prevents it (Q-L2) | **Accepted** |
| D-36 | **Element schema is hybrid**: core categories are typed Rust structs; users add typed parameters; plugins register versioned schemas before storing data; unknown plugin data is preserved when the plugin is absent (Q-B2) | **Accepted** |
| D-37 | **Large projects use federation**: a `Project` groups linked `Document`s (discipline, building, site) sharing coordinates and referencing each other by `Uid`; region streaming / partial loading comes later (Q-B5) | **Accepted** |
| D-38 | **Parametric elements + direct-edit Forms, no global history tree**: BIM elements derive geometry from location + parameters + type; Forms are edited directly; deeper parametrics come from the node graph later (Q-C6) | **Accepted** |
| D-39 | **Surfaces in v1: `Plane` and `Cylinder`**, exact, inside the half-edge B-rep; meshes are only display caches and imported assets / scans; NURBS and SubD added later behind the `Surface` trait (Q-C2) | **Accepted** |
| D-40 | **Classification is Revit-style categories with a 1:1 IFC4 mapping** (tracking IFC5), plus a standard `classification` parameter (Uniclass / OmniClass / Vietnamese codes) (Q-E1) | **Accepted** |
| D-41 | **Walls / floors / roofs carry the full schema from day one** (location line, multi-layer `CompoundStructure` with layer functions and materials, levels, explicit join relationships); UI and the join engine are delivered incrementally; `convert_form_to_walls` turns Forms into real walls (Q-E3) | **Accepted** |
| D-42 | **Rooms are detected from bounding elements** and update automatically; **area schemes** allow several measurement rules on one model; the Vietnamese rule set (DTXD/DTSD, net area) is the first official plugin (Q-E4) | **Accepted** |
| D-43 | **Families are declarative**: a family file holds parameters, Form geometry and simple constraints, can be generated by AI from a catalogue image or description, with scripts for advanced cases; `.rfa` import only via the Revit add-in. **File extensions: `.fro` project, `.ffa` family, `.fte` template** (Q-E6) | **Accepted** |
| D-44 | **AI lives both inside and outside**: the embedded agent uses exactly the same MCP tools as external agents; no privileged internal API (Q-H1) | **Accepted** |
| D-45 | **AI perceives the model through four channels**: an always-present hierarchical model digest, read-only SQL queries, ID-labelled viewport images, and a spatial graph (adjacency, orientation, levels), plus a diff of what the user changed since the last turn (Q-H3) | **Accepted** |
| D-46 | **AI acts on three tiers**: primitive commands, deterministic design skills (room subdivision, stairs, window placement, roofs; pluggable), and sandboxed scripts for the long tail; all end as commands in a transaction (Q-H4) | **Accepted** |
| D-47 | **Generative design = LLM orchestrator + solvers + evaluators**: the model sets constraints, calls layout / optimisation solvers, scores results (area, codes, daylight) and iterates; LLMs never place geometry by guessing coordinates where a solver exists (Q-H6) | **Accepted** |
| D-48 | **Own renderer on wgpu** (`ferro-render`), built as a composable render graph so presentation styles, AI-render buffers and path tracing are configurations of one system; export to external engines via glTF / USD (Q-G1) | **Accepted** |
| D-49 | **Presentation styles are data** (`RenderStyle` element) composed of edge, fill, effect and background layers; user-created, shareable, AI-generatable; includes data-driven colouring (by function, area, height, phase, option) (Q-G4) | **Accepted** |
| D-50 | **AI render is geometry-controlled**: the renderer outputs control buffers (depth, normals, edges, material / category / element masks, lighting) to pluggable diffusion providers (local, Ferro Cloud, third party); every AI image stores view, style, prompt and seed (Q-G7) | **Accepted** |
| D-51 | **Distributed plugins run in a WASM sandbox** (component model, capability permissions); Python scripting is for personal desktop automation; both change the model only through commands (Q-K1) | **Accepted** |
| D-52 | **Primary users: architects / designers in the concept stage**; architecture students are the growth channel; a guided homeowner mode may come later on the same core (Q-A2) | **Accepted** |
| D-53 | **Exact predicates for every topological decision** (orientation, inside / outside, intersection) using Shewchuk-style adaptive arithmetic; coordinates stay `f64`; tolerances are only for vertex merging (Q-C3) | **Accepted** |
| D-54 | **Constraints**: model-level relations (level, grid, host, alignment, equal spacing) solved in dependency order during regeneration; a full geometric constraint solver only inside sketch profiles and the family editor (Q-D1) | **Accepted** |
| D-55 | **Branching is core**: branch from any point, element-level diff shown visually, element-level merge with human conflict resolution, change proposals (PR-style) for humans and AI alike (Q-I3) | **Accepted** |
| D-56 | **IFC**: own Rust IFC4 writer in the open core (WASM-safe); full IFC import via an existing C++ library inside a plugin; track IFC5 (Q-J1) | **Accepted** |
| D-57 | **Revit live link, two-way**: thin C# add-in over `ferro-ffi`, local connection (socket / named pipe), elements matched by `Uid`; import levels, grids, walls, floors, rooms from existing Revit projects; support the three latest Revit versions (Q-J2) | **Accepted** |
| D-58 | **DXF read / write in the open core; native DWG through the ODA SDK in a Ferro Pro / Cloud plugin**, kept outside the MPL core (Q-J3) | **Accepted** |
| D-59 | **Scale**: architecture supports up to ~500k elements per document plus federation; optimisation targets houses and mid-size buildings first; urban / infrastructure scale is out of scope (Q-M2) | **Accepted** |
| D-60 | **Inference engine** is its own pure, tested crate (`ferro-snap`) with a specified priority order (points > edges / faces > directions > grid), snaps to BIM datums (levels, grids, reference planes), uses screen-space snap tolerance, accepts typed values at any time (`3m`, `4000,3000`, `*5`), and is usable by AI (Q-F2) | **Accepted** |
| D-61 | **Plugin permissions are explicit capabilities** (read model, write model, network per domain, files, AI calls), granted by the user at install; official-registry plugins are signed; all plugin edits go through commands (history + undo) (Q-K3) | **Accepted** |
| D-62 | **Performance budgets enforced in CI** (reference: 3-year-old integrated-GPU laptop): house ~2k elements 60 fps / open < 1 s; 20-storey ~100k elements 30 fps (60 fps discrete GPU) / open < 5 s; tool feedback < 16 ms; commit < 10–50 ms; undo < 100 ms; PRs that regress > 10 % are blocked (Q-M1) | **Accepted** |
| D-63 | **All other Decision Book questions (🟠 / 🟢) adopt the written recommendation** as the default; each becomes binding when its area is implemented and may be revisited by ADR (see `docs/decision-book/PHIEU-TRA-LOI.md`) | **Accepted** |
| D-64 | **SketchUp plugins**: no Ruby compatibility layer; the 20 most-used SketchUp plugins in Vietnam become native tools / AI skills, a SketchUp extension syncs to Ferro during migration, and AI helps port open-source Ruby plugins to WASM plugins (Q-R2) | **Accepted** |
| D-65 | **Ferro Link**: one open, versioned link protocol for external apps — the same command / query / event API used by AI and plugins (MCP over WebSocket + binary geometry stream); Speckle connector as a secondary bridge (Q-R3) | **Accepted** |
| D-66 | **Revit library reuse**: a Revit add-in batch-converts `.rfa` types to `.ffa` (geometry per Coarse / Medium / Fine, parameters, materials, category, preview) on the user's machine; AI re-parametrises common families; Ferro never redistributes converted content. Revit materials map to PBR `Material`s, Autodesk textures stay local (Q-R4, Q-R5) | **Accepted** |
| D-67 | **Bundled library**: open Ferro Standard Library + Vietnam region pack + **construction-system packs** (masonry + reinforced concrete, steel pre-engineered buildings, European light steel / timber frame with cladding and roofing; later CLT, modular, bamboo), each with types, families, details, materials and templates (Q-R6) | **Accepted** |
| D-68 | **Workspaces on one document**: Sketch, BIM Modelling, Render (render-app-like: assets, materials, sun, lights, cameras, scenes, post, AI render, video timeline), Presentation, Review; switching is instant and changes only UI, tools and viewport quality (Q-R7) | **Accepted** |
| D-69 | **Detail levels vs LOD**: three per-view display levels (Coarse / Medium / Fine) with per-category default rules (e.g. wall: single mass / core + finishes / all layers + details such as reveals, drips, weep holes) overridable in view templates and project templates; development LOD (100–400) is a separate element parameter with checks the AI can use to upgrade models (Q-R8) | **Accepted** |
| D-70 | **Three family kinds**: system families (walls, floors, roofs, ceilings, stairs, railings, curtain walls, levels, grids, rooms, ramps; types editable and transferable), loadable families (`.ffa`), Forms / components; plugins may register new system families (Q-R9) | **Accepted** |
| D-71 | **Components**: any Form can become a Form Type with instances (edit one, all update; make unique); exportable to `.ffa`; no separate group concept (Q-R10) | **Accepted** |
| D-72 | **Tool contract**: every tool is an app-side state machine (idle → picking → preview → commit; Esc cancels, Backspace steps back) using inference, previewing via repeated start / rollback of a transaction group, always accepting typed values, arrow-key axis locks and modifier variants, and ending in one named command (Q-R11) | **Accepted** |
| D-73 | **Semantic push-pull on BIM elements**: pulling a wall side moves its location line (Alt: changes thickness / type), the top face changes top offset, floor faces change elevation / thickness, window faces change width / height; shapes not expressible by parameters prompt "create a void or convert to Form" (Q-R12) | **Accepted** |
| D-74 | **No CAD layers**: visibility by category, parameter filters and user Tags (SketchUp tags import as Tags), plus worksets in collaboration (Q-R14) | **Accepted** |
| D-75 | **Import / export roadmap**: M1 `.fro` + OBJ / glTF export; M2 SKP and DXF import; M3 Revit live link, IFC export, 3DM, point clouds, PDF underlays, OSM, spreadsheets; M4 IFC import, DWG (Pro), BCF, USD / FBX, STL. All remaining Q-R recommendations (texture mapping, autosave, CAD links, copy / paste, templates, M1–M2 tool set) are adopted (Q-R1, Q-R13, Q-R15–R19) | **Accepted** |
| D-76 | **Foundation 2** (ADR 0004): element semantics separate from geometry source (any category may be free-form); quantity-typed parameters with `Uid` definitions; materials with appearance / physical / thermal / environmental / cost assets; body kinds incl. non-manifold and curve-ready surface types; relationships as data incl. non-destructive cuts; derived `ferro-scene`; data-driven commands and keymaps. Supersedes parts of D-11, D-39, D-41, D-70 | **Accepted** |
| D-77 | **Phases and design options** (ADR 0006): ordered phases with created / demolished per element, status derived per view, ArchiCAD-style simple mode, phased cuts; design options as layers of changes over the shared model (add / remove / override), combinations per view, accept = merge, options kept | **Accepted** |

## Status of the current code

Milestone 1 rebuilt the prototype on this spec (`docs/milestones/M1.md`):
`Point3` / `Vec3`, `Document` / `Element`, typed parameters, half-edge topology
with persistent ids and lineage, commands + transactions, `Uid`, levels and
origins are all in place. Milestone 2 (`docs/milestones/M2.md`, ADR 0001) added
walls, floors, doors and windows with types and materials, Ferro Link, AI
proposals and the in-app chat. Milestone 3 (`docs/milestones/M3.md`, ADR 0002)
added the two-way Revit link, IFC4 export and T-joins. Open items: explicit join
relationships, openings
in curved walls, design options and views as elements (D-32, D-33), exact
single curved faces (D-39), layer / glossary lints and CI performance budgets
(D-19, D-20, D-62).
