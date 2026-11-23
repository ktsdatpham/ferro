//! Showing and reading quantities in the project's display units (M5-24.2).
//!
//! Every place the app prints a length, area, volume or angle goes through
//! here, so the units chosen in *Project settings…* apply everywhere. Values
//! are stored in internal units (mm, mm², mm³, radians, SI); this module only
//! converts at the UI edge.
//!
//! **Typed values:** a bare number is in the project's length unit (as in
//! Revit and SketchUp): with lengths in `m`, typing `4` means 4 m. A number
//! with a unit (`4000mm`, `4m`, `10' 6"`) is read in that unit regardless.

use ferro_core::math::QuantitySpec;
use ferro_core::model::project_settings::{ProjectUnits, UnitChoice};
use ferro_core::Length;

/// Decimals shown when the project has not chosen any for `spec`: whole mm,
/// three decimals for metres, two for m², and so on. `None` = up to six
/// significant digits.
fn default_decimals(spec: QuantitySpec, symbol: &str) -> Option<u8> {
    use QuantitySpec as Q;
    Some(match (spec, symbol) {
        (Q::Length, "mm") => 0,
        (Q::Length, "cm") => 1,
        (Q::Length, "m" | "ft") => 3,
        (Q::Length, "in") => 2,
        (Q::Area, "mm²") => 0,
        (Q::Area, "cm²") => 1,
        (Q::Area, _) => 2,
        (Q::Volume, "mm³") => 0,
        (Q::Volume, "cm³" | "L") => 1,
        (Q::Volume, "ft³") => 2,
        (Q::Volume, _) => 3,
        (Q::Angle, "rad") => 4,
        (Q::Angle, _) => 1,
        (Q::UValue | Q::ThermalConductivity | Q::ThermalResistance, _) => 3,
        _ => return None,
    })
}

/// A stored value of `spec` in the project's display unit with its symbol,
/// e.g. `"3.300 m"`, `"12.00 m²"`, `"90.0°"`. Uses the project's decimals,
/// else [`default_decimals`]. Never prints `-0`; non-finite values show `"—"`.
pub fn quantity(u: &ProjectUnits, spec: QuantitySpec, stored: f64) -> String {
    let unit = u.display_unit(spec);
    let decimals = u
        .units
        .get(&spec)
        .and_then(|c| c.decimals)
        .or_else(|| default_decimals(spec, unit.symbol));
    // One formatting path: the core's, with the decimals filled in.
    let mut one = ProjectUnits::default();
    one.units.insert(
        spec,
        UnitChoice {
            unit: unit.symbol.to_string(),
            decimals,
        },
    );
    one.format(spec, stored)
}

/// A length stored in mm, e.g. `"3300 mm"` or `"3.30 m"`.
pub fn length(u: &ProjectUnits, mm: f64) -> String {
    quantity(u, QuantitySpec::Length, mm)
}

/// An area stored in mm², e.g. `"12.00 m²"`.
pub fn area(u: &ProjectUnits, mm2: f64) -> String {
    quantity(u, QuantitySpec::Area, mm2)
}

/// A volume stored in mm³, e.g. `"2.400 m³"`.
pub fn volume(u: &ProjectUnits, mm3: f64) -> String {
    quantity(u, QuantitySpec::Volume, mm3)
}

/// An angle stored in radians, e.g. `"90.0°"`.
pub fn angle(u: &ProjectUnits, rad: f64) -> String {
    quantity(u, QuantitySpec::Angle, rad)
}

/// Several lengths in mm sharing one unit symbol, e.g. `"4.000 × 3.000 m"`.
pub fn lengths(u: &ProjectUnits, mm: &[f64]) -> String {
    let unit = u.display_unit(QuantitySpec::Length).symbol;
    let parts: Vec<String> = mm
        .iter()
        .map(|v| {
            let s = length(u, *v);
            s.strip_suffix(unit)
                .map(|n| n.trim_end().to_string())
                .unwrap_or(s)
        })
        .collect();
    format!("{} {unit}", parts.join(" × "))
}

/// Read one typed length in mm. A bare number is in the project's length
/// unit; a number with a unit (`4m`, `4000mm`, `10' 6"`) uses that unit.
/// `None` if the text is not a finite length.
pub fn parse_length(u: &ProjectUnits, text: &str) -> Option<f64> {
    let t = text.trim();
    if let Ok(v) = t.parse::<f64>() {
        return v
            .is_finite()
            .then(|| u.display_unit(QuantitySpec::Length).to_storage(v));
    }
    Length::parse(t).map(|l| l.mm()).filter(|v| v.is_finite())
}

/// Read typed lengths separated by `,` or `;` (`"4,3"`, `"4m; 3m"`), in mm.
/// Fails with a message naming the part that could not be read.
pub fn parse_lengths(u: &ProjectUnits, text: &str) -> Result<Vec<f64>, String> {
    let parts: Vec<&str> = text
        .split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        let unit = u.display_unit(QuantitySpec::Length).symbol;
        return Err(format!("type a value, e.g. 3000mm or 3m (bare = {unit})"));
    }
    parts
        .iter()
        .map(|p| parse_length(u, p).ok_or_else(|| format!("cannot read {p:?}")))
        .collect()
}

/// Read a typed angle in radians. A bare number is in the project's angle
/// unit (degrees by default); `°`, `deg` and `rad` suffixes are honoured.
/// `None` if the text is not a finite angle.
pub fn parse_angle(u: &ProjectUnits, text: &str) -> Option<f64> {
    let t = text.trim();
    if let Ok(v) = t.parse::<f64>() {
        return v
            .is_finite()
            .then(|| u.display_unit(QuantitySpec::Angle).to_storage(v));
    }
    ferro_core::math::Angle::parse(t).map(|a| a.rad())
}

/// A material cost per m² with its currency, e.g. `"450000 VND/m²"`.
/// Costs are never converted: when the materials' currency differs from the
/// project's, the text says so (`"12 USD/m² ≠ project VND"`). Without a
/// project currency the materials' own code is shown as is.
pub fn cost_per_m2(value: f64, currency: Option<&str>, project: Option<&str>) -> String {
    let n = if value.is_finite() {
        format!("{value:.0}")
    } else {
        "—".to_string()
    };
    let n = if n == "-0" { "0".to_string() } else { n };
    match (currency, project) {
        (Some(c), Some(p)) if c != p => format!("{n} {c}/m² ≠ project {p}"),
        (Some(c), _) => format!("{n} {c}/m²"),
        (None, Some(p)) => format!("{n} {p}/m²"),
        (None, None) => format!("{n} /m²"),
    }
}

/// `true` when [`cost_per_m2`] flags a currency mismatch.
pub fn currency_mismatch(currency: Option<&str>, project: Option<&str>) -> bool {
    matches!((currency, project), (Some(c), Some(p)) if c != p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(spec: QuantitySpec, unit: &str, decimals: Option<u8>) -> ProjectUnits {
        let mut u = ProjectUnits::default();
        u.units.insert(
            spec,
            UnitChoice {
                unit: unit.into(),
                decimals,
            },
        );
        u
    }

    #[test]
    fn lengths_follow_the_project_unit() {
        let d = ProjectUnits::default();
        assert_eq!(length(&d, 3300.0), "3300 mm");
        assert_eq!(length(&d, 3300.4), "3300 mm");
        let m = with(QuantitySpec::Length, "m", Some(2));
        assert_eq!(length(&m, 3300.0), "3.30 m");
        let m3 = with(QuantitySpec::Length, "m", None);
        assert_eq!(length(&m3, 3300.0), "3.300 m");
        assert_eq!(
            lengths(&m, &[4000.0, 3000.0, 250.0]),
            "4.00 × 3.00 × 0.25 m"
        );
        assert_eq!(lengths(&d, &[900.0, 2100.0]), "900 × 2100 mm");
    }

    #[test]
    fn areas_volumes_angles() {
        let d = ProjectUnits::default();
        assert_eq!(area(&d, 12e6), "12.00 m²");
        assert_eq!(volume(&d, 2.4e9), "2.400 m³");
        assert_eq!(angle(&d, std::f64::consts::FRAC_PI_2), "90.0°");
        let ft2 = with(QuantitySpec::Area, "ft²", Some(1));
        assert_eq!(area(&ft2, 92_903.04 * 10.0), "10.0 ft²");
        let l = with(QuantitySpec::Volume, "L", Some(0));
        assert_eq!(volume(&l, 1e6 * 5.0), "5 L");
    }

    #[test]
    fn negative_zero_and_non_finite() {
        let m = with(QuantitySpec::Length, "m", Some(2));
        assert_eq!(length(&m, -0.001), "0.00 m");
        assert_eq!(length(&ProjectUnits::default(), -0.3), "0 mm");
        assert_eq!(length(&m, -1500.0), "-1.50 m");
        assert_eq!(length(&m, f64::NAN), "—");
        assert_eq!(area(&ProjectUnits::default(), f64::INFINITY), "—");
    }

    #[test]
    fn bare_numbers_use_the_project_length_unit() {
        let d = ProjectUnits::default();
        assert_eq!(parse_lengths(&d, "4000,3000"), Ok(vec![4000.0, 3000.0]));
        let m = with(QuantitySpec::Length, "m", Some(2));
        let v = parse_lengths(&m, "4; 2.5").unwrap_or_default();
        assert!(v.len() == 2 && (v[0] - 4000.0).abs() < 1e-9 && (v[1] - 2500.0).abs() < 1e-9);
        // An explicit unit wins over the project unit.
        assert_eq!(parse_lengths(&m, "4000mm"), Ok(vec![4000.0]));
        let ft = parse_length(&m, "10' 6\"").unwrap_or_default();
        assert!((ft - 3200.4).abs() < 1e-9);
        assert!(parse_lengths(&m, "abc").is_err());
        assert!(parse_lengths(&m, "  ").is_err());
        assert!(parse_length(&m, "inf").is_none());
    }

    #[test]
    fn costs_show_the_project_currency_and_flag_mismatches() {
        assert_eq!(
            cost_per_m2(450000.4, Some("VND"), Some("VND")),
            "450000 VND/m²"
        );
        assert_eq!(
            cost_per_m2(12.0, Some("USD"), Some("VND")),
            "12 USD/m² ≠ project VND"
        );
        assert!(currency_mismatch(Some("USD"), Some("VND")));
        assert!(!currency_mismatch(Some("VND"), Some("VND")));
        assert!(!currency_mismatch(Some("VND"), None));
        assert_eq!(cost_per_m2(12.0, Some("USD"), None), "12 USD/m²");
        assert_eq!(cost_per_m2(-0.2, None, Some("VND")), "0 VND/m²");
    }

    #[test]
    fn typed_angles() {
        let d = ProjectUnits::default();
        let a = parse_angle(&d, "90").unwrap_or_default();
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let r = with(QuantitySpec::Angle, "rad", None);
        assert!((parse_angle(&r, "1").unwrap_or_default() - 1.0).abs() < 1e-12);
        assert!((parse_angle(&r, "90°").unwrap_or_default() - a).abs() < 1e-12);
        assert!(parse_angle(&d, "x").is_none());
    }
}
