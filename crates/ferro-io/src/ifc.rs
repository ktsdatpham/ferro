//! IFC4 export (D-75, milestone 3): STEP physical file (ISO 10303-21),
//! Reference View style.
//!
//! * Spatial structure: project → site → building → one storey per level
//!   marked as a building storey (other levels are not storeys).
//! * Walls, slabs, doors, windows and Forms (by their category's IFC class)
//!   with tessellated body geometry (`IfcTriangulatedFaceSet`) in millimetres,
//!   placed at the project origin (Internal Origin, D-04).
//! * Wall and slab types with material layer sets; doors and windows fill
//!   `IfcOpeningElement`s that void their wall.
//! * Base quantities (`Qto_WallBaseQuantities`, `Qto_SlabBaseQuantities`)
//!   and `Pset_FerroCommon.FerroUid` for round trips.
//!
//! GlobalIds are the element uids in IFC's base-64 encoding, so a re-export
//! keeps the same ids.

use crate::IoError;
use ferro_core::algo::body_mesh;
use ferro_core::model::bim::{CompoundStructure, OpeningKind};
use ferro_core::model::generate;
use ferro_core::model::{
    keys, Category, Document, Element, ElementBody, ElementId, ParamValue, Uid,
};
use std::collections::HashMap;
use std::fmt::Write;

const IFC_B64: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// IFC GlobalId (22 characters) of a 128-bit uid.
pub fn ifc_guid(u: u128) -> String {
    let mut out = String::with_capacity(22);
    // First character carries the top 2 bits, then 21 × 6 bits.
    out.push(IFC_B64[((u >> 126) & 0x3) as usize] as char);
    for i in (0..21).rev() {
        out.push(IFC_B64[((u >> (i * 6)) & 0x3f) as usize] as char);
    }
    out
}

/// Deterministic uid for IFC-only objects (project, site, relations...),
/// derived from a name so re-exports keep their ids.
fn derived(name: &str) -> String {
    let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for b in name.as_bytes() {
        h ^= *b as u128;
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    ifc_guid(h)
}

/// STEP string literal (ISO 10303-21 escapes; non-ASCII as \X2\ hex).
fn s(text: &str) -> String {
    let mut out = String::from("'");
    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                out.push_str("\\X2\\");
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "{:04X}", u);
                }
                out.push_str("\\X0\\");
            }
        }
    }
    out.push('\'');
    out
}

/// STEP real: always with a decimal point.
fn f(x: f64) -> String {
    let x = if x.abs() < 1e-12 { 0.0 } else { x };
    let t = format!("{x}");
    if t.contains('.') || t.contains('e') || t.contains('E') {
        t.replace('e', "E")
    } else {
        format!("{t}.")
    }
}

fn opt(text: Option<&str>) -> String {
    text.map(s).unwrap_or_else(|| "$".into())
}

struct Writer {
    lines: Vec<String>,
}

impl Writer {
    fn add(&mut self, entity: String) -> usize {
        self.lines.push(entity);
        self.lines.len()
    }

    fn point(&mut self, x: f64, y: f64, z: f64) -> usize {
        self.add(format!("IFCCARTESIANPOINT(({},{},{}))", f(x), f(y), f(z)))
    }

    fn dir(&mut self, x: f64, y: f64, z: f64) -> usize {
        self.add(format!("IFCDIRECTION(({},{},{}))", f(x), f(y), f(z)))
    }

    fn placement(&mut self, relative_to: Option<usize>, z: f64) -> usize {
        let p = self.point(0.0, 0.0, z);
        let ax = self.add(format!("IFCAXIS2PLACEMENT3D(#{p},$,$)"));
        let rel = relative_to
            .map(|r| format!("#{r}"))
            .unwrap_or_else(|| "$".into());
        self.add(format!("IFCLOCALPLACEMENT({rel},#{ax})"))
    }
}

fn ids(v: &[usize]) -> String {
    v.iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn color(doc: &Document, e: &Element) -> [f64; 3] {
    let structure = e
        .type_id
        .and_then(|t| doc.get(t))
        .and_then(|t| match &t.body {
            ElementBody::WallType(w) => Some(&w.structure),
            ElementBody::FloorType(f) => Some(&f.structure),
            _ => None,
        });
    if let Some(ElementBody::Material(m)) = structure
        .and_then(|s| s.core_material())
        .and_then(|m| doc.get(m))
        .map(|m| &m.body)
    {
        return m.color.map(|c| c as f64);
    }
    match e.category {
        Category::Doors => [0.62, 0.45, 0.30],
        Category::Windows => [0.62, 0.78, 0.90],
        Category::Floors => [0.82, 0.80, 0.76],
        _ => [0.90, 0.89, 0.85],
    }
}

/// Serialise a document as an IFC4 file.
pub fn export_ifc(doc: &Document) -> Result<String, IoError> {
    let mut w = Writer { lines: Vec::new() };
    let project_name = doc.project().name.clone();

    // Units and contexts.
    let len = w.add("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)".into());
    let area = w.add("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)".into());
    let vol = w.add("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)".into());
    let ang = w.add("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)".into());
    let units = w.add(format!("IFCUNITASSIGNMENT((#{len},#{area},#{vol},#{ang}))"));
    let origin = w.point(0.0, 0.0, 0.0);
    let world = w.add(format!("IFCAXIS2PLACEMENT3D(#{origin},$,$)"));
    let north = w.add("IFCDIRECTION((0.,1.))".into());
    let ctx = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#{world},#{north})"
    ));
    let body_ctx = w.add(format!(
        "IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,#{ctx},$,.MODEL_VIEW.,$)"
    ));

    let project = w.add(format!(
        "IFCPROJECT({},$,{},$,$,$,$,(#{ctx}),#{units})",
        s(&derived("project")),
        s(&project_name)
    ));
    let site_pl = w.placement(None, 0.0);
    let site = w.add(format!(
        "IFCSITE({},$,'Site',$,$,#{site_pl},$,$,.ELEMENT.,$,$,$,$,$)",
        s(&derived("site"))
    ));
    let bld_pl = w.placement(Some(site_pl), 0.0);
    let building = w.add(format!(
        "IFCBUILDING({},$,{},$,$,#{bld_pl},$,$,.ELEMENT.,$,$,$)",
        s(&derived("building")),
        s(&project_name)
    ));
    w.add(format!(
        "IFCRELAGGREGATES({},$,$,$,#{project},(#{site}))",
        s(&derived("rel:project-site"))
    ));
    w.add(format!(
        "IFCRELAGGREGATES({},$,$,$,#{site},(#{building}))",
        s(&derived("rel:site-building"))
    ));

    // Storeys.
    let mut storey_of: HashMap<ElementId, usize> = HashMap::new();
    let mut storeys = Vec::new();
    for l in doc.levels() {
        let Some(lv) = l.as_level() else { continue };
        if !lv.is_building_storey {
            continue;
        }
        let pl = w.placement(Some(bld_pl), lv.elevation.mm());
        let st = w.add(format!(
            "IFCBUILDINGSTOREY({},$,{},$,$,#{pl},$,$,.ELEMENT.,{})",
            s(&ifc_guid(l.uid.0)),
            s(&l.label()),
            f(lv.elevation.mm())
        ));
        storey_of.insert(l.id, st);
        storeys.push(st);
    }
    if !storeys.is_empty() {
        w.add(format!(
            "IFCRELAGGREGATES({},$,$,$,#{building},({}))",
            s(&derived("rel:building-storeys")),
            ids(&storeys)
        ));
    }
    // Elements on a non-storey level go to the storey below it (or the building).
    let container = |e: &Element| -> Option<ElementId> {
        let lvl = e.level_id?;
        let z = generate::level_elevation(doc, Some(lvl));
        doc.levels()
            .into_iter()
            .filter(|l| storey_of.contains_key(&l.id))
            .filter(|l| l.as_level().is_some_and(|x| x.elevation.mm() <= z + 1e-6))
            .map(|l| l.id)
            .next_back()
    };

    // Materials.
    let mut material_of: HashMap<ElementId, usize> = HashMap::new();
    for e in doc.elements() {
        if let ElementBody::Material(_) = e.body {
            let m = w.add(format!("IFCMATERIAL({},$,$)", s(&e.label())));
            material_of.insert(e.id, m);
        }
    }
    let default_material = w.add("IFCMATERIAL('Default',$,$)".into());
    let layer_set = |w: &mut Writer, name: &str, st: &CompoundStructure| -> usize {
        let layers: Vec<usize> = st
            .layers
            .iter()
            .map(|l| {
                let m = l
                    .material
                    .and_then(|m| material_of.get(&m).copied())
                    .unwrap_or(default_material);
                let cat = format!("{:?}", l.function);
                w.add(format!(
                    "IFCMATERIALLAYER(#{m},{},$,$,$,{},$)",
                    f(l.thickness.mm()),
                    s(&cat)
                ))
            })
            .collect();
        w.add(format!(
            "IFCMATERIALLAYERSET(({}),{},$)",
            ids(&layers),
            s(name)
        ))
    };

    // Types.
    let mut type_of: HashMap<ElementId, (usize, usize)> = HashMap::new(); // type → (ifc type, layer set)
    for e in doc.elements() {
        let (class, predefined, st) = match &e.body {
            ElementBody::WallType(t) => ("IFCWALLTYPE", ".STANDARD.", &t.structure),
            ElementBody::FloorType(t) => ("IFCSLABTYPE", ".FLOOR.", &t.structure),
            _ => continue,
        };
        let ls = layer_set(&mut w, &e.label(), st);
        let t = w.add(format!(
            "{class}({},$,{},$,$,$,$,$,$,{predefined})",
            s(&ifc_guid(e.uid.0)),
            s(&e.label())
        ));
        w.add(format!(
            "IFCRELASSOCIATESMATERIAL({},$,$,$,(#{t}),#{ls})",
            s(&derived(&format!("mat:{}", e.uid)))
        ));
        type_of.insert(e.id, (t, ls));
    }

    // Elements.
    let mut contained: HashMap<Option<ElementId>, Vec<usize>> = HashMap::new();
    let mut by_type: std::collections::BTreeMap<ElementId, Vec<usize>> = Default::default();
    let mut ifc_of: HashMap<ElementId, usize> = HashMap::new();
    let mut styles: HashMap<[u16; 3], usize> = HashMap::new();
    for e in doc.elements() {
        let Some(g) = e.geometry() else { continue };
        let mesh = body_mesh(g).map_err(|err| IoError::new("ferro.io.mesh", err.to_string()))?;
        if mesh.indices.is_empty() {
            continue;
        }
        let pts: Vec<String> = mesh
            .positions
            .iter()
            .map(|p| format!("({},{},{})", f(p.x), f(p.y), f(p.z)))
            .collect();
        let tris: Vec<String> = mesh
            .indices
            .iter()
            .map(|t| format!("({},{},{})", t[0] + 1, t[1] + 1, t[2] + 1))
            .collect();
        let list = w.add(format!("IFCCARTESIANPOINTLIST3D(({}),$)", pts.join(",")));
        let closed = if g.is_closed() { ".T." } else { ".F." };
        let faces = w.add(format!(
            "IFCTRIANGULATEDFACESET(#{list},$,{closed},({}),$)",
            tris.join(",")
        ));
        let c = color(doc, e);
        let key = c.map(|x| (x * 1000.0).round() as u16);
        let style = match styles.get(&key) {
            Some(st) => *st,
            None => {
                let rgb = w.add(format!(
                    "IFCCOLOURRGB($,{},{},{})",
                    f(c[0]),
                    f(c[1]),
                    f(c[2])
                ));
                let transparency = if e.category == Category::Windows {
                    0.6
                } else {
                    0.0
                };
                let shading = w.add(format!(
                    "IFCSURFACESTYLESHADING(#{rgb},{})",
                    f(transparency)
                ));
                let st = w.add(format!("IFCSURFACESTYLE($,.BOTH.,(#{shading}))"));
                styles.insert(key, st);
                st
            }
        };
        w.add(format!("IFCSTYLEDITEM(#{faces},(#{style}),$)"));
        let rep = w.add(format!(
            "IFCSHAPEREPRESENTATION(#{body_ctx},'Body','Tessellation',(#{faces}))"
        ));
        let shape = w.add(format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep}))"));
        let pl = w.placement(None, 0.0);
        let guid = s(&ifc_guid(e.uid.0));
        let name = opt(Some(&e.label()));
        let entity = match &e.body {
            ElementBody::Wall(_) => {
                format!("IFCWALL({guid},$,{name},$,$,#{pl},#{shape},$,.STANDARD.)")
            }
            ElementBody::Floor(_) => {
                format!("IFCSLAB({guid},$,{name},$,$,#{pl},#{shape},$,.FLOOR.)")
            }
            ElementBody::Opening(o) => match o.kind {
                OpeningKind::Door => format!(
                    "IFCDOOR({guid},$,{name},$,$,#{pl},#{shape},$,{},{},.DOOR.,$,$)",
                    f(o.height.mm()),
                    f(o.width.mm())
                ),
                OpeningKind::Window => format!(
                    "IFCWINDOW({guid},$,{name},$,$,#{pl},#{shape},$,{},{},.WINDOW.,$,$)",
                    f(o.height.mm()),
                    f(o.width.mm())
                ),
            },
            _ => {
                let class = e.category.ifc_class().to_uppercase();
                match class.as_str() {
                    "IFCWALL" => format!("IFCWALL({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                    "IFCSLAB" => format!("IFCSLAB({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                    "IFCROOF" => format!("IFCROOF({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                    "IFCCOLUMN" => format!("IFCCOLUMN({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                    "IFCFURNITURE" => format!("IFCFURNITURE({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                    "IFCCOVERING" => format!("IFCCOVERING({guid},$,{name},$,$,#{pl},#{shape},$,.CEILING.)"),
                    _ => format!("IFCBUILDINGELEMENTPROXY({guid},$,{name},$,$,#{pl},#{shape},$,.NOTDEFINED.)"),
                }
            }
        };
        let id = w.add(entity);
        ifc_of.insert(e.id, id);
        let host_container = match &e.body {
            ElementBody::Opening(_) => e.host_id.and_then(|h| doc.get(h)).and_then(container),
            _ => container(e),
        };
        contained.entry(host_container).or_default().push(id);
        if let Some(t) = e.type_id.filter(|t| type_of.contains_key(t)) {
            by_type.entry(t).or_default().push(id);
        }

        // Properties and quantities.
        let uid_prop = w.add(format!(
            "IFCPROPERTYSINGLEVALUE('FerroUid',$,IFCIDENTIFIER({}),$)",
            s(&e.uid.to_string())
        ));
        let pset = w.add(format!(
            "IFCPROPERTYSET({},$,'Pset_FerroCommon',$,(#{uid_prop}))",
            s(&derived(&format!("pset:{}", e.uid)))
        ));
        w.add(format!(
            "IFCRELDEFINESBYPROPERTIES({},$,$,$,(#{id}),#{pset})",
            s(&derived(&format!("relpset:{}", e.uid)))
        ));
        let lenp = |k: &str| match e.param(k) {
            Some(ParamValue::Length(l)) => Some(l.mm()),
            _ => None,
        };
        let volume = match e.param(keys::VOLUME) {
            Some(ParamValue::Volume(v)) => Some(v / 1e9),
            _ => None,
        };
        let mut q = Vec::new();
        let (qto, measures): (&str, Vec<(&str, Option<f64>)>) = match &e.body {
            ElementBody::Wall(_) => (
                "Qto_WallBaseQuantities",
                vec![
                    ("Length", lenp(keys::LENGTH)),
                    ("Height", lenp(keys::HEIGHT)),
                    ("Width", lenp(keys::THICKNESS)),
                ],
            ),
            ElementBody::Floor(_) => (
                "Qto_SlabBaseQuantities",
                vec![("Depth", lenp(keys::THICKNESS))],
            ),
            _ => ("", vec![]),
        };
        if !qto.is_empty() {
            for (n, v) in measures {
                if let Some(v) = v {
                    q.push(w.add(format!("IFCQUANTITYLENGTH({},$,$,{},$)", s(n), f(v))));
                }
            }
            if let Some(v) = volume {
                q.push(w.add(format!("IFCQUANTITYVOLUME('NetVolume',$,$,{},$)", f(v))));
            }
            if let Some(ParamValue::Area(a)) = e.param(keys::AREA) {
                q.push(w.add(format!("IFCQUANTITYAREA('NetArea',$,$,{},$)", f(a / 1e6))));
            }
            if !q.is_empty() {
                let eq = w.add(format!(
                    "IFCELEMENTQUANTITY({},$,{},$,'Ferro',({}))",
                    s(&derived(&format!("qto:{}", e.uid))),
                    s(qto),
                    ids(&q)
                ));
                w.add(format!(
                    "IFCRELDEFINESBYPROPERTIES({},$,$,$,(#{id}),#{eq})",
                    s(&derived(&format!("relqto:{}", e.uid)))
                ));
            }
        }
        // Layer usage on the wall / slab instance.
        if let Some((_, ls)) = e.type_id.and_then(|t| type_of.get(&t)) {
            let (dir, offset) = match &e.body {
                ElementBody::Wall(wl) => {
                    let off = generate::wall_layout(doc, e, wl)
                        .map(|l| -l.ext)
                        .unwrap_or(0.0);
                    (".AXIS2.", off)
                }
                _ => (".AXIS3.", 0.0),
            };
            let usage = w.add(format!(
                "IFCMATERIALLAYERSETUSAGE(#{ls},{dir},.POSITIVE.,{},$)",
                f(offset)
            ));
            w.add(format!(
                "IFCRELASSOCIATESMATERIAL({},$,$,$,(#{id}),#{usage})",
                s(&derived(&format!("matuse:{}", e.uid)))
            ));
        }
    }

    // Openings: void the wall, filled by the door / window.
    for e in doc.elements() {
        let ElementBody::Opening(o) = &e.body else {
            continue;
        };
        let (Some(&fill), Some(host)) = (ifc_of.get(&e.id), e.host_id.and_then(|h| doc.get(h)))
        else {
            continue;
        };
        let Some(&wall) = ifc_of.get(&host.id) else {
            continue;
        };
        let Some(lay) = host
            .as_wall()
            .and_then(|wl| generate::wall_layout(doc, host, wl).ok())
        else {
            continue;
        };
        let Some((st, d, n)) = lay.frame() else {
            continue;
        };
        // Box through the wall: origin at the opening's lower exterior-left corner.
        let a = st + d * (o.position.mm() - o.width.mm() * 0.5) + n * (lay.ext + 1.0);
        let p = w.point(a.x, a.y, lay.z0 + o.sill.mm());
        let zdir = w.dir(0.0, 0.0, 1.0);
        let xdir = w.dir(d.x, d.y, 0.0);
        let ax = w.add(format!("IFCAXIS2PLACEMENT3D(#{p},#{zdir},#{xdir})"));
        let p2 = w.add("IFCCARTESIANPOINT((0.,0.))".into());
        let ax2 = w.add(format!("IFCAXIS2PLACEMENT2D(#{p2},$)"));
        // Profile in the opening's local XY: width along the wall, depth into it.
        let depth = lay.thickness() + 2.0;
        let prof = w.add(format!(
            "IFCRECTANGLEPROFILEDEF(.AREA.,$,#{ax2},{},{})",
            f(o.width.mm()),
            f(depth)
        ));
        // The rectangle is centred on its placement: shift by half sizes.
        let centre = w.point(o.width.mm() * 0.5, -depth * 0.5, 0.0);
        let solid_ax = w.add(format!("IFCAXIS2PLACEMENT3D(#{centre},$,$)"));
        let solid = w.add(format!(
            "IFCEXTRUDEDAREASOLID(#{prof},#{solid_ax},#{zdir},{})",
            f(o.height.mm())
        ));
        let rep = w.add(format!(
            "IFCSHAPEREPRESENTATION(#{body_ctx},'Body','SweptSolid',(#{solid}))"
        ));
        let shape = w.add(format!("IFCPRODUCTDEFINITIONSHAPE($,$,(#{rep}))"));
        let lp = w.add(format!("IFCLOCALPLACEMENT($,#{ax})"));
        let opening = w.add(format!(
            "IFCOPENINGELEMENT({},$,'Opening',$,$,#{lp},#{shape},$,.OPENING.)",
            s(&derived(&format!("opening:{}", e.uid)))
        ));
        w.add(format!(
            "IFCRELVOIDSELEMENT({},$,$,$,#{wall},#{opening})",
            s(&derived(&format!("voids:{}", e.uid)))
        ));
        w.add(format!(
            "IFCRELFILLSELEMENT({},$,$,$,#{opening},#{fill})",
            s(&derived(&format!("fills:{}", e.uid)))
        ));
    }

    for (t, elems) in &by_type {
        let (it, _) = type_of[t];
        let uid = doc.get(*t).map(|e| e.uid).unwrap_or(Uid(0));
        w.add(format!(
            "IFCRELDEFINESBYTYPE({},$,$,$,({}),#{it})",
            s(&derived(&format!("type:{uid}"))),
            ids(elems)
        ));
    }
    let mut groups: Vec<_> = contained.into_iter().collect();
    groups.sort_by_key(|(k, _)| *k);
    for (lvl, elems) in groups {
        let (key, relating) = match lvl.and_then(|l| storey_of.get(&l).map(|s| (l, *s))) {
            Some((l, st)) => (format!("contain:{l}"), st),
            None => ("contain:building".to_string(), building),
        };
        w.add(format!(
            "IFCRELCONTAINEDINSPATIALSTRUCTURE({},$,$,$,({}),#{relating})",
            s(&derived(&key)),
            ids(&elems)
        ));
    }

    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    out.push_str("FILE_DESCRIPTION(('ViewDefinition [ReferenceView_V1.2]'),'2;1');\n");
    let _ = writeln!(
        out,
        "FILE_NAME({},'',(''),(''),{},{},'');",
        s(&format!("{project_name}.ifc")),
        s(&format!("Ferro {}", env!("CARGO_PKG_VERSION"))),
        s("Ferro")
    );
    out.push_str("FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n");
    for (i, l) in w.lines.iter().enumerate() {
        let _ = writeln!(out, "#{}={};", i + 1, l);
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    pub(crate) fn house() -> Session {
        let mut s = Session::new(Document::new("Nhà mẫu"), Box::new(SequentialIds::new(1)));
        for (name, args) in [
            ("create_level", json!({"name": "Tầng 1", "elevation": 0})),
            ("create_level", json!({"name": "Tầng 2", "elevation": 3300})),
            (
                "create_material",
                json!({"name": "Gạch", "color": "#b5533c"}),
            ),
            (
                "create_wall_type",
                json!({"name": "Tường 220", "layers": [
                {"function": "finish", "thickness": 10},
                {"function": "structure", "thickness": 200, "material": 3},
                {"function": "finish", "thickness": 10}]}),
            ),
            (
                "create_walls",
                json!({"points": [{"x":0,"y":0,"z":0},{"x":0,"y":6000,"z":0},{"x":8000,"y":6000,"z":0},{"x":8000,"y":0,"z":0}],
                "closed": true, "level": 1, "top_level": 2, "wall_type": 4}),
            ),
            (
                "create_opening",
                json!({"wall": 5, "kind": "door", "position": 2000}),
            ),
            (
                "create_opening",
                json!({"wall": 6, "kind": "window", "position": 4000}),
            ),
            (
                "create_floor",
                json!({"points": [{"x":0,"y":0},{"x":8000,"y":0},{"x":8000,"y":6000},{"x":0,"y":6000}], "level": 2}),
            ),
            (
                "create_box",
                json!({"origin": {"x": 10000, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
            ),
        ] {
            s.execute(
                AnyCommand::from_tool(name, &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap_or_else(|f| panic!("{name}: {f:?}"));
        }
        s
    }

    #[test]
    fn guids_are_22_chars_and_stable() {
        assert_eq!(ifc_guid(0), "0000000000000000000000");
        assert_eq!(ifc_guid(u128::MAX), "3$$$$$$$$$$$$$$$$$$$$$");
        assert_eq!(ifc_guid(u128::MAX).len(), 22);
        assert_eq!(derived("site"), derived("site"));
        assert_ne!(derived("site"), derived("building"));
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(s("it's"), "'it''s'");
        assert_eq!(s("Tầng"), "'T\\X2\\1EA7\\X0\\ng'");
        assert_eq!(f(3.0), "3.");
        assert_eq!(f(-0.0), "0.");
        assert_eq!(f(1e-20), "0.");
    }

    #[test]
    fn house_exports_spatial_structure_and_elements() {
        let s = house();
        let ifc = export_ifc(s.doc()).unwrap();
        assert!(ifc.starts_with("ISO-10303-21;"));
        assert!(ifc.contains("FILE_SCHEMA(('IFC4'))"));
        assert_eq!(ifc.matches("=IFCBUILDINGSTOREY(").count(), 2);
        assert_eq!(ifc.matches("=IFCWALL(").count(), 4);
        assert_eq!(ifc.matches("=IFCSLAB(").count(), 1);
        assert_eq!(ifc.matches("=IFCDOOR(").count(), 1);
        assert_eq!(ifc.matches("=IFCWINDOW(").count(), 1);
        assert_eq!(ifc.matches("=IFCOPENINGELEMENT(").count(), 2);
        assert_eq!(ifc.matches("=IFCBUILDINGELEMENTPROXY(").count(), 1);
        assert!(ifc.contains("Qto_WallBaseQuantities"));
        assert!(ifc.contains("IFCMATERIAL('G\\X2\\1EA1\\X0\\ch'"));
        if let Ok(path) = std::env::var("FERRO_IFC_OUT") {
            std::fs::write(path, &ifc).unwrap();
        }
        // Re-export is identical (deterministic ids).
        assert_eq!(ifc, export_ifc(s.doc()).unwrap());
        // Every reference points at an existing entity.
        let n = ifc.matches("\n#").count();
        for cap in ifc.split('#').skip(1) {
            let num: String = cap.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(i) = num.parse::<usize>() {
                assert!(i >= 1 && i <= n, "dangling #{i}");
            }
        }
    }
}
