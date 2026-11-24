//! Push / pull + imprint stress sequences (M5-15.4) on boxes, axis-aligned,
//! rotated by 30° / 45° and 5 km from the origin. Bodies are validated after
//! every step and volumes are checked against hand calculations.

use super::scenario_common_tests::*;
use super::Session;
use crate::math::{Point3, Vec3};
use crate::model::ElementId;
use crate::topo::FaceId;
use serde_json::json;

const W: f64 = 4000.0;
const D: f64 = 3000.0;
const H: f64 = 2000.0;

/// Placement of the box: rotation about Z through the origin, then a shift.
#[derive(Clone, Copy, Debug)]
struct Frame {
    angle_deg: f64,
    shift: [f64; 3],
}

impl Frame {
    fn world(&self, x: f64, y: f64, z: f64) -> Point3 {
        let a = self.angle_deg.to_radians();
        let (c, s) = (a.cos(), a.sin());
        Point3::new(
            c * x - s * y + self.shift[0],
            s * x + c * y + self.shift[1],
            z + self.shift[2],
        )
    }
    fn dir(&self, x: f64, y: f64, z: f64) -> Vec3 {
        let a = self.angle_deg.to_radians();
        let (c, s) = (a.cos(), a.sin());
        Vec3::new(c * x - s * y, s * x + c * y, z)
    }
    fn pj(&self, x: f64, y: f64, z: f64) -> serde_json::Value {
        let q = self.world(x, y, z);
        p(q.x, q.y, q.z)
    }
}

fn frames() -> Vec<Frame> {
    let far = [5_000_000.0, 3_000_000.0, 100.0];
    let mut v = Vec::new();
    for angle_deg in [0.0, 30.0, 45.0] {
        for shift in [[0.0; 3], far] {
            v.push(Frame { angle_deg, shift });
        }
    }
    v
}

fn placed_box(s: &mut Session, fr: Frame) -> ElementId {
    let e = boxed(s, [0.0; 3], W, D, H);
    if fr.angle_deg != 0.0 {
        expect_ok(
            s,
            "rotate_elements",
            json!({"elements": [e], "center": p(0.0, 0.0, 0.0), "angle": fr.angle_deg.to_radians()}),
        );
    }
    expect_ok(
        s,
        "move_elements",
        json!({"elements": [e], "vector": p(fr.shift[0], fr.shift[1], fr.shift[2])}),
    );
    e
}

fn volume(s: &Session, e: ElementId) -> f64 {
    let b = geometry(s, e);
    b.validate_solid().expect("solid");
    b.volume()
}

fn face(s: &Session, e: ElementId, fr: Frame, x: f64, y: f64, z: f64) -> FaceId {
    face_towards(&geometry(s, e), fr.dir(x, y, z))
}

/// rectangle on top → push through → circle on the front → push in →
/// offset the east side → pull out → line across the top → erase it.
#[test]
fn imprint_push_offset_erase_sequence_in_every_frame() {
    for fr in frames() {
        let ctx = format!("{fr:?}");
        let mut s = session();
        let e = placed_box(&mut s, fr);
        let mut v = W * D * H;
        assert!((volume(&s, e) - v).abs() < 10.0, "{ctx}");

        // 1. A 1 m square hole through the top.
        let top = face(&s, e, fr, 0.0, 0.0, 1.0);
        let o = expect_ok(
            &mut s,
            "imprint_face",
            json!({"element": e, "face": top, "closed": true, "points": [
                fr.pj(1000.0, 1000.0, H), fr.pj(2000.0, 1000.0, H),
                fr.pj(2000.0, 2000.0, H), fr.pj(1000.0, 2000.0, H)]}),
        );
        let sq = new_face(&o);
        let o = expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": sq, "distance": -H}),
        );
        assert_eq!(o.output["mode"], "through", "{ctx}");
        v -= 1000.0 * 1000.0 * H;
        assert!((volume(&s, e) - v).abs() < 10.0, "{ctx}: through");
        // Euler-Poincare with inner loops: V - E + F - holes = 2 - 2 genus.
        let b = geometry(&s, e);
        let holes: usize = b
            .face_ids()
            .map(|f| b.face(f).expect("face").holes.len())
            .sum();
        assert_eq!(
            b.vertex_count() as i64 - b.edge_count() as i64 + b.face_count() as i64 - holes as i64,
            0,
            "{ctx}: Euler characteristic of a box with a hole"
        );

        // 2. A circular pocket in the front face.
        let front = face(&s, e, fr, 0.0, -1.0, 0.0);
        let o = expect_ok(
            &mut s,
            "create_circle",
            json!({"element": e, "face": front, "center": fr.pj(3000.0, 0.0, 1000.0), "radius": 400, "segments": 24}),
        );
        let disc = new_face(&o);
        let area = 0.5 * 24.0 * 400.0 * 400.0 * (std::f64::consts::TAU / 24.0).sin();
        assert!(
            (geometry(&s, e).face_area(disc).expect("area") - area).abs() < 1e-3,
            "{ctx}"
        );
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": disc, "distance": -500}),
        );
        v -= area * 500.0;
        assert!((volume(&s, e) - v).abs() < 10.0, "{ctx}: pocket");

        // 3. Offset the east face by 200 and pull the inner part out 300.
        let east = face(&s, e, fr, 1.0, 0.0, 0.0);
        let o = expect_ok(
            &mut s,
            "offset_face",
            json!({"element": e, "face": east, "distance": 200}),
        );
        let inner = new_face(&o);
        let ia = geometry(&s, e).face_area(inner).expect("area");
        assert!((ia - (D - 400.0) * (H - 400.0)).abs() < 1e-3, "{ctx}");
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": inner, "distance": 300}),
        );
        v += ia * 300.0;
        assert!((volume(&s, e) - v).abs() < 10.0, "{ctx}: boss");

        // 4. A line across the top splits it; erasing the line heals it.
        let faces_before = geometry(&s, e).face_count();
        let top = face(&s, e, fr, 0.0, 0.0, 1.0);
        expect_ok(
            &mut s,
            "imprint_face",
            json!({"element": e, "face": top, "points": [fr.pj(3500.0, 0.0, H), fr.pj(3500.0, D, H)]}),
        );
        let b = geometry(&s, e);
        assert_eq!(b.face_count(), faces_before + 1, "{ctx}: split");
        let (a0, a1) = (fr.world(3500.0, 0.0, H), fr.world(3500.0, D, H));
        let on_line = |q: Point3| {
            let t = (q - a0).dot(a1 - a0) / (a1 - a0).length_squared();
            (a0 + (a1 - a0) * t).distance_to(q) < 1e-3
        };
        let line: Vec<_> = b
            .edges()
            .filter(|(_, x, y)| on_line(*x) && on_line(*y))
            .map(|(id, _, _)| id)
            .collect();
        assert!(!line.is_empty(), "{ctx}");
        let o = expect_ok(&mut s, "erase_edges", json!({"element": e, "edges": line}));
        assert_eq!(o.output["removed_faces"], 0, "{ctx}");
        assert_eq!(geometry(&s, e).face_count(), faces_before, "{ctx}: healed");
        assert!(
            (volume(&s, e) - v).abs() < 10.0,
            "{ctx}: erase keeps volume"
        );

        // 5. Undo everything back to the plain box.
        let steps = s.journal().len();
        for _ in 0..steps {
            s.undo().expect("undo");
        }
        assert!(s.doc().is_empty(), "{ctx}");
    }
}

/// Repeated push / pull of the same face up and down returns the same
/// body (stretch mode is exact).
#[test]
fn repeated_push_pull_returns_to_the_start() {
    for fr in frames() {
        let mut s = session();
        let e = placed_box(&mut s, fr);
        let start = geometry(&s, e);
        let top = face(&s, e, fr, 0.0, 0.0, 1.0);
        for d in [500.0, -1200.0, 0.51, 699.49] {
            expect_ok(
                &mut s,
                "push_pull_face",
                json!({"element": e, "face": top, "distance": d}),
            );
        }
        let end = geometry(&s, e);
        for (v, q) in start.vertices() {
            let r = end.point(v).expect("same vertex ids");
            assert!(q.distance_to(r) < 1e-6, "{fr:?}: {q:?} vs {r:?}");
        }
    }
}

fn pocket(s: &mut Session, fr: Frame) -> (ElementId, FaceId) {
    let e = placed_box(s, fr);
    let top = face(s, e, fr, 0.0, 0.0, 1.0);
    let o = expect_ok(
        s,
        "imprint_face",
        json!({"element": e, "face": top, "closed": true, "points": [
            fr.pj(500.0, 500.0, H), fr.pj(1500.0, 500.0, H),
            fr.pj(1500.0, 1500.0, H), fr.pj(500.0, 1500.0, H)]}),
    );
    let f = new_face(&o);
    expect_ok(
        s,
        "push_pull_face",
        json!({"element": e, "face": f, "distance": -800}),
    );
    assert!(
        (volume(s, e) - (W * D * H - 1e6 * 800.0)).abs() < 10.0,
        "{fr:?}"
    );
    (e, f)
}

/// A pocket floor pulled back up (short of flush) shrinks the pocket.
#[test]
fn pocket_pulled_back_up() {
    for fr in frames() {
        let mut s = session();
        let (e, f) = pocket(&mut s, fr);
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": f, "distance": 799}),
        );
        assert!((volume(&s, e) - (W * D * H - 1e6)).abs() < 10.0, "{fr:?}");
    }
}

/// SketchUp: pulling a pocket floor exactly back to flush removes the
/// pocket; pulling further makes a boss (M5-15.7).
#[test]
fn pocket_back_to_flush_and_beyond() {
    for fr in frames() {
        // Flush, exactly and within VERTEX_TOL: one top face again.
        for d in [800.0, 799.995, 800.005] {
            let mut s = session();
            let (e, f) = pocket(&mut s, fr);
            let o = expect_ok(
                &mut s,
                "push_pull_face",
                json!({"element": e, "face": f, "distance": d}),
            );
            assert!((volume(&s, e) - W * D * H).abs() < 10.0, "{fr:?}: flush");
            let b = geometry(&s, e);
            assert_eq!(b.face_count(), 6, "{fr:?} {d}: healed");
            assert!(b.face(f).is_err(), "{fr:?}: pulled face merged");
            let deleted: Vec<FaceId> =
                serde_json::from_value(o.output["lineage"]["deleted"].clone()).expect("lineage");
            assert!(deleted.contains(&f));
            let top = face(&s, e, fr, 0.0, 0.0, 1.0);
            assert!(b.face(top).is_ok_and(|t| t.holes.is_empty()));
            // The merged face is gone for good.
            expect_code(
                &mut s,
                "push_pull_face",
                json!({"element": e, "face": f, "distance": 100}),
                "ferro.topo.not_found",
            );
        }
        for (d, boss) in [(1000.0, 200.0), (1200.0, 400.0)] {
            let mut s = session();
            let (e, f) = pocket(&mut s, fr);
            expect_ok(
                &mut s,
                "push_pull_face",
                json!({"element": e, "face": f, "distance": d}),
            );
            assert!(
                (volume(&s, e) - (W * D * H + 1e6 * boss)).abs() < 10.0,
                "{fr:?}: boss"
            );
        }
        // 0.02 mm short of flush leaves walls below 0.5 mm: refused.
        let mut s = session();
        let (e, f) = pocket(&mut s, fr);
        expect_code(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": f, "distance": 799.98}),
            "ferro.geom.short_curve",
        );
    }
}

/// Half of a split top pushed down (a step), then pulled back flush: the
/// step wall disappears and the volume is back (M5-15.7).
#[test]
fn step_down_and_back_flush() {
    for fr in frames() {
        let mut s = session();
        let e = placed_box(&mut s, fr);
        let top = face(&s, e, fr, 0.0, 0.0, 1.0);
        let o = expect_ok(
            &mut s,
            "imprint_face",
            json!({"element": e, "face": top, "closed": false, "points": [
                fr.pj(1500.0, 0.0, H), fr.pj(1500.0, D, H)]}),
        );
        let faces: Vec<FaceId> = serde_json::from_value(o.output["faces"].clone()).expect("faces");
        let b = geometry(&s, e);
        // The 1500-wide half.
        let half = faces
            .iter()
            .copied()
            .find(|f| b.face_area(*f).is_ok_and(|a| (a - 1500.0 * D).abs() < 1.0))
            .expect("half");
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": half, "distance": -400}),
        );
        assert!(
            (volume(&s, e) - (W * D * H - 1500.0 * D * 400.0)).abs() < 10.0,
            "{fr:?}"
        );
        let n = geometry(&s, e).face_count();
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": half, "distance": 400}),
        );
        assert!((volume(&s, e) - W * D * H).abs() < 10.0, "{fr:?}");
        assert_eq!(
            geometry(&s, e).face_count(),
            n - 1,
            "{fr:?}: step wall gone"
        );
    }
}

/// Pushing a pocket deeper than the box is refused (it would break through
/// the bottom without landing on it) and leaves the body as it was.
/// (M5-15.8: it used to be committed as a self-intersecting body.)
#[test]
fn pocket_deeper_than_the_box_is_refused() {
    for fr in frames() {
        let mut s = session();
        let e = placed_box(&mut s, fr);
        let f = square_on_top(&mut s, e, fr);
        expect_code(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": f, "distance": -(H + 100.0)}),
            "ferro.topo.self_intersecting",
        );
        // Just short of the bottom (0.5 mm floor left) is fine.
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": f, "distance": -(H - 0.51)}),
        );
    }
}

/// Imprint the 1000 mm square (500..1500) on the top of the placed box.
fn square_on_top(s: &mut Session, e: ElementId, fr: Frame) -> FaceId {
    let top = face(s, e, fr, 0.0, 0.0, 1.0);
    let o = expect_ok(
        s,
        "imprint_face",
        json!({"element": e, "face": top, "closed": true, "points": [
            fr.pj(500.0, 500.0, H), fr.pj(1500.0, 500.0, H),
            fr.pj(1500.0, 1500.0, H), fr.pj(500.0, 1500.0, H)]}),
    );
    new_face(&o)
}

/// Tolerance edges of a pocket against the bottom of a 2000 mm box
/// (M5-15.8): within `VERTEX_TOL` of the bottom (−1999.995, −2000,
/// −2000.005) the pocket goes through (SketchUp push-through) and the volume
/// is exact; 0.02 mm past the bottom is refused; 0.6 mm short is a pocket.
#[test]
fn pocket_depth_tolerance_edges_against_the_bottom() {
    for fr in frames() {
        for (depth, expect) in [
            (H - 0.005, Some("through")),
            (H, Some("through")),
            (H + 0.005, Some("through")),
            (H + 0.02, None),
            (H - 0.6, Some("prism")),
        ] {
            let mut s = session();
            let e = placed_box(&mut s, fr);
            let f = square_on_top(&mut s, e, fr);
            let args = json!({"element": e, "face": f, "distance": -depth});
            match expect {
                Some(mode) => {
                    let o = expect_ok(&mut s, "push_pull_face", args);
                    assert_eq!(o.output["mode"], mode, "{fr:?} {depth}");
                    let removed = if mode == "through" { H } else { depth };
                    assert!(
                        (volume(&s, e) - (W * D * H - 1e6 * removed)).abs() < 1e-6 * W * D * H,
                        "{fr:?} {depth}"
                    );
                }
                None => expect_code(
                    &mut s,
                    "push_pull_face",
                    args,
                    "ferro.topo.self_intersecting",
                ),
            }
        }
    }
}

/// A pocket floor dragged sideways through a wall of the box (move_geometry
/// can do that) leaves a self-intersecting body: refused by the Transaction's
/// body validation, not only by push / pull.
#[test]
fn moving_a_pocket_floor_through_a_side_is_refused() {
    let mut s = session();
    let fr = frames()[0];
    let e = placed_box(&mut s, fr);
    let f = square_on_top(&mut s, e, fr);
    expect_ok(
        &mut s,
        "push_pull_face",
        json!({"element": e, "face": f, "distance": -800}),
    );
    let r = checked(
        &mut s,
        "move_geometry",
        json!({"element": e, "faces": [f], "vector": p(-1000.0, 0.0, 0.0)}),
    );
    assert!(r.is_err(), "pocket floor moved out through a side");
}

/// Two holes through, side by side, then the web between them pushed out:
/// bodies stay valid.
#[test]
fn two_holes_and_a_split_side() {
    for fr in frames() {
        let mut s = session();
        let e = placed_box(&mut s, fr);
        for x0 in [500.0, 2500.0] {
            let top = face(&s, e, fr, 0.0, 0.0, 1.0);
            let o = expect_ok(
                &mut s,
                "imprint_face",
                json!({"element": e, "face": top, "closed": true, "points": [
                    fr.pj(x0, 1000.0, H), fr.pj(x0 + 1000.0, 1000.0, H),
                    fr.pj(x0 + 1000.0, 2000.0, H), fr.pj(x0, 2000.0, H)]}),
            );
            let f = new_face(&o);
            let o = expect_ok(
                &mut s,
                "push_pull_face",
                json!({"element": e, "face": f, "distance": -H}),
            );
            assert_eq!(o.output["mode"], "through", "{fr:?}");
        }
        let v = W * D * H - 2.0 * 1e6 * H;
        assert!((volume(&s, e) - v).abs() < 10.0, "{fr:?}");
        // Split the front face with a vertical line, pull one half out.
        let front = face(&s, e, fr, 0.0, -1.0, 0.0);
        let o = expect_ok(
            &mut s,
            "imprint_face",
            json!({"element": e, "face": front, "points": [fr.pj(2000.0, 0.0, 0.0), fr.pj(2000.0, 0.0, H)]}),
        );
        let half = new_face(&o);
        expect_ok(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": half, "distance": 250}),
        );
        let b = geometry(&s, e);
        b.validate_solid().expect("valid");
        assert!(
            (b.volume() - (v + 2000.0 * H * 250.0)).abs() < 10.0,
            "{fr:?}"
        );
    }
}
