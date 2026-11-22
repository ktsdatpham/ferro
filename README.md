# Ferro

**An open-source, AI-native 3D design tool for architects. Written in Rust.**

*Ferro* means iron: the metal behind Rust, and behind ferro-cement. A
tool that lets designers sketch freely and fast, while real BIM data builds up
underneath, ready to hand to Revit for documentation.

> Status: **early prototype.** The kernel and the AI (MCP) server work. The
> visual app is next. Come build it with us.

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

## Architecture

```
            ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐
            │  ferro-app    │   │  ferro-mcp    │   │  Revit add-in    │
            │  (UI, wgpu)  │   │  (AI agents) │   │  (C# thin shell) │
            └──────┬───────┘   └──────┬───────┘   └────────┬─────────┘
                   │                  │                    │ P/Invoke
                   │                  │            ┌───────▼────────┐
                   │                  │            │   ferro-ffi     │
                   │                  │            │   (C ABI)      │
                   └────────┬─────────┴────────────┴───────┬────────┘
                            ▼                              ▼
                    ┌─────────────────────────────────────────┐
                    │  ferro-core: model, geometry, units      │
                    └─────────────────────────────────────────┘
```

| Crate | What it is |
|---|---|
| `crates/ferro-core` | The model kernel: entities, faces, solids, push-pull, units, JSON format. No UI, no IO. |
| `crates/ferro-mcp` | MCP server over stdio. Exposes tools such as `create_rectangle`, `push_pull`, `set_tags`, `list_entities`. |
| `crates/ferro-ffi` | C ABI for the Revit add-in and native shells (SwiftUI later). |
| `crates/ferro-app` | The app. Placeholder today; winit + wgpu viewport in milestone 1. |
| `revit-addin/` | C# stub that loads the Rust core inside Revit. |

## Quick start

You need [Rust](https://rustup.rs) (stable).

```bash
git clone https://github.com/ktsdatpham/ferro
cd ferro
cargo test --workspace      # run all tests
cargo run -p ferro-app       # headless demo: draws a room, prints the model
```

### Let an AI model with Ferro

```bash
cargo build -p ferro-mcp --release
```

Then add it to your MCP client. For example, in Claude Desktop's config:

```json
{
  "mcpServers": {
    "ferro": { "command": "/absolute/path/to/ferro/target/release/ferro-mcp" }
  }
}
```

Now ask: *"Draw a 4×3 m bedroom, 2.8 m high, and tag it as a room on level L1."*

## Roadmap

See [docs/ROADMAP.md](docs/ROADMAP.md). In short:

1. **Viewport + SketchUp basics**: window, orbit, rectangle, push-pull, snapping
2. **AI in the app**: chat panel driving the same MCP tools, live-updating the view
3. **Revit link**: solids → DirectShape, tags → native walls / floors / rooms
4. **Rhino-style power**: command line, parametric nodes, IFC export
5. **Real-time rendering**: materials and lighting in the viewport, wgpu PBR

## Contributing

Every kind of help is welcome: Rust, geometry, UI, Revit API, docs,
translations, or just testing and telling us what hurts. Start with
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
