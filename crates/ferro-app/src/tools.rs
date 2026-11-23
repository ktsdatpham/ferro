//! Drawing tools (D-72): app-side state machines. Each tool previews by
//! evaluating a command on a copy of the document, and commits exactly one
//! named command, so a human and an AI agent produce identical history.

use ferro_core::model::bim::OpeningKind;
use ferro_core::model::generate;
use ferro_core::model::{Document, ElementId};
use ferro_core::ops::bim_commands::*;
use ferro_core::ops::commands::*;
use ferro_core::topo::FaceId;
use ferro_core::{AnyCommand, Length, Point2, Point3, UnitVec3, Vec3};
use ferro_snap::{Ray, SnapKind, SnapResult};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Select,
    Rectangle,
    Polygon,
    PushPull,
    Move,
    Eraser,
    Wall,
    Floor,
    Door,
    Window,
}

impl ToolKind {
    pub const ALL: [ToolKind; 10] = [
        ToolKind::Select,
        ToolKind::Rectangle,
        ToolKind::Polygon,
        ToolKind::PushPull,
        ToolKind::Move,
        ToolKind::Eraser,
        ToolKind::Wall,
        ToolKind::Floor,
        ToolKind::Door,
        ToolKind::Window,
    ];
    /// Tools that draw a chain of clicked points.
    pub fn is_chain(self) -> bool {
        matches!(self, ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor)
    }
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Select => "Select",
            ToolKind::Rectangle => "Rectangle",
            ToolKind::Polygon => "Polygon",
            ToolKind::PushPull => "Push/Pull",
            ToolKind::Move => "Move",
            ToolKind::Eraser => "Eraser",
            ToolKind::Wall => "Wall",
            ToolKind::Floor => "Floor",
            ToolKind::Door => "Door",
            ToolKind::Window => "Window",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            ToolKind::Select => "Space",
            ToolKind::Rectangle => "R",
            ToolKind::Polygon => "G",
            ToolKind::PushPull => "P",
            ToolKind::Move => "M",
            ToolKind::Eraser => "E",
            ToolKind::Wall => "W",
            ToolKind::Floor => "F",
            ToolKind::Door => "D",
            ToolKind::Window => "N",
        }
    }
    pub fn shortcut(self) -> &'static str {
        match self {
            ToolKind::Select => "Space",
            ToolKind::Rectangle => "R",
            ToolKind::Polygon => "G",
            ToolKind::PushPull => "P",
            ToolKind::Move => "M",
            ToolKind::Eraser => "E",
            ToolKind::Wall => "W",
            ToolKind::Floor => "F",
            ToolKind::Door => "D",
            ToolKind::Window => "N",
        }
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
    /// Cursor distance to the polygon's first point (px), if any.
    pub near_first_px: Option<f32>,
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

#[derive(Debug, Clone)]
struct PushPullDrag {
    element: ElementId,
    face: FaceId,
    start: Point3,
    normal: UnitVec3,
}

#[derive(Debug, Clone)]
pub struct ToolState {
    pub kind: ToolKind,
    rect_first: Option<Point3>,
    poly: Vec<Point3>,
    drag: Option<PushPullDrag>,
    move_base: Option<Point3>,
    move_elems: Vec<ElementId>,
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

/// Point on the horizontal plane z = `z` under the cursor.
fn on_plane(ctx: &ToolCtx, z: f64) -> Option<Point3> {
    if let Some(s) = ctx.snap.filter(|s| is_point_snap(s.kind)) {
        return Some(Point3::new(s.point.x, s.point.y, z));
    }
    let d = ctx.ray.dir.as_vec();
    if d.z.abs() > 1e-9 {
        let t = (z - ctx.ray.origin.z) / d.z;
        if t > 0.0 {
            return Some(ctx.ray.at(t));
        }
    }
    ctx.snap.map(|s| Point3::new(s.point.x, s.point.y, z))
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

impl ToolState {
    pub fn new(kind: ToolKind) -> Self {
        ToolState {
            kind,
            rect_first: None,
            poly: Vec::new(),
            drag: None,
            move_base: None,
            move_elems: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.rect_first.is_some()
            || !self.poly.is_empty()
            || self.drag.is_some()
            || self.move_base.is_some()
    }

    pub fn cancel(&mut self) {
        *self = ToolState::new(self.kind);
    }

    pub fn backspace(&mut self) {
        if self.kind.is_chain() {
            self.poly.pop();
        }
    }

    /// Anchor for axis inference (polygon edges and moves; a rectangle's
    /// opposite corner must not lock to an axis or it would be degenerate).
    pub fn anchor(&self) -> Option<Point3> {
        match self.kind {
            k if k.is_chain() => self.poly.last().copied(),
            ToolKind::Move => self.move_base,
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

    /// Face to highlight under the cursor.
    pub fn hover_face(&self, ctx: &ToolCtx) -> Option<(ElementId, FaceId)> {
        if let Some(d) = &self.drag {
            return Some((d.element, d.face));
        }
        if self.kind != ToolKind::PushPull {
            return None;
        }
        let s = ctx.snap?;
        let (e, f) = (s.element?, s.face?);
        ctx.doc.get(e)?.geometry()?;
        Some((e, f))
    }

    fn rect_plane_z(&self, ctx: &ToolCtx, s: &SnapResult) -> f64 {
        match s.kind {
            SnapKind::OnFace if s.normal.is_some_and(|n| n.as_vec().z.abs() > 0.999) => s.point.z,
            SnapKind::Ground => ctx.level.map(|l| l.1).unwrap_or(s.point.z),
            _ => s.point.z,
        }
    }

    fn rect_cmd(&self, ctx: &ToolCtx, first: Point3, w: f64, d: f64) -> AnyCommand {
        let level = ctx.level.filter(|l| (l.1 - first.z).abs() < 1e-6);
        let origin = match level {
            Some((_, z)) => Point3::new(first.x, first.y, first.z - z),
            None => first,
        };
        CreateRectangle {
            origin,
            width: Length::from_mm(w),
            depth: Length::from_mm(d),
            level: level.map(|l| l.0),
            name: None,
        }
        .into()
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
        if ctx.copy {
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

    /// Command to preview this frame, if any.
    pub fn preview(&self, ctx: &ToolCtx) -> Option<AnyCommand> {
        match self.kind {
            ToolKind::Rectangle => {
                let first = self.rect_first?;
                let p = on_plane(ctx, first.z)?;
                let (w, d) = (p.x - first.x, p.y - first.y);
                (w.abs() >= 1.0 && d.abs() >= 1.0).then(|| self.rect_cmd(ctx, first, w, d))
            }
            ToolKind::Polygon => None,
            ToolKind::Wall => {
                let last = *self.poly.last()?;
                let p = on_plane(ctx, last.z)?;
                let mut pts = self.poly.clone();
                if p.distance_to(last) >= 1.0 {
                    pts.push(p);
                }
                (pts.len() >= 2).then(|| self.walls_cmd(ctx, pts, false))
            }
            ToolKind::Floor => {
                let last = *self.poly.last()?;
                let p = on_plane(ctx, last.z)?;
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
            ToolKind::Move => {
                let base = self.move_base?;
                let t = self.move_target(ctx)?;
                let v = t - base;
                (v.length() >= 1.0).then(|| self.move_cmd(ctx, v))
            }
            _ => None,
        }
    }

    /// Live measurement shown in the value box.
    pub fn measurement(&self, ctx: &ToolCtx) -> String {
        let m = |x: f64| format!("{:.0}", x);
        match self.kind {
            ToolKind::Rectangle => self
                .rect_first
                .and_then(|f| {
                    on_plane(ctx, f.z)
                        .map(|p| format!("{}, {}", m((p.x - f.x).abs()), m((p.y - f.y).abs())))
                })
                .unwrap_or_default(),
            k if k.is_chain() => self
                .poly
                .last()
                .and_then(|l| on_plane(ctx, l.z).map(|p| m(p.distance_to(*l))))
                .unwrap_or_default(),
            ToolKind::PushPull => self.pp_distance(ctx).map(m).unwrap_or_default(),
            ToolKind::Move => self
                .move_base
                .and_then(|b| self.move_target(ctx).map(|t| m(t.distance_to(b))))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn value_label(&self) -> &'static str {
        match self.kind {
            ToolKind::Rectangle => "Dimensions",
            ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor => "Length",
            ToolKind::PushPull | ToolKind::Move => "Distance",
            _ => "Value",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.kind {
            ToolKind::Select => "Click to select. Shift+click to add or remove. Delete removes the selection.",
            ToolKind::Rectangle if self.rect_first.is_none() => "Click the first corner.",
            ToolKind::Rectangle => "Click the opposite corner, or type width,depth (e.g. 4000,3000 or 4m,3m) and press Enter.",
            ToolKind::Polygon if self.poly.is_empty() => "Click the first point.",
            ToolKind::Polygon => "Click the next point. Click the first point or press Enter to close. Backspace removes the last point.",
            ToolKind::PushPull if self.drag.is_none() => "Click a face to push or pull it.",
            ToolKind::PushPull => "Move to set the distance, click to finish, or type a distance and press Enter.",
            ToolKind::Move if self.move_base.is_none() => "Click a base point (on the element to move, or with elements selected). Hold Ctrl to copy.",
            ToolKind::Move => "Click the destination, or type a distance and press Enter. Arrow-free: snap to the red/green/blue axes.",
            ToolKind::Eraser => "Click an element to delete it.",
            ToolKind::Wall if self.poly.is_empty() => "Click the start of the wall. Draw clockwise so the exterior faces out (as in Revit).",
            ToolKind::Wall => "Click the next corner or type a length. Click the first point to close the loop, or press Enter to finish an open chain.",
            ToolKind::Floor if self.poly.is_empty() => "Click the first corner of the floor outline.",
            ToolKind::Floor => "Click the next corner. Click the first point or press Enter to finish.",
            ToolKind::Door => "Hover a wall and click to place a 900 × 2100 door.",
            ToolKind::Window => "Hover a wall and click to place a 1200 × 1200 window (sill 900).",
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
            ToolKind::Door | ToolKind::Window => match self.opening_cmd(ctx) {
                Some(c) => ToolEvent::Commit(c),
                None => ToolEvent::None,
            },
            ToolKind::Rectangle => match self.rect_first {
                None => {
                    let z = self.rect_plane_z(ctx, &s);
                    self.rect_first = Some(Point3::new(s.point.x, s.point.y, z));
                    ToolEvent::None
                }
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.rect_first = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor => {
                if self.poly.len() >= 3 && ctx.near_first_px.is_some_and(|d| d <= 10.0) {
                    return self.close_chain(ctx, true);
                }
                let z = match self.kind {
                    // BIM elements are drawn on the active level.
                    ToolKind::Wall | ToolKind::Floor => ctx.level.map(|l| l.1).unwrap_or(0.0),
                    _ => self
                        .poly
                        .first()
                        .map(|p| p.z)
                        .unwrap_or_else(|| self.rect_plane_z(ctx, &s)),
                };
                if let Some(p) = on_plane(ctx, z) {
                    if self.poly.last().is_none_or(|l| l.distance_to(p) >= 1.0) {
                        self.poly.push(p);
                    }
                }
                ToolEvent::None
            }
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
                Some(_) => match self.preview(ctx) {
                    Some(c) => {
                        self.drag = None;
                        ToolEvent::Commit(c)
                    }
                    None => ToolEvent::None,
                },
            },
            ToolKind::Move => match self.move_base {
                None => {
                    let elems: Vec<ElementId> = if ctx.selection.is_empty() {
                        s.element.into_iter().collect()
                    } else {
                        let mut v: Vec<ElementId> = ctx.selection.iter().copied().collect();
                        v.sort();
                        v
                    };
                    let elems: Vec<ElementId> = elems
                        .into_iter()
                        .filter(|e| ctx.doc.get(*e).is_some_and(|x| x.geometry().is_some()))
                        .collect();
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
        match self.kind {
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
            _ => self.close_polygon(ctx),
        }
    }

    fn close_polygon(&mut self, ctx: &ToolCtx) -> ToolEvent {
        if self.poly.len() < 3 {
            return ToolEvent::None;
        }
        let pts = std::mem::take(&mut self.poly);
        let level = ctx
            .level
            .filter(|l| (l.1 - pts[0].z).abs() < 1e-6)
            .map(|l| l.0);
        ToolEvent::Commit(
            CreatePolygon {
                points: pts,
                level,
                name: None,
                relative_to_level: false,
            }
            .into(),
        )
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
        let vals = parse_lengths(text)?;
        match self.kind {
            ToolKind::Rectangle => {
                let first = self.rect_first.ok_or("click the first corner first")?;
                let (w, d) = match vals.as_slice() {
                    [w, d] => (*w, *d),
                    [s] => (*s, *s),
                    _ => return Err("type width,depth".into()),
                };
                let p = on_plane(ctx, first.z).unwrap_or(first + Vec3::new(1.0, 1.0, 0.0));
                let sx = if p.x < first.x { -1.0 } else { 1.0 };
                let sy = if p.y < first.y { -1.0 } else { 1.0 };
                self.rect_first = None;
                Ok(ToolEvent::Commit(self.rect_cmd(ctx, first, w * sx, d * sy)))
            }
            ToolKind::Polygon | ToolKind::Wall | ToolKind::Floor => {
                let last = *self.poly.last().ok_or("click the first point first")?;
                let p = on_plane(ctx, last.z).ok_or("move the cursor to set a direction")?;
                let dir = (p - last)
                    .normalize()
                    .ok_or("move the cursor to set a direction")?;
                self.poly.push(last + dir * vals[0]);
                Ok(ToolEvent::None)
            }
            ToolKind::PushPull => {
                let d = self.drag.take().ok_or("click a face first")?;
                Ok(ToolEvent::Commit(
                    PushPullFace {
                        element: d.element,
                        face: d.face,
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
                self.move_base = None;
                Ok(ToolEvent::Commit(self.move_cmd(ctx, dir * vals[0])))
            }
            _ => Ok(ToolEvent::None),
        }
    }

    /// Polyline being drawn (polygon tool) plus the rubber band to the cursor.
    pub fn rubber_band(&self, ctx: &ToolCtx) -> Vec<(Point3, Point3)> {
        let mut segs = Vec::new();
        for w in self.poly.windows(2) {
            segs.push((w[0], w[1]));
        }
        if let (Some(l), true) = (self.poly.last(), self.kind.is_chain()) {
            if let Some(p) = on_plane(ctx, l.z) {
                segs.push((*l, p));
            }
        }
        if let (ToolKind::Move, Some(b)) = (self.kind, self.move_base) {
            if let Some(t) = self.move_target(ctx) {
                segs.push((b, t));
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
}
