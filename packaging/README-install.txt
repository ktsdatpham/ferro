Ferro — open-source, AI-native 3D design tool for architects
=============================================================

Windows
  Unzip, then run Ferro\ferro.exe.
  Windows SmartScreen may warn because the build is not code-signed yet:
  click "More info" → "Run anyway".

macOS (Apple Silicon and Intel)
  Unzip and move Ferro.app to Applications.
  The build is not notarised yet. The first time: right-click Ferro.app → Open
  → Open. (Or: System Settings → Privacy & Security → "Open Anyway".)

Using Ferro
  Space Select · R Rectangle · G Polygon · P Push/Pull · M Move · E Eraser
  Middle / right mouse drag: orbit · Shift + drag: pan · Scroll / pinch: zoom
  Z zoom extents · Ctrl/Cmd+Z undo · Ctrl/Cmd+S save (.fro)
  While a tool is active, just type a value (e.g. 4000,3000 or 3m) and press Enter.

Connect an AI agent (Claude Desktop, Claude Code, Cursor …)
  Ferro ships an MCP server next to the app: ferro-mcp (ferro-mcp.exe on Windows;
  inside Ferro.app/Contents/MacOS on macOS). In the app: AI → Connect an AI agent.

Licence: MPL-2.0 · https://github.com/ktsdatpham/ferro
