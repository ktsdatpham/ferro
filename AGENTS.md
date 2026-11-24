# Instructions for AI agents contributing to Ferro

You are working on Ferro, an open-source BIM-grade 3D design tool in Rust.
Its foundations are fixed in [`docs/foundation/`](docs/foundation/README.md).
Read it before writing code. These rules are not optional.

## Before you write code

0. Pick your task from the task tree [`docs/tasks/TREE.md`](docs/tasks/TREE.md)
   and follow its rules (claim, done, verified). The code map is
   [`docs/api/TREE.md`](docs/api/TREE.md).

1. Read `docs/foundation/README.md` and the chapters relevant to your change.
2. Check the glossary in `docs/foundation/05-naming-conventions.md`. Use its
   terms exactly. **Never invent a synonym** (no `Entity`, `Object`, `Model`,
   `Manager`, `Utils`).
3. If your change needs a concept, term, unit, category, verb or dependency that
   the spec does not have, **stop and propose a spec change / ADR** in
   `docs/decisions/` instead of improvising.

## Hard rules

- Internal units: **mm** and **radians**. Convert only at edges (UI, MCP, files, FFI).
- `Point3` ≠ `Vec3`. Don't add points.
- Never compare `f64` with `==`; use `math::tol`.
- Nothing mutates a `Document` except a `Command` inside a `Transaction`.
  Each command has an MCP tool with the same `verb_noun` name.
- `ferro-core` stays pure: no IO, no UI, no global state, deterministic output.
- No `unwrap()` / `expect()` / `panic!` on paths that handle user or agent input.
- Layers depend downward only: `math → geom → topo → algo → model → ops`.
- All geometry belongs to an element; free modelling happens inside `Form`.
- Stable strings (parameter keys, failure codes, command names, file keys) are
  never renamed once released.

## Every PR

- Run `scripts/ferro-check` (or `--quick` while iterating) and paste its
  result in the PR; fill in `.github/pull_request_template.md`.
- Agents verify their work with `ferro-cli check` (build a model with
  `ferro-cli run <script.json> --out m.fro`, then `ferro-cli check m.fro`;
  see `crates/ferro-cli/README.md`).
- If you added or changed public items, regenerate the API map:
  `python3 scripts/api_tree.py`.

- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` pass.
- New algorithms have basic, degenerate and tolerance-edge tests.
- Public items have doc comments (what, units, failure cases).
- If you changed a concept, the spec changed in the same PR.
- Keep PRs small and focused on one concept.
