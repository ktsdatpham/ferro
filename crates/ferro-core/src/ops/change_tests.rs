//! The change format (M5-26) on real edits made by commands.

use super::*;
use crate::model::change::{apply, conflicts, diff, element_value, Change};
use crate::model::{Document, Element};

use serde_json::{json, Value};

fn session() -> Session {
    Session::new(Document::new("T"), Box::new(SequentialIds::new(1)))
}

fn run(s: &mut Session, tool: &str, args: Value) -> Value {
    s.execute(
        AnyCommand::from_tool(tool, &args).expect("args"),
        &ExecContext::user(),
    )
    .unwrap_or_else(|f| panic!("{tool}: {f:?}"))
    .output
}

fn same_content(a: &Document, b: &Document) {
    assert_eq!(a.project(), b.project());
    let ea: Vec<&Element> = a.elements().collect();
    let eb: Vec<&Element> = b.elements().collect();
    assert_eq!(ea, eb);
}

fn house() -> Session {
    let mut s = session();
    run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    );
    run(
        &mut s,
        "create_walls",
        json!({"points": [{"x":0,"y":0,"z":0},{"x":6000,"y":0,"z":0},{"x":6000,"y":4000,"z":0}]}),
    );
    run(
        &mut s,
        "create_box",
        json!({"origin": {"x": 9000, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
    );
    s
}

#[test]
fn diff_then_apply_reproduces_every_kind_of_edit() {
    let mut s = house();
    let a = s.doc().clone();
    let wall = s
        .doc()
        .elements()
        .find(|e| e.as_wall().is_some())
        .unwrap()
        .id;
    let bx = s
        .doc()
        .elements()
        .find(|e| e.as_form().is_some())
        .unwrap()
        .id;
    run(&mut s, "set_wall", json!({"wall": wall, "height": 3500}));
    run(
        &mut s,
        "set_parameter",
        json!({"element": bx, "key": "acme.note.text", "value": {"type": "text", "value": "a/b~c"}}),
    );
    run(
        &mut s,
        "create_box",
        json!({"origin": {"x": 0, "y": 9000, "z": 0}, "width": 500, "depth": 500, "height": 500}),
    );
    run(&mut s, "delete_elements", json!({"elements": [bx]}));
    run(&mut s, "set_project_name", json!({"name": "Renamed"}));
    let b = s.doc().clone();
    let d = diff(&a, &b).unwrap();
    assert!(d.windows(2).all(|w| w[0].path() <= w[1].path()), "sorted");
    same_content(&apply(&a, &d).unwrap(), &b);
    assert!(diff(&b, &b).unwrap().is_empty());
    // Changed wall: property-level edits, not the whole element.
    let wuid = b.get(wall).unwrap().uid.to_string();
    assert!(d
        .iter()
        .any(|c| c.path().starts_with(&format!("/elements/{wuid}/"))));
    assert!(!d.iter().any(|c| c.path() == format!("/elements/{wuid}")));
}

#[test]
fn references_travel_as_uids() {
    let s = house();
    let doc = s.doc();
    let w = doc.elements().find(|e| e.as_wall().is_some()).unwrap();
    let v = element_value(doc, w).unwrap();
    let ty = doc.get(w.type_id.unwrap()).unwrap().uid.to_string();
    assert_eq!(v["type_id"], json!(ty));
    assert!(v.get("id").is_none());
    // A new element added by a change gets a fresh id and its refs resolve.
    let mut nv = v.clone();
    let new_uid = "01a00000-0000-7000-8000-00000000abcd";
    nv["uid"] = json!(new_uid);
    let doc2 = apply(
        doc,
        &[Change::Set {
            path: format!("/elements/{new_uid}"),
            value: nv,
        }],
    )
    .unwrap();
    let added = doc2
        .elements()
        .find(|e| e.uid.to_string() == new_uid)
        .unwrap();
    assert_eq!(added.type_id, w.type_id);
    assert!(added.id.0 > doc.elements().map(|e| e.id.0).max().unwrap());
}

#[test]
fn bad_edits_fail_cleanly_and_change_nothing() {
    let s = house();
    let doc = s.doc();
    let cases = [
        (
            Change::Remove {
                path: "/elements/nope".into(),
            },
            "ferro.change.missing",
        ),
        (
            Change::Set {
                path: "/elements/nope/name".into(),
                value: json!("x"),
            },
            "ferro.change.missing_parent",
        ),
        (
            Change::Set {
                path: "elements".into(),
                value: json!(1),
            },
            "ferro.change.bad_path",
        ),
        (
            Change::Set {
                path: "/".into(),
                value: json!(1),
            },
            "ferro.change.bad_path",
        ),
        (
            Change::Set {
                path: "/project/name/x".into(),
                value: json!(1),
            },
            "ferro.change.bad_path",
        ),
    ];
    for (c, code) in cases {
        let e = apply(doc, std::slice::from_ref(&c)).unwrap_err();
        assert_eq!(e.code, code, "{c:?}");
    }
    // A reference to an element that does not exist.
    let w = doc.elements().find(|e| e.as_wall().is_some()).unwrap();
    let bad = Change::Set {
        path: format!("/elements/{}/type_id", w.uid),
        value: json!("01a00000-0000-7000-8000-000000000000"),
    };
    assert_eq!(
        apply(doc, &[bad]).unwrap_err().code,
        "ferro.change.dangling_reference"
    );
    // Invalid element data.
    let bad = Change::Set {
        path: format!("/elements/{}/category", w.uid),
        value: json!("dragons"),
    };
    assert_eq!(apply(doc, &[bad]).unwrap_err().code, "ferro.change.invalid");
}

#[test]
fn conflicts_are_per_property() {
    let set = |p: &str, v: Value| Change::Set {
        path: p.into(),
        value: v,
    };
    let ours = [
        set("/elements/u1/params/a", json!(1)),
        set("/elements/u2/name", json!("x")),
    ];
    let theirs = [
        set("/elements/u1/params/b", json!(2)), // other property: fine
        set("/elements/u2/name", json!("x")),   // same edit: fine
    ];
    assert!(conflicts(&ours, &theirs).is_empty());
    let theirs = [set("/elements/u1/params/a", json!(3))];
    assert_eq!(conflicts(&ours, &theirs), ["/elements/u1/params/a"]);
    // Deleting an element conflicts with editing inside it.
    let theirs = [Change::Remove {
        path: "/elements/u2".into(),
    }];
    assert_eq!(conflicts(&ours, &theirs), ["/elements/u2"]);
    // Prefix of a different key is not related (u1 vs u10).
    let theirs = [Change::Remove {
        path: "/elements/u10".into(),
    }];
    assert!(conflicts(&[set("/elements/u1/name", json!(1))], &theirs).is_empty());
}

#[test]
fn a_parameter_pointing_at_a_deleted_element_survives() {
    let mut s = house();
    let bx = s
        .doc()
        .elements()
        .find(|e| e.as_form().is_some())
        .unwrap()
        .id;
    let wall = s
        .doc()
        .elements()
        .find(|e| e.as_wall().is_some())
        .unwrap()
        .id;
    run(
        &mut s,
        "set_parameter",
        json!({"element": wall, "key": "acme.link.target", "value": {"type": "element_ref", "value": bx.0}}),
    );
    let a = s.doc().clone();
    run(&mut s, "delete_elements", json!({"elements": [bx]}));
    let b = s.doc().clone();
    let d = diff(&a, &b).unwrap();
    same_content(&apply(&a, &d).unwrap(), &b);
    let w = element_value(&b, b.get(wall).unwrap()).unwrap();
    assert_eq!(w["params"]["acme.link.target"]["value"], json!(bx.0));
}
