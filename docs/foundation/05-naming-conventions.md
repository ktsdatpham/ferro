# 5. Naming and code conventions

Consistent names are what let many humans and AI agents work in one codebase.
These rules are enforced in review.

## 5.1 Glossary (D-20)

Use the **left column**. The right column lists words that must **not** be used
for the same concept.

| Use | Meaning | Don't use |
|---|---|---|
| `Document` | The project container | Model, Scene, Project (as a type) |
| `Element` | Anything in the document with identity | Entity, Object, Node, Item |
| `ElementId` | Document-local id | Handle, Key, Index |
| `Uid` | Stable global id | Guid (as a name), UUID (as a name), PersistentId |
| `Category` | Fixed classification | Kind, Class, Layer |
| `Family` / `FamilyType` | Definition / named type | Template, Style, Preset, Component (for this) |
| `Parameter` | Typed data on an element | Property, Attribute, Tag, Field |
| `ParameterDefinition` | What a parameter key means (data type, group, binding, mapping), with a `Uid` (Revit shared parameter) | ParamDef, Schema, Field definition |
| `QuantitySpec` | Physical quantity of a value (length, density, U-value…), Revit `SpecTypeId` | UnitType, Measure (as a type), Dimension (for this) |
| `DisplayUnit` | A unit a quantity is typed in or shown in | Unit (alone), DisplayFormat |
| `Level` | Named elevation | Storey, Floor (as a datum), Story |
| `Grid` | Named plan datum | Axis line |
| `ReferencePlane` | Named alignment plane | Guide, Construction plane |
| `ReferenceLine` / `ReferencePoint` | Alignment line / point (SketchUp guides) | Guide, Construction line, Construction point |
| `Dimension` | Model distance annotation | Measure, Dim (as a type) |
| `WorkPlane` | Active sketch plane | Sketch plane, Drawing plane |
| `Form` | Geometry edited directly (geometry source `Direct`); any category, e.g. a free-form wall | Mass, Group, Blob, Mesh element |
| `ProjectUnits` | How quantities are shown and typed in a project (display only) | Units settings, Display settings |
| `SiteLocation` | The project's place on Earth: latitude, longitude, elevation, UTC offset, weather file | Location (that is an element's placement), Geo |
| `GeometrySource` | Where an element's geometry comes from: `Parametric` (generated from parameters) or `Direct` | Mode, Kind, Origin (for this) |
| `Profile` | Outer loop + holes | Shape, Outline, Polygon (for this) |
| `CurveLoop` | Closed connected curves | Polyline, Path, Ring |
| `Point3` / `Vec3` | Location / displacement | XYZ, Pos, V3 |
| `Transform` | Affine transform | Matrix (as a name), Xform |
| `Transaction` | Atomic change | Batch, Edit session |
| `Command` | Serialisable intent | Action, Operation, Op |
| `Failure` | Reported problem | Issue, Problem |
| `Regenerate` | Rebuild dependents | Update, Refresh, Recompute |
| `Host` / hosted | Containment relation | Parent / child (for this) |
| `Material` | Appearance + physical data, an element | Texture, Shader (for this) |
| `AppearanceAsset`, `PhysicalAsset`, `ThermalAsset`, `EnvironmentalAsset`, `CostAsset` | Optional data groups of a `Material` (Revit: Appearance / Physical / Thermal asset); appearance follows OpenPBR | Properties, Shader, Profile (for this) |
| `ImageMapping` | An image laid on faces at a real-world tile size (mm) | Texture (as a type), UV map |
| `MaterialUnit` | Unit a GWP figure or price refers to: m³, kg or m² of material (EPD "declared unit") | Measure, Uom |
| `StructureQuantities` | U-value, mass, GWP and cost per m² of a `CompoundStructure`, from its layer materials | Build-up data, Assembly properties |
| `CompoundStructure` / `CompoundLayer` | Layers of a wall / floor type | Build-up, Layer (alone), Ply |
| `WallType` / `FloorType` | System family types | WallStyle, Assembly |
| `Opening` | Door or window hosted by a wall | Hole, Cutout, Insert (as a type) |
| `LocationLine` | Which line of a wall its curve follows | Justification, Alignment |
| Ferro Link | Local protocol for agents to drive the open app | Bridge, Socket API |
| `Plugin` | A folder in `~/.ferro/plugins/` with a `PluginManifest` and Python code on the SDK; runs with the user's rights (ADR 0009) | Add-in, Addon, Extension, Package (for this) |
| `PluginManifest` | A plugin's `plugin.json`: id, name, version, author, description and its `PluginCommand`s | Descriptor, Plugin config, Package file |
| `PluginCommand` | An entry a plugin adds to the palette, the Plugins menu, the left panel and the keymap; runs Python as one script run (it is not a `Command`: the tools the script calls are) | Plugin action, Button (as a type), Macro |
| Proposal | Agent changes awaiting the user's accept / reject | Draft, Suggestion (for this) |
| Pin / `pinned` | An element's flag that keeps it in place (Revit `Element.Pinned`, ADR 0008): a pinned element cannot be moved or deleted (`ferro.model.pinned`), every other edit stays allowed; verbs *pin* / *unpin*, command `set_pinned` | Lock (as a type, field or command), Freeze, Protect (for this) |
| `Phase` | A construction stage (Revit `Phase`), an element of category `Phases`; ordered | Stage, Period (for this) |
| `PhaseStatus` | Status of an element in a view's phase: existing, new, demolished, temporary, future, past (derived, never stored) | RenovationState, Lifecycle |
| `RenovationStatus` | Simple-mode choice existing / demolish / new, mapped to the first and last phase (ArchiCAD renovation status) | Phase type, Status (alone) |
| `PhaseQuantities` | Volumes and areas per category split by `PhaseStatus` in one phase | Phase schedule, Demolition list |
| `Relationship` | A row of the document's relationship table: a `Uid`, a `RelationshipKind` and the linked elements in kind order (ADR 0004 §5) | Link, Association, Connection (alone), Constraint |
| `RelationshipKind` | `join`, `cut`, `host`, `attach`, `bounds`, `connect`, `aggregate` | RelationType, LinkType |
| `View` (`View3D`, `PlanView`, `SectionView`) | An element of category `Views` (Revit `View`, `OST_Views`, D-33): camera, cut, display style, hidden elements / tags, phase and phase filter; `ViewType` says which kind | Viewport, Camera (as an element), Scene (for this), Sheet |
| `ViewCamera` | A view's eye, target, up direction, projection (perspective / parallel) and vertical field of view | Viewpoint, Eye (as a type) |
| `SectionPlane` / `SectionBox` | A plane cutting a view (a point and the normal of the side cut away) / a 3D view's box limiting what is drawn (Revit *Section Box*) | Clip plane, Cutting plane (as a type), Crop box (for this) |
| `ViewRange` | A plan view's top, cut height, bottom and view depth as offsets from its level (Revit *View Range*) | Cut settings, Plan range |
| `DisplayStyle` | How a view draws: shaded, X-ray, wireframe, hidden line (Revit `DisplayStyle`); presentation styles are `RenderStyle` (D-49) | Visual style, Render mode, Shading mode |
| `PhaseFilter` / `PhaseGraphics` | A view's choice of which `PhaseStatus` values are shown and how (Revit *Phase Filter*: show all, show complete, show new…) / normal, overridden (status graphics) or hidden | Phase view mode, Phase visibility mode |
| `OptionSet` | A set of mutually exclusive design alternatives (Revit *Option Set*), an element of category `DesignOptions`; ordered | Option group, Variant set |
| `DesignOption` | One alternative of an `OptionSet` (Revit `DesignOption`): an **option layer** over the shared model; hosted by its set; one open option per set is primary | Variant, Alternative (as a type), Branch |
| Option layer | The list of `Change`s a `DesignOption` holds (add elements, remove shared elements, override properties); always qualified ("option layer"), never "layer" alone | Overlay, Patch, Delta |
| `OptionStatus` | `open` / `accepted` (merged into the shared model) / `archived` (another option of the set was accepted; kept) | Option state, Lifecycle |
| `Combination` | One design option per option set, primary by default (ArchiCAD design option combination); what a view shows | Configuration, Scenario, Variant |
| `Change` | One edit in the change format (M5-26): set or remove the value at a uid-keyed JSON-pointer path | Patch, Delta, Diff (for one edit) |
| `Kernel` | The B-rep library behind Ferro's seam (booleans, later blends and curved geometry): a trait in `ferro_core::algo`, implemented by `ferro-kernel` on solidcore (ADR 0007) | Engine, Backend, Modeler, CAD core |
| `BooleanOp` | Which boolean a `Kernel` computes: `union`, `difference`, `intersection`; a value, not a `Command` | BooleanType, CsgOp, Operation |
| `DisplayScene` / `DrawRecord` | Display data derived from a `Document` snapshot (`ferro-scene`, ADR 0004 §6) / what to draw for one element; never saved | Scene (alone, as a type), Render model, Node |

Adding a term requires adding it here first.

## 5.2 Types

- `PascalCase` nouns, matching the glossary.
- One concept per type. No "manager", "helper", "util" or "data" suffixes
  (`WallManager`, `GeomUtils` are rejected).
- Error types: one `enum` per module boundary, named `<Area>Error`
  (`GeomError`, `CommandError`).

## 5.3 Functions

Follow Rust API guidelines, plus:

| Pattern | Use for | Example |
|---|---|---|
| `new(...)` | Infallible constructor of a plain value | `Point3::new(x, y, z)` |
| `from_<inputs>(...)` | Constructor that names its inputs; returns `Result` if it can be degenerate | `Line::from_points(a, b) -> Result<Line, GeomError>`, `Arc::from_three_points(..)`, `Plane::from_origin_normal(..)` |
| `<noun>()` | Getter (no `get_` prefix) | `level.elevation()` |
| `set_<noun>(..)` | Setter on a value type (documents mutate only via commands) | `profile.set_plane(..)` |
| `is_<adj>()`, `has_<noun>()` | Boolean queries | `loop_.is_closed()`, `face.has_holes()` |
| `to_<x>()` / `into_<x>()` / `as_<x>()` | Conversions (cost / ownership per Rust conventions) | `plane.to_local(p)` |
| `<verb>ed()` | Returns a modified copy | `curve.reversed()`, `solid.transformed(&t)` |
| `<verb>()` on `&mut self` | Modifies in place | `profile.reverse()` |
| `try_<verb>()` | Only when an infallible `<verb>()` also exists | — |

Units are carried by types (`Length`, `Angle`). When a raw number must appear at
an edge (FFI, MCP JSON, file), the name carries the unit: `height_mm`,
`rotation_deg`.

## 5.4 Algorithms

- Free functions in `algo::<verb>` modules.
- Name: `<verb>_<a>_<b>`, arguments in the **same order as the name**:
  `intersect_line_plane(&line, &plane)`, `distance_point_curve(p, &curve)`.
- Pure: no document, no IO, no global state. Inputs by reference, outputs by
  value.
- Results with several outcomes return a dedicated `enum` (see chapter 2).
- Each algorithm documents: inputs' preconditions, tolerance used, complexity, and
  a reference (paper / book / Revit behaviour) when non-trivial.

## 5.5 Commands and MCP tools

- `verb_noun` snake_case, identical for the Rust `Command::NAME` and the MCP tool:
  `create_level`, `create_rectangle`, `push_pull_face`, `set_parameter`,
  `move_elements`, `delete_elements`.
- Standard verbs only: `create`, `delete`, `move`, `rotate`, `copy`, `mirror`,
  `array`, `set`, `get`, `list`, `join`, `unjoin`, `host`, `classify`, `convert`,
  `import`, `export`, plus geometric verbs (`push_pull`, `offset`, `split`,
  `extrude`, `subtract`: a solid boolean difference, ADR 0007). New verbs
  need a glossary entry.
- Verbs of the change format and design options (ADR 0006): `accept`
  (merge a proposal or a design option into the shared model:
  `accept_design_option`), `apply` (replay a change list: `apply_changes`),
  `diff` (compute a change list between two documents: `diff_documents`).
- Verbs of scripts (ADR 0005): `run` (start a script: `run_python`),
  `cancel` (stop a running script and roll it back: `cancel_python`).
- Plural noun when the command takes many ids (`delete_elements`).

## 5.6 Parameters and failure codes

- Parameter keys: `ferro.<category>.<name>`, e.g. `ferro.wall.base_offset`.
- Failure codes: `ferro.<area>.<problem>`, e.g. `ferro.geom.short_curve`.
- Both are `snake_case`, stable forever once released.

## 5.7 Files and modules

- One concept per file, file named after it: `geom/plane.rs`, `topo/half_edge.rs`,
  `algo/intersect.rs`, `model/level.rs`, `ops/transaction.rs`.
- `mod.rs` / `lib.rs` only re-export; no logic.

## 5.8 Tests

- Name: `<behaviour>_<condition>`: `push_pull_face_keeps_normals_outward`,
  `line_from_points_rejects_short_curve`.
- Every algorithm has: a basic case, a degenerate case (zero length, parallel,
  coincident), and a tolerance-edge case.
- Geometry invariants (`Solid::validate`) run after every operation in tests.
- Property-based tests (`proptest`) for core math are encouraged.

## 5.9 Formatting, lint, docs

- `cargo fmt`, `cargo clippy -- -D warnings` must pass (CI enforces).
- Every public item has a doc comment saying **what**, **units**, and **failure
  cases**. Use Revit terms in docs and link the Revit equivalent when useful.
- Code, comments, identifiers and docs are in **English**.
