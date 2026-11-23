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
| Storage | `Quantity { spec, value }` (ADR 0004 §2; `Length`, `Angle`, `Area`, `Volume`, `Number` kept for older files), `Integer`, `Bool`, `Text`, `Tags`, `ElementRef` | `StorageType` + spec |
| Definition | `ParameterDefinition` with a `Uid`: built-in registry, or project definitions in `ProjectInfo` | shared parameter / binding |
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
- **Relationships are data** (ADR 0004 §5, M5-5): the document holds a
  relationship table next to its elements. A row (`Relationship`) has its own
  `Uid`, a `RelationshipKind` and the linked element ids, ordered per kind:

  | Kind | Elements | Revit / IFC |
  |---|---|---|
  | `join` | `[first, second]`; `first` wins where they overlap | Join Geometry, wall joins / `IfcRelConnectsPathElements` |
  | `cut` | `[cutter, cut]`; non-destructive void | Cut Geometry / `IfcRelVoidsElement` |
  | `host` | `[host, hosted]` | `FamilyInstance.Host` / `IfcRelFillsElement` |
  | `attach` | `[attached, target]` | Attach Top/Base |
  | `bounds` | `[space, boundary]` | Room Bounding / `IfcRelSpaceBoundary` |
  | `connect` | `[a, b]`; no geometric effect | `IfcRelConnectsElements` |
  | `aggregate` | `[whole, part, …]` | `IfcRelAggregates` |

  Rules: only elements with geometry take part; elements are distinct; one
  row per kind and pair (a reversed `join` is the same pair, a reversed `cut`
  is not); an optional `phase` says when the row is made (ADR 0006 §5).
  Deleting an element deletes its rows in the same transaction; a phase used
  by a row cannot be deleted except by merging. Files store rows with
  element uids (`relationships` in `.fro.json`, a `relationships` table in
  `.fro`, format 4); the change format addresses them as
  `/relationships/<uid>`. Commands: `create_relationship`,
  `delete_relationship`; query `list_relationships`. Failure codes:
  `ferro.model.invalid_relationship`, `ferro.model.duplicate_relationship`,
  `ferro.model.missing_relationship`, `ferro.model.missing_element`,
  `ferro.model.invalid_uid`.
- **Joins** (wall-to-wall, floor-to-wall) become `join` rows (M5-5.2). Until
  then, generation infers L- and T-joins from coincident wall ends (ADR 0001);
  hosted openings use `host_id` and are deleted with their host.
- **Groups** (later) are elements referencing members; a group is not a Form.

## 3.9 Out of scope for v1 (but reserved)

Worksets, views and sheets are Revit concepts Ferro will
need for collaboration and documentation (phases and design options landed,
§3.10, §3.11). They are **reserved names**: do not use
them for anything else. Documentation (views, sheets, annotation) is
intentionally left to Revit.

## 3.10 Phases (ADR 0006, D-77)

A `Phase` is an element of category `Phases` (Revit: `Phase`, `OST_Phases`)
with an `order` and optional ISO dates (`YYYY-MM-DD`, information only). The
project's phases are ordered by `order` (then id); commands keep `order`
equal to the position. New documents have **no** phases; the first time the
simple mode (`set_renovation_status`) is used, "Existing" and "New
Construction" are created (if one phase exists it plays "Existing" and only
"New Construction" is added). `create_phase` on an empty list creates only
the phase asked for.

Elements placed in the model (Forms, walls, floors, doors, windows) carry
`phase_created` and `phase_demolished` (Revit: *Phase Created*, *Phase
Demolished*); both must be phases and demolished must not be earlier than
created (`ferro.model.invalid_phase_order`). Levels, types, materials and
phases have no phases (`ferro.model.not_phased`) and are in every phase.

**Missing fields.** No `phase_created` means "created before every phase":
the element is *existing* in every phase until it is demolished. This is the
state of unphased data (imports, models drawn before phasing was used, scan
results — ADR 0006 §7 asks for the first phase, which shows the same). Revit
instead assigns the view's phase on creation; the app does the same while
phasing is used (new elements get the view's phase in the same undo step),
but the kernel commands never guess a phase.

**`PhaseStatus`** of an element in a view of phase *v* (pure function
`model::phase::phase_status`):

| Created | Demolished | Status | Shown |
|---|---|---|---|
| after *v* | any | `future` | no |
| any | before *v* | `past` (gone) | no |
| *v* | *v* | `temporary` | yes |
| before *v* / none | *v* | `demolished` | yes |
| *v* | after *v* / none | `new` | yes |
| before *v* / none | after *v* / none | `existing` | yes |

`past` is not in ADR 0006 §3 (which lists five); it names elements already
demolished in an earlier phase, which Revit does not show either.

Deleting a phase used by elements fails with `ferro.model.phase_in_use`;
`delete_phase` with `merge_into` (the phase just before or after,
`ferro.model.phase_not_adjacent` otherwise) moves its elements to that phase
first (Revit: combine with previous / next).

**Simple mode** (`set_renovation_status`, ADR 0006 §4) maps to the first and
last phase: `existing` = created first; `demolish` = created first,
demolished last; `new` = created last.

**Quantities by status** (`PhaseQuantities`, MCP `get_phase_quantities`):
per category and shown status, count, volume of closed bodies and area
(floors: plan area; walls: gross length × height; others: the
`ferro.dimensions.area` parameter).

**Interop.** Phases are not IFC products: IFC export shows the export phase
(the last by default), leaves out future / past elements and writes
`Pset_<Class>Common.Status` = `NEW` / `EXISTING` / `DEMOLISH` / `TEMPORARY`
(`PEnum_ElementStatus`). Revit: phases are looked up by name (the Revit API
cannot create them) and *Phase Created* / *Phase Demolished* are set by name.

Phased cuts (ADR 0006 §5) wait for the relationship table (M5-5).

## 3.11 Design options (ADR 0006, D-32)

An `OptionSet` and its `DesignOption`s are elements of category
`DesignOptions` (Revit: `OST_DesignOptions`). An option's set is its
`host_id`, so deleting a set deletes its options. Sets are ordered by
`order`; options within a set too. `create_option_set` makes the set with
its first option "Option 1", primary.

**Option layer.** An option holds a list of `Change`s in the change format
(M5-26): paths `/elements/<uid>...` and `/relationships/<uid>...` (a join
or cut that exists only in that option). A whole new element adds it, a
`remove` of `/elements/<uid>` hides a shared element in that option, a
property path overrides it (e.g. `/elements/<uid>/body/top/height`). Shared
elements are never copied. A layer never touches the project, option sets /
options or the `design_option` field, and stores no derived data
(regenerated geometry `body/geometry`, read-only computed parameters).
Layers are checked against the shared model on write
(`ferro.model.invalid_option_layer`).

**Status.** `open`; `accepted` (its layer was merged); `archived` (another
option of its set was accepted). Only open options are shown, edited or
primary; every set with open options has exactly one primary
(`ferro.model.invalid_design_options` otherwise). The primary option cannot
be deleted (`ferro.model.option_is_primary`).

**Combination.** One open option per set; sets not named show their primary;
sets without open options show nothing. `resolve(doc, combination)` (pure,
`ops::design_option_commands`) applies the chosen option layers in set order
and regenerates. Two option layers that touch the same property (or one removes
what another changes, `model::change::conflicts`) are reported as
`ferro.model.option_conflict`, never merged. In a resolved document every
option carries an empty layer (resolving again changes nothing) and each
element added by an option layer has `design_option` = that option. In the shared
model `design_option` is always `None`.

**Editing in an option.** `Session::execute_in_option(option, combination,
command)` runs any command on the document seen in the combination and
stores `diff(seen without this option, edited)` (minus derived data) as the
option layer, through one `set_design_option` (one undo step). Editing an
element that only another set's option adds fails with
`ferro.model.invalid_option_layer`. MCP: every command tool takes an
optional `in_design_option`.

**Accept** (`accept_design_option`): the option layer is applied to the shared model;
the option becomes `accepted`, the other open options of its set
`archived` (kept with their layers).

**Stale option layers.** A shared change that makes an open option layer fail (e.g.
deleting an element the option overrides) commits with a
`ferro.model.option_layer_stale` warning on the option; the option layer is kept and
`resolve` reports `ferro.model.invalid_option_layer` for it.
`model::change::partition_applicable` splits the edits that still apply,
for an explicit repair with `set_design_option`.

**Interop.** IFC exports the primary combination (the app: the
combination shown). Revit gets the primary combination, with a `warning`
operation when other open options exist (Revit design options are a later
step).
