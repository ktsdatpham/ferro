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
    /// On the line from the anchor parallel to the reference edge.
    Parallel,
    /// On the horizontal line from the anchor perpendicular to the
    /// reference edge.
    Perpendicular,
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
            SnapKind::Parallel => "Parallel to Edge",
            SnapKind::Perpendicular => "Perpendicular to Edge",
            SnapKind::OnFace => "On Face",
            SnapKind::Ground => "On Ground",
        }
    }
    fn rank(self) -> u8 {
        match self {
            SnapKind::Endpoint | SnapKind::Origin => 0,
            SnapKind::Midpoint => 1,
            SnapKind::OnEdge => 2,
            SnapKind::OnAxis(_) | SnapKind::Parallel | SnapKind::Perpendicular => 3,
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
    /// Direction of the edge snapped to (on-edge and midpoint snaps): the
    /// app keeps it as the reference for parallel / perpendicular inference.
    pub edge_dir: Option<UnitVec3>,
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
    /// For midpoints: the direction of their edge.
    pub edge_dir: Option<UnitVec3>,
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
            edge_dir: None,
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
                    edge_dir: None,
                });
            }
            for (_, a, b) in g.edges() {
                s.points.push(SnapPoint {
                    point: Point3::midpoint(a, b),
                    kind: SnapKind::Midpoint,
                    element: Some(e.id),
                    edge_dir: (b - a).normalize(),
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
    /// Reference edge direction for parallel / perpendicular inference
    /// (usually the last edge the cursor hovered).
    pub reference: Option<UnitVec3>,
}

impl Default for SnapSettings {
    fn default() -> Self {
        SnapSettings {
            tolerance_px: 10.0,
            ground_z: 0.0,
            axes: true,
            reference: None,
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
    // Points and edges a little behind the hovered face (its own boundary
    // seen at a grazing angle) still count as visible: the slack grows with
    // the viewing distance (0.3 %), like the screen-space tolerance does.
    let depth_limit = hit.map(|(t, _)| t * 1.003 + 1.0).unwrap_or(f64::INFINITY);
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
                    edge_dir: p.edge_dir,
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
                    edge_dir: u.normalize(),
                },
            );
        }
    }
    // 3. Axes from the anchor.
    if let (Some(a), true) = (anchor, settings.axes) {
        let mut lines: Vec<(SnapKind, Vec3)> = [Axis::X, Axis::Y, Axis::Z]
            .into_iter()
            .map(|ax| (SnapKind::OnAxis(ax), ax.dir().as_vec()))
            .collect();
        if let Some(r) = settings.reference {
            let off_axis = |d: Vec3| {
                [Vec3::X, Vec3::Y, Vec3::Z]
                    .iter()
                    .all(|ax| d.dot(*ax).abs() < 1.0 - ferro_core::math::tol::ANGLE_TOL)
            };
            let r = r.as_vec();
            if off_axis(r) {
                lines.push((SnapKind::Parallel, r));
            }
            if let Some(p) = r.cross(Vec3::Z).normalize() {
                if off_axis(p.as_vec()) {
                    lines.push((SnapKind::Perpendicular, p.as_vec()));
                }
            }
        }
        for (kind, dir) in lines {
            // Lines run both ways from the anchor.
            let Some((q, _)) = closest_on_line(ray, a, dir) else {
                continue;
            };
            if q.is_almost_equal(a) {
                continue;
            }
            if let Some(d) = screen_dist(proj, q, cursor).filter(|d| *d <= tol) {
                consider(
                    kind.rank(),
                    d,
                    SnapResult {
                        point: q,
                        kind,
                        element: None,
                        face: None,
                        normal: None,
                        edge_dir: None,
                    },
                );
            }
        }
    }
    if let Some((_, _, mut r)) = best {
        // A point or edge of the element under the cursor also reports the
        // face being hovered, so tools know which face the user means.
        if let Some((_, i)) = hit {
            let tri = &scene.triangles[i];
            if r.element == Some(tri.element) && r.face.is_none() {
                r.face = Some(tri.face);
                r.normal = Some(tri.normal);
            }
        }
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
            edge_dir: None,
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
                edge_dir: None,
            });
        }
    }
    None
}

#[cfg(test)]
mod edge_visibility_tests {
    use super::*;

    struct Ortho;
    impl Projector for Ortho {
        fn project(&self, p: Point3) -> Option<[f32; 2]> {
            // Looking along +Y: screen x = model x, screen y = -model z.
            Some([p.x as f32, -p.z as f32])
        }
    }

    #[test]
    fn the_edge_of_the_hovered_face_wins_over_the_face() {
        // A wall face at y = 0 (facing -y); the bottom edge sits 5 mm below
        // where the ray hits the face, within the pixel tolerance.
        let scene = SnapScene {
            points: vec![],
            edges: vec![SnapEdge {
                a: Point3::new(0.0, 0.0, 0.0),
                b: Point3::new(1000.0, 0.0, 0.0),
                element: ElementId(1),
            }],
            triangles: vec![SnapTriangle {
                tri: [
                    Point3::new(0.0, 0.0, 0.0),
                    Point3::new(1000.0, 0.0, 0.0),
                    Point3::new(0.0, 0.0, 1000.0),
                ],
                element: ElementId(1),
                face: FaceId(1),
                normal: -UnitVec3::Y,
            }],
        };
        // A ray coming down at an angle, hitting the face at z = 5.
        let dir = Vec3::new(0.0, 1.0, -0.6).normalize().unwrap();
        let hit = Point3::new(500.0, 0.0, 5.0);
        let ray = Ray {
            origin: hit - dir.as_vec() * 10_000.0,
            dir,
        };
        let r = snap(
            &scene,
            &ray,
            [500.0, -5.0],
            &Ortho,
            None,
            &SnapSettings::default(),
        )
        .unwrap();
        assert_eq!(r.kind, SnapKind::OnEdge);
        assert_eq!(r.face, Some(FaceId(1)), "the hovered face is reported too");
    }
}

#[cfg(test)]
mod inference_tests {
    use super::*;

    /// Plan view: screen x = model x, screen y = -model y.
    struct Top;
    impl Projector for Top {
        fn project(&self, p: Point3) -> Option<[f32; 2]> {
            Some([p.x as f32, -p.y as f32])
        }
    }

    fn at(x: f64, y: f64, reference: Option<Vec3>) -> SnapResult {
        let ray = Ray {
            origin: Point3::new(x, y, 10_000.0),
            dir: -UnitVec3::Z,
        };
        let settings = SnapSettings {
            reference: reference.and_then(|r| r.normalize()),
            ..Default::default()
        };
        let anchor = Some(Point3::new(0.0, 0.0, 0.0));
        snap(
            &SnapScene::default(),
            &ray,
            [x as f32, -y as f32],
            &Top,
            anchor,
            &settings,
        )
        .unwrap()
    }

    #[test]
    fn parallel_and_perpendicular_to_the_reference_edge() {
        let r = Some(Vec3::new(1.0, 1.0, 0.0));
        // Near the 45° line: parallel, snapped exactly onto it.
        let s = at(1000.0, 1004.0, r);
        assert_eq!(s.kind, SnapKind::Parallel);
        assert!((s.point.x - s.point.y).abs() < 1e-6);
        // Near the -45° line, either side of the anchor: perpendicular.
        for (x, y) in [(1000.0, -996.0), (-1000.0, 1003.0)] {
            let s = at(x, y, r);
            assert_eq!(s.kind, SnapKind::Perpendicular);
            assert!((s.point.x + s.point.y).abs() < 1e-6);
        }
    }

    #[test]
    fn no_reference_or_axis_aligned_reference_gives_only_axes() {
        assert_eq!(at(1000.0, 1004.0, None).kind, SnapKind::Ground);
        // An edge along X: parallel and perpendicular are the red and green axes.
        let x = Some(Vec3::X);
        assert_eq!(at(1000.0, 3.0, x).kind, SnapKind::OnAxis(Axis::X));
        assert_eq!(at(3.0, -1000.0, x).kind, SnapKind::OnAxis(Axis::Y));
        assert_eq!(at(1000.0, 1004.0, x).kind, SnapKind::Ground);
        // Just outside the pixel tolerance: no inference.
        assert_eq!(
            at(1000.0, 1015.0, Some(Vec3::new(1.0, 1.0, 0.0))).kind,
            SnapKind::Ground
        );
    }
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
