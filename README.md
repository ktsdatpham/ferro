# Ferro

**An open-source, AI-native 3D design tool for architects. Written in Rust.**

*Ferro* means iron: the metal behind Rust, and behind ferro-cement. A
tool that lets designers sketch freely and fast, while real BIM data builds up
underneath, ready to hand to Revit for documentation.

> Status: **milestone 3 — Revit link and IFC.** Sketch modelling with
> SketchUp-style snapping and push/pull, real walls, floors, doors and windows
> with types and materials, AI agents that edit the open model as proposals,
> a **two-way link with Revit 2025+** and **IFC4 export**.

![Ferro milestone 3: changes pushed from Revit waiting for review](docs/images/screenshot-m3.png)

## Download

Builds for **Windows (x64)** and **macOS (Apple Silicon + Intel)** are produced
by CI on every change: open the latest run of
[Release builds](https://github.com/ktsdatpham/ferro/actions/workflows/release.yml)
and download the artifact. Tagged versions are published under
[Releases](https://github.com/ktsdatpham/ferro/releases). Builds are not
code-signed yet; see `packaging/README-install.txt` for the one-time
"Open anyway" step.

---

## Why

Revit is great at documentation and painful for design. Designers have to pick
wall types, levels and families before the idea even exists. So they sketch
in SketchUp or Rhino, and the model gets rebuilt in Revit later.

Ferro wants to be the design stage that doesn't throw work away:

| From | We take |
|---|---|
| **SketchUp** | Push-pull, inference snapping, drawing to scale fast, simple enough for anyone |
| **Rhino** | Command line with autocomplete, precise input, Grasshopper-style parametric nodes (later) |
| **AI agents** | A built-in **MCP server**: any AI (Claude, Cursor, local models) can read and model in your file |
| **Revit** | Not replaced, **complemented**: one-click link / export, plus a Revit add-in on the same Rust core |
| **Open source** | Public code + docs = AI agents and humans can both understand and extend it |

## Not doing

Keeping the scope tight matters more than piling on features. Ferro core is
**not** a documentation or sheet tool, and is **not** a full Revit clone.
Anything outside the core loop (sketch → AI → BIM data → Revit) should be an
add-in.

## What works today (milestone 3)

| Area | Status |
|---|---|
| Tools | Select, Rectangle, Polygon, Push/Pull, Move (Ctrl = copy), Eraser; Wall, Floor, Door, Window; type exact values while drawing (`4000,3000`, `3m`) |
| BIM | Walls (straight and curved, mitred corners and T-junctions, top attached to a level or fixed height), floors with holes, doors and windows that cut their wall; wall / floor types with layers and materials; push/pull a wall's top to change its height, its end to change its length |
| Snapping | Endpoint, midpoint, on edge, on face, red / green / blue axis inference, ground / active level |
| Model | Immutable document, elements with stable UUIDs, levels (Forms move with their level), Forms classifiable as walls / floors / roofs …, typed parameters, computed area and volume |
| History | Every change is one command + transaction: undo / redo, journal of who did what (user or AI agent) |
| Files | `.fro` (SQLite, autosave, crash-safe), `.fro.json` interchange, OBJ, glTF and **IFC4** export |
| Revit | Add-in for Revit 2025+: **Pull** Ferro → Revit as native levels, types, walls, floors, doors, windows (Forms as DirectShapes); **Push** Revit → Ferro as a proposal; **Live sync** |
| AI | `ferro-mcp`: every command is an MCP tool (units like `"3m"`, angles in degrees). With the app open, agents edit the open model over **Ferro Link**; their changes arrive as a proposal (Accept / Reject). Built-in **AI chat** with your own Anthropic key or any OpenAI-compatible endpoint (including local models) |
| Platforms | Windows, macOS, Linux; kernel builds for WebAssembly |

## Architecture

```
 ferro-app (egui + wgpu) ◄─Ferro Link─ ferro-mcp (AI agents)   Revit add-in (C#)
        │                         │                        │ P/Invoke
        │                         │                   ferro-ffi (C ABI)
        └──────────┬──────────────┴────────────────────────┘
                   ▼
   ferro-tools · ferro-snap (inference) · ferro-io (.fro, JSON, OBJ, glTF)
                   ▼
   ferro-core:  math → geom → topo → algo → model → ops
   (pure: no IO, no UI, deterministic, builds for wasm32)
```

| Crate | What it is |
|---|---|
| `crates/ferro-core` | Kernel: math, curves and profiles, half-edge B-rep with persistent ids, extrude / push-pull / triangulation, immutable `Document`, commands, transactions, undo |
| `crates/ferro-io` | `.fro` SQLite files (incremental autosave), `.fro.json`, OBJ / glTF export |
| `crates/ferro-snap` | Inference engine: SketchUp-style snapping in screen space |
| `crates/ferro-revit` | Revit link logic: Ferro → Revit operations (feet), Revit → Ferro import |
| `crates/ferro-tools` | Shared AI tool surface: schemas, unit conversion, dispatch, Ferro Link protocol |
| `crates/ferro-mcp` | MCP server over stdio; forwards to the open app over Ferro Link, else works headless with save / export |
| `crates/ferro-ffi` | C ABI for the Revit add-in and native shells |
| `crates/ferro-app` | The desktop app (`ferro`): viewport, tools, levels, properties |
| `revit-addin/` | Revit 2025+ add-in (C#): Pull / Push / Live sync, executes what `ferro-revit` decides |

## Build from source

You need [Rust](https://rustup.rs) (stable). On Linux also
`libgtk-3-dev libxkbcommon-dev libwayland-dev`.

```bash
git clone https://github.com/ktsdatpham/ferro
cd ferro
cargo test --workspace
cargo run -p ferro-app --release            # the app
cargo run -p ferro-app --release -- my.fro  # open a project
```

### Let an AI model with Ferro

```bash
cargo build -p ferro-mcp --release
```

Add it to your MCP client, e.g. Claude Desktop:

```json
{
  "mcpServers": {
    "ferro": { "command": "/absolute/path/to/ferro/target/release/ferro-mcp" }
  }
}
```

Open the Ferro app, then ask: *"Draw a 10 × 8 m house with 3 m brick walls, a
door and two windows on the south side, and a floor slab."* The walls appear in
the open app as a proposal; click **Accept** or **Reject**. (Without the app
open, ferro-mcp works on its own copy: ask it to save to `~/house.fro`.)

Prefer to stay in the app? **AI ▸ Chat settings** takes your own Anthropic key,
or the address of an OpenAI-compatible server such as a local Ollama.

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md). In short:

1. ✅ **Viewport + SketchUp basics**: window, orbit, rectangle, push-pull, snapping (`docs/milestones/M1.md`)
2. ✅ **BIM elements + AI in the app**: walls, floors, doors, windows, types, materials; Ferro Link, proposals, chat (`docs/milestones/M2.md`)
3. ✅ **Revit link + IFC**: two-way sync with native elements, IFC4 export, T-joins (`docs/milestones/M3.md`)
4. **Rhino-style power**: command line, parametric nodes, IFC export
5. **Real-time rendering**: materials and lighting in the viewport, wgpu PBR

## Contributing

Before writing code, read the **[Foundation Spec](docs/foundation/README.md)**:
the agreed units, coordinates, Revit-style data model, naming rules and
transaction model that every contribution (human or AI) must follow.

Every kind of help is welcome: Rust, geometry, UI, Revit API, docs,
translations, or just testing and telling us what hurts. Start with
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under the [Mozilla Public License 2.0](LICENSE). Ferro is **open core**: the desktop app, kernel, BIM tools, MCP server and plugin SDK are open; hosted services (AI, sync, cloud rendering, premium assets) are offered separately as Ferro Cloud.
