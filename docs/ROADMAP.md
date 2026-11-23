# Roadmap

Each milestone should ship something a designer can actually try.

## Milestone 0: Foundation ✅ (this repo)

- [x] Cargo workspace, CI, licenses
- [x] `ferro-core`: faces, solids, rectangle, push-pull (outward normals), units parsing, JSON format
- [x] `ferro-mcp`: MCP server over stdio with 7 tools
- [x] `ferro-ffi`: C ABI + Revit C# stub

## Milestone 1: Viewport + SketchUp basics ✅ (v0.1.0)

- [x] Window and wgpu renderer, orbit / pan / zoom, grid, axes
- [x] Rectangle, Polygon, Push/Pull, Move / copy, Eraser; type exact values while drawing
- [x] Inference snapping; undo / redo; `.fro` files with autosave; OBJ / glTF export
- [x] Half-edge kernel with persistent ids; MCP server on kernel commands

## Milestone 2: BIM elements + AI inside the app ✅ (v0.2.0)

- [x] Walls (straight, curved, mitred corners), floors, doors, windows (`docs/milestones/M2.md`)
- [x] Wall / floor types with layers, materials; computed quantities
- [x] Semantic push/pull on BIM elements (height, length, opening size)
- [x] Ferro Link: agents edit the open document live; changes arrive as a proposal
- [x] Chat panel (bring your own API key, or a local model)
- [ ] Tools for selections ("make the selected face 500 mm taller") → M3
- [ ] Screenshot tool so agents can see the viewport → M3

## Milestone 3: Revit link

- [ ] Revit add-in: solids → `DirectShape`
- [ ] Tags → native elements (`category: wall` → Wall, `room` → Room, levels)
- [ ] Round-trip IDs so re-importing updates instead of duplicating
- [ ] Same C# shell, Rust core: no geometry logic in C#

## Milestone 4: Rhino-style power + interop

- [ ] Command line with autocomplete (every tool is also a command, and an MCP tool)
- [ ] IFC4 export (`IfcWall`, `IfcSlab`, `IfcSpace` from tags)
- [ ] Live link: Revit watches the `.fro` file and updates in place
- [ ] Plugin API (WASM) so the community can ship add-ins safely
- [ ] Parametric node editor (Grasshopper-style), later

## Milestone 5: Real-time rendering

- [ ] Materials, PBR shading, sun / sky lighting in the viewport
- [ ] Export glTF so it works with existing renderers
- [ ] iOS / iPad build (same Rust core, touch tools)

## Open questions

- Geometry kernel: grow our own (simple, ours) vs. bind OpenCascade (powerful, C++, LGPL)? Leaning: own half-edge mesh for SketchUp-style modelling, NURBS later.
- UI toolkit: `egui` (fast to build) vs. native shells (SwiftUI / WinUI via `ferro-ffi`)?
- Project name / trademark check before 1.0.
