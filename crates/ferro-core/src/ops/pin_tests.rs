//! Pins as Revit (ADR 0008 revised, M5-27): a pin blocks moving and
//! deleting, and deleting the host of a pinned element; everything else
//! stays allowed, including for agents, scripts and plugins. The
//! every-command check is
//! `scenario_tests::pinned_fixture_refuses_moves_and_deletes_only`.

use super::pin_commands::SetPinned;
use super::scenario_common_tests::*;
use super::*;
use crate::math::tol::VERTEX_TOL;
use crate::model::design_option::Combination;
use crate::model::{Category, ElementId};
use serde_json::{json, Value};

fn uid(s: &Session, e: ElementId) -> String {
    s.doc().get(e).map(|e| e.uid.to_string()).expect("element")
}

fn pinned(s: &Session, e: ElementId) -> bool {
    s.doc().get(e).is_some_and(|e| e.pinned)
}

/// A level, a wall on it with a door, a joined second wall, a floor and a
/// box (none pinned).
struct Px {
    s: Session,
    level: ElementId,
    wall: ElementId,
    wall2: ElementId,
    door: ElementId,
    floor: ElementId,
    bx: ElementId,
}

fn px() -> Px {
    let mut s = session();
    let level = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    let wall = id(&run(
        &mut s,
        "create_wall",
        json!({"start": p(0.0, 0.0, 0.0), "end": p(6000.0, 0.0, 0.0), "level": level, "height": 3000}),
    ));
    let wall2 = id(&run(
        &mut s,
        "create_wall",
        json!({"start": p(6000.0, 0.0, 0.0), "end": p(6000.0, 4000.0, 0.0), "level": level, "height": 3000}),
    ));
    let door = id(&run(
        &mut s,
        "create_opening",
        json!({"wall": wall, "kind": "door", "position": 1500}),
    ));
    let floor = id(&run(
        &mut s,
        "create_floor",
        json!({"points": [{"x":0,"y":0},{"x":6000,"y":0},{"x":6000,"y":4000},{"x":0,"y":4000}], "level": level}),
    ));
    let bx = boxed(&mut s, [10_000.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    Px {
        s,
        level,
        wall,
        wall2,
        door,
        floor,
        bx,
    }
}

fn pin(s: &mut Session, ids: &[ElementId]) {
    run(s, "set_pinned", json!({"elements": ids, "pinned": true}));
}

fn refused(s: &mut Session, tool: &str, args: Value, pinned_id: ElementId) {
    let before = snapshot(s);
    let f = try_run(s, tool, args.clone()).expect_err("refused");
    assert_eq!(f[0].code, "ferro.model.pinned", "{tool} {args}: {f:?}");
    assert_eq!(f[0].elements[0], pinned_id, "{tool}: failure names it");
    assert_eq!(snapshot(s), before, "{tool}: rolled back");
}

#[test]
fn a_pinned_level_cannot_move_or_go_but_takes_other_edits() {
    let mut f = px();
    assert!(
        !pinned(&f.s, f.level),
        "levels are not auto-pinned by default"
    );
    let l = f.level;
    pin(&mut f.s, &[l]);
    refused(
        &mut f.s,
        "set_level_elevation",
        json!({"level": l, "elevation": 100}),
        l,
    );
    refused(&mut f.s, "delete_elements", json!({"elements": [l]}), l);
    run(
        &mut f.s,
        "set_name",
        json!({"element": l, "name": "Ground"}),
    );
    run(
        &mut f.s,
        "set_parameter",
        json!({"element": l, "key": "ferro.common.comments", "value": {"type": "text", "value": "datum"}}),
    );
    let o = run(
        &mut f.s,
        "set_pinned",
        json!({"elements": [l], "pinned": false}),
    );
    assert_eq!(o.output["changed"], json!([l]));
    run(
        &mut f.s,
        "set_level_elevation",
        json!({"level": l, "elevation": 100}),
    );
}

/// Walls: the location curve is locked (move, rotate, drag an end);
/// height, type, phase, freezing stay free.
#[test]
fn a_pinned_wall_keeps_its_location_and_takes_everything_else() {
    let mut f = px();
    let w = f.wall;
    pin(&mut f.s, &[w]);
    refused(
        &mut f.s,
        "move_elements",
        json!({"elements": [w], "vector": p(0.0, 100.0, 0.0)}),
        w,
    );
    refused(
        &mut f.s,
        "rotate_elements",
        json!({"elements": [w], "center": p(0.0, 0.0, 0.0), "angle": 10}),
        w,
    );
    refused(
        &mut f.s,
        "set_wall",
        json!({"wall": w, "start": p(0.0, 0.0, 0.0), "end": p(5000.0, 0.0, 0.0)}),
        w,
    );
    run(&mut f.s, "set_wall", json!({"wall": w, "height": 2500}));
    let ph = id(&run(&mut f.s, "create_phase", json!({"name": "New"})));
    run(
        &mut f.s,
        "set_element_phase",
        json!({"elements": [w], "created": ph}),
    );
    run(
        &mut f.s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    assert!(pinned(&f.s, w), "freezing keeps the pin");
}

#[test]
fn a_pinned_door_cannot_slide_but_can_change_size() {
    let mut f = px();
    let d = f.door;
    pin(&mut f.s, &[d]);
    refused(
        &mut f.s,
        "set_opening",
        json!({"opening": d, "position": 2000}),
        d,
    );
    run(
        &mut f.s,
        "set_opening",
        json!({"opening": d, "width": 1000}),
    );
}

/// Forms and floors: moved as a whole is refused, reshaping is a shape or
/// profile edit and passes.
#[test]
fn shapes_and_profiles_of_pinned_elements_stay_editable() {
    let mut f = px();
    let (bx, fl) = (f.bx, f.floor);
    pin(&mut f.s, &[bx, fl]);
    let top = face_towards(&geometry(&f.s, bx), crate::math::Vec3::Z);
    run(
        &mut f.s,
        "push_pull_face",
        json!({"element": bx, "face": top, "distance": 500}),
    );
    run(
        &mut f.s,
        "scale_elements",
        json!({"elements": [bx], "center": p(10_000.0, 0.0, 0.0), "factors": [2.0, 1.0, 1.0]}),
    );
    refused(
        &mut f.s,
        "move_elements",
        json!({"elements": [bx], "vector": p(0.0, 0.0, 300.0)}),
        bx,
    );
    refused(
        &mut f.s,
        "rotate_elements",
        json!({"elements": [bx], "center": p(0.0, 0.0, 0.0), "angle": 90}),
        bx,
    );
    run(
        &mut f.s,
        "set_floor",
        json!({"floor": fl, "points": [{"x":0,"y":0},{"x":7000,"y":0},{"x":7000,"y":4000},{"x":0,"y":4000}]}),
    );
    refused(
        &mut f.s,
        "move_elements",
        json!({"elements": [fl], "vector": p(500.0, 0.0, 0.0)}),
        fl,
    );
}

#[test]
fn deleting_the_host_of_a_pinned_door_is_refused() {
    let mut f = px();
    let (w, d) = (f.wall, f.door);
    pin(&mut f.s, &[d]);
    let before = snapshot(&f.s);
    let fails =
        try_run(&mut f.s, "delete_elements", json!({"elements": [w]})).expect_err("refused");
    assert_eq!(fails[0].code, "ferro.model.pinned");
    assert_eq!(fails[0].elements, vec![d, w]);
    assert!(fails[0].message.contains("hosted"), "{}", fails[0].message);
    assert_eq!(snapshot(&f.s), before);
}

/// Copies of pinned elements are not pinned (as Revit) and can be placed.
#[test]
fn copies_of_pinned_elements_are_free() {
    let mut f = px();
    let bx = f.bx;
    pin(&mut f.s, &[bx]);
    let o = run(
        &mut f.s,
        "copy_elements",
        json!({"elements": [bx], "vector": p(0.0, 5000.0, 0.0)}),
    );
    let copy = o.changed.added[0];
    assert!(!pinned(&f.s, copy));
    run(
        &mut f.s,
        "move_elements",
        json!({"elements": [copy], "vector": p(100.0, 0.0, 0.0)}),
    );
}

/// Regenerated values of a pinned element may change: a joined neighbour
/// moving (the wall's derived geometry), a pinned Form following its
/// unpinned level.
#[test]
fn regeneration_may_move_what_follows_an_unpinned_element() {
    let mut f = px();
    let (l, w, w2) = (f.level, f.wall, f.wall2);
    let form = id(&run(
        &mut f.s,
        "create_box",
        json!({"origin": p(0.0, 20_000.0, 0.0), "width": 1000, "depth": 1000, "height": 1000, "level": l}),
    ));
    pin(&mut f.s, &[w, form]);
    let g0 = geometry(&f.s, w);
    run(
        &mut f.s,
        "move_elements",
        json!({"elements": [w2], "vector": p(500.0, 0.0, 0.0)}),
    );
    assert_ne!(
        geometry(&f.s, w),
        g0,
        "the join regenerated the pinned wall"
    );
    let z0 = geometry(&f.s, form).bounding_box().expect("bbox").min.z;
    run(
        &mut f.s,
        "set_level_elevation",
        json!({"level": l, "elevation": 300}),
    );
    let z1 = geometry(&f.s, form).bounding_box().expect("bbox").min.z;
    assert!(
        (z1 - z0 - 300.0).abs() < 1e-9,
        "a pinned Form follows its level"
    );
}

/// Tolerance edge: moves within `VERTEX_TOL` are not moves; twice that is.
#[test]
fn moves_below_vertex_tolerance_are_not_moves() {
    let mut f = px();
    let bx = f.bx;
    pin(&mut f.s, &[bx]);
    let _ = try_run(
        &mut f.s,
        "move_elements",
        json!({"elements": [bx], "vector": p(VERTEX_TOL * 0.5, 0.0, 0.0)}),
    )
    .expect("within tolerance");
    refused(
        &mut f.s,
        "move_elements",
        json!({"elements": [bx], "vector": p(VERTEX_TOL * 2.0, 0.0, 0.0)}),
        bx,
    );
}

#[test]
fn set_pinned_degenerate_inputs() {
    let mut f = px();
    let o = run(
        &mut f.s,
        "set_pinned",
        json!({"elements": [], "pinned": true}),
    );
    assert_eq!(o.output["changed"], json!([]));
    let l = f.level;
    pin(&mut f.s, &[l]);
    let o = run(
        &mut f.s,
        "set_pinned",
        json!({"elements": [l, l], "pinned": true}),
    );
    assert_eq!(o.output["changed"], json!([]), "already pinned");
    expect_code(
        &mut f.s,
        "set_pinned",
        json!({"elements": [l, 9999], "pinned": false}),
        "ferro.model.not_found",
    );
    assert!(pinned(&f.s, l), "rolled back");
}

#[test]
fn undo_and_redo_of_set_pinned() {
    let mut f = px();
    let bx = f.bx;
    let before = snapshot(&f.s);
    pin(&mut f.s, &[bx]);
    let after = snapshot(&f.s);
    f.s.undo().expect("undo");
    assert_eq!(snapshot(&f.s), before);
    assert!(!pinned(&f.s, bx));
    f.s.redo().expect("redo");
    assert_eq!(snapshot(&f.s), after);
    refused(&mut f.s, "delete_elements", json!({"elements": [bx]}), bx);
}

#[test]
fn auto_pin_follows_the_project_setting() {
    let mut s = session();
    let a = id(&run(
        &mut s,
        "create_level",
        json!({"name": "A", "elevation": 0}),
    ));
    assert!(!pinned(&s, a), "off by default (as Revit)");
    run(&mut s, "set_project_name", json!({"auto_pin_datums": true}));
    assert!(s.doc().project().settings.auto_pin_datums);
    assert_eq!(s.doc().project().name, "Scenario", "name kept");
    let b = id(&run(
        &mut s,
        "create_level",
        json!({"name": "B", "elevation": 3000}),
    ));
    assert!(pinned(&s, b));
    let bx = boxed(&mut s, [0.0, 0.0, 0.0], 1000.0, 1000.0, 1000.0);
    assert!(!pinned(&s, bx), "only datums");
    run(
        &mut s,
        "set_project_name",
        json!({"auto_pin_datums": false}),
    );
    let c = id(&run(
        &mut s,
        "create_level",
        json!({"name": "C", "elevation": 6000}),
    ));
    assert!(!pinned(&s, c));
    assert!(Category::Levels.is_datum() && Category::Grids.is_datum());
    assert!(!Category::Walls.is_datum());
}

#[test]
fn option_layers_cannot_move_or_remove_pinned_elements() {
    let mut f = px();
    let (w, bx) = (f.wall, f.bx);
    let o = run(&mut f.s, "create_option_set", json!({"name": "Scheme"}));
    let opt: ElementId = serde_json::from_value(o.output["option"].clone()).expect("option");
    pin(&mut f.s, &[w, bx]);
    let (u, ub) = (uid(&f.s, w), uid(&f.s, bx));
    let before = snapshot(&f.s);
    let fails = try_run(
        &mut f.s,
        "set_design_option",
        json!({"option": opt, "changes": [{"op": "remove", "path": format!("/elements/{ub}")}]}),
    )
    .expect_err("refused");
    assert_eq!(fails[0].code, "ferro.model.pinned");
    assert_eq!(fails[0].elements, vec![bx, opt]);
    assert_eq!(snapshot(&f.s), before);
    // Moving in the option (edit in option) is refused.
    let r = f.s.execute_in_option(
        opt,
        &Combination::primary(),
        AnyCommand::from_tool(
            "move_elements",
            &json!({"elements": [w], "vector": p(0.0, 500.0, 0.0)}),
        )
        .expect("args"),
        &ExecContext::user(),
    );
    assert_eq!(r.expect_err("refused")[0].code, "ferro.model.pinned");
    assert_eq!(snapshot(&f.s), before);
    // Other overrides pass: height, name.
    f.s.execute_in_option(
        opt,
        &Combination::primary(),
        AnyCommand::from_tool("set_wall", &json!({"wall": w, "height": 2000})).expect("args"),
        &ExecContext::user(),
    )
    .expect("a height override is not a move");
    run(
        &mut f.s,
        "set_design_option",
        json!({"option": opt, "changes": [{"op": "set", "path": format!("/elements/{u}/name"), "value": "B"}]}),
    );
}

/// A layer that removed an element before it was pinned is not reported
/// again when the layer changes elsewhere.
#[test]
fn layers_made_before_the_pin_are_not_reported_again() {
    let mut f = px();
    let bx = f.bx;
    let o = run(&mut f.s, "create_option_set", json!({"name": "Scheme"}));
    let opt: ElementId = serde_json::from_value(o.output["option"].clone()).expect("option");
    let ub = uid(&f.s, bx);
    let uw = uid(&f.s, f.wall);
    let remove = json!({"op": "remove", "path": format!("/elements/{ub}")});
    run(
        &mut f.s,
        "set_design_option",
        json!({"option": opt, "changes": [remove.clone()]}),
    );
    pin(&mut f.s, &[bx]);
    run(
        &mut f.s,
        "set_design_option",
        json!({"option": opt, "changes": [remove, {"op": "set", "path": format!("/elements/{uw}/name"), "value": "B"}]}),
    );
}

/// No special unpin rule (ADR 0008 §3): agents and plugins pin and unpin
/// like the user.
#[test]
fn anyone_who_may_edit_may_unpin() {
    let mut f = px();
    let l = f.level;
    pin(&mut f.s, &[l]);
    let agent = ExecContext {
        actor: Actor::agent("bot", "m"),
        prompt: None,
        timestamp_ms: 0,
    };
    let plugin = ExecContext {
        actor: Actor {
            kind: ActorKind::Plugin { id: "acme".into() },
            name: "acme".into(),
        },
        prompt: None,
        timestamp_ms: 0,
    };
    for ctx in [&agent, &plugin] {
        let set = |pinned| -> AnyCommand {
            SetPinned {
                elements: vec![l],
                pinned,
            }
            .into()
        };
        f.s.execute(set(false), ctx).expect("unpin");
        assert!(!pinned(&f.s, l));
        f.s.execute(set(true), ctx).expect("pin");
        assert!(pinned(&f.s, l));
    }
}
