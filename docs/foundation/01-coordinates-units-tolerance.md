# 1. Coordinates, units and tolerance

## 1.1 Axes (D-03)

- **Right-handed** coordinate system.
- **+Z is up.** Elevations are Z values.
- **+X = project east, +Y = project north** (in *project* coordinates, see 1.3).
- Positive rotation is **counter-clockwise** when looking down the rotation axis
  (right-hand rule). In plan, positive angles turn from +X towards +Y.

Revit equivalent: identical (Revit is right-handed, Z up).

## 1.2 Units (D-01, D-02)

| Quantity | Internal unit | Storage | Revit internal | Notes |
|---|---|---|---|---|
| Length | **millimetre (mm)** | `f64` | decimal feet | Metric AEC practice; IFC is metric |
| Angle | **radian** | `f64` | radian | Degrees only at UI / MCP / file-display edges |
| Area | mm² | `f64` | ft² | Display usually in m² |
| Volume | mm³ | `f64` | ft³ | Display usually in m³ |
| Slope | rise / run (unitless) | `f64` | same | Display as %, ratio or degrees |

Rules:

1. Inside `ferro-core`, quantities that cross an API boundary use **newtypes**:
   `Length`, `Angle`, `Area`, `Volume`. Raw `f64` is allowed inside a single
   algorithm.
2. **Unit conversion happens only at the edges** (UI input, MCP arguments, file
   import / export, display formatting). The kernel never sees feet or degrees.
3. User / agent input strings are parsed by one function: `Length::parse`
   (`"3000"`, `"3m"`, `"300cm"`, `"10'"`, `"10' 6\""`). A bare number means mm.
4. Display formatting is a *project setting* (`ProjectUnits`), never hard-coded.

## 1.3 Origins and north (D-04)

Ferro adopts Revit's three-point system exactly:

| Point | Meaning | Can move? |
|---|---|---|
| **Internal Origin** | (0, 0, 0) of all stored geometry | **Never** |
| **Project Base Point (PBP)** | The project's reference point (e.g. grid A/1 intersection). Defines *project coordinates* | Yes |
| **Survey Point (SP)** | A real-world reference (benchmark / site coordinates). Defines *shared coordinates* | Yes |

Plus two angles:

- **Project north**: always +Y in the model.
- **True north angle**: rotation from project north to true north, stored on the
  Survey Point relationship (positive = counter-clockwise).

Rules:

1. **All geometry is stored relative to the Internal Origin.** Moving the PBP or
   SP never rewrites element geometry; it only changes reported coordinates.
2. A coordinate is always explicit about its system when it leaves the kernel:
   `CoordSystem::{Internal, Project, Shared}`.
3. Transforms between systems are `Transform`s owned by the `Document`
   (`doc.transform(CoordSystem::Internal, CoordSystem::Shared)`).

## 1.4 Extent and precision (D-05)

- All model geometry must lie within **10 km (1e7 mm)** of the Internal Origin.
  Commands that would place geometry outside this box fail with an Error.
  (Revit enforces ~32 km; we are stricter because the renderer uses `f32`.)
- Kernel math is `f64` everywhere.
- The GPU uses `f32`. To avoid jitter, **rendering is camera-relative**: vertex
  positions are uploaded relative to a per-frame origin near the camera.

## 1.5 Tolerance (D-06)

One set of tolerances, defined once in `ferro_core::math::tol`:

| Constant | Value | Meaning | Revit analogue |
|---|---|---|---|
| `VERTEX_TOL` | 0.01 mm | Two points closer than this are the same point | Vertex tolerance (~0.0005 ft) |
| `SHORT_CURVE_TOL` | 0.5 mm | Shortest allowed edge / curve | `ShortCurveTolerance` (~0.8 mm) |
| `ANGLE_TOL` | 1e-6 rad | Two directions within this are parallel | Angle tolerance |
| `PARAM_TOL` | 1e-9 | Curve parameter equality on [0, 1] | — |

Rules:

1. **Never compare `f64` with `==`.** Use `tol::eq_len(a, b)`,
   `Point3::is_almost_equal(&other)`, `Vec3::is_parallel(&other)` and friends.
2. Algorithms take tolerance from `tol`, not from local magic numbers. An
   algorithm that genuinely needs a different tolerance takes it as an explicit
   argument and documents why.
3. Geometry that would produce an edge shorter than `SHORT_CURVE_TOL` is rejected
   by the creating command (Revit behaves the same way).
4. Snapping / inference tolerances are **screen-space UI settings**, not kernel
   tolerances. Never mix the two.
