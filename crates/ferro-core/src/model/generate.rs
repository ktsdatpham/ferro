//! Regeneration of derived BIM geometry (Foundation Spec §4.4, D-41).
//!
//! Walls, floors and openings store their parameters; their `geometry` is
//! rebuilt from those parameters, the types they use and the levels they sit
//! on. Wall ends join as `model::join` infers them (L mitre or butt, T);
//! openings cut their host wall; `join` and `cut` rows apply through the
//! kernel. The output is deterministic: the same
//! document always yields the same bodies with the same ids.

use super::bim::{CompoundStructure, Floor, LocationLine, OpeningKind, Wall, WallTop};
use super::join::{self, EndCorners, StraightWall};
use super::{cut, Category, Document, Element, ElementBody, ElementId};
use crate::algo::{extrude_profile, Kernel};
use crate::geom::{polygon, Arc, Curve, CurveLoop, Line, Plane, Profile};
use crate::math::{tol, Length, Point2, Point3, Vec2, Vec3};
use crate::topo::{Body, FaceSpec, VertexId};
use std::collections::{BTreeMap, HashMap};

/// Default wall thickness when a wall has no type (mm).
pub const DEFAULT_WALL_THICKNESS: f64 = 200.0;
/// Default floor thickness when a floor has no type (mm).
pub const DEFAULT_FLOOR_THICKNESS: f64 = 200.0;
/// Two wall ends closer than this (mm) are joined.
pub const JOIN_TOL: f64 = 1.0;
/// Smallest gap kept between openings and wall ends (mm).
const OPENING_MARGIN: f64 = 1.0;
/// Sills below this (mm) make a notch (door) instead of a hole.
const NOTCH_SILL: f64 = tol::SHORT_CURVE_TOL;

/// A problem found while regenerating. Errors roll the transaction back;
/// warnings are reported and the element keeps going (Foundation Spec §4.5).
#[derive(Debug, Clone, PartialEq)]
pub struct GenIssue {
    pub code: &'static str,
    pub message: String,
    pub element: ElementId,
    pub is_error: bool,
}

impl GenIssue {
    pub(crate) fn error(
        code: &'static str,
        message: impl Into<String>,
        element: ElementId,
    ) -> Self {
        GenIssue {
            code,
            message: message.into(),
            element,
            is_error: true,
        }
    }
    pub(crate) fn warning(
        code: &'static str,
        message: impl Into<String>,
        element: ElementId,
    ) -> Self {
        GenIssue {
            code,
            message: message.into(),
            element,
            is_error: false,
        }
    }
}

/// Result of [`regenerate`]: new geometry for elements whose geometry changed.
#[derive(Debug, Default)]
pub struct Regenerated {
    /// New geometry of walls, floors and openings (cuts applied), by id.
    pub updates: Vec<(ElementId, Body)>,
    /// New derived `cut_body` of Forms (`None` = uncut), by id.
    pub cut_updates: Vec<(ElementId, Option<Body>)>,
    /// Generation problems: errors roll back, warnings are reported.
    pub issues: Vec<GenIssue>,
    /// Problems of `cut` rows (`ferro.kernel.*`): always warnings.
    pub cut_issues: Vec<GenIssue>,
}

/// Elevation (mm) of a level element; 0 when missing.
pub fn level_elevation(doc: &Document, level: Option<ElementId>) -> f64 {
    level
        .and_then(|l| doc.get(l))
        .and_then(|e| e.as_level())
        .map(|l| l.elevation.mm())
        .unwrap_or(0.0)
}

/// Compound structure of a wall from its type (default single 200 mm layer).
pub fn wall_structure(doc: &Document, e: &Element) -> CompoundStructure {
    match e.type_id.and_then(|t| doc.get(t)).map(|t| &t.body) {
        Some(ElementBody::WallType(t)) => t.structure.clone(),
        _ => CompoundStructure::single(Length::from_mm(DEFAULT_WALL_THICKNESS)),
    }
}

/// Compound structure of a floor from its type (default single 200 mm layer).
pub fn floor_structure(doc: &Document, e: &Element) -> CompoundStructure {
    match e.type_id.and_then(|t| doc.get(t)).map(|t| &t.body) {
        Some(ElementBody::FloorType(t)) => t.structure.clone(),
        _ => CompoundStructure::single(Length::from_mm(DEFAULT_FLOOR_THICKNESS)),
    }
}

/// Resolved placement of a wall: offsets of its faces from the location
/// curve (positive = left / exterior) and its vertical extent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallLayout {
    pub curve: Curve,
    /// Offset of the exterior face (left of the curve), mm.
    pub ext: f64,
    /// Offset of the interior face, mm (`ext - thickness`).
    pub int: f64,
    /// Bottom and top elevations, mm.
    pub z0: f64,
    pub z1: f64,
}

impl WallLayout {
    pub fn thickness(&self) -> f64 {
        self.ext - self.int
    }
    pub fn height(&self) -> f64 {
        self.z1 - self.z0
    }
    /// Straight walls only: start, unit direction and left normal in plan.
    pub fn frame(&self) -> Option<(Point2, Vec2, Vec2)> {
        let Curve::Line(l) = self.curve else {
            return None;
        };
        let s = Point2::new(l.start().x, l.start().y);
        let e = Point2::new(l.end().x, l.end().y);
        let d = e - s;
        let len = d.length();
        if len < tol::SHORT_CURVE_TOL {
            return None;
        }
        let d = d * (1.0 / len);
        Some((s, d, Vec2::new(-d.y, d.x)))
    }
    /// Plan length of the location curve, mm.
    pub fn length(&self) -> f64 {
        match self.curve {
            Curve::Line(l) => {
                let (a, b) = (l.start(), l.end());
                Vec2::new(b.x - a.x, b.y - a.y).length()
            }
            Curve::Arc(a) => a.length(),
        }
    }
}

/// Placement of a wall element. Fails with `ferro.wall.*` codes.
pub fn wall_layout(doc: &Document, e: &Element, w: &Wall) -> Result<WallLayout, GenIssue> {
    let st = wall_structure(doc, e);
    let t = st.thickness();
    if t < tol::SHORT_CURVE_TOL {
        return Err(GenIssue::error(
            "ferro.wall.thin",
            "wall type has no thickness",
            e.id,
        ));
    }
    let ext = match w.location_line {
        LocationLine::WallCenterline => t * 0.5,
        LocationLine::CoreCenterline => st.core_center_from_exterior(),
        LocationLine::FinishExterior => 0.0,
        LocationLine::FinishInterior => t,
    };
    let base = level_elevation(doc, e.level_id);
    let z0 = base + w.base_offset.mm();
    let z1 = match &w.top {
        WallTop::Unconnected { height } => z0 + height.mm(),
        WallTop::Level { level, offset } => {
            if doc.get(*level).and_then(|l| l.as_level()).is_none() {
                return Err(GenIssue::error(
                    "ferro.wall.top_level_missing",
                    format!("top level {level} does not exist"),
                    e.id,
                ));
            }
            level_elevation(doc, Some(*level)) + offset.mm()
        }
    };
    if z1 - z0 < tol::SHORT_CURVE_TOL {
        return Err(GenIssue::error(
            "ferro.wall.no_height",
            "wall top must be above its base",
            e.id,
        ));
    }
    if let Curve::Arc(a) = w.curve {
        if !a.normal().is_parallel(crate::math::UnitVec3::Z) {
            return Err(GenIssue::error(
                "ferro.wall.not_horizontal",
                "wall curve must lie in a horizontal plane",
                e.id,
            ));
        }
    }
    let lay = WallLayout {
        curve: w.curve,
        ext,
        int: ext - t,
        z0,
        z1,
    };
    if lay.length() < tol::SHORT_CURVE_TOL {
        return Err(GenIssue::error(
            "ferro.wall.too_short",
            "wall is too short",
            e.id,
        ));
    }
    Ok(lay)
}

// ------------------------------------------------------------------ builder

/// Collects faces given as world-space loops and builds a body, merging
/// coincident vertices and orienting loops by the intended outward normal.
#[derive(Default)]
struct Builder {
    points: Vec<Point3>,
    index: HashMap<(i64, i64, i64), usize>,
    faces: Vec<(Vec<usize>, Vec<Vec<usize>>, Vec3)>,
}

impl Builder {
    fn vertex(&mut self, p: Point3) -> usize {
        // Quantise to 1 µm: all points are produced by this module, so equal
        // points are computed identically or differ by rounding noise only.
        let k = (
            (p.x * 1000.0).round() as i64,
            (p.y * 1000.0).round() as i64,
            (p.z * 1000.0).round() as i64,
        );
        let n = self.points.len();
        *self.index.entry(k).or_insert_with(|| {
            self.points.push(p);
            n
        })
    }

    fn face(&mut self, outer: &[Point3], holes: &[Vec<Point3>], normal: Vec3) {
        let outer: Vec<usize> = outer.iter().map(|p| self.vertex(*p)).collect();
        let holes = holes
            .iter()
            .map(|h| h.iter().map(|p| self.vertex(*p)).collect())
            .collect();
        self.faces.push((outer, holes, normal));
    }

    fn build(self) -> Result<Body, crate::topo::TopoError> {
        let mut body = Body::default();
        let vids: Vec<VertexId> = self.points.iter().map(|p| body.add_vertex(*p)).collect();
        let orient = |lp: &[usize], normal: Vec3, ccw: bool| -> Vec<VertexId> {
            let pts: Vec<Point3> = lp.iter().map(|&i| self.points[i]).collect();
            let same = polygon::polygon_normal(&pts)
                .map(|n| n.as_vec().dot(normal) >= 0.0)
                .unwrap_or(true);
            let mut v: Vec<VertexId> = lp.iter().map(|&i| vids[i]).collect();
            if same != ccw {
                v.reverse();
            }
            v
        };
        let specs = self
            .faces
            .iter()
            .map(|(o, hs, n)| {
                let mut s = FaceSpec::new(orient(o, *n, true));
                s.holes = hs.iter().map(|h| orient(h, *n, false)).collect();
                s
            })
            .collect();
        let vertices = body.vertex_map();
        body.rebuild(vertices, specs)?;
        Ok(body)
    }

    /// Axis-aligned box in a local frame (origin, u, v unit plan axes, z up).
    #[allow(clippy::too_many_arguments)]
    fn frame_box(
        &mut self,
        o: Point2,
        u: Vec2,
        v: Vec2,
        (u0, u1): (f64, f64),
        (v0, v1): (f64, f64),
        (z0, z1): (f64, f64),
    ) {
        let p = |a: f64, b: f64, z: f64| {
            let q = o + u * a + v * b;
            Point3::new(q.x, q.y, z)
        };
        let u3 = Vec3::new(u.x, u.y, 0.0);
        let v3 = Vec3::new(v.x, v.y, 0.0);
        let z3 = Vec3::new(0.0, 0.0, 1.0);
        let c = |a, b, z| p(a, b, z);
        self.face(
            &[c(u0, v0, z0), c(u1, v0, z0), c(u1, v1, z0), c(u0, v1, z0)],
            &[],
            -z3,
        );
        self.face(
            &[c(u0, v0, z1), c(u1, v0, z1), c(u1, v1, z1), c(u0, v1, z1)],
            &[],
            z3,
        );
        self.face(
            &[c(u0, v0, z0), c(u1, v0, z0), c(u1, v0, z1), c(u0, v0, z1)],
            &[],
            -v3,
        );
        self.face(
            &[c(u0, v1, z0), c(u1, v1, z0), c(u1, v1, z1), c(u0, v1, z1)],
            &[],
            v3,
        );
        self.face(
            &[c(u0, v0, z0), c(u0, v1, z0), c(u0, v1, z1), c(u0, v0, z1)],
            &[],
            -u3,
        );
        self.face(
            &[c(u1, v0, z0), c(u1, v1, z0), c(u1, v1, z1), c(u1, v0, z1)],
            &[],
            u3,
        );
    }
}

fn topo_issue(e: crate::topo::TopoError, id: ElementId) -> GenIssue {
    GenIssue::error(e.code(), e.to_string(), id)
}

// -------------------------------------------------------------------- walls

/// An opening resolved against its host, in wall-local coordinates.
#[derive(Debug, Clone, Copy)]
struct Cut {
    id: ElementId,
    kind: OpeningKind,
    a: f64,
    b: f64,
    /// Bottom and top above the wall base.
    zb: f64,
    zt: f64,
}

impl Cut {
    fn is_notch(&self) -> bool {
        self.zb < NOTCH_SILL
    }
}

fn resolve_cuts(
    doc: &Document,
    wall: &Element,
    lay: &WallLayout,
    range: (f64, f64),
    issues: &mut Vec<GenIssue>,
) -> Vec<Cut> {
    let mut cuts: Vec<Cut> = Vec::new();
    for e in doc.elements().filter(|e| e.host_id == Some(wall.id)) {
        let ElementBody::Opening(o) = &e.body else {
            continue;
        };
        if lay.frame().is_none() {
            issues.push(GenIssue::warning(
                "ferro.opening.curved_host",
                "openings in curved walls are not supported yet",
                e.id,
            ));
            continue;
        }
        let (w, h) = (o.width.mm(), o.height.mm());
        let sill = if o.sill.mm() < NOTCH_SILL {
            0.0
        } else {
            o.sill.mm()
        };
        let c = Cut {
            id: e.id,
            kind: o.kind,
            a: o.position.mm() - w * 0.5,
            b: o.position.mm() + w * 0.5,
            zb: sill,
            zt: sill + h,
        };
        if w < tol::SHORT_CURVE_TOL || h < tol::SHORT_CURVE_TOL {
            issues.push(GenIssue::warning(
                "ferro.opening.too_small",
                "opening width and height must be positive",
                e.id,
            ));
        } else if c.a < range.0 + OPENING_MARGIN || c.b > range.1 - OPENING_MARGIN {
            issues.push(GenIssue::warning(
                "ferro.opening.outside_host",
                format!("{} does not fit along its wall", e.label()),
                e.id,
            ));
        } else if c.zt > lay.height() - OPENING_MARGIN {
            issues.push(GenIssue::warning(
                "ferro.opening.too_tall",
                format!("{} is taller than its wall", e.label()),
                e.id,
            ));
        } else if cuts
            .iter()
            .any(|o| c.a < o.b + OPENING_MARGIN && o.a < c.b + OPENING_MARGIN)
        {
            issues.push(GenIssue::warning(
                "ferro.opening.overlap",
                format!("{} overlaps another opening", e.label()),
                e.id,
            ));
        } else {
            cuts.push(c);
        }
    }
    cuts.sort_by(|x, y| x.a.total_cmp(&y.a));
    cuts
}

/// Build a straight wall body with mitred ends and openings. Returns the
/// body and the accepted cuts.
fn straight_wall(
    lay: &WallLayout,
    ends: [Option<EndCorners>; 2],
    cuts_for: impl FnOnce((f64, f64)) -> Vec<Cut>,
    id: ElementId,
) -> Result<(Body, Vec<Cut>), GenIssue> {
    let (s, d, n) = lay
        .frame()
        .ok_or_else(|| GenIssue::error("ferro.wall.too_short", "wall is too short", id))?;
    let len = lay.length();
    let u_of = |p: Point2| (p - s).dot(d);
    // u ranges of the exterior and interior faces.
    let (mut ue0, mut ui0) = (0.0, 0.0);
    let (mut ue1, mut ui1) = (len, len);
    if let Some((pe, pi)) = ends[0] {
        ue0 = u_of(pe);
        ui0 = u_of(pi);
    }
    if let Some((pe, pi)) = ends[1] {
        ue1 = u_of(pe);
        ui1 = u_of(pi);
    }
    if ue1 - ue0 < tol::SHORT_CURVE_TOL || ui1 - ui0 < tol::SHORT_CURVE_TOL {
        // Mitres crossed over: fall back to square ends.
        (ue0, ui0, ue1, ui1) = (0.0, 0.0, len, len);
    }
    let range = (ue0.max(ui0), ue1.min(ui1));
    let cuts = cuts_for(range);

    let (z0, z1) = (lay.z0, lay.z1);
    let at = |u: f64, v: f64, z: f64| {
        let q = s + d * u + n * v;
        Point3::new(q.x, q.y, z)
    };
    let ext = |u: f64, z: f64| at(u, lay.ext, z);
    let int = |u: f64, z: f64| at(u, lay.int, z);
    let n3 = Vec3::new(n.x, n.y, 0.0);
    let d3 = Vec3::new(d.x, d.y, 0.0);
    let up = Vec3::new(0.0, 0.0, 1.0);

    let mut b = Builder::default();
    // Side faces: outer loop with notches along the bottom, holes for windows.
    let side = |b: &mut Builder, f: &dyn Fn(f64, f64) -> Point3, u0: f64, u1: f64, normal: Vec3| {
        let mut outer = vec![f(u0, z0)];
        for c in cuts.iter().filter(|c| c.is_notch()) {
            outer.extend([f(c.a, z0), f(c.a, z0 + c.zt), f(c.b, z0 + c.zt), f(c.b, z0)]);
        }
        outer.extend([f(u1, z0), f(u1, z1), f(u0, z1)]);
        let holes: Vec<Vec<Point3>> = cuts
            .iter()
            .filter(|c| !c.is_notch())
            .map(|c| {
                vec![
                    f(c.a, z0 + c.zb),
                    f(c.b, z0 + c.zb),
                    f(c.b, z0 + c.zt),
                    f(c.a, z0 + c.zt),
                ]
            })
            .collect();
        b.face(&outer, &holes, normal);
    };
    side(&mut b, &ext, ue0, ue1, n3);
    side(&mut b, &int, ui0, ui1, -n3);
    // Top.
    b.face(
        &[ext(ue0, z1), ext(ue1, z1), int(ui1, z1), int(ui0, z1)],
        &[],
        up,
    );
    // Bottom, split by notches.
    let mut from = (ue0, ui0);
    for c in cuts.iter().filter(|c| c.is_notch()) {
        b.face(
            &[ext(from.0, z0), ext(c.a, z0), int(c.a, z0), int(from.1, z0)],
            &[],
            -up,
        );
        from = (c.b, c.b);
    }
    b.face(
        &[ext(from.0, z0), ext(ue1, z0), int(ui1, z0), int(from.1, z0)],
        &[],
        -up,
    );
    // Ends (outward normals approximately -d / +d; mitred ends are skewed).
    b.face(
        &[ext(ue0, z0), int(ui0, z0), int(ui0, z1), ext(ue0, z1)],
        &[],
        -d3,
    );
    b.face(
        &[ext(ue1, z0), int(ui1, z0), int(ui1, z1), ext(ue1, z1)],
        &[],
        d3,
    );
    // Reveals.
    for c in &cuts {
        let (zb, zt) = (z0 + c.zb, z0 + c.zt);
        b.face(
            &[ext(c.a, zb), ext(c.a, zt), int(c.a, zt), int(c.a, zb)],
            &[],
            d3,
        );
        b.face(
            &[ext(c.b, zb), ext(c.b, zt), int(c.b, zt), int(c.b, zb)],
            &[],
            -d3,
        );
        b.face(
            &[ext(c.a, zt), ext(c.b, zt), int(c.b, zt), int(c.a, zt)],
            &[],
            -up,
        );
        if !c.is_notch() {
            b.face(
                &[ext(c.a, zb), ext(c.b, zb), int(c.b, zb), int(c.a, zb)],
                &[],
                up,
            );
        }
    }
    let body = b.build().map_err(|e| topo_issue(e, id))?;
    Ok((body, cuts))
}

fn offset_point(a: &Arc, p: Point3, off: f64, z: f64) -> Point3 {
    let c = a.center();
    let r = Vec2::new(p.x - c.x, p.y - c.y);
    let r = r * (1.0 / r.length());
    // Travelling counter-clockwise (normal up), the left side is the centre.
    let left = if a.normal().as_vec().z > 0.0 {
        r * -1.0
    } else {
        r
    };
    Point3::new(p.x + left.x * off, p.y + left.y * off, z)
}

fn arc_wall(lay: &WallLayout, id: ElementId) -> Result<Body, GenIssue> {
    let Curve::Arc(a) = lay.curve else {
        return Err(GenIssue::error("ferro.wall.too_short", "not an arc", id));
    };
    let geo = |e: crate::geom::GeomError| GenIssue::error(e.code(), e.to_string(), id);
    let plane = Plane::horizontal(lay.z0);
    let offset_arc = |off: f64| -> Result<Arc, GenIssue> {
        Arc::from_three_points(
            offset_point(&a, a.start(), off, lay.z0),
            offset_point(&a, a.point_at(0.5), off, lay.z0),
            offset_point(&a, a.end(), off, lay.z0),
        )
        .map_err(geo)
    };
    let profile = if a.is_closed() {
        let c = a.center();
        let ccw = a.normal().as_vec().z > 0.0;
        let r_of = |off: f64| {
            if ccw {
                a.radius() - off
            } else {
                a.radius() + off
            }
        };
        let (r1, r2) = (r_of(lay.ext), r_of(lay.int));
        let (rmin, rmax) = (r1.min(r2), r1.max(r2));
        if rmin < tol::SHORT_CURVE_TOL {
            return Err(GenIssue::error(
                "ferro.wall.radius_too_small",
                "wall is thicker than its radius",
                id,
            ));
        }
        let circ = |r: f64| -> Result<CurveLoop, GenIssue> {
            let pl = Plane::horizontal(lay.z0);
            let pl = Plane::from_frame(
                Point3::new(c.x, c.y, lay.z0),
                pl.x_axis().as_vec(),
                pl.y_axis().as_vec(),
            )
            .map_err(geo)?;
            CurveLoop::from_curves(vec![Curve::Arc(Arc::circle(pl, r).map_err(geo)?)]).map_err(geo)
        };
        Profile::from_loops(plane, circ(rmax)?, vec![circ(rmin)?]).map_err(geo)?
    } else {
        let el = offset_arc(lay.ext)?;
        let il = offset_arc(lay.int)?;
        let l1 = Line::from_points(el.end(), il.end()).map_err(geo)?;
        let l2 = Line::from_points(il.start(), el.start()).map_err(geo)?;
        // Arcs from three points may come out reversed; orient them.
        let fwd = |arc: Arc, start: Point3| {
            if arc.start().is_almost_equal(start) {
                Curve::Arc(arc)
            } else {
                Curve::Arc(arc.reversed())
            }
        };
        let lp = CurveLoop::from_curves(vec![
            fwd(el, offset_point(&a, a.start(), lay.ext, lay.z0)),
            Curve::Line(l1),
            fwd(il, offset_point(&a, a.end(), lay.int, lay.z0)),
            Curve::Line(l2),
        ])
        .map_err(geo)?;
        Profile::from_loops(plane, lp, vec![]).map_err(geo)?
    };
    let (body, _) = extrude_profile(&profile, lay.height())
        .map_err(|e| GenIssue::error(e.code(), e.to_string(), id))?;
    Ok(body)
}

/// Display geometry of an opening: a door leaf or a glass pane.
fn opening_body(lay: &WallLayout, c: &Cut) -> Option<Body> {
    let (s, d, n) = lay.frame()?;
    let mid = (lay.ext + lay.int) * 0.5;
    let (half, inset) = match c.kind {
        OpeningKind::Door => (20.0_f64, 10.0_f64),
        OpeningKind::Window => (10.0, 50.0),
    };
    let half = half.min(lay.thickness() * 0.25);
    let w = c.b - c.a;
    let h = c.zt - c.zb;
    let inset = if w > 4.0 * inset && h > 4.0 * inset {
        inset
    } else {
        0.0
    };
    let bottom = if c.is_notch() { 0.0 } else { inset };
    let mut b = Builder::default();
    b.frame_box(
        s,
        d,
        n,
        (c.a + inset, c.b - inset),
        (mid - half, mid + half),
        (lay.z0 + c.zb + bottom, lay.z0 + c.zt - inset),
    );
    b.build().ok()
}

// ------------------------------------------------------------------- floors

/// Build a floor body: the sketch at level + offset, extruded down by the
/// type thickness.
pub fn floor_body(doc: &Document, e: &Element, f: &Floor) -> Result<Body, GenIssue> {
    let geo = |err: crate::geom::GeomError| GenIssue::error(err.code(), err.to_string(), e.id);
    let t = floor_structure(doc, e).thickness();
    if t < tol::SHORT_CURVE_TOL {
        return Err(GenIssue::error(
            "ferro.floor.thin",
            "floor type has no thickness",
            e.id,
        ));
    }
    let z = level_elevation(doc, e.level_id) + f.offset.mm();
    let lp = |pts: &[Point2]| -> Result<CurveLoop, GenIssue> {
        let p3: Vec<Point3> = pts.iter().map(|p| Point3::new(p.x, p.y, z)).collect();
        CurveLoop::from_polygon(&p3).map_err(geo)
    };
    let holes = f
        .holes
        .iter()
        .map(|h| lp(h))
        .collect::<Result<Vec<_>, _>>()?;
    let profile = Profile::from_loops(Plane::horizontal(z), lp(&f.outer)?, holes).map_err(geo)?;
    let (body, _) = extrude_profile(&profile, -t)
        .map_err(|err| GenIssue::error(err.code(), err.to_string(), e.id))?;
    Ok(body)
}

// ------------------------------------------------------------- regenerate

/// `true` if a change to this element can affect derived BIM geometry.
pub fn affects_bim(e: &Element) -> bool {
    matches!(
        e.body,
        ElementBody::Level(_)
            | ElementBody::WallType(_)
            | ElementBody::FloorType(_)
            | ElementBody::Wall(_)
            | ElementBody::Floor(_)
            | ElementBody::Opening(_)
    )
}

/// The straight walls of `layouts` as end joins see them.
fn straight_walls_of(doc: &Document, layouts: &[(ElementId, WallLayout)]) -> Vec<StraightWall> {
    layouts
        .iter()
        .filter_map(|(id, l)| {
            let (s, d, _) = l.frame()?;
            let e = doc.get(*id)?;
            Some(StraightWall {
                id: *id,
                s,
                e: s + d * l.length(),
                d,
                ext: l.ext,
                int: l.int,
                z0: l.z0,
                z1: l.z1,
                disallow: [true, false].map(|at_start| {
                    join::wall_join_at(e, at_start) == super::bim::WallJoin::Disallow
                }),
            })
        })
        .collect()
}

/// Every straight wall of `doc` whose layout resolves, as end joins see it
/// (`model::join`).
pub(crate) fn straight_walls(doc: &Document) -> Vec<StraightWall> {
    let layouts: Vec<(ElementId, WallLayout)> = doc
        .by_category(Category::Walls)
        .filter_map(|e| Some((e.id, wall_layout(doc, e, e.as_wall()?).ok()?)))
        .collect();
    straight_walls_of(doc, &layouts)
}

/// Generated geometry of every wall, floor and opening of `doc`, before
/// cuts, with the problems found (errors and warnings).
fn generate_all(doc: &Document) -> (BTreeMap<ElementId, Body>, Vec<GenIssue>) {
    let mut out = Generated::default();
    let mut layouts: Vec<(ElementId, WallLayout)> = Vec::new();
    for e in doc.by_category(Category::Walls) {
        let Some(w) = e.as_wall() else { continue };
        match wall_layout(doc, e, w) {
            Ok(l) => layouts.push((e.id, l)),
            Err(i) => out.issues.push(i),
        }
    }
    let straight = straight_walls_of(doc, &layouts);
    let joined = join::wall_ends(doc, &straight).corners;

    let mut opening_geom: HashMap<ElementId, Body> = HashMap::new();
    for (id, lay) in &layouts {
        let Some(e) = doc.get(*id) else { continue };
        let result = match lay.curve {
            Curve::Line(_) => {
                let ends = joined.get(id).copied().unwrap_or([None, None]);
                let mut issues = Vec::new();
                let r = straight_wall(
                    lay,
                    ends,
                    |range| resolve_cuts(doc, e, lay, range, &mut issues),
                    *id,
                );
                out.issues.append(&mut issues);
                r.map(|(body, cuts)| {
                    for c in &cuts {
                        if let Some(ob) = opening_body(lay, c) {
                            opening_geom.insert(c.id, ob);
                        }
                    }
                    body
                })
            }
            Curve::Arc(_) => {
                let mut issues = Vec::new();
                resolve_cuts(doc, e, lay, (0.0, 0.0), &mut issues);
                out.issues.append(&mut issues);
                arc_wall(lay, *id)
            }
        };
        match result {
            Ok(body) => {
                out.bodies.insert(*id, body);
            }
            Err(i) => out.issues.push(i),
        }
    }

    for e in doc.elements() {
        match &e.body {
            ElementBody::Floor(f) => match floor_body(doc, e, f) {
                Ok(body) => {
                    out.bodies.insert(e.id, body);
                }
                Err(i) => out.issues.push(i),
            },
            ElementBody::Opening(_) => {
                if e.host_id
                    .and_then(|h| doc.get(h))
                    .and_then(|h| h.as_wall())
                    .is_none()
                {
                    out.issues.push(GenIssue::warning(
                        "ferro.opening.no_host",
                        format!("{} is not hosted by a wall", e.label()),
                        e.id,
                    ));
                }
                let body = opening_geom.remove(&e.id).unwrap_or_default();
                out.bodies.insert(e.id, body);
            }
            _ => {}
        }
    }
    (out.bodies, out.issues)
}

#[derive(Default)]
struct Generated {
    bodies: BTreeMap<ElementId, Body>,
    issues: Vec<GenIssue>,
}

/// Uncut bodies of every element with geometry: generated walls, floors
/// and openings (before `cut` rows), and the source body of Forms. The
/// input of cuts and of phase views (ADR 0006 §5).
pub fn uncut_bodies(doc: &Document) -> BTreeMap<ElementId, Body> {
    let (mut bodies, _) = generate_all(doc);
    for e in doc.elements() {
        if let Some(f) = e.as_form() {
            bodies.insert(e.id, f.body.clone());
        }
    }
    bodies
}

/// The uncut body of one element ([`uncut_bodies`]); `None` without
/// geometry.
pub fn uncut_body(doc: &Document, id: ElementId) -> Option<Body> {
    let e = doc.get(id)?;
    match &e.body {
        ElementBody::Form(f) => Some(f.body.clone()),
        _ if cut::cuts_of(doc, id).is_empty() && join::joins_of(doc, id).is_empty() => {
            e.geometry().cloned()
        }
        _ => generate_all(doc).0.remove(&id),
    }
}

/// Rebuild the geometry of every wall, floor and opening in `doc`, without
/// a kernel: `cut` rows are not applied (one `ferro.kernel.unavailable`
/// warning when there are any). See [`regenerate_with`].
pub fn regenerate(doc: &Document) -> Regenerated {
    regenerate_with(doc, None)
}

/// Rebuild the geometry of every wall, floor and opening in `doc`, then
/// apply the `cut` relationship rows through `kernel` to walls, floors and
/// Forms ([`cut`], M5-5.3). Only elements whose geometry (or a Form's
/// `cut_body`) differs from the stored one are returned. Cut problems are
/// warnings in `cut_issues` and never errors. [`regenerate_cached`]
/// without a cache.
pub fn regenerate_with(doc: &Document, kernel: Option<&dyn Kernel>) -> Regenerated {
    regenerate_cached(doc, kernel, None)
}

/// [`regenerate_with`] through a [`cut::CutCache`] (M5-5.7): `cut` rows
/// whose cutter body, target source body, row and earlier rows are
/// unchanged since a previous regeneration with the same cache do not
/// call the kernel. The result is the same as without the cache.
pub fn regenerate_cached(
    doc: &Document,
    kernel: Option<&dyn Kernel>,
    cache: Option<&cut::CutCache>,
) -> Regenerated {
    let (bodies, issues) = generate_all(doc);
    let mut out = Regenerated {
        issues,
        ..Default::default()
    };
    let cutter_body = |c: ElementId| -> Option<Body> {
        let e = doc.get(c)?;
        match &e.body {
            ElementBody::Form(f) => Some(f.body.clone()),
            _ => bodies.get(&c).or(e.geometry()).cloned(),
        }
    };
    let mut warn = true;
    let mut cut_body = |id: ElementId, uncut: &Body, out: &mut Regenerated| -> Option<Body> {
        let rows = cut::cuts_of(doc, id);
        if rows.is_empty() {
            return None;
        }
        let o = cut::apply_cuts_cached(kernel, id, uncut, &rows, &cutter_body, warn, cache);
        if kernel.is_none() && !o.issues.is_empty() {
            warn = false;
        }
        out.cut_issues.extend(o.issues);
        Some(o.body)
    };
    // Geometry joins (M5-5.2) apply before cuts; their warnings join the
    // cut warnings.
    let mut joiner = join::Joiner::new(doc);
    for (id, body) in &bodies {
        let Some(e) = doc.get(*id) else { continue };
        let body = match e.body {
            ElementBody::Wall(_) | ElementBody::Floor(_) => {
                let joined = joiner.apply(kernel, *id, body, &cutter_body, &mut out.cut_issues);
                let base = joined.as_ref().unwrap_or(body);
                cut_body(*id, base, &mut out)
                    .or(joined)
                    .unwrap_or_else(|| body.clone())
            }
            _ => body.clone(),
        };
        if e.geometry() != Some(&body) {
            out.updates.push((*id, body));
        }
    }
    for e in doc.elements() {
        let Some(f) = e.as_form() else { continue };
        let joined = joiner.apply(kernel, e.id, &f.body, &cutter_body, &mut out.cut_issues);
        let base = joined.as_ref().unwrap_or(&f.body);
        let new = cut_body(e.id, base, &mut out)
            .or(joined)
            .filter(|b| *b != f.body);
        if new != f.cut_body {
            out.cut_updates.push((e.id, new));
        }
    }
    out
}

/// Replace an element's derived geometry (used by `ops::Transaction`).
pub fn set_geometry(e: &mut Element, body: Body) {
    match &mut e.body {
        ElementBody::Form(f) => f.body = body,
        ElementBody::Wall(w) => w.geometry = body,
        ElementBody::Floor(f) => f.geometry = body,
        ElementBody::Opening(o) => o.geometry = body,
        _ => {}
    }
}

/// Opening geometry helper for commands: plan point and wall-local frame of
/// a position along a straight wall.
pub fn point_on_wall(lay: &WallLayout, u: f64) -> Option<Point2> {
    let (s, d, _) = lay.frame()?;
    Some(s + d * u)
}
