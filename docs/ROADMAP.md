# Roadmap

Each milestone should ship something a designer can actually try.

## Milestone 0: Foundation ✅ (this repo)

- [x] Cargo workspace, CI, licenses
- [x] `ferro-core`: faces, solids, rectangle, push-pull (outward normals), units parsing, JSON format
- [x] `ferro-mcp`: MCP server over stdio with 7 tools
- [x] `ferro-ffi`: C ABI + Revit C# stub

## Milestone 1: Viewport + SketchUp basics

- [ ] Window with `winit`, renderer with `wgpu` (Metal on Mac/iOS, DX12 on Windows, Vulkan on Linux)
- [ ] Orbit / pan / zoom camera, ground grid, axes
- [ ] Line, Rectangle and Push/Pull tools with mouse input
- [ ] Type exact values while drawing (VCB, e.g. `4000,3000` or `3m`)
- [ ] Inference snapping: endpoints, midpoints, on-axis, parallel
- [ ] Undo / redo (command pattern in `ferro-core`)
- [ ] Save / open `.ferro` files
- [ ] **Kernel upgrade:** replace the face list with a half-edge mesh so push-pull works on faces of existing solids

## Milestone 2: AI inside the app

- [ ] The app runs the MCP server in-process; agents edit the open document live
- [ ] Chat panel (bring your own API key, or a local model)
- [ ] Tools for selections ("make the selected face 500 mm taller")
- [ ] Grouping / components, so agents can work at the object level
- [ ] Screenshot tool so agents can see the viewport

## Milestone 3: Revit link

- [ ] Revit add-in: solids → `DirectShape`
- [ ] Tags → native elements (`category: wall` → Wall, `room` → Room, levels)
- [ ] Round-trip IDs so re-importing updates instead of duplicating
- [ ] Same C# shell, Rust core: no geometry logic in C#

## Milestone 4: Rhino-style power + interop

- [ ] Command line with autocomplete (every tool is also a command, and an MCP tool)
- [ ] IFC4 export (`IfcWall`, `IfcSlab`, `IfcSpace` from tags)
- [ ] Live link: Revit watches the `.ferro` file and updates in place
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
