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
| `Element.uid` | `UniqueId` (stored in a shared parameter `Ferro_Uid` on export for round-trip) |
| `Category` | `BuiltInCategory` |
| `FamilyType` | `ElementType` / `FamilySymbol` |
| `Parameter` (built-in) | `BuiltInParameter` where one exists, otherwise shared parameter |
| `Level`, `Grid`, `ReferencePlane` | same |
| `WorkPlane` | `SketchPlane` |
| `Form` (unclassified) | `DirectShape` (category Generic Models / Mass) |
| `Form` classified as Walls / Floors | `DirectShape` with that category, or a native element after `convert` |
| `Location::Point / Curve` | `LocationPoint / LocationCurve` |
| Internal Origin / PBP / SP | same |
| Length mm | feet (convert at the C# / FFI edge only) |

Rules:

1. Unit conversion to feet happens only in the Revit bridge.
2. Re-importing the same Ferro file updates elements matched by `Ferro_Uid`
   instead of duplicating them.

## 6.4 IFC mapping (milestone 4)

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
