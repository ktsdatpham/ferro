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
    Polygon,
    PushPull,
    Offset,
    Move,
    Rotate,
    Scale,
    Eraser,
    Paint,
    Wall,
    Floor,
    Door,
    Window,
}

impl ToolKind {
    pub const ALL: [ToolKind; 16] = [
        ToolKind::Select,
        ToolKind::Line,
        ToolKind::Rectangle,
        ToolKind::Circle,
        ToolKind::Polygon,
        ToolKind::PushPull,
        ToolKind::Offset,
        ToolKind::Move,
        ToolKind::Rotate,
        ToolKind::Scale,
        ToolKind::Eraser,
        ToolKind::Paint,
        ToolKind::Wall,
        ToolKind::Floor,
        ToolKind::Door,
        ToolKind::Window,
    ];
    /// Tools that draw a chain of clicked points.
    pub fn is_chain(self) -> bool {
        matches!(
            self,
            ToolKind::Line | ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor
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
        }
    }
    /// Keyboard shortcut (SketchUp defaults; BIM tools on free keys).
    pub fn shortcut(self) -> &'static str {
        match self {
            ToolKind::Select => "Space",
            ToolKind::Line => "L",
            ToolKind::Rectangle => "R",
            ToolKind::Circle => "C",
            ToolKind::Polygon => "G",
            ToolKind::PushPull => "P",
            ToolKind::Offset => "F",
            ToolKind::Move => "M",
            ToolKind::Rotate => "Q",
            ToolKind::Scale => "S",
            ToolKind::Eraser => "E",
            ToolKind::Paint => "B",
            ToolKind::Wall => "W",
            ToolKind::Floor => "U",
            ToolKind::Door => "D",
            ToolKind::Window => "N",
        }
    }
    pub fn icon(self) -> &'static str {
        self.shortcut()
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
    Commit(AnyCommand),
    Select(SelectOp),
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
}

fn is_point_snap(k: SnapKind) -> bool {
    matches!(
        k,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Origin
            | SnapKind::OnEdge
            | SnapKind::OnAxis(_)
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

/// Parse "4000", "4m", "4000,3000", "4m;3m".
pub fn parse_lengths(text: &str) -> Result<Vec<f64>, String> {
    let parts: Vec<&str> = text
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("type a value, e.g. 3000 or 3m".into());
    }
    parts
        .iter()
        .map(|p| {
            Length::parse(p)
                .map(|l| l.mm())
                .ok_or_else(|| format!("cannot read {p:?}"))
        })
        .collect()
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
        }
    }

    pub fn is_active(&self) -> bool {
        self.rect_first.is_some()
            || self.circle_center.is_some()
            || !self.poly.is_empty()
            || self.drag.is_some()
            || self.offset.is_some()
            || self.move_base.is_some()
            || self.pivot.is_some()
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
            ToolKind::Line | ToolKind::Rectangle | ToolKind::Circle | ToolKind::Polygon
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
        let u = d.normal.as_vec();
        let r = ctx.ray.dir.as_vec();
        let w = ctx.ray.origin - d.start;
        let (ur, rr, uw, rw) = (u.dot(r), r.dot(r), u.dot(w), r.dot(w));
        let den = rr - ur * ur;
        if den.abs() < 1e-9 {
            return None;
        }
        Some((rr * uw - ur * rw) / den)
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
        let m = |x: f64| format!("{:.0}", x);
        match self.kind {
            ToolKind::Rectangle => (|| {
                let (f, plane) = (self.rect_first?, self.plane?);
                let p = on_plane(ctx, &plane)?;
                let (a0, b0) = plane.local(f);
                let (a1, b1) = plane.local(p);
                Some(format!("{}, {}", m((a1 - a0).abs()), m((b1 - b0).abs())))
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
                .map(|a| format!("{:.1}°", a.to_degrees()))
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
            k if k.is_chain() => "Length",
            ToolKind::PushPull | ToolKind::Move | ToolKind::Offset => "Distance",
            ToolKind::Rotate => "Angle",
            ToolKind::Scale => "Scale",
            _ => "Value",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.kind {
            ToolKind::Select => "Click to select. Drag left→right for a window, right→left for a crossing. Shift adds / removes.",
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
            ToolKind::Paint => "Click an element to paint it with the current material (pick one in the Materials panel).",
            ToolKind::Wall if self.poly.is_empty() => "Click the start of the wall. Draw clockwise so the exterior faces out (as in Revit).",
            ToolKind::Wall => "Click the next corner or type a length. Click the first point to close the loop, or press Enter to finish an open chain.",
            ToolKind::Floor if self.poly.is_empty() => "Click the first corner of the floor outline.",
            ToolKind::Floor => "Click the next corner. Click the first point or press Enter to finish.",
            ToolKind::Door => "Hover a wall and click to place a 900 × 2100 door.",
            ToolKind::Window => "Hover a wall and click to place a 1200 × 1200 window (sill 900).",
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
                    let e = ctx.editing.expect("editing");
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
                let deg: f64 = text
                    .trim()
                    .trim_end_matches('°')
                    .parse()
                    .map_err(|_| "type an angle in degrees")?;
                let c = self.pivot.ok_or("click the centre first")?;
                self.pivot = None;
                self.reference = None;
                return Ok(ToolEvent::Commit(
                    RotateElements {
                        elements: self.move_elems.clone(),
                        center: c,
                        angle: Angle::from_deg(deg),
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
        let vals = parse_lengths(text)?;
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
    fn value_box_parsing() {
        assert_eq!(parse_lengths("4000,3000").unwrap(), vec![4000.0, 3000.0]);
        assert_eq!(parse_lengths("4m; 3m").unwrap(), vec![4000.0, 3000.0]);
        assert!(parse_lengths("abc").is_err());
        assert!(parse_lengths("  ").is_err());
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
