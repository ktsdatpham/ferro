//! Switching an element's geometry source (ADR 0004 §1): freeze a
//! parametric wall / floor into directly edited geometry, or recognise a
//! directly edited wall / floor shape as parametric again. Lengths in mm;
//! each `NAME` is also the MCP tool name.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::geom::{Curve, Line};
use crate::math::{tol, Length, Point2, Point3};
use crate::model::bim::{Floor, LocationLine, Wall, WallTop};
use crate::model::generate::level_elevation;
use crate::model::{Category, Document, Element, ElementBody, ElementId, Form, GeometrySource};
use crate::topo::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;

fn err(code: &'static str, msg: impl Into<String>) -> CommandError {
    CommandError::new(code, msg)
}

/// Change where an element's geometry comes from.
///
/// * `direct`: a parametric wall or floor keeps its current shape as directly
///   editable geometry (its category, type and parameters stay). Openings
///   hosted by a frozen wall are frozen with it. Always possible.
/// * `parametric`: a directly edited wall or floor whose shape is a plain
///   wall (rectangular, vertical sides) or floor (horizontal prism) becomes
///   parametric again, using its type or a type of the same thickness.
///   Fails with `ferro.model.not_recognised` or
///   `ferro.model.no_matching_type` otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetGeometrySource {
    pub element: ElementId,
    pub source: GeometrySource,
}

impl Command for SetGeometrySource {
    const NAME: &'static str = "set_geometry_source";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let e = tx.element(self.element)?;
        let Some(current) = e.geometry_source() else {
            return Err(err(
                "ferro.model.no_geometry",
                format!("{} has no geometry", e.label()),
            )
            .on(e.id));
        };
        if current == self.source {
            return Ok(json!({ "element": self.element, "changed": [] }));
        }
        let changed = match self.source {
            GeometrySource::Direct => freeze(tx, self.element)?,
            GeometrySource::Parametric => {
                unfreeze(tx, self.element)?;
                vec![self.element]
            }
        };
        Ok(json!({ "element": self.element, "changed": changed }))
    }
}

/// Parametric → direct. Returns the elements changed (the element and the
/// openings it hosts).
fn freeze(tx: &mut Transaction, id: ElementId) -> Result<Vec<ElementId>, CommandError> {
    let e = tx.element(id)?;
    match &e.body {
        ElementBody::Opening(_) => {
            return Err(err(
                "ferro.model.hosted_opening",
                "an opening is frozen with its host wall; freeze the wall",
            )
            .on(id))
        }
        ElementBody::Wall(_) | ElementBody::Floor(_) => {}
        _ => {
            return Err(err("ferro.model.no_geometry", "nothing to freeze").on(id));
        }
    }
    if e.geometry().is_none_or(|b| b.face_ids().next().is_none()) {
        return Err(err(
            "ferro.model.no_geometry",
            format!("{} has no geometry to keep", e.label()),
        )
        .on(id));
    }
    let mut ids = vec![id];
    if matches!(e.body, ElementBody::Wall(_)) {
        let mut hosted: Vec<ElementId> = tx
            .doc()
            .elements()
            .filter(|o| o.host_id == Some(id) && matches!(o.body, ElementBody::Opening(_)))
            .map(|o| o.id)
            .collect();
        hosted.sort();
        ids.extend(hosted);
    }
    for &x in &ids {
        // A cut element freezes uncut: its `cut` rows keep cutting the Form
        // (non-destructive, M5-5.3).
        let uncut = if crate::model::cut::cuts_of(tx.doc(), x).is_empty() {
            None
        } else {
            crate::model::generate::uncut_body(tx.doc(), x)
        };
        tx.modify(x, |e| {
            let body = uncut.or_else(|| e.geometry().cloned()).unwrap_or_default();
            e.body = ElementBody::Form(Form::new(body));
            Ok(())
        })?;
    }
    Ok(ids)
}

/// Direct → parametric, when the shape is recognised.
fn unfreeze(tx: &mut Transaction, id: ElementId) -> Result<(), CommandError> {
    let e = tx.element(id)?;
    let body = e
        .as_form()
        .map(|f| f.body.clone())
        .ok_or_else(|| err("ferro.model.no_geometry", "not directly edited").on(id))?;
    let doc = tx.doc();
    let new_body = match e.category {
        Category::Walls => {
            let w = recognise_wall(&body).ok_or_else(|| not_recognised(e, "a straight wall"))?;
            let (ty, _) = matching_type(doc, e, w.thickness, true)?;
            let base = w.z0 - level_elevation(doc, e.level_id);
            let line = Line::from_points(w.start, w.end)?;
            (
                ElementBody::Wall(Wall {
                    curve: Curve::Line(line),
                    base_offset: Length::from_mm(base),
                    top: WallTop::Unconnected {
                        height: Length::from_mm(w.height),
                    },
                    location_line: LocationLine::WallCenterline,
                    geometry: Body::default(),
                }),
                ty,
            )
        }
        Category::Floors => {
            let f = recognise_floor(&body).ok_or_else(|| not_recognised(e, "a flat floor"))?;
            let (ty, _) = matching_type(doc, e, f.thickness, false)?;
            (
                ElementBody::Floor(Floor {
                    outer: f.outer,
                    holes: f.holes,
                    offset: Length::from_mm(f.top - level_elevation(doc, e.level_id)),
                    geometry: Body::default(),
                }),
                ty,
            )
        }
        c => {
            return Err(err(
                "ferro.model.not_recognised",
                format!("{c} elements have no parametric form yet"),
            )
            .on(id))
        }
    };
    let (body, ty) = new_body;
    tx.modify(id, |e| {
        e.body = body;
        e.type_id = Some(ty);
        Ok(())
    })
}

fn not_recognised(e: &Element, what: &str) -> CommandError {
    err(
        "ferro.model.not_recognised",
        format!("the shape of {} is not {what}", e.label()),
    )
    .on(e.id)
}

/// The element's own type if its thickness matches, else the first type of
/// that thickness (by id). Returns the type and its thickness.
fn matching_type(
    doc: &Document,
    e: &Element,
    thickness: f64,
    wall: bool,
) -> Result<(ElementId, f64), CommandError> {
    let thickness_of = |t: &Element| match (&t.body, wall) {
        (ElementBody::WallType(w), true) => Some(w.structure.thickness()),
        (ElementBody::FloorType(f), false) => Some(f.structure.thickness()),
        _ => None,
    };
    let fits =
        |t: &Element| thickness_of(t).is_some_and(|x| (x - thickness).abs() <= tol::VERTEX_TOL);
    if let Some(t) = e.type_id.and_then(|t| doc.get(t)).filter(|t| fits(t)) {
        return Ok((t.id, thickness));
    }
    let mut candidates: Vec<&Element> = doc.elements().filter(|t| fits(t)).collect();
    candidates.sort_by_key(|t| t.id);
    candidates
        .first()
        .map(|t| (t.id, thickness))
        .ok_or_else(|| {
            err(
                "ferro.model.no_matching_type",
                format!(
                    "no {} type is {:.1} mm thick; create one first",
                    if wall { "wall" } else { "floor" },
                    thickness
                ),
            )
            .on(e.id)
        })
}

/// A straight wall recognised from a solid.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognisedWall {
    /// Centre line at the base (z = `z0`).
    pub start: Point3,
    pub end: Point3,
    pub thickness: f64,
    pub z0: f64,
    pub height: f64,
}

/// A floor recognised from a solid.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognisedFloor {
    pub outer: Vec<Point2>,
    pub holes: Vec<Vec<Point2>>,
    /// Elevation of the top face.
    pub top: f64,
    pub thickness: f64,
}

/// Horizontal faces (bottom, top) of a closed solid whose other faces are
/// all vertical: a vertical prism.
fn prism_caps(body: &Body) -> Option<(crate::topo::FaceId, crate::topo::FaceId)> {
    body.validate_solid().ok()?;
    let (mut top, mut bottom) = (None, None);
    for f in body.face_ids() {
        let n = body.face_normal(f).ok()?.as_vec();
        if n.z > 1.0 - tol::ANGLE_TOL {
            if top.replace(f).is_some() {
                return None;
            }
        } else if n.z < -1.0 + tol::ANGLE_TOL {
            if bottom.replace(f).is_some() {
                return None;
            }
        } else if n.z.abs() > tol::ANGLE_TOL {
            return None;
        }
    }
    Some((bottom?, top?))
}

fn face_z(body: &Body, f: crate::topo::FaceId) -> Option<f64> {
    let face = body.face(f).ok()?;
    body.loop_points(face.outer).first().map(|p| p.z)
}

/// Recognise a straight wall: a box with horizontal top and bottom and a
/// rectangular footprint. The longer side is the wall direction.
pub fn recognise_wall(body: &Body) -> Option<RecognisedWall> {
    let (bottom, top) = prism_caps(body)?;
    let face = body.face(bottom).ok()?;
    if !face.holes.is_empty() {
        return None;
    }
    let p = body.loop_points(face.outer);
    let [a, b, c, d] = <[Point3; 4]>::try_from(p).ok()?;
    let (ab, bc) = (b - a, c - b);
    // Rectangle: right angles and opposite sides equal.
    if ab.dot(bc).abs() > tol::VERTEX_TOL * (ab.length() + bc.length())
        || !(c + (a - b)).is_almost_equal(d)
    {
        return None;
    }
    let (z0, z1) = (face_z(body, bottom)?, face_z(body, top)?);
    let mid = |p: Point3, q: Point3| p + (q - p) * 0.5;
    // Centre line joins the middles of the short sides.
    let (start, end, thickness) = if ab.length() >= bc.length() {
        (mid(d, a), mid(b, c), bc.length())
    } else {
        (mid(a, b), mid(c, d), ab.length())
    };
    if thickness < tol::SHORT_CURVE_TOL || z1 - z0 < tol::SHORT_CURVE_TOL {
        return None;
    }
    Some(RecognisedWall {
        start: Point3::new(start.x, start.y, z0),
        end: Point3::new(end.x, end.y, z0),
        thickness,
        z0,
        height: z1 - z0,
    })
}

/// Recognise a floor: a vertical prism; the outline and holes are those of
/// its top face.
pub fn recognise_floor(body: &Body) -> Option<RecognisedFloor> {
    let (bottom, top) = prism_caps(body)?;
    let face = body.face(top).ok()?;
    let flat = |l| -> Vec<Point2> {
        body.loop_points(l)
            .into_iter()
            .map(|p| Point2::new(p.x, p.y))
            .collect()
    };
    let (zb, zt) = (face_z(body, bottom)?, face_z(body, top)?);
    if zt - zb < tol::SHORT_CURVE_TOL {
        return None;
    }
    Some(RecognisedFloor {
        outer: flat(face.outer),
        holes: face.holes.iter().map(|h| flat(*h)).collect(),
        top: zt,
        thickness: zt - zb,
    })
}
