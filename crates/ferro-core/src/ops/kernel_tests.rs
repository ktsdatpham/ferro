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
fn preview_uses_the_session_kernel() {
    let mut s = kernel_session();
    let (a, b) = two_boxes(&mut s);
    let cmd = AnyCommand::from_tool("subtract_forms", &json!({"target": a, "cutter": b})).unwrap();
    let (doc, _) = s.preview(&cmd).unwrap();
    assert!(doc.get(b).is_none());
    assert!(s.doc().get(b).is_some());
}
