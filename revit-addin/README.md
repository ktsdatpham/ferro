# Phac for Revit (add-in stub)

Revit's API only works through .NET, so the add-in is a **thin C# shell** and all
real logic lives in the Rust core (`crates/phac-ffi` → `phac_ffi.dll`).

```
Revit  ──►  PhacRevit.cs (C#, P/Invoke)  ──►  phac_ffi.dll (Rust)  ──►  phac-core
```

## Build

1. `cargo build -p phac-ffi --release` → `target/release/phac_ffi.dll`
2. Create a Class Library project with `PhacRevit.cs`, referencing `RevitAPI.dll`
   and `RevitAPIUI.dll` from your Revit folder.
   - Revit 2025+: `net8.0-windows`
   - Revit 2024 and older: .NET Framework 4.8 (swap `Marshal.PtrToStringUTF8` for a manual UTF-8 decode)
3. Copy the add-in DLL and `phac_ffi.dll` into the same folder and register it with a `.addin` manifest.

## Status

- [x] Load a `.phac` model through the Rust core
- [ ] Solids → `DirectShape` (milestone 3)
- [ ] Tags → native walls / floors / rooms (milestone 3)
- [ ] Live link: watch the `.phac` file and update in place (milestone 4)
