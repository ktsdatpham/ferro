//! Booleans of planar solids through `SolidcoreKernel` (M5-16.3): exact
//! volumes, D-06 tolerance edges, rotations, far from the origin, lineage,
//! validity and determinism. Lengths in mm.

use ferro_core::algo::{BooleanOp, FaceSource, Kernel, KernelError, KernelResult};
use ferro_core::math::{Angle, Point3, Transform, UnitVec3, Vec3};
use ferro_core::topo::{Body, FaceId};
use ferro_kernel::SolidcoreKernel;

/// Where the operands are placed: turned about Z through the origin, then
/// moved.
#[derive(Clone, Copy, Debug)]
struct Place {
    deg: f64,
    at: [f64; 3],
}

/// 0°, 30°, 45°, and each of them 5 km from the origin.
fn places() -> Vec<Place> {
    let mut v = Vec::new();
    for at in [[0.0, 0.0, 0.0], [5.0e6, 5.0e6, 0.0]] {
        for deg in [0.0, 30.0, 45.0] {
            v.push(Place { deg, at });
        }
    }
    v
}

const ORIGIN: Place = Place {
    deg: 0.0,
    at: [0.0, 0.0, 0.0],
};

/// Axis-aligned box from `lo` to `hi`, then placed; with its face ids
/// (bottom, top, south, east, north, west).
fn cuboid(lo: [f64; 3], hi: [f64; 3], pl: Place) -> (Body, Vec<FaceId>) {
    let [x0, y0, z0] = lo;
    let [x1, y1, z1] = hi;
    let p = [
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let faces = vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    let (mut b, ids) = Body::from_polygons(&p, &faces).expect("box");
    b.transform(&Transform::rotation(
        Point3::new(0.0, 0.0, 0.0),
        UnitVec3::Z,
        Angle::from_deg(pl.deg),
    ));
    b.transform(&Transform::translation(Vec3::new(
        pl.at[0], pl.at[1], pl.at[2],
    )));
    (b, ids)
}

fn cube(pl: Place) -> (Body, Vec<FaceId>) {
    cuboid([0.0; 3], [1000.0; 3], pl)
}

fn run(op: BooleanOp, a: &Body, b: &Body) -> Result<KernelResult, KernelError> {
    SolidcoreKernel::new().boolean(op, a, b)
}

/// The result is a valid solid of `volume` mm³ (relative 1e-9) and `faces`
/// faces; every face's sources are faces of the operands.
fn check(r: &KernelResult, volume: f64, faces: usize, what: &str) {
    r.body
        .validate_solid()
        .unwrap_or_else(|e| panic!("{what}: invalid result: {e}"));
    let v = r.body.volume();
    assert!(
        ((v - volume) / volume).abs() < 1e-9,
        "{what}: volume {v}, expected {volume}"
    );
    assert_eq!(r.body.face_count(), faces, "{what}: face count");
    assert_eq!(r.faces.len(), faces, "{what}: every face has sources");
}

fn ok(op: BooleanOp, a: &Body, b: &Body, what: &str) -> KernelResult {
    run(op, a, b).unwrap_or_else(|e| panic!("{what}: {} ({})", e.message, e.code))
}

// ------------------------------------------------------------ box − box

#[test]
fn through_hole_keeps_the_target_faces_and_exact_volume() {
    for pl in places() {
        let what = format!("through hole {pl:?}");
        let (a, fa) = cube(pl);
        let (b, _) = cuboid([400.0, 400.0, -100.0], [600.0, 600.0, 1100.0], pl);
        let r = ok(BooleanOp::Difference, &a, &b, &what);
        check(&r, 1e9 - 200.0 * 200.0 * 1000.0, 10, &what);
        // All six target faces survive with their ids; top and bottom have a hole.
        assert_eq!(r.lineage.kept.len(), 6, "{what}");
        assert!(r.lineage.deleted.is_empty(), "{what}");
        for f in [fa[0], fa[1]] {
            assert_eq!(r.body.face(f).expect("kept").holes.len(), 1, "{what}");
            assert_eq!(r.faces[&f], vec![FaceSource::FromA(f)], "{what}");
        }
        // The four hole walls are new, from the cutter's sides.
        assert_eq!(r.lineage.created.len(), 4, "{what}");
        for (f, from) in &r.lineage.created {
            assert_eq!(*from, None, "{what}");
            assert!(
                matches!(r.faces[f].as_slice(), [FaceSource::FromB(_)]),
                "{what}: {:?}",
                r.faces[f]
            );
        }
    }
}

#[test]
fn pocket_has_a_floor_from_the_cutter() {
    for pl in places() {
        let what = format!("pocket {pl:?}");
        let (a, fa) = cube(pl);
        let (b, fb) = cuboid([400.0, 400.0, 500.0], [600.0, 600.0, 1100.0], pl);
        let r = ok(BooleanOp::Difference, &a, &b, &what);
        check(&r, 1e9 - 200.0 * 200.0 * 500.0, 11, &what);
        assert_eq!(r.body.face(fa[1]).expect("top kept").holes.len(), 1);
        // The pocket floor lies on the cutter's bottom.
        assert!(
            r.faces
                .values()
                .any(|s| s == &vec![FaceSource::FromB(fb[0])]),
            "{what}"
        );
    }
}

#[test]
fn corner_notch_replaces_a_corner() {
    for pl in places() {
        let what = format!("notch {pl:?}");
        let (a, _) = cube(pl);
        let (b, _) = cuboid([800.0; 3], [1100.0; 3], pl);
        let r = ok(BooleanOp::Difference, &a, &b, &what);
        check(&r, 1e9 - 200.0f64.powi(3), 9, &what);
        assert_eq!(r.lineage.kept.len(), 6, "{what}");
    }
}

// ------------------------------------------------------------ union / intersection

#[test]
fn union_of_boxes_sharing_a_face_merges_coplanar_faces() {
    for pl in places() {
        let what = format!("union {pl:?}");
        let (a, fa) = cube(pl);
        let (b, _) = cuboid([1000.0, 0.0, 0.0], [2000.0, 1000.0, 1000.0], pl);
        let r = ok(BooleanOp::Union, &a, &b, &what);
        check(&r, 2e9, 6, &what);
        assert_eq!(r.body.vertex_count(), 8, "{what}");
        // The top is one face, from both tops, with A's id; A's east face is gone.
        let top = &r.faces[&fa[1]];
        assert_eq!(top.len(), 2, "{what}: {top:?}");
        assert_eq!(r.lineage.deleted, vec![fa[3]], "{what}");
    }
}

#[test]
fn intersection_of_overlapping_boxes() {
    for pl in places() {
        let what = format!("intersection {pl:?}");
        let (a, _) = cube(pl);
        let (b, _) = cuboid([500.0; 3], [1500.0; 3], pl);
        let r = ok(BooleanOp::Intersection, &a, &b, &what);
        check(&r, 500.0f64.powi(3), 6, &what);
        assert_eq!(r.lineage.kept.len(), 3, "{what}");
    }
}

// ------------------------------------------------------------ degenerate

#[test]
fn disjoint_operands() {
    let (a, fa) = cube(ORIGIN);
    let (b, _) = cuboid([3000.0; 3], [4000.0; 3], ORIGIN);
    // Empty intersection: a clear code.
    let e = run(BooleanOp::Intersection, &a, &b).unwrap_err();
    assert_eq!(e.code, KernelError::EMPTY_RESULT);
    // A difference keeps the target as it is.
    let r = ok(BooleanOp::Difference, &a, &b, "disjoint difference");
    check(&r, 1e9, 6, "disjoint difference");
    assert_eq!(
        r.lineage.kept,
        fa.iter().copied().collect::<Vec<_>>().tap_sort()
    );
    // A union is one body of two shells.
    let r = ok(BooleanOp::Union, &a, &b, "disjoint union");
    check(&r, 2e9, 12, "disjoint union");
}

#[test]
fn removing_everything_is_empty() {
    let (a, _) = cube(ORIGIN);
    let (b, _) = cuboid([-10.0; 3], [1010.0; 3], ORIGIN);
    let e = run(BooleanOp::Difference, &a, &b).unwrap_err();
    assert_eq!(e.code, KernelError::EMPTY_RESULT);
}

/// A cutter fully inside leaves an inner void: a valid body of two shells
/// (outer and void), which the seam cannot take back as an operand yet.
#[test]
fn a_cutter_inside_leaves_a_void() {
    let (a, _) = cube(ORIGIN);
    let (b, _) = cuboid([400.0; 3], [600.0; 3], ORIGIN);
    let r = ok(BooleanOp::Difference, &a, &b, "void");
    check(&r, 1e9 - 8e6, 12, "void");
    let (c, _) = cuboid([100.0; 3], [200.0; 3], ORIGIN);
    let e = run(BooleanOp::Difference, &r.body, &c).unwrap_err();
    assert_eq!(e.code, KernelError::UNSUPPORTED);
}

#[test]
fn an_open_operand_is_not_a_solid() {
    let (a, _) = cube(ORIGIN);
    let (sheet, _) = Body::from_polygons(
        &[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(100.0, 0.0, 0.0),
            Point3::new(100.0, 100.0, 0.0),
        ],
        &[vec![0, 1, 2]],
    )
    .expect("sheet");
    let e = run(BooleanOp::Difference, &a, &sheet).unwrap_err();
    assert_eq!(e.code, KernelError::NOT_SOLID);
    let e = run(BooleanOp::Difference, &sheet, &a).unwrap_err();
    assert_eq!(e.code, KernelError::NOT_SOLID);
}

#[test]
fn boxes_touching_along_an_edge_do_not_make_a_body() {
    let (a, _) = cube(ORIGIN);
    let (b, _) = cuboid([1000.0, 1000.0, 0.0], [2000.0, 2000.0, 1000.0], ORIGIN);
    let e = run(BooleanOp::Union, &a, &b).unwrap_err();
    assert_eq!(e.code, KernelError::NON_MANIFOLD_RESULT, "{}", e.message);
}

// ------------------------------------------------------------ flush faces

#[test]
fn opening_flush_with_both_wall_faces() {
    for pl in places() {
        let what = format!("flush opening {pl:?}");
        let (wall, _) = cuboid([0.0; 3], [6000.0, 200.0, 3000.0], pl);
        let (cut, _) = cuboid([2000.0, 0.0, 900.0], [3500.0, 200.0, 2100.0], pl);
        let r = ok(BooleanOp::Difference, &wall, &cut, &what);
        check(
            &r,
            6000.0 * 200.0 * 3000.0 - 1500.0 * 200.0 * 1200.0,
            10,
            &what,
        );
        let (niche, _) = cuboid([2000.0, 100.0, 900.0], [3500.0, 200.0, 2100.0], pl);
        let what = format!("flush niche {pl:?}");
        let r = ok(BooleanOp::Difference, &wall, &niche, &what);
        check(
            &r,
            6000.0 * 200.0 * 3000.0 - 1500.0 * 100.0 * 1200.0,
            11,
            &what,
        );
    }
}

// ------------------------------------------------------------ D-06 tolerance edges

/// A union of a cube and a cube moved `d` into it: one box of
/// `(2000 − d) × 1000 × 1000` whatever `d` (the overlap is absorbed).
#[test]
fn overlaps_around_vertex_tolerance() {
    for pl in places() {
        for d in [0.001, 0.01, 0.02] {
            let what = format!("overlap {d} {pl:?}");
            let (a, _) = cube(pl);
            let (b, _) = cuboid([1000.0 - d, 0.0, 0.0], [2000.0 - d, 1000.0, 1000.0], pl);
            let r = ok(BooleanOp::Union, &a, &b, &what);
            check(&r, (2000.0 - d) * 1e6, 6, &what);
        }
    }
}

/// A union of side-by-side cubes, the second raised by `d`: within
/// `VERTEX_TOL` it is snapped level (one 2 m box, 6 faces); beyond it the
/// steps of `d` at top and bottom remain (10 faces). Exactly at the
/// tolerance either reading is right for each step; it must not fail.
#[test]
fn steps_around_vertex_tolerance() {
    for pl in places() {
        for (d, faces) in [(0.001, 6), (0.02, 10)] {
            let what = format!("step {d} {pl:?}");
            let (a, _) = cube(pl);
            let (b, _) = cuboid([1000.0, 0.0, d], [2000.0, 1000.0, 1000.0 + d], pl);
            let r = ok(BooleanOp::Union, &a, &b, &what);
            check(&r, 2e9, faces, &what);
        }
        let (a, _) = cube(pl);
        let (b, _) = cuboid([1000.0, 0.0, 0.01], [2000.0, 1000.0, 1000.01], pl);
        let r = ok(BooleanOp::Union, &a, &b, "step 0.01");
        r.body.validate_solid().expect("valid");
        // Each step is decided on its own (rounding may snap or merge one and
        // keep the other): 6 to 10 faces, the volume off by one 0.01 mm
        // layer at most.
        let v = r.body.volume();
        assert!((v - 2e9).abs() < 1e4 + 1.0, "step 0.01 {pl:?}: {v}");
        assert!(
            (6..=10).contains(&r.body.face_count()),
            "step 0.01 {pl:?}: {} faces",
            r.body.face_count()
        );
    }
}

/// A cutter stopping `d` short of the far face: within `VERTEX_TOL` it goes
/// through (no 0.001 mm skin), beyond it a skin of `d` remains.
#[test]
fn skins_around_vertex_tolerance() {
    for pl in places() {
        let (a, _) = cube(pl);
        for (d, volume, faces) in [
            (0.001, 1e9 - 4e4 * 1000.0, 10),
            (0.02, 1e9 - 4e4 * (1000.0 - 0.02), 11),
            (-0.001, 1e9 - 4e4 * 1000.0, 10),
        ] {
            let what = format!("skin {d} {pl:?}");
            let (b, _) = cuboid([400.0, 400.0, d], [600.0, 600.0, 1100.0], pl);
            let r = ok(BooleanOp::Difference, &a, &b, &what);
            check(&r, volume, faces, &what);
        }
    }
}

// ------------------------------------------------------------ determinism

#[test]
fn same_input_gives_the_same_result() {
    let pl = Place {
        deg: 30.0,
        at: [5.0e6, 5.0e6, 0.0],
    };
    let (a, _) = cube(pl);
    let (b, _) = cuboid([400.0, 400.0, 500.0], [600.0, 1200.0, 1100.0], pl);
    let r1 = ok(BooleanOp::Difference, &a, &b, "first");
    let r2 = ok(BooleanOp::Difference, &a, &b, "second");
    assert_eq!(r1, r2);
    // And from another thread.
    let (a2, b2) = (a.clone(), b.clone());
    let r3 = std::thread::spawn(move || run(BooleanOp::Difference, &a2, &b2))
        .join()
        .expect("thread")
        .expect("third");
    assert_eq!(r1, r3);
}

#[test]
fn a_result_can_be_cut_again_and_keeps_ids() {
    let (a, fa) = cube(ORIGIN);
    let (b1, _) = cuboid([100.0, 100.0, 500.0], [300.0, 300.0, 1100.0], ORIGIN);
    let (b2, _) = cuboid([600.0, 600.0, -100.0], [800.0, 800.0, 1100.0], ORIGIN);
    let r1 = ok(BooleanOp::Difference, &a, &b1, "first cut");
    let r2 = ok(BooleanOp::Difference, &r1.body, &b2, "second cut");
    check(&r2, 1e9 - 4e4 * 500.0 - 4e4 * 1000.0, 15, "second cut");
    assert_eq!(r2.body.face(fa[1]).expect("top").holes.len(), 2);
    // Faces made by the first cut keep their ids through the second.
    for (f, _) in &r1.lineage.created {
        assert!(r2.lineage.kept.contains(f), "{f} lost");
    }
}

trait TapSort {
    fn tap_sort(self) -> Self;
}
impl TapSort for Vec<FaceId> {
    fn tap_sort(mut self) -> Self {
        self.sort();
        self
    }
}
