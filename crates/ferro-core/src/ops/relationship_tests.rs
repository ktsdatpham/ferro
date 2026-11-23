//! The relationship table (ADR 0004 §5, M5-5.1).

use super::*;
use crate::model::change::{apply, diff};
use crate::model::{Document, ElementId, RelationshipKind, Uid};
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

fn ok(s: &mut Session, tool: &str, args: Value) -> Value {
    run(s, tool, args)
        .unwrap_or_else(|f| panic!("{tool}: {f:?}"))
        .output
}

fn code(s: &mut Session, tool: &str, args: Value) -> String {
    let before = s.doc().clone();
    let f = run(s, tool, args).expect_err("must fail");
    assert_eq!(s.doc(), &before, "a failure changes nothing");
    f[0].code.clone()
}

fn bx(s: &mut Session, x: f64) -> ElementId {
    let o = ok(
        s,
        "create_box",
        json!({"origin": {"x": x, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
    );
    serde_json::from_value(o["element"].clone()).expect("element")
}

fn uid(v: &Value) -> Uid {
    v["relationship"]
        .as_str()
        .expect("uid")
        .parse()
        .expect("uid")
}

/// Three boxes and a level.
fn fixture() -> (Session, [ElementId; 3], ElementId) {
    let mut s = session();
    let o = ok(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    );
    let level: ElementId = serde_json::from_value(o["element"].clone()).expect("level");
    let ids = [bx(&mut s, 0.0), bx(&mut s, 2000.0), bx(&mut s, 4000.0)];
    (s, ids, level)
}

#[test]
fn create_and_delete_are_undoable_rows() {
    let (mut s, [a, b, _], _) = fixture();
    let empty = s.doc().clone();
    let u = uid(&ok(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [a, b]}),
    ));
    let r = s.doc().relationship(u).expect("row").clone();
    assert_eq!(
        (r.kind, r.elements.as_slice()),
        (RelationshipKind::Join, &[a, b][..])
    );
    assert_eq!(s.doc().relationships_of(b).count(), 1);
    let with = s.doc().clone();
    s.undo().unwrap();
    assert_eq!(s.doc(), &empty);
    s.redo().unwrap();
    assert_eq!(s.doc(), &with);
    let out = ok(
        &mut s,
        "delete_relationship",
        json!({"relationship": u.to_string()}),
    );
    assert_eq!(out["elements"], json!([a, b]));
    assert!(s.doc().relationship(u).is_none());
    assert!(s.doc().contains(a) && s.doc().contains(b), "elements stay");
    s.undo().unwrap();
    assert_eq!(s.doc(), &with);
}

#[test]
fn bad_rows_are_refused_with_their_codes() {
    let (mut s, [a, b, c], level) = fixture();
    let mut cr = |k: &str, e: Value| {
        code(
            &mut s,
            "create_relationship",
            json!({"kind": k, "elements": e}),
        )
    };
    let invalid = "ferro.model.invalid_relationship";
    assert_eq!(cr("join", json!([a])), invalid, "one element");
    assert_eq!(
        cr("join", json!([a, b, c])),
        invalid,
        "three for a pair kind"
    );
    assert_eq!(
        cr("aggregate", json!([a])),
        invalid,
        "a whole without parts"
    );
    assert_eq!(cr("cut", json!([a, a])), invalid, "the same element twice");
    assert_eq!(
        cr("host", json!([level, a])),
        invalid,
        "a level has no geometry"
    );
    assert_eq!(cr("join", json!([a, 9999])), "ferro.model.missing_element");
    assert_eq!(cr("join", json!([])), invalid);
    // A phase that is not a phase.
    assert_eq!(
        code(
            &mut s,
            "create_relationship",
            json!({"kind": "cut", "elements": [a, b], "phase": c})
        ),
        invalid
    );
    // Unknown kind: an argument error, not a command.
    assert!(AnyCommand::from_tool(
        "create_relationship",
        &json!({"kind": "glue", "elements": [a, b]})
    )
    .is_err());
    // delete_relationship: not a uid / no such row.
    assert_eq!(
        code(
            &mut s,
            "delete_relationship",
            json!({"relationship": "nope"})
        ),
        "ferro.model.invalid_uid"
    );
    assert_eq!(
        code(
            &mut s,
            "delete_relationship",
            json!({"relationship": "01a00000-0000-7000-8000-000000000000"})
        ),
        "ferro.model.missing_relationship"
    );
}

#[test]
fn duplicates_follow_the_kind() {
    let (mut s, [a, b, c], _) = fixture();
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [a, b]}),
    );
    // The reversed join would disagree on who wins: refused.
    assert_eq!(
        code(
            &mut s,
            "create_relationship",
            json!({"kind": "join", "elements": [b, a]})
        ),
        "ferro.model.duplicate_relationship"
    );
    // Another kind on the same pair is fine, and the reverse cut is a
    // different row from the cut.
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [a, b]}),
    );
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [b, a]}),
    );
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "aggregate", "elements": [a, b, c]}),
    );
    assert_eq!(s.doc().relationships().count(), 4);
}

#[test]
fn deleting_an_element_removes_its_rows_in_the_same_step() {
    let (mut s, [a, b, c], _) = fixture();
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [a, b]}),
    );
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "connect", "elements": [b, c]}),
    );
    let keep = uid(&ok(
        &mut s,
        "create_relationship",
        json!({"kind": "connect", "elements": [a, c]}),
    ));
    let before = s.doc().clone();
    ok(&mut s, "delete_elements", json!({"elements": [b]}));
    let left: Vec<Uid> = s.doc().relationships().map(|r| r.uid).collect();
    assert_eq!(left, [keep]);
    s.undo().unwrap();
    assert_eq!(s.doc(), &before, "one undo brings element and rows back");
}

#[test]
fn a_phase_used_by_a_row_is_kept_or_merged() {
    let (mut s, [a, b, _], _) = fixture();
    let p1 = ok(&mut s, "create_phase", json!({"name": "Existing"}))["element"].clone();
    let p2 = ok(&mut s, "create_phase", json!({"name": "New"}))["element"].clone();
    let u = uid(&ok(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [a, b], "phase": p2}),
    ));
    assert_eq!(
        code(&mut s, "delete_phase", json!({"phase": p2})),
        "ferro.model.phase_in_use"
    );
    ok(
        &mut s,
        "delete_phase",
        json!({"phase": p2, "merge_into": p1}),
    );
    let p1: ElementId = serde_json::from_value(p1).unwrap();
    assert_eq!(s.doc().relationship(u).unwrap().phase, Some(p1));
}

#[test]
fn a_changed_row_reports_its_elements_as_modified() {
    let (mut s, [a, b, c], _) = fixture();
    let before = s.doc().clone();
    ok(
        &mut s,
        "create_relationship",
        json!({"kind": "join", "elements": [a, c]}),
    );
    let ch = DocumentChanged::between("x", &before, s.doc());
    assert_eq!(ch.modified, [a, c]);
    assert!(ch.added.is_empty() && ch.deleted.is_empty());
    assert!(!ch.modified.contains(&b));
}

#[test]
fn rows_travel_through_the_change_format_by_uid() {
    let (mut s, [a, b, c], _) = fixture();
    let base = s.doc().clone();
    let u = uid(&ok(
        &mut s,
        "create_relationship",
        json!({"kind": "aggregate", "elements": [c, a, b]}),
    ));
    let d = diff(&base, s.doc()).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path(), format!("/relationships/{u}"));
    let back = apply(&base, &d).unwrap();
    assert_eq!(back.relationship(u), s.doc().relationship(u));
    // A row pointing at an element the change list removes: refused.
    let gone = crate::model::change::Change::Remove {
        path: format!("/elements/{}", s.doc().get(a).unwrap().uid),
    };
    let e = apply(s.doc(), &[gone]).unwrap_err();
    assert_eq!(e.code, "ferro.change.dangling_reference");
    // No relationships at all: no relationship edits.
    assert!(diff(&base, &base).unwrap().is_empty());
}
