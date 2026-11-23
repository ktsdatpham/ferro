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
| `Level` | Named elevation | Storey, Floor (as a datum), Story |
| `Grid` | Named plan datum | Axis line |
| `ReferencePlane` | Named alignment plane | Guide, Construction plane |
| `ReferenceLine` / `ReferencePoint` | Alignment line / point (SketchUp guides) | Guide, Construction line, Construction point |
| `Dimension` | Model distance annotation | Measure, Dim (as a type) |
| `WorkPlane` | Active sketch plane | Sketch plane, Drawing plane |
| `Form` | Geometry edited directly (geometry source `Direct`); any category, e.g. a free-form wall | Mass, Group, Blob, Mesh element |
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
| `CompoundStructure` / `CompoundLayer` | Layers of a wall / floor type | Build-up, Layer (alone), Ply |
| `WallType` / `FloorType` | System family types | WallStyle, Assembly |
| `Opening` | Door or window hosted by a wall | Hole, Cutout, Insert (as a type) |
| `LocationLine` | Which line of a wall its curve follows | Justification, Alignment |
| Ferro Link | Local protocol for agents to drive the open app | Bridge, Socket API |
| Proposal | Agent changes awaiting the user's accept / reject | Draft, Suggestion (for this) |

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
  `extrude`). New verbs need a glossary entry.
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
