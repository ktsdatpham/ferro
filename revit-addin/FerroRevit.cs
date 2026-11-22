// Ferro -> Revit bridge (stub).
//
// Revit's API is .NET-only, so this thin C# layer loads the Rust core
// (ferro_ffi.dll, built with `cargo build -p ferro-ffi --release`) via P/Invoke.
// Milestone 3 turns the JSON model into DirectShape / native walls & floors.
//
// Build: reference RevitAPI.dll and RevitAPIUI.dll from your Revit install,
// target the same .NET as your Revit version (net8.0-windows for Revit 2025+,
// .NET Framework 4.8 for 2024 and older), and copy ferro_ffi.dll next to the add-in.

using System;
using System.IO;
using System.Runtime.InteropServices;
using Autodesk.Revit.Attributes;
using Autodesk.Revit.DB;
using Autodesk.Revit.UI;

namespace Ferro.Revit
{
    internal static class Native
    {
        private const string Lib = "ferro_ffi";

        [DllImport(Lib)] public static extern IntPtr ferro_version();
        [DllImport(Lib)] public static extern IntPtr ferro_document_from_json(byte[] utf8Json);
        [DllImport(Lib)] public static extern long ferro_document_element_count(IntPtr model);
        [DllImport(Lib)] public static extern void ferro_document_free(IntPtr model);
        [DllImport(Lib)] public static extern void ferro_string_free(IntPtr s);

        public static string TakeString(IntPtr p)
        {
            try { return Marshal.PtrToStringUTF8(p) ?? ""; }
            finally { ferro_string_free(p); }
        }

        public static byte[] Utf8(string s) => System.Text.Encoding.UTF8.GetBytes(s + "\0");
    }

    [Transaction(TransactionMode.Manual)]
    public class ImportFerroCommand : IExternalCommand
    {
        public Result Execute(ExternalCommandData data, ref string message, ElementSet elements)
        {
            var dialog = new Microsoft.Win32.OpenFileDialog { Filter = "Ferro model (*.fro;*.json)|*.fro;*.json" };
            if (dialog.ShowDialog() != true) return Result.Cancelled;

            IntPtr model = Native.ferro_document_from_json(Native.Utf8(File.ReadAllText(dialog.FileName)));
            if (model == IntPtr.Zero) { message = "Not a valid Ferro model."; return Result.Failed; }

            try
            {
                long count = Native.ferro_document_element_count(model);
                string version = Native.TakeString(Native.ferro_version());
                // TODO (milestone 3): build DirectShape geometry per entity, map tags -> categories.
                TaskDialog.Show("Ferro", $"Ferro core {version}\nLoaded {count} entities.\nGeometry import coming in milestone 3.");
                return Result.Succeeded;
            }
            finally
            {
                Native.ferro_document_free(model);
            }
        }
    }
}
