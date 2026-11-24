//! Project settings (M5-24): display units, site location and currency —
//! one place that the UI, MCP and exports read. Revit: Project Units,
//! `SiteLocation`, Project Information.
//!
//! Values are stored in Ferro's internal units (mm, radians, SI); units here
//! only choose how numbers are *shown and typed*.

use crate::math::{Angle, Length, QuantitySpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How one quantity is shown: a unit symbol from the spec's table and a
/// number of decimals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitChoice {
    /// A symbol or alias of one of the spec's display units, e.g. `"m"`.
    pub unit: String,
    /// Decimals shown; `None` = up to six significant digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
}

/// Display units per quantity (Revit: Project Units). Quantities not listed
/// use their spec's default unit (mm for length).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectUnits {
    #[serde(default)]
    pub units: BTreeMap<QuantitySpec, UnitChoice>,
}

/// Largest number of decimals accepted.
pub const MAX_DECIMALS: u8 = 9;

impl ProjectUnits {
    /// The display unit for a spec: the project's choice when it names one of
    /// the spec's units, else the spec default.
    pub fn display_unit(&self, spec: QuantitySpec) -> &'static crate::math::DisplayUnit {
        self.units
            .get(&spec)
            .and_then(|c| spec.unit(&c.unit))
            .unwrap_or_else(|| spec.default_unit())
    }

    /// Format a stored value for display, e.g. `"3.3 m"` or `"3300 mm"`.
    /// Non-finite values show as `"—"`.
    pub fn format(&self, spec: QuantitySpec, stored: f64) -> String {
        if !stored.is_finite() {
            return "—".into();
        }
        let u = self.display_unit(spec);
        let v = u.from_storage(stored);
        let n = match self.units.get(&spec).and_then(|c| c.decimals) {
            Some(d) => {
                let s = format!("{v:.*}", d as usize);
                // "-0.0" reads badly.
                if s.trim_start_matches('-')
                    .chars()
                    .all(|c| c == '0' || c == '.')
                {
                    s.trim_start_matches('-').to_string()
                } else {
                    s
                }
            }
            None => crate::math::format_number(v),
        };
        match u.symbol {
            "" => n,
            "°" => format!("{n}°"),
            s => format!("{n} {s}"),
        }
    }

    /// Check a choice: the unit must belong to the spec, decimals ≤ 9.
    /// Fails with `ferro.model.invalid_unit`.
    pub fn validate_choice(spec: QuantitySpec, c: &UnitChoice) -> Result<(), String> {
        if spec.unit(&c.unit).is_none() {
            let known: Vec<&str> = spec.display_units().iter().map(|u| u.symbol).collect();
            return Err(format!(
                "`{}` is not a unit of {} (use one of: {})",
                c.unit,
                spec.id(),
                known.join(", ")
            ));
        }
        if c.decimals.is_some_and(|d| d > MAX_DECIMALS) {
            return Err(format!("decimals must be 0..={MAX_DECIMALS}"));
        }
        Ok(())
    }
}

/// Where the project is on Earth (Revit: `SiteLocation`), for sun, energy
/// and weather. Angles in radians, elevation in mm (internal units).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteLocation {
    /// Place name, e.g. "Đà Nẵng".
    #[serde(default)]
    pub place: String,
    /// North positive, −π/2..π/2.
    pub latitude: Angle,
    /// East positive, −π..π.
    pub longitude: Angle,
    /// Height of the project's internal origin above sea level.
    #[serde(default)]
    pub elevation: Length,
    /// Offset of local standard time from UTC, minutes (Vietnam: +420).
    #[serde(default)]
    pub utc_offset_minutes: i32,
    /// Weather file (EPW) path or name, used by energy and daylight tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather_file: Option<String>,
}

impl SiteLocation {
    /// Check ranges. Fails with a message for `ferro.model.invalid_location`.
    pub fn validate(&self) -> Result<(), String> {
        let (lat, lon) = (self.latitude.deg(), self.longitude.deg());
        if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
            return Err(format!("latitude {lat}° is outside −90°..90°"));
        }
        if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
            return Err(format!("longitude {lon}° is outside −180°..180°"));
        }
        if !self.elevation.mm().is_finite() {
            return Err("elevation must be a finite length".into());
        }
        // Real offsets run from UTC−12:00 to UTC+14:00.
        if !(-12 * 60..=14 * 60).contains(&self.utc_offset_minutes) {
            return Err(format!(
                "UTC offset {} min is outside −12:00..+14:00",
                self.utc_offset_minutes
            ));
        }
        Ok(())
    }
}

/// Project settings carried by `ProjectInfo`. All parts are optional in
/// files (omitted when default).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default, skip_serializing_if = "is_default_units")]
    pub units: ProjectUnits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SiteLocation>,
    /// ISO 4217 code used for costs, e.g. "VND".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// New datums (levels, grids, reference planes) are created pinned
    /// (ADR 0008 §4), for teams that want it. Default `false` (as Revit);
    /// omitted from files when `false`. Linked models will be created
    /// pinned regardless, once links exist.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_pin_datums: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings {
            units: ProjectUnits::default(),
            location: None,
            currency: None,
            auto_pin_datums: false,
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_default_units(u: &ProjectUnits) -> bool {
    u.units.is_empty()
}

/// `true` for a three-letter upper-case code (ISO 4217 shape).
pub fn valid_currency(code: &str) -> bool {
    code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(spec: QuantitySpec, unit: &str, decimals: Option<u8>) -> ProjectUnits {
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
    fn lengths_show_in_the_chosen_unit() {
        let d = ProjectUnits::default();
        assert_eq!(d.format(QuantitySpec::Length, 3300.0), "3300 mm");
        let m = units(QuantitySpec::Length, "m", Some(2));
        assert_eq!(m.format(QuantitySpec::Length, 3300.0), "3.30 m");
        // Other specs are untouched.
        assert_eq!(
            m.format(QuantitySpec::Area, 12e6),
            d.format(QuantitySpec::Area, 12e6)
        );
    }

    #[test]
    fn degenerate_values_and_choices() {
        let m = units(QuantitySpec::Length, "m", Some(1));
        assert_eq!(m.format(QuantitySpec::Length, f64::NAN), "—");
        // -0.04 mm rounds to 0.0 m, not "-0.0 m".
        assert_eq!(m.format(QuantitySpec::Length, -0.04), "0.0 m");
        // An unknown unit falls back to the default when formatting...
        let bad = units(QuantitySpec::Length, "parsec", None);
        assert_eq!(bad.format(QuantitySpec::Length, 1.0), "1 mm");
        // ...and is refused when set.
        let c = UnitChoice {
            unit: "kg".into(),
            decimals: None,
        };
        assert!(ProjectUnits::validate_choice(QuantitySpec::Length, &c).is_err());
        let c = UnitChoice {
            unit: "m".into(),
            decimals: Some(10),
        };
        assert!(ProjectUnits::validate_choice(QuantitySpec::Length, &c).is_err());
        let c = UnitChoice {
            unit: "m".into(),
            decimals: Some(MAX_DECIMALS),
        };
        assert!(ProjectUnits::validate_choice(QuantitySpec::Length, &c).is_ok());
    }

    #[test]
    fn site_location_ranges() {
        let mut l = SiteLocation {
            place: "Đà Nẵng".into(),
            latitude: Angle::from_deg(16.05),
            longitude: Angle::from_deg(108.2),
            elevation: Length::from_mm(5000.0),
            utc_offset_minutes: 420,
            weather_file: None,
        };
        assert!(l.validate().is_ok());
        l.latitude = Angle::from_deg(90.0);
        assert!(l.validate().is_ok(), "pole is valid");
        l.latitude = Angle::from_deg(90.0001);
        assert!(l.validate().is_err());
        l.latitude = Angle::from_deg(10.0);
        l.utc_offset_minutes = 15 * 60;
        assert!(l.validate().is_err());
        assert!(valid_currency("VND") && !valid_currency("vnd") && !valid_currency("VN"));
    }

    #[test]
    fn default_settings_are_not_written() {
        let s = serde_json::to_string(&ProjectSettings::default()).unwrap();
        assert_eq!(s, "{}");
        let back: ProjectSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(back, ProjectSettings::default());
        assert!(
            !back.auto_pin_datums,
            "datums are not auto-pinned by default"
        );
        let on = ProjectSettings {
            auto_pin_datums: true,
            ..ProjectSettings::default()
        };
        let s = serde_json::to_string(&on).unwrap();
        assert_eq!(s, r#"{"auto_pin_datums":true}"#);
        let back: ProjectSettings = serde_json::from_str(&s).unwrap();
        assert_eq!(back, on);
    }
}
