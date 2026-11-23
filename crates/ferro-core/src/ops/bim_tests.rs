//! Tests of milestone 2 BIM elements through the public command path.

use super::*;
use crate::math::Vec3;
use crate::model::{keys, Category, Document, ElementBody, ElementId, ParamValue};
use crate::topo::FaceId;
use serde_json::{json, Value};

fn session() -> Session {
    Session::new(
        Document::new("Test"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    )
}

fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    let cmd = AnyCommand::from_tool(tool, &args).expect("valid args");
    s.execute(cmd, &ExecContext::user())
}

fn run(s: &mut Session, tool: &str, args: Value) -> Outcome {
    match try_run(s, tool, args) {
        Ok(o) => o,
        Err(f) => panic!("{tool} failed: {f:?}"),
    }
}

fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).unwrap()
}

fn ids(o: &Outcome) -> Vec<ElementId> {
    serde_json::from_value(o.output["elements"].clone()).unwrap()
}

fn volume(s: &Session, e: ElementId) -> f64 {
    let g = s.doc().get(e).unwrap().geometry().unwrap();
    g.validate_solid().expect("valid solid");
    g.volume()
}

fn length_param(s: &Session, e: ElementId, key: &str) -> f64 {
    match s.doc().get(e).unwrap().param(key) {
        Some(ParamValue::Length(l)) => l.mm(),
        other => panic!("{key}: {other:?}"),
    }
}

/// Face whose normal is closest to `dir`; ties broken by the largest
/// centroid coordinate along `dir`.
fn face_towards(s: &Session, e: ElementId, dir: Vec3) -> FaceId {
    let g = s.doc().get(e).unwrap().geometry().unwrap();
    let mut best = None;
    for f in g.face_ids() {
        let n = g.face_normal(f).unwrap().as_vec();
        let c = crate::math::Point3::centroid(&g.face_points(f).unwrap()).unwrap();
        let score = (n.dot(dir) * 1000.0).round() * 1e6 + c.to_vec().dot(dir);
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, f));
        }
    }
    best.unwrap().1
}

fn wall(s: &mut Session, x0: f64, y0: f64, x1: f64, y1: f64) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": {"x": x0, "y": y0, "z": 0}, "end": {"x": x1, "y": y1, "z": 0}}),
    ))
}

const T: f64 = 200.0;
const H: f64 = 3000.0;

#[test]
fn single_wall_is_a_solid_with_computed_dimensions() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 5000.0, 0.0);
    assert!((volume(&s, w) - 5000.0 * T * H).abs() < 1.0);
    assert!((length_param(&s, w, keys::LENGTH) - 5000.0).abs() < 1e-6);
    assert!((length_param(&s, w, keys::HEIGHT) - H).abs() < 1e-6);
    assert!((length_param(&s, w, keys::THICKNESS) - T).abs() < 1e-6);
    // A default type was created and assigned.
    let ty = s.doc().get(w).unwrap().type_id.unwrap();
    assert_eq!(
        s.doc().get(ty).unwrap().name.as_deref(),
        Some("Generic - 200mm")
    );
    // Exterior (left of +x) is +y.
    let bb = s.doc().get(w).unwrap().bounding_box().unwrap();
    assert!((bb.max.y - 100.0).abs() < 1e-6 && (bb.min.y + 100.0).abs() < 1e-6);
}

#[test]
fn closed_chain_is_mitred_without_overlap() {
    let mut s = session();
    let o = run(
        &mut s,
        "create_walls",
        json!({"points": [
            {"x": 0, "y": 0, "z": 0}, {"x": 10000, "y": 0, "z": 0},
            {"x": 10000, "y": 8000, "z": 0}, {"x": 0, "y": 8000, "z": 0}],
            "closed": true}),
    );
    let walls = ids(&o);
    assert_eq!(walls.len(), 4);
    let total: f64 = walls.iter().map(|w| volume(&s, *w)).sum();
    let expected = (10200.0 * 8200.0 - 9800.0 * 7800.0) * H;
    assert!((total - expected).abs() < 10.0, "{total} vs {expected}");
}

#[test]
fn reversed_chain_also_mitres() {
    let mut s = session();
    let o = run(
        &mut s,
        "create_walls",
        json!({"points": [
            {"x": 0, "y": 0, "z": 0}, {"x": 0, "y": 8000, "z": 0},
            {"x": 10000, "y": 8000, "z": 0}, {"x": 10000, "y": 0, "z": 0}],
            "closed": true}),
    );
    let total: f64 = ids(&o).iter().map(|w| volume(&s, *w)).sum();
    let expected = (10200.0 * 8200.0 - 9800.0 * 7800.0) * H;
    assert!((total - expected).abs() < 10.0);
}

#[test]
fn three_walls_at_a_point_stay_butt_jointed() {
    let mut s = session();
    let a = wall(&mut s, 0.0, 0.0, 3000.0, 0.0);
    let b = wall(&mut s, 3000.0, 0.0, 6000.0, 0.0);
    let c = wall(&mut s, 3000.0, 0.0, 3000.0, 3000.0);
    for w in [a, b, c] {
        assert!((volume(&s, w) - 3000.0 * T * H).abs() < 1.0);
    }
}

#[test]
fn door_and_window_cut_the_wall() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 6000.0, 0.0);
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 1500}),
    ));
    let win = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "window", "position": 4000}),
    ));
    let full = 6000.0 * T * H;
    let expected = full - 900.0 * 2100.0 * T - 1200.0 * 1200.0 * T;
    assert!((volume(&s, w) - expected).abs() < 1.0);
    assert_eq!(s.doc().get(d).unwrap().category, Category::Doors);
    assert_eq!(s.doc().get(d).unwrap().host_id, Some(w));
    assert!(volume(&s, d) > 0.0);
    assert!(volume(&s, win) > 0.0);
}

#[test]
fn opening_that_does_not_fit_is_rejected() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 2000.0, 0.0);
    let r = try_run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "window", "position": 1900}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.opening.outside_host");
    let r = try_run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 1000, "height": 3500}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.opening.too_tall");
    // Shortening the wall later only warns.
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 1400}),
    ));
    let end = face_towards(&s, w, Vec3::new(1.0, 0.0, 0.0));
    let o = run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": end, "distance": -800}),
    );
    assert!(o
        .warnings
        .iter()
        .any(|f| f.code == "ferro.opening.outside_host"));
    assert!(s.doc().get(d).unwrap().geometry().unwrap().face_count() == 0);
}

#[test]
fn walls_follow_their_levels() {
    let mut s = session();
    let l1 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    let l2 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L2", "elevation": 3500}),
    ));
    let w = id(&run(
        &mut s,
        "create_wall",
        json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 4000, "y": 0, "z": 0},
               "level": l1, "top_level": l2}),
    ));
    assert!((length_param(&s, w, keys::HEIGHT) - 3500.0).abs() < 1e-6);
    run(
        &mut s,
        "set_level_elevation",
        json!({"level": l2, "elevation": 4000}),
    );
    assert!((length_param(&s, w, keys::HEIGHT) - 4000.0).abs() < 1e-6);
    run(
        &mut s,
        "set_level_elevation",
        json!({"level": l1, "elevation": 500}),
    );
    let bb = s.doc().get(w).unwrap().bounding_box().unwrap();
    assert!((bb.min.z - 500.0).abs() < 1e-6 && (bb.max.z - 4000.0).abs() < 1e-6);
    // Deleting the top level keeps the wall where it is.
    let o = run(&mut s, "delete_elements", json!({"elements": [l2]}));
    assert!(!o.warnings.is_empty());
    let bb = s.doc().get(w).unwrap().bounding_box().unwrap();
    assert!((bb.max.z - 4000.0).abs() < 1e-6);
}

#[test]
fn semantic_push_pull_edits_parameters() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 4000.0, 0.0);
    let top = face_towards(&s, w, Vec3::new(0.0, 0.0, 1.0));
    let o = run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": top, "distance": 500}),
    );
    assert_eq!(o.output["parameter"], "height");
    assert!((length_param(&s, w, keys::HEIGHT) - 3500.0).abs() < 1e-6);

    let end = face_towards(&s, w, Vec3::new(1.0, 0.0, 0.0));
    run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": end, "distance": 1000}),
    );
    assert!((length_param(&s, w, keys::LENGTH) - 5000.0).abs() < 1e-6);

    let start = face_towards(&s, w, Vec3::new(-1.0, 0.0, 0.0));
    run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": start, "distance": 500}),
    );
    assert!((length_param(&s, w, keys::LENGTH) - 5500.0).abs() < 1e-6);

    let side = face_towards(&s, w, Vec3::new(0.0, 1.0, 0.0));
    let o = run(
        &mut s,
        "push_pull_face",
        json!({"element": w, "face": side, "distance": 300}),
    );
    assert_eq!(o.output["parameter"], "location");
    let bb = s.doc().get(w).unwrap().bounding_box().unwrap();
    assert!((bb.max.y - 400.0).abs() < 1e-6);
    // Thickness is never changed by push/pull.
    assert!((length_param(&s, w, keys::THICKNESS) - T).abs() < 1e-6);
}

#[test]
fn push_pull_on_openings_resizes_them() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 6000.0, 0.0);
    let win = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "window", "position": 3000}),
    ));
    let right = face_towards(&s, win, Vec3::new(1.0, 0.0, 0.0));
    run(
        &mut s,
        "push_pull_face",
        json!({"element": win, "face": right, "distance": 300}),
    );
    let ElementBody::Opening(o) = &s.doc().get(win).unwrap().body else {
        panic!()
    };
    assert!((o.width.mm() - 1500.0).abs() < 1e-6);
    assert!((o.position.mm() - 3150.0).abs() < 1e-6);
    let expected = 6000.0 * T * H - 1500.0 * 1200.0 * T;
    assert!((volume(&s, w) - expected).abs() < 1.0);
}

#[test]
fn deleting_a_wall_deletes_its_openings_and_undo_restores() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 6000.0, 0.0);
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 2000}),
    ));
    run(&mut s, "delete_elements", json!({"elements": [w]}));
    assert!(s.doc().get(d).is_none());
    s.undo();
    assert!(s.doc().get(d).is_some());
    assert!((volume(&s, w) - (6000.0 * T * H - 900.0 * 2100.0 * T)).abs() < 1.0);
}

#[test]
fn types_in_use_cannot_be_deleted_and_type_change_regenerates() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 4000.0, 0.0);
    let generic = s.doc().get(w).unwrap().type_id.unwrap();
    let r = try_run(&mut s, "delete_elements", json!({"elements": [generic]}));
    assert_eq!(r.unwrap_err()[0].code, "ferro.model.type_in_use");

    let brick = id(&run(
        &mut s,
        "create_material",
        json!({"name": "Brick", "color": "#b5533c"}),
    ));
    let ty = id(&run(
        &mut s,
        "create_wall_type",
        json!({"name": "Brick 220", "layers": [
            {"function": "finish", "thickness": 15},
            {"function": "structure", "thickness": 190, "material": brick},
            {"function": "finish", "thickness": 15}]}),
    ));
    run(
        &mut s,
        "set_element_type",
        json!({"element": w, "element_type": ty}),
    );
    assert!((length_param(&s, w, keys::THICKNESS) - 220.0).abs() < 1e-6);
    assert!((volume(&s, w) - 4000.0 * 220.0 * H).abs() < 1.0);
    let r = try_run(
        &mut s,
        "create_material",
        json!({"name": "Bad", "color": "red"}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.model.invalid_color");
}

#[test]
fn location_line_places_the_faces() {
    let mut s = session();
    let w = id(&run(
        &mut s,
        "create_wall",
        json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 4000, "y": 0, "z": 0},
               "location_line": "finish_interior"}),
    ));
    let bb = s.doc().get(w).unwrap().bounding_box().unwrap();
    assert!(bb.min.y.abs() < 1e-6 && (bb.max.y - T).abs() < 1e-6);
}

#[test]
fn moving_and_copying_walls_carries_openings() {
    let mut s = session();
    let w = wall(&mut s, 0.0, 0.0, 6000.0, 0.0);
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 2000}),
    ));
    run(
        &mut s,
        "move_elements",
        json!({"elements": [w], "vector": {"x": 0, "y": 5000, "z": 0}}),
    );
    let bb = s.doc().get(d).unwrap().bounding_box().unwrap();
    assert!(bb.min.y > 4000.0);
    // Moving the door alone slides it along the wall.
    run(
        &mut s,
        "move_elements",
        json!({"elements": [d], "vector": {"x": 1000, "y": 300, "z": 0}}),
    );
    let ElementBody::Opening(o) = &s.doc().get(d).unwrap().body else {
        panic!()
    };
    assert!((o.position.mm() - 3000.0).abs() < 1e-6);
    let o = run(
        &mut s,
        "copy_elements",
        json!({"elements": [w], "vector": {"x": 0, "y": 3000, "z": 0}}),
    );
    let copies = ids(&o);
    assert_eq!(copies.len(), 2);
    assert_eq!(s.doc().get(copies[1]).unwrap().host_id, Some(copies[0]));
    assert!((volume(&s, copies[0]) - volume(&s, w)).abs() < 1.0);
}

#[test]
fn floors_extrude_down_from_their_level() {
    let mut s = session();
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L2", "elevation": 3000}),
    ));
    let f = id(&run(
        &mut s,
        "create_floor",
        json!({"points": [{"x": 0, "y": 0}, {"x": 6000, "y": 0}, {"x": 6000, "y": 4000}, {"x": 0, "y": 4000}],
               "holes": [[{"x": 1000, "y": 1000}, {"x": 2000, "y": 1000}, {"x": 2000, "y": 2000}, {"x": 1000, "y": 2000}]],
               "level": l}),
    ));
    let net = 6000.0 * 4000.0 - 1000.0 * 1000.0;
    assert!((volume(&s, f) - net * 200.0).abs() < 1.0);
    let bb = s.doc().get(f).unwrap().bounding_box().unwrap();
    assert!((bb.max.z - 3000.0).abs() < 1e-6 && (bb.min.z - 2800.0).abs() < 1e-6);
    match s.doc().get(f).unwrap().param(keys::AREA) {
        Some(ParamValue::Area(a)) => assert!((a - net).abs() < 1.0),
        other => panic!("{other:?}"),
    }
    let top = face_towards(&s, f, Vec3::new(0.0, 0.0, 1.0));
    run(
        &mut s,
        "push_pull_face",
        json!({"element": f, "face": top, "distance": 50}),
    );
    let bb = s.doc().get(f).unwrap().bounding_box().unwrap();
    assert!((bb.max.z - 3050.0).abs() < 1e-6);
    let side = face_towards(&s, f, Vec3::new(1.0, 0.0, 0.0));
    assert!(try_run(
        &mut s,
        "push_pull_face",
        json!({"element": f, "face": side, "distance": 50})
    )
    .is_err());
}

#[test]
fn arc_walls_are_faceted_solids() {
    let mut s = session();
    let w = id(&run(
        &mut s,
        "create_arc_wall",
        json!({"start": {"x": 5000, "y": 0, "z": 0}, "mid": {"x": 0, "y": 5000, "z": 0},
               "end": {"x": -5000, "y": 0, "z": 0}}),
    ));
    let v = volume(&s, w);
    let exact = std::f64::consts::PI * 5000.0 * T * H; // half annulus
    assert!((v - exact).abs() / exact < 0.01, "{v} vs {exact}");
    let r = try_run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "window", "position": 2000}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.opening.curved_host");
}

#[test]
fn degenerate_inputs_fail_cleanly() {
    let mut s = session();
    let r = try_run(
        &mut s,
        "create_wall",
        json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 0.1, "y": 0, "z": 500}}),
    );
    assert!(r.is_err());
    let r = try_run(
        &mut s,
        "create_wall",
        json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 1000, "y": 0, "z": 0}, "height": -10}),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.wall.no_height");
    let r = try_run(
        &mut s,
        "create_wall_type",
        json!({"name": "Empty", "thickness": 0}),
    );
    assert!(r.is_err());
    // Nothing was committed by the failures except nothing.
    assert!(s.doc().is_empty());
}

#[test]
fn regeneration_is_deterministic() {
    let build = || {
        let mut s = session();
        run(
            &mut s,
            "create_walls",
            json!({"points": [{"x": 0, "y": 0, "z": 0}, {"x": 4000, "y": 0, "z": 0}, {"x": 4000, "y": 3000, "z": 0}]}),
        );
        s.doc().clone()
    };
    let (a, b) = (build(), build());
    for e in a.elements() {
        assert_eq!(Some(e), b.get(e.id));
    }
}
