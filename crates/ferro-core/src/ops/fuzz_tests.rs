//! Property-style fuzzing of Form editing (M5-15.5) with a seeded xorshift
//! generator: random rectangles, circles, offsets, push / pull distances,
//! stretches, moves, rotations, erasing and undo on boxes. Every step either
//! commits a valid document or fails cleanly with a `ferro.*` code and
//! changes nothing (see `scenario_common_tests::checked`).

use super::scenario_common_tests::*;
use super::Session;
use crate::math::{Point3, Vec3};
use crate::model::ElementId;
use crate::topo::FaceId;
use serde_json::{json, Value};

fn forms(s: &Session) -> Vec<ElementId> {
    s.doc()
        .elements()
        .filter(|e| e.as_form().is_some())
        .map(|e| e.id)
        .collect()
}

fn pt(q: Point3) -> Value {
    p(q.x, q.y, q.z)
}

/// A random rectangle in the plane of `face`, around a point near its
/// centroid (it may stick out of the face: that must fail cleanly).
fn rect_on_face(rng: &mut XorShift, s: &Session, e: ElementId, f: FaceId) -> Option<Vec<Value>> {
    let b = geometry(s, e);
    let pts = b.face_points(f).ok()?;
    let n = b.face_normal(f).ok()?.as_vec();
    let c = Point3::centroid(&pts)?;
    let u = (pts[1] - pts[0]).normalize()?.as_vec();
    let v = n.cross(u);
    let c = c + u * rng.range(-500.0, 500.0) + v * rng.range(-500.0, 500.0);
    let (a, h) = (rng.range(1.0, 1500.0), rng.range(1.0, 1500.0));
    Some(
        [(-a, -h), (a, -h), (a, h), (-a, h)]
            .iter()
            .map(|(x, y)| pt(c + u * *x + v * *y))
            .collect(),
    )
}

fn random_step(rng: &mut XorShift, s: &mut Session, step: usize) -> (String, Value) {
    let fs = forms(s);
    if fs.is_empty() || (fs.len() < 6 && rng.pick(10) == 0) {
        let o = [
            rng.range(-20_000.0, 20_000.0),
            rng.range(-20_000.0, 20_000.0),
            0.0,
        ];
        let args = json!({"origin": p(o[0], o[1], o[2]), "width": rng.range(-4000.0, 4000.0),
            "depth": rng.range(-4000.0, 4000.0), "height": rng.range(-3000.0, 3000.0)});
        return ("create_box".into(), args);
    }
    let e = fs[rng.pick(fs.len())];
    let b = geometry(s, e);
    let faces: Vec<FaceId> = b.face_ids().collect();
    let f = faces[rng.pick(faces.len())];
    let edges: Vec<_> = b.edges().map(|(id, _, _)| id).collect();
    let dist = |rng: &mut XorShift| {
        // Mostly ordinary distances, sometimes right at the tolerances.
        match rng.pick(6) {
            0 => [0.49, 0.51, -0.49, -0.51, 0.005, 0.02][rng.pick(6)],
            _ => rng.range(-3000.0, 3000.0),
        }
    };
    match rng.pick(11) {
        0 | 1 => (
            "push_pull_face".into(),
            json!({"element": e, "face": f, "distance": dist(rng)}),
        ),
        2 | 3 => match rect_on_face(rng, s, e, f) {
            Some(points) => (
                "imprint_face".into(),
                json!({"element": e, "face": f, "points": points, "closed": true}),
            ),
            None => (
                "set_project_name".into(),
                json!({"name": format!("step {step}")}),
            ),
        },
        4 => {
            let c =
                Point3::centroid(&b.face_points(f).unwrap_or_default()).unwrap_or(Point3::ORIGIN);
            (
                "create_circle".into(),
                json!({"element": e, "face": f, "center": pt(c), "radius": rng.range(0.2, 1500.0), "segments": 3 + rng.pick(40)}),
            )
        }
        5 => (
            "offset_face".into(),
            json!({"element": e, "face": f, "distance": dist(rng).abs()}),
        ),
        6 => (
            "move_geometry".into(),
            json!({"element": e, "faces": [f], "vector": p(dist(rng), dist(rng), dist(rng))}),
        ),
        7 => (
            "move_elements".into(),
            json!({"elements": [e], "vector": p(rng.range(-5000.0, 5000.0), rng.range(-5000.0, 5000.0), rng.range(-1000.0, 1000.0))}),
        ),
        8 => (
            "rotate_elements".into(),
            json!({"elements": [e], "center": p(rng.range(-5000.0, 5000.0), rng.range(-5000.0, 5000.0), 0.0), "angle": rng.range(-3.2, 3.2)}),
        ),
        9 => (
            "erase_edges".into(),
            json!({"element": e, "edges": [edges[rng.pick(edges.len())]]}),
        ),
        _ => (
            "scale_elements".into(),
            json!({"elements": [e], "center": pt(b.bounding_box().map(|bb| bb.min).unwrap_or(Point3::ORIGIN)), "factor": rng.range(0.2, 3.0)}),
        ),
    }
}

/// 200 random steps per seed, three seeds; the outcome of every step is
/// checked. A panic reports the seed, the step and the command.
#[test]
fn random_form_edits_keep_documents_valid() {
    let mut ok = 0;
    let mut failed = 0;
    for seed in [0x9E37_79B9_7F4A_7C15_u64, 42, 0xDEAD_BEEF] {
        let mut rng = XorShift(seed);
        let mut s = session();
        for step in 0..200 {
            if step % 37 == 36 && s.can_undo() {
                let before = snapshot(&s);
                s.undo();
                s.redo();
                assert_eq!(
                    snapshot(&s),
                    before,
                    "seed {seed:#x} step {step}: undo/redo"
                );
                continue;
            }
            let (tool, args) = random_step(&mut rng, &mut s, step);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                checked(&mut s, &tool, args.clone()).is_ok()
            }));
            match r {
                Ok(true) => ok += 1,
                Ok(false) => failed += 1,
                Err(_) => panic!("seed {seed:#x} step {step}: {tool} {args}"),
            }
        }
    }
    // The generator must exercise both paths.
    assert!(ok > 150 && failed > 30, "ok {ok}, failed {failed}");
}

/// Random rectangles pushed through or pulled out of boxes: when the push /
/// pull succeeds, the volume changes by exactly area × distance (prism and
/// stretch modes) or by area × depth (through).
#[test]
fn random_imprint_and_push_pull_volume_bookkeeping() {
    let mut rng = XorShift(7);
    let mut checked_steps = 0;
    for _ in 0..60 {
        let mut s = session();
        let (w, d, h) = (
            rng.range(1000.0, 5000.0),
            rng.range(1000.0, 5000.0),
            rng.range(1000.0, 5000.0),
        );
        let e = boxed(&mut s, [0.0; 3], w, d, h);
        let top = face_towards(&geometry(&s, e), Vec3::Z);
        // Rectangle strictly inside the top face.
        let (x0, y0) = (rng.range(10.0, w * 0.5), rng.range(10.0, d * 0.5));
        let (x1, y1) = (
            rng.range(x0 + 10.0, w - 10.0),
            rng.range(y0 + 10.0, d - 10.0),
        );
        let o = expect_ok(
            &mut s,
            "imprint_face",
            json!({"element": e, "face": top, "closed": true, "points": [
                p(x0, y0, h), p(x1, y0, h), p(x1, y1, h), p(x0, y1, h)]}),
        );
        let f = new_face(&o);
        let area = (x1 - x0) * (y1 - y0);
        let dist = match rng.pick(3) {
            0 => -h,
            1 => rng.range(-h + 1.0, -1.0),
            _ => rng.range(1.0, 3000.0),
        };
        let v0 = w * d * h;
        if checked(
            &mut s,
            "push_pull_face",
            json!({"element": e, "face": f, "distance": dist}),
        )
        .is_ok()
        {
            let v = geometry(&s, e).volume();
            assert!(
                (v - (v0 + area * dist)).abs() < 1e-6 * v0,
                "box {w}×{d}×{h}, rect {x0},{y0}..{x1},{y1}, distance {dist}: {v} vs {}",
                v0 + area * dist
            );
            checked_steps += 1;
        }
    }
    assert!(checked_steps >= 55, "{checked_steps}");
}
