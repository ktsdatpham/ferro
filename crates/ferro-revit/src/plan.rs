//! Ferro → Revit: the operations that make a Revit model match a document.

use crate::{ft, signature};
use ferro_core::algo::body_mesh;
use ferro_core::geom::Curve;
use ferro_core::model::bim::{
    CompoundStructure, LayerFunction, LocationLine, OpeningKind, WallTop,
};
use ferro_core::model::generate;
use ferro_core::model::material::{PhysicalAsset, ThermalAsset};
use ferro_core::model::{Document, Element, ElementBody};
use ferro_core::Point2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Uid of the level created in Revit for elements that have no level.
pub const DEFAULT_LEVEL_UID: &str = "ferro:level:0";

/// Uid the add-in tags Revit's `SiteLocation` with (one per project). It is
/// never deleted: clearing the location in Ferro leaves Revit's as it was.
pub const SITE_LOCATION_UID: &str = "ferro:site_location";

/// What the Revit model already holds: one record per element tagged with a
/// Ferro uid (the add-in scans the model to build this).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RevitState {
    #[serde(default)]
    pub elements: Vec<RevitRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitRecord {
    pub ferro_uid: String,
    /// Revit `Element.UniqueId`.
    pub unique_id: String,
    /// Signature of the operation that last wrote the element.
    #[serde(default)]
    pub signature: String,
}

/// A compound-structure layer in Revit terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpLayer {
    /// `MaterialFunctionAssignment` name: Structure, Substrate, Insulation,
    /// Finish1 or Membrane.
    pub function: String,
    /// Width in feet (0 for membranes, as Revit requires).
    pub width: f64,
    #[serde(default)]
    pub material_uid: Option<String>,
}

/// A location curve in feet: a line (`mid` absent) or an arc through `mid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpCurve {
    pub start: [f64; 3],
    pub end: [f64; 3],
    #[serde(default)]
    pub mid: Option<[f64; 3]>,
}

/// One operation for the add-in. Every upsert carries the Ferro `uid` (stored
/// on the Revit element) and a `signature`; lengths are in feet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RevitOp {
    /// Delete the Revit element (skip silently if it is already gone).
    Delete {
        ferro_uid: String,
        unique_id: String,
    },
    /// Create / update a material: colour, and when the Ferro material has
    /// them, a Thermal and a Physical (structural) asset. Asset values are
    /// SI as in `ferro_core::model::material` (not feet); the add-in converts
    /// them with `UnitUtils`. Absent assets are omitted from the JSON, so
    /// materials without assets keep their M3 signature.
    Material {
        uid: String,
        signature: String,
        name: String,
        /// sRGB 0–255.
        color: [u8; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thermal: Option<ThermalAsset>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        physical: Option<PhysicalAsset>,
    },
    Level {
        uid: String,
        signature: String,
        name: String,
        elevation: f64,
    },
    /// Duplicate a basic wall type and set its compound structure.
    WallType {
        uid: String,
        signature: String,
        name: String,
        layers: Vec<OpLayer>,
        /// Layers before / after the core (`SetNumberOfShellLayers`).
        exterior_shells: usize,
        interior_shells: usize,
    },
    FloorType {
        uid: String,
        signature: String,
        name: String,
        layers: Vec<OpLayer>,
        exterior_shells: usize,
        interior_shells: usize,
    },
    /// `Wall.Create(curve, type, level, height, base_offset, flip: false)`;
    /// then set the top constraint and `WALL_KEY_REF_PARAM`.
    Wall {
        uid: String,
        signature: String,
        name: Option<String>,
        type_uid: String,
        level_uid: String,
        curve: OpCurve,
        base_offset: f64,
        /// Unconnected height (0 when `top_level_uid` is set).
        height: f64,
        #[serde(default)]
        top_level_uid: Option<String>,
        top_offset: f64,
        /// `WallLocationLine` value: 0 centerline, 1 core centerline,
        /// 2 finish exterior, 3 finish interior.
        location_line: i32,
        /// Phase names for `PHASE_CREATED` / `PHASE_DEMOLISHED` (looked up
        /// by name in Revit). Omitted when the element has no phases, so
        /// unphased elements keep their signature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_created: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_demolished: Option<String>,
    },
    /// `Floor.Create(loops, type, level)`, then `FLOOR_HEIGHTABOVELEVEL_PARAM`.
    /// First loop is the outline (counter-clockwise), then holes (clockwise),
    /// at the level elevation.
    Floor {
        uid: String,
        signature: String,
        name: Option<String>,
        type_uid: String,
        level_uid: String,
        loops: Vec<Vec<[f64; 3]>>,
        offset: f64,
        /// Phase names for `PHASE_CREATED` / `PHASE_DEMOLISHED` (looked up
        /// by name in Revit). Omitted when the element has no phases, so
        /// unphased elements keep their signature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_created: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_demolished: Option<String>,
    },
    /// A door or window family instance hosted by a wall, placed at `point`
    /// (on the wall's location line, at the wall base). Size via the type's
    /// width / height; `sill` is above the level.
    Opening {
        uid: String,
        signature: String,
        name: Option<String>,
        /// `door` or `window`.
        kind: String,
        /// `OST_Doors` or `OST_Windows`.
        category: String,
        host_uid: String,
        level_uid: String,
        point: [f64; 3],
        width: f64,
        height: f64,
        sill: f64,
        /// Phase names for `PHASE_CREATED` / `PHASE_DEMOLISHED` (looked up
        /// by name in Revit). Omitted when the element has no phases, so
        /// unphased elements keep their signature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_created: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_demolished: Option<String>,
    },
    /// A phase (ADR 0006). The Revit API cannot create phases: the add-in
    /// finds the phase by `name`, else renames the phase at position
    /// `order` (0 = first), else reports a problem. Phases are not tagged.
    Phase {
        uid: String,
        signature: String,
        name: String,
        order: u32,
    },
    /// The project's place on Earth: set `Document.SiteLocation`
    /// (`Latitude`, `Longitude`, `TimeZone`, `PlaceName`). Only planned when
    /// the Ferro project has a location (M5-24.3); `uid` is
    /// [`SITE_LOCATION_UID`].
    SiteLocation {
        uid: String,
        signature: String,
        /// `PlaceName`; empty keeps Revit's.
        place: String,
        /// Radians, north positive (as Revit stores it).
        latitude: f64,
        /// Radians, east positive.
        longitude: f64,
        /// Elevation of the project origin above sea level, feet. Revit's
        /// `SiteLocation.Elevation` is read-only, so the add-in reports it
        /// but does not move the survey point.
        elevation: f64,
        /// `TimeZone`: hours from UTC (Vietnam 7, India 5.5).
        time_zone: f64,
    },
    /// Free-form geometry as a `DirectShape` mesh.
    DirectShape {
        uid: String,
        signature: String,
        name: String,
        /// `BuiltInCategory` name, e.g. `OST_Mass`.
        category: String,
        vertices: Vec<[f64; 3]>,
        triangles: Vec<[u32; 3]>,
        /// Phase names for `PHASE_CREATED` / `PHASE_DEMOLISHED` (looked up
        /// by name in Revit). Omitted when the element has no phases, so
        /// unphased elements keep their signature.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_created: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase_demolished: Option<String>,
    },
}

impl RevitOp {
    pub fn uid(&self) -> &str {
        match self {
            RevitOp::Delete { ferro_uid, .. } => ferro_uid,
            RevitOp::Material { uid, .. }
            | RevitOp::Level { uid, .. }
            | RevitOp::WallType { uid, .. }
            | RevitOp::FloorType { uid, .. }
            | RevitOp::Wall { uid, .. }
            | RevitOp::Floor { uid, .. }
            | RevitOp::Opening { uid, .. }
            | RevitOp::SiteLocation { uid, .. }
            | RevitOp::DirectShape { uid, .. }
            | RevitOp::Phase { uid, .. } => uid,
        }
    }

    fn signature_mut(&mut self) -> Option<&mut String> {
        match self {
            RevitOp::Delete { .. } => None,
            RevitOp::Material { signature, .. }
            | RevitOp::Level { signature, .. }
            | RevitOp::WallType { signature, .. }
            | RevitOp::FloorType { signature, .. }
            | RevitOp::Wall { signature, .. }
            | RevitOp::Floor { signature, .. }
            | RevitOp::Opening { signature, .. }
            | RevitOp::SiteLocation { signature, .. }
            | RevitOp::DirectShape { signature, .. }
            | RevitOp::Phase { signature, .. } => Some(signature),
        }
    }

    /// Fill in the signature from the operation's content.
    fn signed(mut self) -> Self {
        if let Some(s) = self.signature_mut() {
            s.clear();
        }
        let text = serde_json::to_string(&self).unwrap_or_default();
        if let Some(s) = self.signature_mut() {
            *s = signature(&text);
        }
        self
    }

    fn signature(&self) -> &str {
        match self {
            RevitOp::Delete { .. } => "",
            RevitOp::Material { signature, .. }
            | RevitOp::Level { signature, .. }
            | RevitOp::WallType { signature, .. }
            | RevitOp::FloorType { signature, .. }
            | RevitOp::Wall { signature, .. }
            | RevitOp::Floor { signature, .. }
            | RevitOp::Opening { signature, .. }
            | RevitOp::SiteLocation { signature, .. }
            | RevitOp::DirectShape { signature, .. }
            | RevitOp::Phase { signature, .. } => signature,
        }
    }
}

/// Rounded feet (1e-9 ft ≈ 0.3 nm) so signatures ignore float noise.
fn r(x: f64) -> f64 {
    (x * 1e9).round() / 1e9
}

fn p3(x: f64, y: f64, z: f64) -> [f64; 3] {
    [r(ft(x)), r(ft(y)), r(ft(z))]
}

fn layers(
    st: &CompoundStructure,
    uid_of: &dyn Fn(ferro_core::ElementId) -> Option<String>,
) -> (Vec<OpLayer>, usize, usize) {
    let out: Vec<OpLayer> = st
        .layers
        .iter()
        .map(|l| {
            let (function, width) = match l.function {
                LayerFunction::Structure => ("Structure", l.thickness.mm()),
                LayerFunction::Substrate => ("Substrate", l.thickness.mm()),
                LayerFunction::Insulation => ("Insulation", l.thickness.mm()),
                LayerFunction::Finish => ("Finish1", l.thickness.mm()),
                // Revit membranes have no width; a thick membrane becomes a
                // substrate so the total thickness is kept.
                LayerFunction::Membrane if l.thickness.mm() > 0.0 => {
                    ("Substrate", l.thickness.mm())
                }
                LayerFunction::Membrane => ("Membrane", 0.0),
            };
            OpLayer {
                function: function.into(),
                width: r(ft(width)),
                material_uid: l.material.and_then(uid_of),
            }
        })
        .collect();
    let first = st
        .layers
        .iter()
        .position(|l| l.function == LayerFunction::Structure);
    let last = st
        .layers
        .iter()
        .rposition(|l| l.function == LayerFunction::Structure);
    let (ext, int) = match (first, last) {
        (Some(a), Some(b)) => (a, st.layers.len() - 1 - b),
        _ => (0, 0),
    };
    (out, ext, int)
}

fn location_line(l: LocationLine) -> i32 {
    match l {
        LocationLine::WallCenterline => 0,
        LocationLine::CoreCenterline => 1,
        LocationLine::FinishExterior => 2,
        LocationLine::FinishInterior => 3,
    }
}

fn signed_area(pts: &[Point2]) -> f64 {
    let mut a = 0.0;
    for (i, p) in pts.iter().enumerate() {
        let q = pts[(i + 1) % pts.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

/// The `site_location` operation for the project's location, if it has one.
/// Angles stay in radians, elevation goes to feet, the UTC offset to hours.
fn site_location_op(doc: &Document) -> Option<RevitOp> {
    let l = doc.project().settings.location.as_ref()?;
    // Rounded so signatures ignore float noise (1e-12 rad ≈ 6 µm on Earth).
    let rad = |a: f64| (a * 1e12).round() / 1e12;
    Some(RevitOp::SiteLocation {
        uid: SITE_LOCATION_UID.into(),
        signature: String::new(),
        place: l.place.trim().to_string(),
        latitude: rad(l.latitude.rad()),
        longitude: rad(l.longitude.rad()),
        elevation: r(ft(l.elevation.mm())),
        time_zone: l.utc_offset_minutes as f64 / 60.0,
    })
}

/// Operations that bring a Revit model holding `state` up to date with
/// `doc`. Order: deletes, site location, materials, levels and phases,
/// types, walls, floors, openings, direct shapes. Elements whose signature
/// is unchanged are skipped.
pub fn plan(doc: &Document, state: &RevitState) -> Vec<RevitOp> {
    let uid_of = |id: ferro_core::ElementId| doc.get(id).map(|e| e.uid.to_string());
    let needs_default_level = doc.elements().any(|e| {
        e.level_id.is_none()
            && matches!(
                e.body,
                ElementBody::Wall(_) | ElementBody::Floor(_) | ElementBody::Opening(_)
            )
    });
    let level_uid = |e: &Element| {
        e.level_id
            .and_then(uid_of)
            .unwrap_or_else(|| DEFAULT_LEVEL_UID.to_string())
    };
    let level_z = |e: &Element| generate::level_elevation(doc, e.level_id);
    let phase_name = |id: Option<ferro_core::ElementId>| {
        id.and_then(|i| doc.get(i))
            .filter(|p| p.as_phase().is_some())
            .map(|p| p.label())
    };

    let mut groups: [Vec<RevitOp>; 7] = Default::default();
    groups[0].extend(site_location_op(doc));
    if needs_default_level {
        groups[1].push(RevitOp::Level {
            uid: DEFAULT_LEVEL_UID.into(),
            signature: String::new(),
            name: "Ferro Level 0".into(),
            elevation: 0.0,
        });
    }
    for e in doc.elements() {
        let uid = e.uid.to_string();
        let name = e.name.clone();
        let (group, op) = match &e.body {
            // Views are not synced to Revit yet (M5-22): Revit views need
            // view family types and sheets; a later task maps them.
            ElementBody::View(_) => continue,
            ElementBody::Material(m) => (
                0,
                RevitOp::Material {
                    uid,
                    signature: String::new(),
                    name: e.label(),
                    color: m.color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8),
                    thermal: m.thermal.clone(),
                    physical: m.physical.clone(),
                },
            ),
            ElementBody::Level(l) => (
                1,
                RevitOp::Level {
                    uid,
                    signature: String::new(),
                    name: e.label(),
                    elevation: r(ft(l.elevation.mm())),
                },
            ),
            ElementBody::WallType(t) => {
                let (layers, exterior_shells, interior_shells) = layers(&t.structure, &uid_of);
                (
                    2,
                    RevitOp::WallType {
                        uid,
                        signature: String::new(),
                        name: e.label(),
                        layers,
                        exterior_shells,
                        interior_shells,
                    },
                )
            }
            ElementBody::FloorType(t) => {
                let (layers, exterior_shells, interior_shells) = layers(&t.structure, &uid_of);
                (
                    2,
                    RevitOp::FloorType {
                        uid,
                        signature: String::new(),
                        name: e.label(),
                        layers,
                        exterior_shells,
                        interior_shells,
                    },
                )
            }
            ElementBody::Wall(w) => {
                let Some(type_uid) = e.type_id.and_then(uid_of) else {
                    continue;
                };
                let lvl = level_z(e);
                let curve = match w.curve {
                    Curve::Line(l) => OpCurve {
                        start: p3(l.start().x, l.start().y, lvl),
                        end: p3(l.end().x, l.end().y, lvl),
                        mid: None,
                    },
                    Curve::Arc(a) => {
                        let m = a.point_at(0.5);
                        OpCurve {
                            start: p3(a.start().x, a.start().y, lvl),
                            end: p3(a.end().x, a.end().y, lvl),
                            mid: Some(p3(m.x, m.y, lvl)),
                        }
                    }
                };
                // A wall attached to a level follows it in Revit, so its
                // height is not part of the operation (no needless updates).
                let (height, top_level_uid, top_offset) = match &w.top {
                    WallTop::Level { level, offset } => (0.0, uid_of(*level), offset.mm()),
                    WallTop::Unconnected { height } => (height.mm(), None, 0.0),
                };
                (
                    3,
                    RevitOp::Wall {
                        uid,
                        signature: String::new(),
                        name,
                        type_uid,
                        level_uid: level_uid(e),
                        curve,
                        base_offset: r(ft(w.base_offset.mm())),
                        height: r(ft(height)),
                        top_level_uid,
                        top_offset: r(ft(top_offset)),
                        location_line: location_line(w.location_line),
                        phase_created: phase_name(e.phase_created),
                        phase_demolished: phase_name(e.phase_demolished),
                    },
                )
            }
            ElementBody::Floor(f) => {
                let Some(type_uid) = e.type_id.and_then(uid_of) else {
                    continue;
                };
                let z = level_z(e);
                let ring = |pts: &[Point2], ccw: bool| -> Vec<[f64; 3]> {
                    let mut v: Vec<[f64; 3]> = pts.iter().map(|p| p3(p.x, p.y, z)).collect();
                    if (signed_area(pts) > 0.0) != ccw {
                        v.reverse();
                    }
                    v
                };
                let mut loops = vec![ring(&f.outer, true)];
                loops.extend(f.holes.iter().map(|h| ring(h, false)));
                (
                    4,
                    RevitOp::Floor {
                        uid,
                        signature: String::new(),
                        name,
                        type_uid,
                        level_uid: level_uid(e),
                        loops,
                        offset: r(ft(f.offset.mm())),
                        phase_created: phase_name(e.phase_created),
                        phase_demolished: phase_name(e.phase_demolished),
                    },
                )
            }
            ElementBody::Opening(o) => {
                let Some(host) = e.host_id.and_then(|h| doc.get(h)) else {
                    continue;
                };
                let Some(lay) = host
                    .as_wall()
                    .and_then(|w| generate::wall_layout(doc, host, w).ok())
                else {
                    continue;
                };
                let Some((s, d, _)) = lay.frame() else {
                    continue;
                };
                let c = s + d * o.position.mm();
                let host_level = generate::level_elevation(doc, host.level_id);
                let (kind, category) = match o.kind {
                    OpeningKind::Door => ("door", "OST_Doors"),
                    OpeningKind::Window => ("window", "OST_Windows"),
                };
                (
                    5,
                    RevitOp::Opening {
                        uid,
                        signature: String::new(),
                        name,
                        kind: kind.into(),
                        category: category.into(),
                        host_uid: host.uid.to_string(),
                        level_uid: level_uid(host),
                        point: p3(c.x, c.y, lay.z0),
                        width: r(ft(o.width.mm())),
                        height: r(ft(o.height.mm())),
                        sill: r(ft(lay.z0 - host_level + o.sill.mm())),
                        phase_created: phase_name(e.phase_created),
                        phase_demolished: phase_name(e.phase_demolished),
                    },
                )
            }
            // Phases go first, with the levels (elements refer to them).
            ElementBody::Phase(p) => (
                1,
                RevitOp::Phase {
                    uid,
                    signature: String::new(),
                    name: e.label(),
                    order: doc.phase_index(e.id).map(|i| i as u32).unwrap_or(p.order),
                },
            ),
            ElementBody::Form(f) => {
                let Ok(m) = body_mesh(&f.body) else { continue };
                if m.indices.is_empty() {
                    continue;
                }
                (
                    6,
                    RevitOp::DirectShape {
                        uid,
                        signature: String::new(),
                        name: e.label(),
                        category: e.category.revit_builtin().into(),
                        vertices: m.positions.iter().map(|p| p3(p.x, p.y, p.z)).collect(),
                        triangles: m.indices.clone(),
                        phase_created: phase_name(e.phase_created),
                        phase_demolished: phase_name(e.phase_demolished),
                    },
                )
            }
        };
        groups[group].push(op);
    }

    let records: HashMap<&str, &RevitRecord> = state
        .elements
        .iter()
        .map(|r| (r.ferro_uid.as_str(), r))
        .collect();
    let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut upserts = Vec::new();
    for g in groups {
        for op in g {
            let op = op.signed();
            present.insert(op.uid().to_string());
            let unchanged = records
                .get(op.uid())
                .is_some_and(|rec| rec.signature == op.signature());
            if !unchanged {
                upserts.push(op);
            }
        }
    }
    for rec in &state.elements {
        if !present.contains(&rec.ferro_uid) && rec.ferro_uid != SITE_LOCATION_UID {
            out.push(RevitOp::Delete {
                ferro_uid: rec.ferro_uid.clone(),
                unique_id: rec.unique_id.clone(),
            });
        }
    }
    out.extend(upserts);
    out
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    /// A 20 × 10 ft room on L1 with walls up to L2, a door, a floor and a box.
    pub(crate) fn house() -> Session {
        let mut s = Session::new(Document::new("House"), Box::new(SequentialIds::new(1)));
        for (name, args) in [
            ("create_level", json!({"name": "L1", "elevation": 0})),
            ("create_level", json!({"name": "L2", "elevation": 3048})),
            (
                "create_material",
                json!({"name": "Brick", "color": "#ff0000"}),
            ),
            (
                "create_wall_type",
                json!({"name": "Brick 230", "layers": [
                {"function": "finish", "thickness": 15},
                {"function": "structure", "thickness": 200, "material": 3},
                {"function": "membrane", "thickness": 0},
                {"function": "finish", "thickness": 15}]}),
            ),
            (
                "create_walls",
                json!({"points": [{"x":0,"y":0,"z":0},{"x":0,"y":3048,"z":0},{"x":6096,"y":3048,"z":0},{"x":6096,"y":0,"z":0}],
                "closed": true, "level": 1, "top_level": 2, "wall_type": 4}),
            ),
            (
                "create_opening",
                json!({"wall": 5, "kind": "door", "position": 1524}),
            ),
            (
                "create_floor",
                json!({"points": [{"x":0,"y":0},{"x":6096,"y":0},{"x":6096,"y":3048},{"x":0,"y":3048}], "level": 1}),
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
}

#[cfg(test)]
mod tests {
    use super::tests_support::house;
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    #[test]
    fn phases_and_element_phase_names_are_planned() {
        let mut s = house();
        // Unphased elements carry no phase fields (signatures unchanged).
        let before = plan(s.doc(), &RevitState::default());
        assert!(!serde_json::to_string(&before).unwrap().contains("phase"));
        let wall = s
            .doc()
            .elements()
            .find(|e| e.as_wall().is_some())
            .map(|e| e.id)
            .unwrap();
        s.execute(
            AnyCommand::from_tool(
                "set_renovation_status",
                &json!({"elements": [wall], "status": "demolish"}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let ops = plan(s.doc(), &RevitState::default());
        let phases: Vec<(String, u32)> = ops
            .iter()
            .filter_map(|o| match o {
                RevitOp::Phase { name, order, .. } => Some((name.clone(), *order)),
                _ => None,
            })
            .collect();
        assert_eq!(
            phases,
            [
                ("Existing".to_string(), 0),
                ("New Construction".to_string(), 1)
            ]
        );
        let uid = s.doc().get(wall).unwrap().uid.to_string();
        let op = ops.iter().find(|o| o.uid() == uid).unwrap();
        let RevitOp::Wall {
            phase_created,
            phase_demolished,
            ..
        } = op
        else {
            panic!("{op:?}")
        };
        assert_eq!(phase_created.as_deref(), Some("Existing"));
        assert_eq!(phase_demolished.as_deref(), Some("New Construction"));
        // Phases come before walls.
        let first_wall = ops.iter().position(|o| matches!(o, RevitOp::Wall { .. }));
        let last_phase = ops.iter().rposition(|o| matches!(o, RevitOp::Phase { .. }));
        assert!(last_phase < first_wall);
    }

    #[test]
    fn full_plan_is_ordered_and_in_feet() {
        let s = house();
        let ops = plan(s.doc(), &RevitState::default());
        let kinds: Vec<&str> = ops
            .iter()
            .map(|o| match o {
                RevitOp::Material { .. } => "material",
                RevitOp::Level { .. } => "level",
                RevitOp::WallType { .. } => "wall_type",
                RevitOp::FloorType { .. } => "floor_type",
                RevitOp::Wall { .. } => "wall",
                RevitOp::Floor { .. } => "floor",
                RevitOp::Opening { .. } => "opening",
                RevitOp::DirectShape { .. } => "direct_shape",
                RevitOp::SiteLocation { .. } => "site_location",
                RevitOp::Phase { .. } => "phase",
                RevitOp::Delete { .. } => "delete",
            })
            .collect();
        let mut sorted = kinds.clone();
        let rank = |k: &str| {
            [
                "material",
                "level",
                "wall_type",
                "floor_type",
                "wall",
                "floor",
                "opening",
                "direct_shape",
            ]
            .iter()
            .position(|x| *x == k)
            .unwrap()
        };
        sorted.sort_by_key(|k| rank(k));
        assert_eq!(kinds, sorted);
        assert_eq!(kinds.iter().filter(|k| **k == "wall").count(), 4);
        // The box has no level, but walls do: no default level needed.
        assert_eq!(kinds.iter().filter(|k| **k == "level").count(), 2);

        let RevitOp::Level { elevation, .. } = &ops[2] else {
            panic!("{:?}", ops[2])
        };
        assert!((elevation - 10.0).abs() < 1e-9, "3048 mm = 10 ft");
        let wall = ops
            .iter()
            .find(|o| matches!(o, RevitOp::Wall { .. }))
            .unwrap();
        let RevitOp::Wall {
            curve,
            height,
            top_level_uid,
            location_line,
            ..
        } = wall
        else {
            unreachable!()
        };
        assert_eq!(curve.end, [0.0, 10.0, 0.0]);
        assert_eq!(*height, 0.0, "attached to L2");
        assert!(top_level_uid.is_some());
        assert_eq!(*location_line, 0);
        let wt = ops
            .iter()
            .find(|o| matches!(o, RevitOp::WallType { .. }))
            .unwrap();
        let RevitOp::WallType {
            layers,
            exterior_shells,
            interior_shells,
            ..
        } = wt
        else {
            unreachable!()
        };
        assert_eq!(layers[0].function, "Finish1");
        assert_eq!(layers[2].function, "Membrane");
        assert_eq!(layers[2].width, 0.0);
        assert!(layers[1].material_uid.is_some());
        assert_eq!((*exterior_shells, *interior_shells), (1, 2));
        let op = ops
            .iter()
            .find(|o| matches!(o, RevitOp::Opening { .. }))
            .unwrap();
        let RevitOp::Opening {
            point,
            category,
            sill,
            ..
        } = op
        else {
            unreachable!()
        };
        assert_eq!(category, "OST_Doors");
        assert_eq!(*point, [0.0, 5.0, 0.0]);
        assert_eq!(*sill, 0.0);
        let fl = ops
            .iter()
            .find(|o| matches!(o, RevitOp::Floor { .. }))
            .unwrap();
        let RevitOp::Floor { loops, .. } = fl else {
            unreachable!()
        };
        assert_eq!(loops[0].len(), 4);
        let ds = ops
            .iter()
            .find(|o| matches!(o, RevitOp::DirectShape { .. }))
            .unwrap();
        let RevitOp::DirectShape {
            category,
            triangles,
            ..
        } = ds
        else {
            unreachable!()
        };
        assert_eq!(category, "OST_Mass");
        assert_eq!(triangles.len(), 12);
    }

    #[test]
    fn unchanged_elements_are_skipped_and_removed_ones_deleted() {
        let mut s = house();
        let first = plan(s.doc(), &RevitState::default());
        let mut state = RevitState {
            elements: first
                .iter()
                .enumerate()
                .map(|(i, o)| RevitRecord {
                    ferro_uid: o.uid().to_string(),
                    unique_id: format!("rvt-{i}"),
                    signature: o.signature().to_string(),
                })
                .collect(),
        };
        assert!(plan(s.doc(), &state).is_empty(), "nothing changed");

        // Raise L2: only the level changes in Revit (walls follow it there).
        s.execute(
            AnyCommand::from_tool(
                "set_level_elevation",
                &json!({"level": 2, "elevation": 3500}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let ops = plan(s.doc(), &state);
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(matches!(&ops[0], RevitOp::Level { name, .. } if name == "L2"));

        // A record whose element no longer exists is deleted.
        state.elements.push(RevitRecord {
            ferro_uid: "gone".into(),
            unique_id: "rvt-x".into(),
            signature: String::new(),
        });
        let ops = plan(s.doc(), &state);
        assert!(matches!(&ops[0], RevitOp::Delete { unique_id, .. } if unique_id == "rvt-x"));
    }

    #[test]
    fn elements_without_level_get_a_default_level() {
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        s.execute(
            AnyCommand::from_tool(
                "create_wall",
                &json!({"start": {"x":0,"y":0,"z":0}, "end": {"x":1000,"y":0,"z":0}}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let ops = plan(s.doc(), &RevitState::default());
        assert!(ops.iter().any(|o| o.uid() == DEFAULT_LEVEL_UID));
        let RevitOp::Wall { level_uid, .. } = ops
            .iter()
            .find(|o| matches!(o, RevitOp::Wall { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(level_uid, DEFAULT_LEVEL_UID);
    }

    #[test]
    fn site_location_op_follows_the_project_location() {
        let mut s = house();
        let none = plan(s.doc(), &RevitState::default());
        assert!(!none
            .iter()
            .any(|o| matches!(o, RevitOp::SiteLocation { .. })));
        let set = |s: &mut Session, lat: f64| {
            let args = json!({"latitude": f64::to_radians(lat), "longitude": f64::to_radians(108.2),
                "elevation": 3048.0, "utc_offset_minutes": 420, "place": "Da Nang"});
            s.execute(
                AnyCommand::from_tool("set_site_location", &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap();
        };
        set(&mut s, 16.05);
        let ops = plan(s.doc(), &RevitState::default());
        let first = ops.first().cloned();
        let Some(RevitOp::SiteLocation {
            uid,
            signature,
            place,
            latitude,
            longitude,
            elevation,
            time_zone,
        }) = first
        else {
            panic!("site location is planned first: {ops:?}");
        };
        assert_eq!(uid, SITE_LOCATION_UID);
        assert_eq!(place, "Da Nang");
        assert!((latitude - 16.05f64.to_radians()).abs() < 1e-9);
        assert!((longitude - 108.2f64.to_radians()).abs() < 1e-9);
        assert!((elevation - 10.0).abs() < 1e-9, "3048 mm = 10 ft");
        assert!((time_zone - 7.0).abs() < 1e-12);
        let json = serde_json::to_value(&ops[0]).unwrap();
        assert_eq!(json["op"], "site_location");

        // Unchanged → skipped; a new latitude changes the signature.
        let state = RevitState {
            elements: vec![RevitRecord {
                ferro_uid: SITE_LOCATION_UID.into(),
                unique_id: "site".into(),
                signature: signature.clone(),
            }],
        };
        assert!(!plan(s.doc(), &state)
            .iter()
            .any(|o| o.uid() == SITE_LOCATION_UID));
        set(&mut s, -33.87);
        let moved = plan(s.doc(), &state);
        let again = moved.iter().find(|o| o.uid() == SITE_LOCATION_UID);
        assert!(again.is_some_and(|o| o.signature() != signature));

        // Clearing the location never deletes Revit's SiteLocation.
        s.execute(
            AnyCommand::from_tool("set_site_location", &json!({"clear": true})).unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        assert!(!plan(s.doc(), &state)
            .iter()
            .any(|o| o.uid() == SITE_LOCATION_UID));
    }

    #[test]
    fn material_op_carries_thermal_and_physical_assets() {
        let mut s = house();
        let ops = plan(s.doc(), &RevitState::default());
        let mat = |ops: &[RevitOp]| {
            ops.iter()
                .find(|o| matches!(o, RevitOp::Material { .. }))
                .cloned()
                .unwrap()
        };
        let before = mat(&ops);
        // Without assets the JSON is as in M3 (no asset keys).
        let text = serde_json::to_string(&before).unwrap();
        assert!(
            !text.contains("thermal") && !text.contains("physical"),
            "{text}"
        );

        s.execute(
            AnyCommand::from_tool(
                "set_material",
                &json!({"material": 3, "thermal": {"conductivity_w_mk": 0.77},
                    "physical": {"density_kg_m3": 1800}, "cost": {"unit_price": 1, "currency": "VND", "unit": "m3"}}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let after = mat(&plan(s.doc(), &RevitState::default()));
        let RevitOp::Material {
            thermal, physical, ..
        } = &after
        else {
            unreachable!()
        };
        assert!((thermal.as_ref().unwrap().conductivity_w_mk.unwrap() - 0.77).abs() < 1e-12);
        assert!((physical.as_ref().unwrap().density_kg_m3.unwrap() - 1800.0).abs() < 1e-12);
        // New data changes the signature, so Revit updates the material.
        assert_ne!(before.signature(), after.signature());
        let v = serde_json::to_value(&after).unwrap();
        assert!((v["thermal"]["conductivity_w_mk"].as_f64().unwrap() - 0.77).abs() < 1e-12);
    }
}
