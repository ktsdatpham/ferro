//! Joins (M5-5.2) through the transaction: wall-end join states, butt
//! order from `join` rows, geometry joins through a stand-in kernel
//! ([`ClipKernel`]), freezing, undo / redo, the change format and pins.
//! Exact booleans of real overlaps are tested with solidcore in
//! `ferro-kernel/tests/join.rs`.

use super::cut_tests::ClipKernel;
use super::scenario_common_tests::*;
use super::*;
use crate::math::Point3;
use crate::model::{change, join, keys, ElementId, ParamValue, RelationshipKind};
use serde_json::{json, Value};
use std::sync::Arc;

/// Volume of a 200 × 3000 wall section per mm of length.
const SECTION: f64 = 200.0 * 3000.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

fn volume(s: &Session, e: ElementId) -> f64 {
    match s.doc().get(e).and_then(|x| x.param(keys::VOLUME)) {
        Some(ParamValue::Volume(v)) => *v,
        other => panic!("{e} has no volume: {other:?}"),
    }
}

fn codes(o: &Outcome) -> Vec<&str> {
    o.warnings.iter().map(|w| w.code.as_str()).collect()
}

fn wall_at(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": p(a.0, a.1, 0.0), "end": p(b.0, b.1, 0.0)}),
    ))
}

fn join_row(s: &mut Session, first: ElementId, second: ElementId) -> String {
    let o = run(
        s,
        "create_relationship",
        json!({"kind": "join", "elements": [first, second]}),
    );
    o.output["relationship"].as_str().unwrap().to_string()
}

/// Plan extent of an element along X and Y: (min, max).
fn extent(s: &Session, e: ElementId) -> (Point3, Point3) {
    let b = geometry(s, e).bounding_box().unwrap();
    (b.min, b.max)
}

/// An L corner at the origin: `a` (-4000, 0) → (0, 0), `b` (0, 0) → (0, 4000).
fn corner(s: &mut Session) -> (ElementId, ElementId) {
    let a = wall_at(s, (-4000.0, 0.0), (0.0, 0.0));
    let b = wall_at(s, (0.0, 0.0), (0.0, 4000.0));
    (a, b)
}

#[test]
fn l_t_and_x_corners_keep_the_inferred_volumes() {
    let mut s = session();
    // L: both mitred, each exactly its centre-line length.
    let (a, b) = corner(&mut s);
    assert!(close(volume(&s, a), 4000.0 * SECTION));
    assert!(close(volume(&s, b), 4000.0 * SECTION));
    // T: the stem stops at the face (100 mm short of the centre line).
    let mut s = session();
    let bar = wall_at(&mut s, (-4000.0, 0.0), (4000.0, 0.0));
    let stem = wall_at(&mut s, (0.0, 0.0), (0.0, 3000.0));
    assert!(close(volume(&s, bar), 8000.0 * SECTION));
    assert!(close(volume(&s, stem), 2900.0 * SECTION));
    // X (four ends at one point): every end stays square.
    let mut s = session();
    let ws = [
        wall_at(&mut s, (0.0, 0.0), (3000.0, 0.0)),
        wall_at(&mut s, (0.0, 0.0), (-3000.0, 0.0)),
        wall_at(&mut s, (0.0, 0.0), (0.0, 3000.0)),
        wall_at(&mut s, (0.0, 0.0), (0.0, -3000.0)),
    ];
    for w in ws {
        assert!(close(volume(&s, w), 3000.0 * SECTION));
    }
}

#[test]
fn a_disallowed_end_stays_square_and_allow_restores_the_mitre() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let mitred = (geometry(&s, a), geometry(&s, b));
    run(
        &mut s,
        "set_wall",
        json!({"wall": b, "join_start": "disallow"}),
    );
    // Both ends square: each wall exactly its length (no join at all).
    assert!(close(volume(&s, a), 4000.0 * SECTION));
    assert!(close(volume(&s, b), 4000.0 * SECTION));
    let (lo, hi) = extent(&s, a);
    assert!(close(hi.x, 0.0) && close(lo.x, -4000.0), "a is square");
    let (lo, _) = extent(&s, b);
    assert!(close(lo.y, 0.0), "b is square");
    assert_eq!(
        s.doc()
            .get(b)
            .and_then(|e| e.as_wall())
            .map(|w| w.join_start),
        Some(crate::model::bim::WallJoin::Disallow)
    );
    // Moving a wall into contact keeps the disallowed end square.
    run(
        &mut s,
        "set_wall",
        json!({"wall": b, "start": p(0.0, 0.0, 0.0), "end": p(0.0, 5000.0, 0.0)}),
    );
    assert!(close(volume(&s, b), 5000.0 * SECTION));
    run(
        &mut s,
        "set_wall",
        json!({"wall": b, "start": p(0.0, 0.0, 0.0), "end": p(0.0, 4000.0, 0.0), "join_start": "auto"}),
    );
    assert_eq!((geometry(&s, a), geometry(&s, b)), mitred);
}

#[test]
fn a_join_row_between_corner_walls_makes_a_butt_and_switching_swaps_it() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let uid = join_row(&mut s, a, b);
    // `a` (first) runs through to b's far face; `b` stops at a's face.
    assert!(close(volume(&s, a), 4100.0 * SECTION));
    assert!(close(volume(&s, b), 3900.0 * SECTION));
    assert!(close(extent(&s, a).1.x, 100.0));
    assert!(close(extent(&s, b).0.y, 100.0));
    // Switch join order: same row, other winner.
    let o = run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    assert_eq!(o.output["relationship"], uid);
    assert!(close(volume(&s, a), 3900.0 * SECTION));
    assert!(close(volume(&s, b), 4100.0 * SECTION));
    assert!(close(extent(&s, b).0.y, -100.0));
    // Unjoin: back to the mitre.
    run(&mut s, "delete_relationship", json!({"relationship": uid}));
    assert!(close(volume(&s, a), 4000.0 * SECTION));
    assert!(close(volume(&s, b), 4000.0 * SECTION));
}

#[test]
fn a_t_stem_put_first_runs_through_and_needs_the_kernel() {
    let mut s = session();
    let bar = wall_at(&mut s, (-4000.0, 0.0), (4000.0, 0.0));
    let stem = wall_at(&mut s, (0.0, 0.0), (0.0, 3000.0));
    // Bar first: the default T, settled by the ends (no kernel needed).
    let uid = join_row(&mut s, bar, stem);
    assert!(close(volume(&s, stem), 2900.0 * SECTION));
    // Stem first: it reaches the bar's far face; without a kernel the bar
    // keeps its body and regeneration warns.
    let o = run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    assert!(close(volume(&s, stem), 3100.0 * SECTION));
    assert!(close(volume(&s, bar), 8000.0 * SECTION));
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
}

/// A slab whose top is at 3100 and bottom at 2900, over the whole wall
/// (0, 0) → (4000, 0).
fn slab(s: &mut Session) -> ElementId {
    id(&run(
        s,
        "create_floor",
        json!({"points": [{"x": -500, "y": -1000}, {"x": 4500, "y": -1000}, {"x": 4500, "y": 1000}, {"x": -500, "y": 1000}], "offset": 3100}),
    ))
}

#[test]
fn a_floor_joined_first_trims_the_wall_and_keeps_its_own_body() {
    let mut s = session().with_kernel(Arc::new(ClipKernel));
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s);
    let floor_body = geometry(&s, f);
    let uid = join_row(&mut s, f, w);
    assert!(
        close(volume(&s, w), 4000.0 * 200.0 * 2900.0),
        "wall − floor"
    );
    assert_eq!(
        geometry(&s, f),
        floor_body,
        "nothing removed from the first"
    );
    // Undo / redo.
    let joined = geometry(&s, w);
    s.undo().unwrap();
    assert!(close(volume(&s, w), 4000.0 * SECTION));
    s.redo().unwrap();
    assert_eq!(geometry(&s, w), joined);
    // Moving the floor regenerates the join.
    run(&mut s, "set_floor", json!({"floor": f, "offset": 3200}));
    assert!(close(volume(&s, w), 4000.0 * 200.0 * 3000.0), "now apart");
    run(&mut s, "set_floor", json!({"floor": f, "offset": 3100}));
    // Switch order: floor − wall is not a top clip → the stand-in kernel
    // refuses: the row is skipped with its code, never an error.
    let o = run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    assert_eq!(codes(&o), ["ferro.kernel.unsupported"]);
    assert!(close(volume(&s, w), 4000.0 * SECTION));
    assert_eq!(geometry(&s, f), floor_body);
    // Unjoin restores.
    run(&mut s, "delete_relationship", json!({"relationship": uid}));
    assert!(close(volume(&s, w), 4000.0 * SECTION));
    assert!(o.warnings.iter().all(|w| w.severity == Severity::Warning));
}

#[test]
fn without_a_kernel_a_geometry_join_warns_and_keeps_the_body() {
    let mut s = session();
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s);
    let o = run(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [f, w]}),
    );
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
    assert!(close(volume(&s, w), 4000.0 * SECTION));
}

#[test]
fn forms_join_through_a_derived_body() {
    let mut s = session().with_kernel(Arc::new(ClipKernel));
    let low = boxed(&mut s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let lid = boxed(&mut s, [-100.0, -100.0, 800.0], 1200.0, 1200.0, 400.0);
    join_row(&mut s, lid, low);
    let e = s.doc().get(low).unwrap();
    let f = e.as_form().unwrap();
    assert!(close(f.body.volume(), 1e9), "the drawn body stays");
    assert!(close(e.geometry().unwrap().volume(), 0.8e9), "joined");
    assert!(s
        .doc()
        .get(lid)
        .unwrap()
        .as_form()
        .unwrap()
        .cut_body
        .is_none());
    // A first Form is never a void (only cut rows make voids).
    assert!(!crate::model::cut::is_void(
        s.doc(),
        s.doc().get(lid).unwrap()
    ));
}

#[test]
fn join_rows_refuse_openings_and_duplicates() {
    let mut s = session();
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 2000}),
    ));
    let bx = boxed(&mut s, [0.0, 2000.0, 0.0], 500.0, 500.0, 500.0);
    expect_code(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [d, bx]}),
        "ferro.model.invalid_relationship",
    );
    let uid = join_row(&mut s, w, bx);
    expect_code(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [bx, w]}),
        "ferro.model.duplicate_relationship",
    );
    // set_relationship refusals.
    expect_code(
        &mut s,
        "set_relationship",
        json!({"relationship": "nope", "reverse": true}),
        "ferro.model.invalid_uid",
    );
    expect_code(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "elements": [w, w]}),
        "ferro.model.invalid_relationship",
    );
    expect_code(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "elements": [w, bx], "reverse": true}),
        "ferro.model.invalid_relationship",
    );
    expect_code(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "elements": [w, d]}),
        "ferro.model.invalid_relationship",
    );
}

#[test]
fn freezing_a_corner_wall_keeps_its_shape_and_makes_the_join_explicit() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let mitred = geometry(&s, a);
    let o = run(
        &mut s,
        "set_geometry_source",
        json!({"element": a, "source": "direct"}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!(e.as_form().map(|f| &f.body), Some(&mitred), "frozen mitred");
    let row = join::join_between(s.doc(), a, b).expect("explicit join row");
    assert_eq!(row.elements, vec![a, b], "the frozen wall wins");
    // `b` reaches the far face of the Form; the kernel would trim it back to
    // the mitre (ferro-kernel tests), here it warns once.
    assert!(close(extent(&s, b).0.y, -100.0));
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
    // Undo restores the walls and removes the row in one step.
    s.undo().unwrap();
    assert!(join::join_between(s.doc(), a, b).is_none());
    assert!(close(volume(&s, b), 4000.0 * SECTION));
}

#[test]
fn freezing_keeps_explicit_rows_and_a_geometry_join_on_the_form() {
    let mut s = session().with_kernel(Arc::new(ClipKernel));
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s);
    let uid = join_row(&mut s, f, w);
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    let r = s.doc().relationships().find(|r| r.uid.to_string() == uid);
    assert_eq!(r.map(|r| r.elements.clone()), Some(vec![f, w]));
    let e = s.doc().get(w).unwrap();
    let form = e.as_form().unwrap();
    assert!(
        close(form.body.volume(), 4000.0 * SECTION),
        "frozen unjoined"
    );
    assert!(close(
        e.geometry().unwrap().volume(),
        4000.0 * 200.0 * 2900.0
    ));
}

#[test]
fn pinned_elements_may_be_joined_and_unjoined() {
    let mut s = session().with_kernel(Arc::new(ClipKernel));
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s);
    run(
        &mut s,
        "set_pinned",
        json!({"elements": [w, f], "pinned": true}),
    );
    let uid = join_row(&mut s, f, w);
    run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    run(
        &mut s,
        "set_wall",
        json!({"wall": w, "join_end": "disallow"}),
    );
    run(&mut s, "delete_relationship", json!({"relationship": uid}));
}

#[test]
fn ends_within_the_join_tolerance_still_mitre() {
    for gap in [0.005, 0.01, 0.02] {
        let mut s = session();
        let a = wall_at(&mut s, (-4000.0, 0.0), (0.0, 0.0));
        let b = wall_at(&mut s, (gap, 0.0), (gap, 4000.0));
        assert!(
            (volume(&s, a) - 4000.0 * SECTION).abs() < gap * 200.0 * 3000.0 + 1.0,
            "gap {gap}"
        );
        let uid = join_row(&mut s, a, b);
        assert!((volume(&s, a) - 4100.0 * SECTION).abs() < 2.0 * gap * SECTION);
        run(
            &mut s,
            "set_wall",
            json!({"wall": b, "join_start": "disallow"}),
        );
        assert!(close(volume(&s, a), 4000.0 * SECTION), "gap {gap}: square");
        let _ = uid;
    }
}

#[test]
fn rotated_and_far_from_the_origin() {
    let (c, sn) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let o = (5_000_000.0, 3_000_000.0);
    let at = |x: f64, y: f64| (o.0 + c * x - sn * y, o.1 + sn * x + c * y);
    let mut s = session();
    let a = wall_at(&mut s, at(-4000.0, 0.0), at(0.0, 0.0));
    let b = wall_at(&mut s, at(0.0, 0.0), at(0.0, 4000.0));
    let tol = 1e-6 * 4000.0 * SECTION;
    assert!((volume(&s, a) - 4000.0 * SECTION).abs() < tol);
    join_row(&mut s, b, a);
    assert!((volume(&s, a) - 3900.0 * SECTION).abs() < tol);
    assert!((volume(&s, b) - 4100.0 * SECTION).abs() < tol);
    run(
        &mut s,
        "set_wall",
        json!({"wall": a, "join_end": "disallow"}),
    );
    assert!((volume(&s, a) - 4000.0 * SECTION).abs() < tol);
    assert!((volume(&s, b) - 4000.0 * SECTION).abs() < tol);
}

#[test]
fn join_state_and_order_round_trip_through_the_change_format() {
    let mut s = session();
    let (a, b) = corner(&mut s);
    let before = s.doc().clone();
    run(
        &mut s,
        "set_wall",
        json!({"wall": a, "join_end": "disallow"}),
    );
    let uid = join_row(&mut s, a, b);
    run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    let after = s.doc().clone();
    let changes = change::diff(&before, &after).unwrap();
    let text = serde_json::to_string(&changes).unwrap();
    assert!(text.contains("disallow"), "{text}");
    let applied = change::apply(&before, &changes).unwrap();
    let w = applied.get(a).and_then(|e| e.as_wall()).unwrap();
    assert_eq!(w.join_end, crate::model::bim::WallJoin::Disallow);
    let r = applied
        .relationships()
        .find(|r| r.kind == RelationshipKind::Join)
        .unwrap();
    assert_eq!(r.elements, vec![b, a]);
    // A wall with both ends `auto` serialises as before (no new keys).
    let v: Value = serde_json::to_value(before.get(b).unwrap()).unwrap();
    assert!(v.to_string().find("join_").is_none(), "{v}");
    // Unknown states are refused by the tool schema path.
    assert!(AnyCommand::from_tool("set_wall", &json!({"wall": a, "join_end": "maybe"})).is_err());
}

/// [`ClipKernel`] counting its calls.
struct Counting(std::sync::atomic::AtomicUsize);

impl crate::algo::Kernel for Counting {
    fn name(&self) -> &'static str {
        "counting-join-test"
    }
    fn boolean(
        &self,
        op: crate::algo::BooleanOp,
        a: &crate::topo::Body,
        b: &crate::topo::Body,
    ) -> Result<crate::algo::KernelResult, crate::algo::KernelError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::algo::Kernel::boolean(&ClipKernel, op, a, b)
    }
}

#[test]
fn join_rows_go_through_the_session_cut_cache() {
    use std::sync::atomic::Ordering::SeqCst;
    let k = Arc::new(Counting(Default::default()));
    let mut s = session().with_kernel(k.clone());
    let w = wall_at(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let other = wall_at(&mut s, (0.0, 9000.0), (4000.0, 9000.0));
    let f = slab(&mut s);
    let uid = join_row(&mut s, f, w);
    assert_eq!(k.0.load(SeqCst), 1, "computed once");
    // An unrelated wall edit regenerates every wall: no kernel call.
    run(
        &mut s,
        "set_wall_height",
        json!({"wall": other, "height": 3500}),
    );
    assert_eq!(k.0.load(SeqCst), 1, "cached");
    // Switch order (same uid): never the old result.
    run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    assert_eq!(k.0.load(SeqCst), 2, "reversed row recomputed");
    assert!(close(volume(&s, w), 4000.0 * SECTION));
    run(
        &mut s,
        "set_relationship",
        json!({"relationship": uid, "reverse": true}),
    );
    assert_eq!(k.0.load(SeqCst), 2, "back: seen before");
    assert!(close(volume(&s, w), 4000.0 * 200.0 * 2900.0));
    // Stored geometry equals an uncached regeneration.
    let fresh = crate::model::generate::regenerate_with(s.doc(), Some(&ClipKernel));
    assert!(fresh.updates.is_empty() && fresh.cut_updates.is_empty());
}
