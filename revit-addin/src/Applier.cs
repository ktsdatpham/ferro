// Executes the operations planned by the Rust core (ferro-revit::plan).
// No geometry decisions here: every length is already in feet and every
// placement already computed. Each operation is independent; one failure
// is reported and the rest continue.

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using Autodesk.Revit.DB;
using Autodesk.Revit.DB.Structure;

namespace Ferro.Revit
{
    internal sealed class ApplyReport
    {
        public int Created, Updated, Deleted;
        public List<string> Problems { get; } = new();
        public override string ToString() =>
            $"{Created} created, {Updated} updated, {Deleted} deleted" +
            (Problems.Count == 0 ? "" : $"\n{Problems.Count} problem(s):\n- " + string.Join("\n- ", Problems.Take(12)));
    }

    internal sealed class Applier
    {
        private readonly Document _doc;
        private readonly Dictionary<string, Element> _byUid;
        private readonly ApplyReport _report = new();

        public Applier(Document doc)
        {
            _doc = doc;
            _byUid = Tags.Index(doc);
        }

        /// <summary>Apply all operations inside one transaction.</summary>
        public ApplyReport Apply(JsonArray ops)
        {
            using var tx = new Transaction(_doc, "Ferro: pull changes");
            tx.Start();
            var options = tx.GetFailureHandlingOptions();
            options.SetFailuresPreprocessor(new QuietWarnings());
            tx.SetFailureHandlingOptions(options);
            foreach (var op in ops.OfType<JsonObject>())
            {
                var kind = Str(op, "op");
                try
                {
                    switch (kind)
                    {
                        case "delete": Delete(op); break;
                        case "material": Material(op); break;
                        case "level": Level(op); break;
                        case "wall_type": HostType(op, floor: false); break;
                        case "floor_type": HostType(op, floor: true); break;
                        case "wall": Wall(op); break;
                        case "floor": Floor(op); break;
                        case "opening": Opening(op); break;
                        case "direct_shape": DirectShape(op); break;
                        case "site_location": SiteLocationOf(op); break;
                        case "phase": PhaseOp(op); break;
                        default: _report.Problems.Add($"unknown operation {kind}"); break;
                    }
                }
                catch (Exception ex)
                {
                    _report.Problems.Add($"{kind} {Name(op)}: {ex.Message}");
                }
            }
            Tags.SaveKnownUids(_doc);
            tx.Commit();
            return _report;
        }

        // ------------------------------------------------------------ helpers

        private static string Str(JsonObject o, string k) => o[k]?.GetValue<string>() ?? "";
        private static string? OptStr(JsonObject o, string k) => o[k] is JsonValue v && v.TryGetValue<string>(out var s) ? s : null;
        private static double Num(JsonObject o, string k) => o[k]?.GetValue<double>() ?? 0.0;
        private static string Name(JsonObject o) => OptStr(o, "name") ?? Str(o, "uid");
        private static XYZ Xyz(JsonNode? p) => new(p![0]!.GetValue<double>(), p[1]!.GetValue<double>(), p[2]!.GetValue<double>());

        private Element? Existing(JsonObject op) =>
            _byUid.TryGetValue(Str(op, "uid"), out var e) && e.IsValidObject ? e : null;

        private T Need<T>(string uid, string what) where T : Element =>
            _byUid.TryGetValue(uid, out var e) && e.IsValidObject && e is T t
                ? t
                : throw new InvalidOperationException($"{what} {uid} is not in this model yet");

        private void Tag(Element e, JsonObject op, bool created)
        {
            Phases(e, op);
            Tags.Set(e, Str(op, "uid"), Str(op, "signature"));
            _byUid[Str(op, "uid")] = e;
            if (created) _report.Created++; else _report.Updated++;
        }

        private string UniqueName<T>(string name, Element? self) where T : Element
        {
            var taken = new FilteredElementCollector(_doc).OfClass(typeof(T))
                .Where(e => self == null || e.Id != self.Id).Select(e => e.Name).ToHashSet();
            if (!taken.Contains(name)) return name;
            for (var i = 2; ; i++)
                if (!taken.Contains($"{name} ({i})")) return $"{name} ({i})";
        }

        private static void SetParam(Element e, BuiltInParameter p, double v) { var x = e.get_Parameter(p); if (x != null && !x.IsReadOnly) x.Set(v); }
        private static void SetParam(Element e, BuiltInParameter p, int v) { var x = e.get_Parameter(p); if (x != null && !x.IsReadOnly) x.Set(v); }
        private static void SetParam(Element e, BuiltInParameter p, ElementId v) { var x = e.get_Parameter(p); if (x != null && !x.IsReadOnly) x.Set(v); }

        // ------------------------------------------------------------ operations

        private void Delete(JsonObject op)
        {
            var e = _doc.GetElement(Str(op, "unique_id"));
            if (e == null) return;
            _doc.Delete(e.Id);
            _byUid.Remove(Str(op, "ferro_uid"));
            _report.Deleted++;
        }

        /// <summary>Project location (M5-24.3): latitude / longitude in
        /// radians, time zone in hours, place name. Revit's SiteLocation
        /// elevation is read-only, so the op's `elevation` (feet) is not
        /// applied here.</summary>
        private void SiteLocationOf(JsonObject op)
        {
            var site = _doc.SiteLocation ?? throw new InvalidOperationException("this model has no site location");
            site.Latitude = Num(op, "latitude");
            site.Longitude = Num(op, "longitude");
            site.TimeZone = Num(op, "time_zone");
            var place = Str(op, "place");
            if (place.Length > 0) site.PlaceName = place;
            var created = Existing(op) == null;
            try { Tag(site, op, created); }
            catch (Exception ex)
            {
                // Values are applied; without the tag the op is just re-sent.
                _report.Problems.Add($"site location: not tagged ({ex.Message})");
            }
        }

        private void Material(JsonObject op)
        {
            var name = Str(op, "name");
            var m = Existing(op) as Material
                ?? new FilteredElementCollector(_doc).OfClass(typeof(Material)).Cast<Material>().FirstOrDefault(x => x.Name == name);
            var created = m == null;
            m ??= (Material)_doc.GetElement(Autodesk.Revit.DB.Material.Create(_doc, UniqueName<Material>(name, null)));
            var c = op["color"]!;
            m.Color = new Color((byte)c[0]!.GetValue<int>(), (byte)c[1]!.GetValue<int>(), (byte)c[2]!.GetValue<int>());
            if (op["thermal"] is JsonObject th) ThermalAssetOf(m, th);
            if (op["physical"] is JsonObject ph) StructuralAssetOf(m, ph);
            Tag(m, op, created);
        }

        // Optional SI number from an asset object (null when absent).
        private static double? Si(JsonObject o, string k) =>
            o[k] is JsonValue v && v.TryGetValue<double>(out var d) ? d : null;

        private static double ToInternal(double v, ForgeTypeId unit) => UnitUtils.ConvertToInternalUnits(v, unit);

        /// <summary>Thermal asset from Ferro's `thermal` (SI units): reuse the
        /// material's own property set or create one.</summary>
        private void ThermalAssetOf(Material m, JsonObject t)
        {
            var asset = new ThermalAsset(UniqueName<PropertySetElement>(m.Name + " (Ferro thermal)", null), ThermalMaterialType.Solid);
            if (Si(t, "conductivity_w_mk") is double k) asset.ThermalConductivity = ToInternal(k, UnitTypeId.WattsPerMeterKelvin);
            if (Si(t, "specific_heat_j_kgk") is double cp) asset.SpecificHeat = ToInternal(cp, UnitTypeId.JoulesPerKilogramDegreeCelsius);
            if (Si(t, "density_kg_m3") is double rho) asset.Density = ToInternal(rho, UnitTypeId.KilogramsPerCubicMeter);
            if (Si(t, "emissivity") is double e) asset.Emissivity = e;
            if (_doc.GetElement(m.ThermalAssetId) is PropertySetElement pse)
            {
                asset.Name = pse.Name;
                pse.SetThermalAsset(asset);
            }
            else
            {
                m.SetMaterialAspectByPropertySet(MaterialAspect.Thermal, PropertySetElement.Create(_doc, asset).Id);
            }
        }

        /// <summary>Structural (physical) asset from Ferro's `physical` (SI units).</summary>
        private void StructuralAssetOf(Material m, JsonObject p)
        {
            var asset = new StructuralAsset(UniqueName<PropertySetElement>(m.Name + " (Ferro physical)", null), StructuralAssetClass.Generic);
            if (Si(p, "density_kg_m3") is double rho) asset.Density = ToInternal(rho, UnitTypeId.KilogramsPerCubicMeter);
            if (Si(p, "youngs_modulus_pa") is double e) asset.SetYoungModulus(ToInternal(e, UnitTypeId.Pascals));
            if (Si(p, "poisson_ratio") is double nu) asset.SetPoissonRatio(nu);
            if (Si(p, "thermal_expansion_per_k") is double a) asset.SetThermalExpansionCoefficient(ToInternal(a, UnitTypeId.InverseDegreesCelsius));
            if (_doc.GetElement(m.StructuralAssetId) is PropertySetElement pse)
            {
                asset.Name = pse.Name;
                pse.SetStructuralAsset(asset);
            }
            else
            {
                m.SetMaterialAspectByPropertySet(MaterialAspect.Structural, PropertySetElement.Create(_doc, asset).Id);
            }
        }

        private void Level(JsonObject op)
        {
            var name = Str(op, "name");
            var elev = Num(op, "elevation");
            var lvl = Existing(op) as Level
                ?? new FilteredElementCollector(_doc).OfClass(typeof(Level)).Cast<Level>().FirstOrDefault(x => x.Name == name);
            var created = lvl == null;
            lvl ??= Autodesk.Revit.DB.Level.Create(_doc, elev);
            if (Math.Abs(lvl.Elevation - elev) > 1e-9) lvl.Elevation = elev;
            if (lvl.Name != name) lvl.Name = UniqueName<Level>(name, lvl);
            Tag(lvl, op, created);
        }

        // ------------------------------------------------------------ phases

        private Phase? PhaseNamed(string name)
        {
            foreach (Phase p in _doc.Phases)
                if (p.Name == name) return p;
            return null;
        }

        /// <summary>The Revit API cannot create phases: find the phase by name,
        /// else rename the phase at the same position, else report.</summary>
        private void PhaseOp(JsonObject op)
        {
            var name = Str(op, "name");
            if (PhaseNamed(name) != null) return;
            var order = (int)Num(op, "order");
            if (order >= 0 && order < _doc.Phases.Size)
            {
                var p = _doc.Phases[order];
                p.Name = name;
                _report.Updated++;
                return;
            }
            _report.Problems.Add($"phase {name}: add it in Manage > Phases (Revit cannot create phases from an add-in)");
        }

        /// <summary>PHASE_CREATED / PHASE_DEMOLISHED from phase names; only
        /// when the operation carries phases (unphased elements are left alone).</summary>
        private void Phases(Element e, JsonObject op)
        {
            var created = OptStr(op, "phase_created");
            var demolished = OptStr(op, "phase_demolished");
            if (created == null && demolished == null) return;
            if (created != null)
            {
                var p = PhaseNamed(created);
                if (p != null) SetParam(e, BuiltInParameter.PHASE_CREATED, p.Id);
                else _report.Problems.Add($"{Name(op)}: no phase {created}");
            }
            if (demolished == null) SetParam(e, BuiltInParameter.PHASE_DEMOLISHED, ElementId.InvalidElementId);
            else
            {
                var p = PhaseNamed(demolished);
                if (p != null) SetParam(e, BuiltInParameter.PHASE_DEMOLISHED, p.Id);
                else _report.Problems.Add($"{Name(op)}: no phase {demolished}");
            }
        }

        private CompoundStructure Structure(JsonObject op)
        {
            var layers = new List<CompoundStructureLayer>();
            foreach (var l in op["layers"]!.AsArray().OfType<JsonObject>())
            {
                var fn = Enum.TryParse<MaterialFunctionAssignment>(Str(l, "function"), out var f) ? f : MaterialFunctionAssignment.Structure;
                var mat = OptStr(l, "material_uid") is { } mu && _byUid.TryGetValue(mu, out var me) ? me.Id : ElementId.InvalidElementId;
                layers.Add(new CompoundStructureLayer(Num(l, "width"), fn, mat));
            }
            var cs = CompoundStructure.CreateSimpleCompoundStructure(layers);
            cs.SetNumberOfShellLayers(ShellLayerType.Exterior, (int)Num(op, "exterior_shells"));
            cs.SetNumberOfShellLayers(ShellLayerType.Interior, (int)Num(op, "interior_shells"));
            return cs;
        }

        private void HostType(JsonObject op, bool floor)
        {
            var name = Str(op, "name");
            HostObjAttributes? t = Existing(op) as HostObjAttributes;
            var created = t == null;
            if (t == null)
            {
                HostObjAttributes? template = floor
                    ? new FilteredElementCollector(_doc).OfClass(typeof(FloorType)).Cast<FloorType>().FirstOrDefault(x => !x.IsFoundationSlab)
                    : new FilteredElementCollector(_doc).OfClass(typeof(WallType)).Cast<WallType>().FirstOrDefault(x => x.Kind == WallKind.Basic);
                if (template == null) throw new InvalidOperationException(floor ? "no floor type to copy" : "no basic wall type to copy");
                t = (HostObjAttributes)(floor
                    ? ((FloorType)template).Duplicate(UniqueName<FloorType>(name, null))
                    : ((WallType)template).Duplicate(UniqueName<WallType>(name, null)));
            }
            else if (t.Name != name)
            {
                t.Name = floor ? UniqueName<FloorType>(name, t) : UniqueName<WallType>(name, t);
            }
            t.SetCompoundStructure(Structure(op));
            Tag(t, op, created);
        }

        private static Curve MakeCurve(JsonNode c)
        {
            var a = Xyz(c["start"]);
            var b = Xyz(c["end"]);
            return c["mid"] is JsonArray m ? Arc.Create(a, b, Xyz(m)) : Line.CreateBound(a, b);
        }

        private void Wall(JsonObject op)
        {
            var type = Need<WallType>(Str(op, "type_uid"), "wall type");
            var level = Need<Level>(Str(op, "level_uid"), "level");
            var curve = MakeCurve(op["curve"]!);
            var w = Existing(op) as Wall;
            var created = w == null;
            w ??= Autodesk.Revit.DB.Wall.Create(_doc, curve, type.Id, level.Id, Math.Max(Num(op, "height"), 1.0), Num(op, "base_offset"), false, false);
            if (w.WallType.Id != type.Id) w.ChangeTypeId(type.Id);
            SetParam(w, BuiltInParameter.WALL_BASE_CONSTRAINT, level.Id);
            SetParam(w, BuiltInParameter.WALL_BASE_OFFSET, Num(op, "base_offset"));
            // Location line first, then the curve, so the curve means that line.
            SetParam(w, BuiltInParameter.WALL_KEY_REF_PARAM, (int)Num(op, "location_line"));
            ((LocationCurve)w.Location).Curve = curve;
            if (OptStr(op, "top_level_uid") is { } top)
            {
                SetParam(w, BuiltInParameter.WALL_HEIGHT_TYPE, Need<Level>(top, "level").Id);
                SetParam(w, BuiltInParameter.WALL_TOP_OFFSET, Num(op, "top_offset"));
            }
            else
            {
                SetParam(w, BuiltInParameter.WALL_HEIGHT_TYPE, ElementId.InvalidElementId);
                SetParam(w, BuiltInParameter.WALL_USER_HEIGHT_PARAM, Num(op, "height"));
            }
            if (OptStr(op, "name") is { } n) w.get_Parameter(BuiltInParameter.ALL_MODEL_MARK)?.Set(n);
            Tag(w, op, created);
        }

        private void Floor(JsonObject op)
        {
            var type = Need<FloorType>(Str(op, "type_uid"), "floor type");
            var level = Need<Level>(Str(op, "level_uid"), "level");
            var loops = new List<CurveLoop>();
            foreach (var ring in op["loops"]!.AsArray().OfType<JsonArray>())
            {
                var pts = ring.Select(Xyz).ToList();
                var loop = new CurveLoop();
                for (var i = 0; i < pts.Count; i++) loop.Append(Line.CreateBound(pts[i], pts[(i + 1) % pts.Count]));
                loops.Add(loop);
            }
            // Floor sketches are replaced rather than edited (simple and reliable).
            var old = Existing(op);
            var created = old == null;
            if (old != null) _doc.Delete(old.Id);
            var f = Autodesk.Revit.DB.Floor.Create(_doc, loops, type.Id, level.Id);
            SetParam(f, BuiltInParameter.FLOOR_HEIGHTABOVELEVEL_PARAM, Num(op, "offset"));
            Tag(f, op, created);
        }

        private FamilySymbol Symbol(string category, double width, double height)
        {
            var bic = category == "OST_Windows" ? BuiltInCategory.OST_Windows : BuiltInCategory.OST_Doors;
            var symbols = new FilteredElementCollector(_doc).OfClass(typeof(FamilySymbol)).OfCategory(bic).Cast<FamilySymbol>().ToList();
            if (symbols.Count == 0) throw new InvalidOperationException($"load a {(bic == BuiltInCategory.OST_Doors ? "door" : "window")} family first");
            var name = $"Ferro {Math.Round(width * 304.8)} x {Math.Round(height * 304.8)} mm";
            var sym = symbols.FirstOrDefault(s => s.Name == name);
            if (sym == null)
            {
                sym = (FamilySymbol)symbols[0].Duplicate(name);
                var (wp, hp) = bic == BuiltInCategory.OST_Doors
                    ? (BuiltInParameter.DOOR_WIDTH, BuiltInParameter.DOOR_HEIGHT)
                    : (BuiltInParameter.WINDOW_WIDTH, BuiltInParameter.WINDOW_HEIGHT);
                SetSize(sym, width, wp, BuiltInParameter.FAMILY_WIDTH_PARAM, "Width");
                SetSize(sym, height, hp, BuiltInParameter.FAMILY_HEIGHT_PARAM, "Height");
            }
            if (!sym.IsActive) sym.Activate();
            return sym;
        }

        private static void SetSize(Element e, double v, BuiltInParameter a, BuiltInParameter b, string name)
        {
            var p = e.get_Parameter(a) ?? e.get_Parameter(b) ?? e.LookupParameter(name);
            if (p != null && !p.IsReadOnly) p.Set(v);
        }

        private void Opening(JsonObject op)
        {
            var host = Need<Wall>(Str(op, "host_uid"), "wall");
            var level = Need<Level>(Str(op, "level_uid"), "level");
            var sym = Symbol(Str(op, "category"), Num(op, "width"), Num(op, "height"));
            var point = Xyz(op["point"]);
            var fi = Existing(op) as FamilyInstance;
            var created = fi == null;
            if (fi != null && fi.Host?.Id != host.Id)
            {
                _doc.Delete(fi.Id);
                fi = null;
            }
            fi ??= _doc.Create.NewFamilyInstance(point, sym, host, level, StructuralType.NonStructural);
            if (fi.Symbol.Id != sym.Id) fi.Symbol = sym;
            if (fi.Location is LocationPoint lp && !lp.Point.IsAlmostEqualTo(point)) lp.Point = point;
            SetParam(fi, BuiltInParameter.INSTANCE_SILL_HEIGHT_PARAM, Num(op, "sill"));
            if (OptStr(op, "name") is { } n) fi.get_Parameter(BuiltInParameter.ALL_MODEL_MARK)?.Set(n);
            Tag(fi, op, created);
        }

        private void DirectShape(JsonObject op)
        {
            var verts = op["vertices"]!.AsArray().Select(Xyz).ToList();
            var builder = new TessellatedShapeBuilder();
            builder.OpenConnectedFaceSet(false);
            foreach (var t in op["triangles"]!.AsArray())
            {
                var tri = new List<XYZ> { verts[t![0]!.GetValue<int>()], verts[t[1]!.GetValue<int>()], verts[t[2]!.GetValue<int>()] };
                builder.AddFace(new TessellatedFace(tri, ElementId.InvalidElementId));
            }
            builder.CloseConnectedFaceSet();
            builder.Target = TessellatedShapeBuilderTarget.AnyGeometry;
            builder.Fallback = TessellatedShapeBuilderFallback.Mesh;
            builder.Build();
            var geometry = builder.GetBuildResult().GetGeometricalObjects();
            var ds = Existing(op) as DirectShape;
            var created = ds == null;
            if (ds == null)
            {
                var bic = Enum.TryParse<BuiltInCategory>(Str(op, "category"), out var c) ? c : BuiltInCategory.OST_GenericModel;
                if (!Autodesk.Revit.DB.DirectShape.IsValidCategoryId(new ElementId(bic), _doc)) bic = BuiltInCategory.OST_GenericModel;
                ds = Autodesk.Revit.DB.DirectShape.CreateElement(_doc, new ElementId(bic));
                ds.ApplicationId = "Ferro";
                ds.ApplicationDataId = Str(op, "uid");
            }
            ds.SetShape(geometry);
            ds.SetName(Name(op));
            Tag(ds, op, created);
        }
    }

    /// <summary>Keep sync quiet: warnings (e.g. overlapping walls) do not stop it.</summary>
    internal sealed class QuietWarnings : IFailuresPreprocessor
    {
        public FailureProcessingResult PreprocessFailures(FailuresAccessor a)
        {
            foreach (var f in a.GetFailureMessages())
                if (f.GetSeverity() == FailureSeverity.Warning) a.DeleteWarning(f);
            return FailureProcessingResult.Continue;
        }
    }
}
