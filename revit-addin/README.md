# Ferro for Revit (add-in stub)

Revit's API only works through .NET, so the add-in is a **thin C# shell** and all
real logic lives in the Rust core (`crates/ferro-ffi` → `ferro_ffi.dll`).

```
Revit  ──►  FerroRevit.cs (C#, P/Invoke)  ──►  ferro_ffi.dll (Rust)  ──►  ferro-core
```

## Build

1. `cargo build -p ferro-ffi --release` → `target/release/ferro_ffi.dll`
2. Create a Class Library project with `FerroRevit.cs`, referencing `RevitAPI.dll`
   and `RevitAPIUI.dll` from your Revit folder.
   - Revit 2025+: `net8.0-windows`
   - Revit 2024 and older: .NET Framework 4.8 (swap `Marshal.PtrToStringUTF8` for a manual UTF-8 decode)
3. Copy the add-in DLL and `ferro_ffi.dll` into the same folder and register it with a `.addin` manifest.

## Status

- [x] Load a `.fro` model through the Rust core
- [ ] Solids → `DirectShape` (milestone 3)
- [ ] Tags → native walls / floors / rooms (milestone 3)
- [ ] Live link: watch the `.fro` file and update in place (milestone 4)
