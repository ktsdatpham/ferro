// Ferro for Revit: ribbon, commands and live sync.
//
//   Pull     Ferro → Revit (from the open Ferro app, or a .fro.json file)
//   Push     Revit → Ferro (into the open Ferro app, as a proposal)
//   Live     pull automatically whenever the Ferro model changes

using System;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;
using Autodesk.Revit.Attributes;
using Autodesk.Revit.DB;
using Autodesk.Revit.UI;
using Autodesk.Revit.UI.Events;

namespace Ferro.Revit
{
    public class App : IExternalApplication
    {
        internal static bool Live;
        private static long _lastVersion = -1;
        private static DateTime _lastCheck = DateTime.MinValue;

        public Result OnStartup(UIControlledApplication app)
        {
            const string tab = "Ferro";
            try { app.CreateRibbonTab(tab); } catch (Autodesk.Revit.Exceptions.ArgumentException) { /* exists */ }
            var panel = app.CreateRibbonPanel(tab, "Link");
            var path = Assembly.GetExecutingAssembly().Location;
            panel.AddItem(new PushButtonData("FerroPull", "Pull\nfrom Ferro", path, typeof(PullCommand).FullName) { ToolTip = "Bring changes from the open Ferro app (or a .fro.json file) into this model." });
            panel.AddItem(new PushButtonData("FerroPush", "Push\nto Ferro", path, typeof(PushCommand).FullName) { ToolTip = "Send this model's walls, floors, doors, windows and levels to the open Ferro app as a proposal." });
            panel.AddItem(new PushButtonData("FerroLive", "Live\nsync", path, typeof(LiveCommand).FullName) { ToolTip = "Pull automatically whenever the Ferro model changes." });
            app.Idling += OnIdling;
            return Result.Succeeded;
        }

        public Result OnShutdown(UIControlledApplication app)
        {
            app.Idling -= OnIdling;
            return Result.Succeeded;
        }

        private static void OnIdling(object? sender, IdlingEventArgs e)
        {
            if (!Live || sender is not UIApplication ui || ui.ActiveUIDocument == null) return;
            if ((DateTime.Now - _lastCheck).TotalSeconds < 1.5) return;
            _lastCheck = DateTime.Now;
            var link = FerroLink.Find();
            if (link == null) return;
            try
            {
                var v = FerroLink.Call(link, "get_document_version", new JsonObject(), 2000)["version"]?.GetValue<long>() ?? -1;
                if (v == _lastVersion) return;
                _lastVersion = v;
                Sync.Pull(ui.ActiveUIDocument.Document, link);
            }
            catch (Exception) { /* Ferro closed or busy: try again later */ }
        }
    }

    internal static class Sync
    {
        /// <summary>Ferro → Revit through the open app.</summary>
        public static ApplyReport Pull(Document doc, LinkInfo link)
        {
            var body = FerroLink.Call(link, "revit_plan", new JsonObject { ["state"] = Snapshot.State(doc) });
            var ops = body["ops"]?.AsArray() ?? new JsonArray();
            return ops.Count == 0 ? new ApplyReport() : new Applier(doc).Apply(ops);
        }

        /// <summary>Ferro → Revit from a .fro.json file through ferro_ffi.dll.</summary>
        public static ApplyReport PullFile(Document doc, string path)
        {
            var handle = Native.ferro_document_from_json(Native.Utf8(File.ReadAllText(path)));
            if (handle == IntPtr.Zero) throw new InvalidOperationException("Not a valid .fro.json file.");
            try
            {
                var json = Native.TakeString(Native.ferro_revit_plan(handle, Native.Utf8(Snapshot.State(doc).ToJsonString())));
                var body = JsonNode.Parse(json) ?? new JsonObject();
                if (body["error"] is { } err) throw new InvalidOperationException(err.GetValue<string>());
                return new Applier(doc).Apply(body["ops"]!.AsArray());
            }
            finally { Native.ferro_document_free(handle); }
        }

        /// <summary>Revit → Ferro into the open app.</summary>
        public static string Push(Document doc, LinkInfo link)
        {
            var body = FerroLink.Call(link, "revit_import", new JsonObject { ["snapshot"] = Snapshot.Build(doc) });
            var report = body["report"] ?? new JsonObject();
            var linked = Snapshot.ApplyCreated(doc, report);
            var warnings = report["warnings"]?.AsArray().Select(w => w!.GetValue<string>()).ToList() ?? new();
            return $"{linked} new element(s) linked, {report["updated"]} updated, {report["deleted"]} deleted in Ferro." +
                   (warnings.Count == 0 ? "" : "\n" + string.Join("\n", warnings.Take(10))) +
                   "\nReview and accept the proposal in Ferro.";
        }
    }

    [Transaction(TransactionMode.Manual)]
    public class PullCommand : IExternalCommand
    {
        public Result Execute(ExternalCommandData data, ref string message, ElementSet elements)
        {
            var doc = data.Application.ActiveUIDocument.Document;
            try
            {
                var link = FerroLink.Find();
                ApplyReport r;
                if (link != null)
                {
                    r = Sync.Pull(doc, link);
                }
                else
                {
                    var dialog = new Microsoft.Win32.OpenFileDialog { Filter = "Ferro model (*.fro.json;*.json)|*.fro.json;*.json", Title = "Ferro is not open: pick a .fro.json file" };
                    if (dialog.ShowDialog() != true) return Result.Cancelled;
                    r = Sync.PullFile(doc, dialog.FileName);
                }
                TaskDialog.Show("Ferro", r.ToString());
                return Result.Succeeded;
            }
            catch (Exception ex) { message = ex.Message; return Result.Failed; }
        }
    }

    [Transaction(TransactionMode.Manual)]
    public class PushCommand : IExternalCommand
    {
        public Result Execute(ExternalCommandData data, ref string message, ElementSet elements)
        {
            var link = FerroLink.Find();
            if (link == null) { message = "Open the Ferro app first."; return Result.Failed; }
            try
            {
                TaskDialog.Show("Ferro", Sync.Push(data.Application.ActiveUIDocument.Document, link));
                return Result.Succeeded;
            }
            catch (Exception ex) { message = ex.Message; return Result.Failed; }
        }
    }

    [Transaction(TransactionMode.ReadOnly)]
    public class LiveCommand : IExternalCommand
    {
        public Result Execute(ExternalCommandData data, ref string message, ElementSet elements)
        {
            App.Live = !App.Live;
            TaskDialog.Show("Ferro", App.Live ? "Live sync on: changes in Ferro appear here within seconds." : "Live sync off.");
            return Result.Succeeded;
        }
    }

    internal static class Native
    {
        private const string Lib = "ferro_ffi";
        [DllImport(Lib)] public static extern IntPtr ferro_document_from_json(byte[] utf8Json);
        [DllImport(Lib)] public static extern IntPtr ferro_revit_plan(IntPtr doc, byte[] utf8State);
        [DllImport(Lib)] public static extern void ferro_document_free(IntPtr doc);
        [DllImport(Lib)] public static extern void ferro_string_free(IntPtr s);

        public static string TakeString(IntPtr p)
        {
            try { return Marshal.PtrToStringUTF8(p) ?? ""; }
            finally { ferro_string_free(p); }
        }

        public static byte[] Utf8(string s) => Encoding.UTF8.GetBytes(s + "\0");
    }
}
