//! Inference engine (D-60): finds the point the user means under the cursor.
//!
//! Priority (Foundation Spec D-60): points (endpoint, origin, midpoint) >
//! edges > axis directions from the anchor > faces > ground / work plane.
//! Snap tolerance is in **screen pixels** and never mixed with kernel
//! tolerances (D-06). Pure and deterministic; AI tools can use it too.

use ferro_core::algo::body_mesh;
use ferro_core::model::{Document, ElementId};
use ferro_core::topo::FaceId;
use ferro_core::{Point3, UnitVec3, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn dir(self) -> UnitVec3 {
        match self {
            Axis::X => UnitVec3::X,
            Axis::Y => UnitVec3::Y,
            Axis::Z => UnitVec3::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Origin,
    Midpoint,
    OnEdge,
    OnAxis(Axis),
    OnFace,
    Ground,
}

impl SnapKind {
    /// Label shown next to the cursor (SketchUp style).
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "Endpoint",
            SnapKind::Origin => "Origin",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::OnEdge => "On Edge",
            SnapKind::OnAxis(Axis::X) => "On Red Axis",
            SnapKind::OnAxis(Axis::Y) => "On Green Axis",
            SnapKind::OnAxis(Axis::Z) => "On Blue Axis",
            SnapKind::OnFace => "On Face",
            SnapKind::Ground => "On Ground",
        }
    }
    fn rank(self) -> u8 {
        match self {
            SnapKind::Endpoint | SnapKind::Origin => 0,
            SnapKind::Midpoint => 1,
            SnapKind::OnEdge => 2,
            SnapKind::OnAxis(_) => 3,
            SnapKind::OnFace => 4,
            SnapKind::Ground => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapResult {
    pub point: Point3,
    pub kind: SnapKind,
    pub element: Option<ElementId>,
    pub face: Option<FaceId>,
    /// Normal of the face under the cursor, when there is one.
    pub normal: Option<UnitVec3>,
}

/// A picking ray in model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Point3,
    pub dir: UnitVec3,
}

impl Ray {
    pub fn at(&self, t: f64) -> Point3 {
        self.origin + self.dir * t
    }
    /// Parameter of the point on the ray closest to `p`.
    pub fn param_of(&self, p: Point3) -> f64 {
        (p - self.origin).dot(self.dir.as_vec())
    }
}

/// Maps model points to screen pixels (supplied by the viewport camera).
pub trait Projector {
    fn project(&self, p: Point3) -> Option<[f32; 2]>;
}

#[derive(Debug, Clone)]
pub struct SnapPoint {
    pub point: Point3,
    pub kind: SnapKind,
    pub element: Option<ElementId>,
}

#[derive(Debug, Clone)]
pub struct SnapEdge {
    pub a: Point3,
    pub b: Point3,
    pub element: ElementId,
}

#[derive(Debug, Clone)]
pub struct SnapTriangle {
    pub tri: [Point3; 3],
    pub element: ElementId,
    pub face: FaceId,
    pub normal: UnitVec3,
}

/// Snappable geometry extracted from a document.
#[derive(Debug, Clone, Default)]
pub struct SnapScene {
    pub points: Vec<SnapPoint>,
    pub edges: Vec<SnapEdge>,
    pub triangles: Vec<SnapTriangle>,
}

impl SnapScene {
    /// Build from all Forms except `exclude` (e.g. the element being moved).
    pub fn from_document(doc: &Document, exclude: &[ElementId]) -> SnapScene {
        let mut s = SnapScene::default();
        s.points.push(SnapPoint {
            point: Point3::ORIGIN,
            kind: SnapKind::Origin,
            element: None,
        });
        for e in doc.elements() {
            if exclude.contains(&e.id) {
                continue;
            }
            let Some(g) = e.geometry() else { continue };
            for (_, p) in g.vertices() {
                s.points.push(SnapPoint {
                    point: p,
                    kind: SnapKind::Endpoint,
                    element: Some(e.id),
                });
            }
            for (_, a, b) in g.edges() {
                s.points.push(SnapPoint {
                    point: Point3::midpoint(a, b),
                    kind: SnapKind::Midpoint,
                    element: Some(e.id),
                });
                s.edges.push(SnapEdge {
                    a,
                    b,
                    element: e.id,
                });
            }
            if let Ok(m) = body_mesh(g) {
                for (k, t) in m.indices.iter().enumerate() {
                    let tri = t.map(|i| m.positions[i as usize]);
                    let face = m.triangle_faces[k];
                    let normal = g.face_normal(face).unwrap_or(UnitVec3::Z);
                    s.triangles.push(SnapTriangle {
                        tri,
                        element: e.id,
                        face,
                        normal,
                    });
                }
            }
        }
        s
    }

    /// Nearest face hit along the ray: (t, triangle index).
    pub fn raycast(&self, ray: &Ray) -> Option<(f64, usize)> {
        let mut best: Option<(f64, usize)> = None;
        for (i, t) in self.triangles.iter().enumerate() {
            if let Some(d) = ray_triangle(ray, &t.tri) {
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, i));
                }
            }
        }
        best
    }
}

/// Snap settings (UI-level).
#[derive(Debug, Clone, Copy)]
pub struct SnapSettings {
    /// Screen-space tolerance in pixels.
    pub tolerance_px: f32,
    /// Elevation of the ground / active work plane (mm).
    pub ground_z: f64,
    /// Infer axis directions from the anchor point.
    pub axes: bool,
}

impl Default for SnapSettings {
    fn default() -> Self {
        SnapSettings {
            tolerance_px: 10.0,
            ground_z: 0.0,
            axes: true,
        }
    }
}

fn screen_dist(proj: &dyn Projector, p: Point3, cursor: [f32; 2]) -> Option<f32> {
    let s = proj.project(p)?;
    Some(((s[0] - cursor[0]).powi(2) + (s[1] - cursor[1]).powi(2)).sqrt())
}

/// Möller–Trumbore ray / triangle intersection; returns the ray parameter.
pub fn ray_triangle(ray: &Ray, tri: &[Point3; 3]) -> Option<f64> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let d = ray.dir.as_vec();
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = ray.origin - tri[0];
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-9).then_some(t)
}

/// Closest points between a ray and an infinite line through `a` along `u`.
/// Returns the point on the line, or `None` when parallel.
fn closest_on_line(ray: &Ray, a: Point3, u: Vec3) -> Option<(Point3, f64)> {
    let d = ray.dir.as_vec();
    let w = ray.origin - a;
    let (uu, ud, dd) = (u.dot(u), u.dot(d), d.dot(d));
    let (uw, dw) = (u.dot(w), d.dot(w));
    let den = uu * dd - ud * ud;
    if den.abs() < 1e-12 {
        return None;
    }
    let s = (dd * uw - ud * dw) / den; // along line
    let t = (ud * uw - uu * dw) / den; // along ray
    Some((a + u * s, t))
}

/// Find the snapped point for a cursor position.
pub fn snap(
    scene: &SnapScene,
    ray: &Ray,
    cursor: [f32; 2],
    proj: &dyn Projector,
    anchor: Option<Point3>,
    settings: &SnapSettings,
) -> Option<SnapResult> {
    let hit = scene.raycast(ray);
    let depth_limit = hit
        .map(|(t, _)| t * (1.0 + 1e-6) + 1.0)
        .unwrap_or(f64::INFINITY);
    let visible = |p: Point3| ray.param_of(p) <= depth_limit;
    let tol = settings.tolerance_px;

    // 1. Points.
    let mut best: Option<(u8, f32, SnapResult)> = None;
    let mut consider = |rank: u8, dist: f32, r: SnapResult| {
        if best.is_none_or(|(br, bd, _)| (rank, dist) < (br, bd)) {
            best = Some((rank, dist, r));
        }
    };
    for p in &scene.points {
        if !visible(p.point) {
            continue;
        }
        if let Some(d) = screen_dist(proj, p.point, cursor).filter(|d| *d <= tol) {
            consider(
                p.kind.rank(),
                d,
                SnapResult {
                    point: p.point,
                    kind: p.kind,
                    element: p.element,
                    face: None,
                    normal: None,
                },
            );
        }
    }
    // 2. Edges.
    for e in &scene.edges {
        let u = e.b - e.a;
        let Some((q, _)) = closest_on_line(ray, e.a, u) else {
            continue;
        };
        let s = (q - e.a).dot(u) / u.length_squared();
        if !(0.0..=1.0).contains(&s) || !visible(q) {
            continue;
        }
        if let Some(d) = screen_dist(proj, q, cursor).filter(|d| *d <= tol) {
            consider(
                SnapKind::OnEdge.rank(),
                d,
                SnapResult {
                    point: q,
                    kind: SnapKind::OnEdge,
                    element: Some(e.element),
                    face: None,
                    normal: None,
                },
            );
        }
    }
    // 3. Axes from the anchor.
    if let (Some(a), true) = (anchor, settings.axes) {
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let Some((q, t)) = closest_on_line(ray, a, axis.dir().as_vec()) else {
                continue;
            };
            if t <= 0.0 || q.is_almost_equal(a) {
                continue;
            }
            if let Some(d) = screen_dist(proj, q, cursor).filter(|d| *d <= tol) {
                consider(
                    SnapKind::OnAxis(axis).rank(),
                    d,
                    SnapResult {
                        point: q,
                        kind: SnapKind::OnAxis(axis),
                        element: None,
                        face: None,
                        normal: None,
                    },
                );
            }
        }
    }
    if let Some((_, _, r)) = best {
        return Some(r);
    }
    // 4. Face under the cursor.
    if let Some((t, i)) = hit {
        let tri = &scene.triangles[i];
        return Some(SnapResult {
            point: ray.at(t),
            kind: SnapKind::OnFace,
            element: Some(tri.element),
            face: Some(tri.face),
            normal: Some(tri.normal),
        });
    }
    // 5. Ground / work plane.
    let dz = ray.dir.as_vec().z;
    if dz.abs() > 1e-9 {
        let t = (settings.ground_z - ray.origin.z) / dz;
        if t > 0.0 {
            return Some(SnapResult {
                point: ray.at(t),
                kind: SnapKind::Ground,
                element: None,
                face: None,
                normal: Some(UnitVec3::Z),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;

    /// Orthographic top view: 1 px = 10 mm, screen y down = model −y.
    struct Top;
    impl Projector for Top {
        fn project(&self, p: Point3) -> Option<[f32; 2]> {
            Some([(p.x / 10.0) as f32, (-p.y / 10.0) as f32])
        }
    }
    fn ray_at(x: f64, y: f64) -> Ray {
        Ray {
            origin: Point3::new(x, y, 100_000.0),
            dir: -UnitVec3::Z,
        }
    }
    fn cursor(x: f64, y: f64) -> [f32; 2] {
        Top.project(Point3::new(x, y, 0.0)).unwrap()
    }

    fn scene() -> SnapScene {
        let mut s = Session::new(
            ferro_core::Document::new("t"),
            Box::new(SequentialIds::new(1)),
        );
        s.execute(
            AnyCommand::from_tool(
                "create_box",
                &serde_json::json!({"origin":{"x":1000,"y":1000,"z":0},"width":4000,"depth":3000,"height":3000}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        SnapScene::from_document(s.doc(), &[])
    }

    #[test]
    fn snaps_to_endpoint_near_corner() {
        let sc = scene();
        let r = snap(
            &sc,
            &ray_at(1050.0, 1040.0),
            cursor(1050.0, 1040.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::Endpoint);
        assert!(r.point.is_almost_equal(Point3::new(1000.0, 1000.0, 3000.0)));
    }

    #[test]
    fn snaps_to_midpoint_and_edge() {
        let sc = scene();
        let r = snap(
            &sc,
            &ray_at(3000.0, 1030.0),
            cursor(3000.0, 1030.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::Midpoint);
        let r = snap(
            &sc,
            &ray_at(2000.0, 1030.0),
            cursor(2000.0, 1030.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::OnEdge);
        assert!((r.point.y - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn face_then_ground() {
        let sc = scene();
        let r = snap(
            &sc,
            &ray_at(3000.0, 2500.0),
            cursor(3000.0, 2500.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::OnFace);
        assert!((r.point.z - 3000.0).abs() < 1e-6);
        assert!(r.normal.unwrap().as_vec().is_almost_equal(Vec3::Z));
        let r = snap(
            &sc,
            &ray_at(-5000.0, -5000.0),
            cursor(-5000.0, -5000.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::Ground);
        assert!(r.point.z.abs() < 1e-6);
    }

    #[test]
    fn axis_inference_from_anchor() {
        let sc = SnapScene::default();
        let anchor = Point3::new(0.0, 0.0, 0.0);
        // Slightly off the X axis, far from any geometry: ray straight down,
        // so the X-axis point is directly below if within tolerance.
        let r = snap(
            &sc,
            &ray_at(5000.0, 40.0),
            cursor(5000.0, 40.0),
            &Top,
            Some(anchor),
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::OnAxis(Axis::X));
        assert!(r.point.y.abs() < 1e-6);
    }

    #[test]
    fn hidden_points_do_not_snap() {
        let sc = scene();
        // Bottom corner at (1000,1000,0) is under the box top: hidden from above.
        let mut hidden_only = sc.clone();
        hidden_only.points.retain(|p| p.point.z < 1.0);
        hidden_only.edges.clear();
        let r = snap(
            &hidden_only,
            &ray_at(1010.0, 1010.0),
            cursor(1010.0, 1010.0),
            &Top,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::OnFace);
    }
}
