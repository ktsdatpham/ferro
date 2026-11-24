//! Joins (M5-5.2) through a session with `SolidcoreKernel`: exact volumes
//! of geometry joins (floor over a wall, a T stem that runs through),
//! frozen corner walls that keep their neighbours mitred, switch order,
//! tolerance edges, rotation and distance from the origin. Lengths in mm.

use ferro_core::model::{join, keys, ElementId, ParamValue};
use ferro_core::ops::{AnyCommand, ExecContext, Failure, Outcome, SequentialIds};
use ferro_core::{Document, Session};
use ferro_kernel::SolidcoreKernel;
use serde_json::{json, Value};
use std::sync::Arc;

fn session() -> Session {
    Session::new(
        Document::new("Joins"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    )
    .with_kernel(Arc::new(SolidcoreKernel::new()))
}

fn p(x: f64, y: f64, z: f64) -> Value {
    json!({"x": x, "y": y, "z": z})
}

fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    let cmd = AnyCommand::from_tool(tool, &args).unwrap_or_else(|m| panic!("{tool}: {m}"));
    s.execute(cmd, &ExecContext::user())
}

fn run(s: &mut Session, tool: &str, args: Value) -> Outcome {
    try_run(s, tool, args.clone()).unwrap_or_else(|f| panic!("{tool} {args}: {f:?}"))
}

fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).expect("element")
}

fn wall(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> ElementId {
    id(&run(
        s,
        "create_wall",
        json!({"start": p(a.0, a.1, 0.0), "end": p(b.0, b.1, 0.0)}),
    ))
}

/// A slab from `z0` up to `z1` over the plan rectangle `lo` – `hi`.
fn slab(s: &mut Session, lo: (f64, f64), hi: (f64, f64), z0: f64, z1: f64) -> ElementId {
    let f = id(&run(
        s,
        "create_floor",
        json!({"points": [{"x": lo.0, "y": lo.1}, {"x": hi.0, "y": lo.1}, {"x": hi.0, "y": hi.1}, {"x": lo.0, "y": hi.1}], "offset": z1}),
    ));
    // The default floor type is 200 thick: adjust with the offset only.
    assert!(((z1 - z0) - 200.0).abs() < 1e-9, "slabs are 200 mm thick");
    f
}

fn join_row(s: &mut Session, first: ElementId, second: ElementId) -> (String, Outcome) {
    let o = run(
        s,
        "create_relationship",
        json!({"kind": "join", "elements": [first, second]}),
    );
    (o.output["relationship"].as_str().unwrap().to_string(), o)
}

fn volume(s: &Session, e: ElementId) -> f64 {
    match s.doc().get(e).and_then(|x| x.param(keys::VOLUME)) {
        Some(ParamValue::Volume(v)) => *v,
        other => panic!("{e}: no volume ({other:?})"),
    }
}

fn geometry_volume(s: &Session, e: ElementId) -> f64 {
    let g = s.doc().get(e).and_then(|x| x.geometry()).expect("geometry");
    g.validate_solid().expect("a valid solid");
    g.volume()
}

fn assert_volume(got: f64, want: f64, what: &str) {
    assert!(
        ((got - want) / want).abs() < 1e-9,
        "{what}: volume {got}, expected {want}"
    );
}

const SECTION: f64 = 200.0 * 3000.0;
const WALL: f64 = 4000.0 * SECTION;
/// The part of a 200 thick, 3000 high wall under a 2000 long slab whose
/// bottom is 100 below the wall top.
const UNDER_SLAB: f64 = 2000.0 * 200.0 * 100.0;

#[test]
fn a_floor_joined_first_takes_the_exact_overlap_from_the_wall() {
    let mut s = session();
    let w = wall(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s, (1000.0, -1000.0), (3000.0, 1000.0), 2900.0, 3100.0);
    let floor_volume = volume(&s, f);
    let (row, o) = join_row(&mut s, f, w);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(volume(&s, w), WALL - UNDER_SLAB, "wall − floor");
    assert_volume(geometry_volume(&s, w), WALL - UNDER_SLAB, "geometry");
    assert_volume(volume(&s, f), floor_volume, "the floor keeps everything");
    // Switch join order: now the floor loses the overlap.
    let o = run(
        &mut s,
        "set_relationship",
        json!({"relationship": row, "reverse": true}),
    );
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(volume(&s, w), WALL, "the wall is whole");
    assert_volume(volume(&s, f), floor_volume - UNDER_SLAB, "floor − wall");
    // Unjoin restores both.
    run(&mut s, "delete_relationship", json!({"relationship": row}));
    assert_volume(volume(&s, w), WALL, "unjoined wall");
    assert_volume(volume(&s, f), floor_volume, "unjoined floor");
    s.undo().unwrap();
    assert_volume(volume(&s, f), floor_volume - UNDER_SLAB, "undo");
}

#[test]
fn a_t_stem_put_first_runs_through_and_notches_the_bar() {
    let mut s = session();
    let bar = wall(&mut s, (-4000.0, 0.0), (4000.0, 0.0));
    let stem = wall(&mut s, (0.0, 0.0), (0.0, 3000.0));
    assert_volume(volume(&s, stem), 2900.0 * SECTION, "default T");
    let (row, _) = join_row(&mut s, stem, bar);
    assert_volume(volume(&s, stem), 3100.0 * SECTION, "stem runs through");
    assert_volume(
        volume(&s, bar),
        8000.0 * SECTION - 200.0 * SECTION,
        "bar notched",
    );
    run(
        &mut s,
        "set_relationship",
        json!({"relationship": row, "reverse": true}),
    );
    assert_volume(volume(&s, stem), 2900.0 * SECTION, "back to the T");
    assert_volume(volume(&s, bar), 8000.0 * SECTION, "bar whole");
}

#[test]
fn freezing_a_corner_wall_keeps_both_walls_as_they_were() {
    for (name, butt) in [("mitre", false), ("butt", true)] {
        let mut s = session();
        let a = wall(&mut s, (-4000.0, 0.0), (0.0, 0.0));
        let b = wall(&mut s, (0.0, 0.0), (0.0, 4000.0));
        if butt {
            join_row(&mut s, b, a);
        }
        let (va, vb) = (volume(&s, a), volume(&s, b));
        let o = run(
            &mut s,
            "set_geometry_source",
            json!({"element": a, "source": "direct"}),
        );
        assert!(o.warnings.is_empty(), "{name}: {:?}", o.warnings);
        assert!(join::join_between(s.doc(), a, b).is_some(), "{name}: row");
        assert_volume(geometry_volume(&s, a), va, &format!("{name}: frozen wall"));
        assert_volume(geometry_volume(&s, b), vb, &format!("{name}: neighbour"));
        // Moving the neighbour away and back: it follows, then rejoins.
        run(
            &mut s,
            "set_wall",
            json!({"wall": b, "start": p(0.0, 1000.0, 0.0), "end": p(0.0, 4000.0, 0.0)}),
        );
        assert_volume(volume(&s, b), 3000.0 * SECTION, &format!("{name}: apart"));
        run(
            &mut s,
            "set_wall",
            json!({"wall": b, "start": p(0.0, 0.0, 0.0), "end": p(0.0, 4000.0, 0.0)}),
        );
        assert_volume(geometry_volume(&s, b), vb, &format!("{name}: rejoined"));
    }
}

#[test]
fn freezing_the_crossed_wall_of_a_t_keeps_the_stem_at_its_face() {
    let mut s = session();
    let bar = wall(&mut s, (-4000.0, 0.0), (4000.0, 0.0));
    let stem = wall(&mut s, (0.0, 0.0), (0.0, 3000.0));
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": bar, "source": "direct"}),
    );
    assert_volume(geometry_volume(&s, stem), 2900.0 * SECTION, "stem");
    assert_volume(geometry_volume(&s, bar), 8000.0 * SECTION, "bar");
}

#[test]
fn a_geometry_join_keeps_working_on_a_frozen_wall() {
    let mut s = session();
    let w = wall(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s, (1000.0, -1000.0), (3000.0, 1000.0), 2900.0, 3100.0);
    join_row(&mut s, f, w);
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": w, "source": "direct"}),
    );
    let e = s.doc().get(w).unwrap();
    assert_volume(e.as_form().unwrap().body.volume(), WALL, "drawn body");
    assert_volume(geometry_volume(&s, w), WALL - UNDER_SLAB, "joined Form");
    // Moving the slab moves the notch on the Form.
    run(
        &mut s,
        "move_elements",
        json!({"elements": [f], "vector": p(2000.0, 0.0, 0.0)}),
    );
    assert_volume(geometry_volume(&s, w), WALL - UNDER_SLAB / 2.0, "half over");
}

#[test]
fn touching_and_tolerance_edge_overlaps() {
    // Slab bottom `d` below the wall top: touching (0) and within
    // VERTEX_TOL change nothing; beyond it the sliver goes.
    for (d, removed) in [
        (0.0, 0.0),
        (0.005, 0.0),
        (0.01, 0.0),
        (0.02, 2000.0 * 200.0 * 0.02),
    ] {
        let mut s = session();
        let w = wall(&mut s, (0.0, 0.0), (4000.0, 0.0));
        let f = slab(
            &mut s,
            (1000.0, -1000.0),
            (3000.0, 1000.0),
            3000.0 - d,
            3200.0 - d,
        );
        let (_, o) = join_row(&mut s, f, w);
        assert!(o.warnings.is_empty(), "overlap {d}: {:?}", o.warnings);
        let got = volume(&s, w);
        if d == 0.01 {
            // Exactly VERTEX_TOL: float noise decides; either way exact.
            let sliver = 2000.0 * 200.0 * 0.01;
            assert!(
                ((got - WALL) / WALL).abs() < 1e-9 || ((got - WALL + sliver) / WALL).abs() < 1e-9,
                "overlap {d}: {got}"
            );
            continue;
        }
        assert_volume(got, WALL - removed, &format!("overlap {d}"));
    }
}

#[test]
fn rotated_and_far_from_the_origin() {
    let mut s = session();
    let w = wall(&mut s, (0.0, 0.0), (4000.0, 0.0));
    let f = slab(&mut s, (1000.0, -1000.0), (3000.0, 1000.0), 2900.0, 3100.0);
    join_row(&mut s, f, w);
    run(
        &mut s,
        "rotate_elements",
        json!({"elements": [w, f], "center": p(0.0, 0.0, 0.0), "angle": std::f64::consts::PI / 6.0}),
    );
    assert_volume(volume(&s, w), WALL - UNDER_SLAB, "rotated 30°");
    let o = run(
        &mut s,
        "move_elements",
        json!({"elements": [w, f], "vector": p(5.0e6, 0.0, 0.0)}),
    );
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    assert_volume(volume(&s, w), WALL - UNDER_SLAB, "rotated and 5 km away");
}
