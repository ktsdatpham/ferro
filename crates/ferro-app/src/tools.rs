//! Drawing tools (D-72): app-side state machines. Each tool previews by
//! evaluating a command on a copy of the document, and commits exactly one
//! named command, so a human and an AI agent produce identical history.
//!
//! Drawing tools work on a **draw plane** picked by the first click: the face
//! of a Form under the cursor (any orientation; the shape is imprinted on it,
//! like SketchUp's sticky geometry), otherwise the active level.

use ferro_core::model::bim::OpeningKind;
use ferro_core::model::generate;
use ferro_core::model::{keys, Document, ElementId, ParamValue};
use ferro_core::ops::bim_commands::*;
use ferro_core::ops::commands::*;
use ferro_core::ops::form_commands::*;
use ferro_core::topo::{EdgeId, FaceId, VertexId};
use ferro_core::{Angle, AnyCommand, Length, Point2, Point3, UnitVec3, Vec3};
use ferro_snap::{Ray, SnapKind, SnapResult};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polygon,
    PushPull,
    Offset,
    Move,
    Rotate,
    Scale,
    Eraser,
    Paint,
    Tape,
    Section,
    Wall,
    Floor,
    Door,
    Window,
    /// Solid tools (two picks: target, then the other Form): booleans
    /// through the kernel (ADR 0007) and the non-destructive cut (M5-5.3).
    Subtract,
    Unite,
    Intersect,
    /// Cut (keep cutter): a `cut` relationship row; the cutter stays.
    CutGeometry,
}

impl ToolKind {
    pub const ALL: [ToolKind; 23] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Arc,
        ToolKind::Polygon,
        ToolKind::PushPull,
        ToolKind::Offset,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::Scale,
        ToolKind::Eraser,
        ToolKind::Paint,
        ToolKind::Tape,
        ToolKind::Section,
        ToolKind::Wall,
        ToolKind::Floor,
        ToolKind::Door,
        ToolKind::Window,
        ToolKind::Subtract,
        ToolKind::Unite,
        ToolKind::Intersect,
        ToolKind::CutGeometry,
    ];
    /// Tools that draw a chain of clicked points.
    pub fn is_chain(self) -> bool {
        matches!(
            self,
            ToolKind::Line | ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor
        )
    }
    /// The solid tools: pick a target, then the other element.
    pub fn is_solid(self) -> bool {
        matches!(
            self,
            ToolKind::Subtract | ToolKind::Unite | ToolKind::Intersect | ToolKind::CutGeometry
        )
    }
    pub fn is_bim(self) -> bool {
        matches!(
            self,
            ToolKind::Wall | ToolKind::Floor | ToolKind::Door | ToolKind::Window
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Line => "Line",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::Circle => "Circle",
            ToolKind::Arc => "Arc",
            ToolKind::Tape => "Tape Measure",
            ToolKind::Section => "Section Plane",
            ToolKind::Polygon => "Polygon",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Offset => "Offset",
            ToolKind::Move => "Move",
            ToolKind::Rotate => "Rotate",
            ToolKind::Scale => "Scale",
            ToolKind::Eraser => "Eraser",
            ToolKind::Paint => "Paint",
            ToolKind::Wall => "Wall",
            ToolKind::Floor => "Floor",
            ToolKind::Door => "Door",
            ToolKind::Window => "Window",
            ToolKind::Subtract => "Subtract",
            ToolKind::Unite => "Unite",
            ToolKind::Intersect => "Intersect",
            ToolKind::CutGeometry => "Cut (keep cutter)",
        }
    }
}

/// The command of a solid tool: `target` picked first, then `other`.
pub fn solid_command(kind: ToolKind, target: ElementId, other: ElementId) -> AnyCommand {
    use ferro_core::model::RelationshipKind;
    use ferro_core::ops::relationship_commands::CreateRelationship;
    match kind {
        ToolKind::Unite => UniteForms {
            target,
            other,
            keep_other: false,
        }
        .into(),
        ToolKind::Intersect => IntersectForms {
            target,
            other,
            keep_other: false,
        }
        .into(),
        ToolKind::CutGeometry => CreateRelationship {
            kind: RelationshipKind::Cut,
            elements: vec![other, target],
            phase: None,
        }
        .into(),
        _ => SubtractForms {
            target,
            cutter: other,
            keep_cutter: false,
        }
        .into(),
    }
}

/// What the tool sees each frame.
pub struct ToolCtx<'a> {
    pub doc: &'a Document,
    pub snap: Option<SnapResult>,
    pub ray: Ray,
    pub selection: &'a HashSet<ElementId>,
    /// Active level (id, elevation mm).
    pub level: Option<(ElementId, f64)>,
    /// Ctrl / Alt held: copy while moving.
    pub copy: bool,
    /// Cursor distance to the chain's first point (px), if any.
    pub near_first_px: Option<f32>,
    /// Material the Paint tool applies.
    pub material: Option<ElementId>,
    /// The Form being edited (double-clicked), and its picked parts.
    pub editing: Option<ElementId>,
    pub sub: &'a [SubPick],
}

/// Part of a Form picked while editing it (SketchUp group editing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubPick {
    Face(FaceId),
    Edge(EdgeId),
    Vertex(VertexId),
}

/// The vertex, edge or face of `element` under the snap.
pub fn pick_sub(doc: &Document, element: ElementId, s: &SnapResult) -> Option<SubPick> {
    if s.element != Some(element) {
        return None;
    }
    let body = doc.get(element)?.geometry()?;
    match s.kind {
        SnapKind::Endpoint => body
            .vertices()
            .find(|(_, p)| p.distance_to(s.point) <= 0.5)
            .map(|(v, _)| SubPick::Vertex(v)),
        SnapKind::OnEdge | SnapKind::Midpoint => body
            .edges()
            .find(|(_, a, b)| {
                let ab = *b - *a;
                let t = ((s.point - *a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                (*a + ab * t).distance_to(s.point) <= 0.5
            })
            .map(|(e, _, _)| SubPick::Edge(e)),
        _ => s.face.map(SubPick::Face),
    }
}

pub enum SelectOp {
    Set(Vec<ElementId>),
    Toggle(ElementId),
    Clear,
}

pub enum ToolEvent {
    None,
    /// Show a message (measurements).
    Message(String),
    /// Cut the view with a section plane: geometry on the side `normal`
    /// points to is not drawn (view state, not a document change).
    Section(SectionPlane),
    Commit(AnyCommand),
    Select(SelectOp),
}

/// A section plane: a point on it and the normal of the side cut away
/// (the same type views store, M5-22).
pub use ferro_core::model::SectionPlane;

/// Parameter along the line `start + t·dir` of the point closest to `ray`.
/// `None` when the ray runs along the line.
fn line_param(start: Point3, dir: UnitVec3, ray: &Ray) -> Option<f64> {
    let u = dir.as_vec();
    let r = ray.dir.as_vec();
    let w = ray.origin - start;
    let (ur, rr, uw, rw) = (u.dot(r), r.dot(r), u.dot(w), r.dot(w));
    let den = rr - ur * ur;
    if den.abs() < 1e-9 {
        return None;
    }
    Some((rr * uw - ur * rw) / den)
}

/// A plane to draw on, with an in-plane frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawPlane {
    pub origin: Point3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: UnitVec3,
    /// The Form face this plane belongs to (shapes are imprinted on it).
    pub face: Option<(ElementId, FaceId)>,
    /// Horizontal at the active level (shapes are placed on that level).
    pub level: Option<ElementId>,
}

impl DrawPlane {
    fn horizontal(z: f64, level: Option<ElementId>) -> Self {
        DrawPlane {
            origin: Point3::new(0.0, 0.0, z),
            u: Vec3::X,
            v: Vec3::Y,
            n: UnitVec3::Z,
            face: None,
            level,
        }
    }

    fn on_face(p: Point3, n: UnitVec3, face: Option<(ElementId, FaceId)>) -> Self {
        // u: world X projected into the plane (world Y for faces facing X).
        let a = if n.as_vec().x.abs() > 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let nv = n.as_vec();
        let u = (a - nv * a.dot(nv))
            .normalize()
            .map(|x| x.as_vec())
            .unwrap_or(Vec3::X);
        let v = nv.cross(u);
        DrawPlane {
            origin: p,
            u,
            v,
            n,
            face,
            level: None,
        }
    }

    pub fn is_horizontal(&self) -> bool {
        self.n.as_vec().z.abs() > 1.0 - 1e-9
    }

    fn local(&self, p: Point3) -> (f64, f64) {
        let d = p - self.origin;
        (d.dot(self.u), d.dot(self.v))
    }

    fn at(&self, a: f64, b: f64) -> Point3 {
        self.origin + self.u * a + self.v * b
    }

    fn project(&self, p: Point3) -> Point3 {
        let d = p - self.origin;
        p - self.n.as_vec() * d.dot(self.n.as_vec())
    }
}

#[derive(Debug, Clone)]
struct PushPullDrag {
    element: ElementId,
    face: FaceId,
    start: Point3,
    normal: UnitVec3,
}

#[derive(Debug, Clone)]
struct OffsetPick {
    element: ElementId,
    face: FaceId,
    boundary: Vec<Point3>,
    plane: DrawPlane,
}

#[derive(Debug, Clone)]
pub struct ToolState {
    pub kind: ToolKind,
    plane: Option<DrawPlane>,
    rect_first: Option<Point3>,
    circle_center: Option<Point3>,
    poly: Vec<Point3>,
    drag: Option<PushPullDrag>,
    offset: Option<OffsetPick>,
    move_base: Option<Point3>,
    move_elems: Vec<ElementId>,
    pivot: Option<Point3>,
    reference: Option<Point3>,
    /// Last push / pull distance (double-click repeats it, as in SketchUp).
    pub last_push: Option<f64>,
    /// Move started by Paste: always copies.
    paste: bool,
    /// The chain started on an edge or corner of this Form: its plane is
    /// decided by the second point.
    pending_face_of: Option<ElementId>,
    /// Move of parts of a Form (stretch) instead of whole elements.
    geom: Option<(ElementId, Vec<SubPick>)>,
    /// Arc: start and end points (the third click sets the bulge).
    arc: Vec<Point3>,
    /// Tape measure start.
    tape: Option<Point3>,
    /// Section Plane: the clicked face (point and outward normal).
    section: Option<SectionPlane>,
    /// Solid tools: the target picked first.
    solid_target: Option<ElementId>,
}

fn is_point_snap(k: SnapKind) -> bool {
    matches!(
        k,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Origin
            | SnapKind::OnEdge
            | SnapKind::OnAxis(_)
            | SnapKind::Parallel
            | SnapKind::Perpendicular
    )
}

/// Point on `plane` under the cursor: a point snap projected onto the plane,
/// else where the cursor ray meets it.
fn on_plane(ctx: &ToolCtx, plane: &DrawPlane) -> Option<Point3> {
    if let Some(s) = ctx.snap.filter(|s| is_point_snap(s.kind)) {
        return Some(plane.project(s.point));
    }
    let n = plane.n.as_vec();
    let d = ctx.ray.dir.as_vec();
    let den = d.dot(n);
    if den.abs() > 1e-9 {
        let t = (plane.origin - ctx.ray.origin).dot(n) / den;
        if t > 0.0 {
            return Some(ctx.ray.at(t));
        }
    }
    ctx.snap.map(|s| plane.project(s.point))
}

/// The face of Form `element` whose plane holds every point of `pts`, with
/// the segment's midpoint inside it (how SketchUp decides which face a line
/// started on an edge belongs to).
fn face_containing(
    doc: &Document,
    element: ElementId,
    pts: &[Point3],
) -> Option<(FaceId, UnitVec3)> {
    let body = doc.get(element)?.geometry()?;
    let mid = Point3::centroid(pts)?;
    body.face_ids().find_map(|f| {
        let n = body.face_normal(f).ok()?;
        let fp = body.face_points(f).ok()?;
        let plane = ferro_core::geom::Plane::from_origin_normal(fp[0], n);
        if pts.iter().any(|p| plane.signed_distance_to(*p).abs() > 0.5) {
            return None;
        }
        let poly: Vec<Point2> = fp.iter().map(|q| plane.to_local(*q)).collect();
        ferro_core::geom::predicates::point_in_polygon(plane.to_local(mid), &poly).then_some((f, n))
    })
}

/// `true` if `p` lies on an edge of face `f` of `element` (within 1 mm).
fn on_face_boundary(doc: &Document, element: ElementId, f: FaceId, p: Point3) -> bool {
    let Some(body) = doc.get(element).and_then(|e| e.geometry()) else {
        return false;
    };
    let Ok(face) = body.face(f) else { return false };
    std::iter::once(face.outer)
        .chain(face.holes.iter().copied())
        .any(|l| {
            let pts = body.loop_points(l);
            (0..pts.len()).any(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                (a + ab * t).distance_to(p) <= 1.0
            })
        })
}

fn is_form(doc: &Document, e: ElementId) -> bool {
    doc.get(e).is_some_and(|x| x.as_form().is_some())
}

/// Points of the arc from `a` to `b` (in `plane`), about every 7.5°. As in
/// SketchUp, `m` sets the bulge: the arc's apex is as far from the chord as
/// `m` is, on the same side. `None` when there is no bulge.
fn arc_points(plane: &DrawPlane, a: Point3, b: Point3, m: Point3) -> Option<Vec<Point3>> {
    let (ax, ay) = plane.local(a);
    let (bx, by) = plane.local(b);
    let (px, py) = plane.local(m);
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
    if len < 1e-6 {
        return None;
    }
    // Unit normal of the chord and the signed bulge.
    let (nx, ny) = (-(by - ay) / len, (bx - ax) / len);
    let h = (px - ax) * nx + (py - ay) * ny;
    let (mx, my) = ((ax + bx) / 2.0 + h * nx, (ay + by) / 2.0 + h * ny);
    let d = 2.0 * (ax * (by - my) + bx * (my - ay) + mx * (ay - by));
    if d.abs() < 1e-6 {
        return None;
    }
    let sq = |x: f64, y: f64| x * x + y * y;
    let cx = (sq(ax, ay) * (by - my) + sq(bx, by) * (my - ay) + sq(mx, my) * (ay - by)) / d;
    let cy = (sq(ax, ay) * (mx - bx) + sq(bx, by) * (ax - mx) + sq(mx, my) * (bx - ax)) / d;
    let r = ((ax - cx).powi(2) + (ay - cy).powi(2)).sqrt();
    let ang = |x: f64, y: f64| (y - cy).atan2(x - cx);
    let (t0, t1, tm) = (ang(ax, ay), ang(bx, by), ang(mx, my));
    let tau = std::f64::consts::TAU;
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
    // Sweep from a to b passing through m.
    let sweep = if ccw(t0, tm) <= ccw(t0, t1) {
        ccw(t0, t1)
    } else {
        -(tau - ccw(t0, t1))
    };
    // Even, so the apex is one of the points.
    let n = ((sweep.abs() / 15f64.to_radians()).ceil() as usize).max(1) * 2;
    Some(
        (0..=n)
            .map(|i| {
                let t = t0 + sweep * i as f64 / n as f64;
                plane.at(cx + r * t.cos(), cy + r * t.sin())
            })
            .collect(),
    )
}

/// Closest distance from `p` to the polygon boundary (2D in `plane`).
fn distance_to_boundary(plane: &DrawPlane, p: Point3, boundary: &[Point3]) -> f64 {
    let (px, py) = plane.local(p);
    let q = Point2::new(px, py);
    let n = boundary.len();
    (0..n)
        .map(|i| {
            let (ax, ay) = plane.local(boundary[i]);
            let (bx, by) = plane.local(boundary[(i + 1) % n]);
            let (a, b) = (Point2::new(ax, ay), Point2::new(bx, by));
            let ab = b - a;
            let t = ((q - a).dot(ab) / ab.dot(ab).max(1e-12)).clamp(0.0, 1.0);
            (a + ab * t).distance_to(q)
        })
        .fold(f64::INFINITY, f64::min)
}

impl ToolState {
    pub fn new(kind: ToolKind) -> Self {
        ToolState {
            kind,
            plane: None,
            rect_first: None,
            circle_center: None,
            poly: Vec::new(),
            drag: None,
            offset: None,
            move_base: None,
            move_elems: Vec::new(),
            pivot: None,
            reference: None,
            last_push: None,
            paste: false,
            pending_face_of: None,
            geom: None,
            arc: Vec::new(),
            tape: None,
            section: None,
            solid_target: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.rect_first.is_some()
            || self.circle_center.is_some()
            || !self.arc.is_empty()
            || self.tape.is_some()
            || self.section.is_some()
            || !self.poly.is_empty()
            || self.drag.is_some()
            || self.offset.is_some()
            || self.move_base.is_some()
            || self.pivot.is_some()
            || self.solid_target.is_some()
    }

    pub fn cancel(&mut self) {
        let last = self.last_push;
        *self = ToolState::new(self.kind);
        self.last_push = last;
    }

    pub fn backspace(&mut self) {
        if self.kind.is_chain() {
            self.poly.pop();
            if self.poly.is_empty() {
                self.plane = None;
            }
        }
    }

    /// Anchor for axis inference.
    pub fn anchor(&self) -> Option<Point3> {
        match self.kind {
            k if k.is_chain() => self.poly.last().copied(),
            ToolKind::Move => self.move_base,
            ToolKind::Rotate | ToolKind::Scale => self.reference.or(self.pivot),
            _ => None,
        }
    }

    pub fn polygon_first(&self) -> Option<Point3> {
        self.poly.first().copied()
    }

    /// Elements being moved (excluded from snapping).
    pub fn moving(&self) -> &[ElementId] {
        if self.move_base.is_some() {
            &self.move_elems
        } else {
            &[]
        }
    }

    /// Start a Move-copy of `elements` from `base` (paste).
    pub fn start_paste(&mut self, elements: Vec<ElementId>, base: Point3) {
        *self = ToolState::new(ToolKind::Move);
        self.move_elems = elements;
        self.move_base = Some(base);
        self.paste = true;
    }

    /// Face to highlight under the cursor.
    pub fn hover_face(&self, ctx: &ToolCtx) -> Option<(ElementId, FaceId)> {
        if let Some(d) = &self.drag {
            return Some((d.element, d.face));
        }
        if let Some(o) = &self.offset {
            return Some((o.element, o.face));
        }
        let s = ctx.snap?;
        let (e, f) = (s.element?, s.face?);
        match self.kind {
            ToolKind::PushPull => {
                ctx.doc.get(e)?.geometry()?;
                Some((e, f))
            }
            ToolKind::Offset => is_form(ctx.doc, e).then_some((e, f)),
            ToolKind::Line
            | ToolKind::Rectangle
            | ToolKind::Circle
            | ToolKind::Arc
            | ToolKind::Polygon
                if !self.is_active() && s.kind == SnapKind::OnFace =>
            {
                is_form(ctx.doc, e).then_some((e, f))
            }
            _ => None,
        }
    }

    /// The draw plane the first click would pick.
    fn pick_plane(&self, ctx: &ToolCtx, s: &SnapResult) -> DrawPlane {
        let level = ctx.level;
        if let (Some(e), Some(f), Some(n)) = (s.element, s.face, s.normal) {
            if is_form(ctx.doc, e) && !self.kind.is_bim() {
                return DrawPlane::on_face(s.point, n, Some((e, f)));
            }
            if n.as_vec().z.abs() > 0.999 && !self.kind.is_bim() {
                return DrawPlane::horizontal(s.point.z, None);
            }
        }
        match s.kind {
            SnapKind::Ground => {
                DrawPlane::horizontal(level.map(|l| l.1).unwrap_or(s.point.z), level.map(|l| l.0))
            }
            _ if self.kind.is_bim() => {
                DrawPlane::horizontal(level.map(|l| l.1).unwrap_or(0.0), level.map(|l| l.0))
            }
            _ => {
                let lvl = level.filter(|l| (l.1 - s.point.z).abs() < 1e-6);
                DrawPlane::horizontal(s.point.z, lvl.map(|l| l.0))
            }
        }
    }

    fn rect_points(&self, plane: &DrawPlane, first: Point3, p: Point3) -> Option<[Point3; 4]> {
        let (a0, b0) = plane.local(first);
        let (a1, b1) = plane.local(p);
        if (a1 - a0).abs() < 1.0 || (b1 - b0).abs() < 1.0 {
            return None;
        }
        let mut pts = [
            plane.at(a0, b0),
            plane.at(a1, b0),
            plane.at(a1, b1),
            plane.at(a0, b1),
        ];
        if (a1 - a0) * (b1 - b0) < 0.0 {
            pts.reverse();
        }
        Some(pts)
    }

    /// The command that draws closed `pts` on `plane`.
    fn shape_cmd(&self, plane: &DrawPlane, pts: Vec<Point3>) -> AnyCommand {
        match plane.face {
            Some((element, face)) => ImprintFace {
                element,
                face,
                points: pts,
                closed: true,
            }
            .into(),
            None => CreatePolygon {
                points: pts,
                level: plane.level,
                name: None,
                relative_to_level: false,
            }
            .into(),
        }
    }

    fn pp_distance(&self, ctx: &ToolCtx) -> Option<f64> {
        let d = self.drag.as_ref()?;
        // Closest point on the normal line through `start` to the cursor ray.
        line_param(d.start, d.normal, &ctx.ray)
    }

    /// Section being placed: the clicked face's plane moved to the cursor
    /// along its normal (the outside of the face is cut away).
    pub fn section_preview(&self, ctx: &ToolCtx) -> Option<SectionPlane> {
        let s = self.section?;
        let t = line_param(s.point, s.normal, &ctx.ray)?;
        Some(SectionPlane {
            point: s.point + s.normal.as_vec() * t,
            normal: s.normal,
        })
    }

    fn move_target(&self, ctx: &ToolCtx) -> Option<Point3> {
        ctx.snap.map(|s| s.point)
    }

    fn move_cmd(&self, ctx: &ToolCtx, v: Vec3) -> AnyCommand {
        if let Some((element, parts)) = &self.geom {
            let mut cmd = MoveGeometry {
                element: *element,
                vertices: vec![],
                edges: vec![],
                faces: vec![],
                vector: v,
            };
            for p in parts {
                match p {
                    SubPick::Vertex(x) => cmd.vertices.push(*x),
                    SubPick::Edge(x) => cmd.edges.push(*x),
                    SubPick::Face(x) => cmd.faces.push(*x),
                }
            }
            return cmd.into();
        }
        if ctx.copy || self.paste {
            CopyElements {
                elements: self.move_elems.clone(),
                vector: v,
                count: 1,
            }
            .into()
        } else {
            MoveElements {
                elements: self.move_elems.clone(),
                vector: v,
            }
            .into()
        }
    }

    /// Horizontal angle from the pivot to the reference and to the cursor.
    fn rotate_angle(&self, ctx: &ToolCtx) -> Option<f64> {
        let (c, r) = (self.pivot?, self.reference?);
        let p = ctx.snap.map(|s| s.point)?;
        let a0 = (r.y - c.y).atan2(r.x - c.x);
        let a1 = (p.y - c.y).atan2(p.x - c.x);
        let mut a = a1 - a0;
        while a > std::f64::consts::PI {
            a -= std::f64::consts::TAU;
        }
        while a < -std::f64::consts::PI {
            a += std::f64::consts::TAU;
        }
        // Snap to 15° steps within 1.5° (SketchUp protractor feel).
        let step = 15f64.to_radians();
        let snapped = (a / step).round() * step;
        Some(if (a - snapped).abs() < 1.5f64.to_radians() {
            snapped
        } else {
            a
        })
    }

    fn scale_factor(&self, ctx: &ToolCtx) -> Option<f64> {
        let (c, r) = (self.pivot?, self.reference?);
        let p = ctx.snap.map(|s| s.point)?;
        let d0 = (r - c).length();
        (d0 > 1e-6).then(|| ((p - c).length() / d0).max(0.01))
    }

    fn targets(&self, ctx: &ToolCtx, s: &SnapResult, forms_only: bool) -> Vec<ElementId> {
        let mut v: Vec<ElementId> = if ctx.selection.is_empty() {
            s.element.into_iter().collect()
        } else {
            ctx.selection.iter().copied().collect()
        };
        v.sort();
        v.retain(|e| {
            ctx.doc
                .get(*e)
                .is_some_and(|x| x.geometry().is_some() && (!forms_only || x.as_form().is_some()))
        });
        v
    }

    /// Command to preview this frame, if any.
    pub fn preview(&self, ctx: &ToolCtx) -> Option<AnyCommand> {
        match self.kind {
            ToolKind::Rectangle => {
                let (first, plane) = (self.rect_first?, self.plane?);
                let p = on_plane(ctx, &plane)?;
                let pts = self.rect_points(&plane, first, p)?;
                Some(if plane.face.is_none() && plane.is_horizontal() {
                    self.rect_cmd(&plane, first, p)
                } else {
                    self.shape_cmd(&plane, pts.to_vec())
                })
            }
            ToolKind::Circle => {
                let (c, plane) = (self.circle_center?, self.plane?);
                let p = on_plane(ctx, &plane)?;
                let r = p.distance_to(c);
                (r >= 1.0).then(|| self.circle_cmd(&plane, c, r))
            }
            ToolKind::Arc => {
                let plane = self.arc_plane(ctx)?;
                let m = on_plane(ctx, &plane)?;
                self.arc_cmd(&plane, m)
            }
            ToolKind::Polygon | ToolKind::Line => None,
            ToolKind::Wall => {
                let last = *self.poly.last()?;
                let p = on_plane(ctx, &self.plane?)?;
                let mut pts = self.poly.clone();
                if p.distance_to(last) >= 1.0 {
                    pts.push(p);
                }
                (pts.len() >= 2).then(|| self.walls_cmd(ctx, pts, false))
            }
            ToolKind::Floor => {
                let p = on_plane(ctx, &self.plane?)?;
                let mut pts = self.poly.clone();
                pts.push(p);
                (pts.len() >= 3).then(|| self.floor_cmd(ctx, &pts))
            }
            ToolKind::Door | ToolKind::Window => self.opening_cmd(ctx),
            ToolKind::PushPull => {
                let d = self.drag.as_ref()?;
                let dist = self.pp_distance(ctx)?;
                (dist.abs() >= 1.0).then(|| {
                    PushPullFace {
                        element: d.element,
                        face: d.face,
                        distance: Length::from_mm(dist),
                    }
                    .into()
                })
            }
            ToolKind::Offset => {
                let o = self.offset.as_ref()?;
                let p = on_plane(ctx, &o.plane)?;
                let d = distance_to_boundary(&o.plane, p, &o.boundary);
                (d >= 1.0).then(|| {
                    OffsetFace {
                        element: o.element,
                        face: o.face,
                        distance: Length::from_mm(d.round()),
                    }
                    .into()
                })
            }
            ToolKind::Move => {
                let base = self.move_base?;
                let t = self.move_target(ctx)?;
                let v = t - base;
                (v.length() >= 1.0).then(|| self.move_cmd(ctx, v))
            }
            ToolKind::Rotate => {
                let a = self.rotate_angle(ctx)?;
                (a.abs() > 1e-4).then(|| {
                    RotateElements {
                        elements: self.move_elems.clone(),
                        center: self.pivot.expect("pivot"),
                        angle: Angle::from_rad(a),
                    }
                    .into()
                })
            }
            ToolKind::Scale => {
                let f = self.scale_factor(ctx)?;
                ((f - 1.0).abs() > 1e-4).then(|| {
                    ScaleElements {
                        elements: self.move_elems.clone(),
                        center: self.pivot.expect("pivot"),
                        factor: Some(f),
                        factors: None,
                    }
                    .into()
                })
            }
            _ => None,
        }
    }

    fn rect_cmd(&self, plane: &DrawPlane, first: Point3, p: Point3) -> AnyCommand {
        let (w, d) = (p.x - first.x, p.y - first.y);
        let level_z = plane.level.map(|_| plane.origin.z).unwrap_or(0.0);
        let origin = if plane.level.is_some() {
            Point3::new(first.x, first.y, first.z - level_z)
        } else {
            first
        };
        CreateRectangle {
            origin,
            width: Length::from_mm(w),
            depth: Length::from_mm(d),
            level: plane.level,
            name: None,
        }
        .into()
    }

    /// The arc's plane. An arc started on a Form edge lies on the face that
    /// holds its ends and the bulge point (decided while setting the bulge).
    fn arc_plane(&self, ctx: &ToolCtx) -> Option<DrawPlane> {
        match (self.pending_face_of, self.arc.as_slice(), ctx.snap) {
            (Some(e), [a, b], Some(s)) => {
                Some(match face_containing(ctx.doc, e, &[*a, *b, s.point]) {
                    Some((f, n)) => DrawPlane::on_face(*a, n, Some((e, f))),
                    None => self.plane?,
                })
            }
            _ => self.plane,
        }
    }

    /// The arc through the bulge point `m`: on a face it splits it (its ends
    /// should be on the face's edges); elsewhere arc + chord make a face.
    fn arc_cmd(&self, plane: &DrawPlane, m: Point3) -> Option<AnyCommand> {
        let [a, b] = <[Point3; 2]>::try_from(self.arc.as_slice()).ok()?;
        let pts = arc_points(plane, a, b, m)?;
        Some(match plane.face {
            Some((element, face)) => ImprintFace {
                element,
                face,
                points: pts,
                closed: false,
            }
            .into(),
            None => self.shape_cmd(plane, pts),
        })
    }

    fn circle_cmd(&self, plane: &DrawPlane, c: Point3, r: f64) -> AnyCommand {
        let (element, face) = match plane.face {
            Some((e, f)) => (Some(e), Some(f)),
            None => (None, None),
        };
        CreateCircle {
            center: c,
            radius: Length::from_mm(r.round().max(1.0)),
            normal: Some(plane.n.as_vec()),
            segments: None,
            element,
            face,
            level: plane.level,
            name: None,
        }
        .into()
    }

    /// Live measurement shown in the value box.
    pub fn measurement(&self, ctx: &ToolCtx) -> String {
        let units = &ctx.doc.project().settings.units;
        let m = |x: f64| crate::units::length(units, x);
        match self.kind {
            ToolKind::Rectangle => (|| {
                let (f, plane) = (self.rect_first?, self.plane?);
                let p = on_plane(ctx, &plane)?;
                let (a0, b0) = plane.local(f);
                let (a1, b1) = plane.local(p);
                Some(crate::units::lengths(
                    units,
                    &[(a1 - a0).abs(), (b1 - b0).abs()],
                ))
            })()
            .unwrap_or_default(),
            ToolKind::Tape => (|| {
                let a = self.tape?;
                Some(m(ctx.snap?.point.distance_to(a)))
            })()
            .unwrap_or_default(),
            ToolKind::Section => (|| {
                let s = self.section?;
                Some(m(-line_param(s.point, s.normal, &ctx.ray)?))
            })()
            .unwrap_or_default(),
            ToolKind::Circle => (|| {
                let p = on_plane(ctx, &self.plane?)?;
                Some(m(p.distance_to(self.circle_center?)))
            })()
            .unwrap_or_default(),
            k if k.is_chain() => (|| {
                let l = *self.poly.last()?;
                let p = on_plane(ctx, &self.plane?)?;
                Some(m(p.distance_to(l)))
            })()
            .unwrap_or_default(),
            ToolKind::PushPull => self.pp_distance(ctx).map(m).unwrap_or_default(),
            ToolKind::Offset => (|| {
                let o = self.offset.as_ref()?;
                let p = on_plane(ctx, &o.plane)?;
                Some(m(distance_to_boundary(&o.plane, p, &o.boundary)))
            })()
            .unwrap_or_default(),
            ToolKind::Move => self
                .move_base
                .and_then(|b| self.move_target(ctx).map(|t| m(t.distance_to(b))))
                .unwrap_or_default(),
            ToolKind::Rotate => self
                .rotate_angle(ctx)
                .map(|a| crate::units::angle(units, a))
                .unwrap_or_default(),
            ToolKind::Scale => self
                .scale_factor(ctx)
                .map(|f| format!("{f:.3}"))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn value_label(&self) -> &'static str {
        match self.kind {
            ToolKind::Rectangle => "Dimensions",
            ToolKind::Circle => "Radius",
            ToolKind::Tape => "Length",
            ToolKind::Section => "Depth",
            k if k.is_chain() => "Length",
            ToolKind::PushPull | ToolKind::Move | ToolKind::Offset => "Distance",
            ToolKind::Rotate => "Angle",
            ToolKind::Scale => "Scale",
            _ => "Value",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.kind {
            ToolKind::Select => "Click to select. Drag left to right for a window, right to left for a crossing. Shift adds / removes.",
            ToolKind::Line if self.poly.is_empty() => "Click the start point. Start on a face to split it.",
            ToolKind::Line => "Click the next point or type a length. Ending on the face's edge splits it; click the first point to close.",
            ToolKind::Rectangle if self.rect_first.is_none() => "Click the first corner (on a face to draw on it).",
            ToolKind::Rectangle => "Click the opposite corner, or type width,depth (e.g. 4000,3000 or 4m,3m) and press Enter.",
            ToolKind::Circle if self.circle_center.is_none() => "Click the centre (on a face to draw on it).",
            ToolKind::Circle => "Click to set the radius, or type it and press Enter.",
            ToolKind::Polygon if self.poly.is_empty() => "Click the first point.",
            ToolKind::Polygon => "Click the next point. Click the first point or press Enter to close. Backspace removes the last point.",
            ToolKind::PushPull if self.drag.is_none() => "Click a face to push or pull it. Double-click repeats the last distance.",
            ToolKind::PushPull => "Move to set the distance, click to finish, or type a distance and press Enter.",
            ToolKind::Offset if self.offset.is_none() => "Click a face to offset its outline.",
            ToolKind::Offset => "Move inwards to set the distance, click, or type it and press Enter.",
            ToolKind::Move if self.move_base.is_none() => "Click a base point (on the element to move, or with elements selected). Hold Ctrl to copy.",
            ToolKind::Move => "Click the destination, or type a distance. After a copy, type 5x or /5 for an array.",
            ToolKind::Rotate if self.pivot.is_none() => "Click the centre of rotation (selected elements, or the one under the cursor).",
            ToolKind::Rotate if self.reference.is_none() => "Click a reference direction.",
            ToolKind::Rotate => "Click to set the angle (snaps every 15°), or type degrees and press Enter.",
            ToolKind::Scale if self.pivot.is_none() => "Click the fixed point to scale about (selected Forms, or the one under the cursor).",
            ToolKind::Scale if self.reference.is_none() => "Click a reference point.",
            ToolKind::Scale => "Move to scale, click, or type a factor (e.g. 2 or 0.5) and press Enter.",
            ToolKind::Eraser => "Click an element to delete it.",
            ToolKind::Arc if self.arc.is_empty() => "Click the start of the arc (on a face's edge to round a corner).",
            ToolKind::Arc if self.arc.len() == 1 => "Click the end of the arc.",
            ToolKind::Arc => "Move to set the bulge and click.",
            ToolKind::Section if self.section.is_none() => "Click a face: the section plane starts on it and cuts away what is in front.",
            ToolKind::Section => "Move into the model and click, or type the depth behind the face.",
            ToolKind::Tape if self.tape.is_none() => "Click the first point to measure from.",
            ToolKind::Tape => "Click the second point: the distance shows in the status bar.",
            ToolKind::Paint => "Click an element to paint it with the current material (pick one in the Materials panel).",
            ToolKind::Wall if self.poly.is_empty() => "Click the start of the wall. Draw clockwise so the exterior faces out (as in Revit).",
            ToolKind::Wall => "Click the next corner or type a length. Click the first point to close the loop, or press Enter to finish an open chain.",
            ToolKind::Floor if self.poly.is_empty() => "Click the first corner of the floor outline.",
            ToolKind::Floor => "Click the next corner. Click the first point or press Enter to finish.",
            ToolKind::Door => "Hover a wall and click to place a 900 × 2100 door.",
            ToolKind::Window => "Hover a wall and click to place a 1200 × 1200 window (sill 900).",
            ToolKind::Subtract if self.solid_target.is_none() => "Click the Form to cut (the target).",
            ToolKind::Subtract => "Click the Form to cut away: it is subtracted and deleted.",
            ToolKind::Unite if self.solid_target.is_none() => "Click the Form to keep (the target).",
            ToolKind::Unite => "Click the Form to add: it is united with the target and deleted.",
            ToolKind::Intersect if self.solid_target.is_none() => "Click the Form to keep (the target).",
            ToolKind::Intersect => "Click the other Form: the target keeps what both share; the other is deleted.",
            ToolKind::CutGeometry if self.solid_target.is_none() => "Click the element to cut (a wall, floor or Form).",
            ToolKind::CutGeometry => "Click the cutter: it stays (a Form becomes a hidden void) and the cut follows it when it moves.",
        }
    }

    /// Double-click (Push/Pull repeats the last distance on the face).
    pub fn double_click(&mut self, ctx: &ToolCtx) -> ToolEvent {
        if self.kind != ToolKind::PushPull {
            return ToolEvent::None;
        }
        let (Some(d), Some(s)) = (self.last_push, ctx.snap) else {
            return ToolEvent::None;
        };
        self.drag = None;
        match (s.element, s.face) {
            (Some(element), Some(face)) => ToolEvent::Commit(
                PushPullFace {
                    element,
                    face,
                    distance: Length::from_mm(d),
                }
                .into(),
            ),
            _ => ToolEvent::None,
        }
    }

    pub fn click(&mut self, ctx: &ToolCtx, shift: bool) -> ToolEvent {
        let Some(s) = ctx.snap else {
            return ToolEvent::None;
        };
        match self.kind {
            ToolKind::Select => match s.element {
                Some(e) if shift => ToolEvent::Select(SelectOp::Toggle(e)),
                Some(e) => ToolEvent::Select(SelectOp::Set(vec![e])),
                None if shift => ToolEvent::None,
                None => ToolEvent::Select(SelectOp::Clear),
            },
            ToolKind::Eraser => match s.element {
                Some(e) => ToolEvent::Commit(DeleteElements { elements: vec![e] }.into()),
                None => ToolEvent::None,
            },
            k @ (ToolKind::Subtract
            | ToolKind::Unite
            | ToolKind::Intersect
            | ToolKind::CutGeometry) => {
                let Some(e) = s.element else {
                    return ToolEvent::None;
                };
                let Some(target) = self.solid_target.filter(|t| *t != e) else {
                    self.solid_target = Some(e);
                    return ToolEvent::None;
                };
                self.solid_target = None;
                ToolEvent::Commit(solid_command(k, target, e))
            }
            ToolKind::Paint => match (s.element, ctx.material) {
                (Some(e), Some(m)) => ToolEvent::Commit(
                    SetParameter {
                        element: e,
                        key: keys::MATERIAL.into(),
                        value: ParamValue::ElementRef(m),
                    }
                    .into(),
                ),
                _ => ToolEvent::None,
            },
            ToolKind::Section => match self.section {
                None => {
                    if let (SnapKind::OnFace, Some(normal)) = (s.kind, s.normal) {
                        self.section = Some(SectionPlane {
                            point: s.point,
                            normal,
                        });
                    }
                    ToolEvent::None
                }
                Some(_) => match self.section_preview(ctx) {
                    Some(p) => {
                        self.section = None;
                        ToolEvent::Section(p)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Tape => match self.tape {
                None => {
                    self.tape = Some(s.point);
                    ToolEvent::None
                }
                Some(a) => {
                    self.tape = None;
                    let d = s.point - a;
                    let u = &ctx.doc.project().settings.units;
                    let l = |v: f64| crate::units::length(u, v);
                    ToolEvent::Message(format!(
                        "Distance {}  (Δx {}, Δy {}, Δz {})",
                        l(d.length()),
                        l(d.x),
                        l(d.y),
                        l(d.z)
                    ))
                }
            },
            ToolKind::Arc => match self.arc.len() {
                0 => {
                    let plane = self.pick_plane(ctx, &s);
                    let on_edge = matches!(
                        s.kind,
                        SnapKind::Endpoint | SnapKind::Midpoint | SnapKind::OnEdge
                    ) && s.element.is_some_and(|e| is_form(ctx.doc, e));
                    if on_edge {
                        // Which face? Decided with the bulge point.
                        self.pending_face_of = s.element;
                        self.arc.push(s.point);
                    } else {
                        self.arc.push(plane.project(s.point));
                    }
                    self.plane = Some(plane);
                    ToolEvent::None
                }
                1 => {
                    let p = if self.pending_face_of.is_some() {
                        Some(s.point)
                    } else {
                        self.plane.and_then(|pl| on_plane(ctx, &pl))
                    };
                    if let Some(p) = p {
                        if p.distance_to(self.arc[0]) >= 1.0 {
                            self.arc.push(p);
                        }
                    }
                    ToolEvent::None
                }
                _ => match self.preview(ctx) {
                    Some(c) => {
                        self.arc.clear();
                        self.plane = None;
                        self.pending_face_of = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Rectangle => match self.rect_first {
                None => {
                    let plane = self.pick_plane(ctx, &s);
                    self.rect_first = Some(plane.project(s.point));
                    self.plane = Some(plane);
                    ToolEvent::None
                }
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.rect_first = None;
                        self.plane = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Circle => match self.circle_center {
                None => {
                    let plane = self.pick_plane(ctx, &s);
                    self.circle_center = Some(plane.project(s.point));
                    self.plane = Some(plane);
                    ToolEvent::None
                }
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.circle_center = None;
                        self.plane = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Polygon | ToolKind::Line | ToolKind::Wall | ToolKind::Floor => {
                if self.poly.len() >= 3 && ctx.near_first_px.is_some_and(|d| d <= 10.0) {
                    return self.close_chain(ctx, true);
                }
                let edge_snap = matches!(
                    s.kind,
                    SnapKind::Endpoint | SnapKind::Midpoint | SnapKind::OnEdge
                );
                if self.poly.is_empty() {
                    let start_on_edge = matches!(self.kind, ToolKind::Line | ToolKind::Polygon)
                        && edge_snap
                        && s.element.is_some_and(|e| is_form(ctx.doc, e));
                    if start_on_edge {
                        // Which face? Decided by the next point.
                        self.pending_face_of = s.element;
                        self.poly.push(s.point);
                        self.plane = Some(self.pick_plane(ctx, &s));
                        return ToolEvent::None;
                    }
                    self.plane = Some(self.pick_plane(ctx, &s));
                }
                if let Some(e) = self.pending_face_of.take() {
                    let second = s.point;
                    let pts = [self.poly[0], second];
                    self.plane = Some(match face_containing(ctx.doc, e, &pts) {
                        Some((f, n)) => DrawPlane::on_face(self.poly[0], n, Some((e, f))),
                        None => DrawPlane::horizontal(self.poly[0].z, None),
                    });
                }
                let plane = self.plane.expect("set");
                if let Some(p) = on_plane(ctx, &plane) {
                    if self.poly.last().is_none_or(|l| l.distance_to(p) >= 1.0) {
                        self.poly.push(p);
                    }
                }
                // A line on a face that ends on the face's boundary splits it
                // (checked geometrically: axis inferences may win the snap).
                if self.kind == ToolKind::Line && self.poly.len() >= 2 {
                    if let (Some((e, f)), Some(last)) = (plane.face, self.poly.last()) {
                        if on_face_boundary(ctx.doc, e, f, *last) {
                            return self.close_chain(ctx, false);
                        }
                    }
                }
                ToolEvent::None
            }
            ToolKind::Door | ToolKind::Window => match self.opening_cmd(ctx) {
                Some(c) => ToolEvent::Commit(c),
                None => ToolEvent::None,
            },
            ToolKind::PushPull => match &self.drag {
                None => {
                    if let Some((element, face)) = self.hover_face(ctx) {
                        let normal = ctx
                            .doc
                            .get(element)
                            .and_then(|e| e.geometry())
                            .and_then(|g| g.face_normal(face).ok());
                        if let Some(normal) = normal {
                            self.drag = Some(PushPullDrag {
                                element,
                                face,
                                start: s.point,
                                normal,
                            });
                        }
                    }
                    ToolEvent::None
                }
                Some(_) => {
                    let dist = self.pp_distance(ctx);
                    match self.preview(ctx) {
                        Some(c) => {
                            self.drag = None;
                            self.last_push = dist;
                            ToolEvent::Commit(c)
                        }
                        None => ToolEvent::None,
                    }
                }
            },
            ToolKind::Offset => match &self.offset {
                None => {
                    if let Some((element, face)) = self.hover_face(ctx) {
                        let g = ctx.doc.get(element).and_then(|e| e.geometry());
                        if let (Some(g), Some(n)) = (g, s.normal) {
                            if let Ok(f) = g.face(face) {
                                let boundary = g.loop_points(f.outer);
                                let plane = DrawPlane::on_face(s.point, n, Some((element, face)));
                                self.offset = Some(OffsetPick {
                                    element,
                                    face,
                                    boundary,
                                    plane,
                                });
                            }
                        }
                    }
                    ToolEvent::None
                }
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.offset = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Move => match self.move_base {
                None if ctx.editing.is_some() => {
                    let Some(e) = ctx.editing else {
                        return ToolEvent::None;
                    };
                    let parts: Vec<SubPick> = if ctx.sub.is_empty() {
                        pick_sub(ctx.doc, e, &s).into_iter().collect()
                    } else {
                        ctx.sub.to_vec()
                    };
                    if parts.is_empty() {
                        return ToolEvent::None;
                    }
                    self.geom = Some((e, parts));
                    self.move_base = Some(s.point);
                    ToolEvent::None
                }
                None => {
                    let elems = self.targets(ctx, &s, false);
                    if elems.is_empty() {
                        return ToolEvent::None;
                    }
                    self.move_elems = elems;
                    self.move_base = Some(s.point);
                    ToolEvent::None
                }
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.move_base = None;
                        self.paste = false;
                        self.geom = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Rotate | ToolKind::Scale => match (self.pivot, self.reference) {
                (None, _) => {
                    let elems = self.targets(ctx, &s, self.kind == ToolKind::Scale);
                    if elems.is_empty() {
                        return ToolEvent::None;
                    }
                    self.move_elems = elems;
                    self.pivot = Some(s.point);
                    ToolEvent::None
                }
                (Some(p), None) => {
                    if s.point.distance_to(p) >= 1.0 {
                        self.reference = Some(s.point);
                    }
                    ToolEvent::None
                }
                (Some(_), Some(_)) => match self.preview(ctx) {
                    Some(c) => {
                        self.pivot = None;
                        self.reference = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
        }
    }

    fn walls_cmd(&self, ctx: &ToolCtx, points: Vec<Point3>, closed: bool) -> AnyCommand {
        CreateWalls {
            points,
            closed,
            level: ctx.level.map(|l| l.0),
            wall_type: None,
            height: None,
            top_level: None,
            top_offset: Length::ZERO,
            base_offset: Length::ZERO,
            location_line: Default::default(),
            name: None,
        }
        .into()
    }

    fn floor_cmd(&self, ctx: &ToolCtx, pts: &[Point3]) -> AnyCommand {
        CreateFloor {
            points: pts.iter().map(|p| Point2::new(p.x, p.y)).collect(),
            holes: Vec::new(),
            level: ctx.level.map(|l| l.0),
            offset: Length::ZERO,
            floor_type: None,
            name: None,
        }
        .into()
    }

    /// Door / window under the cursor: the hovered wall and the position of
    /// the cursor along it.
    fn opening_cmd(&self, ctx: &ToolCtx) -> Option<AnyCommand> {
        let s = ctx.snap?;
        let e = ctx.doc.get(s.element?)?;
        let w = e.as_wall()?;
        let lay = generate::wall_layout(ctx.doc, e, w).ok()?;
        let (o, d, _) = lay.frame()?;
        let (host, u) = (e.id, (Point2::new(s.point.x, s.point.y) - o).dot(d));
        let kind = if self.kind == ToolKind::Door {
            OpeningKind::Door
        } else {
            OpeningKind::Window
        };
        Some(
            CreateOpening {
                wall: host,
                kind,
                position: Length::from_mm(u.round()),
                width: None,
                height: None,
                sill: None,
                name: None,
            }
            .into(),
        )
    }

    fn close_chain(&mut self, ctx: &ToolCtx, closed: bool) -> ToolEvent {
        let plane = self.plane;
        let ev = match self.kind {
            ToolKind::Wall => {
                if self.poly.len() < 2 || (closed && self.poly.len() < 3) {
                    return ToolEvent::None;
                }
                let pts = std::mem::take(&mut self.poly);
                ToolEvent::Commit(self.walls_cmd(ctx, pts, closed))
            }
            ToolKind::Floor => {
                if self.poly.len() < 3 {
                    return ToolEvent::None;
                }
                let pts = std::mem::take(&mut self.poly);
                ToolEvent::Commit(self.floor_cmd(ctx, &pts))
            }
            _ => {
                let Some(plane) = plane else {
                    return ToolEvent::None;
                };
                match plane.face {
                    // On a face: an open chain splits it; a closed one too.
                    Some((element, face)) if self.poly.len() >= 2 => {
                        let pts = std::mem::take(&mut self.poly);
                        ToolEvent::Commit(
                            ImprintFace {
                                element,
                                face,
                                points: pts,
                                closed,
                            }
                            .into(),
                        )
                    }
                    _ if self.poly.len() >= 3 => {
                        let pts = std::mem::take(&mut self.poly);
                        ToolEvent::Commit(self.shape_cmd(&plane, pts))
                    }
                    _ => return ToolEvent::None,
                }
            }
        };
        self.plane = None;
        ev
    }

    /// Enter pressed with typed text (value control box).
    pub fn typed(&mut self, text: &str, ctx: &ToolCtx) -> Result<ToolEvent, String> {
        if text.trim().is_empty() {
            return Ok(if self.kind.is_chain() {
                self.close_chain(ctx, false)
            } else {
                ToolEvent::None
            });
        }
        match self.kind {
            ToolKind::Rotate => {
                let rad = crate::units::parse_angle(&ctx.doc.project().settings.units, text)
                    .ok_or("type an angle, e.g. 90 or 90°")?;
                let c = self.pivot.ok_or("click the centre first")?;
                self.pivot = None;
                self.reference = None;
                return Ok(ToolEvent::Commit(
                    RotateElements {
                        elements: self.move_elems.clone(),
                        center: c,
                        angle: Angle::from_rad(rad),
                    }
                    .into(),
                ));
            }
            ToolKind::Scale => {
                let f: f64 = text
                    .trim()
                    .parse()
                    .map_err(|_| "type a factor such as 2 or 0.5")?;
                let c = self.pivot.ok_or("click the fixed point first")?;
                self.pivot = None;
                self.reference = None;
                return Ok(ToolEvent::Commit(
                    ScaleElements {
                        elements: self.move_elems.clone(),
                        center: c,
                        factor: Some(f),
                        factors: None,
                    }
                    .into(),
                ));
            }
            _ => {}
        }
        let vals = crate::units::parse_lengths(&ctx.doc.project().settings.units, text)?;
        match self.kind {
            ToolKind::Rectangle => {
                let first = self.rect_first.ok_or("click the first corner first")?;
                let plane = self.plane.ok_or("click the first corner first")?;
                let (w, d) = match vals.as_slice() {
                    [w, d] => (*w, *d),
                    [s] => (*s, *s),
                    _ => return Err("type width,depth".into()),
                };
                let p = on_plane(ctx, &plane).unwrap_or(first + plane.u + plane.v);
                let (a0, b0) = plane.local(first);
                let (a1, b1) = plane.local(p);
                let sa = if a1 < a0 { -1.0 } else { 1.0 };
                let sb = if b1 < b0 { -1.0 } else { 1.0 };
                let corner = plane.at(a0 + w * sa, b0 + d * sb);
                let cmd = if plane.face.is_none() && plane.is_horizontal() {
                    self.rect_cmd(&plane, first, corner)
                } else {
                    let pts = self
                        .rect_points(&plane, first, corner)
                        .ok_or("size too small")?;
                    self.shape_cmd(&plane, pts.to_vec())
                };
                self.rect_first = None;
                self.plane = None;
                Ok(ToolEvent::Commit(cmd))
            }
            ToolKind::Circle => {
                let c = self.circle_center.ok_or("click the centre first")?;
                let plane = self.plane.ok_or("click the centre first")?;
                self.circle_center = None;
                self.plane = None;
                Ok(ToolEvent::Commit(self.circle_cmd(&plane, c, vals[0])))
            }
            ToolKind::Polygon | ToolKind::Line | ToolKind::Wall | ToolKind::Floor => {
                let last = *self.poly.last().ok_or("click the first point first")?;
                let plane = self.plane.ok_or("click the first point first")?;
                let p = on_plane(ctx, &plane).ok_or("move the cursor to set a direction")?;
                let dir = (p - last)
                    .normalize()
                    .ok_or("move the cursor to set a direction")?;
                self.poly.push(last + dir * vals[0]);
                Ok(ToolEvent::None)
            }
            ToolKind::Section => {
                let s = self.section.take().ok_or("click a face first")?;
                Ok(ToolEvent::Section(SectionPlane {
                    point: s.point - s.normal.as_vec() * vals[0],
                    normal: s.normal,
                }))
            }
            ToolKind::PushPull => {
                let d = self.drag.take().ok_or("click a face first")?;
                self.last_push = Some(vals[0]);
                Ok(ToolEvent::Commit(
                    PushPullFace {
                        element: d.element,
                        face: d.face,
                        distance: Length::from_mm(vals[0]),
                    }
                    .into(),
                ))
            }
            ToolKind::Offset => {
                let o = self.offset.take().ok_or("click a face first")?;
                Ok(ToolEvent::Commit(
                    OffsetFace {
                        element: o.element,
                        face: o.face,
                        distance: Length::from_mm(vals[0]),
                    }
                    .into(),
                ))
            }
            ToolKind::Move => {
                let base = self.move_base.ok_or("click a base point first")?;
                let t = self
                    .move_target(ctx)
                    .ok_or("move the cursor to set a direction")?;
                let dir = (t - base)
                    .normalize()
                    .ok_or("move the cursor to set a direction")?;
                let cmd = self.move_cmd(ctx, dir * vals[0]);
                self.move_base = None;
                self.geom = None;
                Ok(ToolEvent::Commit(cmd))
            }
            _ => Ok(ToolEvent::None),
        }
    }

    /// Lines to draw: the chain being drawn, the rubber band to the cursor,
    /// move / rotate / scale guides.
    pub fn rubber_band(&self, ctx: &ToolCtx) -> Vec<(Point3, Point3)> {
        let mut segs = Vec::new();
        for w in self.poly.windows(2) {
            segs.push((w[0], w[1]));
        }
        if let (Some(l), true, Some(plane)) = (self.poly.last(), self.kind.is_chain(), self.plane) {
            let p = if self.pending_face_of.is_some() {
                ctx.snap.map(|s| s.point)
            } else {
                on_plane(ctx, &plane)
            };
            if let Some(p) = p {
                segs.push((*l, p));
            }
        }
        if let (ToolKind::Move, Some(b)) = (self.kind, self.move_base) {
            if let Some(t) = self.move_target(ctx) {
                segs.push((b, t));
            }
        }
        if let Some(c) = self.pivot {
            if let Some(r) = self.reference {
                segs.push((c, r));
            }
            if let Some(p) = ctx.snap.map(|s| s.point) {
                segs.push((c, p));
            }
        }
        if let Some(a) = self.tape {
            if let Some(p) = ctx.snap.map(|s| s.point) {
                segs.push((a, p));
            }
        }
        if let (1, Some(plane)) = (self.arc.len(), self.plane) {
            let p = if self.pending_face_of.is_some() {
                ctx.snap.map(|s| s.point)
            } else {
                on_plane(ctx, &plane)
            };
            if let Some(p) = p {
                segs.push((self.arc[0], p));
            }
        }
        if let (Some(c), Some(plane)) = (self.circle_center, self.plane) {
            if let Some(p) = on_plane(ctx, &plane) {
                segs.push((c, p));
            }
        }
        segs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_pass_through_the_bulge_point() {
        let pl = DrawPlane::horizontal(0.0, None);
        let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
        let near = |a: Point3, b: Point3| a.distance_to(b) < 1e-6;
        // Half circle above the chord, and below it.
        for m in [p(0.0, 1000.0), p(0.0, -1000.0)] {
            let pts = arc_points(&pl, p(-1000.0, 0.0), p(1000.0, 0.0), m).unwrap();
            assert!(near(pts[0], p(-1000.0, 0.0)) && near(*pts.last().unwrap(), p(1000.0, 0.0)));
            assert!(pts.iter().any(|q| near(*q, m)));
            assert!(pts
                .iter()
                .all(|q| (q.distance_to(p(0.0, 0.0)) - 1000.0).abs() < 1e-6));
        }
        // Major arc: the apex is 3000 from the chord wherever along it `m` is.
        let pts = arc_points(&pl, p(-1000.0, 0.0), p(1000.0, 0.0), p(700.0, 3000.0)).unwrap();
        assert!(pts.iter().any(|q| near(*q, p(0.0, 3000.0))));
        // Wider than the chord (radius 1667), never below it.
        assert!(pts.iter().any(|q| q.x > 1600.0) && pts.iter().all(|q| q.y > -1e-6));
        // Shallow bulge still gives a few segments; a straight line gives none.
        assert!(arc_points(&pl, p(0.0, 0.0), p(1000.0, 0.0), p(500.0, 0.01))
            .is_some_and(|v| v.len() >= 3));
        assert!(arc_points(&pl, p(0.0, 0.0), p(1000.0, 0.0), p(500.0, 0.0)).is_none());
        assert!(arc_points(&pl, p(0.0, 0.0), p(0.0, 0.0), p(500.0, 50.0)).is_none());
    }

    #[test]
    fn solid_tools_pick_the_target_then_the_other() {
        use ferro_core::model::RelationshipKind;
        match solid_command(ToolKind::CutGeometry, ElementId(1), ElementId(2)) {
            AnyCommand::CreateRelationship(c) => {
                assert_eq!(c.kind, RelationshipKind::Cut);
                assert_eq!(c.elements, [ElementId(2), ElementId(1)], "[cutter, cut]");
                assert_eq!(c.phase, None);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            solid_command(ToolKind::Subtract, ElementId(1), ElementId(2)),
            AnyCommand::SubtractForms(SubtractForms {
                target: ElementId(1),
                cutter: ElementId(2),
                keep_cutter: false
            })
        ));
        assert!(matches!(
            solid_command(ToolKind::Unite, ElementId(1), ElementId(2)),
            AnyCommand::UniteForms(UniteForms {
                target: ElementId(1),
                other: ElementId(2),
                keep_other: false
            })
        ));
        assert!(matches!(
            solid_command(ToolKind::Intersect, ElementId(1), ElementId(2)),
            AnyCommand::IntersectForms(IntersectForms {
                target: ElementId(1),
                other: ElementId(2),
                ..
            })
        ));
        assert!(ToolKind::ALL.iter().filter(|k| k.is_solid()).count() == 4);
        let t = ToolState::new(ToolKind::Unite);
        assert!(!t.is_active());
        assert!(t.hint().contains("keep"));
    }

    #[test]
    fn value_box_parsing() {
        use crate::units::parse_lengths;
        let u = ferro_core::model::project_settings::ProjectUnits::default();
        assert_eq!(parse_lengths(&u, "4000,3000"), Ok(vec![4000.0, 3000.0]));
        assert_eq!(parse_lengths(&u, "4m; 3m"), Ok(vec![4000.0, 3000.0]));
        assert!(parse_lengths(&u, "abc").is_err());
        assert!(parse_lengths(&u, "  ").is_err());
    }

    #[test]
    fn face_planes_have_a_right_handed_frame() {
        for n in [Vec3::Z, Vec3::X, -Vec3::Y, Vec3::new(1.0, 1.0, 1.0)] {
            let n = n.normalize().unwrap();
            let p = DrawPlane::on_face(Point3::new(1.0, 2.0, 3.0), n, None);
            assert!(p.u.dot(n.as_vec()).abs() < 1e-12);
            assert!((p.u.cross(p.v) - n.as_vec()).length() < 1e-12);
            let q = p.at(10.0, 20.0);
            let (a, b) = p.local(q);
            assert!((a - 10.0).abs() < 1e-9 && (b - 20.0).abs() < 1e-9);
        }
    }

    #[test]
    fn offset_distance_is_to_the_nearest_edge() {
        let plane = DrawPlane::horizontal(0.0, None);
        let sq = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1000.0, 0.0, 0.0),
            Point3::new(1000.0, 1000.0, 0.0),
            Point3::new(0.0, 1000.0, 0.0),
        ];
        assert!(
            (distance_to_boundary(&plane, Point3::new(100.0, 500.0, 0.0), &sq) - 100.0).abs()
                < 1e-9
        );
    }
}
