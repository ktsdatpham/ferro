# 6. Interop: file format, MCP, Revit, IFC

## 6.1 The `.fro` file (D-21, D-43)

| Extension | Content | Revit analogue |
|---|---|---|
| `.fro` | Project document | `.rvt` |
| `.ffa` | Family definition (parameters + Form geometry + constraints) | `.rfa` |
| `.fte` | Project template | `.rte` |
| `.fro.json` / `.ffa.json` | JSON interchange form of the same schema | — |

- A `.fro` file is a **SQLite database** holding one `Document`: element table,
  geometry blobs, command journal (D-29), snapshots and embedded assets.
  Saves are incremental and crash-safe.
- The **JSON interchange form** (`.fro.json`) carries the same logical schema
  for AI agents, git diffs and external tools. Its top-level keys:

```json
{
  "format": "ferro",
  "format_version": 1,
  "app_version": "0.2.0",
  "project": { "units": {}, "info": {}, "origins": {}, "true_north_rad": 0.0 },
  "elements": [ { "uid": "…", "category": "walls", "type_uid": "…", "parameters": {}, "location": {}, "body": {} } ]
}
```

Rules:

1. Elements reference each other by **`Uid`** in files (never `ElementId`).
2. Keys are `snake_case` and stable. Removing or renaming a key requires bumping
   `format_version` and a migration function `migrate_v<n>_to_v<n+1>`.
3. Lengths are stored in mm, angles in radians, with no unit strings.
4. Readers must ignore unknown keys (forward compatibility).

## 6.2 MCP

- One `Command` = one MCP tool, same name and argument schema (chapter 4).
- Read-only tools (`list_elements`, `get_element`, `get_parameters`) do not open
  transactions.
- Tool arguments accept lengths as numbers (mm) or unit strings (`"3m"`), angles
  in **degrees** (agent-friendly), converted at the edge.
- Results return `uid`s alongside `id`s, plus any `Failure`s.
- Tool descriptions are written for an AI reader: purpose, units, preconditions,
  example.

## 6.3 Revit mapping

| Ferro | Revit |
|---|---|
| `Document` | `Document` |
| `Element.uid` | Stored on the Revit element in Extensible Storage (`FerroElement.Uid`, with the last sync signature); the Revit `UniqueId` is kept on the Ferro side of the sync state |
| `Category` | `BuiltInCategory` |
| `FamilyType` | `ElementType` / `FamilySymbol` |
| `WallType` / `FloorType` + `CompoundStructure` | Duplicated basic `WallType` / `FloorType` with `SetCompoundStructure`; layer functions map to `MaterialFunctionAssignment` |
| `Material` | `Material` (name, colour); `thermal` → Thermal asset, `physical` → Structural asset (`PropertySetElement`, SI converted with `UnitUtils`) |
| `Wall` | `Wall.Create` (+ `WALL_KEY_REF_PARAM`, `WALL_HEIGHT_TYPE` / `WALL_USER_HEIGHT_PARAM`, `WALL_TOP_OFFSET`); exterior on the left in both |
| `Floor` | `Floor.Create` with curve loops; `FLOOR_HEIGHTABOVELEVEL_PARAM` |
| `Opening` (door / window) | `FamilyInstance` of the first loaded door / window family, type duplicated per size, hosted by the wall |
| `Parameter` (built-in) | `BuiltInParameter` where one exists, otherwise shared parameter |
| `Level`, `Grid`, `ReferencePlane` | same |
| `WorkPlane` | `SketchPlane` |
| `Form` (unclassified) | `DirectShape` (category Generic Models / Mass) |
| `Form` classified as Walls / Floors | `DirectShape` with that category, or a native element after `convert` |
| `Location::Point / Curve` | `LocationPoint / LocationCurve` |
| Internal Origin / PBP / SP | same |
| Length mm | feet (convert at the C# / FFI edge only) |

Rules (ADR 0002):

1. Unit conversion to feet happens only in the Revit bridge (`ferro-revit`).
2. All mapping decisions are made in Rust (`ferro_revit::plan` /
   `ferro_revit::import`); the C# add-in only executes operations and reads
   the model.
3. Re-syncing updates elements matched by uid instead of duplicating them;
   unchanged elements (same signature) are not touched.
4. Changes coming from Revit enter Ferro as commands, as one proposal.

## 6.4 IFC mapping (export since milestone 3; import milestone 4)

| Ferro category | IFC4 class |
|---|---|
| `Walls` | `IfcWall` |
| `Floors` | `IfcSlab` (`FLOOR`) |
| `Roofs` | `IfcRoof` / `IfcSlab` (`ROOF`) |
| `Rooms` | `IfcSpace` |
| `Levels` | `IfcBuildingStorey` |
| `Doors`, `Windows` | `IfcDoor`, `IfcWindow` |
| `Forms` (unclassified) | `IfcBuildingElementProxy` |

`Uid` maps to `IfcGloballyUniqueId` (base64-compressed form of the same UUID).
Types map to `IfcWallType` / `IfcSlabType` with `IfcMaterialLayerSet`;
openings to `IfcOpeningElement` + `IfcRelVoidsElement` + `IfcRelFillsElement`;
quantities to `Qto_WallBaseQuantities` / `Qto_SlabBaseQuantities`; the uid
also to `Pset_FerroCommon.FerroUid`. Material assets become
`IfcMaterialProperties`: `Pset_MaterialCommon.MassDensity`,
`Pset_MaterialThermal` (`ThermalConductivity`, `SpecificHeatCapacity`),
`Pset_MaterialMechanical` (`YoungModulus`, `PoissonRatio`,
`ThermalExpansionCoefficient`) and `Pset_FerroMaterial` for the rest
(strengths, emissivity, solar absorptance, vapour resistance, GWP per stage,
EPD source, price); a complete U-value becomes
`Pset_WallCommon` / `Pset_SlabCommon.ThermalTransmittance` on the type. SI
derived units are declared only when such data exists. Geometry is `IfcTriangulatedFaceSet`
in millimetres.

Views (§3.11) are neither exported to IFC nor synced to Revit yet.
