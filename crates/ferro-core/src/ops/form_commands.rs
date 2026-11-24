//! SketchUp-style editing of Forms (milestone 4): drawing on faces, circles,
//! offset, stretching vertices / edges / faces, scaling and erasing edges.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::algo::{imprint_face, BooleanOp};
use crate::geom::{polygon, Plane};
use crate::math::{tol, Length, Point3, Transform, UnitVec3, Vec3};
use crate::model::{Category, ElementBody, ElementId, Form};
use crate::topo::{Body, EdgeId, FaceId, FaceSpec, VertexId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

fn err(code: &'static str, msg: impl Into<String>) -> CommandError {
    CommandError::new(code, msg)
}

fn form_body(tx: &Transaction, id: ElementId) -> Result<Body, CommandError> {
    tx.element(id)?
        .as_form()
        .map(|f| f.body.clone())
        .ok_or_else(|| err("ferro.model.not_a_form", format!("{id} is not a Form")).on(id))
}

fn store(tx: &mut Transaction, id: ElementId, body: Body) -> Result<(), CommandError> {
    tx.modify(id, |e| {
        e.body = ElementBody::Form(Form::new(body));
        Ok(())
    })
}

/// Points of a regular polygon (a circle drawn with `segments` sides).
pub fn circle_points(center: Point3, normal: UnitVec3, radius: f64, segments: u32) -> Vec<Point3> {
    let plane = Plane::from_origin_normal(center, normal);
    let (x, y) = (plane.x_axis().as_vec(), plane.y_axis().as_vec());
    (0..segments)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / segments as f64;
            center + x * (radius * a.cos()) + y * (radius * a.sin())
        })
        .collect()
}

// ------------------------------------------------------------------ imprint

/// Draw on a face of a Form: the polyline `points` (closed or open) splits
/// the face (SketchUp's sticky geometry). Returns the faces bounded by the
/// new lines, ready to push / pull.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImprintFace {
    pub element: ElementId,
    pub face: FaceId,
    pub points: Vec<Point3>,
    #[serde(default)]
    pub closed: bool,
}

impl Command for ImprintFace {
    const NAME: &'static str = "imprint_face";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let mut body = form_body(tx, self.element)?;
        let (imp, lineage) = imprint_face(&mut body, self.face, &self.points, self.closed)
            .map_err(|e| CommandError::from(e).on(self.element))?;
        store(tx, self.element, body)?;
        Ok(
            json!({"element": self.element, "faces": imp.faces, "new_faces": imp.new_faces, "lineage": lineage}),
        )
    }
}

// ------------------------------------------------------------------ circle

/// Draw a circle (as `segments` edges, default 24). With `element` + `face`
/// it is drawn on that face; otherwise it becomes a new face. `normal`
/// defaults to up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCircle {
    pub center: Point3,
    pub radius: Length,
    #[serde(default)]
    pub normal: Option<Vec3>,
    #[serde(default)]
    pub segments: Option<u32>,
    #[serde(default)]
    pub element: Option<ElementId>,
    #[serde(default)]
    pub face: Option<FaceId>,
    #[serde(default)]
    pub level: Option<ElementId>,
    #[serde(default)]
    pub name: Option<String>,
}

impl Command for CreateCircle {
    const NAME: &'static str = "create_circle";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let r = self.radius.mm();
        if r < tol::SHORT_CURVE_TOL {
            return Err(err(
                "ferro.geom.short_curve",
                "radius must be at least 0.5 mm",
            ));
        }
        let segments = self.segments.unwrap_or(24).clamp(3, 360);
        let normal = match (self.element, self.face) {
            (Some(e), Some(f)) => form_body(tx, e)?
                .face_normal(f)
                .map_err(|x| CommandError::from(x).on(e))?,
            _ => self
                .normal
                .unwrap_or(Vec3::Z)
                .normalize()
                .ok_or_else(|| err("ferro.geom.degenerate", "normal must not be zero"))?,
        };
        let pts = circle_points(self.center, normal, r, segments);
        match (self.element, self.face) {
            (Some(element), Some(face)) => ImprintFace {
                element,
                face,
                points: pts,
                closed: true,
            }
            .execute(tx),
            _ => {
                let idx: Vec<usize> = (0..pts.len()).collect();
                let (body, faces) = Body::from_polygons(&pts, &[idx])?;
                let id = tx.create(
                    Category::Forms,
                    self.name.clone(),
                    self.level,
                    ElementBody::Form(Form::new(body)),
                )?;
                Ok(json!({"element": id, "face": faces[0]}))
            }
        }
    }
}

// ------------------------------------------------------------------ offset

/// Offset the outer boundary of a face inwards by `distance` (SketchUp
/// Offset): the inner copy splits the face. Negative distances are not
/// supported on faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OffsetFace {
    pub element: ElementId,
    pub face: FaceId,
    pub distance: Length,
}

/// Inset a planar polygon (counter-clockwise about `normal`) by `d`.
pub fn inset_polygon(pts: &[Point3], normal: UnitVec3, d: f64) -> Option<Vec<Point3>> {
    let plane = Plane::from_origin_normal(pts[0], normal);
    let p: Vec<crate::math::Point2> = pts.iter().map(|q| plane.to_local(*q)).collect();
    let n = p.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = p[(i + n - 1) % n];
        let cur = p[i];
        let next = p[(i + 1) % n];
        let e1 = cur - prev;
        let e2 = next - cur;
        let (l1, l2) = (e1.length(), e2.length());
        if l1 < 1e-9 || l2 < 1e-9 {
            return None;
        }
        // Inward normals (left of the edge for a CCW polygon).
        let n1 = crate::math::Vec2::new(-e1.y / l1, e1.x / l1);
        let n2 = crate::math::Vec2::new(-e2.y / l2, e2.x / l2);
        let a1 = prev + n1 * d;
        let a2 = cur + n2 * d;
        let den = e1.perp_dot(e2);
        let q = if den.abs() < 1e-9 * l1 * l2 {
            cur + n1 * d // collinear
        } else {
            let t = (a2 - a1).perp_dot(e2) / den;
            a1 + e1 * t
        };
        out.push(plane.to_world(q));
    }
    // The inset must keep the orientation (no flip) and stay simple.
    // An inset past the middle of a convex face mirrors it through its
    // centre: same orientation, but every edge reversed. Each edge must keep
    // its direction.
    let before = polygon::polygon_normal(pts)?;
    let after = polygon::polygon_normal(&out)?;
    let edges_kept = (0..n).all(|i| {
        let j = (i + 1) % n;
        (out[j] - out[i]).dot(pts[j] - pts[i]) > 0.0
    });
    (before.dot(after) > 0.0 && edges_kept).then_some(out)
}

impl Command for OffsetFace {
    const NAME: &'static str = "offset_face";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let d = self.distance.mm();
        if d < tol::SHORT_CURVE_TOL {
            return Err(err(
                "ferro.geom.short_curve",
                "offset distance must be at least 0.5 mm, inwards",
            ));
        }
        let body = form_body(tx, self.element)?;
        let n = body
            .face_normal(self.face)
            .map_err(|x| CommandError::from(x).on(self.element))?;
        let f = body
            .face(self.face)
            .map_err(|x| CommandError::from(x).on(self.element))?;
        let outer = body.loop_points(f.outer);
        let inset = inset_polygon(&outer, n, d).ok_or_else(|| {
            err(
                "ferro.geom.offset_too_large",
                "the offset is larger than the face",
            )
        })?;
        ImprintFace {
            element: self.element,
            face: self.face,
            points: inset,
            closed: true,
        }
        .execute(tx)
    }
}

// ------------------------------------------------------------------ stretch

/// Move some vertices, edges or faces of a Form (SketchUp Move on selected
/// geometry): connected faces stretch. Faces must stay planar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveGeometry {
    pub element: ElementId,
    #[serde(default)]
    pub vertices: Vec<VertexId>,
    #[serde(default)]
    pub edges: Vec<EdgeId>,
    #[serde(default)]
    pub faces: Vec<FaceId>,
    pub vector: Vec3,
}

/// Vertices of the given edges and faces (plus explicit vertices).
pub fn geometry_vertices(
    body: &Body,
    vertices: &[VertexId],
    edges: &[EdgeId],
    faces: &[FaceId],
) -> Result<BTreeSet<VertexId>, CommandError> {
    let mut set: BTreeSet<VertexId> = vertices.iter().copied().collect();
    for f in faces {
        let face = body.face(*f)?;
        for l in std::iter::once(face.outer).chain(face.holes.iter().copied()) {
            set.extend(body.loop_vertices(l));
        }
    }
    if !edges.is_empty() {
        let pos: Vec<(EdgeId, Point3, Point3)> =
            body.edges().filter(|(e, _, _)| edges.contains(e)).collect();
        if pos.len() != edges.len() {
            return Err(err("ferro.topo.not_found", "an edge does not exist"));
        }
        for (_, a, b) in pos {
            for (v, p) in body.vertices() {
                if p.distance_to(a) <= tol::VERTEX_TOL || p.distance_to(b) <= tol::VERTEX_TOL {
                    set.insert(v);
                }
            }
        }
    }
    Ok(set)
}

impl Command for MoveGeometry {
    const NAME: &'static str = "move_geometry";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let mut body = form_body(tx, self.element)?;
        let set = geometry_vertices(&body, &self.vertices, &self.edges, &self.faces)
            .map_err(|e| e.on(self.element))?;
        if set.is_empty() {
            return Err(err(
                "ferro.ops.nothing_selected",
                "give vertices, edges or faces",
            ));
        }
        let before = body.clone();
        let moves: Vec<(VertexId, Point3)> = set
            .iter()
            .map(|v| {
                Ok((
                    *v,
                    body.point(*v)
                        .ok_or_else(|| err("ferro.topo.not_found", format!("vertex {v}")))?
                        + self.vector,
                ))
            })
            .collect::<Result<_, CommandError>>()?;
        body.move_vertices(&moves)?;
        // Every face must stay flat and keep a positive area.
        for f in body.face_ids().collect::<Vec<_>>() {
            let pts = body.face_points(f)?;
            let Some(n) = polygon::polygon_normal(&pts) else {
                return Err(err("ferro.geom.degenerate", "a face would collapse").on(self.element));
            };
            let plane = Plane::from_origin_normal(pts[0], n);
            if pts
                .iter()
                .any(|p| plane.signed_distance_to(*p).abs() > tol::VERTEX_TOL)
            {
                return Err(err(
                    "ferro.geom.not_planar",
                    "this move would bend a face; move whole faces or edges in their plane",
                )
                .on(self.element));
            }
        }
        if before.is_closed() && body.volume() <= 0.0 {
            return Err(err(
                "ferro.algo.collapse",
                "this move turns the solid inside out",
            )
            .on(self.element));
        }
        store(tx, self.element, body)?;
        Ok(json!({"element": self.element, "moved_vertices": set.len()}))
    }
}

// ------------------------------------------------------------------ scale

/// Scale Forms about `center` by `factor` (uniform) or per axis `factors`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScaleElements {
    pub elements: Vec<ElementId>,
    pub center: Point3,
    #[serde(default)]
    pub factor: Option<f64>,
    #[serde(default)]
    pub factors: Option<[f64; 3]>,
}

impl Command for ScaleElements {
    const NAME: &'static str = "scale_elements";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let [sx, sy, sz] = match (self.factors, self.factor) {
            (Some(f), _) => f,
            (None, Some(f)) => [f, f, f],
            _ => return Err(err("ferro.ops.invalid_scale", "give factor or factors")),
        };
        if [sx, sy, sz]
            .iter()
            .any(|f| !f.is_finite() || f.abs() < 1e-6)
        {
            return Err(err(
                "ferro.ops.invalid_scale",
                "scale factors must be non-zero",
            ));
        }
        let c = self.center.to_vec();
        let t = Transform::translation(-c)
            .then(&Transform {
                basis_x: Vec3::new(sx, 0.0, 0.0),
                basis_y: Vec3::new(0.0, sy, 0.0),
                basis_z: Vec3::new(0.0, 0.0, sz),
                origin: Vec3::ZERO,
            })
            .then(&Transform::translation(c));
        for id in &self.elements {
            let mut body = form_body(tx, *id).map_err(|e| {
                if e.code == "ferro.model.not_a_form" {
                    err("ferro.ops.not_scalable", "only Forms can be scaled; walls and floors change through their parameters").on(*id)
                } else {
                    e
                }
            })?;
            body.transform(&t);
            store(tx, *id, body)?;
        }
        Ok(json!({"elements": self.elements}))
    }
}

// ------------------------------------------------------------------ erase

/// Erase edges of a Form. An edge between two faces in the same plane merges
/// them (healing); any other edge takes its faces with it, leaving the Form
/// open (SketchUp Eraser).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EraseEdges {
    pub element: ElementId,
    pub edges: Vec<EdgeId>,
}

impl Command for EraseEdges {
    const NAME: &'static str = "erase_edges";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let body = form_body(tx, self.element)?;
        let ends: Vec<(Point3, Point3)> = body
            .edges()
            .filter(|(e, _, _)| self.edges.contains(e))
            .map(|(_, a, b)| (a, b))
            .collect();
        if ends.len() != self.edges.len() {
            return Err(err("ferro.topo.not_found", "an edge does not exist").on(self.element));
        }
        let vid = |p: Point3| {
            body.vertices()
                .find(|(_, q)| q.distance_to(p) <= tol::VERTEX_TOL)
                .map(|(v, _)| v)
        };
        let mut specs = body.face_specs();
        let mut merged = 0;
        let mut removed = 0;
        for (a, b) in ends {
            let (Some(va), Some(vb)) = (vid(a), vid(b)) else {
                continue;
            };
            let holds = |s: &FaceSpec, x: VertexId, y: VertexId| {
                let n = s.outer.len();
                (0..n).any(|i| s.outer[i] == x && s.outer[(i + 1) % n] == y)
            };
            let fa = specs.iter().position(|s| holds(s, va, vb));
            let fb = specs.iter().position(|s| holds(s, vb, va));
            match (fa, fb) {
                (Some(i), Some(j)) if i != j && same_plane(&body, &specs[i], &specs[j]) => {
                    // Splice: A' = A from vb to va, B' = B from va to vb.
                    let rot = |lp: &[VertexId], start: VertexId| {
                        let k = lp.iter().position(|v| *v == start).expect("on loop");
                        lp[k..].iter().chain(&lp[..k]).copied().collect::<Vec<_>>()
                    };
                    let a2 = rot(&specs[i].outer, vb);
                    let b2 = rot(&specs[j].outer, va);
                    let mut lp = a2.clone();
                    lp.extend_from_slice(&b2[1..b2.len() - 1]);
                    let mut holes = specs[i].holes.clone();
                    holes.extend(specs[j].holes.clone());
                    specs[i].outer = lp;
                    specs[i].holes = holes;
                    specs.remove(j);
                    merged += 1;
                }
                (fa, fb) => {
                    let mut drop: Vec<usize> = fa.into_iter().chain(fb).collect();
                    drop.sort_unstable();
                    drop.dedup();
                    for i in drop.into_iter().rev() {
                        specs.remove(i);
                        removed += 1;
                    }
                }
            }
        }
        let mut out = body.clone();
        let verts = out.vertex_map();
        out.rebuild(verts, specs)
            .map_err(|e| CommandError::from(e).on(self.element))?;
        if out.face_count() == 0 {
            tx.delete(self.element)?;
            return Ok(json!({"element": self.element, "deleted": true}));
        }
        store(tx, self.element, out)?;
        Ok(json!({"element": self.element, "merged": merged, "removed_faces": removed}))
    }
}

/// Erase faces of a Form (the Form becomes open, like deleting a face in
/// SketchUp). Erasing every face deletes the Form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EraseFaces {
    pub element: ElementId,
    pub faces: Vec<FaceId>,
}

impl Command for EraseFaces {
    const NAME: &'static str = "erase_faces";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let mut body = form_body(tx, self.element)?;
        for f in &self.faces {
            body.face(*f)
                .map_err(|e| CommandError::from(e).on(self.element))?;
        }
        let specs: Vec<FaceSpec> = body
            .face_specs()
            .into_iter()
            .filter(|s| s.id.is_none_or(|id| !self.faces.contains(&id)))
            .collect();
        if specs.is_empty() {
            tx.delete(self.element)?;
            return Ok(json!({"element": self.element, "deleted": true}));
        }
        let verts = body.vertex_map();
        body.rebuild(verts, specs)
            .map_err(|e| CommandError::from(e).on(self.element))?;
        store(tx, self.element, body)?;
        Ok(json!({"element": self.element, "erased": self.faces.len()}))
    }
}

// ------------------------------------------------------------------ booleans

/// Subtract one Form from another (solid boolean difference through the
/// geometry kernel, ADR 0007): the target's body becomes target − cutter in
/// one transaction, and the cutter is deleted unless `keep_cutter`. Faces of
/// the target that survive keep their ids (D-25); the output's `lineage`
/// says which were kept, created and deleted, and `faces` maps every result
/// face to the input faces it came from (`from_a` = target, `from_b` =
/// cutter). Both Forms must be closed solids with flat faces (mm).
///
/// Fails with `ferro.kernel.unavailable` (session without a kernel),
/// `ferro.ops.same_element`, `ferro.model.not_a_form`, or the kernel's codes
/// (`ferro.kernel.not_solid`, `ferro.kernel.empty_result` when nothing is
/// left, `ferro.kernel.non_manifold_result`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtractForms {
    pub target: ElementId,
    pub cutter: ElementId,
    #[serde(default)]
    pub keep_cutter: bool,
}

impl Command for SubtractForms {
    const NAME: &'static str = "subtract_forms";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if self.target == self.cutter {
            return Err(err(
                "ferro.ops.same_element",
                "the target and the cutter must be different Forms",
            )
            .on(self.target));
        }
        let a = form_body(tx, self.target)?;
        let b = form_body(tx, self.cutter)?;
        let r = tx
            .kernel()?
            .boolean(BooleanOp::Difference, &a, &b)
            .map_err(|e| CommandError::from(e).on(self.target).on(self.cutter))?;
        store(tx, self.target, r.body)?;
        if !self.keep_cutter {
            tx.delete(self.cutter)?;
        }
        Ok(json!({
            "element": self.target,
            "cutter_deleted": !self.keep_cutter,
            "lineage": r.lineage,
            "faces": r.faces,
        }))
    }
}

/// Shared body of [`UniteForms`] and [`IntersectForms`]: `target` becomes
/// `target op other` through the kernel; `other` is deleted unless `keep`.
fn boolean_forms(
    tx: &mut Transaction,
    op: BooleanOp,
    target: ElementId,
    other: ElementId,
    keep: bool,
) -> Result<CommandOutput, CommandError> {
    if target == other {
        return Err(err("ferro.ops.same_element", "the two Forms must be different").on(target));
    }
    let a = form_body(tx, target)?;
    let b = form_body(tx, other)?;
    let r = tx
        .kernel()?
        .boolean(op, &a, &b)
        .map_err(|e| CommandError::from(e).on(target).on(other))?;
    store(tx, target, r.body)?;
    if !keep {
        tx.delete(other)?;
    }
    Ok(json!({
        "element": target,
        "other_deleted": !keep,
        "lineage": r.lineage,
        "faces": r.faces,
    }))
}

/// Unite two Forms (solid boolean union through the geometry kernel, ADR
/// 0007; Revit `BooleanOperationsType.Union`): the target's body becomes
/// target ∪ other in one transaction, and `other` is deleted unless
/// `keep_other`. Faces of the target that survive keep their ids (D-25);
/// `lineage` and `faces` as for [`SubtractForms`] (`from_b` = other).
/// Disjoint Forms give one body of several shells. Both Forms must be
/// closed solids with flat faces (mm).
///
/// Fails with `ferro.kernel.unavailable`, `ferro.ops.same_element`,
/// `ferro.model.not_a_form`, or the kernel's codes
/// (`ferro.kernel.not_solid`, `ferro.kernel.non_manifold_result` for
/// solids touching only along an edge, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UniteForms {
    pub target: ElementId,
    pub other: ElementId,
    #[serde(default)]
    pub keep_other: bool,
}

impl Command for UniteForms {
    const NAME: &'static str = "unite_forms";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        boolean_forms(
            tx,
            BooleanOp::Union,
            self.target,
            self.other,
            self.keep_other,
        )
    }
}

/// Intersect two Forms (solid boolean intersection through the geometry
/// kernel, ADR 0007; Revit `BooleanOperationsType.Intersect`): the
/// target's body becomes what lies in both, and `other` is deleted unless
/// `keep_other`. Lineage as for [`UniteForms`]. Both Forms must be closed
/// solids with flat faces (mm).
///
/// Fails with `ferro.kernel.empty_result` when they do not overlap, and
/// otherwise as [`UniteForms`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntersectForms {
    pub target: ElementId,
    pub other: ElementId,
    #[serde(default)]
    pub keep_other: bool,
}

impl Command for IntersectForms {
    const NAME: &'static str = "intersect_forms";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        boolean_forms(
            tx,
            BooleanOp::Intersection,
            self.target,
            self.other,
            self.keep_other,
        )
    }
}

fn same_plane(body: &Body, a: &FaceSpec, b: &FaceSpec) -> bool {
    let pa: Vec<Point3> = a.outer.iter().filter_map(|v| body.point(*v)).collect();
    let pb: Vec<Point3> = b.outer.iter().filter_map(|v| body.point(*v)).collect();
    match (polygon::polygon_normal(&pa), polygon::polygon_normal(&pb)) {
        (Some(na), Some(nb)) if na.dot(nb) > 1.0 - 1e-9 => {
            let plane = Plane::from_origin_normal(pa[0], na);
            pb.iter().all(|p| plane.contains(*p))
        }
        _ => false,
    }
}
