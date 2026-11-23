# 0003. Guides, dimensions, UI languages, failure reports and agent contributions

- **Status:** Proposed
- **Date:** 2026-11-23
- **Affects:** D-20 (glossary), §3.9 (annotation left to Revit), chapter 7

## Context

Milestone 4 aims at SketchUp parity. Three SketchUp features are not
covered by the spec:

- **Guides**: Tape Measure leaves infinite construction lines and points.
  The glossary reserves "Guide" as a word *not* to use for `ReferencePlane`.
- **Dimensions** placed in the model. §3.9 leaves annotation to Revit.
- The UI is English only. Every user-facing string is written in code.

The project owner also wants:

- error reports users can send;
- users' own AI agents to build tools and propose pull requests that follow
  `AGENTS.md`, checking themselves before a PR is opened.

## Decision

### 1. Construction geometry (Guides)

1. New glossary terms: `ConstructionLine` (infinite line through a point with
   a direction) and `ConstructionPoint`. UI label: "Guide", as in SketchUp.
2. Both are elements of a new category `ConstructionGeometry`. They are saved
   with the document, can be selected, moved, erased and hidden. They are not
   building elements: they have no geometry body, are not exported to IFC,
   and go to Revit only as reference planes when the user asks for it.
3. Snapping uses them: points on them, and their intersections with edges,
   faces and each other.
4. Commands: `create_construction_line`, `create_construction_point`. Erasing
   and moving them uses the existing commands.

### 2. Dimensions

1. New glossary term `Dimension`: a linear distance between two points, drawn
   offset from them, with a label that updates when the points move.
2. It is an element of a new category `Dimensions`, saved with the document.
   §3.9 is amended: *model* dimensions are part of the design, while sheets,
   views and drawing annotation stay with Revit.
3. In milestone 4 a dimension stores its two points (no association with
   edges yet). Associative dimensions come later.
4. Command: `create_dimension`.

### 3. UI languages

1. All user-facing strings go through one function, `tr!("key")`, backed by
   Fluent files `ferro-app/locales/<lang>.ftl`. English and Vietnamese ship
   first.
2. Stable strings are never translated: command names, failure codes,
   parameter keys and file keys stay English (agents and Revit rely on them).
   Failure *messages* are translated in the app.
3. A CI check fails when a new string literal is shown in the UI without
   `tr!`, and when a key is missing from any shipped language.

### 4. Failure reports

1. The app keeps a local log of the session: version, OS, commands run,
   failures and crashes (panic hook). No document content is logged.
2. "Report a problem" opens a prefilled GitHub issue (or saves a report file)
   that the user reads before sending. Nothing is sent automatically. The
   document is never attached unless the user attaches it themselves.

### 5. Agent contributions

1. **Local tools first.** An agent can build tools on the user's machine
   through Ferro Link. Every change it makes is a Proposal the user accepts,
   as today.
2. **Changes to Ferro itself go through a pull request.** An agent that
   writes a PR must read `AGENTS.md` and `docs/foundation/`, then run
   `ferro-check` (a script: fmt, clippy, tests, glossary terms, unit rules,
   layer dependencies, new-dependency check) and paste its result in the PR.
3. **CI is the gate, not the agent.** CI runs the same `ferro-check`. A
   maintainer approves every merge. Agents never merge.
4. **Updates** install only builds made from `main` and signed by the
   project. Nothing is ever built or run from an unmerged PR.
5. A PR template asks for: concept touched, spec section, tests added,
   `ferro-check` output, and whether an AI wrote it.

## Alternatives considered

- **Guides and dimensions as view state only** (not saved). Simpler and
  needs no new category, but SketchUp users expect them to be saved with the
  model and shared.
- **Guides as `ReferencePlane`s.** A guide is a line, not a plane, and
  reference planes are named datums that Revit users manage. Too heavy.
- **gettext instead of Fluent.** Fluent handles plurals and Vietnamese
  word order better and is pure Rust.
- **Automatic crash upload.** Faster feedback, but it sends data from
  client projects without consent.
- **Agents merging their own PRs after green CI.** One compromised agent
  could ship malicious code to every user through updates.

## Consequences

- New categories `ConstructionGeometry` and `Dimensions` (a spec change to
  the closed `Category` enum, with Revit and IFC mappings in chapter 6).
- `ferro-app` gains the `fluent-bundle` dependency (not `ferro-core`).
- A one-time sweep moves all UI strings into `en.ftl`.
- New `ferro-check` script and PR template; `AGENTS.md` points to them.
