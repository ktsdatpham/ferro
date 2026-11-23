// Ferro uid + signature stored on Revit elements (Extensible Storage), and
// the list of uids known after the last sync (on Project Information).

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using Autodesk.Revit.DB;
using Autodesk.Revit.DB.ExtensibleStorage;

namespace Ferro.Revit
{
    internal static class Tags
    {
        private static readonly Guid ElementSchemaId = new("8b3c2f4e-6a1d-4d7e-9f20-5c4b1a7e3d11");
        private static readonly Guid ProjectSchemaId = new("8b3c2f4e-6a1d-4d7e-9f20-5c4b1a7e3d12");

        private static Schema ElementSchema()
        {
            var s = Schema.Lookup(ElementSchemaId);
            if (s != null) return s;
            var b = new SchemaBuilder(ElementSchemaId);
            b.SetSchemaName("FerroElement");
            b.SetReadAccessLevel(AccessLevel.Public);
            b.SetWriteAccessLevel(AccessLevel.Public);
            b.SetVendorId("FERRO");
            b.AddSimpleField("Uid", typeof(string));
            b.AddSimpleField("Signature", typeof(string));
            return b.Finish();
        }

        private static Schema ProjectSchema()
        {
            var s = Schema.Lookup(ProjectSchemaId);
            if (s != null) return s;
            var b = new SchemaBuilder(ProjectSchemaId);
            b.SetSchemaName("FerroProject");
            b.SetReadAccessLevel(AccessLevel.Public);
            b.SetWriteAccessLevel(AccessLevel.Public);
            b.SetVendorId("FERRO");
            b.AddSimpleField("KnownUids", typeof(string));
            return b.Finish();
        }

        /// <summary>Ferro uid of an element, or null.</summary>
        public static string? Uid(Element e)
        {
            var s = Schema.Lookup(ElementSchemaId);
            if (s == null) return null;
            var ent = e.GetEntity(s);
            if (!ent.IsValid()) return null;
            var v = ent.Get<string>("Uid");
            return string.IsNullOrEmpty(v) ? null : v;
        }

        public static string Signature(Element e)
        {
            var s = Schema.Lookup(ElementSchemaId);
            if (s == null) return "";
            var ent = e.GetEntity(s);
            return ent.IsValid() ? ent.Get<string>("Signature") ?? "" : "";
        }

        /// <summary>Tag an element (inside a transaction).</summary>
        public static void Set(Element e, string uid, string signature)
        {
            var s = ElementSchema();
            var ent = new Entity(s);
            ent.Set("Uid", uid);
            ent.Set("Signature", signature);
            e.SetEntity(ent);
        }

        /// <summary>All tagged elements, by Ferro uid.</summary>
        public static Dictionary<string, Element> Index(Document doc)
        {
            var map = new Dictionary<string, Element>();
            var s = Schema.Lookup(ElementSchemaId);
            if (s == null) return map;
            var all = new FilteredElementCollector(doc).WherePasses(new ExtensibleStorageFilter(ElementSchemaId));
            foreach (var e in all)
            {
                var uid = Uid(e);
                if (uid != null) map[uid] = e;
            }
            // Types are not returned by instance collectors on some versions.
            var types = new FilteredElementCollector(doc).WhereElementIsElementType().WherePasses(new ExtensibleStorageFilter(ElementSchemaId));
            foreach (var e in types)
            {
                var uid = Uid(e);
                if (uid != null) map[uid] = e;
            }
            return map;
        }

        public static List<string> KnownUids(Document doc)
        {
            var s = Schema.Lookup(ProjectSchemaId);
            if (s == null) return new List<string>();
            var ent = doc.ProjectInformation.GetEntity(s);
            if (!ent.IsValid()) return new List<string>();
            try { return JsonSerializer.Deserialize<List<string>>(ent.Get<string>("KnownUids") ?? "[]") ?? new List<string>(); }
            catch (JsonException) { return new List<string>(); }
        }

        /// <summary>Remember the uids present now (inside a transaction).</summary>
        public static void SaveKnownUids(Document doc)
        {
            var uids = Index(doc).Keys.OrderBy(u => u).ToList();
            var ent = new Entity(ProjectSchema());
            ent.Set("KnownUids", JsonSerializer.Serialize(uids));
            doc.ProjectInformation.SetEntity(ent);
        }
    }
}
