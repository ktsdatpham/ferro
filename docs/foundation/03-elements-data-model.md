# 3. Elements and data model

This chapter follows Revit's data model closely. If you know the Revit API,
almost every term here means what you expect.

## 3.1 Document

`Document` is the single container of a project (Revit: `Document`). It owns:

- all elements, keyed by `ElementId`
- project settings (`ProjectUnits`, `ProjectInfo`, origins and north, chapter 1)
- the undo history and the transaction state (chapter 4)
- the dependency graph used for regeneration

There is no global document. Every API that touches elements takes `&Document`
or works inside a `Transaction`.

## 3.2 Element and identity (D-13)

Everything that exists in the model and can be selected, scheduled or exported is
an **Element**.

| Field | Type | Meaning | Revit |
|---|---|---|---|
| `id` | `ElementId(u64)` | Fast, document-local id. Never reused within a document | `ElementId` |
| `uid` | `Uid` (UUID v7) | Stable, globally unique. Survives save, copy between files, export, Revit round-trip | `UniqueId` |
| `category` | `Category` | What kind of thing it is | `Category` |
| `type_id` | `Option<ElementId>` | Its `FamilyType`, if it has one | `GetTypeId()` |
| `name` | `Option<String>` | User-visible name | `Name` |
| `parameters` | `ParameterSet` | Typed data (3.5) | `Parameters` |
| `location` | `Location` | Where it is (3.7) | `Location` |
| `host_id` | `Option<ElementId>` | Hosting element (3.8) | `Host` |
| `level_id` | `Option<ElementId>` | Associated level (3.6) | `LevelId` |
| `body` | `ElementBody` | Category-specific data and geometry | — |

Rules:

1. `ElementId` is for runtime references; `Uid` is for anything that leaves the
   process (files, MCP results the agent may store, Revit / IFC export).
2. Copying an element creates a new `ElementId` **and** a new `Uid`.
3. References between elements use `ElementId` in memory and are rewritten to
   `Uid` on save.

## 3.3 Classification: Category → Family → Type → Instance (D-14)

| Level | Meaning | Example | Revit |
|---|---|---|---|
| **Category** | Fixed classification, defined by Ferro | `Walls`, `Floors`, `Doors`, `Rooms`, `Levels`, `Forms` | `BuiltInCategory` |
| **Family** | A definition of how a kind of thing is built and behaves | "Basic Wall", "Single Flush Door" | `Family` |
| **Type** (`FamilyType`) | A named set of type parameters within a family | "Interior 100 mm Partition" | `ElementType` / `FamilySymbol` |
| **Instance** | A placed element of a type | The wall on grid B | `FamilyInstance`, `Wall`, … |

Family kinds:

| Kind | Meaning | Revit |
|---|---|---|
| **System family** | Built into Ferro, geometry generated from parameters + profile/curve | Wall, Floor, Roof, Ceiling |
| **Loadable family** | Defined in a separate file, placed as instances (later milestone) | `.rfa` families |
| **Form** | Free-form geometry edited directly with SketchUp/Rhino-style tools | In-place mass / generic model |

Milestone 2 implements the system families `Wall` and `Floor` with `WallType`
/ `FloorType` (a `CompoundStructure` of `CompoundLayer`s), `Material`, and the
hosted `Opening` (door / window). Types and materials are elements. See ADR 0001.

A `Material` (ADR 0004 §3) has a colour (OpenPBR `base_color`), a roughness
(`specular_roughness`) and optional assets: `appearance` (further OpenPBR
parameters, image mappings), `physical` (density, Young's modulus, Poisson
ratio, strengths, thermal expansion), `thermal` (conductivity, specific heat,
density, emissivity, solar absorptance, vapour resistance factor),
`environmental` (GWP per EN 15804 stage, declared unit, EPD source) and
`cost` (unit price, ISO 4217 currency, unit). Asset values are SI with the
unit in the field name (`conductivity_w_mk`); unknown values are absent,
never zero. A wall / floor type derives `StructureQuantities` from its
layers when read (not stored, since they depend on the materials):
U-value by ISO 6946 (Rsi 0.13 / 0.10 / 0.17 m²·K/W for horizontal / upward /
downward heat flow, Rse 0.04; wall types horizontal, floor types downward),
mass, GWP A1–A3 and cost per m². A quantity is only given when every layer
has the data; the layers that lack it are listed. Layers thinner than
`VERTEX_TOL` add no resistance or mass, but per-m² GWP and prices apply.

`Category` is a closed Rust `enum` (adding one is a spec change). Each category
declares its Revit `BuiltInCategory` and IFC class (chapter 6).

## 3.4 Forms: where free modelling lives (D-12)

The designer-first workflow happens inside **`Form`** elements:

- A `Form` owns an editable half-edge body. Push-pull, lines, faces, offset all
  operate *inside* one Form.
- **Geometry never sticks across elements.** This is SketchUp's "group" rule made
  the default, and avoids SketchUp's sticky-geometry mess.
- A Form can be **classified** later: assigning a category (`Walls`, `Floors`,
  `Rooms` …) plus parameters turns sketch geometry into BIM data. Converting a
  Form into a true system family (a real `Wall` with a location curve) is a
  separate, explicit command.

This is the bridge between "design freely" and "Revit-grade data".

## 3.5 Parameters (D-15)

All data on elements is held in **typed parameters**, never free-form strings.

| Property | Values | Revit |
|---|---|---|
| Key | Stable string id, e.g. `ferro.wall.unconnected_height` | `BuiltInParameter` / shared GUID |
| Storage | `Length`, `Angle`, `Area`, `Volume`, `Number`, `Integer`, `Bool`, `Text`, `ElementRef` | `StorageType` + spec |
| Scope | **Instance** or **Type** | instance / type parameter |
| Origin | **Built-in** (defined by Ferro), **Shared** (defined in a shared parameter file, has a GUID), **Project** (user, this document only) | same |
| Read-only | `true` when computed (e.g. `area`) | `IsReadOnly` |

Rules:

1. Built-in keys are namespaced `ferro.<category>.<name>` in `snake_case` and
   are **never renamed** once released.
2. Shared parameters carry a GUID so they can be matched to Revit shared
   parameters.
3. A type parameter value is read through the instance (`element.param(key)`
   falls back to the type), exactly like Revit.
4. Computed parameters (area, volume, length) are produced by regeneration, never
   written by commands.

## 3.6 Datums: levels, grids, reference planes (D-16)

Datums are elements that other elements are constrained to.

| Element | Defines | Revit |
|---|---|---|
| `Level` | A named elevation (Z) | `Level` |
| `Grid` | A named vertical plane from a line / arc in plan | `Grid` |
| `ReferencePlane` | A named infinite plane used for alignment and as a work plane host | `ReferencePlane` |
| `WorkPlane` | The plane sketches are drawn on (hosted by a level, reference plane or face) | `SketchPlane` |

Rules:

1. Vertical position of building elements is **level + offset**
   (`base_level`, `base_offset`, `top_level`, `top_offset`), never a raw Z alone.
   Moving a level moves everything constrained to it (via regeneration).
2. A `Form` may be unconstrained (pure sketch) or attached to a level.

## 3.7 Location

| Variant | Used by | Revit |
|---|---|---|
| `Location::None` | Datums with own geometry, Forms | — |
| `Location::Point { point, rotation }` | Furniture, columns, doors | `LocationPoint` |
| `Location::Curve { curve }` | Walls, beams, grids | `LocationCurve` |

Moving an element means changing its `Location` (or its body, for Forms) through
a command. Geometry is derived from location + parameters + type.

## 3.8 Hosting and relationships

- A **hosted** element (door, window, opening) has `host_id` pointing to its host
  (wall, roof, floor). Deleting the host deletes or orphans hosted elements per
  category rule, reported as a failure (chapter 4).
- **Joins** (wall-to-wall, floor-to-wall) are explicit relationships stored on
  the document, not hidden geometry hacks. Until those land, milestone 2 infers
  L-joins from coincident wall ends (ADR 0001); hosted openings are deleted
  with their host.
- **Groups** (later) are elements referencing members; a group is not a Form.

## 3.9 Out of scope for v1 (but reserved)

Phases, design options, worksets, views and sheets are Revit concepts Ferro will
need for collaboration and documentation. They are **reserved names**: do not use
them for anything else. Documentation (views, sheets, annotation) is
intentionally left to Revit.
