//! Tests of geometry sources (ADR 0004 §1): freezing parametric elements,
//! classifying free-form geometry and recognising it as parametric again.

use super::*;
use crate::model::{Category, Document, ElementBody, ElementId, GeometrySource};
use serde_json::{json, Value};

fn session() -> Session {
    Session::new(Document::new("T"), Box::new(SequentialIds::new(1)))
}

fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    s.execute(
        AnyCommand::from_tool(tool, &args).expect("args"),
        &ExecContext::user(),
    )
}

fn run(s: &mut Session, tool: &str, args: Value) -> Outcome {
    try_run(s, tool, args).unwrap_or_else(|f| panic!("{tool}: {f:?}"))
}

fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).unwrap()
}

fn code(r: Result<Outcome, Vec<Failure>>) -> String {
    r.expect_err("should fail")[0].code.clone()
}

fn volume(s: &Session, e: ElementId) -> f64 {
    s.doc().get(e).unwrap().geometry().unwrap().volume()
}

fn source(s: &Session, e: ElementId) -> Option<GeometrySource> {
    s.doc().get(e).unwrap().geometry_source()
}

fn wall(s: &mut Session) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 6000, "y": 0, "z": 0}}),
    ))
}

fn boxed(s: &mut Session, w: f64, d: f64, h: f64) -> ElementId {
    id(&run(
        s,
        "create_box",
        json!({"origin": {"x": 0, "y": 0, "z": 0}, "width": w, "depth": d, "height": h}),
    ))
}

#[test]
fn freezing_a_wall_keeps_shape_type_and_freezes_its_door() {
    let mut s = session();
    let w = wall(&mut s);
    let door = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 1500}),
    ));
    let (v, ty) = (volume(&s, w), s.doc().get(w).unwrap().type_id);
    let o = run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    assert_eq!(o.output["changed"], json!([w, door]));
    let e = s.doc().get(w).unwrap();
    assert_eq!(
        (e.category, e.type_id, source(&s, w)),
        (Category::Walls, ty, Some(GeometrySource::Direct))
    );
    assert!((volume(&s, w) - v).abs() < 1e-6);
    assert_eq!(source(&s, door), Some(GeometrySource::Direct));
    assert!(volume(&s, door) > 0.0);
    // A frozen wall is edited directly: push / pull changes its solid.
    let g = s.doc().get(w).unwrap().geometry().unwrap();
    let top = g
        .face_ids()
        .find(|f| g.face_normal(*f).unwrap().as_vec().z > 0.99)
        .unwrap();
    run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": top, "distance": 500}),
    );
    assert!(volume(&s, w) > v);
    // Freezing again changes nothing.
    let o = run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    assert_eq!(o.output["changed"], json!([]));
}

#[test]
fn frozen_wall_becomes_parametric_again() {
    let mut s = session();
    let w = wall(&mut s);
    let v = volume(&s, w);
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "parametric"}),
    );
    let e = s.doc().get(w).unwrap();
    let wall = e.as_wall().expect("parametric wall");
    assert!((wall.curve.length() - 6000.0).abs() < 1e-6);
    assert!((volume(&s, w) - v).abs() < 1e-3);
}

#[test]
fn free_form_box_classified_as_wall_uses_a_matching_type() {
    let mut s = session();
    let w = wall(&mut s); // creates the 200 mm default type
    let ty = s.doc().get(w).unwrap().type_id;
    let b = boxed(&mut s, 4000.0, 200.0, 2800.0);
    run(
        &mut s,
        "set_category",
        json!({"element": b, "category": "walls"}),
    );
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": b, "source": "parametric"}),
    );
    let e = s.doc().get(b).unwrap();
    assert_eq!(e.type_id, ty);
    let wl = e.as_wall().expect("wall");
    assert!((wl.curve.length() - 4000.0).abs() < 1e-6);
    assert!((volume(&s, b) - 4000.0 * 200.0 * 2800.0).abs() < 1e-3);
}

#[test]
fn thickness_must_match_a_type_within_tolerance() {
    let mut s = session();
    wall(&mut s);
    // 0.005 mm off: still the 200 mm type.
    let ok = boxed(&mut s, 3000.0, 200.005, 2800.0);
    run(
        &mut s,
        "set_category",
        json!({"element": ok, "category": "walls"}),
    );
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": ok, "source": "parametric"}),
    );
    // 0.1 mm off: no type fits.
    let off = boxed(&mut s, 3000.0, 200.1, 2800.0);
    run(
        &mut s,
        "set_category",
        json!({"element": off, "category": "walls"}),
    );
    let c = code(try_run(
        &mut s,
        "set_geometry_source",
        json!({"element": off, "source": "parametric"}),
    ));
    assert_eq!(c, "ferro.model.no_matching_type");
}

#[test]
fn shapes_that_are_not_plain_walls_or_floors_are_not_recognised() {
    let mut s = session();
    wall(&mut s);
    // A box with a raised half on top is not a wall.
    let b = boxed(&mut s, 4000.0, 200.0, 2800.0);
    run(
        &mut s,
        "set_category",
        json!({"element": b, "category": "walls"}),
    );
    let g = s.doc().get(b).unwrap().geometry().unwrap().clone();
    let top = g
        .face_ids()
        .find(|f| g.face_normal(*f).unwrap().as_vec().z > 0.99)
        .unwrap();
    run(
        &mut s,
        "imprint_face",
        json!({"element": b, "face": top, "closed": false, "points": [
        {"x": 2000, "y": 0, "z": 2800}, {"x": 2000, "y": 200, "z": 2800}]}),
    );
    let g = s.doc().get(b).unwrap().geometry().unwrap().clone();
    let half = g
        .face_ids()
        .filter(|f| g.face_normal(*f).unwrap().as_vec().z > 0.99)
        .max_by(|x, y| {
            let cx = |f| {
                crate::math::Point3::centroid(&g.face_points(f).unwrap())
                    .unwrap()
                    .x
            };
            cx(*x).total_cmp(&cx(*y))
        })
        .unwrap();
    run(
        &mut s,
        "push_pull_face",
        json!({"element": b, "face": half, "distance": 400}),
    );
    let c = code(try_run(
        &mut s,
        "set_geometry_source",
        json!({"element": b, "source": "parametric"}),
    ));
    assert_eq!(c, "ferro.model.not_recognised");
    // Unclassified geometry has no parametric form.
    let f = boxed(&mut s, 1000.0, 1000.0, 1000.0);
    let c = code(try_run(
        &mut s,
        "set_geometry_source",
        json!({"element": f, "source": "parametric"}),
    ));
    assert_eq!(c, "ferro.model.not_recognised");
}

#[test]
fn l_shaped_slab_becomes_a_parametric_floor() {
    let mut s = session();
    run(
        &mut s,
        "create_floor_type",
        json!({"name": "Slab 150", "thickness": 150}),
    );
    let p = id(&run(
        &mut s,
        "create_polygon",
        json!({"points": [
        {"x": 0, "y": 0, "z": 0}, {"x": 6000, "y": 0, "z": 0}, {"x": 6000, "y": 3000, "z": 0},
        {"x": 3000, "y": 3000, "z": 0}, {"x": 3000, "y": 5000, "z": 0}, {"x": 0, "y": 5000, "z": 0}]}),
    ));
    let g = s.doc().get(p).unwrap().geometry().unwrap().clone();
    let face = g.face_ids().next().unwrap();
    let n = g.face_normal(face).unwrap().as_vec().z;
    run(
        &mut s,
        "push_pull_face",
        json!({"element": p, "face": face, "distance": 150.0 * n.signum()}),
    );
    let v = volume(&s, p);
    run(
        &mut s,
        "set_category",
        json!({"element": p, "category": "floors"}),
    );
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": p, "source": "parametric"}),
    );
    let e = s.doc().get(p).unwrap();
    let ElementBody::Floor(f) = &e.body else {
        panic!("{:?}", e.body)
    };
    assert_eq!(f.outer.len(), 6);
    assert!((f.offset.mm() - 150.0).abs() < 1e-6);
    assert!((volume(&s, p) - v).abs() < 1e-3);
}

#[test]
fn reclassifying_drops_a_type_that_no_longer_fits_and_openings_cannot_freeze_alone() {
    let mut s = session();
    let w = wall(&mut s);
    let door = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 1500}),
    ));
    let c = code(try_run(
        &mut s,
        "set_geometry_source",
        json!({"element": door, "source": "direct"}),
    ));
    assert_eq!(c, "ferro.model.hosted_opening");
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    run(
        &mut s,
        "set_category",
        json!({"element": w, "category": "floors"}),
    );
    assert_eq!(s.doc().get(w).unwrap().type_id, None);
    // Levels have no geometry source.
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L2", "elevation": 3000}),
    ));
    let c = code(try_run(
        &mut s,
        "set_geometry_source",
        json!({"element": l, "source": "direct"}),
    ));
    assert_eq!(c, "ferro.model.no_geometry");
}
