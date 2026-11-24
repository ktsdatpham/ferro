// Reads the Revit model into the snapshot format of ferro-revit::import
// (lengths in feet, as Revit stores them) and the sync state for planning.

using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using Autodesk.Revit.DB;

namespace Ferro.Revit
{
    internal static class Snapshot
    {
        private static JsonArray P(XYZ p) => new() { p.X, p.Y, p.Z };

        private static double Num(Element e, BuiltInParameter p) => e.get_Parameter(p)?.AsDouble() ?? 0.0;

        private static ElementId Id(Element e, BuiltInParameter p) => e.get_Parameter(p)?.AsElementId() ?? ElementId.InvalidElementId;

        private static JsonNode? Uid(Element e) => Tags.Uid(e) is { } u ? JsonValue.Create(u) : null;

        /// <summary>What the model already holds, for revit_plan.</summary>
        public static JsonObject State(Document doc)
        {
            var arr = new JsonArray();
            foreach (var (uid, e) in Tags.Index(doc))
                arr.Add(new JsonObject { ["ferro_uid"] = uid, ["unique_id"] = e.UniqueId, ["signature"] = Tags.Signature(e) });
            return new JsonObject { ["elements"] = arr };
        }

        private static JsonObject HostType(Document doc, HostObjAttributes t)
        {
            var layers = new JsonArray();
            var cs = t.GetCompoundStructure();
            if (cs != null)
            {
                foreach (var l in cs.GetLayers())
                {
                    var mat = doc.GetElement(l.MaterialId) as Material;
                    layers.Add(new JsonObject
                    {
                        ["function"] = l.Function.ToString(),
                        ["width"] = l.Width,
                        ["material_name"] = mat?.Name,
                    });
                }
            }
            return new JsonObject { ["unique_id"] = t.UniqueId, ["ferro_uid"] = Uid(t), ["name"] = t.Name, ["layers"] = layers };
        }

        /// <summary>The Revit model for revit_import.</summary>
        public static JsonObject Build(Document doc)
        {
            var levels = new JsonArray();
            foreach (var l in new FilteredElementCollector(doc).OfClass(typeof(Level)).Cast<Level>().OrderBy(l => l.Elevation))
                levels.Add(new JsonObject { ["unique_id"] = l.UniqueId, ["ferro_uid"] = Uid(l), ["name"] = l.Name, ["elevation"] = l.Elevation, ["pinned"] = l.Pinned });

            var wallTypes = new Dictionary<ElementId, JsonObject>();
            var floorTypes = new Dictionary<ElementId, JsonObject>();

            var walls = new JsonArray();
            foreach (var w in new FilteredElementCollector(doc).OfClass(typeof(Wall)).Cast<Wall>())
            {
                if (w.WallType.Kind != WallKind.Basic || w.Location is not LocationCurve lc) continue;
                if (lc.Curve is not Line && lc.Curve is not Arc) continue;
                var c = lc.Curve;
                var curve = new JsonObject { ["start"] = P(c.GetEndPoint(0)), ["end"] = P(c.GetEndPoint(1)) };
                if (c is Arc) curve["mid"] = P(c.Evaluate(0.5, true));
                var top = Id(w, BuiltInParameter.WALL_HEIGHT_TYPE);
                var j = new JsonObject
                {
                    ["unique_id"] = w.UniqueId,
                    ["ferro_uid"] = Uid(w),
                    ["type_unique_id"] = w.WallType.UniqueId,
                    ["level_unique_id"] = doc.GetElement(Id(w, BuiltInParameter.WALL_BASE_CONSTRAINT))?.UniqueId ?? "",
                    ["curve"] = curve,
                    ["base_offset"] = Num(w, BuiltInParameter.WALL_BASE_OFFSET),
                    ["height"] = Num(w, BuiltInParameter.WALL_USER_HEIGHT_PARAM),
                    ["top_offset"] = Num(w, BuiltInParameter.WALL_TOP_OFFSET),
                    ["location_line"] = w.get_Parameter(BuiltInParameter.WALL_KEY_REF_PARAM)?.AsInteger() ?? 0,
                    ["pinned"] = w.Pinned,
                };
                if (top != ElementId.InvalidElementId && doc.GetElement(top) is Level tl) j["top_level_unique_id"] = tl.UniqueId;
                walls.Add(j);
                if (!wallTypes.ContainsKey(w.WallType.Id)) wallTypes[w.WallType.Id] = HostType(doc, w.WallType);
            }

            var floors = new JsonArray();
            foreach (var f in new FilteredElementCollector(doc).OfClass(typeof(Floor)).Cast<Floor>())
            {
                if (doc.GetElement(f.SketchId) is not Sketch sketch) continue;
                var loops = new JsonArray();
                foreach (CurveArray ring in sketch.Profile)
                {
                    var pts = new JsonArray();
                    foreach (Curve c in ring)
                    {
                        // Arcs are written as their tessellation (floors keep polygon outlines).
                        var t = c.Tessellate();
                        for (var i = 0; i < t.Count - 1; i++) pts.Add(P(t[i]));
                    }
                    loops.Add(pts);
                }
                floors.Add(new JsonObject
                {
                    ["unique_id"] = f.UniqueId,
                    ["ferro_uid"] = Uid(f),
                    ["type_unique_id"] = f.FloorType.UniqueId,
                    ["level_unique_id"] = doc.GetElement(f.LevelId)?.UniqueId ?? "",
                    ["loops"] = loops,
                    ["offset"] = Num(f, BuiltInParameter.FLOOR_HEIGHTABOVELEVEL_PARAM),
                    ["pinned"] = f.Pinned,
                });
                if (!floorTypes.ContainsKey(f.FloorType.Id)) floorTypes[f.FloorType.Id] = HostType(doc, f.FloorType);
            }

            var openings = new JsonArray();
            var cats = new ElementMulticategoryFilter(new[] { BuiltInCategory.OST_Doors, BuiltInCategory.OST_Windows });
            foreach (var fi in new FilteredElementCollector(doc).OfClass(typeof(FamilyInstance)).WherePasses(cats).Cast<FamilyInstance>())
            {
                if (fi.Host is not Wall host || fi.Location is not LocationPoint lp) continue;
                var door = fi.Category.Id.Value == (long)BuiltInCategory.OST_Doors;
                var sym = fi.Symbol;
                double Size(BuiltInParameter a, BuiltInParameter b, string n) =>
                    (fi.get_Parameter(a) ?? sym.get_Parameter(a) ?? sym.get_Parameter(b) ?? sym.LookupParameter(n))?.AsDouble() ?? 0.0;
                openings.Add(new JsonObject
                {
                    ["unique_id"] = fi.UniqueId,
                    ["ferro_uid"] = Uid(fi),
                    ["kind"] = door ? "door" : "window",
                    ["host_unique_id"] = host.UniqueId,
                    ["point"] = P(lp.Point),
                    ["width"] = door ? Size(BuiltInParameter.DOOR_WIDTH, BuiltInParameter.FAMILY_WIDTH_PARAM, "Width") : Size(BuiltInParameter.WINDOW_WIDTH, BuiltInParameter.FAMILY_WIDTH_PARAM, "Width"),
                    ["height"] = door ? Size(BuiltInParameter.DOOR_HEIGHT, BuiltInParameter.FAMILY_HEIGHT_PARAM, "Height") : Size(BuiltInParameter.WINDOW_HEIGHT, BuiltInParameter.FAMILY_HEIGHT_PARAM, "Height"),
                    ["sill"] = Num(fi, BuiltInParameter.INSTANCE_SILL_HEIGHT_PARAM),
                    ["pinned"] = fi.Pinned,
                });
            }

            var known = new JsonArray();
            foreach (var u in Tags.KnownUids(doc)) known.Add(u);
            return new JsonObject
            {
                ["known_ferro_uids"] = known,
                ["levels"] = levels,
                ["wall_types"] = new JsonArray(wallTypes.Values.Select(v => (JsonNode)v).ToArray()),
                ["floor_types"] = new JsonArray(floorTypes.Values.Select(v => (JsonNode)v).ToArray()),
                ["walls"] = walls,
                ["floors"] = floors,
                ["openings"] = openings,
            };
        }

        /// <summary>Tag Revit elements with the uids Ferro created for them.</summary>
        public static int ApplyCreated(Document doc, JsonNode report)
        {
            var n = 0;
            using var tx = new Transaction(doc, "Ferro: link elements");
            tx.Start();
            foreach (var pair in report["created"]?.AsArray() ?? new JsonArray())
            {
                var e = doc.GetElement(pair![0]!.GetValue<string>());
                if (e == null) continue;
                Tags.Set(e, pair[1]!.GetValue<string>(), "");
                n++;
            }
            Tags.SaveKnownUids(doc);
            tx.Commit();
            return n;
        }
    }
}
