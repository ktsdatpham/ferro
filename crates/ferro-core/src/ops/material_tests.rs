//! Tests of material assets and type quantities through the command path.

use super::*;
use crate::model::material::{Material, MaterialUnit};
use crate::model::structure_quantities::{GapReason, HeatFlow, StructureQuantities};
use crate::model::{Document, ElementBody, ElementId};
use serde_json::{json, Value};

fn session() -> Session {
    Session::new(
        Document::new("Test"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    )
}

fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    let cmd = AnyCommand::from_tool(tool, &args)
        .map_err(|m| vec![Failure::error("test.bad_args", m, vec![])])?;
    s.execute(cmd, &ExecContext::user())
}

fn run(s: &mut Session, tool: &str, args: Value) -> ElementId {
    match try_run(s, tool, args) {
        Ok(o) => serde_json::from_value(o.output["element"].clone()).unwrap(),
        Err(f) => panic!("{tool} failed: {f:?}"),
    }
}

fn code(r: Result<Outcome, Vec<Failure>>) -> String {
    match r {
        Ok(_) => panic!("expected a failure"),
        Err(f) => f[0].code.clone(),
    }
}

fn material(s: &Session, id: ElementId) -> Material {
    match &s.doc().get(id).unwrap().body {
        ElementBody::Material(m) => (**m).clone(),
        other => panic!("not a material: {other:?}"),
    }
}

fn concrete(s: &mut Session) -> ElementId {
    run(
        s,
        "create_material",
        json!({"name": "Concrete", "color": "#a0a0a0", "roughness": 0.7,
            "physical": {"density_kg_m3": 2400, "youngs_modulus_pa": 31e9, "poisson_ratio": 0.2},
            "thermal": {"conductivity_w_mk": 2.0, "specific_heat_j_kgk": 1000},
            "environmental": {"declared_unit": "m3", "gwp_a1_a3": 300, "epd_source": "generic"},
            "cost": {"unit_price": 2000000, "currency": "VND", "unit": "m3"}}),
    )
}

#[test]
fn create_material_stores_assets() {
    let mut s = session();
    let id = concrete(&mut s);
    let m = material(&s, id);
    assert!((m.roughness - 0.7).abs() < 1e-6);
    assert!((m.density_kg_m3().unwrap() - 2400.0).abs() < 1e-9);
    let e = m.environmental.unwrap();
    assert_eq!(e.declared_unit, MaterialUnit::CubicMetre);
    assert_eq!(m.cost.unwrap().currency, "VND");
    assert!(m.appearance.is_none());
}

#[test]
fn create_material_without_assets_is_as_before() {
    let mut s = session();
    let id = run(
        &mut s,
        "create_material",
        json!({"name": "Brick", "color": "#b5533c"}),
    );
    let m = material(&s, id);
    assert!(m.physical.is_none() && m.thermal.is_none() && m.environmental.is_none());
    // The element serialises with colour and roughness only.
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v.as_object().unwrap().len(), 2);
}

#[test]
fn create_material_rejects_bad_values() {
    let mut s = session();
    let r = try_run(
        &mut s,
        "create_material",
        json!({"name": "X", "color": "#ffffff", "thermal": {"conductivity_w_mk": -1}}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
    let r = try_run(
        &mut s,
        "create_material",
        json!({"name": "X", "color": "#ffffff", "cost": {"unit_price": 5, "currency": "đ", "unit": "m2"}}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
    assert_eq!(s.doc().elements().count(), 0, "nothing committed");
}

#[test]
fn set_material_patch_keeps_omitted_fields() {
    let mut s = session();
    let id = concrete(&mut s);
    run(
        &mut s,
        "set_material",
        json!({"material": id, "thermal": {"conductivity_w_mk": 1.8}, "color": "#808080"}),
    );
    let m = material(&s, id);
    let t = m.thermal.as_ref().unwrap();
    assert!((t.conductivity_w_mk.unwrap() - 1.8).abs() < 1e-12);
    assert!(
        (t.specific_heat_j_kgk.unwrap() - 1000.0).abs() < 1e-12,
        "kept"
    );
    assert!((m.roughness - 0.7).abs() < 1e-6, "kept");
    assert!((m.color[0] - 128.0 / 255.0).abs() < 1e-6);
    assert!(
        m.physical.is_some() && m.cost.is_some(),
        "other assets kept"
    );
}

#[test]
fn set_material_null_clears_a_field_and_clear_removes_assets() {
    let mut s = session();
    let id = concrete(&mut s);
    run(
        &mut s,
        "set_material",
        json!({"material": id, "physical": {"poisson_ratio": null}, "clear": ["cost"]}),
    );
    let m = material(&s, id);
    assert!(m.physical.as_ref().unwrap().poisson_ratio.is_none());
    assert!(m.physical.as_ref().unwrap().density_kg_m3.is_some());
    assert!(m.cost.is_none());
}

#[test]
fn set_material_creates_missing_asset_from_patch() {
    let mut s = session();
    let id = run(
        &mut s,
        "create_material",
        json!({"name": "Glass", "color": "#c0e0f0"}),
    );
    run(
        &mut s,
        "set_material",
        json!({"material": id, "appearance": {"transmission_weight": 0.9}}),
    );
    let a = material(&s, id).appearance.unwrap();
    assert!((a.transmission_weight - 0.9).abs() < 1e-12);
    assert!((a.specular_ior - 1.5).abs() < 1e-12, "OpenPBR default");
    // A new cost asset needs all its required fields.
    let r = try_run(
        &mut s,
        "set_material",
        json!({"material": id, "cost": {"unit_price": 10}}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
}

#[test]
fn set_material_rejects_unknown_fields_and_bad_targets() {
    let mut s = session();
    let id = concrete(&mut s);
    let r = try_run(
        &mut s,
        "set_material",
        json!({"material": id, "thermal": {"conductivity": 1.0}}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
    let r = try_run(
        &mut s,
        "set_material",
        json!({"material": id, "thermal": 1.0}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
    let r = try_run(
        &mut s,
        "set_material",
        json!({"material": id, "roughness": 1.5}),
    );
    assert_eq!(code(r), "ferro.model.invalid_material");
    let level = run(
        &mut s,
        "create_level",
        json!({"name": "L1", "elevation": 0}),
    );
    let r = try_run(&mut s, "set_material", json!({"material": level}));
    assert_eq!(code(r), "ferro.model.not_a_material");
    let r = try_run(
        &mut s,
        "set_material",
        json!({"material": id, "color": "red"}),
    );
    assert_eq!(code(r), "ferro.model.invalid_color");
    // Failed edits changed nothing.
    let t = material(&s, id).thermal.unwrap();
    assert!((t.conductivity_w_mk.unwrap() - 2.0).abs() < 1e-12);
}

#[test]
fn set_material_is_undoable() {
    let mut s = session();
    let id = concrete(&mut s);
    run(
        &mut s,
        "set_material",
        json!({"material": id, "clear": ["thermal"]}),
    );
    assert!(material(&s, id).thermal.is_none());
    s.undo();
    assert!(material(&s, id).thermal.is_some());
}

#[test]
fn wall_type_quantities_follow_material_edits() {
    // 200 mm concrete: R = 0.13 + 0.2/2.0 + 0.04 = 0.27, U = 3.7037.
    let mut s = session();
    let c = concrete(&mut s);
    let wool = run(
        &mut s,
        "create_material",
        json!({"name": "Wool", "color": "#eeee88"}),
    );
    let wt = run(
        &mut s,
        "create_wall_type",
        json!({"name": "W", "layers": [{"function": "structure", "thickness": 200, "material": c}]}),
    );
    let q = StructureQuantities::from_type(s.doc(), wt).unwrap();
    assert_eq!(q.heat_flow, HeatFlow::Horizontal);
    assert!((q.u_value_w_m2k.unwrap() - 1.0 / 0.27).abs() < 1e-9);
    assert!((q.mass_kg_m2.unwrap() - 480.0).abs() < 1e-9);
    assert!((q.gwp_a1_a3_kgco2e_m2.unwrap() - 60.0).abs() < 1e-9);
    assert!((q.cost_per_m2.unwrap() - 400_000.0).abs() < 1e-6);

    // An insulation layer without data: reported.
    let wt2 = run(
        &mut s,
        "create_wall_type",
        json!({"name": "W2", "layers": [
            {"function": "structure", "thickness": 200, "material": c},
            {"function": "insulation", "thickness": 100, "material": wool}]}),
    );
    let q = StructureQuantities::from_type(s.doc(), wt2).unwrap();
    assert!(q.u_value_w_m2k.is_none());
    assert!(q
        .gaps
        .iter()
        .any(|g| g.layer == 1 && g.reason == GapReason::NoConductivity));

    // Giving the wool data completes the U-value without touching the type.
    run(
        &mut s,
        "set_material",
        json!({"material": wool, "thermal": {"conductivity_w_mk": 0.04}}),
    );
    let q = StructureQuantities::from_type(s.doc(), wt2).unwrap();
    // R = 0.27 + 0.1/0.04 = 2.77
    assert!((q.u_value_w_m2k.unwrap() - 1.0 / 2.77).abs() < 1e-9);

    // Floor types use downward heat flow.
    let ft = run(
        &mut s,
        "create_floor_type",
        json!({"name": "F", "thickness": 200, "material": c}),
    );
    let q = StructureQuantities::from_type(s.doc(), ft).unwrap();
    assert_eq!(q.heat_flow, HeatFlow::Downward);
    assert!((q.thermal_resistance_m2k_w.unwrap() - 0.31).abs() < 1e-9);
    assert!(StructureQuantities::from_type(s.doc(), c).is_none());
}
