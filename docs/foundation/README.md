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
| 6 | [Interop](06-interop.md) | `.ferro` file format, MCP, Revit and IFC mapping |
| 7 | [Architecture and layering](07-architecture.md) | Crates, layers, dependency rules |

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
| D-21 | `.ferro` files are versioned (`format_version`), JSON in v0, with stable keys | Proposed |

## Status of the current code

The v0.1 prototype predates this spec. Known gaps, to be closed in milestone 1:

- `Vec3` is used for points → split into `Point3` / `Vec3` (D-07).
- `Model` / `Entity` / `EntityKind` → `Document` / `Element` / categories (D-13, D-14, D-20).
- Free-form `tags` → typed parameters (D-15).
- `Face` / `Solid` as vertex lists → half-edge topology (D-11).
- Mutations are direct method calls → commands + transactions (D-17).
- No `Uid`, no levels, no origins (D-04, D-13, D-16).
