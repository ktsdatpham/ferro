# Kernel spike: truck, monstertruck, solidcore vs OCCT (M5-9.2)

- **Status:** Evaluation report, 2026-11-24. The recommendation in §7 needs the owner's approval
  (it amends D-30 / D-39 via ADR 0004 §4).
- **Question:** which library should give Ferro curved surfaces (NURBS, fillets) and robust
  booleans, behind Ferro's own APIs, while `ferro-core` keeps building for
  `wasm32-unknown-unknown` (D-35) and keeps persistent ids + lineage (D-25)?
- **Candidates:** [truck](https://github.com/ricosjp/truck) (ricosjp, Rust),
  [monstertruck](https://docs.rs/monstertruck) (fork of truck, Rust),
  [solidcore](https://github.com/tangency-labs/solidcore) (safe-Rust port of NVIDIA SMLib, Rust;
  added at the lead's request), and OCCT in a native plugin (from documentation only, not built).
- **Spike code:** `spikes/kernel-eval/` (separate Cargo workspaces, not part of the Ferro
  workspace): `truck-eval`, `mt-eval`, `sc-eval`. Each runs the same experiments and prints one
  table row per experiment.

## 1. Summary

| | truck 0.6 (crates.io) | monstertruck 0.4.3 | solidcore (git 3d86b62) | OCCT 8.0 (docs) |
|---|---|---|---|---|
| Fillet of wall edges | **no API** (rounded profile + extrude works) | yes, but **silently skipped 1 of 4 edges** | **yes, exact** (1 and 4 edges) | yes (`BRepFilletAPI_MakeFillet`) |
| Box − box / box − cylinder | pass / pass (1.9 s) | pass only at tol 0.01 / **fail** | **pass / pass**, exact volume | yes |
| Wall + wall at a corner (coplanar tops / bottoms) | **fail** | **fail** | **pass** | yes |
| NURBS roof ∩ / − wall | fragile: pass or fail by tolerance, 3-60 s | pass, 11-145 s, one result 1.8e-3 off | **pass, exact**, 3-4 s (unoptimised build) | yes |
| Coplanar / touching / flush faces | **fail** (all 5 coplanar cases) | flush openings pass; coplanar unions fail | **all pass** | yes |
| 0.001 / 0.01 mm offsets | gaps pass, overlaps fail | gaps pass, overlaps fail | all pass | yes (fuzzy booleans) |
| 5 km from origin | pass | box pass, cylinder fail | pass | yes |
| Volume | from mesh only | from mesh only | **exact mass properties** | exact |
| STEP round trip | roof ok; **boolean result panics on re-import** | roof ok | ok, exact volume back | yes (mature) |
| History / provenance for lineage | none | none | **per face and edge** (`History`) | yes (`BRepTools_History`, `Modified/Generated`) |
| wasm32-unknown-unknown | claimed, not verified here | claimed, not verified here | CI `cargo check --target wasm32` | **no** (emscripten only, 21-49 MB) |
| Licence | Apache-2.0 | Apache-2.0 | Apache-2.0 | LGPL-2.1 + OCCT exception 1.0 |

**Recommendation: (a) adopt solidcore for curved geometry and booleans, behind a single Ferro
seam (`ferro-kernel`, M5-16), keeping Ferro's own kernel for planar push-pull and imprint.
Do not adopt truck or monstertruck.** Their booleans fail the most common architectural case
(two walls sharing their base and top planes) and they report no history for lineage. OCCT
stays an optional native plugin for exchange, not the kernel. Reasons and conditions in §7.

## 2. Method

- Units mm. Expected volumes are exact closed forms (Cavalieri's principle for the roofs:
  every roof has constant vertical thickness 200 mm over a 7000 × 2200 mm footprint, so its
  volume is 3.08e9 mm³ and its intersection with the 6000 × 200 mm wall is 2.4e8 mm³ whatever
  the curvature).
- Pass = the result is a closed (manifold) solid and its volume is within 2e-3 of the exact
  value. truck and monstertruck have no mass properties, so their volume is measured on a
  tessellation (chord 0.5 mm, 2 mm for the roofs); solidcore's volume is its own exact mass
  property, and its mesh volume is reported as a second column.
- truck and monstertruck booleans take a tolerance argument; it was swept (0.01, 0.05, 0.1,
  0.5, 1, 2, 5, 10, 20 mm). The default run is 1 mm for truck and 0.01 mm for monstertruck
  (the values where each passed most cases). solidcore takes none; it ran with every upstream
  fix flag on, as SolidCraft and solidcore's own tests do.
- Machine: 2 cores shared with other agents, 8 GB RAM. truck and monstertruck: release builds.
  solidcore: **unoptimised dev build** (release builds of its 1.1 M-line kernel crate were
  killed for memory on this machine; the dev build peaked at 5 GB rustc RSS). So solidcore's
  times are pessimistic and all times are indicative only.
- Spike sources: `spikes/kernel-eval/{truck-eval,mt-eval,sc-eval}/src/main.rs`. Commands:
  `ONLY=<experiment prefix> MESH_TOL=<mm> target/release/truck-eval <tol>` (same for
  `mt-eval`; `sc-eval` from its own folder).

## 3. Results per experiment

Times are milliseconds for the operation (not the measurement). Rel err = (volume − exact) / exact.

### E1 Wall 6000 × 200 × 3000 with vertical edges filleted r = 50

| Case | truck | monstertruck | solidcore |
|---|---|---|---|
| Rounded profile (4 arcs) extruded | PASS, rel −4.2e-5 (mesh), 532 tris | PASS, rel −6.9e-6 (mesh), 38 900 tris, mesh 3.1 s | PASS, exact (−1.3e-15), 10 faces |
| Fillet 1 vertical edge | no fillet operation | PASS 18 ms, rel −1.7e-6 | PASS 304 ms, exact, 7 faces |
| Fillet 4 vertical edges | no fillet operation | **wrong**: closed solid but rel +4.4e-4 = only 3 of 4 rounds; no error returned | PASS 1235 ms, exact, 10 faces |

monstertruck's `fillet_edges` restores a checkpoint and `continue`s when a chain fails
(`monstertruck-fillet` `edge_select.rs`, `chain_failed`), so a failed edge is dropped
silently. For Ferro a silent partial result is worse than an error.

### E2 Booleans

| Case | exact mm³ | truck (tol 1) | monstertruck | solidcore |
|---|---|---|---|---|
| a. wall − window box (through, 1500 × 1200) | 3.24e9 | PASS 0.8 ms | tol 0.01: PASS 9 ms; tol 0.1-20: **fail** "shell is not oriented and closed" | PASS 40 ms, exact |
| b. wall − cylinder r400 (round window) | 3.499469e9 | PASS 1864 ms, rel +2.1e-5 | **fail** at every tol 0.01-2 ("not oriented and closed"), 0.4-11 s | PASS 597 ms, exact, cylinder kept as 1 face with a seam |
| c. wall ∪ wall, L corner (shared base / top planes) | 5.88e9 | **fail** at every tol 0.01-10 (`or` returns `None`) | **fail** at every tol | PASS 78 ms, exact, 8 faces (coplanar faces merged) |

Scale check (E9): upstream's own example, a unit cube minus a cylinder r 0.3 at tol 0.05,
passes on both truck and monstertruck. The same geometry at ×1000 (mm-sized): truck passes at
tol 1 but fails at tol 0.01 (27 s, then `None`); monstertruck passes at tol 0.01 but **returns
an empty solid with no error** at tol 1. Both use one absolute `TOLERANCE = 1e-6` in
`truck-base` / `monstertruck-core` plus the caller's boolean tolerance, and the working window
differs per kernel and per case. Ferro would have to guess a tolerance per operation.

### E3 Curved (NURBS) roof vs vertical wall

Roof for truck / monstertruck: one bicubic Bézier face (single-curved: rises 1500 mm along x;
double-curved: a dome), swept up 200 mm into a solid. Roof for solidcore: a cubic loft through
four sections with the same constant vertical thickness (solidcore's ops API builds solids from
curves, not from a raw surface; a record could also be built directly). Wall: 6000 × 200 × 5000.

| Case | exact mm³ | truck | monstertruck (tol 0.01) | solidcore |
|---|---|---|---|---|
| single-curved roof alone | 3.08e9 | PASS (mesh 2 mm takes 2.8 s; **0.5 mm ran out of 4 GB**) | PASS | PASS exact |
| wall ∩ single-curved roof | 2.4e8 | tol 1: **fail** (`None`, 12 s); tol 0.1: PASS 57 s; tol 5: PASS 3.2 s | PASS 97 s | PASS 3.0 s, exact |
| wall − single-curved roof | 5.76e9 | tol 1: **fail** (12 s) | PASS 146 s, 2 lumps | PASS 2.9 s, exact, 2 lumps |
| double-curved roof alone | 3.08e9 | PASS | PASS (but meshed as 12 flat triangles, see E5) | PASS exact |
| wall ∩ double-curved roof | 2.4e8 | PASS 20.6 s, rel −4.9e-6 | PASS 11.4 s, rel **+1.8e-3** | PASS 3.9 s, exact |
| wall − double-curved roof | 5.76e9 | PASS 21.2 s | PASS 11.4 s, rel −7.6e-5 | PASS 3.6 s, exact |

### E4 Robustness

| Case | truck (tol 1; also 0.01, 0.1, 10) | monstertruck (tol 0.01) | solidcore |
|---|---|---|---|
| two boxes sharing a face (union) | **fail** | **fail** | PASS → 1 box, 6 faces |
| gap 0.001 mm | PASS (2 solids) | PASS (2 solids) | PASS (2 solids) |
| gap 0.01 mm | PASS (2 solids) | PASS (2 solids) | PASS (2 solids) |
| overlap 0.001 mm | **fail** | **fail** ("no boundary shells") | PASS, 1 solid, exact |
| overlap 0.01 mm | **fail** | **fail** | PASS, 1 solid, exact |
| wall − opening flush with both faces | **fail** | PASS | PASS |
| wall − niche flush with one face | **fail** | PASS | PASS |
| wall − window at (5 km, 5 km) | PASS | PASS | PASS |
| wall − cylinder at (5 km, 5 km) | PASS | **fail** | PASS, exact |

Note for Ferro: a 0.001 mm gap is below Ferro's `VERTEX_TOL` (0.01 mm), yet every kernel keeps
two solids. Ferro must snap at its own tolerance before calling any kernel, or a kernel must
accept a body tolerance (solidcore's record carries per-body, per-edge and per-vertex
tolerances, so this is a setting, not a redesign).

### E5 Tessellation and STEP

Wall − cylinder (the E2b result), display tessellation by chord tolerance:

| chord | truck | solidcore (dev build) |
|---|---|---|
| 5 mm | 244-398 ms, 546 tris, vol err +3.1e-4 | 29 ms, 148 tris, +1.9e-4 |
| 1 mm | 543-698 ms, 4064 tris, +6.2e-5 | 50 ms, 320 tris, +4.6e-5 |
| 0.1 mm | 1.3-2.3 s, 24 976 tris, +5.2e-6 | 200 ms, 1224 tris, +3.3e-6 |

Double-curved roof: solidcore 275 ms / 2360 tris (5 mm), 1.0 s / 11 256 (1 mm), 7.9 s / 100 884
(0.1 mm). truck meshed the single-curved roof in 2.8 s at 2 mm and ran out of memory (4 GB
limit) at 0.5 mm. monstertruck meshed the double-curved Bézier face, whose boundary lies in one
plane, as 12 flat triangles at 2 mm (its sag test missed the dome), which is also why its E3
volume was off. solidcore gives fewer triangles for the same accuracy, because it meshes each
face in its own (u, v) and shares edge samples between faces.

STEP (write, parse back, compare volumes):

| | truck-stepio 0.3 | monstertruck-io 0.4.3 | solidcore-step |
|---|---|---|---|
| double-curved roof | ok, 13 kB, 6 faces, vol diff −2.5e-15 | ok, 6 faces, diff 0 | ok, 15 kB, 6 faces, diff 1.6e-16 |
| wall − cylinder (has intersection-curve edges) | written, but re-import prints "Lookup failed for #243 …" then **panics** (index out of bounds) | not tested (its boolean failed) | ok, 12 kB, 7 faces, diff 0 |

### E6 WASM

Not testable here: `rustup target add wasm32-unknown-unknown` failed because
static.rust-lang.org is blocked by the sandbox proxy ("unsuccessful tunnel"), and no toolchain
here has the wasm32 std. Evidence instead:

- truck: `cfg(target_arch = "wasm32")` paths in truck-meshalgo (rayon and vtkio left out),
  getrandom wasm features, a `truck-js` crate. Pulls `wasm-bindgen` / `js-sys` into the wasm
  dependency tree (`cargo tree --target wasm32-unknown-unknown`).
- monstertruck: same pattern, plus a `monstertruck-wasm` crate on crates.io.
- solidcore: its CI runs `cargo check -p solidcore --target wasm32-unknown-unknown`
  (`.github/workflows/ci.yml`, `release.yml`); dependencies of the kernel crate are only
  `serde` and `thiserror`. Its README asks wasm consumers to link with an 8 MiB stack because the
  wasm stack has no guard page. solidcore-tess leaves rayon out on wasm.
- OCCT: only through emscripten (`wasm32-unknown-emscripten`), not `wasm32-unknown-unknown`;
  opencascade.js full build is 48.9 MB of wasm (9.1 MB brotli), occt-wasm 21 MB.

To do when a wasm toolchain is available: `cargo check --target wasm32-unknown-unknown` on
`sc-eval`'s dependencies (and on the future `ferro-kernel`).

## 4. Why SolidCraft left truck

SolidCraft (Rust parametric CAD, MIT/Apache) started on truck, moved its kernel to SMLib and
then to solidcore, and has removed truck (`solidcraft/docs/kernel-smlib-history.md`, branch
`kernel-smlib`, tracked in solidcraft#4012). What their documents record:

- **Correctness backlog.** `docs/kernel-cases.md` turns "every geometry case the old kernel
  (truck) got wrong, as an issue, a PR or a crash report recorded it" into a test with an
  analytic answer: 1573 sources read, 372 cases kept (89 booleans, 50 fillets, 83 sweeps, 57
  sheet metal, 18 shells, …). Examples: a join of 64 separate pegs took 36 s on truck and grew
  4-9× per doubling; a ring of 90 cylinder segments gave a caught truck panic, then a refusal
  after 21 minutes; repeated `intersection_curve` unwrap panics; "not implemented: intersection
  curve cannot connect by homotopy"; "tolerance must be no less than 1e-6" panics on lofts.
  On solidcore 316 of the 372 pass today; the rest are open kernel bugs, never worked around in
  the application.
- **Speed.** Their side-by-side runner (411 booleans: 128 random box / cylinder / sphere / torus
  pairs and 9 named cases, each union, intersection and cut): truck median 8.8 ms, worst 6.2 s,
  total 44 s; SMLib median 1.8 ms, worst 25 ms, total 1.1 s. In a cut sequence truck slowed down
  with each cut (9 → 61 → 378 → 383 ms), SMLib stayed flat (4-7 ms).
- **Exact measures.** truck volumes come from a tessellation (within 1e-3 of analytic);
  SMLib's are exact (within 1e-6).
- **Where truck was better then:** 11 of 411 operations with a cylinder or torus tangent to a
  plane failed on SMLib and passed on truck (solidcore#383), and ~15 SMLib volumes were wrong
  (solidcore#384). The kernel ladder (`docs/kernel-ladder.md`) now passes all 25 boolean
  degeneracy cases including tangencies at 1e-6 and 1e-3 mm.
- **How they moved without breaking names:** a kernel-neutral B-rep record ("schema 4",
  `docs/native-brep.md`) with STEP names, unbounded analytic geometry bounded by edge intervals
  and face domains, a pcurve on every oriented edge, finite tolerances per body, edge and vertex,
  and one canonical order of faces / edges / vertices that their persistent names index. truck
  bodies round-tripped through it exactly before the switch. A naming guard,
  `apps/solidcraft-cli/tests/naming_corpus.rs`, rebuilds 154 designs and checks every face and
  edge name and every stored reference; each operation moved to SMLib had to keep them.

Our own E2c / E4 failures (coplanar unions) and E3 tolerance windows reproduce the same class of
problems at architectural scale, independently.

## 5. Project health and licence

| | truck | monstertruck | solidcore | OCCT |
|---|---|---|---|---|
| Licence | Apache-2.0 | Apache-2.0 | Apache-2.0 (© NVIDIA for the ported SMLib) | LGPL-2.1 + Open CASCADE Exception 1.0 |
| Compatible with Ferro's MPL-2.0 | yes | yes | yes | only as a separately linked plugin (LGPL relinking terms) |
| What it is | original kernel by RICOS (Japan) | **fork of truck** with renamed crates, by Moritz Mœller (`virtualritz`); same history from 2020-05-18, upstream commits still merged | port of NVIDIA SMLib (usd-brep v1.0.1) to safe Rust, developed inside SolidCraft (monorepo subtree, read-only mirror) | Open Cascade SAS, C++, since 1999 |
| Last crates.io release | **truck-modeling 0.6.0, 2024-09-20** (master moved on since) | 0.4.3, 2026-11-23 (8 releases since 2026-03) | `solidcore` 1.0.1, 2026-10-30; **ops, tess, step, record not published** (`publish = false`) | 8.0.1, 2026-07-30 |
| Commit activity | 2892 commits; latest 2026-11-18; ~5-45 per month in 2026 | latest 2026-11-23; ~200 own commits since 2025-06 | very active (many agent PRs, live progress site); shallow clone, counts not measured | steady, company-backed |
| Maintainers | essentially one (Yoshinori Tanimura: ~300 of the commits since 2025-06) | one (Moritz Mœller); 17 commit messages reference Claude (AI-assisted) | one org (tangency-labs), crates.io owner `bherbruck`; AI agents explicitly welcome | Open Cascade SAS + community |
| API stability | 0.x, breaking minors | 0.x, renames vs truck, "v2" traits in progress | **experimental 0.0.x** (ops), kernel crate 1.0.1 mirrors upstream's numbering | stable C++ API, 8.0 dropped legacy |
| Docs | tutorials, docs.rs | docs.rs (80 % documented) | design docs (`docs/layers.md`, `docs/ssi.md`, `docs/generic-blend.md`), doc comments with errors on every op | extensive reference docs |
| Size / build | small (~2 min release build here) | small-medium | **kernel crate ~1.1 M lines; dev build 5 min, 5 GB rustc RAM; release OOM on 8 GB**; repo 564 MB with fixtures | large C++; wasm 21-49 MB |
| Stars / forks | 1.5k / 110, 32 open issues | small (2k downloads) | new | large |

Open-issue themes for truck (from the SolidCraft kernel-cases list, which reads truck issues
and PRs as sources): boolean failures and hangs with many operands, intersection-curve panics,
tolerance panics, no fillet in truck itself (SolidCraft wrote its own on top).

## 6. Integration with Ferro (D-25 persistent ids + lineage)

Ferro's `Body` today (`crates/ferro-core/src/topo/body.rs`): half-edge topology in ordered
maps, ids from one never-reused counter, faces carrying `Surface::{Plane, Cylinder}`, **edges
with no curve** (an edge is the straight segment between its vertices) and `Lineage` for faces
only. Any curved kernel needs edges with curves (and, for robust exchange, pcurves and
tolerances), so Ferro's topology has to grow whichever library is chosen.

### Options

| Option | What | Effort (engineer-weeks) | Verdict |
|---|---|---|---|
| A. Keep Ferro topology, call the kernel per operation, convert in / out | Ferro `Body` ↔ kernel record at the `ferro-kernel` seam; lineage from the kernel's per-face / per-edge history mapped back to Ferro's ids | 8-10 (below) | **recommended, with solidcore** |
| B. Adopt the library's topology for curved Forms | Curved Forms hold a kernel body; planar Forms keep Ferro's | 4-6, but two models of everything (ids, references, writers, snapping) | no: doubles the D-25 work and ties file format to a 0.0.x API |
| C. truck / monstertruck + contribute | Fix coplanar classification, add history, add fillet (truck), tolerance model | 6-12 months of kernel work before parity with what solidcore passes today | no |
| D. OCCT native plugin | C++ FFI, LGPL plugin, no `wasm32-unknown-unknown` | 6-8 for the bridge; web build impossible without emscripten | only for exchange / validation, optional |

Option A with solidcore, in steps:

1. `ferro-kernel` seam crate (M5-16), the only crate that names solidcore; feature-gated so
   `ferro-core` can still build without it. 1 week.
2. Surface / Curve types (M5-9.1) shaped like solidcore-record's (below), so conversion is field
   by field. 1-2 weeks.
3. Edge curves, edge intervals and tolerances in Ferro's `Body`; Body ↔ `solidcore_record::Brep`
   conversion (pcurves are exact on planes; cylinders closed form; solidcore's `pcurve` module
   fits the rest). 2-3 weeks.
4. Lineage: solidcore reports `History { faces: Vec<FaceProv>, edges: Vec<EdgeProv> }` with
   `FaceOrigin::Face { operand, face }`, `Profile { segment }`, `Cap { end }` and
   `Change::{Kept, Modified, Split, …}` per result face, and `EdgeOrigin::{Edge, Meet}` per edge
   (`solidcore-record/src/brep/history.rs`). Ferro keeps its own ids: a kept face keeps its id,
   a split face's pieces get new ids with `created: (new, Some(old))`, generated faces (fillets,
   intersection walls) get new ids with their origin edge. Extend `Lineage` to edges. 1-2 weeks.
5. Commands: `subtract_form`, `fillet_edges`, `chamfer_edges`, `cut` relations (ADR 0004 §6)
   through the seam, with the model corpus (M5-17) and our E-cases as tests. 2-3 weeks.

SolidCraft's naming (`crates/doc/src/persistent_name.rs`, `lineage.rs`) builds text names from
feature history (`F5:blend(<edge>)`, pieces told apart by neighbouring faces) and falls back to
a geometric trace (`crates/kernel/src/provenance.rs`: a result face lying on an input face's
surface inside its trim comes from it) where an operation reports no history. Ferro does not
need text names (it has persistent integer ids), but it needs the same two parts: kernel
history first, geometric trace as a fallback, and a corpus test that rebuilds models and checks
every id and reference (M5-17).

Costs and risks of solidcore: experimental ops API (pin a commit, keep the seam narrow);
unpublished ops / tess / step crates (git dependency or vendoring); a very large kernel crate
(build time and memory in CI, wasm binary size to measure); thread-local session state in ops
(acceptable inside `ferro-kernel`, not inside `ferro-core`); a single organisation, much of the
code written by agents. Mitigations: the seam, the corpus, and keeping Ferro's own planar
kernel as the default for push-pull.

## 7. Recommendation

**(a) Adopt solidcore for curved geometry and booleans behind Ferro's APIs.** Not truck, not
monstertruck.

Why:

1. It is the only candidate that passed every experiment, with exact volumes, including the
   cases architecture hits daily: walls sharing base and top planes (E2c), openings flush with
   wall faces, 0.001-0.01 mm overlaps, curved roofs against walls.
2. It reports per-face and per-edge history, which D-25 lineage needs; truck and monstertruck
   report none.
3. It has fillets and chamfers (rolling-ball blend), offsets, shells and STEP in the same crate
   family; truck has no fillet and monstertruck's skipped an edge silently.
4. It meets Ferro's rules by design: safe Rust, no panics on input, deterministic, no global
   state in the kernel, wasm32 checked in CI, Apache-2.0.
5. A real Rust CAD project made the same move away from truck for the same reasons and
   documented them with 372 regression cases.

What to contribute upstream first (to tangency-labs/solidcraft, kernel branch):

1. Our architectural cases (E1-E4 above, plus wall joins, openings, roofs at 5 km) into their
   kernel-cases rig, so their CI protects what Ferro relies on.
2. Publishing (or tagged git releases of) `solidcore-ops`, `solidcore-record`,
   `solidcore-tess`, `solidcore-step`, and feature gates so a consumer builds only booleans,
   blends, tessellation and STEP (build memory and wasm size).
3. A body tolerance a caller can set (Ferro's 0.01 mm), so near-coincident inputs merge the way
   Ferro snaps.
4. A wasm32 build and test job for ops and tess, not only the kernel crate.

(b) "reference only, write our own" is not justified: solidcore already does what we would
write, and its booleans and blends are the expensive part. (c) OCCT in a native plugin stays an
option for import / export and as an independent oracle in tests, not as the modelling kernel,
because it cannot run in `wasm32-unknown-unknown` and is LGPL C++.

### What Ferro's `Surface` / `Curve` must look like now (M5-9.1), so any choice fits

Follow STEP AP242 / solidcore-record names and conventions:

- **Analytic geometry is unbounded and placed by a frame** (`Frame { origin, z_axis, x_axis }`,
  right-handed, mm); bounds live in topology (edge `interval`, face domain), never in the
  surface.
- `Surface`: `Plane { frame }`, `Cylinder { frame, radius }`, `Cone { frame, radius,
  half_angle }`, `Sphere { frame, radius }`, `Torus { frame, major, minor }`,
  `Nurbs { degree_u, degree_v, knots_u, knots_v, control_points (row-major, rectangular),
  weights: Option<…> }`; leave room for `Revolution` and `Offset` (stable serde tags, never
  renamed).
- `Curve`: `Line { origin, direction }` (or two points for a segment), `Circle { frame,
  radius }`, `Ellipse { frame, radii }`, `Nurbs { degree, knots, points, weights }`; a 2D
  `Curve2` (line, circle, NURBS) for pcurves.
- Angles in radians (SMLib's STEP-style degrees are converted at the seam).
- Topology additions: edge `curve` + `interval` + `tolerance`; vertex `tolerance`; face
  `forward` flag relative to the surface's natural normal (∂S/∂u × ∂S/∂v); optional pcurve per
  oriented edge; loops counter-clockwise (outer) / clockwise (holes) about the natural normal.
- Keep Ferro's persistent ids as the identity; derive a canonical order only for export and
  hashing.

Every item maps one-to-one onto solidcore-record, IFC `IfcAdvancedBrep`, STEP and Revit's
`BRepBuilder`; truck / monstertruck and OCCT have the same concepts, so this shape does not
lock Ferro in.

## 8. Not tested, and why

- **WASM builds**: wasm32 target not installable (static.rust-lang.org blocked). See §3 E6.
- **OCCT**: not built, by instruction; from documentation.
- **solidcore release-build timings**: release builds were killed for memory; times are from an
  unoptimised build.
- **monstertruck tessellation and STEP of the wall − cylinder result**: its boolean failed, so
  there was no body to mesh.
- **solidcore health counts**: the clone is shallow, so commit counts and contributor numbers
  were not measured; the SolidCraft GitHub page was not reachable from the sandbox.
- **truck master**: only the crates.io releases were tested (0.6.0, Sept 2024); master has two
  years of further commits, but SolidCraft's evidence covers truck up to their switch in
  Oct 2026.

## Sources

- truck: <https://github.com/ricosjp/truck>; crates.io API for `truck-modeling` and
  `truck-shapeops` versions; source of truck-shapeops 0.4.0 (`transversal/integrate/mod.rs`),
  truck-base 0.5.0 (`tolerance.rs`).
- monstertruck: <https://docs.rs/monstertruck> (README: "Forked from `ricosjp/truck` and
  renamed"); crates.io owners API (virtualritz, Moritz Mœller); source of monstertruck-fillet
  0.4.3 (`edge_select.rs`), monstertruck-solid 0.4.3; git history of
  github.com/virtualritz/monstertruck.
- solidcore: <https://github.com/tangency-labs/solidcore> (README, `docs/layers.md`,
  `crates/solidcore-record/src/brep.rs`, `crates/solidcore-record/src/brep/history.rs`,
  `.github/workflows/ci.yml`), crates.io API for `solidcore`.
- SolidCraft: <https://github.com/tangency-labs/solidcraft> — `docs/kernel-smlib-history.md`,
  `docs/native-brep.md`, `docs/kernel-ladder.md`, `docs/kernel-cases.md`,
  `apps/solidcraft-cli/tests/naming_corpus.rs`, `crates/kernel/src/provenance.rs`,
  `crates/doc/src/persistent_name.rs`, `crates/doc/src/lineage.rs`.
- OCCT licence: <https://dev.opencascade.org/doc/overview/html/occt_public_license.html>,
  <https://spdx.org/licenses/OCCT-exception-1.0.html>.
- OCCT 8.0.1 release: <https://github.com/Open-Cascade-SAS/OCCT/discussions/1415>.
- OCCT in wasm: <https://ocjs.org/docs/getting-started/file-size> (opencascade.js 48.9 MB),
  <https://docs.rs/crate/occt-wasm/3.2.3> (21 MB, LGPL-2.1 binary, wasmtime / emscripten).
- Ferro: `docs/decisions/0004-foundation-2.md` §4, `docs/review/2026-11-ra-soat-mong.md` §5,
  `docs/review/2026-11-solidcraft-solidworks.md`, `docs/foundation/README.md` (D-25, D-30,
  D-35, D-39).
