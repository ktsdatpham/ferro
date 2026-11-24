//! Non-destructive cuts (M5-5.3) and phased cuts (M5-23.6) through the
//! transaction, with a stand-in kernel ([`ClipKernel`]). The real kernel's
//! geometry (exact volumes of any overlap, D-06 tolerance edges, rotation,
//! far from the origin) is tested in `ferro-kernel/tests/cut.rs`.

use super::scenario_common_tests::*;
use super::*;
use crate::algo::{BooleanOp, FaceSource, Kernel, KernelError, KernelResult};
use crate::model::{cut, keys, ElementBody, ElementId, ParamValue};
use crate::topo::Body;
use serde_json::{json, Value};
use std::sync::Arc;

/// A stand-in kernel for differences of boxes: a cutter apart from `a`
/// leaves it as it is; a cutter covering `a`'s footprint and its top clips
/// the top down to the cutter's bottom (ids of `a` kept); a cutter covering
/// all of `a` gives `ferro.kernel.empty_result`; anything else is
/// `ferro.kernel.unsupported`.
pub(super) struct ClipKernel;

impl Kernel for ClipKernel {
    fn name(&self) -> &'static str {
        "clip-test"
    }
    fn boolean(&self, op: BooleanOp, a: &Body, b: &Body) -> Result<KernelResult, KernelError> {
        if op != BooleanOp::Difference {
            return Err(KernelError::new(
                KernelError::UNSUPPORTED,
                "the test kernel only subtracts",
            ));
        }
        for x in [a, b] {
            x.validate_solid()
                .map_err(|e| KernelError::new(KernelError::NOT_SOLID, e.to_string()))?;
        }
        let (Some(ba), Some(bb)) = (a.bounding_box(), b.bounding_box()) else {
            return Err(KernelError::new(KernelError::NOT_SOLID, "empty operand"));
        };
        let t = 1e-6;
        let result = |body: Body| KernelResult {
            faces: body
                .face_ids()
                .map(|f| (f, vec![FaceSource::FromA(f)]))
                .collect(),
            lineage: crate::topo::Lineage {
                kept: body.face_ids().collect(),
                ..Default::default()
            },
            body,
        };
        if !ba.intersects(bb) {
            return Ok(result(a.clone()));
        }
        let covers_plan = bb.min.x <= ba.min.x + t
            && bb.max.x >= ba.max.x - t
            && bb.min.y <= ba.min.y + t
            && bb.max.y >= ba.max.y - t
            && bb.max.z >= ba.max.z - t;
        if !covers_plan {
            return Err(KernelError::new(KernelError::UNSUPPORTED, "not a top clip"));
        }
        if bb.min.z <= ba.min.z + t {
            return Err(KernelError::new(
                KernelError::EMPTY_RESULT,
                "nothing is left",
            ));
        }
        let mut out = a.clone();
        let mut v = out.vertex_map();
        for x in v.values_mut() {
            if x.point.z > bb.min.z {
                x.point.z = bb.min.z;
            }
        }
        let mut specs = out.face_specs();
        for sp in &mut specs {
            sp.surface = None; // planes through the moved loops
        }
        out.rebuild(v, specs)
            .map_err(|e| KernelError::new(KernelError::FAILED, e.to_string()))?;
        Ok(result(out))
    }
}

fn clip_session() -> Session {
    session().with_kernel(Arc::new(ClipKernel))
}

/// A straight wall along X, 4000 long, 200 thick, 3000 high (default type).
fn wall(s: &mut Session, y: f64) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": p(0.0, y, 0.0), "end": p(4000.0, y, 0.0)}),
    ))
}

/// A box over the whole wall at `y` from height `z` up.
fn clipper(s: &mut Session, y: f64, z: f64) -> ElementId {
    boxed(s, [-500.0, y - 500.0, z], 5000.0, 1000.0, 4000.0)
}

fn cut_row(s: &mut Session, cutter: ElementId, target: ElementId) -> (String, Outcome) {
    let o = run(
        s,
        "create_relationship",
        json!({"kind": "cut", "elements": [cutter, target]}),
    );
    (o.output["relationship"].as_str().unwrap().to_string(), o)
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

const WALL_MM3: f64 = 4000.0 * 200.0 * 3000.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

#[test]
fn a_wall_cut_by_a_box_loses_exactly_the_box() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    let (_, o) = cut_row(&mut s, c, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert!(o.changed.modified.contains(&w));
    assert!(close(volume(&s, w), 4000.0 * 200.0 * 2000.0));
    assert!(close(geometry(&s, w).volume(), 4000.0 * 200.0 * 2000.0));
    // The cutter is a void: a Form that cuts.
    let ce = s.doc().get(c).unwrap();
    assert!(cut::is_void(s.doc(), ce));
    assert!(!cut::is_void(s.doc(), s.doc().get(w).unwrap()));
}

#[test]
fn moving_the_cutter_regenerates_and_deleting_the_row_restores() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    let (uid, _) = cut_row(&mut s, c, w);
    let o = run(
        &mut s,
        "move_elements",
        json!({"elements": [c], "vector": p(0.0, 0.0, -500.0)}),
    );
    assert!(o.changed.modified.contains(&w));
    assert!(close(volume(&s, w), 4000.0 * 200.0 * 1500.0));
    run(&mut s, "delete_relationship", json!({"relationship": uid}));
    assert!(close(volume(&s, w), WALL_MM3));
    assert!(!cut::is_void(s.doc(), s.doc().get(c).unwrap()));
}

#[test]
fn deleting_the_cutter_restores_the_target() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    cut_row(&mut s, c, w);
    run(&mut s, "delete_elements", json!({"elements": [c]}));
    assert_eq!(s.doc().relationships().count(), 0);
    assert!(close(volume(&s, w), WALL_MM3));
}

#[test]
fn two_cuts_apply_one_after_the_other() {
    let mut s = clip_session();
    // A box Form 4000 x 200 x 3000 (the stand-in kernel clips boxes).
    let w = boxed(&mut s, [0.0, -100.0, 0.0], 4000.0, 200.0, 3000.0);
    let hi = clipper(&mut s, 0.0, 2500.0);
    let lo = clipper(&mut s, 0.0, 1000.0);
    cut_row(&mut s, hi, w);
    let (_, o) = cut_row(&mut s, lo, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_eq!(cut::cuts_of(s.doc(), w).len(), 2);
    assert!(close(geometry(&s, w).volume(), 4000.0 * 200.0 * 1000.0));
    // Removing the lower cut leaves the upper one.
    let lower = cut::cuts_of(s.doc(), w)[1].uid.to_string();
    run(
        &mut s,
        "delete_relationship",
        json!({"relationship": lower}),
    );
    assert!(close(geometry(&s, w).volume(), 4000.0 * 200.0 * 2500.0));
}

#[test]
fn without_a_kernel_the_body_stays_uncut_with_one_warning() {
    let mut s = session();
    let w1 = wall(&mut s, 0.0);
    let w2 = wall(&mut s, 3000.0);
    let c1 = clipper(&mut s, 0.0, 2000.0);
    let c2 = clipper(&mut s, 3000.0, 2000.0);
    let (_, o) = cut_row(&mut s, c1, w1);
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
    assert!(close(volume(&s, w1), WALL_MM3));
    let (_, o) = cut_row(&mut s, c2, w2);
    // Two cut elements, still one warning per regeneration.
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
    assert!(close(volume(&s, w2), WALL_MM3));
}

#[test]
fn a_kernel_error_keeps_the_uncut_body_with_its_code() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    // Covers the whole wall: nothing would be left.
    let c = clipper(&mut s, 0.0, -100.0);
    let (_, o) = cut_row(&mut s, c, w);
    assert_eq!(codes(&o), ["ferro.kernel.empty_result"]);
    assert!(close(volume(&s, w), WALL_MM3));
    // A cutter that is not a solid (an open Form) is skipped the same way.
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let r = id(&run(
        &mut s,
        "create_rectangle",
        json!({"origin": p(0.0, -1000.0, 2000.0), "width": 1000, "depth": 1000}),
    ));
    let (_, o) = cut_row(&mut s, r, w);
    assert_eq!(codes(&o), ["ferro.kernel.not_solid"]);
    assert!(close(volume(&s, w), WALL_MM3));
}

#[test]
fn editing_a_cut_wall_without_a_kernel_warns_and_commits() {
    let mut s = session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    cut_row(&mut s, c, w);
    let o = run(
        &mut s,
        "move_elements",
        json!({"elements": [w], "vector": p(0.0, 10.0, 0.0)}),
    );
    assert_eq!(codes(&o), ["ferro.kernel.unavailable"]);
    assert!(close(volume(&s, w), WALL_MM3));
}

#[test]
fn undo_and_redo_restore_both_bodies() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    cut_row(&mut s, c, w);
    let cut_body = geometry(&s, w);
    s.undo().unwrap();
    assert!(close(volume(&s, w), WALL_MM3));
    s.redo().unwrap();
    assert_eq!(geometry(&s, w), cut_body);
}

#[test]
fn a_form_target_keeps_its_source_and_its_face_ids() {
    let mut s = clip_session();
    let f = boxed(&mut s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let c = boxed(&mut s, [-100.0, -100.0, 600.0], 1200.0, 1200.0, 1000.0);
    let source = s.doc().get(f).unwrap().as_form().unwrap().body.clone();
    cut_row(&mut s, c, f);
    let form = s.doc().get(f).unwrap().as_form().unwrap().clone();
    assert_eq!(form.body, source, "the source body stays as drawn");
    let cut = form.cut_body.clone().expect("a cut body");
    assert!(close(cut.volume(), 1000.0 * 1000.0 * 600.0));
    assert_eq!(geometry(&s, f), cut);
    assert_eq!(
        cut.face_ids().collect::<Vec<_>>(),
        source.face_ids().collect::<Vec<_>>(),
        "surviving faces keep their ids"
    );
    match s.doc().get(f).unwrap().param(keys::FORM_VOLUME) {
        Some(ParamValue::Volume(v)) => assert!(close(*v, 6.0e8)),
        other => panic!("{other:?}"),
    }
    // Push / pull on a surviving face edits the source; the cut follows.
    let bottom = face_towards(&source, crate::math::Vec3::new(0.0, 0.0, -1.0));
    run(
        &mut s,
        "push_pull_face",
        json!({"element": f, "face": bottom, "distance": 400}),
    );
    let form = s.doc().get(f).unwrap().as_form().unwrap().clone();
    assert!(close(form.body.volume(), 1.4e9));
    assert!(close(form.cut_body.unwrap().volume(), 1.0e9));
}

#[test]
fn freezing_a_cut_wall_keeps_the_cut_non_destructive() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    let (uid, _) = cut_row(&mut s, c, w);
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    let form = s.doc().get(w).unwrap().as_form().unwrap().clone();
    assert!(close(form.body.volume(), WALL_MM3), "frozen uncut");
    assert!(close(
        form.cut_body.unwrap().volume(),
        4000.0 * 200.0 * 2000.0
    ));
    run(&mut s, "delete_relationship", json!({"relationship": uid}));
    let form = s.doc().get(w).unwrap().as_form().unwrap().clone();
    assert!(form.cut_body.is_none());
    assert!(close(volume(&s, w), WALL_MM3));
}

#[test]
fn openings_are_not_cut_targets_and_pinned_elements_can_be_cut() {
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let d = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": w, "kind": "door", "position": 2000}),
    ));
    let c = clipper(&mut s, 0.0, 2500.0);
    let f = try_run(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [c, d]}),
    )
    .unwrap_err();
    assert_eq!(f[0].code, "ferro.model.invalid_relationship");
    // An opening may cut (it has a body); a pinned wall may be cut (ADR
    // 0008: a pin blocks only move and delete).
    run(
        &mut s,
        "set_pinned",
        json!({"elements": [w], "pinned": true}),
    );
    let (_, o) = cut_row(&mut s, c, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert!(volume(&s, w) < WALL_MM3);
}

#[test]
fn the_change_format_carries_the_row_and_regenerates_on_apply() {
    use crate::model::change;
    let mut s = clip_session();
    let w = wall(&mut s, 0.0);
    let c = clipper(&mut s, 0.0, 2000.0);
    let before = s.doc().clone();
    let (uid, _) = cut_row(&mut s, c, w);
    let changes = change::diff(&before, s.doc()).unwrap();
    assert!(changes
        .iter()
        .any(|ch| ch.path() == format!("/relationships/{uid}")));
    // Without derived data (as an option layer stores it), apply_changes
    // regenerates the cut.
    let layer: Vec<Value> = changes
        .iter()
        .filter(|ch| !crate::model::design_option::is_derived_path(ch.path()))
        .filter(|ch| !ch.path().contains("/params/"))
        .map(|ch| serde_json::to_value(ch).unwrap())
        .collect();
    let mut t = clip_session();
    let w2 = wall(&mut t, 0.0);
    clipper(&mut t, 0.0, 2000.0);
    assert_eq!(w2, w);
    assert_eq!(t.doc().get(w).unwrap().uid, before.get(w).unwrap().uid);
    run(&mut t, "apply_changes", json!({"changes": layer}));
    assert_eq!(t.doc().relationships().count(), 1);
    assert!(close(volume(&t, w), 4000.0 * 200.0 * 2000.0));
}

// ------------------------------------------------------------- phases

/// Two phases ("Existing", "New Construction"); a wall without phase
/// fields (existing in every phase), cut in the second.
fn phased(s: &mut Session) -> (ElementId, ElementId, ElementId, ElementId) {
    let p1 = id(&run(s, "create_phase", json!({"name": "Existing"})));
    let p2 = id(&run(s, "create_phase", json!({"name": "New Construction"})));
    let w = wall(s, 0.0);
    let c = clipper(s, 0.0, 2000.0);
    run(
        s,
        "create_relationship",
        json!({"kind": "cut", "elements": [c, w], "phase": p2}),
    );
    (p1, p2, w, c)
}

#[test]
fn a_phased_cut_applies_from_its_phase_on() {
    let mut s = clip_session();
    let (p1, p2, w, _) = phased(&mut s);
    let doc = s.doc().clone();
    // The stored geometry is the latest state: cut.
    assert!(close(volume(&s, w), 4000.0 * 200.0 * 2000.0));
    assert!(cut::has_phased_cuts(&doc, w));
    assert!(cut::cuts_in_phase(&doc, w, p1).is_empty());
    assert_eq!(cut::cuts_in_phase(&doc, w, p2).len(), 1);
    let uncut = crate::model::generate::uncut_body(&doc, w).unwrap();
    assert!(close(uncut.volume(), WALL_MM3));
    let bodies = crate::model::generate::uncut_bodies(&doc);
    let cutter = |c: ElementId| bodies.get(&c).cloned();
    let k = ClipKernel;
    let before = cut::body_in_phase(&doc, Some(&k), w, p1, &uncut, &cutter);
    let after = cut::body_in_phase(&doc, Some(&k), w, p2, &uncut, &cutter);
    assert!(close(before.body.volume(), WALL_MM3));
    assert!(close(after.body.volume(), 4000.0 * 200.0 * 2000.0));
}

#[test]
fn phase_quantities_count_the_removed_part_as_demolished() {
    use crate::model::{PhaseQuantities, PhaseStatus};
    let mut s = clip_session();
    let (p1, p2, w, _) = phased(&mut s);
    let k = ClipKernel;
    let doc = s.doc().clone();
    let cat = doc.get(w).unwrap().category;
    let q1 = PhaseQuantities::from_document_with_kernel(&doc, p1, Some(&k)).unwrap();
    let e1 = q1.categories[&cat][&PhaseStatus::Existing];
    assert!(
        close(e1.volume_mm3, WALL_MM3),
        "uncut before the cut's phase"
    );
    assert!(!q1.categories[&cat].contains_key(&PhaseStatus::Demolished));
    let q2 = PhaseQuantities::from_document_with_kernel(&doc, p2, Some(&k)).unwrap();
    let e2 = q2.categories[&cat][&PhaseStatus::Existing];
    let d2 = q2.categories[&cat][&PhaseStatus::Demolished];
    assert!(close(e2.volume_mm3, 4000.0 * 200.0 * 2000.0));
    assert!(close(d2.volume_mm3, 4000.0 * 200.0 * 1000.0));
    assert_eq!(d2.count, 0, "a removed part is not an element");
    // A wall new in the cut's phase: the cut is not demolition.
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [w], "created": p2}),
    );
    let q = PhaseQuantities::from_document_with_kernel(s.doc(), p2, Some(&k)).unwrap();
    assert!(!q.categories[&cat].contains_key(&PhaseStatus::Demolished));
    assert!(close(
        q.categories[&cat][&PhaseStatus::New].volume_mm3,
        4000.0 * 200.0 * 2000.0
    ));
}

#[test]
fn phase_quantities_without_a_kernel_use_the_stored_body() {
    use crate::model::{PhaseQuantities, PhaseStatus};
    let mut s = clip_session();
    let (p1, _, w, _) = phased(&mut s);
    let doc = s.doc().clone();
    let cat = doc.get(w).unwrap().category;
    let q = PhaseQuantities::from_document_with_kernel(&doc, p1, None).unwrap();
    assert_eq!(q, PhaseQuantities::from_document(&doc, p1).unwrap());
    assert!(close(
        q.categories[&cat][&PhaseStatus::Existing].volume_mm3,
        4000.0 * 200.0 * 2000.0
    ));
}

#[test]
fn a_cut_body_survives_a_body_kind_check() {
    // `cut_body` is omitted from JSON when absent, so older files and
    // uncut Forms serialise as before.
    let mut s = clip_session();
    let f = boxed(&mut s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let v = serde_json::to_value(s.doc().get(f).unwrap()).unwrap();
    assert!(v["body"].get("cut_body").is_none());
    let ElementBody::Form(form) = &s.doc().get(f).unwrap().body else {
        panic!()
    };
    assert!(form.cut_body.is_none());
}
