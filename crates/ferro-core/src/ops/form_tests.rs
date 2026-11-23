//! Tests of the SketchUp-style Form commands through the command path.

use super::*;
use crate::model::{Document, ElementId};
use crate::topo::{Body, FaceId};
use serde_json::{json, Value};

fn session() -> Session {
    Session::new(Document::new("T"), Box::new(SequentialIds::new(1)))
}

fn run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    s.execute(
        AnyCommand::from_tool(tool, &args).expect("args"),
        &ExecContext::user(),
    )
}

fn body(s: &Session, e: ElementId) -> Body {
    s.doc().get(e).unwrap().as_form().unwrap().body.clone()
}

fn face_towards(b: &Body, dir: [f64; 3]) -> FaceId {
    let d = crate::math::Vec3::new(dir[0], dir[1], dir[2]);
    b.face_ids()
        .max_by(|x, y| {
            let sx = b.face_normal(*x).unwrap().as_vec().dot(d);
            let sy = b.face_normal(*y).unwrap().as_vec().dot(d);
            sx.total_cmp(&sy)
        })
        .unwrap()
}

fn boxed(s: &mut Session) -> ElementId {
    let o = run(
        s,
        "create_box",
        json!({"origin": {"x":0,"y":0,"z":0}, "width": 4000, "depth": 3000, "height": 1000}),
    )
    .unwrap();
    serde_json::from_value(o.output["element"].clone()).unwrap()
}

#[test]
fn draw_a_rectangle_on_top_then_push_it_through() {
    let mut s = session();
    let e = boxed(&mut s);
    let top = face_towards(&body(&s, e), [0.0, 0.0, 1.0]);
    let o = run(&mut s, "imprint_face", json!({"element": e, "face": top, "closed": true, "points": [
        {"x":1000,"y":1000,"z":1000},{"x":2000,"y":1000,"z":1000},{"x":2000,"y":2000,"z":1000},{"x":1000,"y":2000,"z":1000}]})).unwrap();
    let f: FaceId = serde_json::from_value(o.output["new_faces"][0].clone()).unwrap();
    let o = run(
        &mut s,
        "push_pull_face",
        json!({"element": e, "face": f, "distance": -1000}),
    )
    .unwrap();
    assert_eq!(o.output["mode"], "through");
    let b = body(&s, e);
    b.validate_solid().unwrap();
    assert!((b.volume() - (1.2e7 - 1e6) * 1000.0).abs() < 1.0);
}

#[test]
fn circle_on_a_face_pulls_into_a_smooth_cylinder() {
    let mut s = session();
    let e = boxed(&mut s);
    let top = face_towards(&body(&s, e), [0.0, 0.0, 1.0]);
    let o = run(
        &mut s,
        "create_circle",
        json!({"center": {"x":2000,"y":1500,"z":1000}, "radius": 500, "element": e, "face": top}),
    )
    .unwrap();
    let f: FaceId = serde_json::from_value(o.output["new_faces"][0].clone()).unwrap();
    run(
        &mut s,
        "push_pull_face",
        json!({"element": e, "face": f, "distance": 800}),
    )
    .unwrap();
    let b = body(&s, e);
    b.validate_solid().unwrap();
    // 24 side faces share one smooth group.
    let groups: std::collections::BTreeSet<u32> = b
        .face_ids()
        .filter_map(|f| b.face(f).unwrap().smooth_group)
        .collect();
    assert_eq!(groups.len(), 1);
    let n = std::f64::consts::PI / 24.0;
    let area = 24.0 * 500.0 * 500.0 * n.sin() * n.cos();
    assert!((b.volume() - (1.2e10 + area * 800.0)).abs() < 1.0);
    // A free-standing circle on the ground.
    let o = run(
        &mut s,
        "create_circle",
        json!({"center": {"x":0,"y":0,"z":0}, "radius": 300, "segments": 12}),
    )
    .unwrap();
    let c: ElementId = serde_json::from_value(o.output["element"].clone()).unwrap();
    assert_eq!(body(&s, c).vertex_count(), 12);
    assert!(run(
        &mut s,
        "create_circle",
        json!({"center": {"x":0,"y":0,"z":0}, "radius": 0.1})
    )
    .is_err());
}

#[test]
fn offset_then_pull_makes_a_frame() {
    let mut s = session();
    let e = boxed(&mut s);
    let front = face_towards(&body(&s, e), [0.0, -1.0, 0.0]);
    let o = run(
        &mut s,
        "offset_face",
        json!({"element": e, "face": front, "distance": 100}),
    )
    .unwrap();
    let inner: FaceId = serde_json::from_value(o.output["new_faces"][0].clone()).unwrap();
    let b = body(&s, e);
    assert!((b.face_area(inner).unwrap() - 3800.0 * 800.0).abs() < 1e-6);
    run(
        &mut s,
        "push_pull_face",
        json!({"element": e, "face": inner, "distance": -50}),
    )
    .unwrap();
    body(&s, e).validate_solid().unwrap();
    assert!(run(
        &mut s,
        "offset_face",
        json!({"element": e, "face": inner, "distance": 5000})
    )
    .is_err());
}

#[test]
fn move_geometry_stretches_and_refuses_bending() {
    let mut s = session();
    let e = boxed(&mut s);
    let top = face_towards(&body(&s, e), [0.0, 0.0, 1.0]);
    run(
        &mut s,
        "move_geometry",
        json!({"element": e, "faces": [top], "vector": {"x":0,"y":0,"z":500}}),
    )
    .unwrap();
    assert!((body(&s, e).volume() - 1.8e10).abs() < 1.0);
    // Moving one top edge sideways makes a sloped roof: still planar faces.
    let b = body(&s, e);
    let edge = b
        .edges()
        .find(|(_, a, b2)| a.z > 1400.0 && b2.z > 1400.0 && (a.y - b2.y).abs() < 1e-6 && a.y < 1.0)
        .unwrap()
        .0;
    run(
        &mut s,
        "move_geometry",
        json!({"element": e, "edges": [edge], "vector": {"x":0,"y":1000,"z":0}}),
    )
    .unwrap();
    body(&s, e).validate_solid().unwrap();
    // Moving one corner up would bend faces.
    let b = body(&s, e);
    let v = b.vertices().find(|(_, p)| p.z > 1400.0).unwrap().0;
    let r = run(
        &mut s,
        "move_geometry",
        json!({"element": e, "vertices": [v], "vector": {"x":0,"y":0,"z":300}}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.geom.not_planar");
}

#[test]
fn scale_and_erase_edges() {
    let mut s = session();
    let e = boxed(&mut s);
    run(
        &mut s,
        "scale_elements",
        json!({"elements": [e], "center": {"x":0,"y":0,"z":0}, "factor": 2.0}),
    )
    .unwrap();
    assert!((body(&s, e).volume() - 8.0 * 1.2e10).abs() < 10.0);
    run(
        &mut s,
        "scale_elements",
        json!({"elements": [e], "center": {"x":0,"y":0,"z":0}, "factors": [0.5, 1.0, 1.0]}),
    )
    .unwrap();
    assert!((body(&s, e).volume() - 4.0 * 1.2e10).abs() < 10.0);
    assert!(run(
        &mut s,
        "scale_elements",
        json!({"elements": [e], "center": {"x":0,"y":0,"z":0}, "factor": 0.0})
    )
    .is_err());

    // Split the top, then erase the split line: the faces merge back.
    let top = face_towards(&body(&s, e), [0.0, 0.0, 1.0]);
    run(&mut s, "imprint_face", json!({"element": e, "face": top, "points": [{"x":1000,"y":0,"z":2000},{"x":1000,"y":6000,"z":2000}]})).unwrap();
    assert_eq!(body(&s, e).face_count(), 7);
    let b = body(&s, e);
    let edge = b
        .edges()
        .find(|(_, a, b2)| {
            (a.x - 1000.0).abs() < 1e-6
                && (b2.x - 1000.0).abs() < 1e-6
                && a.z > 1999.0
                && b2.z > 1999.0
        })
        .unwrap()
        .0;
    let o = run(
        &mut s,
        "erase_edges",
        json!({"element": e, "edges": [edge]}),
    )
    .unwrap();
    assert_eq!(o.output["merged"], 1);
    let b = body(&s, e);
    assert_eq!(b.face_count(), 6);
    b.validate_solid().unwrap();
    // Erasing a box edge removes its two faces: the Form is open now.
    let edge = b.edges().next().unwrap().0;
    let o = run(
        &mut s,
        "erase_edges",
        json!({"element": e, "edges": [edge]}),
    )
    .unwrap();
    assert_eq!(o.output["removed_faces"], 2);
    assert!(!body(&s, e).is_closed());
}

#[test]
fn erase_faces_opens_the_form_then_deletes_it() {
    let mut s = session();
    let e = boxed(&mut s);
    let top = face_towards(&body(&s, e), [0.0, 0.0, 1.0]);
    run(&mut s, "erase_faces", json!({"element": e, "faces": [top]})).unwrap();
    let b = body(&s, e);
    assert_eq!(b.face_count(), 5);
    assert!(!b.is_closed());
    let all: Vec<FaceId> = b.face_ids().collect();
    let o = run(&mut s, "erase_faces", json!({"element": e, "faces": all})).unwrap();
    assert_eq!(o.output["deleted"], true);
    assert!(s.doc().get(e).is_none());
    assert!(run(&mut s, "erase_faces", json!({"element": e, "faces": [1]})).is_err());
}
