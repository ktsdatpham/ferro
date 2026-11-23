# 0006. Phases and design options

- **Status:** Accepted (2026-11-24, owner approval)
- **Affects:** D-32, D-33, D-34, D-55, F10 (review), M5-5, M5-23, M5-26

## Context

D-32 reserves phase and design-option fields; nothing uses them yet.

- **ArchiCAD** uses one renovation status per element (existing / to be demolished / new) and filters by view. It is easy to use but cannot describe several stages.
- **Revit** uses ordered phases with *phase created* and *phase demolished* per element, plus phase filters per view. This covers multi-stage and temporary work, but it is hard to use.
- **Revit design options** have three weaknesses:
  - a shared element cannot be changed inside an option without copying it;
  - options of different sets cannot be combined;
  - accepting the primary option deletes the others.
- **ArchiCAD** combines one option per set in "combinations" shown by views.

## Decision

### Phases

1. **Phase list.** The project holds an ordered list of `Phase`s (name, optional dates). New projects get two: "Existing" and "New Construction". Phases are elements of category `Phases` (Revit: `OST_Phases`), so they get ids, undo and saving for free.
2. **Element fields.** Each element has `phase_created` and `phase_demolished` (the existing reserved fields). `phase_demolished` must be the same phase as created or a later one.
3. **Status by view.** Each view has a phase. An element's status in that view is derived from the two fields:
   - *existing*: created earlier;
   - *new*: created in this phase;
   - *demolished*: demolished in this phase;
   - *temporary*: created and demolished in this phase;
   - *future*: created later; hidden in this view.

   Graphics per status come from the view's style (D-49).
4. **Simple mode.** The app shows ArchiCAD-like buttons (Existing / Demolish / New) that set the two fields for the common two-phase renovation.
5. **Phased cuts.** A `Cut` relationship (M5-5) carries the phase it is made in. A new opening in an existing wall therefore shows the removed part as demolished, and the removed volume is counted as demolition.
6. **Quantities.** Schedules and quantities can be split by status (demolished volume, new volume) for cost and carbon.
7. **Reality data.** Elements built from point clouds default to the first phase (Existing).

### Design options

1. **Option sets.** The project has option sets. Each set has options, and one option per set is primary. Sets and options are elements.
2. **An option is a layer of changes over the shared model.** It can:
   - add elements;
   - remove shared elements (in that option only);
   - override properties of shared elements, such as the height of a shared wall.

   It uses the same path-based change format as sync and branches (M5-26). No copy of shared elements is needed.
3. **Combinations.** A view shows a *combination*: one option per set, primary by default. Views can sit side by side, and a comparison table gives area, quantities, cost, carbon and U-values per combination.
4. **Accepting an option.** Accepting merges its layer into the shared model, like merging a branch. Other options are kept, archived, never deleted silently.
5. **Generated options.** AI-generated alternatives (D-47) are option layers, reviewed like proposals (D-26).
6. **Interop.**
   - IFC: the primary combination is exported (others optional, as separate files).
   - Revit: options of an exported combination become Revit design options where Revit's limits allow, otherwise the primary combination only, with a warning.

## Alternatives considered

- **One renovation-status parameter (ArchiCAD).** Simple, but no multi-stage work and no temporary elements.
- **Revit-style options (elements belong to one option).** Copying shared elements breaks joins and hosting, and options cannot be combined.

## Consequences

- New categories `Phases` and `DesignOptions` (category enum change; Revit `OST_Phases`, `OST_DesignOptions`).
- Views (M5-22) carry a phase and a combination.
- The change format (M5-26) must land before design options. Phases can land first.
