use super::*;
use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
use ferro_core::{Document, Session};
use serde_json::{json, Value};
use std::time::Instant;

fn session() -> Session {
    Session::new(Document::new("t"), Box::new(SequentialIds::new(1)))
}

fn run(s: &mut Session, tool: &str, args: Value) -> Value {
    let cmd = AnyCommand::from_tool(tool, &args).unwrap();
    s.execute(cmd, &ExecContext::user()).unwrap().output
}

fn id(v: &Value) -> ElementId {
    serde_json::from_value(v["element"].clone()).unwrap()
}

fn ids(v: &Value) -> Vec<ElementId> {
    serde_json::from_value(v["elements"].clone()).unwrap()
}

fn create_box(s: &mut Session, x: f64, y: f64, w: f64) -> ElementId {
    id(&run(
        s,
        "create_box",
        json!({"origin":{"x":x,"y":y,"z":0},"width":w,"depth":1000,"height":3000}),
    ))
}

fn move_by(s: &mut Session, e: ElementId, x: f64) {
    run(
        s,
        "move_elements",
        json!({"elements":[e],"vector":{"x":x,"y":0,"z":0}}),
    );
}

#[test]
fn box_gives_twelve_triangles_and_twelve_edges() {
    let mut s = session();
    let b = create_box(&mut s, 1000.0, 2000.0, 4000.0);
    let sc = DisplayScene::from_document(s.doc());
    assert_eq!(sc.len(), 1);
    let (r, m) = sc.local_mesh(b).unwrap();
    assert_eq!(m.mesh.indices.len(), 12);
    assert_eq!(m.edges.len(), 12);
    assert_eq!(m.users, 1);
    assert_eq!(r.category, Category::Forms);
    // Local mesh starts at its minimum corner; the offset brings it back.
    assert!((r.offset - Vec3::new(1000.0, 2000.0, 0.0)).length() < 1e-9);
    let bb = r.bounds.unwrap();
    assert!(bb.max.is_almost_equal(Point3::new(5000.0, 3000.0, 3000.0)));
    let w = sc.world_mesh(b).unwrap();
    assert!(w
        .positions
        .iter()
        .any(|p| p.is_almost_equal(Point3::new(1000.0, 2000.0, 0.0))));
    assert!(sc.bounds().is_some());
}

#[test]
fn empty_document_and_element_without_geometry() {
    let mut sc = DisplayScene::new();
    let mut s = session();
    let st = sc.sync(s.doc());
    assert!(sc.is_empty());
    assert_eq!(st.meshed, 0);
    assert!(sc.bounds().is_none());
    assert!(sc
        .section_segments(Point3::ORIGIN, UnitVec3::X, |_| false)
        .is_empty());
    let level = id(&run(
        &mut s,
        "create_level",
        json!({"name":"L1","elevation":0}),
    ));
    sc.sync(s.doc());
    assert!(sc.record(level).is_none());
    assert!(sc.world_mesh(level).is_none());
    assert!(sc.is_empty());
}

#[test]
fn open_body_keeps_its_boundary_edges() {
    let mut s = session();
    let r = id(&run(
        &mut s,
        "create_rectangle",
        json!({"origin":{"x":0,"y":0,"z":0},"width":2000,"depth":1000}),
    ));
    let sc = DisplayScene::from_document(s.doc());
    let (_, m) = sc.local_mesh(r).unwrap();
    assert_eq!(m.mesh.indices.len(), 2);
    assert_eq!(m.edges.len(), 4);
}

#[test]
fn changing_one_element_remeshes_only_it() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    let b = create_box(&mut s, 5000.0, 0.0, 2000.0);
    let _c = create_box(&mut s, 10000.0, 0.0, 3000.0);
    let mut sc = DisplayScene::from_document(s.doc());
    let rev = sc.revision();
    let rb = sc.record(b).unwrap().revision;

    // Same document: nothing to do.
    assert_eq!(sc.sync(s.doc()), SyncStats::default());
    assert_eq!(sc.revision(), rev);

    move_by(&mut s, a, 500.0);
    let st = sc.sync(s.doc());
    assert_eq!(st.meshed, 1);
    assert_eq!(st.reused, 2);
    assert!(sc.revision() > rev);
    assert_eq!(sc.record(b).unwrap().revision, rb);
    assert!(sc.record(a).unwrap().revision > rb);
    assert!((sc.record(a).unwrap().offset.x - 500.0).abs() < 1e-9);

    // Delete b: one record removed, its mesh freed.
    run(&mut s, "delete_elements", json!({"elements":[b]}));
    let st = sc.sync(s.doc());
    assert_eq!((st.meshed, st.removed), (0, 1));
    assert!(sc.record(b).is_none());
    assert_eq!(sc.shared_meshes().count(), 2);

    // Undo goes back to a previous version: b is meshed again, a and c kept.
    s.undo();
    let st = sc.sync(s.doc());
    assert_eq!((st.meshed, st.reused), (1, 2));
}

#[test]
fn array_copies_share_one_mesh() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    let out = run(
        &mut s,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":1500.3,"y":-700.1,"z":0},"count":4}),
    );
    let copies = ids(&out);
    assert_eq!(copies.len(), 4);
    let mut sc = DisplayScene::from_document(s.doc());
    assert_eq!(sc.len(), 5);
    assert_eq!(sc.shared_meshes().count(), 1);
    let (mid, m) = sc.shared_meshes().next().unwrap();
    assert_eq!(m.users, 5);
    assert!(sc.records().all(|r| r.mesh == mid));
    // Each copy is drawn where it is.
    let w = sc.world_mesh(copies[3]).unwrap();
    assert!(w.positions.iter().any(|p| p.is_almost_equal(Point3::new(
        4.0 * 1500.3,
        4.0 * -700.1,
        0.0
    ))));

    // A moved copy is re-meshed but stays an instance.
    move_by(&mut s, copies[1], 333.3);
    let st = sc.sync(s.doc());
    assert_eq!(st.meshed, 1);
    assert_eq!(sc.shared_meshes().count(), 1);
    assert_eq!(sc.record(copies[1]).unwrap().mesh, mid);

    // An edited copy leaves the group.
    let face = s
        .doc()
        .get(copies[2])
        .and_then(|e| e.geometry())
        .and_then(|g| g.face_ids().next())
        .unwrap();
    run(
        &mut s,
        "push_pull_face",
        json!({"element":copies[2],"face":face,"distance":250}),
    );
    sc.sync(s.doc());
    assert_eq!(sc.shared_meshes().count(), 2);
    assert_ne!(sc.record(copies[2]).unwrap().mesh, mid);
    assert_eq!(sc.shared_mesh(mid).unwrap().users, 4);

    // Deleting all but the edited one frees the shared mesh.
    let rest: Vec<ElementId> = std::iter::once(a)
        .chain(copies.iter().copied())
        .filter(|x| *x != copies[2])
        .collect();
    run(&mut s, "delete_elements", json!({ "elements": rest }));
    sc.sync(s.doc());
    assert!(sc.shared_mesh(mid).is_none());
    assert_eq!(sc.shared_meshes().count(), 1);
}

#[test]
fn rotated_copy_is_not_an_instance() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 2000.0);
    let c = ids(&run(
        &mut s,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":5000,"y":0,"z":0}}),
    ))[0];
    run(
        &mut s,
        "rotate_elements",
        json!({"elements":[c],"center":{"x":5000,"y":0,"z":0},"angle":0.5}),
    );
    let sc = DisplayScene::from_document(s.doc());
    assert_eq!(sc.shared_meshes().count(), 2);
}

#[test]
fn instance_match_within_tolerance_only() {
    let mut s = session();
    // Widths differing by less than the tolerance share; by more do not.
    create_box(&mut s, 0.0, 0.0, 1000.0);
    create_box(&mut s, 3000.0, 0.0, 1000.0 + INSTANCE_TOL * 0.01);
    create_box(&mut s, 6000.0, 0.0, 1000.0 + 1e-3);
    let sc = DisplayScene::from_document(s.doc());
    assert_eq!(sc.shared_meshes().count(), 2);
}

#[test]
fn same_document_gives_same_scene() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    run(
        &mut s,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":1200,"y":0,"z":0},"count":3}),
    );
    create_box(&mut s, 0.0, 4000.0, 2500.0);
    let x = DisplayScene::from_document(s.doc());
    let y = DisplayScene::from_document(s.doc());
    let rx: Vec<_> = x.records().cloned().collect();
    let ry: Vec<_> = y.records().cloned().collect();
    assert_eq!(rx, ry);
    let mx: Vec<_> = x.shared_meshes().map(|(i, m)| (i, m.clone())).collect();
    let my: Vec<_> = y.shared_meshes().map(|(i, m)| (i, m.clone())).collect();
    assert_eq!(mx, my);
}

#[test]
fn section_cut_through_instances() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    let c = ids(&run(
        &mut s,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":2000,"y":0,"z":0}}),
    ))[0];
    let sc = DisplayScene::from_document(s.doc());
    // Horizontal plane at z = 1500 cuts both boxes: 4 sides × 2 triangles.
    let cut = sc.section_segments(Point3::new(0.0, 0.0, 1500.0), UnitVec3::Z, |_| false);
    assert_eq!(cut.len(), 16);
    assert!(cut
        .iter()
        .all(|(p, q)| (p.z - 1500.0).abs() < 1e-9 && (q.z - 1500.0).abs() < 1e-9));
    assert!(cut.iter().any(|(p, _)| p.x > 1999.0));
    // Hidden elements are skipped; a plane missing the boxes cuts nothing.
    let only_a = sc.section_segments(Point3::new(0.0, 0.0, 1500.0), UnitVec3::Z, |e| e == c);
    assert_eq!(only_a.len(), 8);
    assert!(sc
        .section_segments(Point3::new(0.0, 0.0, 9000.0), UnitVec3::Z, |_| false)
        .is_empty());
}

#[test]
fn section_plane_on_a_face_within_tolerance() {
    let mut s = session();
    create_box(&mut s, 0.0, 0.0, 1000.0);
    let sc = DisplayScene::from_document(s.doc());
    // Just inside the tolerance of the top face: the top triangles lie in
    // the plane (no segment); side triangles touch it along their top edge.
    let p = Point3::new(0.0, 0.0, 3000.0 + 0.005);
    let cut = sc.section_segments(p, UnitVec3::Z, |_| false);
    assert!(cut
        .iter()
        .all(|(a, b)| (a.z - 3000.0).abs() < 1e-9 && (b.z - 3000.0).abs() < 1e-9));
}

#[test]
fn category_change_reuses_the_stored_mesh() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    let mut sc = DisplayScene::from_document(s.doc());
    let before = sc.record(a).unwrap().clone();
    run(
        &mut s,
        "set_category",
        json!({"element": a, "category": "furniture"}),
    );
    // Category change edits the element, so it is re-meshed; the mesh is
    // identical and reused from the store.
    let st = sc.sync(s.doc());
    assert_eq!(st.meshed, 1);
    let after = sc.record(a).unwrap();
    assert_eq!(after.mesh, before.mesh);
    assert_eq!(after.category, Category::Furniture);
    assert!(!same_color(after.color, before.color));
}

#[test]
fn editing_a_material_recolours_without_remeshing() {
    let mut s = session();
    let a = create_box(&mut s, 0.0, 0.0, 1000.0);
    let b = create_box(&mut s, 3000.0, 0.0, 1000.0);
    let m = id(&run(
        &mut s,
        "create_material",
        json!({"name": "Brick", "color": "#b5533c"}),
    ));
    let value = serde_json::to_value(ferro_core::ParamValue::ElementRef(m)).unwrap();
    run(
        &mut s,
        "set_parameter",
        json!({"element": a, "key": ferro_core::model::keys::MATERIAL, "value": value}),
    );
    let mut sc = DisplayScene::from_document(s.doc());
    let rb = sc.record(b).unwrap().revision;
    let before = sc.record(a).unwrap().clone();
    run(
        &mut s,
        "set_material",
        json!({"material": m, "color": "#808080"}),
    );
    let st = sc.sync(s.doc());
    assert_eq!((st.meshed, st.recolored), (0, 1));
    let after = sc.record(a).unwrap();
    assert_eq!(after.mesh, before.mesh);
    assert!(after.revision > before.revision);
    assert!(!same_color(after.color, before.color));
    assert_eq!(sc.record(b).unwrap().revision, rb);
}

/// Timings for D-62 budgets. Run with
/// `cargo test -p ferro-scene --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn benchmark_two_thousand_boxes() {
    let mut s = session();
    let mut boxes = Vec::new();
    for i in 0..2000 {
        let (x, y) = ((i % 50) as f64 * 3000.0, (i / 50) as f64 * 3000.0);
        // Distinct widths: no instancing, the worst case for meshing.
        boxes.push(create_box(&mut s, x, y, 1000.0 + i as f64));
    }
    let t = Instant::now();
    let mut sc = DisplayScene::from_document(s.doc());
    let full = t.elapsed();

    move_by(&mut s, boxes[1000], 10.0);
    let t = Instant::now();
    let st = sc.sync(s.doc());
    let one = t.elapsed();
    assert_eq!(st.meshed, 1);

    // A selection change does not change the document.
    let t = Instant::now();
    let st = sc.sync(s.doc());
    let sel = t.elapsed();
    assert_eq!(st, SyncStats::default());

    // Same scene made of copies: one shared mesh.
    let mut c = session();
    let a = create_box(&mut c, 0.0, 0.0, 1000.0);
    run(
        &mut c,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":3000,"y":0,"z":0},"count":999}),
    );
    run(
        &mut c,
        "copy_elements",
        json!({"elements":[a],"vector":{"x":0,"y":3000,"z":0},"count":1000}),
    );
    let t = Instant::now();
    let ci = DisplayScene::from_document(c.doc());
    let copies = t.elapsed();
    assert_eq!(ci.len(), 2000);
    assert_eq!(ci.shared_meshes().count(), 1);

    println!("ferro-scene 2,000 boxes: full build {full:?}, one element changed {one:?}, selection change {sel:?}, 2,000 copies full build {copies:?} (1 shared mesh)");
}
