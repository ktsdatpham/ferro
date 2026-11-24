# 0007. Curved geometry and booleans: solidcore behind a Kernel seam

- **Status:** Accepted (2026-11-24, owner approval)
- **Date:** 2026-11-24
- **Affects:** D-06, D-25, D-53, D-76; M5-9, M5-16, M5-17; `docs/review/kernel-spike.md`

## Context

Ferro's own kernel models planar polyhedra (push / pull, imprint, extrude).
Curved surfaces, booleans (cuts, ADR 0004 §5), fillets and STEP need a real
B-rep kernel. The owner first proposed "use truck / monstertruck if good
enough, improve upstream". The spike (`docs/review/kernel-spike.md`) ran the
same architectural cases on truck, monstertruck and solidcore (a safe-Rust
port of SMLib, Apache-2.0):

- truck and monstertruck fail common cases (coplanar faces, flush openings,
  0.001-0.01 mm overlaps) and give no per-face history;
- solidcore passes every case with exact volumes, reports per-face and
  per-edge history (D-25 lineage), has blends, offsets, shells and STEP,
  no panics on input, deterministic, wasm32 in its CI.

## Decision

1. **solidcore** is Ferro's kernel for booleans and curved geometry, pinned by
   git revision (`tangency-labs/solidcore`, rev in `Cargo.lock`) until it
   publishes releases. We contribute our cases upstream (spike §7).
2. **Seam.** `ferro-core` defines a trait `Kernel` (in `algo`) with the
   operations Ferro needs (boolean union / difference / intersection of
   `Body`s first; blends, offsets, curved primitives later). Results carry
   lineage: every output face / edge maps to its input ids or is new.
   `ferro-core` keeps no kernel dependency and stays pure.
3. **`ferro-kernel` crate** implements `Kernel` on solidcore: Ferro `Body` →
   solidcore record → operation → `Body`, mapping history back to Ferro's
   persistent ids. It is the only crate that depends on solidcore.
4. **Injection.** A `Session` gets its kernel like its `IdSource`
   (`Box<dyn Kernel>`); commands reach it through the `Transaction`. Without a
   kernel (tests, minimal builds) kernel commands fail with
   `ferro.kernel.unavailable`, never panic.
5. **Types.** `Surface` / `Curve` follow the STEP / solidcore-record shapes of
   spike §7 (M5-9.1): unbounded analytic geometry placed by a frame, bounds in
   topology, radians inside, stable serde tags.
6. **Oracle.** OCCT stays a test-only oracle option, never the modelling
   kernel (LGPL C++, no wasm32).

## Alternatives considered

- **truck / monstertruck, improved upstream**: they fail daily cases and lack
  history; fixing that is writing a kernel.
- **Write our own booleans**: solidcore already does the expensive part.
- **OCCT**: no wasm32, C++ / LGPL.

## Consequences

- One crate to swap if solidcore stalls; the rest of Ferro sees only `Kernel`.
- Build time and size grow for apps that link `ferro-kernel`; CLI, app and MCP
  link it, core tests do not.
- `Cut` relationships (M5-5.3), phased cuts (ADR 0006 §5) and curved Forms
  become possible.

## Notes

- 2026-11-24 (M5-16): `subtract` joins the standard geometric verbs (§5.5)
  for the first kernel command, `subtract_forms` (a solid boolean
  difference); `cut` stays the `RelationshipKind`.
- 2026-11-24 (M5-16): the session holds `Option<Arc<dyn Kernel>>` (shared
  with each transaction and the preview), not `Box<dyn Kernel>`.
- 2026-11-24 (M5-5.3): `unite` and `intersect` join the geometric verbs
  (§5.5) for `unite_forms` / `intersect_forms`; `join` stays a
  `RelationshipKind` and is never a boolean. Regeneration reaches the
  kernel too (`generate::regenerate_with`), for `cut` rows; without one
  the cut is skipped with a warning, never an error.
