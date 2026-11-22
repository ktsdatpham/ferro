# 2. Geometry

Geometry types are **values**: immutable, `Copy` where small, no identity, no
document. Identity belongs to elements (chapter 3). The layering is:

```
math     Point2, Point3, Vec2, Vec3, UnitVec3, Transform, BoundingBox3, tol
geom     Plane, Line, Arc, (Ellipse, NurbsCurve later), Curve, CurveLoop, Profile
topo     Vertex, HalfEdge, Edge, Loop, Face, Shell, Solid   (half-edge B-rep)
algo     intersect, distance, project, offset, triangulate, boolean, extrude ...
```

## 2.1 Points and vectors (D-07)

| Type | Meaning | Revit |
|---|---|---|
| `Point3` | A location in space | `XYZ` |
| `Vec3` | A displacement / direction with length | `XYZ` |
| `UnitVec3` | A direction, guaranteed length 1 | `XYZ` (normalised) |
| `Point2`, `Vec2` | Same, in a 2D parameter or sketch space | `UV` |

Revit uses one type (`XYZ`) for all three, which allows nonsense such as adding
two points. Ferro makes it a compile error:

| Expression | Result |
|---|---|
| `Point3 - Point3` | `Vec3` |
| `Point3 + Vec3` | `Point3` |
| `Vec3 + Vec3`, `Vec3 * f64` | `Vec3` |
| `Point3 + Point3` | **does not compile** |
| `Vec3::normalize()` | `Option<UnitVec3>` (None if near zero) |

Weighted combinations of points (midpoint, centroid) go through explicit
functions: `Point3::midpoint(a, b)`, `Point3::centroid(&[...])`.

## 2.2 Transform

`Transform` is an affine transform: 3×3 linear part + translation (Revit:
`Transform` with `BasisX/Y/Z` and `Origin`).

- `Transform::IDENTITY`, `::translation(v)`, `::rotation(axis_origin, axis, angle)`,
  `::scale_about(point, factor)`, `::mirror(plane)`, `::from_frame(&Plane)`.
- `t.apply(point)`, `t.apply_vec(vec)` (no translation), `t.apply_dir(unit)`.
- Composition: `a.then(&b)` means "apply `a`, then `b`". (We avoid `*` because
  the order is a classic source of bugs.)
- `t.inverse() -> Option<Transform>`; `t.is_conformal()` (no shear / non-uniform
  scale); `t.has_reflection()`.

## 2.3 Plane and work plane (D-08)

A `Plane` is a **frame**, not just origin + normal:

```
Plane { origin: Point3, x_axis: UnitVec3, y_axis: UnitVec3 }   // normal = x × y
```

The in-plane axes matter: they define 2D sketch coordinates, the orientation of
text and hatches, and which way "counter-clockwise" is. Revit's `Plane` has
`Origin`, `XVec`, `YVec`, `Normal` for the same reason.

- Constructors: `Plane::XY` (world ground), `Plane::from_origin_normal(o, n)`
  (picks x axis by a documented rule), `Plane::from_frame(o, x, y)`,
  `Plane::from_three_points(a, b, c) -> Result`.
- Queries: `normal()`, `signed_distance_to(p)`, `project(p) -> Point3`,
  `to_local(p) -> Point2`, `to_world(uv) -> Point3`.

A **`WorkPlane`** is an *element* (chapter 3) that owns a `Plane` and a name, and
may be hosted by a level, a reference plane or a face. Sketching tools always
draw on the active work plane. Revit: `SketchPlane`.

## 2.4 Curves (D-09)

All curves are **bounded** (no infinite lines in the model; infinite helpers
live only inside algorithms).

| Type | Definition | Revit |
|---|---|---|
| `Line` | start, end | `Line.CreateBound` |
| `Arc` | plane (frame), radius, start angle, end angle | `Arc` |
| `Ellipse` (later) | plane, two radii, angles | `Ellipse` |
| `NurbsCurve` (later) | degree, control points, weights, knots | `NurbSpline` |
| `Curve` | `enum` over the above | `Curve` |

Common interface (trait `CurveGeom`):

- `start()`, `end()`, `length()`, `is_closed()`
- `point_at(t)`, `tangent_at(t)` with **t ∈ [0, 1] normalised** in the public
  API (Revit offers both raw and normalised; we expose only normalised)
- `closest_param(p) -> f64`, `reversed()`, `transformed(&Transform)`,
  `split_at(t) -> (Curve, Curve)`, `bounding_box()`

A **circle** is an `Arc` whose sweep is 2π, and `is_closed()` is true.

## 2.5 Curve loops and profiles (D-10)

- `CurveLoop`: an ordered list of curves where each `end()` meets the next
  `start()` within `VERTEX_TOL`. `is_closed()`, `is_planar()`, `plane()`,
  `orientation(&Plane) -> Orientation::{CounterClockwise, Clockwise}`.
  Revit: `CurveLoop`.
- `Profile`: one **outer** loop (counter-clockwise about the plane normal) and
  zero or more **hole** loops (clockwise), all coplanar and non-intersecting.
  `Profile::from_loops(...)` normalises orientation and validates.
  Revit: the sketch of a floor / roof / extrusion (`CurveArrArray`).

Every sketch-based element (floor, roof, extrusion, opening) is defined by a
`Profile` on a `WorkPlane`.

## 2.6 Topology: half-edge B-rep (D-11)

Solids and editable geometry use a **half-edge boundary representation**:

| Entity | Meaning |
|---|---|
| `Vertex` | A point (geometry: `Point3`) |
| `HalfEdge` | A directed use of an edge by one loop; knows `next`, `prev`, `twin`, `origin` vertex, owning `Loop` |
| `Edge` | A pair of twin half-edges (geometry: `Curve`) |
| `Loop` | A closed cycle of half-edges bounding a face (outer or hole) |
| `Face` | One outer loop + hole loops (geometry: `Surface`; `Plane` or `Cylinder` in v1) |
| `Shell` | A connected set of faces |
| `Solid` | One or more closed shells |

Rules:

1. Topology (connectivity) and geometry (positions, curves, surfaces) are stored
   separately. Topological ids are indices into arenas inside the B-rep, local to
   one body, **not** `ElementId`s.
2. A `Solid` is **closed, manifold, and outward-facing** (face normals point out).
   `Solid::validate()` checks this and is run in debug builds after every
   operation.
3. Faces may have holes (SketchUp behaviour). Each face carries a `Surface`;
   v1 supports **`Plane` and `Cylinder`** (curved walls, round columns, barrel
   vaults, from extruded arcs) exactly. NURBS and SubD surfaces plug in later
   behind the same trait without changing the topology (D-39).
4. Revit exposes the same structure read-only (`Solid` → `Face` → `EdgeArrayArray`
   → `Edge`), which makes export straightforward.

## 2.7 Algorithms

Algorithms are free functions grouped by verb in `ferro_core::algo::<verb>`.
Naming rules are in chapter 5. Examples:

| Module | Example |
|---|---|
| `algo::intersect` | `intersect_line_plane(&Line, &Plane) -> LinePlaneIntersection` |
| `algo::distance` | `distance_point_line(Point3, &Line) -> Length` |
| `algo::project` | `project_point_plane(Point3, &Plane) -> Point3` |
| `algo::offset` | `offset_loop(&CurveLoop, Length, &Plane) -> Result<CurveLoop, GeomError>` |
| `algo::extrude` | `extrude_profile(&Profile, &Plane, Length) -> Result<Solid, GeomError>` |
| `algo::push_pull` | `push_pull_face(&mut Solid, FaceId, Length) -> Result<(), GeomError>` |
| `algo::boolean` | `boolean_union(&Solid, &Solid) -> Result<Solid, GeomError>` |
| `algo::triangulate` | `triangulate_face(&Solid, FaceId) -> Mesh` |

Intersections return an explicit enum, never a magic value:

```rust
pub enum LinePlaneIntersection {
    None,                 // parallel, not touching
    Point { point: Point3, t: f64 },
    Coincident,           // line lies in the plane
}
```
