# Ferro for Revit

Two-way link between Ferro and Revit 2025+ (ADR 0002). The add-in is a thin
C# shell: what to create, update or delete is decided by the Rust core
(`crates/ferro-revit`), reached over **Ferro Link** (the Ferro app open on the
same computer) or through `ferro_ffi.dll` (a `.fro.json` file).

```
Revit ── FerroRevit.dll (C#) ── Ferro Link (TCP, localhost) ── Ferro app ── ferro-revit
                             └─ ferro_ffi.dll (file mode) ──────────────── ferro-revit
```

## Buttons (ribbon tab "Ferro")

| Button | Does |
|---|---|
| **Pull from Ferro** | Brings levels, materials, wall / floor types, walls, floors, doors, windows and Forms (as DirectShapes) into Revit. Unchanged elements are skipped. Without the Ferro app open, asks for a `.fro.json` file. |
| **Push to Ferro** | Sends Revit levels, types, walls, floors, doors and windows to the open Ferro app. Changes arrive there as one proposal (Accept / Reject). New Revit elements get linked to their new Ferro elements. |
| **Live sync** | Pulls automatically within seconds whenever the Ferro model changes. |

Doors and windows use the first door / window family loaded in the project,
with one type per size ("Ferro 900 x 2100 mm"). Load the families you want
first.

## Install

From a Ferro release: copy the `revit-addin` folder anywhere, then copy
`Ferro.addin` to `%AppData%\Autodesk\Revit\Addins\2025\` and edit its
`<Assembly>` to the full path of `FerroRevit.dll`. Keep `ferro_ffi.dll` next to
`FerroRevit.dll`.

## Build

```bash
cargo build -p ferro-ffi --release            # ferro_ffi.dll
dotnet build revit-addin/FerroRevit.csproj -c Release
```

The project references Revit 2025 API reference assemblies from NuGet
(`Nice3point.Revit.Api.*`), so it builds without Revit installed (CI does).

## Status

- [x] Ferro → Revit: levels, materials, types with layers, walls (line / arc,
      top constraint, location line), floors with holes, doors, windows, Forms
- [x] Revit → Ferro: levels, types, walls, floors, doors, windows; deletions
- [x] Live pull
- [ ] Revit 2024 and older (.NET Framework 4.8)
- [ ] Roofs, rooms, grids, curved-wall openings, loadable families
