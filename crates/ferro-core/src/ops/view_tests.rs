//! View commands, validation, delete rules and the change format on views
//! (M5-22).

use super::*;
use crate::math::tol;
use crate::model::change::{apply, diff};
use crate::model::{Category, DisplayStyle, Document, ElementId, PhaseFilter, View, ViewType};
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
    let f = try_run(s, tool, args.clone()).expect_err("should fail");
    assert_eq!(f[0].code, code, "{tool} {args}: {f:?}");
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

fn view(s: &Session, v: ElementId) -> View {
    s.doc().get(v).and_then(|e| e.as_view()).cloned().unwrap()
}

fn p(x: f64, y: f64, z: f64) -> Value {
    json!({"x": x, "y": y, "z": z})
}

fn plane(z: f64) -> Value {
    json!({"point": p(0.0, 0.0, z), "normal": p(0.0, -1.0, 0.0)})
}

#[test]
fn create_set_delete_views_with_undo_and_redo() {
    let mut s = session();
    let l1 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 3300}),
    ));
    let empty = s.doc().clone();
    let v3 = id(&run(
        &mut s,
        "create_view",
        json!({"name": "3D", "view_type": "view_3d"}),
    ));
    let plan = id(&run(
        &mut s,
        "create_view",
        json!({"name": "L1 plan", "view_type": "plan_view", "level": l1}),
    ));
    let sec = id(&run(
        &mut s,
        "create_view",
        json!({"name": "Section A", "view_type": "section_view", "section_plane": plane(0.0), "depth": 5000}),
    ));
    assert_eq!(s.doc().views().len(), 3);
    let pv = s.doc().get(plan).unwrap();
    assert_eq!(pv.category, Category::Views);
    assert_eq!(pv.level_id, Some(l1));
    let cut = s.doc().view_cut_plane(pv).unwrap();
    assert!(tol::eq_len(cut.point.z, 4500.0), "1200 above L1");
    assert_eq!(view(&s, plan).display_style, DisplayStyle::HiddenLine);
    assert_eq!(view(&s, v3).display_style, DisplayStyle::Shaded);
    assert!(
        matches!(view(&s, sec).view_type, ViewType::SectionView { depth, .. } if tol::eq_len(depth.mm(), 5000.0))
    );

    let before_set = s.doc().clone();
    run(
        &mut s,
        "set_view",
        json!({"view": v3, "name": "Iso", "display_style": "x_ray",
               "camera": {"eye": p(10.0, 10.0, 10.0), "target": p(0.0, 0.0, 0.0), "up": p(0.0, 0.0, 1.0), "projection": "parallel"},
               "section_plane": plane(1000.0),
               "section_box": {"min": p(0.0, 0.0, 0.0), "max": p(1.0, 1.0, 1.0)},
               "hidden_tags": [" b ", "a", "a"], "phase_filter": "show_complete"}),
    );
    let v = view(&s, v3);
    assert_eq!(s.doc().get(v3).unwrap().name.as_deref(), Some("Iso"));
    assert_eq!(v.display_style, DisplayStyle::XRay);
    assert_eq!(v.hidden_tags, ["a", "b"]);
    assert_eq!(v.phase_filter, PhaseFilter::ShowComplete);
    assert!(matches!(
        v.view_type,
        ViewType::View3D {
            section_plane: Some(_),
            section_box: Some(_)
        }
    ));
    // One undo step back, one redo step forward.
    let after_set = s.doc().clone();
    s.undo();
    assert_eq!(s.doc(), &before_set);
    s.redo();
    assert_eq!(s.doc(), &after_set);
    // `null` clears the 3D cut and box.
    run(
        &mut s,
        "set_view",
        json!({"view": v3, "section_plane": null, "section_box": null}),
    );
    assert!(matches!(
        view(&s, v3).view_type,
        ViewType::View3D {
            section_plane: None,
            section_box: None
        }
    ));
    run(
        &mut s,
        "set_view",
        json!({"view": plan, "view_range": {"top": 3000, "cut_height": 900, "bottom": 0, "view_depth": -100}}),
    );
    let cut = s.doc().view_cut_plane(s.doc().get(plan).unwrap()).unwrap();
    assert!(tol::eq_len(cut.point.z, 4200.0));
    run(&mut s, "delete_view", json!({"view": sec}));
    run(&mut s, "delete_elements", json!({"elements": [v3, plan]}));
    assert!(s.doc().views().is_empty());
    for _ in 0..8 {
        s.undo();
    }
    assert_eq!(s.doc(), &empty);
}

#[test]
fn hiding_and_unhiding_elements() {
    let mut s = session();
    let (a, b) = (cube(&mut s, 0.0), cube(&mut s, 2000.0));
    let v = id(&run(
        &mut s,
        "create_view",
        json!({"name": "3D", "view_type": "view_3d"}),
    ));
    let o = run(&mut s, "set_view", json!({"view": v, "hide": [b, a, b]}));
    assert_eq!(o.output["hidden_elements"], json!([a, b]), "sorted, unique");
    assert!(view(&s, v).hides(a));
    run(&mut s, "set_view", json!({"view": v, "unhide": [a, 999]}));
    assert_eq!(view(&s, v).hidden_elements, [b]);
    run(
        &mut s,
        "set_view",
        json!({"view": v, "unhide_all": true, "hide": [a]}),
    );
    assert_eq!(view(&s, v).hidden_elements, [a]);
}

#[test]
fn deleting_an_element_removes_it_from_every_view() {
    let mut s = session();
    let (a, b) = (cube(&mut s, 0.0), cube(&mut s, 2000.0));
    let mut views = Vec::new();
    for n in ["A", "B"] {
        let v = id(&run(
            &mut s,
            "create_view",
            json!({"name": n, "view_type": "view_3d"}),
        ));
        run(&mut s, "set_view", json!({"view": v, "hide": [a, b]}));
        views.push(v);
    }
    let before = s.doc().clone();
    let o = run(&mut s, "delete_elements", json!({"elements": [a]}));
    assert!(o.warnings.is_empty(), "silent clean-up: {:?}", o.warnings);
    for v in &views {
        assert_eq!(view(&s, *v).hidden_elements, [b]);
    }
    // The change format and files see no dangling reference.
    crate::model::change::element_value(s.doc(), s.doc().get(views[0]).unwrap()).unwrap();
    s.undo();
    assert_eq!(s.doc(), &before, "undo brings the hidden lists back");
}

#[test]
fn deleting_a_level_deletes_its_plan_views() {
    let mut s = session();
    let l1 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    let l2 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L2", "elevation": 3000}),
    ));
    let p1 = id(&run(
        &mut s,
        "create_view",
        json!({"name": "P1", "view_type": "plan_view", "level": l1}),
    ));
    let p2 = id(&run(
        &mut s,
        "create_view",
        json!({"name": "P2", "view_type": "plan_view", "level": l2}),
    ));
    let o = run(&mut s, "delete_elements", json!({"elements": [l1]}));
    assert!(o.changed.deleted.contains(&p1));
    assert!(!s.doc().contains(p1));
    assert!(s.doc().contains(p2));
}

#[test]
fn view_phase_follows_phase_deletion_rules() {
    let mut s = session();
    let ph1 = id(&run(&mut s, "create_phase", json!({"name": "Existing"})));
    let ph2 = id(&run(&mut s, "create_phase", json!({"name": "New"})));
    let ph3 = id(&run(&mut s, "create_phase", json!({"name": "Later"})));
    let v = id(&run(
        &mut s,
        "create_view",
        json!({"name": "3D", "view_type": "view_3d", "phase": ph2}),
    ));
    assert_eq!(view(&s, v).phase, Some(ph2));
    // A phase used only by views is not "in use": deleting it clears them.
    run(&mut s, "delete_phase", json!({"phase": ph3}));
    run(&mut s, "set_view", json!({"view": v, "phase": ph1}));
    run(&mut s, "delete_elements", json!({"elements": [ph1]}));
    assert_eq!(view(&s, v).phase, None, "cleared: shows the last phase");
    // Merging moves views to the phase merged into.
    let ph4 = id(&run(&mut s, "create_phase", json!({"name": "Final"})));
    run(&mut s, "set_view", json!({"view": v, "phase": ph4}));
    run(
        &mut s,
        "delete_phase",
        json!({"phase": ph4, "merge_into": ph2}),
    );
    assert_eq!(view(&s, v).phase, Some(ph2));
    run(&mut s, "set_view", json!({"view": v, "phase": null}));
    assert_eq!(view(&s, v).phase, None);
}

#[test]
fn invalid_input_fails_with_documented_codes() {
    let mut s = session();
    let l1 = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    let bx = cube(&mut s, 0.0);
    let mat = id(&run(
        &mut s,
        "create_material",
        json!({"name": "Brick", "color": "#aa5533"}),
    ));
    let ph = id(&run(&mut s, "create_phase", json!({"name": "P"})));
    let v = id(&run(
        &mut s,
        "create_view",
        json!({"name": "3D", "view_type": "view_3d"}),
    ));
    let plan = id(&run(
        &mut s,
        "create_view",
        json!({"name": "Plan", "view_type": "plan_view", "level": l1}),
    ));
    let sec = id(&run(
        &mut s,
        "create_view",
        json!({"name": "Sec", "view_type": "section_view", "section_plane": plane(0.0)}),
    ));
    let mk = |extra: Value| {
        let mut a = json!({"name": "New", "view_type": "view_3d"});
        for (k, x) in extra.as_object().unwrap() {
            a[k] = x.clone();
        }
        a
    };
    let cam =
        |eye: Value, target: Value, up: Value| json!({"eye": eye, "target": target, "up": up});
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "create_view",
            mk(json!({"name": "3D"})),
            "ferro.model.duplicate_name",
        ),
        (
            "create_view",
            mk(json!({"name": "  "})),
            "ferro.model.invalid_name",
        ),
        (
            "create_view",
            mk(json!({"view_type": "plan_view"})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"view_type": "plan_view", "level": bx})),
            "ferro.model.not_a_level",
        ),
        (
            "create_view",
            mk(json!({"view_type": "plan_view", "level": 999})),
            "ferro.model.not_found",
        ),
        (
            "create_view",
            mk(json!({"view_type": "section_view"})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"level": l1})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"depth": 100})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"view_type": "plan_view", "level": l1, "section_plane": plane(0.0)})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"view_type": "section_view", "section_plane": plane(0.0), "depth": 0})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(
                json!({"view_type": "section_view", "section_plane": plane(0.0), "depth": tol::SHORT_CURVE_TOL}),
            ),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"camera": cam(p(1.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 0.0, 1.0))})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"camera": cam(p(0.0, 0.0, 10.0), p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0))})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"section_box": {"min": p(0.0, 0.0, 0.0), "max": p(1.0, 1.0, 0.0)}})),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(
                json!({"view_type": "plan_view", "level": l1, "view_range": {"top": 1000, "cut_height": 1200, "bottom": 0, "view_depth": 0}}),
            ),
            "ferro.model.invalid_view",
        ),
        (
            "create_view",
            mk(json!({"phase": bx})),
            "ferro.model.not_a_phase",
        ),
        (
            "set_view",
            json!({"view": bx, "name": "x"}),
            "ferro.model.not_a_view",
        ),
        ("set_view", json!({"view": 999}), "ferro.model.not_found"),
        (
            "set_view",
            json!({"view": v, "name": "Plan"}),
            "ferro.model.duplicate_name",
        ),
        (
            "set_view",
            json!({"view": v, "hide": [mat]}),
            "ferro.model.not_hideable",
        ),
        (
            "set_view",
            json!({"view": v, "hide": [ph]}),
            "ferro.model.not_hideable",
        ),
        (
            "set_view",
            json!({"view": v, "hide": [plan]}),
            "ferro.model.not_hideable",
        ),
        (
            "set_view",
            json!({"view": v, "hide": [999]}),
            "ferro.model.not_found",
        ),
        (
            "set_view",
            json!({"view": v, "hidden_tags": ["ok", " "]}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": v, "phase": mat}),
            "ferro.model.not_a_phase",
        ),
        (
            "set_view",
            json!({"view": v, "view_range": {"top": 2000, "cut_height": 1000, "bottom": 0, "view_depth": 0}}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": v, "depth": 1000}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": plan, "section_plane": plane(0.0)}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": plan, "section_box": null}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": sec, "section_plane": null}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": sec, "depth": -5}),
            "ferro.model.invalid_view",
        ),
        (
            "set_view",
            json!({"view": plan, "view_range": {"top": 0, "cut_height": 1000, "bottom": 0, "view_depth": 0}}),
            "ferro.model.invalid_view",
        ),
        ("delete_view", json!({"view": bx}), "ferro.model.not_a_view"),
        ("delete_view", json!({"view": 999}), "ferro.model.not_found"),
    ];
    for (tool, args, code) in cases {
        fails(&mut s, tool, args, code);
    }
    // Degenerate JSON never reaches the kernel: a zero normal is refused
    // while reading the arguments.
    assert!(AnyCommand::from_tool(
        "set_view",
        &json!({"view": sec, "section_plane": {"point": p(0.0, 0.0, 0.0), "normal": p(0.0, 0.0, 0.0)}})
    )
    .is_err());
}

#[test]
fn tolerance_edges_accept_just_above_the_limit() {
    let mut s = session();
    // Depth just above SHORT_CURVE_TOL; camera eye and target just over
    // VERTEX_TOL apart.
    run(
        &mut s,
        "create_view",
        json!({"name": "S", "view_type": "section_view", "section_plane": plane(0.0), "depth": tol::SHORT_CURVE_TOL * 1.01}),
    );
    let d = tol::VERTEX_TOL * 2.0;
    run(
        &mut s,
        "create_view",
        json!({"name": "C", "view_type": "view_3d", "camera": {"eye": p(0.0, 0.0, d), "target": p(0.0, 0.0, 0.0), "up": p(0.0, 1.0, 0.0)}}),
    );
    fails(
        &mut s,
        "create_view",
        json!({"name": "C2", "view_type": "view_3d", "camera": {"eye": p(0.0, 0.0, tol::VERTEX_TOL * 0.5), "target": p(0.0, 0.0, 0.0), "up": p(0.0, 1.0, 0.0)}}),
        "ferro.model.invalid_view",
    );
    // View range values equal within VERTEX_TOL count as ordered.
    let l = id(&run(
        &mut s,
        "create_level",
        json!({"name": "L", "elevation": 0}),
    ));
    run(
        &mut s,
        "create_view",
        json!({"name": "P", "view_type": "plan_view", "level": l,
               "view_range": {"top": 1200, "cut_height": 1200.0 + tol::VERTEX_TOL * 0.5, "bottom": 0, "view_depth": 0}}),
    );
}

#[test]
fn change_format_round_trips_views_with_hidden_lists() {
    let mut s = session();
    let (a, b) = (cube(&mut s, 0.0), cube(&mut s, 2000.0));
    let ph = id(&run(&mut s, "create_phase", json!({"name": "P"})));
    let before = s.doc().clone();
    let v = id(&run(
        &mut s,
        "create_view",
        json!({"name": "3D", "view_type": "view_3d", "phase": ph}),
    ));
    run(&mut s, "set_view", json!({"view": v, "hide": [a]}));
    let mid = s.doc().clone();
    let d = diff(&before, &mid).unwrap();
    let back = apply(&before, &d).unwrap();
    assert_eq!(back.views().len(), 1);
    let bv = back.views()[0].as_view().cloned().unwrap();
    assert_eq!(bv, view(&s, v), "phase and hidden list kept (same ids)");
    // References are written as uids in the change.
    let text = serde_json::to_string(&d).unwrap();
    let uid_a = s.doc().get(a).unwrap().uid.to_string();
    assert!(text.contains(&uid_a), "hidden element as uid: {text}");
    // A property-level edit of the hidden list.
    run(&mut s, "set_view", json!({"view": v, "hide": [b]}));
    let after = s.doc().clone();
    let d2 = diff(&mid, &after).unwrap();
    assert_eq!(d2.len(), 1, "{d2:?}");
    assert!(d2[0].path().ends_with("/body/hidden_elements"), "{d2:?}");
    assert_eq!(
        apply(&mid, &d2).unwrap().views()[0].as_view(),
        after.views()[0].as_view()
    );
}
