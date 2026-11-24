//! Save / load round trips and export determinism (M5-15.3) on a document
//! holding every element kind: levels, Forms (imprinted, pushed, circle),
//! materials with assets, wall and floor types with layers, straight / arc /
//! chained walls with joins, floors with holes, doors, windows, a frozen
//! wall, a classified Form, parameters of every type and a parameter
//! definition.

use ferro_core::model::{generate, Document, Element, ElementBody, ElementId};
use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
use ferro_core::{Session, Vec3};
use ferro_io::{
    document_from_json, document_to_json, export_glb, export_ifc, export_obj, load_fro, save_fro,
};
use serde_json::{json, Value};

fn p(x: f64, y: f64, z: f64) -> Value {
    json!({"x": x, "y": y, "z": z})
}

fn run(s: &mut Session, tool: &str, args: Value) -> Value {
    let cmd = AnyCommand::from_tool(tool, &args).unwrap_or_else(|m| panic!("{tool}: {m}"));
    s.execute(cmd, &ExecContext::user())
        .unwrap_or_else(|f| panic!("{tool} {args}: {f:?}"))
        .output
}

fn el(o: &Value) -> u64 {
    o["element"].as_u64().expect("element id")
}

fn face_towards(s: &Session, e: u64, dir: Vec3) -> Value {
    let b = s
        .doc()
        .get(ElementId(e))
        .and_then(|x| x.geometry())
        .expect("geometry");
    b.face_ids()
        .max_by(|x, y| {
            // Normal closest to `dir`, then the outermost along `dir`.
            let score = |f: ferro_core::topo::FaceId| {
                let n = b.face_normal(f).expect("n").as_vec().dot(dir);
                let c = ferro_core::Point3::centroid(&b.face_points(f).expect("pts"))
                    .expect("centroid");
                (n * 1000.0).round() * 1e9 + c.to_vec().dot(dir)
            };
            score(*x).total_cmp(&score(*y))
        })
        .map(|f| json!(f))
        .expect("face")
}

/// Every element kind. With `gaps`, some elements are deleted on the way so
/// ids are not contiguous (files renumber them on load).
fn house(gaps: bool) -> Session {
    let mut s = Session::new(
        Document::new("Nhà phố — round trip"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    );
    let l1 = el(&run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    ));
    if gaps {
        let scratch = el(&run(
            &mut s,
            "create_level",
            json!({"name": "tmp", "elevation": 99}),
        ));
        run(&mut s, "delete_elements", json!({"elements": [scratch]}));
    }
    let l2 = el(&run(
        &mut s,
        "create_level",
        json!({"name": "L2", "elevation": 3200}),
    ));
    let brick = el(&run(
        &mut s,
        "create_material",
        json!({"name": "Brick", "color": "#aa5533", "roughness": 0.6,
               "thermal": {"conductivity_w_mk": 0.77, "specific_heat_j_kgk": 840}}),
    ));
    let ins = el(&run(
        &mut s,
        "create_material",
        json!({"name": "Mineral wool", "color": "#e0d080"}),
    ));
    let wt = el(&run(
        &mut s,
        "create_wall_type",
        json!({"name": "Cavity 300", "layers": [
            {"function": "finish", "thickness": 20},
            {"function": "structure", "thickness": 110, "material": brick},
            {"function": "insulation", "thickness": 60, "material": ins},
            {"function": "structure", "thickness": 110, "material": brick}]}),
    ));
    let ft = el(&run(
        &mut s,
        "create_floor_type",
        json!({"name": "Slab 250", "layers": [
            {"function": "finish", "thickness": 50},
            {"function": "structure", "thickness": 200, "material": brick}]}),
    ));
    if gaps {
        let b = el(&run(
            &mut s,
            "create_box",
            json!({"origin": p(0.0, -9000.0, 0.0), "width": 10, "depth": 10, "height": 10}),
        ));
        run(&mut s, "delete_elements", json!({"elements": [b]}));
    }
    // A closed chain of four mitred walls, top attached to L2.
    let chain = run(
        &mut s,
        "create_walls",
        json!({"points": [p(0.0,0.0,0.0), p(0.0,8000.0,0.0), p(6000.0,8000.0,0.0), p(6000.0,0.0,0.0)],
               "closed": true, "level": l1, "top_level": l2, "wall_type": wt}),
    );
    let south = chain["elements"][3].as_u64().expect("wall");
    run(
        &mut s,
        "create_opening",
        json!({"wall": south, "kind": "door", "position": 2000, "name": "Front door"}),
    );
    run(
        &mut s,
        "create_opening",
        json!({"wall": south, "kind": "window", "position": 4300, "width": 900}),
    );
    // A tee wall and a frozen wall with a window.
    run(
        &mut s,
        "create_wall",
        json!({"start": p(3000.0, 0.0, 0.0), "end": p(3000.0, 8000.0, 0.0), "level": l1, "height": 2800}),
    );
    let frozen = el(&run(
        &mut s,
        "create_wall",
        json!({"start": p(10_000.0, 0.0, 0.0), "end": p(14_000.0, 0.0, 0.0), "wall_type": wt, "base_offset": 100}),
    ));
    run(
        &mut s,
        "create_opening",
        json!({"wall": frozen, "kind": "window", "position": 2000}),
    );
    run(
        &mut s,
        "set_geometry_source",
        json!({"element": frozen, "source": "direct"}),
    );
    // An arc wall on L2.
    run(
        &mut s,
        "create_arc_wall",
        json!({"start": p(0.0, 12_000.0, 0.0), "mid": p(3000.0, 13_500.0, 0.0), "end": p(6000.0, 12_000.0, 0.0), "level": l2, "height": 2500}),
    );
    // Floors with a hole, on two levels.
    run(
        &mut s,
        "create_floor",
        json!({"points": [{"x":0,"y":0},{"x":6000,"y":0},{"x":6000,"y":8000},{"x":0,"y":8000}],
               "holes": [[{"x":1000,"y":5000},{"x":2000,"y":5000},{"x":2000,"y":7000},{"x":1000,"y":7000}]],
               "level": l1, "floor_type": ft, "name": "Ground slab"}),
    );
    run(
        &mut s,
        "create_floor",
        json!({"points": [{"x":0,"y":0},{"x":6000,"y":0},{"x":6000,"y":8000}], "level": l2, "offset": 50}),
    );
    // Forms: imprinted + pushed through, a pocket with a circle, a sheet.
    let bx = el(&run(
        &mut s,
        "create_box",
        json!({"origin": p(20_000.0, 0.0, 0.0), "width": 4000, "depth": 3000, "height": 2000, "level": l1, "name": "Block"}),
    ));
    let top = face_towards(&s, bx, Vec3::Z);
    let o = run(
        &mut s,
        "imprint_face",
        json!({"element": bx, "face": top, "closed": true, "points": [
            p(21_000.0,1000.0,2000.0), p(22_000.0,1000.0,2000.0), p(22_000.0,2000.0,2000.0), p(21_000.0,2000.0,2000.0)]}),
    );
    let hole = o["new_faces"][0].clone();
    run(
        &mut s,
        "push_pull_face",
        json!({"element": bx, "face": hole, "distance": -2000}),
    );
    let front = face_towards(&s, bx, Vec3::new(0.0, -1.0, 0.0));
    let o = run(
        &mut s,
        "create_circle",
        json!({"element": bx, "face": front, "center": p(23_000.0, 0.0, 1000.0), "radius": 400}),
    );
    let disc = o["new_faces"][0].clone();
    run(
        &mut s,
        "push_pull_face",
        json!({"element": bx, "face": disc, "distance": -300}),
    );
    run(
        &mut s,
        "rotate_elements",
        json!({"elements": [bx], "center": p(20_000.0, 0.0, 0.0), "angle": 30}),
    );
    run(
        &mut s,
        "create_circle",
        json!({"center": p(30_000.0, 0.0, 0.0), "radius": 600, "segments": 32, "name": "Disc"}),
    );
    // A Form classified as a floor with a floor type.
    let slab = el(&run(
        &mut s,
        "create_box",
        json!({"origin": p(0.0, 20_000.0, 0.0), "width": 3000, "depth": 3000, "height": 250}),
    ));
    run(
        &mut s,
        "set_category",
        json!({"element": slab, "category": "floors"}),
    );
    run(
        &mut s,
        "set_element_type",
        json!({"element": slab, "element_type": ft}),
    );
    // Parameters of every type and a definition.
    run(
        &mut s,
        "set_parameter_definition",
        json!({"key": "acme.fire.rating", "data_type": "text", "description": "Fire rating",
               "ifc": {"property_set": "Pset_WallCommon", "property": "FireRating"}}),
    );
    for (key, value) in [
        (
            "acme.fire.rating",
            json!({"type": "text", "value": "EI 60"}),
        ),
        ("acme.notes.count", json!({"type": "integer", "value": -3})),
        ("acme.notes.flag", json!({"type": "bool", "value": true})),
        (
            "acme.notes.tags",
            json!({"type": "tags", "value": ["phase-1", "client"]}),
        ),
        (
            "acme.notes.ratio",
            json!({"type": "number", "value": 0.125}),
        ),
        ("acme.notes.len", json!({"type": "length", "value": 1234.5})),
        ("acme.notes.angle", json!({"type": "angle", "value": 0.5})),
        (
            "acme.notes.ref",
            json!({"type": "element_ref", "value": bx}),
        ),
        (
            "acme.notes.density",
            json!({"type": "quantity", "value": {"spec": "density", "value": 1800.0}}),
        ),
    ] {
        run(
            &mut s,
            "set_parameter",
            json!({"element": south, "key": key, "value": value}),
        );
    }
    run(
        &mut s,
        "set_name",
        json!({"element": south, "name": "Tường nam"}),
    );
    // Phases (M5-23): the two defaults from the simple mode plus a dated
    // stage; the south wall is demolished, the box is new.
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [south], "status": "demolish"}),
    );
    run(
        &mut s,
        "set_renovation_status",
        json!({"elements": [bx], "status": "new"}),
    );
    run(
        &mut s,
        "create_phase",
        json!({"name": "Hoàn thiện", "start_date": "2027-01-15", "end_date": "2027-06-30"}),
    );
    // Views (M5-22): a 3D view hiding two elements in a phase, a plan of
    // L1 and a section.
    let first_phase = s.doc().phases()[0].id;
    let v3 = el(&run(
        &mut s,
        "create_view",
        json!({"name": "3D phối cảnh", "view_type": "view_3d", "phase": first_phase, "phase_filter": "show_previous_and_demo",
               "section_plane": {"point": p(0.0, 0.0, 1500.0), "normal": p(0.0, 0.0, 1.0)}}),
    ));
    run(
        &mut s,
        "set_view",
        json!({"view": v3, "hide": [bx, south], "hidden_tags": ["client"], "display_style": "x_ray"}),
    );
    run(
        &mut s,
        "create_view",
        json!({"name": "Mặt bằng L1", "view_type": "plan_view", "level": l1}),
    );
    run(
        &mut s,
        "create_view",
        json!({"name": "Mặt cắt A", "view_type": "section_view", "section_plane": {"point": p(0.0, 2000.0, 0.0), "normal": p(0.0, -1.0, 0.0)}, "depth": 8000}),
    );
    // Design options (M5-23.9): set "Mái" with A (primary: adds a
    // pavilion) and B (renames the south wall, adds another pavilion).
    let o = run(&mut s, "create_option_set", json!({"name": "Mái"}));
    let a = ElementId(o["option"].as_u64().expect("option"));
    let b = ElementId(el(&run(
        &mut s,
        "create_design_option",
        json!({"set": el(&o), "name": "B"}),
    )));
    in_option(
        &mut s,
        a,
        "create_box",
        json!({"origin": p(0.0, 20_000.0, 0.0), "width": 3000, "depth": 3000, "height": 3000, "name": "Pavilion A"}),
    );
    in_option(
        &mut s,
        b,
        "create_box",
        json!({"origin": p(0.0, 20_000.0, 0.0), "width": 4000, "depth": 3000, "height": 3000, "name": "Pavilion B"}),
    );
    in_option(
        &mut s,
        b,
        "set_name",
        json!({"element": south, "name": "South wall B"}),
    );
    s
}

fn in_option(s: &mut Session, option: ElementId, tool: &str, args: Value) {
    let cmd = AnyCommand::from_tool(tool, &args).unwrap_or_else(|m| panic!("{tool}: {m}"));
    s.execute_in_option(
        option,
        &ferro_core::model::Combination::primary(),
        cmd,
        &ExecContext::user(),
    )
    .unwrap_or_else(|f| panic!("{tool} in option: {f:?}"));
}

/// Element as JSON with every element reference replaced by the target's
/// uid, so documents whose ids were renumbered compare equal.
fn normalised(doc: &Document, e: &Element) -> Value {
    let uid = |id: &Value| -> Value {
        id.as_u64()
            .and_then(|i| doc.get(ElementId(i)))
            .map(|t| json!(t.uid.to_string()))
            .unwrap_or(Value::Null)
    };
    let mut v = serde_json::to_value(e).expect("element");
    let o = v.as_object_mut().expect("object");
    o.remove("id");
    for f in [
        "level_id",
        "type_id",
        "host_id",
        "design_option",
        "phase_created",
        "phase_demolished",
    ] {
        if let Some(x) = o.get(f).cloned() {
            o.insert(f.into(), uid(&x));
        }
    }
    if let Some(Value::Object(params)) = o.get_mut("params") {
        for pv in params.values_mut() {
            if pv["type"] == "element_ref" {
                pv["value"] = uid(&pv["value"]);
            }
        }
    }
    if let Some(Value::Object(body)) = o.get_mut("body") {
        if let Some(l) = body.get_mut("top").and_then(|t| t.get_mut("level")) {
            *l = uid(l);
        }
        if body.get("kind").and_then(Value::as_str) == Some("view") {
            if let Some(ph) = body.get_mut("phase") {
                *ph = uid(ph);
            }
            for list in ["hidden_elements", "design_options"] {
                if let Some(Value::Array(ids)) = body.get_mut(list) {
                    let mut u: Vec<Value> = ids.iter().map(&uid).collect();
                    u.sort_by_key(|x| x.to_string());
                    *ids = u;
                }
            }
        }
        if let Some(Value::Array(ls)) = body
            .get_mut("structure")
            .and_then(|st| st.get_mut("layers"))
        {
            for l in ls {
                if let Some(m) = l.get_mut("material") {
                    *m = uid(m);
                }
            }
        }
    }
    v
}

fn assert_same(a: &Document, b: &Document, what: &str) {
    assert_eq!(a.project(), b.project(), "{what}: project");
    assert_eq!(a.len(), b.len(), "{what}: element count");
    for e in a.elements() {
        let f = b
            .find_uid(e.uid)
            .unwrap_or_else(|| panic!("{what}: {} missing", e.label()));
        assert_eq!(normalised(a, e), normalised(b, f), "{what}: {}", e.label());
    }
}

/// A loaded document is consistent: regenerating it changes nothing, and
/// it accepts new commands without id clashes.
fn assert_usable(doc: Document, what: &str) {
    let regen = generate::regenerate(&doc);
    assert!(
        regen.updates.is_empty(),
        "{what}: stored geometry differs from regenerated geometry for {:?}",
        regen.updates.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );
    let n = doc.len();
    let mut s = Session::new(doc, Box::new(SequentialIds::new(1_800_000_000_000)));
    run(
        &mut s,
        "create_wall",
        json!({"start": p(0.0, -5000.0, 0.0), "end": p(4000.0, -5000.0, 0.0)}),
    );
    assert!(s.doc().len() > n, "{what}");
    let ids: std::collections::HashSet<_> = s.doc().elements().map(|e| e.uid).collect();
    assert_eq!(ids.len(), s.doc().len(), "{what}: uid clash");
}

#[test]
fn every_element_kind_is_present() {
    let s = house(true);
    let has = |f: fn(&Element) -> bool| s.doc().elements().any(f);
    assert!(has(|e| matches!(e.body, ElementBody::Level(_))));
    assert!(has(|e| matches!(e.body, ElementBody::Material(_))));
    assert!(has(|e| matches!(e.body, ElementBody::WallType(_))));
    assert!(has(|e| matches!(e.body, ElementBody::FloorType(_))));
    assert!(has(|e| matches!(e.body, ElementBody::Wall(_))));
    assert!(has(|e| matches!(e.body, ElementBody::Floor(_))));
    assert!(has(|e| matches!(e.body, ElementBody::Opening(_))));
    assert!(has(
        |e| e.as_form().is_some() && e.category == ferro_core::Category::Walls
    ));
    assert!(has(
        |e| e.as_form().is_some() && e.category == ferro_core::Category::Floors
    ));
    assert!(has(
        |e| e.as_form().is_some() && e.category == ferro_core::Category::Windows
    ));
    assert!(!s.doc().project().parameter_definitions.is_empty());
    assert!(has(|e| matches!(e.body, ElementBody::Phase(_))));
    assert!(has(|e| e.phase_demolished.is_some()));
    assert!(has(|e| e.as_view().is_some_and(|v| v
        .hidden_elements
        .len()
        == 2
        && v.phase.is_some())));
    assert_eq!(s.doc().views().len(), 3);
    assert!(has(|e| matches!(e.body, ElementBody::OptionSet(_))));
    assert!(has(|e| e
        .as_design_option()
        .is_some_and(|o| o.changes.len() >= 2)));
}

/// IFC shows the primary combination: option A's pavilion, not B's; option
/// sets themselves are not exported.
#[test]
fn ifc_exports_the_primary_combination() {
    let s = house(false);
    let ifc = export_ifc(s.doc()).expect("ifc");
    assert!(ifc.contains("'Pavilion A'"), "primary option exported");
    assert!(!ifc.contains("Pavilion B"), "other options left out");
    assert!(!ifc.contains("South wall B"), "B's rename left out");
    // Another combination on request.
    let b = s
        .doc()
        .elements()
        .find(|e| e.name.as_deref() == Some("B"))
        .expect("option B")
        .id;
    let combo = ferro_core::model::Combination { options: vec![b] };
    let other = ferro_io::export_ifc_combination(s.doc(), &combo, None).expect("ifc");
    assert!(other.contains("'Pavilion B'") && !other.contains("Pavilion A"));
}

#[test]
fn json_roundtrip_keeps_every_element() {
    for gaps in [false, true] {
        let s = house(gaps);
        let text = document_to_json(s.doc()).expect("save");
        let back = document_from_json(&text).expect("load");
        assert_same(s.doc(), &back, &format!("json gaps={gaps}"));
        // Saving the loaded document again gives the same text.
        assert_eq!(
            document_to_json(&back).expect("save again"),
            text,
            "gaps={gaps}"
        );
        assert_usable(back, "json");
    }
}

#[test]
fn json_roundtrip_without_gaps_is_exact() {
    let s = house(false);
    let back = document_from_json(&document_to_json(s.doc()).expect("save")).expect("load");
    for e in s.doc().elements() {
        assert_eq!(back.get(e.id), Some(e), "{}", e.label());
    }
}

#[test]
fn fro_roundtrip_keeps_every_element_and_the_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    for gaps in [false, true] {
        let s = house(gaps);
        let path = dir.path().join(format!("house-{gaps}.fro"));
        save_fro(&path, s.doc(), s.journal()).expect("save");
        let (back, journal) = load_fro(&path).expect("load");
        assert_same(s.doc(), &back, &format!("fro gaps={gaps}"));
        assert_eq!(journal, s.journal(), "journal");
        // Save over the same file and load again: stable.
        save_fro(&path, &back, &journal).expect("save again");
        let (again, _) = load_fro(&path).expect("load again");
        assert_same(&back, &again, "fro second round");
        // .fro and .fro.json agree.
        let via_json = document_from_json(&document_to_json(s.doc()).expect("json")).expect("json");
        assert_same(&back, &via_json, "fro vs json");
        assert_usable(back, "fro");
    }
}

#[test]
fn exports_are_deterministic_and_survive_a_round_trip() {
    let s = house(true);
    let ifc = export_ifc(s.doc()).expect("ifc");
    assert_eq!(
        export_ifc(s.doc()).expect("ifc"),
        ifc,
        "IFC export is not deterministic"
    );
    assert!(ifc.starts_with("ISO-10303-21;"));
    for entity in [
        "IFCWALL",
        "IFCSLAB",
        "IFCDOOR",
        "IFCWINDOW",
        "IFCBUILDINGSTOREY",
        "IFCMATERIALLAYERSET",
    ] {
        assert!(ifc.contains(entity), "IFC has no {entity}");
    }
    assert_eq!(
        export_obj(s.doc()).expect("obj"),
        export_obj(s.doc()).expect("obj")
    );
    assert_eq!(
        export_glb(s.doc()).expect("glb"),
        export_glb(s.doc()).expect("glb")
    );
}

/// A document read back from a file exports the same IFC (GUIDs come from
/// uids, not from local ids).
#[test]
#[ignore = "bug: unnamed elements are exported with Name 'walls #<local id>', which changes when a file renumbers ids on load (M5-15.9)"]
fn ifc_export_is_stable_across_a_round_trip() {
    let s = house(true);
    let ifc = export_ifc(s.doc()).expect("ifc");
    let back = document_from_json(&document_to_json(s.doc()).expect("save")).expect("load");
    let again = export_ifc(&back).expect("ifc");
    let diff: Vec<String> = ifc
        .lines()
        .zip(again.lines())
        .filter(|(a, b)| a != b)
        .take(4)
        .map(|(a, b)| format!("before: {a}\nafter:  {b}"))
        .collect();
    assert!(
        diff.is_empty(),
        "IFC differs after a round trip:\n{}",
        diff.join("\n")
    );
}

#[test]
fn corrupt_and_foreign_files_are_clean_errors() {
    for text in [
        "",
        "null",
        "[]",
        "{\"format\":\"ferro\"}",
        "{\"format\":\"ferro\",\"format_version\":1,\"elements\":[42]}",
    ] {
        assert!(document_from_json(text).is_err(), "{text:?} loaded");
    }
    // A dangling reference is reported, not dropped.
    let s = house(false);
    let mut v: Value =
        serde_json::from_str(&document_to_json(s.doc()).expect("save")).expect("json");
    let els = v["elements"].as_array_mut().expect("elements");
    let lvl = els
        .iter()
        .position(|e| e["category"] == "levels")
        .expect("a level");
    els.remove(lvl);
    let e = document_from_json(&v.to_string()).expect_err("dangling level accepted");
    assert_eq!(e.code, "ferro.io.dangling_reference");
    let dir = tempfile::tempdir().expect("tempdir");
    let junk = dir.path().join("junk.fro");
    std::fs::write(&junk, b"not a database").expect("write");
    assert!(load_fro(&junk).is_err());
    assert_eq!(
        load_fro(&dir.path().join("missing.fro"))
            .expect_err("missing")
            .code,
        "ferro.io.file"
    );
}

/// A stand-in kernel whose difference is the cutter itself: enough to give
/// a Form a `cut_body` that differs from its source (M5-5.3).
struct CutterKernel;

impl ferro_core::algo::Kernel for CutterKernel {
    fn name(&self) -> &'static str {
        "cutter-test"
    }
    fn boolean(
        &self,
        _op: ferro_core::algo::BooleanOp,
        _a: &ferro_core::topo::Body,
        b: &ferro_core::topo::Body,
    ) -> Result<ferro_core::algo::KernelResult, ferro_core::algo::KernelError> {
        Ok(ferro_core::algo::KernelResult {
            body: b.clone(),
            faces: Default::default(),
            lineage: Default::default(),
        })
    }
}

#[test]
fn cut_rows_and_cut_bodies_survive_json_and_fro() {
    let mut s = Session::new(
        Document::new("Cuts"),
        Box::new(SequentialIds::new(1_800_000_000_000)),
    )
    .with_kernel(std::sync::Arc::new(CutterKernel));
    let a = el(&run(
        &mut s,
        "create_box",
        json!({"origin": p(0.0, 0.0, 0.0), "width": 1000, "depth": 1000, "height": 1000}),
    ));
    let c = el(&run(
        &mut s,
        "create_box",
        json!({"origin": p(200.0, 200.0, 600.0), "width": 300, "depth": 300, "height": 600}),
    ));
    let w = el(&run(
        &mut s,
        "create_wall",
        json!({"start": p(0.0, 3000.0, 0.0), "end": p(4000.0, 3000.0, 0.0)}),
    ));
    run(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [c, a]}),
    );
    run(
        &mut s,
        "create_relationship",
        json!({"kind": "cut", "elements": [c, w]}),
    );
    let form = s
        .doc()
        .get(ElementId(a))
        .unwrap()
        .as_form()
        .unwrap()
        .clone();
    assert!(form.cut_body.is_some());
    let back = document_from_json(&document_to_json(s.doc()).expect("save")).expect("load");
    assert_same(s.doc(), &back, "json with cuts");
    assert_eq!(back.relationships().count(), 2);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("cuts.fro");
    save_fro(&path, s.doc(), s.journal()).expect("save");
    let (fro, _) = load_fro(&path).expect("load");
    assert_same(s.doc(), &fro, "fro with cuts");
    let uid = s.doc().get(ElementId(a)).unwrap().uid;
    let b = fro.find_uid(uid).unwrap();
    assert_eq!(b.as_form().unwrap().cut_body, form.cut_body);
    // Regenerating with the same kernel changes nothing.
    let regen = generate::regenerate_with(&fro, Some(&CutterKernel));
    assert!(regen.updates.is_empty() && regen.cut_updates.is_empty());
}
