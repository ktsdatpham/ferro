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
| D-01 | Internal length unit is the **millimetre**, stored as `f64` | Proposed |
| D-02 | Internal angle unit is the **radian**; degrees only at UI / MCP edges | Proposed |
| D-03 | Right-handed coordinates, **Z up**, X = project east, Y = project north, counter-clockwise positive | Proposed |
| D-04 | Three origins as in Revit: **Internal Origin** (fixed), **Project Base Point**, **Survey Point**, plus true-north angle | Proposed |
| D-05 | Model geometry must stay within **10 km** of the Internal Origin; rendering is camera-relative | Proposed |
| D-06 | Fixed global tolerances (`VERTEX_TOL` = 0.01 mm, `SHORT_CURVE_TOL` = 0.5 mm, `ANGLE_TOL` = 1e-6 rad); never compare `f64` with `==` | Proposed |
| D-07 | **`Point3` and `Vec3` are distinct types** (Revit's `XYZ` conflates them) | Proposed |
| D-08 | A `Plane` is a full frame: origin + X axis + Y axis (normal derived) | Proposed |
| D-09 | Curves are bounded and parameterised on **[0, 1]** in the public API | Proposed |
| D-10 | Profiles: outer `CurveLoop` counter-clockwise about the plane normal, holes clockwise | Proposed |
| D-11 | Solid topology is a **half-edge B-rep**; faces may have holes; solids are closed, manifold, outward-facing | Proposed |
| D-12 | **All geometry belongs to an element.** Free SketchUp-style editing happens inside a `Form` element; geometry never sticks across elements | Proposed |
| D-13 | Identity: `ElementId` (u64, document-local, never reused) **and** `Uid` (UUID v7, stable across save / export / Revit round-trip) | Proposed |
| D-14 | Classification: **Category → Family → Type → Instance**, as in Revit | Proposed |
| D-15 | Data lives in **typed parameters** (instance or type), not free-form string tags | Proposed |
| D-16 | **Levels** are elements; vertical position is expressed as level + offset | Proposed |
| D-17 | Every mutation is a **Command** executed inside a **Transaction**; MCP tools are commands 1:1 | Proposed |
| D-18 | After commit, a **dependency graph** regenerates dependents; failures are Warning (commit allowed) or Error (rollback) | Proposed |
| D-19 | `ferro-core` is pure: no IO, no UI, no global mutable state, deterministic | Proposed |
| D-20 | Use Revit's term whenever the concept matches; never invent synonyms (see glossary) | Proposed |
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

## Status of the current code

The v0.1 prototype predates this spec. Known gaps, to be closed in milestone 1:

- `Vec3` is used for points → split into `Point3` / `Vec3` (D-07).
- `Model` / `Entity` / `EntityKind` → `Document` / `Element` / categories (D-13, D-14, D-20).
- Free-form `tags` → typed parameters (D-15).
- `Face` / `Solid` as vertex lists → half-edge topology (D-11).
- Mutations are direct method calls → commands + transactions (D-17).
- No `Uid`, no levels, no origins (D-04, D-13, D-16).
