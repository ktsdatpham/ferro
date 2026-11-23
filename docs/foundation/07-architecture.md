# 7. Architecture and layering

## 7.1 Layers inside `ferro-core` (D-19)

```
ops      Command, Transaction, regeneration, failures, events, undo
model    Document, Element, Category, Family, FamilyType, Parameter, Level, Grid, WorkPlane, Form
algo     intersect, distance, project, offset, extrude, push_pull, boolean, triangulate
topo     half-edge B-rep: Vertex, HalfEdge, Edge, Loop, Face, Shell, Solid
geom     Plane, Line, Arc, Curve, CurveLoop, Profile
math     Point3, Vec3, UnitVec3, Transform, BoundingBox3, Length, Angle, tol
```

**A layer may only depend on layers below it.** `geom` never imports `model`;
`algo` never imports `ops`. CI will check this with a module dependency lint.

`ferro-core` is pure:

- no file / network IO (serialisation to/from `String` / bytes is allowed)
- no UI, no GPU, no async runtime
- no global mutable state, no thread-locals for model data
- deterministic (no `HashMap` iteration order in outputs; use `BTreeMap` or
  sorted output)

## 7.2 Crates

| Crate | Depends on | Role |
|---|---|---|
| `ferro-core` | `serde`, minimal math deps | Everything in 7.1 |
| `ferro-io` | `ferro-core`, `rusqlite` | `.fro` / `.fro.json` files, OBJ / glTF export |
| `ferro-snap` | `ferro-core` | Inference (snapping) |
| `ferro-scene` | `ferro-core` | Display scene derived from a document snapshot (ADR 0004 §6): draw records, shared meshes for translated copies, incremental sync; pure, no GPU |
| `ferro-tools` | `ferro-core` | Tool schemas, unit normalisation, dispatch, Ferro Link protocol (ADR 0001) |
| `ferro-mcp` | `ferro-core`, `ferro-io`, `ferro-tools` | MCP server; forwards to the open app over Ferro Link |
| `ferro-revit` | `ferro-core` | Revit edge: Ferro → Revit operations (feet), Revit → Ferro import (ADR 0002) |
| `ferro-ffi` | `ferro-core`, `ferro-io`, `ferro-revit` | C ABI for the Revit add-in and native shells |
| `ferro-render` (M1) | `ferro-core`, `wgpu` | Turns elements into GPU meshes, camera-relative |
| `ferro-app` | all above, `eframe` / `wgpu`, `ureq` | Tools, selection, UI, input, Ferro Link server, AI chat |

Rules:

1. Logic that decides *what the model is* lives in `ferro-core` only. Other
   crates translate (UI ↔ commands, JSON ↔ commands, C ABI ↔ commands).
2. New dependencies in `ferro-core` need an ADR. Keep the kernel small and
   auditable.
3. The C# Revit add-in contains no geometry logic.

## 7.3 Decisions and ADRs

Foundational changes are recorded as Architecture Decision Records in
[`../decisions/`](../decisions/), numbered `NNNN-short-title.md`, using the
template there. An ADR is required to:

- change an Accepted `D-xx` decision
- add a category, a standard verb, or a glossary term
- add a dependency to `ferro-core`
- change the `.fro` format version
