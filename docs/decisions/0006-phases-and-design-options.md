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

## Implementation notes (M5-23, phases)

- New projects get **no** phases (not two): "Existing" and "New
  Construction" are created the first time the simple mode is used, so
  unphased projects stay free of phase data (owner / lead decision for
  M5-23). Elements without `phase_created` are existing in every phase.
- `PhaseStatus` has a sixth value, `past`: demolished in an earlier phase,
  hidden (Revit behaves the same).
- Commands: `create_phase`, `set_phase`, `delete_phase` (with `merge_into`),
  `set_element_phase`, `set_renovation_status`; query `get_phase_quantities`.
  Details in Foundation §3.10.


## Implementation notes (M5-23.6, phased cuts)

- A `cut` row's `phase` orders it (rows without a phase first) and decides
  where it applies: uncut before, removed part demolished in, cut after.
  Stored geometry is the latest state; bodies per phase come from the
  kernel on demand (`model::cut::body_in_phase`, `removed_in_phase`).
- `get_phase_quantities` counts the removed part as demolished volume of
  the element's category when the element is existing in that phase.
- The app switches cut / uncut by view phase; drawing the removed part as
  a red demolished solid is left for later (Foundation §3.10).
- 2026-11-24 (M5-23.10): the app draws the removed part in the row's phase
  for an element existing there, as the phase filter shows demolished
  elements: a display-only element (not in the document, not selectable)
  with the demolished look when overridden; nothing when hidden; the
  element as before the phase's rows under *No overrides*
  (`ferro-app` `phases::phased_cut_doc`).
- 2026-11-24 (M5-5.6): resolving a combination (`resolve`,
  `Session::resolve`, `execute_in_option`) regenerates with the session's
  kernel, so cuts on elements an option layer changes stay applied;
  `resolve` / `resolve_options` and `export_ifc*` take the kernel.

## Implementation notes (M5-23.9, design options; M5-26.2, M5-26.3)

- **Data.** One category `DesignOptions` with two bodies: `OptionSet
  {order}` and `DesignOption {order, is_primary, status, changes}`. An
  option's set is its `host_id` (containment; deleting a set deletes its
  options, and the change format already maps `host_id` to uids). Status
  `open` / `accepted` / `archived` (glossary `OptionStatus`). Every set with
  open options has exactly one primary; the primary cannot be deleted.
- **Option layers** use the M5-26 change format unchanged (M5-26.3):
  `/elements/<uid>...` and `/relationships/<uid>...` paths (M5-5 rows
  diff and apply like elements), never the project, option elements or
  `design_option`; derived data (`body/geometry`, read-only computed
  parameters) is not stored, `resolve` regenerates it. Layers are checked
  against the shared model on every write (`ferro.model.invalid_option_layer`).
- **Edit in option** is the editing model (not hand-written change lists):
  `Session::execute_in_option(option, combination, command)` runs the
  command on the document seen in the combination, then stores
  `diff(seen without this option, edited)` as the option layer with one
  `set_design_option`. Undo / redo, groups and the journal need nothing
  new; the journal replays the stored layer, so it stays deterministic.
  The app routes its commands this way while an option is being edited;
  MCP command tools take an optional `in_design_option`.
- **Target rule for agents (M5-14.8).** Agent commands and `run_python`
  scripts follow the user: while the user edits an option, they go into it
  by default. `in_design_option` = an option id wins; `"shared"` targets
  the shared model; omitted or `null` = the default (the edited option,
  else the shared model; headless hosts have no edited option).
  `ferro_tools::with_default_design_option` is the one place this rule
  is written. An agent's Proposal bar shows "into option: X" or "shared
  model". Agent commands into an option see the primary combination of the
  other sets (as an explicit `in_design_option` always did), the user's
  own commands the combination the view shows.
  `set_design_option {changes}` stays available for agents and sync.
- **`design_option` field (D-32) stays**, as a derived index: always `None`
  in the shared model and in files; in a document resolved for a
  combination it names the option whose layer added the element (used by
  the app to label and colour those elements). Membership is never stored
  twice, so it cannot disagree with the layer. A resolved document carries
  empty option layers, so resolving it again changes nothing.
- **Conflicts** between the option layers of a combination
  (`model::change::conflicts`) fail with `ferro.model.option_conflict`;
  nothing is merged silently.
- **Stale option layers.** A shared change that breaks an open option layer (e.g.
  deleting an element it overrides) commits with a
  `ferro.model.option_layer_stale` warning; the option layer is kept as it was, `resolve`
  of that option fails with `ferro.model.invalid_option_layer`, and the app
  offers "keep what still applies" (`change::partition_applicable`), an
  explicit `set_design_option`.
- **Ids.** `change::apply` now gives new elements ids from the document's
  next free id (never a reused id). Elements added by options exist only in
  resolved documents; their ids are stable while the shared model's next
  id does not move.
- **M5-26.2.** `apply_changes {changes}` is a command (validated, all or
  nothing, keeps option rules). `diff_documents {from, to}` is a query whose
  two sides are combinations of the open document (`"shared"`, `[]` for
  the primary combination, or option ids): its input is clear and it is
  what a comparison of options needs. Diffs against another file wait for
  sync (M8).
- **File format 5** (additive; a format 4 reader refuses the new category).
- **Interop.** IFC exports the primary combination (`export_ifc`), the app
  the combination shown. Revit gets the primary combination, with a
  `warning` operation when other open options exist.
- **New glossary terms:** `OptionSet`, `DesignOption`, option layer,
  `OptionStatus`, `Combination`, `Change`; verbs `accept`, `apply`, `diff`.
