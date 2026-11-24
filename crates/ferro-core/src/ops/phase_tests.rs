//! Phase commands, validation, delete rules and quantities (M5-23).

use super::*;
use crate::model::{Category, Document, ElementId, PhaseQuantities, PhaseStatus};
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
    match try_run(s, tool, args) {
        Ok(o) => o,
        Err(f) => panic!("{tool} failed: {f:?}"),
    }
}

fn fails(s: &mut Session, tool: &str, args: Value, code: &str) {
    let before = s.doc().clone();
    let f = try_run(s, tool, args).expect_err("should fail");
    assert_eq!(f[0].code, code, "{f:?}");
    assert_eq!(s.doc(), &before, "rolled back");
}

fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).unwrap()
}

fn cube(s: &mut Session, x: f64) -> ElementId {
    id(&run(
        s,
        "create_box",
        json!({"origin": {"x": x, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
    ))
}

fn names(s: &Session) -> Vec<String> {
    s.doc()
        .phases()
        .iter()
        .map(|p| p.name.clone().unwrap_or_default())
        .collect()
}

#[test]
fn new_documents_have_no_phases_until_the_simple_mode_is_used() {
    let mut s = session();
    let a = cube(&mut s, 0.0);
    assert!(s.doc().phases().is_empty());
    let before = s.doc().clone();
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [a], "status": "new"}),
    );
    assert_eq!(names(&s), ["Existing", "New Construction"]);
    let ph = s.doc().phases().iter().map(|p| p.id).collect::<Vec<_>>();
    let e = s.doc().get(a).unwrap();
    assert_eq!((e.phase_created, e.phase_demolished), (Some(ph[1]), None));
    // One undo step removes the phases too.
    s.undo().unwrap();
    assert_eq!(s.doc(), &before);
}

#[test]
fn simple_mode_maps_to_the_first_and_last_phase() {
    let mut s = session();
    let (a, b, c) = (
        cube(&mut s, 0.0),
        cube(&mut s, 2000.0),
        cube(&mut s, 4000.0),
    );
    for (e, st) in [(a, "existing"), (b, "demolish"), (c, "new")] {
        run(
            &mut s,
            "set_renovation_status",
            json!({"elements": [e], "status": st}),
        );
    }
    assert_eq!(s.doc().phases().len(), 2, "defaults created once");
    let ph: Vec<ElementId> = s.doc().phases().iter().map(|p| p.id).collect();
    let st = |s: &Session, e, v| s.doc().phase_status(s.doc().get(e).unwrap(), v);
    let st1 = |e, v| st(&s, e, v);
    assert_eq!(
        [st1(a, ph[1]), st1(b, ph[1]), st1(c, ph[1])],
        [
            PhaseStatus::Existing,
            PhaseStatus::Demolished,
            PhaseStatus::New
        ]
    );
    // In the Existing phase the new cube is not built yet.
    assert_eq!(
        [st1(a, ph[0]), st1(b, ph[0]), st1(c, ph[0])],
        [PhaseStatus::New, PhaseStatus::New, PhaseStatus::Future]
    );
    // An element without phases counts as existing everywhere.
    let d = cube(&mut s, 6000.0);
    assert_eq!(st(&s, d, ph[0]), PhaseStatus::Existing);
    assert_eq!(st(&s, d, ph[1]), PhaseStatus::Existing);
}

#[test]
fn simple_mode_with_one_user_phase_adds_new_construction() {
    let mut s = session();
    run(&mut s, "create_phase", json!({"name": "Survey"}));
    let a = cube(&mut s, 0.0);
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [a], "status": "existing"}),
    );
    assert_eq!(names(&s), ["Survey", "New Construction"]);
}

#[test]
fn simple_mode_refuses_levels_and_types() {
    let mut s = session();
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    fails(
        &mut s,
        "set_renovation_status",
        json!({"elements": [l], "status": "new"}),
        "ferro.model.not_phased",
    );
    fails(
        &mut s,
        "set_renovation_status",
        json!({"elements": [999], "status": "new"}),
        "ferro.model.not_found",
    );
}

#[test]
fn phases_are_ordered_renamed_and_dated() {
    let mut s = session();
    let a = id(&run(&mut s, "create_phase", json!({"name": "A"})));
    let c = id(&run(&mut s, "create_phase", json!({"name": "C"})));
    let b = id(&run(
        &mut s,
        "create_phase",
        json!({"name": "B", "before": c, "start_date": "2027-01-01", "end_date": "2027-03-31"}),
    ));
    assert_eq!(names(&s), ["A", "B", "C"]);
    let orders: Vec<u32> = s
        .doc()
        .phases()
        .iter()
        .map(|p| p.as_phase().unwrap().order)
        .collect();
    assert_eq!(orders, [0, 1, 2]);
    run(
        &mut s,
        "set_phase",
        json!({"phase": a, "position": 5, "name": "Z"}),
    );
    assert_eq!(names(&s), ["B", "C", "Z"]);
    run(&mut s, "set_phase", json!({"phase": a, "position": 0}));
    assert_eq!(names(&s), ["Z", "B", "C"]);
    // "" clears a date; the other is kept.
    run(&mut s, "set_phase", json!({"phase": b, "start_date": ""}));
    let p = s.doc().get(b).unwrap().as_phase().unwrap().clone();
    assert_eq!(
        (p.start_date, p.end_date.as_deref()),
        (None, Some("2027-03-31"))
    );
    fails(
        &mut s,
        "create_phase",
        json!({"name": "B"}),
        "ferro.model.duplicate_name",
    );
    fails(
        &mut s,
        "create_phase",
        json!({"name": "  "}),
        "ferro.model.invalid_name",
    );
    fails(
        &mut s,
        "set_phase",
        json!({"phase": c, "name": "Z"}),
        "ferro.model.duplicate_name",
    );
    fails(
        &mut s,
        "create_phase",
        json!({"name": "D", "start_date": "2027-02-30"}),
        "ferro.model.invalid_date",
    );
    fails(
        &mut s,
        "set_phase",
        json!({"phase": b, "start_date": "2027-04-01"}),
        "ferro.model.invalid_date",
    );
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    fails(
        &mut s,
        "set_phase",
        json!({"phase": l, "name": "x"}),
        "ferro.model.not_a_phase",
    );
    fails(
        &mut s,
        "create_phase",
        json!({"name": "E", "before": l}),
        "ferro.model.not_a_phase",
    );
}

#[test]
fn element_phases_are_validated() {
    let mut s = session();
    let p1 = id(&run(&mut s, "create_phase", json!({"name": "1"})));
    let p2 = id(&run(&mut s, "create_phase", json!({"name": "2"})));
    let a = cube(&mut s, 0.0);
    fails(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": p2, "demolished": p1}),
        "ferro.model.invalid_phase_order",
    );
    fails(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": a}),
        "ferro.model.not_a_phase",
    );
    fails(
        &mut s,
        "set_element_phase",
        json!({"elements": [p1], "created": p1}),
        "ferro.model.not_phased",
    );
    // Same phase = temporary.
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": p2, "demolished": p2}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!(s.doc().phase_status(e, p2), PhaseStatus::Temporary);
    // Missing fields are kept, null clears.
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "demolished": null}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!((e.phase_created, e.phase_demolished), (Some(p2), None));
    // Demolished only, created missing (= existing before every phase).
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": null, "demolished": p1}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!(s.doc().phase_status(e, p1), PhaseStatus::Demolished);
    assert_eq!(s.doc().phase_status(e, p2), PhaseStatus::Past);
    // Undo restores exactly.
    let before = s.doc().clone();
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": p1}),
    );
    s.undo().unwrap();
    assert_eq!(s.doc(), &before);
}

#[test]
fn reordering_phases_keeps_demolition_after_creation() {
    let mut s = session();
    let p1 = id(&run(&mut s, "create_phase", json!({"name": "1"})));
    let p2 = id(&run(&mut s, "create_phase", json!({"name": "2"})));
    let a = cube(&mut s, 0.0);
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": p1, "demolished": p2}),
    );
    fails(
        &mut s,
        "set_phase",
        json!({"phase": p2, "position": 0}),
        "ferro.model.invalid_phase_order",
    );
}

#[test]
fn used_phases_are_deleted_only_by_merging_into_a_neighbour() {
    let mut s = session();
    let p1 = id(&run(&mut s, "create_phase", json!({"name": "1"})));
    let p2 = id(&run(&mut s, "create_phase", json!({"name": "2"})));
    let p3 = id(&run(&mut s, "create_phase", json!({"name": "3"})));
    let a = cube(&mut s, 0.0);
    run(
        &mut s,
        "set_element_phase",
        json!({"elements": [a], "created": p1, "demolished": p2}),
    );
    fails(
        &mut s,
        "delete_elements",
        json!({"elements": [p2]}),
        "ferro.model.phase_in_use",
    );
    fails(
        &mut s,
        "delete_phase",
        json!({"phase": p2}),
        "ferro.model.phase_in_use",
    );
    fails(
        &mut s,
        "delete_phase",
        json!({"phase": p1, "merge_into": p3}),
        "ferro.model.phase_not_adjacent",
    );
    fails(
        &mut s,
        "delete_phase",
        json!({"phase": p1, "merge_into": a}),
        "ferro.model.not_a_phase",
    );
    let before = s.doc().clone();
    run(
        &mut s,
        "delete_phase",
        json!({"phase": p2, "merge_into": p3}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!((e.phase_created, e.phase_demolished), (Some(p1), Some(p3)));
    assert_eq!(names(&s), ["1", "3"]);
    assert_eq!(s.doc().get(p3).unwrap().as_phase().unwrap().order, 1);
    s.undo().unwrap();
    assert_eq!(s.doc(), &before);
    // Merging the first into the second: created and demolished in one
    // phase (temporary).
    run(
        &mut s,
        "delete_phase",
        json!({"phase": p1, "merge_into": p2}),
    );
    let e = s.doc().get(a).unwrap();
    assert_eq!((e.phase_created, e.phase_demolished), (Some(p2), Some(p2)));
    // An unused phase is deleted plainly, also through delete_elements.
    run(&mut s, "delete_elements", json!({"elements": [p3]}));
    assert_eq!(names(&s), ["2"]);
}

#[test]
fn phases_cannot_be_moved_or_classified() {
    let mut s = session();
    let p = id(&run(&mut s, "create_phase", json!({"name": "1"})));
    fails(
        &mut s,
        "move_elements",
        json!({"elements": [p], "vector": {"x": 1, "y": 0, "z": 0}}),
        "ferro.model.not_movable",
    );
    assert_eq!(s.doc().get(p).unwrap().category, Category::Phases);
}

#[test]
fn quantities_are_split_by_status() {
    let mut s = session();
    let (a, b, c) = (
        cube(&mut s, 0.0),
        cube(&mut s, 2000.0),
        cube(&mut s, 4000.0),
    );
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    let w = id(&run(
        &mut s,
        "create_wall",
        json!({"start": {"x": 0, "y": 5000, "z": 0}, "end": {"x": 4000, "y": 5000, "z": 0}, "level": l, "height": 3000}),
    ));
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [a, w], "status": "demolish"}),
    );
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [b], "status": "new"}),
    );
    let ph: Vec<ElementId> = s.doc().phases().iter().map(|p| p.id).collect();
    let q = PhaseQuantities::from_document(s.doc(), ph[1]).unwrap();
    let forms = &q.categories[&Category::Forms];
    let v = 1e9; // 1 m³ in mm³
    assert!((forms[&PhaseStatus::Demolished].volume_mm3 - v).abs() < 1e-3);
    assert!((forms[&PhaseStatus::New].volume_mm3 - v).abs() < 1e-3);
    // `c` has no phases: existing.
    assert_eq!(forms[&PhaseStatus::Existing].count, 1);
    let walls = &q.categories[&Category::Walls][&PhaseStatus::Demolished];
    assert_eq!(walls.count, 1);
    assert!(walls.volume_mm3 > 0.0);
    assert!((walls.area_mm2 - 4000.0 * 3000.0).abs() < 1.0, "{walls:?}");
    assert_eq!(q.total(PhaseStatus::Demolished).count, 2);
    // In the Existing phase the new cube is future: not counted.
    let q0 = PhaseQuantities::from_document(s.doc(), ph[0]).unwrap();
    assert_eq!(
        q0.total(PhaseStatus::New).count,
        2,
        "a and w built in phase 0"
    );
    assert_eq!(q0.total(PhaseStatus::Existing).count, 1, "c only");
    assert!(PhaseQuantities::from_document(s.doc(), c).is_none());
    // Deterministic JSON.
    assert_eq!(
        serde_json::to_string(&q).unwrap(),
        serde_json::to_string(&PhaseQuantities::from_document(s.doc(), ph[1]).unwrap()).unwrap()
    );
}
