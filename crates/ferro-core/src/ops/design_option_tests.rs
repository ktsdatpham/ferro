//! Design options (ADR 0006 §Design options, M5-23.9): layers, combinations,
//! edit in option, accept, stale layers, undo / redo.

use super::design_option_commands::{resolve, resolve_options};
use super::scenario_common_tests::*;
use super::*;
use crate::model::change::{partition_applicable, Change};
use crate::model::design_option::{
    check_design_options, layer_counts, stale_layers, Combination, OptionStatus,
};
use crate::model::{keys, Document, ElementId, ParamValue};
use serde_json::{json, Value};

fn tool(name: &str, args: Value) -> AnyCommand {
    AnyCommand::from_tool(name, &args).unwrap_or_else(|m| panic!("{name}: {m}"))
}

fn in_option(s: &mut Session, option: ElementId, name: &str, args: Value) -> Outcome {
    s.execute_in_option(
        option,
        &Combination::primary(),
        tool(name, args),
        &ExecContext::user(),
    )
    .unwrap_or_else(|f| panic!("{name} in option: {f:?}"))
}

fn ids(v: &Value) -> Vec<ElementId> {
    serde_json::from_value(v.clone()).expect("ids")
}

fn changes_of(s: &Session, option: ElementId) -> Vec<Change> {
    s.doc()
        .get(option)
        .and_then(|e| e.as_design_option())
        .map(|o| o.changes.clone())
        .expect("option")
}

fn height(doc: &Document, wall: ElementId) -> f64 {
    match doc.get(wall).and_then(|e| e.param(keys::HEIGHT)) {
        Some(ParamValue::Length(l)) => l.mm(),
        other => panic!("no height: {other:?}"),
    }
}

/// Shared model: a 6 m wall (3 m high) and a 1 m box; one option set
/// "Roof" with options A (primary) and B.
struct Fx {
    s: Session,
    wall: ElementId,
    bx: ElementId,
    set: ElementId,
    a: ElementId,
    b: ElementId,
}

fn fx() -> Fx {
    let mut s = session();
    let wall = id(&run(
        &mut s,
        "create_wall",
        json!({"start": p(0.0, 0.0, 0.0), "end": p(6000.0, 0.0, 0.0), "height": 3000}),
    ));
    let bx = boxed(&mut s, [10_000.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let o = run(&mut s, "create_option_set", json!({"name": "Roof"}));
    let set = id(&o);
    let a: ElementId = serde_json::from_value(o.output["option"].clone()).unwrap();
    let b = id(&run(
        &mut s,
        "create_design_option",
        json!({"set": set, "name": "B"}),
    ));
    Fx {
        s,
        wall,
        bx,
        set,
        a,
        b,
    }
}

#[test]
fn resolve_without_options_is_the_shared_model() {
    let mut s = session();
    boxed(&mut s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    let r = resolve(s.doc(), &Combination::primary(), None).unwrap();
    assert_eq!(&r, s.doc());
    // Empty layers change nothing either.
    let f = fx();
    let r = resolve(f.s.doc(), &Combination::primary(), None).unwrap();
    let shared: Vec<_> = f.s.doc().elements().collect();
    let seen: Vec<_> = r.elements().collect();
    assert_eq!(shared, seen);
}

#[test]
fn new_set_has_one_primary_option_and_new_options_are_secondary() {
    let f = fx();
    let doc = f.s.doc();
    assert_eq!(doc.option_sets().len(), 1);
    assert_eq!(doc.primary_option(f.set), Some(f.a));
    assert_eq!(doc.secondary_options(), vec![f.b]);
    assert_eq!(doc.get(f.a).unwrap().label(), "Option 1");
    assert!(check_design_options(doc).is_ok());
    assert_eq!(
        Combination::primary().chosen(doc).unwrap(),
        vec![f.a],
        "primary by default"
    );
}

#[test]
fn an_option_adds_removes_and_overrides_without_touching_the_shared_model() {
    let mut f = fx();
    let shared = f.s.doc().clone();
    // B: taller wall (override), box removed, a new box added.
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 4500}),
    );
    in_option(
        &mut f.s,
        f.b,
        "delete_elements",
        json!({"elements": [f.bx]}),
    );
    let added = id(&in_option(
        &mut f.s,
        f.b,
        "create_box",
        json!({"origin": p(0.0, 5000.0, 0.0), "width": 2000, "depth": 2000, "height": 2000}),
    ));
    let layer = changes_of(&f.s, f.b);
    let c = layer_counts(f.s.doc(), &layer);
    assert_eq!((c.added, c.removed, c.overridden), (1, 1, 1), "{layer:?}");
    // Derived data (wall geometry, computed lengths) is not stored.
    assert!(
        layer.iter().all(|c| !c.path().contains("/body/geometry")),
        "{layer:?}"
    );
    // Shared elements untouched.
    for e in shared.elements() {
        if e.as_design_option().is_none() {
            assert_eq!(f.s.doc().get(e.id), Some(e), "{}", e.label());
        }
    }
    // Seen with A (primary): the shared model.
    let in_a = resolve(f.s.doc(), &Combination::primary(), None).unwrap();
    assert!((height(&in_a, f.wall) - 3000.0).abs() < 1e-6);
    assert!(in_a.contains(f.bx));
    // Seen with B: overridden, removed, added (geometry regenerated).
    let combo = Combination { options: vec![f.b] };
    let in_b = resolve(f.s.doc(), &combo, None).unwrap();
    assert!((height(&in_b, f.wall) - 4500.0).abs() < 1e-6);
    let bb = in_b.get(f.wall).unwrap().bounding_box().unwrap();
    assert!((bb.max.z - 4500.0).abs() < 1e-6, "geometry follows: {bb:?}");
    assert!(!in_b.contains(f.bx));
    let new = in_b
        .get(added)
        .expect("added element keeps the id it was given");
    assert_eq!(new.design_option, Some(f.b), "derived index");
    assert!(
        in_b.elements()
            .all(|e| e.as_design_option().is_none_or(|o| o.changes.is_empty())),
        "a resolved document carries no layers"
    );
    // Resolving a resolved document changes nothing.
    assert_eq!(resolve(&in_b, &Combination::primary(), None).unwrap(), in_b);
}

#[test]
fn layers_given_as_changes_are_checked_against_the_shared_model() {
    let mut f = fx();
    let wall_uid = f.s.doc().get(f.wall).unwrap().uid;
    let ok = json!([{"op": "set", "path": format!("/elements/{wall_uid}/name"), "value": "Tall"}]);
    expect_ok(
        &mut f.s,
        "set_design_option",
        json!({"option": f.b, "changes": ok}),
    );
    let combo = Combination { options: vec![f.b] };
    let r = resolve(f.s.doc(), &combo, None).unwrap();
    assert_eq!(r.get(f.wall).unwrap().name.as_deref(), Some("Tall"));
    for (bad, why) in [
        (
            json!([{"op": "remove", "path": "/elements/00000000-0000-7000-8000-000000000000"}]),
            "missing element",
        ),
        (
            json!([{"op": "set", "path": "/project/name", "value": "X"}]),
            "project",
        ),
        (
            json!([{"op": "set", "path": format!("/elements/{}/name", f.s.doc().get(f.a).unwrap().uid), "value": "X"}]),
            "an option",
        ),
        (
            json!([{"op": "set", "path": format!("/elements/{wall_uid}/design_option"), "value": null}]),
            "index",
        ),
        (
            json!([{"op": "set", "path": format!("/elements/{wall_uid}/level_id"), "value": "nope"}]),
            "dangling reference",
        ),
    ] {
        expect_code(
            &mut f.s,
            "set_design_option",
            json!({"option": f.b, "changes": bad}),
            "ferro.model.invalid_option_layer",
        );
        let _ = why;
    }
}

#[test]
fn two_sets_combine_and_conflicting_layers_are_reported() {
    let mut f = fx();
    let o = run(&mut f.s, "create_option_set", json!({"name": "Facade"}));
    let set2 = id(&o);
    let c: ElementId = serde_json::from_value(o.output["option"].clone()).unwrap();
    let d = id(&run(&mut f.s, "create_design_option", json!({"set": set2})));
    assert_eq!(f.s.doc().get(d).unwrap().label(), "Option 2");
    // Set 1 / B: taller wall. Set 2 / D: box moved. Combined: both.
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 4000}),
    );
    in_option(
        &mut f.s,
        d,
        "move_elements",
        json!({"elements": [f.bx], "vector": p(0.0, 3000.0, 0.0)}),
    );
    let both = Combination {
        options: vec![d, f.b],
    };
    assert_eq!(both.chosen(f.s.doc()).unwrap(), vec![f.b, d], "set order");
    let r = resolve(f.s.doc(), &both, None).unwrap();
    assert!((height(&r, f.wall) - 4000.0).abs() < 1e-6);
    let bb = r.get(f.bx).unwrap().bounding_box().unwrap();
    assert!((bb.min.y - 3000.0).abs() < 1e-6);
    // Each alone.
    let only_d = resolve(f.s.doc(), &Combination { options: vec![d] }, None).unwrap();
    assert!((height(&only_d, f.wall) - 3000.0).abs() < 1e-6);
    // Two options of one set, or a non-option, are refused.
    let twice = Combination {
        options: vec![f.a, f.b],
    };
    assert_eq!(
        resolve(f.s.doc(), &twice, None).unwrap_err()[0].code,
        "ferro.model.option_set_twice"
    );
    assert_eq!(
        resolve(
            f.s.doc(),
            &Combination {
                options: vec![f.wall]
            },
            None
        )
        .unwrap_err()[0]
            .code,
        "ferro.model.not_a_design_option"
    );
    // Conflict: C (set 2) also changes the wall height.
    in_option(
        &mut f.s,
        c,
        "set_wall_height",
        json!({"wall": f.wall, "height": 3500}),
    );
    let clash = Combination {
        options: vec![f.b, c],
    };
    let err = resolve(f.s.doc(), &clash, None).unwrap_err();
    assert_eq!(err[0].code, "ferro.model.option_conflict");
    assert!(err[0].message.contains("/body/top"), "{}", err[0].message);
    // Editing in C while B is shown is refused too (layers would merge).
    let r = f.s.execute_in_option(
        c,
        &Combination { options: vec![f.b] },
        tool("set_name", json!({"element": f.wall, "name": "x"})),
        &ExecContext::user(),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.model.option_conflict");
    let _ = set2;
}

#[test]
fn editing_an_element_added_by_another_set_is_refused() {
    let mut f = fx();
    let o = run(&mut f.s, "create_option_set", json!({"name": "Site"}));
    let c: ElementId = serde_json::from_value(o.output["option"].clone()).unwrap();
    let added = id(&in_option(
        &mut f.s,
        f.a,
        "create_box",
        json!({"origin": p(0.0, 8000.0, 0.0), "width": 1000, "depth": 1000, "height": 1000}),
    ));
    let before = snapshot(&f.s);
    let r = f.s.execute_in_option(
        c,
        &Combination::primary(),
        tool("set_name", json!({"element": added, "name": "mine"})),
        &ExecContext::user(),
    );
    assert_eq!(r.unwrap_err()[0].code, "ferro.model.invalid_option_layer");
    assert_eq!(snapshot(&f.s), before, "nothing changed");
}

#[test]
fn an_edit_that_changes_nothing_leaves_an_empty_layer() {
    let mut f = fx();
    // Tolerance edge: a zero move and a rename to the same value.
    in_option(
        &mut f.s,
        f.b,
        "move_elements",
        json!({"elements": [f.bx], "vector": p(0.0, 0.0, 0.0)}),
    );
    assert!(changes_of(&f.s, f.b).is_empty());
    // A sub-tolerance edge made inside an option fails like anywhere else.
    let before = snapshot(&f.s);
    let r = f.s.execute_in_option(
        f.b,
        &Combination::primary(),
        tool(
            "create_box",
            json!({"origin": p(0.0, 0.0, 9000.0), "width": 0.4, "depth": 1000, "height": 1000}),
        ),
        &ExecContext::user(),
    );
    assert!(r.is_err());
    assert_eq!(snapshot(&f.s), before);
    // Just above the tolerance it works.
    in_option(
        &mut f.s,
        f.b,
        "create_box",
        json!({"origin": p(0.0, 0.0, 9000.0), "width": 0.6, "depth": 1000, "height": 1000}),
    );
    assert_eq!(layer_counts(f.s.doc(), &changes_of(&f.s, f.b)).added, 1);
}

#[test]
fn accept_merges_the_layer_and_archives_the_other_options() {
    let mut f = fx();
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 4200}),
    );
    let added = id(&in_option(
        &mut f.s,
        f.b,
        "create_box",
        json!({"origin": p(0.0, 5000.0, 0.0), "width": 1000, "depth": 1000, "height": 1000}),
    ));
    in_option(
        &mut f.s,
        f.a,
        "set_name",
        json!({"element": f.bx, "name": "kept in A"}),
    );
    let before = snapshot(&f.s);
    let o = expect_ok(&mut f.s, "accept_design_option", json!({"option": f.b}));
    assert_eq!(ids(&o.output["archived"]), vec![f.a]);
    let doc = f.s.doc();
    assert!((height(doc, f.wall) - 4200.0).abs() < 1e-6, "merged");
    let new = doc.get(added).expect("added element is now shared");
    assert_eq!(new.design_option, None, "shared elements have no option");
    assert!(new.bounding_box().is_some());
    let st = |id| doc.get(id).unwrap().as_design_option().unwrap().clone();
    assert_eq!(st(f.b).status, OptionStatus::Accepted);
    assert_eq!(st(f.a).status, OptionStatus::Archived);
    assert!(!st(f.a).changes.is_empty(), "archived layer kept");
    assert_eq!(doc.primary_option(f.set), None);
    assert!(!doc.has_open_design_options());
    assert!(check_design_options(doc).is_ok());
    // The set now shows nothing: the resolved document is the shared one.
    let r = resolve(doc, &Combination::primary(), None).unwrap();
    assert_eq!(r.get(f.bx).unwrap().name, None);
    // Archived / accepted options cannot be shown, edited or accepted.
    expect_code(
        &mut f.s,
        "accept_design_option",
        json!({"option": f.a}),
        "ferro.model.option_not_open",
    );
    expect_code(
        &mut f.s,
        "set_design_option",
        json!({"option": f.a, "is_primary": true}),
        "ferro.model.option_not_open",
    );
    assert_eq!(
        resolve(f.s.doc(), &Combination { options: vec![f.a] }, None).unwrap_err()[0].code,
        "ferro.model.option_not_open"
    );
    // Undo restores the exact document.
    f.s.undo().unwrap();
    assert_eq!(snapshot(&f.s), before);
    // A new option in the accepted set becomes its primary.
    f.s.redo().unwrap();
    let n = expect_ok(&mut f.s, "create_design_option", json!({"set": f.set}));
    assert_eq!(n.output["is_primary"], true);
}

#[test]
fn deleting_a_shared_element_an_option_overrides_is_reported_and_repairable() {
    let mut f = fx();
    in_option(
        &mut f.s,
        f.b,
        "set_name",
        json!({"element": f.bx, "name": "B box"}),
    );
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 3300}),
    );
    // Deleting the box from the shared model warns; nothing is dropped.
    let o = run(&mut f.s, "delete_elements", json!({"elements": [f.bx]}));
    let w: Vec<&str> = o.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(
        w,
        vec!["ferro.model.option_layer_stale"],
        "{:?}",
        o.warnings
    );
    assert_eq!(o.warnings[0].elements, vec![f.b]);
    assert_eq!(changes_of(&f.s, f.b).len(), 2, "layer kept as it was");
    let stale = stale_layers(f.s.doc());
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].option, Some(f.b));
    // Showing B fails with a clear code; A (primary, empty) still works.
    let combo = Combination { options: vec![f.b] };
    let err = resolve(f.s.doc(), &combo, None).unwrap_err();
    assert_eq!(err[0].code, "ferro.model.invalid_option_layer");
    assert!(resolve(f.s.doc(), &Combination::primary(), None).is_ok());
    // A later, unrelated command does not warn again.
    let o = run(&mut f.s, "set_project_name", json!({"name": "again"}));
    assert!(o.warnings.is_empty());
    // Repair: keep the edits that still apply.
    let (kept, dropped) = partition_applicable(f.s.doc(), &changes_of(&f.s, f.b)).unwrap();
    assert_eq!((kept.len(), dropped.len()), (1, 1));
    expect_ok(
        &mut f.s,
        "set_design_option",
        json!({"option": f.b, "changes": kept}),
    );
    let r = resolve(f.s.doc(), &combo, None).unwrap();
    assert!((height(&r, f.wall) - 3300.0).abs() < 1e-6);
    assert!(stale_layers(f.s.doc()).is_empty());
}

#[test]
fn rules_on_names_primary_and_deletion() {
    let mut f = fx();
    expect_code(
        &mut f.s,
        "create_option_set",
        json!({"name": "Roof"}),
        "ferro.model.duplicate_name",
    );
    expect_code(
        &mut f.s,
        "create_option_set",
        json!({"name": "  "}),
        "ferro.model.invalid_name",
    );
    expect_code(
        &mut f.s,
        "create_design_option",
        json!({"set": f.set, "name": "B"}),
        "ferro.model.duplicate_name",
    );
    expect_code(
        &mut f.s,
        "create_design_option",
        json!({"set": f.a}),
        "ferro.model.not_an_option_set",
    );
    expect_code(
        &mut f.s,
        "set_design_option",
        json!({"option": f.set, "name": "x"}),
        "ferro.model.not_a_design_option",
    );
    expect_code(
        &mut f.s,
        "set_design_option",
        json!({"option": f.a, "is_primary": false}),
        "ferro.model.primary_required",
    );
    expect_code(
        &mut f.s,
        "delete_design_option",
        json!({"option": f.a}),
        "ferro.model.option_is_primary",
    );
    // Switch primary, then the old primary can go.
    expect_ok(
        &mut f.s,
        "set_design_option",
        json!({"option": f.b, "is_primary": true, "name": "Flat roof"}),
    );
    assert_eq!(f.s.doc().primary_option(f.set), Some(f.b));
    assert_eq!(f.s.doc().get(f.b).unwrap().label(), "Flat roof");
    expect_ok(&mut f.s, "delete_design_option", json!({"option": f.a}));
    assert_eq!(f.s.doc().design_options(f.set).len(), 1);
    // Sets reorder and delete with their options.
    let s2 = id(&run(&mut f.s, "create_option_set", json!({"name": "Site"})));
    expect_ok(
        &mut f.s,
        "set_option_set",
        json!({"set": s2, "position": 0, "name": "Site plan"}),
    );
    let order: Vec<ElementId> = f.s.doc().option_sets().iter().map(|e| e.id).collect();
    assert_eq!(order, vec![s2, f.set]);
    let o = expect_ok(&mut f.s, "delete_option_set", json!({"set": f.set}));
    assert_eq!(ids(&o.output["options"]), vec![f.b]);
    assert!(!f.s.doc().contains(f.b));
    assert!(check_design_options(f.s.doc()).is_ok());
    expect_code(
        &mut f.s,
        "create_design_option",
        json!({"set": s2, "copy_from": 9999}),
        "ferro.model.not_found",
    );
}

#[test]
fn copy_from_duplicates_a_layer() {
    let mut f = fx();
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 3900}),
    );
    let c = id(&run(
        &mut f.s,
        "create_design_option",
        json!({"set": f.set, "copy_from": f.b}),
    ));
    assert_eq!(changes_of(&f.s, c), changes_of(&f.s, f.b));
}

#[test]
fn every_option_command_undoes_and_redoes_exactly() {
    let base = fx();
    let wall_uid = base.s.doc().get(base.wall).unwrap().uid;
    let layer = json!([{"op": "set", "path": format!("/elements/{wall_uid}/name"), "value": "W"}]);
    let calls = [
        ("create_option_set", json!({"name": "Site"})),
        (
            "set_option_set",
            json!({"set": base.set, "name": "Roofs", "position": 0}),
        ),
        ("delete_option_set", json!({"set": base.set})),
        (
            "create_design_option",
            json!({"set": base.set, "name": "C", "copy_from": base.b}),
        ),
        (
            "set_design_option",
            json!({"option": base.b, "is_primary": true, "changes": layer.clone()}),
        ),
        ("delete_design_option", json!({"option": base.b})),
        ("accept_design_option", json!({"option": base.a})),
        ("apply_changes", json!({"changes": layer})),
    ];
    for (name, args) in calls {
        let mut f = fx();
        let before = snapshot(&f.s);
        expect_ok(&mut f.s, name, args.clone());
        let after = snapshot(&f.s);
        assert_ne!(before, after, "{name} changed nothing");
        f.s.undo().unwrap();
        assert_eq!(snapshot(&f.s), before, "{name}: undo");
        f.s.redo().unwrap();
        assert_eq!(snapshot(&f.s), after, "{name}: redo");
    }
    // Edit in option: one undo step, named after the stored command.
    let mut f = fx();
    let before = snapshot(&f.s);
    in_option(
        &mut f.s,
        f.b,
        "set_wall_height",
        json!({"wall": f.wall, "height": 5000}),
    );
    assert_eq!(f.s.undo_name(), Some("set_design_option"));
    let after = snapshot(&f.s);
    f.s.undo().unwrap();
    assert_eq!(snapshot(&f.s), before);
    f.s.redo().unwrap();
    assert_eq!(snapshot(&f.s), after);
}

#[test]
fn edits_inside_a_group_are_one_undo_step() {
    let mut f = fx();
    let before = snapshot(&f.s);
    f.s.begin_group("gesture", ExecContext::user());
    for h in [3100, 3200, 3300] {
        in_option(
            &mut f.s,
            f.b,
            "set_wall_height",
            json!({"wall": f.wall, "height": h}),
        );
    }
    f.s.end_group();
    let combo = Combination { options: vec![f.b] };
    let r = resolve(f.s.doc(), &combo, None).unwrap();
    assert!((height(&r, f.wall) - 3300.0).abs() < 1e-6);
    f.s.undo().unwrap();
    assert_eq!(snapshot(&f.s), before);
}

#[test]
fn apply_changes_replays_a_diff_and_keeps_option_rules() {
    let mut f = fx();
    let a = f.s.doc().clone();
    run(
        &mut f.s,
        "set_wall_height",
        json!({"wall": f.wall, "height": 3600}),
    );
    let b = f.s.doc().clone();
    let changes = crate::model::change::diff(&a, &b).unwrap();
    let mut g = fx();
    expect_ok(&mut g.s, "apply_changes", json!({"changes": changes}));
    assert!((height(g.s.doc(), g.wall) - 3600.0).abs() < 1e-6);
    // Two primaries in a set break the rules.
    let b_uid = g.s.doc().get(g.b).unwrap().uid;
    expect_code(
        &mut g.s,
        "apply_changes",
        json!({"changes": [{"op": "set", "path": format!("/elements/{b_uid}/body/is_primary"), "value": true}]}),
        "ferro.model.invalid_design_options",
    );
    expect_code(
        &mut g.s,
        "apply_changes",
        json!({"changes": [{"op": "remove", "path": "/elements/nope"}]}),
        "ferro.change.missing",
    );
}

#[test]
fn an_added_opening_that_does_not_fit_fails_to_resolve() {
    let mut f = fx();
    in_option(
        &mut f.s,
        f.b,
        "create_opening",
        json!({"wall": f.wall, "kind": "door", "position": 3000, "width": 900, "height": 2100}),
    );
    let combo = Combination { options: vec![f.b] };
    assert!(resolve(f.s.doc(), &combo, None).is_ok());
    // The shared wall gets shorter: the option's door no longer fits.
    run(
        &mut f.s,
        "set_wall_height",
        json!({"wall": f.wall, "height": 1500}),
    );
    let err = resolve(f.s.doc(), &combo, None).unwrap_err();
    assert!(err[0].code.starts_with("ferro."), "{err:?}");
    let _ = resolve_options(f.s.doc(), &[], None).unwrap();
}

#[test]
fn an_option_can_hold_a_relationship_row() {
    let mut f = fx();
    let o = in_option(
        &mut f.s,
        f.b,
        "create_relationship",
        json!({"kind": "connect", "elements": [f.wall, f.bx]}),
    );
    let uid = o.output["relationship"].as_str().expect("uid").to_string();
    assert_eq!(
        f.s.doc().relationships().count(),
        0,
        "shared model unchanged"
    );
    let layer = changes_of(&f.s, f.b);
    assert!(
        layer
            .iter()
            .any(|c| c.path() == format!("/relationships/{uid}")),
        "{layer:?}"
    );
    let combo = Combination { options: vec![f.b] };
    let r = resolve(f.s.doc(), &combo, None).unwrap();
    assert_eq!(r.relationships().count(), 1);
    // Accepting moves the row into the shared model.
    expect_ok(&mut f.s, "accept_design_option", json!({"option": f.b}));
    assert_eq!(f.s.doc().relationships().count(), 1);
    f.s.undo().unwrap();
    assert_eq!(f.s.doc().relationships().count(), 0);
}
