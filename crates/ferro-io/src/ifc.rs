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
//! * Material property sets (`Pset_MaterialCommon`, `Pset_MaterialThermal`,
//!   `Pset_MaterialMechanical`, `Pset_FerroMaterial`) and type
//!   `ThermalTransmittance` when materials carry data, with SI units.
//!
//! * Phases (ADR 0006): with phases, the file shows the export phase (the
//!   last by default): elements not built yet or already demolished are left
//!   out, the others get `Pset_<Class>Common.Status` = `NEW`, `EXISTING`,
//!   `DEMOLISH` or `TEMPORARY` (IFC4 `PEnum_ElementStatus`). Phases are not
//!   IFC products and are not exported themselves.
//!
//! GlobalIds are the element uids in IFC's base-64 encoding, so a re-export
//! keeps the same ids.

use crate::IoError;
use ferro_core::algo::body_mesh;
use ferro_core::model::bim::{CompoundStructure, OpeningKind};
use ferro_core::model::generate;
use ferro_core::model::structure_quantities::StructureQuantities;
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

/// An `IfcCompoundPlaneAngleMeasure` for an angle in decimal degrees:
/// `(degrees,minutes,seconds,millionths-of-a-second)`. IFC4 requires every
/// component to have the angle's sign, so -33.87° is `(-33,-52,-12,0)`.
/// Non-finite input gives `$`.
fn compound_angle(deg: f64) -> String {
    if !deg.is_finite() {
        return "$".into();
    }
    // Whole millionths of an arc second, rounded once so 16.05 is exact.
    let total = (deg.abs() * 3600.0 * 1e6).round() as i64;
    let sign = if deg < 0.0 && total != 0 { -1 } else { 1 };
    let d = total / 3_600_000_000;
    let m = total / 60_000_000 % 60;
    let sec = total / 1_000_000 % 60;
    let micro = total % 1_000_000;
    format!(
        "({},{},{},{})",
        sign * d,
        sign * m,
        sign * sec,
        sign * micro
    )
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

/// SI units for material and type properties: the named units go into the
/// project's `IfcUnitAssignment` (returned ids); base units used only inside
/// derived units are written but not returned. All are SI base / derived
/// units (kg, K, Pa, kg/m³, W/(m·K), J/(kg·K), 1/K, W/(m²·K)), matching the
/// values stored in `ferro_core::model::material`.
fn property_units(w: &mut Writer) -> Vec<usize> {
    let kg = w.add("IFCSIUNIT(*,.MASSUNIT.,.KILO.,.GRAM.)".into());
    let kelvin = w.add("IFCSIUNIT(*,.THERMODYNAMICTEMPERATUREUNIT.,$,.KELVIN.)".into());
    let pascal = w.add("IFCSIUNIT(*,.PRESSUREUNIT.,$,.PASCAL.)".into());
    // Not assigned (the project length unit is mm): only used below.
    let metre = w.add("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)".into());
    let watt = w.add("IFCSIUNIT(*,.POWERUNIT.,$,.WATT.)".into());
    let joule = w.add("IFCSIUNIT(*,.ENERGYUNIT.,$,.JOULE.)".into());
    let derived_unit = |w: &mut Writer, kind: &str, parts: &[(usize, i32)]| -> usize {
        let elems: Vec<usize> = parts
            .iter()
            .map(|(u, e)| w.add(format!("IFCDERIVEDUNITELEMENT(#{u},{e})")))
            .collect();
        w.add(format!("IFCDERIVEDUNIT(({}),.{kind}.,$)", ids(&elems)))
    };
    let density = derived_unit(w, "MASSDENSITYUNIT", &[(kg, 1), (metre, -3)]);
    let conductivity = derived_unit(
        w,
        "THERMALCONDUCTANCEUNIT",
        &[(watt, 1), (metre, -1), (kelvin, -1)],
    );
    let specific_heat = derived_unit(
        w,
        "SPECIFICHEATCAPACITYUNIT",
        &[(joule, 1), (kg, -1), (kelvin, -1)],
    );
    let modulus = derived_unit(w, "MODULUSOFELASTICITYUNIT", &[(pascal, 1)]);
    let expansion = derived_unit(w, "THERMALEXPANSIONCOEFFICIENTUNIT", &[(kelvin, -1)]);
    let transmittance = derived_unit(
        w,
        "THERMALTRANSMITTANCEUNIT",
        &[(watt, 1), (metre, -2), (kelvin, -1)],
    );
    vec![
        kg,
        kelvin,
        pascal,
        density,
        conductivity,
        specific_heat,
        modulus,
        expansion,
        transmittance,
    ]
}

/// `IfcMaterialProperties` of a material, one per property set that has
/// values: `Pset_MaterialCommon` (MassDensity), `Pset_MaterialThermal`
/// (ThermalConductivity, SpecificHeatCapacity), `Pset_MaterialMechanical`
/// (YoungModulus, PoissonRatio, ThermalExpansionCoefficient) as named in
/// IFC4, and `Pset_FerroMaterial` for values IFC4 has no material property
/// for (strengths, emissivity, solar absorptance, vapour resistance, GWP per
/// stage with declared unit and EPD source, unit price).
fn material_psets(w: &mut Writer, mat: usize, m: &ferro_core::model::bim::Material) {
    let prop = |w: &mut Writer, name: &str, measure: &str, v: Option<f64>| -> Option<usize> {
        v.filter(|v| v.is_finite()).map(|v| {
            w.add(format!(
                "IFCPROPERTYSINGLEVALUE({},$,{measure}({}),$)",
                s(name),
                f(v)
            ))
        })
    };
    let text = |w: &mut Writer, name: &str, measure: &str, v: Option<&str>| -> Option<usize> {
        v.map(|v| {
            w.add(format!(
                "IFCPROPERTYSINGLEVALUE({},$,{measure}({}),$)",
                s(name),
                s(v)
            ))
        })
    };
    let pset = |w: &mut Writer, name: &str, props: Vec<Option<usize>>| {
        let props: Vec<usize> = props.into_iter().flatten().collect();
        if !props.is_empty() {
            w.add(format!(
                "IFCMATERIALPROPERTIES({},$,({}),#{mat})",
                s(name),
                ids(&props)
            ));
        }
    };
    let ph = m.physical.clone().unwrap_or_default();
    let th = m.thermal.clone().unwrap_or_default();

    let p = vec![prop(
        w,
        "MassDensity",
        "IFCMASSDENSITYMEASURE",
        m.density_kg_m3(),
    )];
    pset(w, "Pset_MaterialCommon", p);
    let p = vec![
        prop(
            w,
            "SpecificHeatCapacity",
            "IFCSPECIFICHEATCAPACITYMEASURE",
            th.specific_heat_j_kgk,
        ),
        prop(
            w,
            "ThermalConductivity",
            "IFCTHERMALCONDUCTIVITYMEASURE",
            th.conductivity_w_mk,
        ),
    ];
    pset(w, "Pset_MaterialThermal", p);
    let p = vec![
        prop(
            w,
            "YoungModulus",
            "IFCMODULUSOFELASTICITYMEASURE",
            ph.youngs_modulus_pa,
        ),
        // IfcPositiveRatioMeasure: auxetic (ν ≤ 0) values are not written.
        prop(
            w,
            "PoissonRatio",
            "IFCPOSITIVERATIOMEASURE",
            ph.poisson_ratio.filter(|v| *v > 0.0),
        ),
        prop(
            w,
            "ThermalExpansionCoefficient",
            "IFCTHERMALEXPANSIONCOEFFICIENTMEASURE",
            ph.thermal_expansion_per_k,
        ),
    ];
    pset(w, "Pset_MaterialMechanical", p);

    let mut p = vec![
        prop(
            w,
            "CompressiveStrength",
            "IFCPRESSUREMEASURE",
            ph.compressive_strength_pa,
        ),
        prop(
            w,
            "TensileStrength",
            "IFCPRESSUREMEASURE",
            ph.tensile_strength_pa,
        ),
        prop(w, "Emissivity", "IFCNORMALISEDRATIOMEASURE", th.emissivity),
        prop(
            w,
            "SolarAbsorptance",
            "IFCNORMALISEDRATIOMEASURE",
            th.solar_absorptance,
        ),
        prop(
            w,
            "VapourResistanceFactor",
            "IFCPOSITIVERATIOMEASURE",
            th.vapour_resistance_factor,
        ),
    ];
    if let Some(env) = &m.environmental {
        p.push(text(
            w,
            "GWPDeclaredUnit",
            "IFCLABEL",
            Some(env.declared_unit.as_str()),
        ));
        for (k, v) in env.gwp_stages() {
            // e.g. GWP_A1_A3, kg CO2e per declared unit.
            p.push(prop(w, &k.to_uppercase(), "IFCREAL", v));
        }
        p.push(text(w, "EPDSource", "IFCTEXT", env.epd_source.as_deref()));
    }
    if let Some(c) = &m.cost {
        p.push(prop(w, "UnitPrice", "IFCREAL", Some(c.unit_price)));
        p.push(text(w, "Currency", "IFCLABEL", Some(&c.currency)));
        p.push(text(w, "PriceUnit", "IFCLABEL", Some(c.unit.as_str())));
    }
    pset(w, "Pset_FerroMaterial", p);
}

/// Serialise a document as an IFC4 file.
pub fn export_ifc(doc: &Document) -> Result<String, IoError> {
    export_ifc_phase(doc, None)
}

/// IFC4 export of the document as it is in `phase` (`None` = the last
/// phase; ignored when the document has no phases). See the module docs
/// for how phases are written.
pub fn export_ifc_phase(doc: &Document, phase: Option<ElementId>) -> Result<String, IoError> {
    let export_phase = phase
        .filter(|p| doc.phase_index(*p).is_some())
        .or_else(|| doc.phases().last().map(|p| p.id));
    let mut w = Writer { lines: Vec::new() };
    let project_name = doc.project().name.clone();

    // Units and contexts.
    let len = w.add("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)".into());
    let area = w.add("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)".into());
    let vol = w.add("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)".into());
    let ang = w.add("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)".into());
    let mut unit_ids = vec![len, area, vol, ang];
    // Units of material / type properties, only when there are some, so
    // files without material data are unchanged.
    let has_material_data = doc.elements().any(|e| match &e.body {
        ElementBody::Material(m) => {
            m.physical.is_some()
                || m.thermal.is_some()
                || m.environmental.is_some()
                || m.cost.is_some()
        }
        _ => false,
    });
    if has_material_data {
        unit_ids.extend(property_units(&mut w));
    }
    let units = w.add(format!("IFCUNITASSIGNMENT(({}))", ids(&unit_ids)));
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
    // RefLatitude / RefLongitude / RefElevation from the project's site
    // location (M5-24.3); `$` when the project has none. LongName = place.
    let (long_name, lat, lon, elev) = match &doc.project().settings.location {
        Some(l) => (
            if l.place.trim().is_empty() {
                "$".to_string()
            } else {
                s(l.place.trim())
            },
            compound_angle(l.latitude.deg()),
            compound_angle(l.longitude.deg()),
            // IfcLengthMeasure in the project length unit (mm, see above).
            f(l.elevation.mm()),
        ),
        None => ("$".into(), "$".into(), "$".into(), "$".into()),
    };
    let site = w.add(format!(
        "IFCSITE({},$,'Site',$,$,#{site_pl},$,{long_name},.ELEMENT.,{lat},{lon},{elev},$,$)",
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
    for e in doc.elements() {
        if let (ElementBody::Material(m), Some(&mat)) = (&e.body, material_of.get(&e.id)) {
            material_psets(&mut w, mat, m);
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
        // Pset_WallCommon / Pset_SlabCommon.ThermalTransmittance when every
        // layer has a conductivity (ISO 6946, see StructureQuantities).
        let u_value = StructureQuantities::from_type(doc, e.id).and_then(|q| q.u_value_w_m2k);
        let psets = match u_value {
            Some(u) if has_material_data => {
                let pset_name = if class == "IFCWALLTYPE" {
                    "Pset_WallCommon"
                } else {
                    "Pset_SlabCommon"
                };
                let p = w.add(format!(
                    "IFCPROPERTYSINGLEVALUE('ThermalTransmittance',$,IFCTHERMALTRANSMITTANCEMEASURE({}),$)",
                    f(u)
                ));
                let ps = w.add(format!(
                    "IFCPROPERTYSET({},$,{},$,(#{p}))",
                    s(&derived(&format!("typepset:{}", e.uid))),
                    s(pset_name)
                ));
                format!("(#{ps})")
            }
            _ => "$".into(),
        };
        let t = w.add(format!(
            "{class}({},$,{},$,$,{psets},$,$,$,{predefined})",
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
        let status = export_phase.map(|p| doc.phase_status(e, p));
        if status.is_some_and(|st| !st.is_shown()) {
            continue;
        }
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
        let entity_class = entity.split('(').next().unwrap_or_default().to_string();
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
        if let Some(st) = status {
            status_pset(&mut w, e, id, &entity_class, st);
        }
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

/// `Pset_<Class>Common.Status` (IFC4 `PEnum_ElementStatus`) of an element
/// in the export phase. Classes whose common property set has no `Status`
/// (furniture) get none.
fn status_pset(
    w: &mut Writer,
    e: &Element,
    id: usize,
    class: &str,
    status: ferro_core::model::PhaseStatus,
) {
    let pset_name = match class {
        "IFCWALL" => "Pset_WallCommon",
        "IFCSLAB" => "Pset_SlabCommon",
        "IFCDOOR" => "Pset_DoorCommon",
        "IFCWINDOW" => "Pset_WindowCommon",
        "IFCROOF" => "Pset_RoofCommon",
        "IFCCOLUMN" => "Pset_ColumnCommon",
        "IFCCOVERING" => "Pset_CoveringCommon",
        "IFCBUILDINGELEMENTPROXY" => "Pset_BuildingElementProxyCommon",
        _ => return,
    };
    let Some(value) = status.ifc_status() else {
        return;
    };
    let p = w.add(format!(
        "IFCPROPERTYENUMERATEDVALUE('Status',$,(IFCLABEL({})),$)",
        s(value)
    ));
    let ps = w.add(format!(
        "IFCPROPERTYSET({},$,{},$,(#{p}))",
        s(&derived(&format!("statuspset:{}", e.uid))),
        s(pset_name)
    ));
    w.add(format!(
        "IFCRELDEFINESBYPROPERTIES({},$,$,$,(#{id}),#{ps})",
        s(&derived(&format!("relstatus:{}", e.uid)))
    ));
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
    fn views_are_not_exported() {
        let plain = export_ifc(house().doc()).unwrap();
        let mut s = house();
        s.execute(
            AnyCommand::from_tool(
                "create_view",
                &json!({"name": "ViewOnly", "view_type": "view_3d"}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let with_view = export_ifc(s.doc()).unwrap();
        assert!(!with_view.contains("ViewOnly"));
        assert_eq!(with_view, plain, "a view changes nothing in IFC");
    }

    #[test]
    fn renovation_status_is_written_for_the_export_phase() {
        let plain = export_ifc(house().doc()).unwrap();
        assert!(!plain.contains("'Status'"), "no phases, no status");
        let mut s = house();
        for (els, st) in [
            (json!([5]), "demolish"),
            (json!([6]), "new"),
            (json!([12]), "existing"),
        ] {
            s.execute(
                AnyCommand::from_tool(
                    "set_renovation_status",
                    &json!({"elements": els, "status": st}),
                )
                .unwrap(),
                &ExecContext::user(),
            )
            .unwrap();
        }
        let doc = s.doc();
        let ifc = export_ifc(doc).unwrap();
        for v in ["DEMOLISH", "NEW", "EXISTING"] {
            assert!(
                ifc.contains(&format!(
                    "IFCPROPERTYENUMERATEDVALUE('Status',$,(IFCLABEL('{v}')),$)"
                )),
                "{v}"
            );
        }
        assert!(ifc.contains("'Pset_WallCommon'"));
        assert!(ifc.contains("'Pset_BuildingElementProxyCommon'"));
        // In the Existing phase the new wall is not built yet: left out.
        let phases: Vec<ElementId> = doc.phases().iter().map(|p| p.id).collect();
        let new_wall = ifc_guid(doc.get(ElementId(6)).unwrap().uid.0);
        assert!(ifc.contains(&new_wall));
        let early = export_ifc_phase(doc, Some(phases[0])).unwrap();
        assert!(!early.contains(&new_wall));
        assert!(!early.contains("'DEMOLISH'"));
        assert_eq!(export_ifc_phase(doc, Some(phases[1])).unwrap(), ifc);
        // Deterministic.
        assert_eq!(export_ifc(doc).unwrap(), ifc);
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
    fn compound_angles_carry_the_sign_on_every_part() {
        assert_eq!(compound_angle(16.05), "(16,3,0,0)");
        assert_eq!(compound_angle(108.2), "(108,12,0,0)");
        assert_eq!(compound_angle(-33.87), "(-33,-52,-12,0)");
        assert_eq!(compound_angle(-0.5), "(0,-30,0,0)");
        assert_eq!(compound_angle(0.0), "(0,0,0,0)");
        assert_eq!(compound_angle(-0.0), "(0,0,0,0)");
        // Sub-microsecond noise rounds away; one millionth survives.
        assert_eq!(compound_angle(10.0 + 1e-12), "(10,0,0,0)");
        assert_eq!(compound_angle(1.0 / 3600e6), "(0,0,0,1)");
        assert_eq!(compound_angle(f64::NAN), "$");
    }

    #[test]
    fn site_location_fills_ifcsite() {
        let mut s = house();
        let site_line = |ifc: &str| {
            ifc.lines()
                .find(|l| l.contains("=IFCSITE("))
                .unwrap_or("")
                .to_string()
        };
        let none = export_ifc(s.doc()).unwrap();
        assert!(site_line(&none).ends_with(",$,$,.ELEMENT.,$,$,$,$,$);"));
        for (lat, lon, want) in [
            (
                16.05,
                108.2,
                "'Ha Noi',.ELEMENT.,(16,3,0,0),(108,12,0,0),5000.,$,$);",
            ),
            (
                -33.87,
                151.21,
                "'Ha Noi',.ELEMENT.,(-33,-52,-12,0),(151,12,36,0),5000.,$,$);",
            ),
        ] {
            let cmd = AnyCommand::from_tool(
                "set_site_location",
                // Core commands take radians and mm (degrees are an MCP edge).
                &json!({"latitude": f64::to_radians(lat), "longitude": f64::to_radians(lon),
                    "elevation": 5000.0, "place": "Ha Noi"}),
            )
            .unwrap();
            s.execute(cmd, &ExecContext::user()).unwrap();
            let ifc = export_ifc(s.doc()).unwrap();
            assert!(site_line(&ifc).ends_with(want), "{}", site_line(&ifc));
            assert_eq!(ifc, export_ifc(s.doc()).unwrap(), "deterministic");
        }
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

    #[test]
    fn material_data_becomes_property_sets() {
        let s = house();
        let plain = export_ifc(s.doc()).unwrap();
        assert!(
            !plain.contains("IFCMATERIALPROPERTIES"),
            "no data, no psets"
        );
        assert!(!plain.contains("IFCDERIVEDUNIT("));

        let mut s = house();
        s.execute(
            AnyCommand::from_tool(
                "set_material",
                &json!({"material": 3,
                    "thermal": {"conductivity_w_mk": 0.77, "specific_heat_j_kgk": 840},
                    "physical": {"density_kg_m3": 1800, "youngs_modulus_pa": 5e9, "poisson_ratio": 0.2},
                    "environmental": {"declared_unit": "kg", "gwp_a1_a3": 0.24, "epd_source": "EPD-1"},
                    "cost": {"unit_price": 1500000, "currency": "VND", "unit": "m3"}}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let ifc = export_ifc(s.doc()).unwrap();
        for needle in [
            "IFCMATERIALPROPERTIES('Pset_MaterialCommon'",
            "IFCMATERIALPROPERTIES('Pset_MaterialThermal'",
            "IFCMATERIALPROPERTIES('Pset_MaterialMechanical'",
            "IFCMATERIALPROPERTIES('Pset_FerroMaterial'",
            "'MassDensity',$,IFCMASSDENSITYMEASURE(1800.)",
            "'ThermalConductivity',$,IFCTHERMALCONDUCTIVITYMEASURE(0.77)",
            "'YoungModulus',$,IFCMODULUSOFELASTICITYMEASURE(5000000000.)",
            "'GWP_A1_A3',$,IFCREAL(0.24)",
            "'Currency',$,IFCLABEL('VND')",
            ".MASSDENSITYUNIT.",
            ".THERMALCONDUCTANCEUNIT.",
        ] {
            assert!(ifc.contains(needle), "missing {needle}");
        }
        // The finish layers of the wall type have no material, so no
        // U-value is claimed for it.
        assert!(!ifc.contains("ThermalTransmittance"));
        let n = ifc.matches("\n#").count();
        for cap in ifc.split('#').skip(1) {
            let num: String = cap.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(i) = num.parse::<usize>() {
                assert!(i >= 1 && i <= n, "dangling #{i}");
            }
        }
    }

    #[test]
    fn complete_wall_type_gets_thermal_transmittance() {
        let mut s = Session::new(Document::new("U"), Box::new(SequentialIds::new(1)));
        for (name, args) in [
            (
                "create_material",
                json!({"name": "Brick", "color": "#b5533c", "thermal": {"conductivity_w_mk": 0.77}}),
            ),
            (
                "create_wall_type",
                json!({"name": "Brick 200", "thickness": 200, "material": 1}),
            ),
        ] {
            s.execute(
                AnyCommand::from_tool(name, &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap();
        }
        let ifc = export_ifc(s.doc()).unwrap();
        // U = 1 / (0.13 + 0.2/0.77 + 0.04) = 2.3172...
        let at = ifc
            .find("IFCTHERMALTRANSMITTANCEMEASURE(")
            .expect("thermal transmittance");
        let rest = &ifc[at + "IFCTHERMALTRANSMITTANCEMEASURE(".len()..];
        let v: f64 = rest[..rest.find(')').unwrap()].parse().unwrap();
        assert!((v - 1.0 / (0.13 + 0.2 / 0.77 + 0.04)).abs() < 1e-9);
        assert!(ifc.contains("'Pset_WallCommon'"));
        assert!(ifc.contains(".THERMALTRANSMITTANCEUNIT."));
    }
}
