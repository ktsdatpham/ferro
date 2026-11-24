//! Kernel injection (ADR 0007, M5-16): sessions without a kernel refuse
//! kernel commands with `ferro.kernel.unavailable`; with one, the command
//! reaches it through the transaction. The real kernel's geometry is tested
//! in `ferro-kernel`; here a stand-in answers operands that are apart.

use super::scenario_common_tests::*;
use super::*;
use crate::model::ElementId;
use serde_json::json;

fn two_boxes(s: &mut Session) -> (ElementId, ElementId) {
    let a = boxed(s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let b = boxed(s, [5000.0, 0.0, 0.0], 500.0, 500.0, 500.0);
    (a, b)
}

#[test]
fn subtract_forms_without_a_kernel_is_unavailable_and_rolls_back() {
    let mut s = session();
    let (a, b) = two_boxes(&mut s);
    let before = snapshot(&s);
    let f = try_run(&mut s, "subtract_forms", json!({"target": a, "cutter": b})).unwrap_err();
    assert_eq!(f[0].code, "ferro.kernel.unavailable");
    assert_eq!(snapshot(&s), before);
    assert!(s.kernel().is_none());
}

#[test]
fn subtract_forms_reaches_the_kernel_and_deletes_the_cutter() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let faces: Vec<_> = geometry(&s, a).face_ids().collect();
    let o = run(&mut s, "subtract_forms", json!({"target": a, "cutter": b}));
    assert_eq!(o.output["cutter_deleted"], json!(true));
    assert!(s.doc().get(b).is_none());
    // Every face of the target survives with its id.
    assert_eq!(geometry(&s, a).face_ids().collect::<Vec<_>>(), faces);
    assert_eq!(
        o.output["lineage"]["kept"].as_array().map(Vec::len),
        Some(6)
    );
    s.undo().unwrap();
    assert!(s.doc().get(b).is_some());
}

#[test]
fn subtract_forms_can_keep_the_cutter() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let o = run(
        &mut s,
        "subtract_forms",
        json!({"target": a, "cutter": b, "keep_cutter": true}),
    );
    assert_eq!(o.output["cutter_deleted"], json!(false));
    assert!(s.doc().get(b).is_some());
}

#[test]
fn a_session_kernel_can_be_removed() {
    let mut s = kernel_session();
    assert!(s.kernel().is_some());
    s.set_kernel(None);
    let (a, b) = two_boxes(&mut s);
    let f = try_run(&mut s, "subtract_forms", json!({"target": a, "cutter": b})).unwrap_err();
    assert_eq!(f[0].code, "ferro.kernel.unavailable");
}

#[test]
fn editing_in_a_design_option_uses_the_session_kernel() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let o = run(&mut s, "create_option_set", json!({"name": "Cut"}));
    let option: ElementId = serde_json::from_value(o.output["option"].clone()).unwrap();
    let cmd = AnyCommand::from_tool("subtract_forms", &json!({"target": a, "cutter": b})).unwrap();
    s.execute_in_option(
        option,
        &crate::model::Combination::primary(),
        cmd,
        &ExecContext::user(),
    )
    .unwrap();
    // The shared model keeps the cutter; the option's layer removes it.
    assert!(s.doc().get(b).is_some());
}

#[test]
fn preview_uses_the_session_kernel() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let cmd = AnyCommand::from_tool("subtract_forms", &json!({"target": a, "cutter": b})).unwrap();
    let (doc, _) = s.preview(&cmd).unwrap();
    assert!(doc.get(b).is_none());
    assert!(s.doc().get(b).is_some());
}

#[test]
fn unite_and_intersect_without_a_kernel_are_unavailable() {
    let mut s = session();
    let (a, b) = two_boxes(&mut s);
    for tool in ["unite_forms", "intersect_forms"] {
        let f = try_run(&mut s, tool, json!({"target": a, "other": b})).unwrap_err();
        assert_eq!(f[0].code, "ferro.kernel.unavailable", "{tool}");
    }
}

#[test]
fn unite_forms_reaches_the_kernel_and_deletes_the_other() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let faces: Vec<_> = geometry(&s, a).face_ids().collect();
    let o = run(&mut s, "unite_forms", json!({"target": a, "other": b}));
    assert_eq!(o.output["other_deleted"], json!(true));
    assert!(s.doc().get(b).is_none());
    let g = geometry(&s, a);
    // Two shells; the target's faces keep their ids.
    assert_eq!(g.face_count(), 12);
    assert!(faces.iter().all(|f| g.face(*f).is_ok()));
    assert!((g.volume() - (1.0e9 + 1.25e8)).abs() < 1e-3);
    s.undo().unwrap();
    assert!(s.doc().get(b).is_some());
}

#[test]
fn intersect_forms_of_apart_forms_is_refused() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let before = snapshot(&s);
    let f = try_run(&mut s, "intersect_forms", json!({"target": a, "other": b})).unwrap_err();
    assert_eq!(f[0].code, "ferro.kernel.empty_result");
    assert_eq!(snapshot(&s), before);
    // A Form inside a box: the intersection is the inner Form, kept.
    let c = boxed(&mut s, [100.0, 100.0, 100.0], 200.0, 200.0, 200.0);
    let o = run(
        &mut s,
        "intersect_forms",
        json!({"target": a, "other": c, "keep_other": true}),
    );
    assert_eq!(o.output["other_deleted"], json!(false));
    assert!((geometry(&s, a).volume() - 8.0e6).abs() < 1e-3);
}
