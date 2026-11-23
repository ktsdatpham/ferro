//! Milestone 2 commands: materials, system family types, walls, floors and
//! hosted openings (D-41, D-70, D-73). Lengths in mm; each `NAME` is also the
//! MCP tool name.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::geom::{Arc, Curve, Line};
use crate::math::{tol, Length, Point2, Point3, Vec2, Vec3};
use crate::model::bim::{
    CompoundLayer, CompoundStructure, Floor, FloorType, LayerFunction, LocationLine, Material,
    Opening, OpeningKind, Wall, WallTop, WallType,
};
use crate::model::generate::{self, WallLayout};
use crate::model::{Category, Element, ElementBody, ElementId};
use crate::topo::FaceId;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Name of the wall type created on demand when none is given.
pub const DEFAULT_WALL_TYPE: &str = "Generic - 200mm";
/// Name of the floor type created on demand when none is given.
pub const DEFAULT_FLOOR_TYPE: &str = "Generic - 200mm";
/// Default unconnected wall height (mm).
pub const DEFAULT_WALL_HEIGHT: f64 = 3000.0;

fn err(code: &'static str, msg: impl Into<String>) -> CommandError {
    CommandError::new(code, msg)
}

fn flat(p: Point3) -> Point3 {
    Point3::new(p.x, p.y, 0.0)
}

/// Parse `#rrggbb` / `rrggbb` into linear-ish 0..1 sRGB components.
pub fn parse_color(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some([c(0)?, c(2)?, c(4)?])
}

fn check_unique_name(
    tx: &Transaction,
    name: &str,
    same_kind: impl Fn(&Element) -> bool,
) -> Result<(), CommandError> {
    if tx
        .doc()
        .elements()
        .any(|e| same_kind(e) && e.name.as_deref() == Some(name))
    {
        return Err(err(
            "ferro.model.duplicate_name",
            format!("{name:?} already exists"),
        ));
    }
    Ok(())
}

// --------------------------------------------------------------- materials

/// Create a material with a base colour (`"#rrggbb"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateMaterial {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub roughness: Option<f32>,
}

impl Command for CreateMaterial {
    const NAME: &'static str = "create_material";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_unique_name(tx, &self.name, |e| {
            matches!(e.body, ElementBody::Material(_))
        })?;
        let color = parse_color(&self.color).ok_or_else(|| {
            err(
                "ferro.model.invalid_color",
                format!("{:?} is not a #rrggbb colour", self.color),
            )
        })?;
        let roughness = self.roughness.unwrap_or(0.8).clamp(0.0, 1.0);
        let id = tx.create(
            Category::Materials,
            Some(self.name.clone()),
            None,
            ElementBody::Material(Material { color, roughness }),
        )?;
        Ok(json!({ "element": id }))
    }
}

// ------------------------------------------------------------------- types

fn structure(
    tx: &Transaction,
    layers: &Option<Vec<CompoundLayer>>,
    thickness: Option<Length>,
    material: Option<ElementId>,
) -> Result<CompoundStructure, CommandError> {
    let st = match (layers, thickness) {
        (Some(l), _) if !l.is_empty() => CompoundStructure { layers: l.clone() },
        (_, Some(t)) => CompoundStructure {
            layers: vec![CompoundLayer {
                function: LayerFunction::Structure,
                thickness: t,
                material,
            }],
        },
        _ => {
            return Err(err(
                "ferro.model.invalid_structure",
                "give either layers or a thickness",
            ))
        }
    };
    for l in &st.layers {
        if l.thickness.mm() < 0.0 {
            return Err(err(
                "ferro.model.invalid_structure",
                "layer thickness cannot be negative",
            ));
        }
        if let Some(m) = l.material {
            if !matches!(tx.element(m)?.body, ElementBody::Material(_)) {
                return Err(err(
                    "ferro.model.not_a_material",
                    format!("{m} is not a material"),
                )
                .on(m));
            }
        }
    }
    if st.thickness() < tol::SHORT_CURVE_TOL {
        return Err(err(
            "ferro.model.invalid_structure",
            "total thickness must be at least 0.5 mm",
        ));
    }
    Ok(st)
}

/// Create a wall type: one layer of `thickness`, or explicit `layers` listed
/// from the exterior to the interior face.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateWallType {
    pub name: String,
    #[serde(default)]
    pub thickness: Option<Length>,
    #[serde(default)]
    pub material: Option<ElementId>,
    #[serde(default)]
    pub layers: Option<Vec<CompoundLayer>>,
}

impl Command for CreateWallType {
    const NAME: &'static str = "create_wall_type";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_unique_name(tx, &self.name, |e| {
            matches!(e.body, ElementBody::WallType(_))
        })?;
        let st = structure(tx, &self.layers, self.thickness, self.material)?;
        let id = tx.create(
            Category::Walls,
            Some(self.name.clone()),
            None,
            ElementBody::WallType(WallType { structure: st }),
        )?;
        Ok(json!({ "element": id }))
    }
}

/// Create a floor type (layers from top to bottom).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateFloorType {
    pub name: String,
    #[serde(default)]
    pub thickness: Option<Length>,
    #[serde(default)]
    pub material: Option<ElementId>,
    #[serde(default)]
    pub layers: Option<Vec<CompoundLayer>>,
}

impl Command for CreateFloorType {
    const NAME: &'static str = "create_floor_type";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_unique_name(tx, &self.name, |e| {
            matches!(e.body, ElementBody::FloorType(_))
        })?;
        let st = structure(tx, &self.layers, self.thickness, self.material)?;
        let id = tx.create(
            Category::Floors,
            Some(self.name.clone()),
            None,
            ElementBody::FloorType(FloorType { structure: st }),
        )?;
        Ok(json!({ "element": id }))
    }
}

fn find_or_create_type(
    tx: &mut Transaction,
    requested: Option<ElementId>,
    floor: bool,
) -> Result<ElementId, CommandError> {
    let is_kind = |e: &Element| {
        if floor {
            matches!(e.body, ElementBody::FloorType(_))
        } else {
            matches!(e.body, ElementBody::WallType(_))
        }
    };
    if let Some(t) = requested {
        let e = tx.element(t)?;
        if !is_kind(e) {
            let what = if floor { "floor type" } else { "wall type" };
            return Err(err("ferro.model.wrong_type", format!("{t} is not a {what}")).on(t));
        }
        return Ok(t);
    }
    let default_name = if floor {
        DEFAULT_FLOOR_TYPE
    } else {
        DEFAULT_WALL_TYPE
    };
    if let Some(e) = tx
        .doc()
        .elements()
        .find(|e| is_kind(e) && e.name.as_deref() == Some(default_name))
    {
        return Ok(e.id);
    }
    let st = CompoundStructure::single(Length::from_mm(200.0));
    let (cat, body) = if floor {
        (
            Category::Floors,
            ElementBody::FloorType(FloorType { structure: st }),
        )
    } else {
        (
            Category::Walls,
            ElementBody::WallType(WallType { structure: st }),
        )
    };
    tx.create(cat, Some(default_name.into()), None, body)
}

// ------------------------------------------------------------------- walls

/// Placement options shared by the wall-creating commands.
struct WallOptions {
    level: Option<ElementId>,
    wall_type: Option<ElementId>,
    height: Option<Length>,
    top_level: Option<ElementId>,
    top_offset: Length,
    base_offset: Length,
    location_line: LocationLine,
    name: Option<String>,
}

fn create_wall(
    tx: &mut Transaction,
    curve: Curve,
    o: &WallOptions,
) -> Result<ElementId, CommandError> {
    let type_id = find_or_create_type(tx, o.wall_type, false)?;
    let top = match o.top_level {
        Some(l) => {
            if tx.element(l)?.as_level().is_none() {
                return Err(err("ferro.model.not_a_level", format!("{l} is not a level")).on(l));
            }
            WallTop::Level {
                level: l,
                offset: o.top_offset,
            }
        }
        None => WallTop::Unconnected {
            height: o.height.unwrap_or(Length::from_mm(DEFAULT_WALL_HEIGHT)),
        },
    };
    let id = tx.create(
        Category::Walls,
        o.name.clone(),
        o.level,
        ElementBody::Wall(Wall {
            curve,
            base_offset: o.base_offset,
            top,
            location_line: o.location_line,
            geometry: Default::default(),
        }),
    )?;
    tx.modify(id, |e| {
        e.type_id = Some(type_id);
        Ok(())
    })?;
    Ok(id)
}

macro_rules! wall_options {
    ($s:expr) => {
        WallOptions {
            level: $s.level,
            wall_type: $s.wall_type,
            height: $s.height,
            top_level: $s.top_level,
            top_offset: $s.top_offset,
            base_offset: $s.base_offset,
            location_line: $s.location_line,
            name: $s.name.clone(),
        }
    };
}

/// Create a straight wall from `start` to `end` (z is ignored; the base is
/// the level + `base_offset`). The exterior face is on the left of the
/// direction of travel. Height: `top_level` (+ `top_offset`), else
/// `height` (default 3 m).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateWall {
    pub start: Point3,
    pub end: Point3,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub wall_type: Option<ElementId>,
    #[serde(default)]
    pub height: Option<Length>,
    #[serde(default)]
    pub top_level: Option<ElementId>,
    #[serde(default)]
    pub top_offset: Length,
    #[serde(default)]
    pub base_offset: Length,
    #[serde(default)]
    pub location_line: LocationLine,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateWall {
    const NAME: &'static str = "create_wall";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let line = Line::from_points(flat(self.start), flat(self.end))?;
        let id = create_wall(tx, Curve::Line(line), &wall_options!(self))?;
        Ok(json!({ "element": id }))
    }
}

/// Create a chain of joined straight walls through `points`; `closed` adds
/// the closing wall. Corners are mitred automatically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateWalls {
    pub points: Vec<Point3>,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub wall_type: Option<ElementId>,
    #[serde(default)]
    pub height: Option<Length>,
    #[serde(default)]
    pub top_level: Option<ElementId>,
    #[serde(default)]
    pub top_offset: Length,
    #[serde(default)]
    pub base_offset: Length,
    #[serde(default)]
    pub location_line: LocationLine,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateWalls {
    const NAME: &'static str = "create_walls";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let n = self.points.len();
        if n < 2 || (self.closed && n < 3) {
            return Err(err(
                "ferro.geom.degenerate",
                "a wall chain needs 2 points (3 when closed)",
            ));
        }
        let opts = wall_options!(self);
        let segs = if self.closed { n } else { n - 1 };
        let mut ids = Vec::new();
        for i in 0..segs {
            let line = Line::from_points(flat(self.points[i]), flat(self.points[(i + 1) % n]))?;
            ids.push(create_wall(tx, Curve::Line(line), &opts)?);
        }
        Ok(json!({ "elements": ids }))
    }
}

/// Create a curved wall along the arc from `start` through `mid` to `end`.
/// Openings in curved walls are not supported yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateArcWall {
    pub start: Point3,
    pub mid: Point3,
    pub end: Point3,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub wall_type: Option<ElementId>,
    #[serde(default)]
    pub height: Option<Length>,
    #[serde(default)]
    pub top_level: Option<ElementId>,
    #[serde(default)]
    pub top_offset: Length,
    #[serde(default)]
    pub base_offset: Length,
    #[serde(default)]
    pub location_line: LocationLine,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateArcWall {
    const NAME: &'static str = "create_arc_wall";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let arc = Arc::from_three_points(flat(self.start), flat(self.mid), flat(self.end))?;
        // Keep the direction the user drew (exterior on the left).
        let arc = if arc.start().is_almost_equal(flat(self.start)) {
            arc
        } else {
            arc.reversed()
        };
        let id = create_wall(tx, Curve::Arc(arc), &wall_options!(self))?;
        Ok(json!({ "element": id }))
    }
}

/// Set a wall to a fixed (unconnected) height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetWallHeight {
    pub wall: ElementId,
    pub height: Length,
}

impl Command for SetWallHeight {
    const NAME: &'static str = "set_wall_height";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let h = self.height;
        tx.modify(self.wall, |e| match &mut e.body {
            ElementBody::Wall(w) => {
                w.top = WallTop::Unconnected { height: h };
                Ok(())
            }
            _ => Err(err("ferro.model.not_a_wall", "element is not a wall").on(e.id)),
        })?;
        Ok(json!({ "element": self.wall }))
    }
}

/// Change the type of a wall or floor (Revit: `ChangeTypeId`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetElementType {
    pub element: ElementId,
    pub element_type: ElementId,
}

impl Command for SetElementType {
    const NAME: &'static str = "set_element_type";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let t = tx.element(self.element_type)?;
        let ok = matches!(
            (&tx.element(self.element)?.body, &t.body),
            (ElementBody::Wall(_), ElementBody::WallType(_))
                | (ElementBody::Floor(_), ElementBody::FloorType(_))
        );
        if !ok {
            return Err(err(
                "ferro.model.wrong_type",
                format!("{} cannot take type {}", self.element, self.element_type),
            )
            .on(self.element));
        }
        let ty = self.element_type;
        tx.modify(self.element, |e| {
            e.type_id = Some(ty);
            Ok(())
        })?;
        Ok(json!({ "element": self.element }))
    }
}

// ------------------------------------------------------------------ floors

/// Create a floor from a plan outline (mm). Its top sits at level + `offset`
/// and it extends down by the type thickness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateFloor {
    pub points: Vec<Point2>,
    #[serde(default)]
    pub holes: Vec<Vec<Point2>>,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub offset: Length,
    #[serde(default)]
    pub floor_type: Option<ElementId>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateFloor {
    const NAME: &'static str = "create_floor";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if self.points.len() < 3 {
            return Err(err(
                "ferro.geom.degenerate",
                "a floor needs at least 3 points",
            ));
        }
        let type_id = find_or_create_type(tx, self.floor_type, true)?;
        let id = tx.create(
            Category::Floors,
            self.name.clone(),
            self.level,
            ElementBody::Floor(Floor {
                outer: self.points.clone(),
                holes: self.holes.clone(),
                offset: self.offset,
                geometry: Default::default(),
            }),
        )?;
        tx.modify(id, |e| {
            e.type_id = Some(type_id);
            Ok(())
        })?;
        Ok(json!({ "element": id }))
    }
}

// ---------------------------------------------------------------- openings

fn straight_layout(tx: &Transaction, wall: ElementId) -> Result<WallLayout, CommandError> {
    let e = tx.element(wall)?;
    let w = e
        .as_wall()
        .ok_or_else(|| err("ferro.model.not_a_wall", format!("{wall} is not a wall")).on(wall))?;
    let lay = generate::wall_layout(tx.doc(), e, w)
        .map_err(|i| CommandError::new(i.code, i.message).on(wall))?;
    if lay.frame().is_none() {
        return Err(err(
            "ferro.opening.curved_host",
            "openings in curved walls are not supported yet",
        )
        .on(wall));
    }
    Ok(lay)
}

/// Place a door or window in a straight wall. `position` is the distance from
/// the wall start to the opening centre. Defaults: door 900 × 2100, window
/// 1200 × 1200 with a 900 sill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateOpening {
    pub wall: ElementId,
    pub kind: OpeningKind,
    pub position: Length,
    #[serde(default)]
    pub width: Option<Length>,
    #[serde(default)]
    pub height: Option<Length>,
    #[serde(default)]
    pub sill: Option<Length>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateOpening {
    const NAME: &'static str = "create_opening";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        straight_layout(tx, self.wall)?;
        let level = tx.element(self.wall)?.level_id;
        let (dw, dh, ds, cat) = match self.kind {
            OpeningKind::Door => (900.0, 2100.0, 0.0, Category::Doors),
            OpeningKind::Window => (1200.0, 1200.0, 900.0, Category::Windows),
        };
        let mm = Length::from_mm;
        let id = tx.create(
            cat,
            self.name.clone(),
            level,
            ElementBody::Opening(Opening {
                kind: self.kind,
                position: self.position,
                width: self.width.unwrap_or(mm(dw)),
                height: self.height.unwrap_or(mm(dh)),
                sill: self.sill.unwrap_or(mm(ds)),
                geometry: Default::default(),
            }),
        )?;
        let host = self.wall;
        tx.modify(id, |e| {
            e.host_id = Some(host);
            Ok(())
        })?;
        Ok(json!({ "element": id }))
    }
}

/// Change an opening's placement or size; omitted fields keep their value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetOpening {
    pub opening: ElementId,
    #[serde(default)]
    pub position: Option<Length>,
    #[serde(default)]
    pub width: Option<Length>,
    #[serde(default)]
    pub height: Option<Length>,
    #[serde(default)]
    pub sill: Option<Length>,
}

impl Command for SetOpening {
    const NAME: &'static str = "set_opening";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let s = self.clone();
        tx.modify(self.opening, |e| match &mut e.body {
            ElementBody::Opening(o) => {
                o.position = s.position.unwrap_or(o.position);
                o.width = s.width.unwrap_or(o.width);
                o.height = s.height.unwrap_or(o.height);
                o.sill = s.sill.unwrap_or(o.sill);
                Ok(())
            }
            _ => Err(err(
                "ferro.model.not_an_opening",
                "element is not a door or window",
            )
            .on(e.id)),
        })?;
        Ok(json!({ "element": self.opening }))
    }
}

// ------------------------------------------------------ semantic push/pull

fn face_info(e: &Element, face: FaceId) -> Result<(Vec3, Point3), CommandError> {
    let body = e
        .geometry()
        .ok_or_else(|| err("ferro.model.no_geometry", "element has no geometry").on(e.id))?;
    let n = body
        .face_normal(face)
        .map_err(|x| CommandError::from(x).on(e.id))?
        .as_vec();
    let pts = body
        .face_points(face)
        .map_err(|x| CommandError::from(x).on(e.id))?;
    let c = Point3::centroid(&pts).ok_or_else(|| err("ferro.geom.degenerate", "empty face"))?;
    Ok((n, c))
}

fn unsupported(id: ElementId, what: &str) -> CommandError {
    err(
        "ferro.ops.unsupported_push_pull",
        format!("push/pull on this face is not supported: {what}"),
    )
    .on(id)
}

fn openings_of(tx: &Transaction, wall: ElementId) -> Vec<(ElementId, Opening)> {
    tx.doc()
        .elements()
        .filter(|e| e.host_id == Some(wall))
        .filter_map(|e| match &e.body {
            ElementBody::Opening(o) => Some((e.id, o.clone())),
            _ => None,
        })
        .collect()
}

fn adjust_opening(
    tx: &mut Transaction,
    id: ElementId,
    f: impl FnOnce(&mut Opening),
) -> Result<(), CommandError> {
    tx.modify(id, |e| {
        if let ElementBody::Opening(o) = &mut e.body {
            f(o);
        }
        Ok(())
    })
}

/// Push/pull a face of a wall, floor or opening by editing its parameters
/// (D-73). Returns the parameter that changed.
pub(super) fn semantic_push_pull(
    tx: &mut Transaction,
    id: ElementId,
    face: FaceId,
    dist: f64,
) -> Result<CommandOutput, CommandError> {
    let e = tx.element(id)?.clone();
    let (n, c) = face_info(&e, face)?;
    let done = |what: &str| Ok(json!({ "element": id, "mode": "parameter", "parameter": what }));
    match &e.body {
        ElementBody::Wall(w) => {
            let lay = generate::wall_layout(tx.doc(), &e, w)
                .map_err(|i| CommandError::new(i.code, i.message).on(id))?;
            let tol_z = 1.0;
            if n.z > 0.9 {
                if (c.z - lay.z1).abs() < tol_z {
                    tx.modify(id, |e| {
                        if let ElementBody::Wall(w) = &mut e.body {
                            match &mut w.top {
                                WallTop::Unconnected { height } => {
                                    *height = Length::from_mm(height.mm() + dist)
                                }
                                WallTop::Level { offset, .. } => {
                                    *offset = Length::from_mm(offset.mm() + dist)
                                }
                            }
                        }
                        Ok(())
                    })?;
                    return done("height");
                }
                // Window sill reveal.
                let u = wall_u(&lay, c);
                for (oid, o) in openings_of(tx, id) {
                    if (lay.z0 + o.sill.mm() - c.z).abs() < tol_z && covers(&o, u) {
                        adjust_opening(tx, oid, |o| {
                            o.sill = Length::from_mm(o.sill.mm() + dist);
                            o.height = Length::from_mm(o.height.mm() - dist);
                        })?;
                        return done("sill");
                    }
                }
                return Err(unsupported(id, "horizontal face"));
            }
            if n.z < -0.9 {
                if (c.z - lay.z0).abs() < tol_z {
                    tx.modify(id, |e| {
                        if let ElementBody::Wall(w) = &mut e.body {
                            w.base_offset = Length::from_mm(w.base_offset.mm() - dist);
                            if let WallTop::Unconnected { height } = &mut w.top {
                                *height = Length::from_mm(height.mm() + dist);
                            }
                        }
                        Ok(())
                    })?;
                    return done("base_offset");
                }
                let u = wall_u(&lay, c);
                for (oid, o) in openings_of(tx, id) {
                    let head = lay.z0 + o.sill.mm() + o.height.mm();
                    if (head - c.z).abs() < tol_z && covers(&o, u) {
                        adjust_opening(tx, oid, |o| {
                            o.height = Length::from_mm(o.height.mm() - dist);
                        })?;
                        return done("height");
                    }
                }
                return Err(unsupported(id, "horizontal face"));
            }
            let Some((s, d, nn)) = lay.frame() else {
                return Err(unsupported(id, "side of a curved wall"));
            };
            let h = Vec2::new(n.x, n.y);
            if h.dot(nn).abs() > 0.9 {
                // Side face: move the wall along the face normal.
                let v = Vec3::new(n.x, n.y, 0.0) * dist;
                move_wall_curve(tx, id, v)?;
                return done("location");
            }
            let u = wall_u(&lay, c);
            for (oid, o) in openings_of(tx, id) {
                let (a, b) = (
                    o.position.mm() - o.width.mm() * 0.5,
                    o.position.mm() + o.width.mm() * 0.5,
                );
                if (u - a).abs() < tol_z && h.dot(d) > 0.0 {
                    adjust_opening(tx, oid, |o| {
                        o.width = Length::from_mm(o.width.mm() - dist);
                        o.position = Length::from_mm(o.position.mm() + dist * 0.5);
                    })?;
                    return done("width");
                }
                if (u - b).abs() < tol_z && h.dot(d) < 0.0 {
                    adjust_opening(tx, oid, |o| {
                        o.width = Length::from_mm(o.width.mm() - dist);
                        o.position = Length::from_mm(o.position.mm() - dist * 0.5);
                    })?;
                    return done("width");
                }
            }
            // End face: extend or shorten the wall.
            let len = lay.length();
            let at_start = u < len * 0.5;
            let e_pt = s + d * len;
            let (ns, ne) = if at_start {
                (s + d * (-dist), e_pt)
            } else {
                (s, e_pt + d * dist)
            };
            let line =
                Line::from_points(Point3::new(ns.x, ns.y, 0.0), Point3::new(ne.x, ne.y, 0.0))?;
            let shift = if at_start { dist } else { 0.0 };
            tx.modify(id, |e| {
                if let ElementBody::Wall(w) = &mut e.body {
                    w.curve = Curve::Line(line);
                }
                Ok(())
            })?;
            // Openings keep their place in space when the start moves.
            if shift != 0.0 {
                for (oid, _) in openings_of(tx, id) {
                    adjust_opening(tx, oid, |o| {
                        o.position = Length::from_mm(o.position.mm() + shift)
                    })?;
                }
            }
            done("length")
        }
        ElementBody::Floor(_) => {
            if n.z > 0.9 {
                tx.modify(id, |e| {
                    if let ElementBody::Floor(f) = &mut e.body {
                        f.offset = Length::from_mm(f.offset.mm() + dist);
                    }
                    Ok(())
                })?;
                return done("offset");
            }
            Err(unsupported(
                id,
                "floor thickness comes from its type; change the type instead",
            ))
        }
        ElementBody::Opening(o) => {
            let host = e
                .host_id
                .ok_or_else(|| err("ferro.opening.no_host", "opening has no host").on(id))?;
            let lay = straight_layout(tx, host)?;
            let (_, d, _) = lay.frame().ok_or_else(|| unsupported(id, "curved host"))?;
            let h = Vec2::new(n.x, n.y);
            let kind = o.kind;
            if n.z > 0.9 {
                adjust_opening(tx, id, |o| o.height = Length::from_mm(o.height.mm() + dist))?;
                return done("height");
            }
            if n.z < -0.9 {
                if kind == OpeningKind::Door {
                    return Err(unsupported(id, "a door sits on the floor"));
                }
                adjust_opening(tx, id, |o| {
                    o.sill = Length::from_mm(o.sill.mm() - dist);
                    o.height = Length::from_mm(o.height.mm() + dist);
                })?;
                return done("sill");
            }
            let along = h.dot(d);
            if along.abs() > 0.7 {
                let sign = along.signum();
                adjust_opening(tx, id, |o| {
                    o.width = Length::from_mm(o.width.mm() + dist);
                    o.position = Length::from_mm(o.position.mm() + sign * dist * 0.5);
                })?;
                return done("width");
            }
            Err(unsupported(id, "opening depth follows the wall"))
        }
        _ => Err(unsupported(id, "element kind")),
    }
}

fn wall_u(lay: &WallLayout, p: Point3) -> f64 {
    match lay.frame() {
        Some((s, d, _)) => (Point2::new(p.x, p.y) - s).dot(d),
        None => 0.0,
    }
}

fn covers(o: &Opening, u: f64) -> bool {
    let half = o.width.mm() * 0.5;
    u > o.position.mm() - half - 1.0 && u < o.position.mm() + half + 1.0
}

fn move_wall_curve(tx: &mut Transaction, id: ElementId, v: Vec3) -> Result<(), CommandError> {
    let t = crate::math::Transform::translation(v);
    tx.modify(id, |e| {
        if let ElementBody::Wall(w) = &mut e.body {
            w.curve = w.curve.transformed(&t)?;
        }
        Ok(())
    })
}
