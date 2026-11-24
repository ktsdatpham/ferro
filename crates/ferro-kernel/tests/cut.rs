//! Non-destructive cuts (M5-5.3) and phased cuts (M5-23.6) through a
//! session with `SolidcoreKernel`: exact volumes of walls cut by boxes,
//! flush and D-06 tolerance-edge cutters, rotation and distance from the
//! origin, regeneration on edits, unite / intersect. Lengths in mm.

use ferro_core::model::{keys, ElementId, ParamValue, PhaseQuantities, PhaseStatus};
use ferro_core::ops::{AnyCommand, ExecContext, Failure, Outcome, SequentialIds};
use ferro_core::{Document, Session};
use ferro_kernel::SolidcoreKernel;
use serde_json::{json, Value};
use std::sync::Arc;

fn session() -> Session {
    Session::new(
        Document::new("Cuts"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    )
    .with_kernel(Arc::new(SolidcoreKernel::new()))
}

fn p(x: f64, y: f64, z: f64) -> Value {
    json!({"x": x, "y": y, "z": z})
}

fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    let cmd = AnyCommand::from_tool(tool, &args).unwrap_or_else(|m| panic!("{tool}: {m}"));
    s.execute(cmd, &ExecContext::user())
}

fn run(s: &mut Session, tool: &str, args: Value) -> Outcome {
    try_run(s, tool, args.clone()).unwrap_or_else(|f| panic!("{tool} {args}: {f:?}"))
}

fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).expect("element")
}

/// Wall along X from (0, 0) to (4000, 0): 200 thick (y −100 … 100), 3000
/// high.
fn wall(s: &mut Session) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": p(0.0, 0.0, 0.0), "end": p(4000.0, 0.0, 0.0)}),
    ))
}

/// Axis-aligned box from `lo` to `hi`.
fn cuboid(s: &mut Session, lo: [f64; 3], hi: [f64; 3]) -> ElementId {
    id(&run(
        s,
        "create_box",
        json!({"origin": p(lo[0], lo[1], lo[2]), "width": hi[0] - lo[0], "depth": hi[1] - lo[1], "height": hi[2] - lo[2]}),
    ))
}

fn cut(s: &mut Session, cutter: ElementId, target: ElementId) -> Outcome {
    run(
        s,
        "create_relationship",
        json!({"kind": "cut", "elements": [cutter, target]}),
    )
}

fn volume(s: &Session, e: ElementId) -> f64 {
    match s.doc().get(e).and_then(|x| x.param(keys::VOLUME)) {
        Some(ParamValue::Volume(v)) => *v,
        other => panic!("{e}: no volume ({other:?})"),
    }
}

fn geometry_volume(s: &Session, e: ElementId) -> f64 {
    let g = s.doc().get(e).and_then(|x| x.geometry()).expect("geometry");
    g.validate_solid().expect("a valid solid");
    g.volume()
}

fn assert_volume(got: f64, want: f64, what: &str) {
    assert!(
        ((got - want) / want).abs() < 1e-9,
        "{what}: volume {got}, expected {want}"
    );
}

const WALL: f64 = 4000.0 * 200.0 * 3000.0;
/// A 1200 × 1200 window through the 200 mm wall.
const WINDOW: f64 = 1200.0 * 200.0 * 1200.0;

#[test]
fn a_wall_cut_by_a_box_loses_the_exact_volume() {
    let mut s = session();
    let w = wall(&mut s);
    let c = cuboid(&mut s, [1000.0, -300.0, 900.0], [2200.0, 300.0, 2100.0]);
    let o = cut(&mut s, c, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(volume(&s, w), WALL - WINDOW, "window");
    assert_volume(geometry_volume(&s, w), WALL - WINDOW, "geometry");
    // The cut wall keeps its six faces' ids and gains the four reveals.
    assert_eq!(s.doc().get(w).unwrap().geometry().unwrap().face_count(), 10);
}

#[test]
fn moving_the_cutter_regenerates_and_deleting_the_row_restores() {
    let mut s = session();
    let w = wall(&mut s);
    let c = cuboid(&mut s, [1000.0, -300.0, 900.0], [2200.0, 300.0, 2100.0]);
    let row = cut(&mut s, c, w).output["relationship"]
        .as_str()
        .unwrap()
        .to_string();
    let faces: Vec<_> = s
        .doc()
        .get(w)
        .unwrap()
        .geometry()
        .unwrap()
        .face_ids()
        .collect();
    // Half the window moves past the wall's end.
    run(
        &mut s,
        "move_elements",
        json!({"elements": [c], "vector": p(2400.0, 0.0, 0.0)}),
    );
    assert_volume(volume(&s, w), WALL - WINDOW / 2.0, "notch at the end");
    run(
        &mut s,
        "move_elements",
        json!({"elements": [c], "vector": p(-2400.0, 0.0, 0.0)}),
    );
    assert_volume(volume(&s, w), WALL - WINDOW, "back");
    // Same cut again: the same face ids (lineage).
    let again: Vec<_> = s
        .doc()
        .get(w)
        .unwrap()
        .geometry()
        .unwrap()
        .face_ids()
        .collect();
    assert_eq!(faces, again);
    run(&mut s, "delete_relationship", json!({"relationship": row}));
    assert_volume(volume(&s, w), WALL, "restored");
    s.undo().unwrap();
    assert_volume(volume(&s, w), WALL - WINDOW, "undo");
    s.redo().unwrap();
    assert_volume(volume(&s, w), WALL, "redo");
}

#[test]
fn two_cuts_apply_in_uid_order() {
    let mut s = session();
    let w = wall(&mut s);
    let a = cuboid(&mut s, [500.0, -300.0, 900.0], [1700.0, 300.0, 2100.0]);
    let b = cuboid(&mut s, [2300.0, -300.0, 0.0], [3200.0, 300.0, 2100.0]);
    cut(&mut s, a, w);
    let o = cut(&mut s, b, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(
        volume(&s, w),
        WALL - WINDOW - 900.0 * 200.0 * 2100.0,
        "window and door",
    );
}

#[test]
fn flush_and_tolerance_edge_cutters() {
    // Through the wall by `d` on both sides (0 = flush): always the window.
    for d in [0.0, 0.005, 0.01, 0.02] {
        let mut s = session();
        let w = wall(&mut s);
        let c = cuboid(
            &mut s,
            [1000.0, -100.0 - d, 900.0],
            [2200.0, 100.0 + d, 2100.0],
        );
        let o = cut(&mut s, c, w);
        assert!(o.warnings.is_empty(), "overlap {d}: {:?}", o.warnings);
        assert_volume(volume(&s, w), WALL - WINDOW, &format!("overlap {d}"));
    }
    // Stopping `d` short of the far face: within VERTEX_TOL it goes
    // through, beyond it a skin of `d` remains.
    for (d, removed) in [(0.005, WINDOW), (0.02, 1200.0 * 1200.0 * (200.0 - 0.02))] {
        let mut s = session();
        let w = wall(&mut s);
        let c = cuboid(&mut s, [1000.0, -300.0, 900.0], [2200.0, 100.0 - d, 2100.0]);
        let o = cut(&mut s, c, w);
        assert!(o.warnings.is_empty(), "skin {d}: {:?}", o.warnings);
        assert_volume(volume(&s, w), WALL - removed, &format!("skin {d}"));
    }
}

#[test]
fn rotated_and_far_from_the_origin() {
    let mut s = session();
    let w = wall(&mut s);
    let c = cuboid(&mut s, [1000.0, -300.0, 900.0], [2200.0, 300.0, 2100.0]);
    cut(&mut s, c, w);
    run(
        &mut s,
        "rotate_elements",
        json!({"elements": [w, c], "center": p(0.0, 0.0, 0.0), "angle": std::f64::consts::PI / 6.0}),
    );
    assert_volume(volume(&s, w), WALL - WINDOW, "rotated 30°");
    let o = run(
        &mut s,
        "move_elements",
        json!({"elements": [w, c], "vector": p(5.0e6, 0.0, 0.0)}),
    );
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(volume(&s, w), WALL - WINDOW, "rotated and 5 km away");
}

#[test]
fn a_cutter_that_removes_everything_keeps_the_wall_with_a_warning() {
    let mut s = session();
    let w = wall(&mut s);
    let c = cuboid(&mut s, [-100.0, -300.0, -100.0], [4100.0, 300.0, 3100.0]);
    let o = cut(&mut s, c, w);
    assert_eq!(o.warnings.len(), 1);
    assert_eq!(o.warnings[0].code, "ferro.kernel.empty_result");
    assert_volume(volume(&s, w), WALL, "uncut");
}

#[test]
fn a_phased_cut_counts_the_removed_part_as_demolished() {
    let mut s = session();
    let p1 = id(&run(&mut s, "create_phase", json!({"name": "Existing"})));
    let p2 = id(&run(
        &mut s,
        "create_phase",
        json!({"name": "New Construction"}),
    ));
    let w = wall(&mut s);
    let c = cuboid(&mut s, [1000.0, -300.0, 900.0], [2200.0, 300.0, 2100.0]);
    run(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [c, w], "phase": p2}),
    );
    let k = SolidcoreKernel::new();
    let cat = s.doc().get(w).unwrap().category;
    let q1 = PhaseQuantities::from_document_with_kernel(s.doc(), p1, Some(&k)).unwrap();
    assert_volume(
        q1.categories[&cat][&PhaseStatus::Existing].volume_mm3,
        WALL,
        "phase 1",
    );
    let q2 = PhaseQuantities::from_document_with_kernel(s.doc(), p2, Some(&k)).unwrap();
    assert_volume(
        q2.categories[&cat][&PhaseStatus::Existing].volume_mm3,
        WALL - WINDOW,
        "phase 2 existing",
    );
    assert_volume(
        q2.categories[&cat][&PhaseStatus::Demolished].volume_mm3,
        WINDOW,
        "phase 2 demolished",
    );
    // The removed part, for demolished graphics.
    let doc = s.doc();
    let bodies = ferro_core::model::generate::uncut_bodies(doc);
    let uncut = bodies[&w].clone();
    let removed = ferro_core::model::cut::removed_in_phase(doc, Some(&k), w, p2, &uncut, &|e| {
        bodies.get(&e).cloned()
    })
    .expect("a removed part");
    assert_volume(removed.volume(), WINDOW, "removed part");
    assert!(
        ferro_core::model::cut::removed_in_phase(doc, Some(&k), w, p1, &uncut, &|e| bodies
            .get(&e)
            .cloned())
        .is_none()
    );
}

#[test]
fn unite_and_intersect_overlapping_boxes() {
    let mut s = session();
    let a = cuboid(&mut s, [0.0, 0.0, 0.0], [1000.0, 1000.0, 1000.0]);
    let b = cuboid(&mut s, [500.0, 0.0, 0.0], [1500.0, 1000.0, 1000.0]);
    run(&mut s, "unite_forms", json!({"target": a, "other": b}));
    assert!(s.doc().get(b).is_none());
    assert_volume(geometry_volume(&s, a), 1.5e9, "union");
    let c = cuboid(&mut s, [1000.0, 0.0, 0.0], [2000.0, 1000.0, 1000.0]);
    run(
        &mut s,
        "intersect_forms",
        json!({"target": a, "other": c, "keep_other": true}),
    );
    assert!(s.doc().get(c).is_some());
    assert_volume(geometry_volume(&s, a), 0.5e9, "intersection");
}
