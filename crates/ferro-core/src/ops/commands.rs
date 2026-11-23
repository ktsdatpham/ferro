//! Core commands (milestone 1) and the dispatch enum; BIM commands live in
//! `bim_commands`. Each command's `NAME` is also its MCP tool name
//! (D-17, `verb_noun`). Arguments use kernel units: lengths in mm, angles in
//! radians; the MCP edge converts user units (`"3m"`, degrees).

use super::bim_commands::*;
use super::form_commands::*;
use super::geometry_source::*;
use super::parameter_commands::*;
use super::phase_commands::*;
use super::project_commands::*;
use super::relationship_commands::*;
use super::{CommandError, Transaction};
use crate::algo::{push_pull_face, PushPullMode};
use crate::geom::{polygon, Plane};
use crate::math::{tol, Angle, Length, Point3, Transform, UnitVec3, Vec3};
use crate::model::generate;
use crate::model::{keys, Category, ElementBody, ElementId, Form, Level, ParamValue};
use crate::topo::{Body, FaceId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// JSON result of a command (ids created, lineage, ...).
pub type CommandOutput = Value;

/// A serialisable intent (§4.2). Deterministic: same document + same command
/// (+ same id source) = same result.
pub trait Command: Serialize + DeserializeOwned {
    const NAME: &'static str;
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError>;
}

fn level_z(tx: &Transaction, level: Option<ElementId>) -> Result<f64, CommandError> {
    match level {
        None => Ok(0.0),
        Some(l) => tx
            .element(l)?
            .as_level()
            .map(|lv| lv.elevation.mm())
            .ok_or_else(|| {
                CommandError::new("ferro.model.not_a_level", format!("{l} is not a level")).on(l)
            }),
    }
}

fn form_body(tx: &Transaction, id: ElementId) -> Result<Body, CommandError> {
    tx.element(id)?
        .as_form()
        .map(|f| f.body.clone())
        .ok_or_else(|| {
            CommandError::new("ferro.model.not_a_form", format!("{id} is not a Form")).on(id)
        })
}

// ------------------------------------------------------------------ levels

/// Create a level at an elevation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateLevel {
    pub name: String,
    pub elevation: Length,
    #[serde(default = "yes")]
    pub is_building_storey: bool,
}
fn yes() -> bool {
    true
}

impl Command for CreateLevel {
    const NAME: &'static str = "create_level";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if tx
            .doc()
            .by_category(Category::Levels)
            .any(|l| l.name.as_deref() == Some(self.name.as_str()))
        {
            return Err(CommandError::new(
                "ferro.model.duplicate_name",
                format!("a level named {:?} already exists", self.name),
            ));
        }
        let id = tx.create(
            Category::Levels,
            Some(self.name.clone()),
            None,
            ElementBody::Level(Level {
                elevation: self.elevation,
                is_building_storey: self.is_building_storey,
            }),
        )?;
        Ok(json!({ "element": id }))
    }
}

/// Change a level's elevation; Forms on that level move with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetLevelElevation {
    pub level: ElementId,
    pub elevation: Length,
}

impl Command for SetLevelElevation {
    const NAME: &'static str = "set_level_elevation";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let elev = self.elevation;
        tx.modify(self.level, |e| match &mut e.body {
            ElementBody::Level(l) => {
                l.elevation = elev;
                Ok(())
            }
            _ => {
                Err(CommandError::new("ferro.model.not_a_level", "element is not a level").on(e.id))
            }
        })?;
        Ok(json!({ "element": self.level }))
    }
}

// ------------------------------------------------------------------ forms

/// Draw a horizontal rectangle face (SketchUp Rectangle tool). With a level,
/// `origin.z` is relative to the level elevation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateRectangle {
    pub origin: Point3,
    pub width: Length,
    pub depth: Length,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateRectangle {
    const NAME: &'static str = "create_rectangle";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let o = self.origin + Vec3::new(0.0, 0.0, level_z(tx, self.level)?);
        let (w, d) = (self.width.mm(), self.depth.mm());
        let mut pts = vec![
            o,
            o + Vec3::new(w, 0.0, 0.0),
            o + Vec3::new(w, d, 0.0),
            o + Vec3::new(0.0, d, 0.0),
        ];
        // Negative width/depth flip the winding; keep the face facing up.
        if w * d < 0.0 {
            pts.reverse();
        }
        CreatePolygon {
            points: pts,
            level: self.level,
            name: self.name.clone(),
            relative_to_level: false,
        }
        .execute(tx)
    }
}

/// Draw a planar polygon face from 3+ points (mm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatePolygon {
    pub points: Vec<Point3>,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub name: Option<String>,
    /// When true and a level is given, point z values are relative to it.
    #[serde(default)]
    pub relative_to_level: bool,
}

impl Command for CreatePolygon {
    const NAME: &'static str = "create_polygon";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let dz = if self.relative_to_level {
            level_z(tx, self.level)?
        } else {
            0.0
        };
        let pts: Vec<Point3> = self
            .points
            .iter()
            .map(|p| *p + Vec3::new(0.0, 0.0, dz))
            .collect();
        if pts.len() < 3 {
            return Err(CommandError::new(
                "ferro.geom.degenerate",
                "a face needs at least 3 points",
            ));
        }
        for i in 0..pts.len() {
            if pts[i].distance_to(pts[(i + 1) % pts.len()]) < tol::SHORT_CURVE_TOL {
                return Err(CommandError::new(
                    "ferro.geom.short_curve",
                    "edges must be at least 0.5 mm long",
                ));
            }
        }
        let n = polygon::polygon_normal(&pts)
            .ok_or_else(|| CommandError::new("ferro.geom.degenerate", "points are collinear"))?;
        let plane = Plane::from_origin_normal(pts[0], n);
        if pts.iter().any(|p| !plane.contains(*p)) {
            return Err(CommandError::new(
                "ferro.geom.not_planar",
                "points are not coplanar",
            ));
        }
        // Exact simple-polygon test (D-53): no bow-ties, no repeated points.
        let local: Vec<crate::math::Point2> = pts.iter().map(|p| plane.to_local(*p)).collect();
        if crate::geom::predicates::points_are_collinear(&local) {
            return Err(CommandError::new(
                "ferro.geom.degenerate",
                "points are collinear",
            ));
        }
        if !crate::geom::predicates::polygon_is_simple(&local) {
            return Err(crate::geom::GeomError::SelfIntersecting.into());
        }
        let idx: Vec<usize> = (0..pts.len()).collect();
        let (body, faces) = Body::from_polygons(&pts, &[idx])?;
        let id = tx.create(
            Category::Forms,
            self.name.clone(),
            self.level,
            ElementBody::Form(Form { body }),
        )?;
        Ok(json!({ "element": id, "face": faces[0] }))
    }
}

/// Convenience for agents: rectangle + push/pull in one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateBox {
    pub origin: Point3,
    pub width: Length,
    pub depth: Length,
    pub height: Length,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateBox {
    const NAME: &'static str = "create_box";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let r = CreateRectangle {
            origin: self.origin,
            width: self.width,
            depth: self.depth,
            level: self.level,
            name: self.name.clone(),
        }
        .execute(tx)?;
        let element: ElementId = serde_json::from_value(r["element"].clone()).expect("own output");
        let face: FaceId = serde_json::from_value(r["face"].clone()).expect("own output");
        PushPullFace {
            element,
            face,
            distance: self.height,
        }
        .execute(tx)?;
        Ok(json!({ "element": element }))
    }
}

/// Push / pull a face of a Form along its normal (negative = inward).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushPullFace {
    pub element: ElementId,
    pub face: FaceId,
    pub distance: Length,
}

impl Command for PushPullFace {
    const NAME: &'static str = "push_pull_face";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if tx.element(self.element)?.as_form().is_none() {
            return semantic_push_pull(tx, self.element, self.face, self.distance.mm());
        }
        let mut body = form_body(tx, self.element)?;
        let (mode, lineage) = push_pull_face(&mut body, self.face, self.distance.mm())
            .map_err(|e| CommandError::from(e).on(self.element))?;
        tx.modify(self.element, |e| {
            e.body = ElementBody::Form(Form { body });
            Ok(())
        })?;
        let mode = match mode {
            PushPullMode::Extrude => "extrude",
            PushPullMode::Stretch => "stretch",
            PushPullMode::Prism => "prism",
            PushPullMode::Through => "through",
        };
        Ok(json!({ "element": self.element, "mode": mode, "lineage": lineage }))
    }
}

// ------------------------------------------------------------------ edits

fn transform_elements(
    tx: &mut Transaction,
    ids: &[ElementId],
    t: &Transform,
) -> Result<(), CommandError> {
    let set: BTreeSet<ElementId> = ids.iter().copied().collect();
    let dz = t.apply(Point3::ORIGIN).z;
    for id in ids {
        let e = tx.element(*id)?.clone();
        // Hosted openings follow their host; alone they slide along it.
        let mut host_frame = None;
        if let ElementBody::Opening(_) = e.body {
            match e.host_id {
                Some(h) if set.contains(&h) => continue,
                Some(h) => {
                    let host = tx.element(h)?;
                    if let Some(w) = host.as_wall() {
                        host_frame = generate::wall_layout(tx.doc(), host, w)
                            .ok()
                            .and_then(|l| l.frame());
                    }
                }
                None => {}
            }
        }
        tx.modify(*id, |e| match &mut e.body {
            ElementBody::Form(f) => {
                f.body.transform(t);
                Ok(())
            }
            ElementBody::Level(_) => Err(CommandError::new(
                "ferro.model.not_movable",
                "levels move with set_level_elevation",
            )
            .on(e.id)),
            ElementBody::Material(_)
            | ElementBody::WallType(_)
            | ElementBody::FloorType(_)
            | ElementBody::Phase(_) => Err(CommandError::new(
                "ferro.model.not_movable",
                "types, materials and phases have no location",
            )
            .on(e.id)),
            ElementBody::Wall(w) => {
                w.curve = match w.curve.transformed(t)? {
                    crate::geom::Curve::Line(l) => {
                        let f = |p: Point3| Point3::new(p.x, p.y, 0.0);
                        crate::geom::Curve::Line(crate::geom::Line::from_points(
                            f(l.start()),
                            f(l.end()),
                        )?)
                    }
                    c => c,
                };
                w.base_offset = Length::from_mm(w.base_offset.mm() + dz);
                if let crate::model::bim::WallTop::Level { offset, .. } = &mut w.top {
                    *offset = Length::from_mm(offset.mm() + dz);
                }
                Ok(())
            }
            ElementBody::Floor(f) => {
                let m = |p: &mut crate::math::Point2| {
                    let q = t.apply(Point3::new(p.x, p.y, 0.0));
                    *p = crate::math::Point2::new(q.x, q.y);
                };
                f.outer.iter_mut().for_each(m);
                f.holes.iter_mut().flatten().for_each(m);
                f.offset = Length::from_mm(f.offset.mm() + dz);
                Ok(())
            }
            ElementBody::Opening(o) => {
                let Some((s, d, _)) = host_frame else {
                    return Err(CommandError::new(
                        "ferro.opening.no_host",
                        "a door or window moves along its host wall",
                    )
                    .on(e.id));
                };
                let c = s + d * o.position.mm();
                let q = t.apply(Point3::new(c.x, c.y, 0.0));
                let u = (crate::math::Point2::new(q.x, q.y) - s).dot(d);
                o.position = Length::from_mm(u);
                Ok(())
            }
        })?;
    }
    Ok(())
}

/// Move elements by a vector (mm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveElements {
    pub elements: Vec<ElementId>,
    pub vector: Vec3,
}

impl Command for MoveElements {
    const NAME: &'static str = "move_elements";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        transform_elements(tx, &self.elements, &Transform::translation(self.vector))?;
        Ok(json!({ "elements": self.elements }))
    }
}

/// Rotate elements about a vertical axis through `center`
/// (counter-clockwise positive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RotateElements {
    pub elements: Vec<ElementId>,
    pub center: Point3,
    pub angle: Angle,
}

impl Command for RotateElements {
    const NAME: &'static str = "rotate_elements";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let t = Transform::rotation(self.center, UnitVec3::Z, self.angle);
        transform_elements(tx, &self.elements, &t)?;
        Ok(json!({ "elements": self.elements }))
    }
}

/// Copy elements, offset by a vector (mm). `count` > 1 makes a linear array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyElements {
    pub elements: Vec<ElementId>,
    pub vector: Vec3,
    #[serde(default = "one")]
    pub count: u32,
}
fn one() -> u32 {
    1
}

impl Command for CopyElements {
    const NAME: &'static str = "copy_elements";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if self.count == 0 || self.count > 1000 {
            return Err(CommandError::new(
                "ferro.ops.invalid_count",
                "count must be 1..=1000",
            ));
        }
        let set: BTreeSet<ElementId> = self.elements.iter().copied().collect();
        let mut created = Vec::new();
        for i in 1..=self.count {
            for id in &self.elements {
                let e = tx.element(*id)?;
                if e.host_id.is_some_and(|h| set.contains(&h)) {
                    continue; // copied with its host below
                }
                let hosted: Vec<ElementId> = tx
                    .doc()
                    .elements()
                    .filter(|o| o.host_id == Some(*id))
                    .map(|o| o.id)
                    .collect();
                let new = tx.duplicate(*id)?;
                transform_elements(tx, &[new], &Transform::translation(self.vector * i as f64))?;
                created.push(new);
                for h in hosted {
                    let c = tx.duplicate(h)?;
                    tx.modify(c, |e| {
                        e.host_id = Some(new);
                        Ok(())
                    })?;
                    created.push(c);
                }
            }
        }
        Ok(json!({ "elements": created }))
    }
}

/// Delete elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteElements {
    pub elements: Vec<ElementId>,
}

impl Command for DeleteElements {
    const NAME: &'static str = "delete_elements";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        for id in &self.elements {
            tx.delete(*id)?;
        }
        Ok(json!({ "deleted": self.elements }))
    }
}

// ------------------------------------------------------------------ data

/// Rename an element (`null` clears the name). `name` is required: a
/// missing `name` is rejected as bad arguments, not read as `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetName {
    pub element: ElementId,
    /// `deserialize_with` makes the field required while still taking `null`.
    #[serde(deserialize_with = "Option::deserialize")]
    pub name: Option<String>,
}

impl Command for SetName {
    const NAME: &'static str = "set_name";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let name = self.name.clone();
        tx.modify(self.element, |e| {
            e.name = name;
            Ok(())
        })?;
        Ok(json!({ "element": self.element }))
    }
}

/// Set a typed parameter (D-15). Computed keys are read-only. Quantities are
/// in storage units (`ParamValue::Quantity`, ADR 0004 §2).
///
/// Fails with `ferro.model.invalid_parameter_key`,
/// `ferro.model.read_only_parameter`, `ferro.model.invalid_parameter_value`
/// (NaN / infinite number) or `ferro.model.parameter_type_mismatch` (the key
/// has a definition whose data type the value does not fit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetParameter {
    pub element: ElementId,
    pub key: String,
    pub value: ParamValue,
}

impl Command for SetParameter {
    const NAME: &'static str = "set_parameter";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if !keys::is_valid(&self.key) {
            return Err(CommandError::new(
                "ferro.model.invalid_parameter_key",
                format!("{:?} is not a valid key (namespace.group.name)", self.key),
            ));
        }
        if keys::is_read_only(&self.key) {
            return Err(CommandError::new(
                "ferro.model.read_only_parameter",
                format!("{} is computed and read-only", self.key),
            ));
        }
        if !self.value.is_finite() {
            return Err(CommandError::new(
                "ferro.model.invalid_parameter_value",
                format!("{} must be a finite number", self.key),
            )
            .on(self.element));
        }
        if let Some(def) = tx.doc().parameter_definition(&self.key) {
            if !def.accepts(&self.value) {
                return Err(CommandError::new(
                    "ferro.model.parameter_type_mismatch",
                    format!(
                        "{} holds {}, not {}",
                        self.key,
                        def.data_type.describe(),
                        self.value.type_name()
                    ),
                )
                .on(self.element));
            }
        }
        if let ParamValue::ElementRef(r) = &self.value {
            tx.element(*r)?;
        }
        let (k, v) = (self.key.clone(), self.value.clone());
        tx.modify(self.element, |e| {
            e.params.insert(k, v);
            Ok(())
        })?;
        Ok(json!({ "element": self.element }))
    }
}

/// Classify directly edited geometry as a BIM category (D-12, ADR 0004),
/// e.g. `walls`: a free-form wall keeps its shape and gets the category's
/// data. A type that does not fit the new category is removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetCategory {
    pub element: ElementId,
    pub category: Category,
}

impl Command for SetCategory {
    const NAME: &'static str = "set_category";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if !self.category.classifiable_form() {
            return Err(CommandError::new(
                "ferro.model.invalid_category",
                format!("a Form cannot be classified as {}", self.category),
            ));
        }
        let c = self.category;
        let type_fits = tx
            .element(self.element)?
            .type_id
            .and_then(|t| tx.doc().get(t))
            .is_some_and(|t| {
                matches!(
                    (c, &t.body),
                    (Category::Walls, crate::model::ElementBody::WallType(_))
                        | (Category::Floors, crate::model::ElementBody::FloorType(_))
                )
            });
        tx.modify(self.element, |e| {
            if e.as_form().is_none() {
                return Err(CommandError::new(
                    "ferro.model.not_a_form",
                    "only Forms can be classified",
                )
                .on(e.id));
            }
            e.category = c;
            // Keep the type only if it still fits (wall types for walls...).
            if !type_fits {
                e.type_id = None;
            }
            Ok(())
        })?;
        Ok(json!({ "element": self.element }))
    }
}

/// Rename the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetProjectName {
    pub name: String,
}

impl Command for SetProjectName {
    const NAME: &'static str = "set_project_name";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        tx.project_name(self.name.clone());
        Ok(json!({}))
    }
}

// ------------------------------------------------------------------ dispatch

macro_rules! any_command {
    ($($variant:ident),* $(,)?) => {
        /// Any command, serialised as `{"command": "<name>", ...args}`.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "command", rename_all = "snake_case")]
        pub enum AnyCommand {
            $($variant($variant),)*
        }

        impl AnyCommand {
            pub fn name(&self) -> &'static str {
                match self { $(AnyCommand::$variant(_) => $variant::NAME,)* }
            }
            pub fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
                match self { $(AnyCommand::$variant(c) => c.execute(tx),)* }
            }
            /// All command names (= MCP tool names).
            pub const NAMES: &'static [&'static str] = &[$($variant::NAME,)*];
        }

        $(impl From<$variant> for AnyCommand {
            fn from(c: $variant) -> Self { AnyCommand::$variant(c) }
        })*
    };
}

any_command!(
    CreateLevel,
    SetLevelElevation,
    CreateRectangle,
    CreatePolygon,
    CreateBox,
    PushPullFace,
    MoveElements,
    RotateElements,
    CopyElements,
    DeleteElements,
    SetName,
    SetParameter,
    SetCategory,
    SetProjectName,
    CreateMaterial,
    CreateWallType,
    CreateFloorType,
    CreateWall,
    CreateWalls,
    CreateArcWall,
    SetWallHeight,
    SetElementType,
    CreateFloor,
    CreateOpening,
    SetOpening,
    SetWall,
    SetFloor,
    ImprintFace,
    CreateCircle,
    OffsetFace,
    MoveGeometry,
    ScaleElements,
    EraseEdges,
    EraseFaces,
    SetGeometrySource,
    SetProjectUnits,
    SetSiteLocation,
    SetProjectCurrency,
    SetMaterial,
    SetParameterDefinition,
    CreatePhase,
    SetPhase,
    DeletePhase,
    SetElementPhase,
    SetRenovationStatus,
    CreateRelationship,
    DeleteRelationship,
);

impl AnyCommand {
    /// Build from a tool name and JSON arguments (MCP).
    pub fn from_tool(name: &str, args: &Value) -> Result<AnyCommand, String> {
        if !Self::NAMES.contains(&name) {
            return Err(format!("unknown command: {name}"));
        }
        let mut obj = args.as_object().cloned().unwrap_or_default();
        obj.insert("command".into(), Value::String(name.into()));
        serde_json::from_value(Value::Object(obj))
            .map_err(|e| format!("invalid arguments for {name}: {e}"))
    }
}
