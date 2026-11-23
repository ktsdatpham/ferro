//! Snapping scenarios (M5-15.6): priority order, occlusion, parallel /
//! perpendicular inference with and without a reference, pixel tolerance,
//! ground plane and degenerate inputs.

use ferro_core::model::ElementId;
use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
use ferro_core::topo::FaceId;
use ferro_core::{Document, Point3, Session, UnitVec3, Vec3};
use ferro_snap::*;

/// Plan view, 1 px = 10 mm, screen y down = model -y.
struct Top;
impl Projector for Top {
    fn project(&self, p: Point3) -> Option<[f32; 2]> {
        Some([(p.x / 10.0) as f32, (-p.y / 10.0) as f32])
    }
}

/// Front view looking along +y, 1 px = 10 mm, screen y down = model -z.
struct Front;
impl Projector for Front {
    fn project(&self, p: Point3) -> Option<[f32; 2]> {
        Some([(p.x / 10.0) as f32, (-p.z / 10.0) as f32])
    }
}

/// A camera that sees nothing (every point behind it).
struct Blind;
impl Projector for Blind {
    fn project(&self, _: Point3) -> Option<[f32; 2]> {
        None
    }
}

fn down(x: f64, y: f64) -> (Ray, [f32; 2]) {
    (
        Ray {
            origin: Point3::new(x, y, 100_000.0),
            dir: -UnitVec3::Z,
        },
        Top.project(Point3::new(x, y, 0.0)).expect("projects"),
    )
}

fn forward(x: f64, z: f64) -> (Ray, [f32; 2]) {
    (
        Ray {
            origin: Point3::new(x, -100_000.0, z),
            dir: UnitVec3::Y,
        },
        Front.project(Point3::new(x, 0.0, z)).expect("projects"),
    )
}

fn doc_with(boxes: &[[f64; 6]]) -> Document {
    let mut s = Session::new(Document::new("snap"), Box::new(SequentialIds::new(1)));
    for b in boxes {
        let args = serde_json::json!({"origin": {"x": b[0], "y": b[1], "z": b[2]},
            "width": b[3], "depth": b[4], "height": b[5]});
        s.execute(
            AnyCommand::from_tool("create_box", &args).expect("args"),
            &ExecContext::user(),
        )
        .expect("box");
    }
    s.doc().clone()
}

fn snap_top(
    sc: &SnapScene,
    x: f64,
    y: f64,
    anchor: Option<Point3>,
    set: &SnapSettings,
) -> SnapResult {
    let (ray, c) = down(x, y);
    snap(sc, &ray, c, &Top, anchor, set).expect("a snap")
}

fn edge(a: Point3, b: Point3) -> SnapEdge {
    SnapEdge {
        a,
        b,
        element: ElementId(1),
    }
}

fn point(p: Point3, kind: SnapKind) -> SnapPoint {
    SnapPoint {
        point: p,
        kind,
        element: Some(ElementId(1)),
        edge_dir: None,
    }
}

// ------------------------------------------------------------ priority

#[test]
fn endpoint_beats_a_closer_midpoint_edge_axis_and_face() {
    // 1 m box: corner (0,0,1000); the midpoint of a 100 mm edge lies 50 mm
    // (5 px) away, the corner 80 mm (8 px) away from the cursor.
    let mut sc =
        SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 1000.0, 1000.0, 1000.0]]), &[]);
    sc.points
        .push(point(Point3::new(80.0, 30.0, 1000.0), SnapKind::Midpoint));
    let r = snap_top(
        &sc,
        70.0,
        30.0,
        Some(Point3::new(-5000.0, 0.0, 1000.0)),
        &SnapSettings::default(),
    );
    assert_eq!(r.kind, SnapKind::Endpoint, "{r:?}");
    assert!(r.point.is_almost_equal(Point3::new(0.0, 0.0, 1000.0)));
    // The endpoint lies on the hovered top face: the face is reported too.
    assert!(r.face.is_some() && r.normal.is_some());
}

#[test]
fn midpoint_beats_edge_and_edge_beats_axis_and_face() {
    let sc = SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 1000.0, 1000.0, 1000.0]]), &[]);
    let set = SnapSettings::default();
    let anchor = Some(Point3::new(-5000.0, 20.0, 1000.0));
    // Near the middle of the south top edge (y = 0): midpoint.
    let r = snap_top(&sc, 500.0, 40.0, anchor, &set);
    assert_eq!(r.kind, SnapKind::Midpoint);
    assert!(r.edge_dir.is_some(), "midpoints carry their edge direction");
    // Along that edge but away from the middle, near the axis line from the
    // anchor (y = 20): the edge wins over the axis and the face.
    let r = snap_top(&sc, 300.0, 30.0, anchor, &set);
    assert_eq!(r.kind, SnapKind::OnEdge);
    assert!(r.point.y.abs() < 1e-6 && (r.point.z - 1000.0).abs() < 1e-6);
}

#[test]
fn face_beats_ground_and_ground_follows_the_work_plane() {
    let sc = SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 1000.0, 1000.0, 1000.0]]), &[]);
    let r = snap_top(&sc, 500.0, 500.0, None, &SnapSettings::default());
    assert_eq!(r.kind, SnapKind::OnFace);
    assert!((r.point.z - 1000.0).abs() < 1e-6);
    let set = SnapSettings {
        ground_z: 3000.0,
        ..Default::default()
    };
    let r = snap_top(&sc, 5000.0, 5000.0, None, &set);
    assert_eq!(r.kind, SnapKind::Ground);
    assert!((r.point.z - 3000.0).abs() < 1e-6);
}

// ------------------------------------------------------------ occlusion

#[test]
fn points_and_edges_behind_a_face_are_hidden() {
    // Front view: a near box (y 0..1000) hides a far box (y 5000..6000)
    // that has a corner at x = 2000 where the near box spans 0..3000.
    let doc = doc_with(&[
        [0.0, 0.0, 0.0, 3000.0, 1000.0, 3000.0],
        [2000.0, 5000.0, 0.0, 4000.0, 1000.0, 1000.0],
    ]);
    let sc = SnapScene::from_document(&doc, &[]);
    let set = SnapSettings::default();
    // The far box's top-left corner (2000, 5000, 1000) is behind the near
    // box's front face: the face is snapped, not the hidden corner or edge.
    let (ray, c) = forward(2010.0, 1010.0);
    let r = snap(&sc, &ray, c, &Front, None, &set).expect("snap");
    assert_eq!(r.kind, SnapKind::OnFace, "{r:?}");
    assert!(r.point.y.abs() < 1e-6, "hit the near face");
    // Beyond the near box (x > 3000) the same far corner row is visible.
    let (ray, c) = forward(4005.0, 1003.0);
    let r = snap(&sc, &ray, c, &Front, None, &set).expect("snap");
    assert!(
        matches!(
            r.kind,
            SnapKind::OnEdge | SnapKind::Midpoint | SnapKind::Endpoint
        ),
        "{r:?}"
    );
    assert!((r.point.y - 5000.0).abs() < 1e-6);
    // Excluding the near box (e.g. while moving it) uncovers the corner.
    let near = doc.elements().next().expect("near").id;
    let sc = SnapScene::from_document(&doc, &[near]);
    let (ray, c) = forward(2010.0, 1010.0);
    let r = snap(&sc, &ray, c, &Front, None, &set).expect("snap");
    assert_eq!(r.kind, SnapKind::Endpoint);
    assert!(r.point.is_almost_equal(Point3::new(2000.0, 5000.0, 1000.0)));
}

#[test]
fn the_boundary_of_the_hovered_face_is_visible() {
    // The top front edge of a box seen from the front sits exactly on the
    // silhouette: it must snap even though the ray grazes the top face.
    let sc = SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 3000.0, 1000.0, 3000.0]]), &[]);
    let (ray, c) = forward(1200.0, 2995.0);
    let r = snap(&sc, &ray, c, &Front, None, &SnapSettings::default()).expect("snap");
    assert_eq!(r.kind, SnapKind::OnEdge, "{r:?}");
    assert!((r.point.z - 3000.0).abs() < 1e-6 && r.point.y.abs() < 1e-6);
}

// ----------------------------------------------- parallel / perpendicular

fn ground_only(x: f64, y: f64, reference: Option<Vec3>) -> SnapResult {
    let set = SnapSettings {
        reference: reference.and_then(|r| r.normalize()),
        ..Default::default()
    };
    snap_top(&SnapScene::default(), x, y, Some(Point3::ORIGIN), &set)
}

#[test]
fn parallel_and_perpendicular_follow_any_reference_direction() {
    for deg in [10.0_f64, 30.0, 60.0, 135.0, -75.0] {
        let a = deg.to_radians();
        let r = Vec3::new(a.cos(), a.sin(), 0.0);
        // 3 px (30 mm) off the parallel line, 5 m out.
        let n = Vec3::new(-a.sin(), a.cos(), 0.0);
        let q = Point3::ORIGIN + r * 5000.0 + n * 30.0;
        let s = ground_only(q.x, q.y, Some(r));
        assert_eq!(s.kind, SnapKind::Parallel, "{deg}°: {s:?}");
        let on = s.point - Point3::ORIGIN;
        assert!(on.cross(r).length() < 1e-6, "{deg}°: not on the line");
        // 3 px off the perpendicular line, on both sides of the anchor.
        for sign in [1.0, -1.0] {
            let q = Point3::ORIGIN + n * (5000.0 * sign) + r * 30.0;
            let s = ground_only(q.x, q.y, Some(r));
            assert_eq!(s.kind, SnapKind::Perpendicular, "{deg}° {sign}: {s:?}");
            assert!((s.point - Point3::ORIGIN).dot(r).abs() < 1e-6);
        }
    }
}

#[test]
fn reference_inference_without_or_with_degenerate_references() {
    // No reference: only the axes.
    assert_eq!(ground_only(5000.0, 5030.0, None).kind, SnapKind::Ground);
    assert_eq!(
        ground_only(5000.0, 30.0, None).kind,
        SnapKind::OnAxis(Axis::X)
    );
    // A vertical reference: parallel is the blue axis, perpendicular has
    // no horizontal direction: nothing extra, no panic.
    assert_eq!(
        ground_only(5000.0, 5030.0, Some(Vec3::Z)).kind,
        SnapKind::Ground
    );
    // A reference almost along X (within ANGLE_TOL) is the X axis itself.
    let s = ground_only(5000.0, 30.0, Some(Vec3::new(1.0, 1e-9, 0.0)));
    assert_eq!(s.kind, SnapKind::OnAxis(Axis::X));
    // Axes switched off: no inference at all.
    let set = SnapSettings {
        axes: false,
        reference: Vec3::new(1.0, 1.0, 0.0).normalize(),
        ..Default::default()
    };
    let r = snap_top(
        &SnapScene::default(),
        5000.0,
        5030.0,
        Some(Point3::ORIGIN),
        &set,
    );
    assert_eq!(r.kind, SnapKind::Ground);
}

#[test]
fn a_sloped_reference_gives_a_sloped_parallel() {
    // Front view, reference rising at 30° in the x-z plane: parallel works
    // in 3D; the perpendicular is horizontal (along y), seen end-on.
    let a = 30.0_f64.to_radians();
    let r = Vec3::new(a.cos(), 0.0, a.sin());
    let set = SnapSettings {
        reference: r.normalize(),
        ..Default::default()
    };
    let q = Point3::ORIGIN + r * 4000.0;
    let (ray, c) = forward(q.x + 20.0, q.z - 20.0);
    let s = snap(
        &SnapScene::default(),
        &ray,
        c,
        &Front,
        Some(Point3::ORIGIN),
        &set,
    )
    .expect("snap");
    assert_eq!(s.kind, SnapKind::Parallel, "{s:?}");
    assert!((s.point - Point3::ORIGIN).cross(r).length() < 1e-6);
}

// ------------------------------------------------------------ tolerance

#[test]
fn pixel_tolerance_is_inclusive_and_configurable() {
    let sc = SnapScene {
        points: vec![point(Point3::new(1000.0, 0.0, 0.0), SnapKind::Endpoint)],
        ..Default::default()
    };
    let at = |dx: f64, tol: f32| {
        let set = SnapSettings {
            tolerance_px: tol,
            ..Default::default()
        };
        snap_top(&sc, 1000.0 + dx, 0.0, None, &set).kind
    };
    // 1 px = 10 mm.
    assert_eq!(at(99.0, 10.0), SnapKind::Endpoint);
    assert_eq!(at(101.0, 10.0), SnapKind::Ground);
    assert_eq!(at(45.0, 5.0), SnapKind::Endpoint);
    assert_eq!(at(55.0, 5.0), SnapKind::Ground);
    assert_eq!(
        at(0.0, 0.0),
        SnapKind::Endpoint,
        "exact hit at zero tolerance"
    );
    assert_eq!(at(1.0, 0.0), SnapKind::Ground);
    // The tolerance is in pixels, not millimetres: zooming out (1 px =
    // 100 mm) makes the same model distance snap.
    struct Far;
    impl Projector for Far {
        fn project(&self, p: Point3) -> Option<[f32; 2]> {
            Some([(p.x / 100.0) as f32, (-p.y / 100.0) as f32])
        }
    }
    let (ray, _) = down(1500.0, 0.0);
    let c = Far
        .project(Point3::new(1500.0, 0.0, 0.0))
        .expect("projects");
    let r = snap(&sc, &ray, c, &Far, None, &SnapSettings::default()).expect("snap");
    assert_eq!(r.kind, SnapKind::Endpoint);
}

#[test]
fn closest_of_the_same_kind_wins() {
    let sc = SnapScene {
        points: vec![
            point(Point3::new(1000.0, 0.0, 0.0), SnapKind::Endpoint),
            point(Point3::new(1060.0, 0.0, 0.0), SnapKind::Endpoint),
        ],
        edges: vec![
            edge(
                Point3::new(0.0, 500.0, 0.0),
                Point3::new(3000.0, 500.0, 0.0),
            ),
            edge(
                Point3::new(0.0, 540.0, 0.0),
                Point3::new(3000.0, 540.0, 0.0),
            ),
        ],
        ..Default::default()
    };
    let set = SnapSettings::default();
    let r = snap_top(&sc, 1040.0, 0.0, None, &set);
    assert!(r.point.is_almost_equal(Point3::new(1060.0, 0.0, 0.0)));
    let r = snap_top(&sc, 2000.0, 525.0, None, &set);
    assert_eq!(r.kind, SnapKind::OnEdge);
    assert!((r.point.y - 540.0).abs() < 1e-6);
}

// ------------------------------------------------------------ degenerate

#[test]
fn degenerate_inputs_do_not_panic() {
    let set = SnapSettings::default();
    // Horizontal ray over an empty scene: no face, no ground hit.
    let ray = Ray {
        origin: Point3::new(0.0, 0.0, 1000.0),
        dir: UnitVec3::X,
    };
    let r = snap(&SnapScene::default(), &ray, [0.0, 0.0], &Top, None, &set);
    assert!(r.is_none(), "{r:?}");
    // Looking away from the ground: nothing. From below, the work plane is
    // hit from its underside.
    let ray = Ray {
        origin: Point3::new(0.0, 0.0, 1000.0),
        dir: UnitVec3::Z,
    };
    let r = snap(&SnapScene::default(), &ray, [0.0, 0.0], &Top, None, &set);
    assert!(r.is_none(), "{r:?}");
    let ray = Ray {
        origin: Point3::new(0.0, 0.0, -1000.0),
        dir: UnitVec3::Z,
    };
    let r = snap(&SnapScene::default(), &ray, [0.0, 0.0], &Top, None, &set).expect("snap");
    assert_eq!(r.kind, SnapKind::Ground);
    assert!(r.point.z.abs() < 1e-9);
    // Nothing projects: only face / ground snaps remain.
    let sc = SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 1000.0, 1000.0, 1000.0]]), &[]);
    let (ray, c) = down(10.0, 10.0);
    let r = snap(&sc, &ray, c, &Blind, Some(Point3::ORIGIN), &set).expect("snap");
    assert_eq!(r.kind, SnapKind::OnFace);
    // Cursor exactly on the anchor: axis lines through it are skipped.
    let r = snap_top(&SnapScene::default(), 0.0, 0.0, Some(Point3::ORIGIN), &set);
    assert!(
        matches!(r.kind, SnapKind::Origin | SnapKind::Ground),
        "{r:?}"
    );
    // A zero-length edge and a degenerate triangle.
    let sc = SnapScene {
        points: vec![],
        edges: vec![edge(Point3::new(5.0, 5.0, 0.0), Point3::new(5.0, 5.0, 0.0))],
        triangles: vec![SnapTriangle {
            tri: [
                Point3::ORIGIN,
                Point3::new(100.0, 0.0, 0.0),
                Point3::new(200.0, 0.0, 0.0),
            ],
            element: ElementId(1),
            face: FaceId(1),
            normal: UnitVec3::Z,
        }],
    };
    let r = snap_top(&sc, 5.0, 5.0, None, &set);
    assert_eq!(r.kind, SnapKind::Ground);
}

#[test]
fn bim_elements_are_snappable() {
    let mut s = Session::new(Document::new("snap"), Box::new(SequentialIds::new(1)));
    let args = serde_json::json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 4000, "y": 0, "z": 0}, "height": 3000});
    s.execute(
        AnyCommand::from_tool("create_wall", &args).expect("args"),
        &ExecContext::user(),
    )
    .expect("wall");
    let sc = SnapScene::from_document(s.doc(), &[]);
    // Wall corner at (0, 100, 3000) (200 mm thick, centred); the cursor is
    // on it, inside the wall outline.
    let r = snap_top(&sc, 20.0, 90.0, None, &SnapSettings::default());
    assert_eq!(r.kind, SnapKind::Endpoint, "{r:?}");
    assert!(
        r.point.is_almost_equal(Point3::new(0.0, 100.0, 3000.0)),
        "{r:?}"
    );
}

/// In plan view a top corner and the bottom corner below it are on the
/// same pixel. With the cursor just outside the outline (no face under it,
/// so nothing occludes) the corner nearer the camera must win.
#[test]
fn in_plan_view_the_nearer_of_two_coincident_corners_wins() {
    let sc = SnapScene::from_document(&doc_with(&[[0.0, 0.0, 0.0, 1000.0, 1000.0, 3000.0]]), &[]);
    for (x, y) in [(-20.0, -20.0), (1020.0, 30.0), (40.0, 1030.0)] {
        let r = snap_top(&sc, x, y, None, &SnapSettings::default());
        assert_eq!(r.kind, SnapKind::Endpoint, "{r:?}");
        assert!((r.point.z - 3000.0).abs() < 1e-9, "({x}, {y}): {r:?}");
    }
    // Along an outline edge: the top edge, not the bottom one.
    let r = snap_top(&sc, 500.0, -30.0, None, &SnapSettings::default());
    assert!(matches!(r.kind, SnapKind::Midpoint), "{r:?}");
    assert!((r.point.z - 3000.0).abs() < 1e-9, "{r:?}");
    let r = snap_top(&sc, 300.0, -30.0, None, &SnapSettings::default());
    assert_eq!(r.kind, SnapKind::OnEdge, "{r:?}");
    assert!((r.point.z - 3000.0).abs() < 1e-9, "{r:?}");
}
