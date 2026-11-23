# 0004. Foundation 2: semantics vs geometry, quantities, materials, relationships, scene

- **Status:** Accepted (2026-11-23, owner approval of the foundation review)
- **Affects:** D-01, D-11, D-12, D-15, D-18, D-30, D-36, D-38, D-39, D-41,
  D-48, D-70; chapters 2, 3, 4, 6, 7
- **Background:** [`docs/review/2026-11-ra-soat-mong.md`](../review/2026-11-ra-soat-mong.md) (Vietnamese)

## Context

The foundation review compared the accepted decisions with Revit, IFC, USD,
SketchUp, Rhino, Parasolid, Unreal and energy / point-cloud tools. Six
changes must land before more features are built on top. Later, they
would force rewrites of every exporter, tool and renderer.

## Decision

### 1. Element = semantics + geometry source + representations (F1)

1. An element's *semantics* (category, type, parameters, level, host,
   tags) are separate from its *geometry source*:
   - `Parametric(generator)`: wall from location line and type, floor from profile…
   - `Direct(body)`: B-rep edited directly (SketchUp style).
   - `Mesh(asset)`: imported mesh.
   - `Reality(asset, region)`: part of a point cloud or splat.
2. Every non-datum category accepts every geometry source. A free-form
   wall is a `Wall` with a `Direct` source; it keeps its type, materials and
   parameters, and its quantities come from the solid.
3. `Parametric → Direct` is always allowed (freeze). `Direct → Parametric`
   only when the shape is recognised.
4. Regeneration derives *representations*: Body (Coarse / Medium / Fine),
   Axis, Footprint, Analytical, EnergySurfaces, Collider.
5. Revit export: Parametric → native elements. Direct / Mesh → a
   **loadable family** of the element's category with one instance
   (default), or a `DirectShape` (option). In-place families are not
   possible through the Revit API.
6. IFC export: the element's class, with an extrusion (Parametric) or
   `IfcAdvancedBrep` / `IfcFacetedBrep` (Direct) body.
7. Commands `set_category` and `set_geometry_source` replace
   `convert_form_to_walls`.

### 2. Quantity-typed parameters (F2)

1. `ParamValue::Quantity { spec, value }` with `spec` from a fixed table of
   physical quantities (length, area, volume, angle, mass, density, force,
   stress, thermal conductivity, thermal resistance, U-value, heat capacity,
   temperature, power, illuminance, flow, pressure, currency, carbon mass,
   time, ratio…), each mapped 1:1 to Revit `SpecTypeId` and IFC measures.
2. Values are stored in SI, except length (mm, D-01) and angle (rad, D-02).
3. Parameter definitions are data with a `Uid` (like Revit shared
   parameters): key, spec, group, instance / type, description, Revit and
   IFC Pset mapping. Round-trips match by `Uid`, not by name.

**Amendment (M5-3, 2026-11-23).** How §2 is built:

- The table is `QuantitySpec` (`ferro_core::math`); its stable ids are the
  snake_case names above plus `moment`, `pressure`, `number`. Each has a
  storage unit, a Revit `SpecTypeId` name, an IFC measure and display units
  (`DisplayUnit`, `storage = value × factor + offset`; only temperature has
  an offset). Temperature is stored in K. Revit has no carbon or ratio spec
  (they map to `Mass` / `Number`); IFC has no stress or carbon measure
  (`IfcPressureMeasure` / `IfcMassMeasure`). Reverse lookups take the first
  spec in table order.
- Text input is read by one function, `QuantitySpec::parse(text, expected)`.
  A bare number is in the spec's default display unit (mm, degrees, °C …),
  as with `Length::parse` / `Angle::parse`. When a unit belongs to several
  specs (`Pa`, `kPa`) and none is expected, the first in table order wins
  (pressure before stress).
- `ParamValue::Length / Angle / Area / Volume / Number` stay so older files
  load; `ParamValue::spec()` / `as_quantity()` report their spec. New keys
  use `ParamValue::Quantity`.
- Non-length, non-angle quantities compare with the relative tolerance
  `tol::QUANTITY_REL_TOL` (1e-9).
- `ParameterDefinition` origins follow §3.5: **built-in** definitions
  (`ferro.*` keys) come from a fixed registry with `Uid =
  Uid::from_name("ferro.parameter_definition:" + key)` (name-based UUID v8,
  128-bit FNV-1a; never changes); **project** definitions are stored in
  `ProjectInfo.parameter_definitions` (omitted when empty, so older files
  load unchanged). A Revit shared parameter keeps its GUID as `Uid`.
- New command / MCP tool `set_parameter_definition` (adds or replaces a
  project definition). `set_parameter` checks values against the key's
  definition. New failure codes: `ferro.model.invalid_parameter_value`,
  `ferro.model.parameter_type_mismatch`,
  `ferro.model.builtin_parameter_definition`,
  `ferro.model.invalid_parameter_definition`, `ferro.model.duplicate_uid`.

### 3. Materials with assets (F3)

`Material` holds optional assets: `appearance` (OpenPBR parameters, texture
mapping), `physical`, `thermal`, `acoustic`, `environmental` (GWP per stage,
EPD source), `cost`. `CompoundStructure` derives U-value, mass and carbon
per area from them.

### 4. Geometry ready for curves and non-manifold Forms (F4)

1. Body kinds: `Solid`, `Sheet`, `Wire`, `General` (non-manifold). Forms
   default to `General`; building elements need a `Solid` to report volume.
2. Topology moves to a radial-edge structure (an edge may have any number
   of faces); half-edge solids are the manifold case.
3. The `Surface` / `Curve` types include Plane, Cylinder, Cone, Sphere,
   Torus, NURBS and Line, Arc, Ellipse, NURBS now. The file format and the
   IFC / Revit writers carry them, even where algorithms come later.
4. The curved-surface kernel is decided after a spike evaluating
   `truck` / `monstertruck` (Rust, WASM-capable) against OCCT in a plugin.
   No new `ferro-core` dependency without a follow-up ADR.

### 5. Relationships are data (F5)

A typed relationship table in the document, with `Uid`s, part of
regeneration: `Join`, `Cut` (non-destructive void), `Host`, `Attach`,
`Bounds` (room bounding), `Connect`, `Aggregate`. Inside a Form editing
stays direct (D-38); between elements cuts are relationships.

### 6. Scene derived from the document (F6)

New crate `ferro-scene`: an incremental, ECS-style scene derived from the
`Document` (meshes with LOD, instances, compiled materials, lights,
colliders, reality-data nodes). Rendering, picking, physics, analysis and
USD / glTF export read the scene; agents keep reading the document.

### 7. Commands and keymaps (F11)

Commands are registered as data (stable name, translated label, context).
Shortcuts come from a user-editable keymap. Presets: SketchUp, Revit,
and **Ferro (default)**: one-key SketchUp shortcuts for modelling, Revit
two-letter codes for BIM elements, a command palette for everything.

## Alternatives considered

See the review. Keeping `ElementBody` as one enum and adding "convert"
commands keeps Revit's in-place limitation. Adding physical units per
plugin leaves round-trips unmappable. Building the curved kernel first
delays everything else. Keeping the app-side scene cache cannot meet D-62.

## Consequences

- `.fro` `format_version` bumps; files from M1–M4 are migrated on open.
- `ElementBody`, `ParamValue`, `Material`, `Body` and the IFC / Revit
  writers change shape; MCP tool names stay (stable strings), new tools
  are added.
- The rest of milestone 4 (reference lines, dimensions, per-face paint,
  components) is built on this foundation in milestone 6.
