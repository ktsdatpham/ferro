//! Project settings commands (M5-24).

use super::*;
use crate::math::QuantitySpec;
use crate::model::Document;
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

#[test]
fn units_change_display_only_and_undo() {
    let mut s = session();
    let before = s.doc().clone();
    // Unit names match like everywhere else (exact, then ignoring case) and
    // are stored by their own symbol.
    run(
        &mut s,
        "set_project_units",
        json!({"spec": "length", "unit": "M", "decimals": 2}),
    )
    .unwrap();
    assert_eq!(
        s.doc().project().settings.units.units[&QuantitySpec::Length].unit,
        "m"
    );
    let u = &s.doc().project().settings.units;
    assert_eq!(u.format(QuantitySpec::Length, 3300.0), "3.30 m");
    s.undo().unwrap();
    assert_eq!(s.doc(), &before);
    s.redo().unwrap();
    run(
        &mut s,
        "set_project_units",
        json!({"spec": "length", "unit": null}),
    )
    .unwrap();
    assert_eq!(
        s.doc()
            .project()
            .settings
            .units
            .format(QuantitySpec::Length, 3300.0),
        "3300 mm"
    );
}

#[test]
fn bad_units_are_refused_without_change() {
    let mut s = session();
    let before = s.doc().clone();
    for args in [
        json!({"spec": "length", "unit": "kg"}),
        json!({"spec": "length", "unit": "m", "decimals": 12}),
    ] {
        let f = run(&mut s, "set_project_units", args).unwrap_err();
        assert_eq!(f[0].code, "ferro.model.invalid_unit");
    }
    assert_eq!(s.doc(), &before);
}

#[test]
fn site_location_is_validated_and_kept_partially() {
    let mut s = session();
    // Radians inside; the MCP edge converts degrees (ferro-tools).
    let lat = 16.05f64.to_radians();
    let lon = 108.2f64.to_radians();
    run(
        &mut s,
        "set_site_location",
        json!({"latitude": lat, "longitude": lon, "utc_offset_minutes": 420, "place": "Đà Nẵng"}),
    )
    .unwrap();
    // Only the elevation changes; the rest is kept.
    run(&mut s, "set_site_location", json!({"elevation": 5000})).unwrap();
    let l = s.doc().project().settings.location.clone().unwrap();
    assert_eq!((l.place.as_str(), l.utc_offset_minutes), ("Đà Nẵng", 420));
    assert!((l.elevation.mm() - 5000.0).abs() < 1e-9);
    // Out of range: refused, location unchanged.
    let f = run(&mut s, "set_site_location", json!({"latitude": 1.6})).unwrap_err();
    assert_eq!(f[0].code, "ferro.model.invalid_location");
    assert_eq!(s.doc().project().settings.location.as_ref(), Some(&l));
    // No location yet and no coordinates: refused.
    let mut t = session();
    let f = run(&mut t, "set_site_location", json!({"place": "x"})).unwrap_err();
    assert_eq!(f[0].code, "ferro.model.invalid_location");
    run(&mut s, "set_site_location", json!({"clear": true})).unwrap();
    assert!(s.doc().project().settings.location.is_none());
}

#[test]
fn currency_codes() {
    let mut s = session();
    run(&mut s, "set_project_currency", json!({"currency": " vnd "})).unwrap();
    assert_eq!(s.doc().project().settings.currency.as_deref(), Some("VND"));
    let f = run(&mut s, "set_project_currency", json!({"currency": "DONG"})).unwrap_err();
    assert_eq!(f[0].code, "ferro.model.invalid_currency");
    run(&mut s, "set_project_currency", json!({"currency": null})).unwrap();
    assert_eq!(s.doc().project().settings.currency, None);
}
