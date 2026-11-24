//! The fixed table of physical quantities for typed parameters (ADR 0004 §2).
//!
//! Each [`QuantitySpec`] has a stable snake_case id, a storage unit, a Revit
//! `SpecTypeId` name, an IFC measure type and a list of display units.
//! Values are stored in SI, except length (mm, D-01) and angle (rad, D-02),
//! and derived units keep SI metres (density is kg/m³, not kg/mm³). Unit
//! conversion happens only at the edges (UI, MCP, files).

use super::{tol, Length};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A physical quantity a parameter value can carry (Revit: `SpecTypeId`,
/// IFC: `Ifc*Measure`). The serde form is the stable id, e.g.
/// `"thermal_conductivity"`; ids are never renamed once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantitySpec {
    /// Stored in mm.
    Length,
    /// Stored in mm².
    Area,
    /// Stored in mm³.
    Volume,
    /// Stored in radians.
    Angle,
    /// Stored in kg.
    Mass,
    /// Mass density, stored in kg/m³.
    Density,
    /// Stored in N.
    Force,
    /// Bending moment or torque, stored in N·m.
    Moment,
    /// Mechanical stress or strength, stored in Pa.
    Stress,
    /// Stored in W/(m·K).
    ThermalConductivity,
    /// Stored in m²·K/W.
    ThermalResistance,
    /// Thermal transmittance, stored in W/(m²·K).
    UValue,
    /// Specific heat capacity, stored in J/(kg·K).
    HeatCapacity,
    /// Thermodynamic temperature, stored in K.
    Temperature,
    /// Stored in W.
    Power,
    /// Stored in lx.
    Illuminance,
    /// Volumetric flow rate, stored in m³/s.
    Flow,
    /// Fluid pressure, stored in Pa.
    Pressure,
    /// Money in the project currency (no conversion between currencies).
    Currency,
    /// Global warming potential, stored in kg CO₂-equivalent.
    CarbonMass,
    /// Stored in s.
    Time,
    /// Unitless ratio, stored as a fraction (0.5 = 50 %).
    Ratio,
    /// Unitless real number.
    Number,
}

/// A unit a quantity can be typed in or shown in.
///
/// `storage = value × factor + offset`. Only temperatures have an offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayUnit {
    /// Symbol used for display, e.g. `"W/(m·K)"`. Empty for unitless values.
    pub symbol: &'static str,
    /// Other spellings accepted on input, e.g. `"W/mK"`.
    pub aliases: &'static [&'static str],
    /// Storage units per display unit.
    pub factor: f64,
    /// Added after scaling (storage units); 273.15 for °C.
    pub offset: f64,
}

impl DisplayUnit {
    const fn new(symbol: &'static str, aliases: &'static [&'static str], factor: f64) -> Self {
        DisplayUnit {
            symbol,
            aliases,
            factor,
            offset: 0.0,
        }
    }

    /// Convert a value in this unit to the storage unit.
    pub fn to_storage(&self, value: f64) -> f64 {
        value * self.factor + self.offset
    }

    /// Convert a stored value to this unit.
    pub fn from_storage(&self, stored: f64) -> f64 {
        (stored - self.offset) / self.factor
    }

    /// `true` if `text` names this unit. Spaces, `·`, `*`, `-`, `.` and
    /// brackets are ignored and `²` / `³` equal `2` / `3`, so `"W/(m·K)"`,
    /// `"W/mK"` and `"W/m.K"` all match. Case-sensitive (`MPa` ≠ `mPa`).
    pub fn matches(&self, text: &str) -> bool {
        let n = normalize_unit(text);
        std::iter::once(self.symbol)
            .chain(self.aliases.iter().copied())
            .any(|a| normalize_unit(a) == n)
    }

    fn matches_ignore_case(&self, text: &str) -> bool {
        let n = normalize_unit(text).to_lowercase();
        std::iter::once(self.symbol)
            .chain(self.aliases.iter().copied())
            .any(|a| normalize_unit(a).to_lowercase() == n)
    }
}

/// Why a quantity text could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuantityError {
    /// The text does not start with a finite number.
    NotANumber(String),
    /// A bare number was given and no spec says what it measures.
    MissingUnit,
    /// The unit is not in the table (for the expected spec, if any).
    UnknownUnit {
        unit: String,
        expected: Option<QuantitySpec>,
    },
}

impl fmt::Display for QuantityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantityError::NotANumber(t) => write!(f, "{t:?} does not start with a number"),
            QuantityError::MissingUnit => {
                write!(
                    f,
                    "a unit is needed to know the quantity, e.g. \"0.8 W/mK\""
                )
            }
            QuantityError::UnknownUnit {
                unit,
                expected: Some(s),
            } => {
                let known: Vec<&str> = s
                    .display_units()
                    .iter()
                    .map(|u| u.symbol)
                    .filter(|u| !u.is_empty())
                    .collect();
                write!(
                    f,
                    "unknown unit {unit:?} for {s}; use one of: {}",
                    known.join(", ")
                )
            }
            QuantityError::UnknownUnit {
                unit,
                expected: None,
            } => write!(f, "unknown unit {unit:?}"),
        }
    }
}

impl std::error::Error for QuantityError {}

const DEG: f64 = std::f64::consts::PI / 180.0;

const LENGTH_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("mm", &[], 1.0),
    DisplayUnit::new("cm", &[], 10.0),
    DisplayUnit::new("m", &[], 1000.0),
    DisplayUnit::new("in", &["\""], 25.4),
    DisplayUnit::new("ft", &["'"], 304.8),
];
const AREA_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("m²", &[], 1e6),
    DisplayUnit::new("mm²", &[], 1.0),
    DisplayUnit::new("cm²", &[], 100.0),
    DisplayUnit::new("ft²", &["sf", "sqft"], 92_903.04),
];
const VOLUME_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("m³", &[], 1e9),
    DisplayUnit::new("mm³", &[], 1.0),
    DisplayUnit::new("cm³", &[], 1e3),
    DisplayUnit::new("L", &["l"], 1e6),
    DisplayUnit::new("ft³", &["cf"], 28_316_846.592),
];
const ANGLE_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("°", &["deg"], DEG),
    DisplayUnit::new("rad", &[], 1.0),
];
const MASS_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("kg", &[], 1.0),
    DisplayUnit::new("g", &[], 1e-3),
    DisplayUnit::new("t", &[], 1e3),
    DisplayUnit::new("lb", &[], 0.453_592_37),
];
const DENSITY_UNITS: &[DisplayUnit] = &[DisplayUnit::new("kg/m³", &[], 1.0)];
const FORCE_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("kN", &[], 1e3),
    DisplayUnit::new("N", &[], 1.0),
    DisplayUnit::new("MN", &[], 1e6),
];
const MOMENT_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("kN·m", &[], 1e3),
    DisplayUnit::new("N·m", &[], 1.0),
];
const STRESS_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("MPa", &["N/mm²"], 1e6),
    DisplayUnit::new("kPa", &[], 1e3),
    DisplayUnit::new("Pa", &[], 1.0),
    DisplayUnit::new("GPa", &[], 1e9),
];
const THERMAL_CONDUCTIVITY_UNITS: &[DisplayUnit] = &[DisplayUnit::new("W/(m·K)", &[], 1.0)];
const THERMAL_RESISTANCE_UNITS: &[DisplayUnit] = &[DisplayUnit::new("m²·K/W", &[], 1.0)];
const U_VALUE_UNITS: &[DisplayUnit] = &[DisplayUnit::new("W/(m²·K)", &[], 1.0)];
const HEAT_CAPACITY_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("J/(kg·K)", &[], 1.0),
    DisplayUnit::new("kJ/(kg·K)", &[], 1e3),
];
const TEMPERATURE_UNITS: &[DisplayUnit] = &[
    DisplayUnit {
        symbol: "°C",
        aliases: &["degC", "C"],
        factor: 1.0,
        offset: 273.15,
    },
    DisplayUnit::new("K", &[], 1.0),
    DisplayUnit {
        symbol: "°F",
        aliases: &["degF", "F"],
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
    },
];
const POWER_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("W", &[], 1.0),
    DisplayUnit::new("kW", &[], 1e3),
    DisplayUnit::new("MW", &[], 1e6),
];
const ILLUMINANCE_UNITS: &[DisplayUnit] = &[DisplayUnit::new("lx", &["lux"], 1.0)];
const FLOW_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("L/s", &["l/s"], 1e-3),
    DisplayUnit::new("m³/s", &[], 1.0),
    DisplayUnit::new("m³/h", &[], 1.0 / 3600.0),
];
const PRESSURE_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("Pa", &[], 1.0),
    DisplayUnit::new("kPa", &[], 1e3),
    DisplayUnit::new("bar", &[], 1e5),
];
const CURRENCY_UNITS: &[DisplayUnit] = &[DisplayUnit::new("", &[], 1.0)];
const CARBON_MASS_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("kgCO2e", &["kgCO₂e", "kgCO2eq"], 1.0),
    DisplayUnit::new("tCO2e", &["tCO₂e", "tCO2eq"], 1e3),
];
const TIME_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("s", &[], 1.0),
    DisplayUnit::new("min", &[], 60.0),
    DisplayUnit::new("h", &[], 3600.0),
    DisplayUnit::new("d", &[], 86_400.0),
];
const RATIO_UNITS: &[DisplayUnit] = &[
    DisplayUnit::new("", &[], 1.0),
    DisplayUnit::new("%", &[], 0.01),
];
const NUMBER_UNITS: &[DisplayUnit] = &[DisplayUnit::new("", &[], 1.0)];

impl QuantitySpec {
    /// Every spec, in table order. When a unit symbol belongs to several
    /// specs (`Pa`: pressure and stress) and no spec is expected, the first
    /// one in this order wins.
    pub const ALL: [QuantitySpec; 23] = [
        QuantitySpec::Length,
        QuantitySpec::Area,
        QuantitySpec::Volume,
        QuantitySpec::Angle,
        QuantitySpec::Mass,
        QuantitySpec::Density,
        QuantitySpec::Force,
        QuantitySpec::Moment,
        QuantitySpec::Pressure,
        QuantitySpec::Stress,
        QuantitySpec::ThermalConductivity,
        QuantitySpec::ThermalResistance,
        QuantitySpec::UValue,
        QuantitySpec::HeatCapacity,
        QuantitySpec::Temperature,
        QuantitySpec::Power,
        QuantitySpec::Illuminance,
        QuantitySpec::Flow,
        QuantitySpec::Currency,
        QuantitySpec::CarbonMass,
        QuantitySpec::Time,
        QuantitySpec::Ratio,
        QuantitySpec::Number,
    ];

    /// Stable snake_case id (same as the serde form), e.g. `"u_value"`.
    pub fn id(self) -> &'static str {
        use QuantitySpec::*;
        match self {
            Length => "length",
            Area => "area",
            Volume => "volume",
            Angle => "angle",
            Mass => "mass",
            Density => "density",
            Force => "force",
            Moment => "moment",
            Stress => "stress",
            ThermalConductivity => "thermal_conductivity",
            ThermalResistance => "thermal_resistance",
            UValue => "u_value",
            HeatCapacity => "heat_capacity",
            Temperature => "temperature",
            Power => "power",
            Illuminance => "illuminance",
            Flow => "flow",
            Pressure => "pressure",
            Currency => "currency",
            CarbonMass => "carbon_mass",
            Time => "time",
            Ratio => "ratio",
            Number => "number",
        }
    }

    /// Look a spec up by its stable id. `None` if unknown.
    pub fn from_id(id: &str) -> Option<QuantitySpec> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Symbol of the storage unit, e.g. `"mm"`, `"kg/m³"`, `"K"`.
    pub fn storage_unit(self) -> &'static str {
        use QuantitySpec::*;
        match self {
            Length => "mm",
            Area => "mm²",
            Volume => "mm³",
            Angle => "rad",
            Mass => "kg",
            Density => "kg/m³",
            Force => "N",
            Moment => "N·m",
            Stress | Pressure => "Pa",
            ThermalConductivity => "W/(m·K)",
            ThermalResistance => "m²·K/W",
            UValue => "W/(m²·K)",
            HeatCapacity => "J/(kg·K)",
            Temperature => "K",
            Power => "W",
            Illuminance => "lx",
            Flow => "m³/s",
            Currency => "",
            CarbonMass => "kgCO2e",
            Time => "s",
            Ratio | Number => "",
        }
    }

    /// Name of the Revit `SpecTypeId` property, e.g. `"HeatTransferCoefficient"`.
    ///
    /// Revit has no carbon or ratio spec: carbon mass maps to `Mass` and
    /// ratio to `Number`. [`QuantitySpec::from_revit_spec`] returns the first
    /// spec in table order for such shared names.
    pub fn revit_spec(self) -> &'static str {
        use QuantitySpec::*;
        match self {
            Length => "Length",
            Area => "Area",
            Volume => "Volume",
            Angle => "Angle",
            Mass | CarbonMass => "Mass",
            Density => "MassDensity",
            Force => "Force",
            Moment => "Moment",
            Stress => "Stress",
            ThermalConductivity => "ThermalConductivity",
            ThermalResistance => "ThermalResistance",
            UValue => "HeatTransferCoefficient",
            HeatCapacity => "SpecificHeat",
            Temperature => "HvacTemperature",
            Power => "HvacPower",
            Illuminance => "Illuminance",
            Flow => "Flow",
            Pressure => "HvacPressure",
            Currency => "Currency",
            Time => "Time",
            Ratio | Number => "Number",
        }
    }

    /// IFC4 measure type, e.g. `"IfcThermalConductivityMeasure"`.
    ///
    /// IFC has no stress measure (stress uses `IfcPressureMeasure`) and no
    /// carbon measure (carbon mass uses `IfcMassMeasure`); unitless numbers
    /// use `IfcReal`.
    pub fn ifc_measure(self) -> &'static str {
        use QuantitySpec::*;
        match self {
            Length => "IfcLengthMeasure",
            Area => "IfcAreaMeasure",
            Volume => "IfcVolumeMeasure",
            Angle => "IfcPlaneAngleMeasure",
            Mass | CarbonMass => "IfcMassMeasure",
            Density => "IfcMassDensityMeasure",
            Force => "IfcForceMeasure",
            Moment => "IfcTorqueMeasure",
            Stress | Pressure => "IfcPressureMeasure",
            ThermalConductivity => "IfcThermalConductivityMeasure",
            ThermalResistance => "IfcThermalResistanceMeasure",
            UValue => "IfcThermalTransmittanceMeasure",
            HeatCapacity => "IfcSpecificHeatCapacityMeasure",
            Temperature => "IfcThermodynamicTemperatureMeasure",
            Power => "IfcPowerMeasure",
            Illuminance => "IfcIlluminanceMeasure",
            Flow => "IfcVolumetricFlowRateMeasure",
            Currency => "IfcMonetaryMeasure",
            Time => "IfcTimeMeasure",
            Ratio => "IfcRatioMeasure",
            Number => "IfcReal",
        }
    }

    /// The spec for a Revit `SpecTypeId` name; first in table order when
    /// several specs share it. `None` if Ferro has no such spec.
    pub fn from_revit_spec(name: &str) -> Option<QuantitySpec> {
        Self::ALL.into_iter().find(|s| s.revit_spec() == name)
    }

    /// The spec for an IFC measure type name; first in table order when
    /// several specs share it. `None` if Ferro has no such spec.
    pub fn from_ifc_measure(name: &str) -> Option<QuantitySpec> {
        Self::ALL.into_iter().find(|s| s.ifc_measure() == name)
    }

    /// Units this quantity can be typed in or shown in. Never empty; the
    /// first one is the default display unit.
    pub fn display_units(self) -> &'static [DisplayUnit] {
        use QuantitySpec::*;
        match self {
            Length => LENGTH_UNITS,
            Area => AREA_UNITS,
            Volume => VOLUME_UNITS,
            Angle => ANGLE_UNITS,
            Mass => MASS_UNITS,
            Density => DENSITY_UNITS,
            Force => FORCE_UNITS,
            Moment => MOMENT_UNITS,
            Stress => STRESS_UNITS,
            ThermalConductivity => THERMAL_CONDUCTIVITY_UNITS,
            ThermalResistance => THERMAL_RESISTANCE_UNITS,
            UValue => U_VALUE_UNITS,
            HeatCapacity => HEAT_CAPACITY_UNITS,
            Temperature => TEMPERATURE_UNITS,
            Power => POWER_UNITS,
            Illuminance => ILLUMINANCE_UNITS,
            Flow => FLOW_UNITS,
            Pressure => PRESSURE_UNITS,
            Currency => CURRENCY_UNITS,
            CarbonMass => CARBON_MASS_UNITS,
            Time => TIME_UNITS,
            Ratio => RATIO_UNITS,
            Number => NUMBER_UNITS,
        }
    }

    /// Default display unit (also the unit of a bare number typed as text).
    pub fn default_unit(self) -> &'static DisplayUnit {
        &self.display_units()[0]
    }

    /// The display unit named `text` (exact match first, then ignoring case).
    /// `None` if this spec has no such unit.
    pub fn unit(self, text: &str) -> Option<&'static DisplayUnit> {
        let units = self.display_units();
        units
            .iter()
            .find(|u| u.matches(text))
            .or_else(|| units.iter().find(|u| u.matches_ignore_case(text)))
    }

    /// Convert `value` given in `unit` to the storage unit. An empty unit
    /// means the storage unit.
    ///
    /// Fails with [`QuantityError::UnknownUnit`] if the spec has no such unit.
    pub fn to_storage(self, value: f64, unit: &str) -> Result<f64, QuantityError> {
        if normalize_unit(unit).is_empty() {
            return Ok(value);
        }
        self.unit(unit)
            .map(|u| u.to_storage(value))
            .ok_or_else(|| QuantityError::UnknownUnit {
                unit: unit.to_string(),
                expected: Some(self),
            })
    }

    /// `true` if two stored values are equal within tolerance: lengths use
    /// `tol::VERTEX_TOL` (mm), angles `tol::ANGLE_TOL` (rad), areas and
    /// volumes the vertex tolerance squared / cubed, everything else
    /// `tol::QUANTITY_REL_TOL` (relative).
    pub fn eq_values(self, a: f64, b: f64) -> bool {
        match self {
            QuantitySpec::Length => tol::eq_len(a, b),
            QuantitySpec::Angle => tol::eq_angle(a, b),
            QuantitySpec::Area => (a - b).abs() <= tol::VERTEX_TOL * tol::VERTEX_TOL,
            QuantitySpec::Volume => (a - b).abs() <= tol::VERTEX_TOL.powi(3),
            _ => tol::eq_quantity(a, b),
        }
    }

    /// Format a stored value in the default display unit with up to six
    /// significant digits, e.g. `"0.8 W/(m·K)"`, `"20 °C"`, `"1200 kg/m³"`.
    pub fn format(self, stored: f64) -> String {
        let u = self.default_unit();
        let n = format_number(u.from_storage(stored));
        match u.symbol {
            "" => n,
            "°" => format!("{n}°"),
            s => format!("{n} {s}"),
        }
    }

    /// Read user / agent text such as `"0.8 W/mK"`, `"1200 kg/m3"`,
    /// `"35 MPa"`, `"20 °C"`, `"3m"` or `"10' 6\""`. Returns the spec and the
    /// value in storage units.
    ///
    /// With `expected`, the unit must belong to that spec and a bare number
    /// is in its [default unit](QuantitySpec::default_unit) (mm for length,
    /// degrees for angle, °C for temperature, like `Length::parse` and
    /// `Angle::parse`). Without `expected`, the spec is found from the unit
    /// (first in [`QuantitySpec::ALL`] order) and a bare number fails with
    /// [`QuantityError::MissingUnit`].
    ///
    /// Fails with [`QuantityError::NotANumber`] if the text does not start
    /// with a finite number, [`QuantityError::UnknownUnit`] if the unit is not
    /// in the table.
    pub fn parse(
        text: &str,
        expected: Option<QuantitySpec>,
    ) -> Result<(QuantitySpec, f64), QuantityError> {
        let text = text.trim();
        let (value, unit) =
            split_number(text).ok_or_else(|| QuantityError::NotANumber(text.to_string()))?;
        let candidates: &[QuantitySpec] = match &expected {
            Some(s) => std::slice::from_ref(s),
            None => &Self::ALL,
        };
        if normalize_unit(unit).is_empty() {
            return match expected {
                Some(s) => Ok((s, s.default_unit().to_storage(value))),
                None => Err(QuantityError::MissingUnit),
            };
        }
        let found = candidates
            .iter()
            .find_map(|s| {
                s.display_units()
                    .iter()
                    .find(|u| u.matches(unit))
                    .map(|u| (*s, u))
            })
            .or_else(|| {
                candidates.iter().find_map(|s| {
                    s.display_units()
                        .iter()
                        .find(|u| u.matches_ignore_case(unit))
                        .map(|u| (*s, u))
                })
            });
        if let Some((s, u)) = found {
            return Ok((s, u.to_storage(value)));
        }
        // Feet-inches (`10' 6"`) are only understood by `Length::parse`.
        if candidates.contains(&QuantitySpec::Length) {
            if let Some(l) = Length::parse(text) {
                return Ok((QuantitySpec::Length, l.mm()));
            }
        }
        Err(QuantityError::UnknownUnit {
            unit: unit.trim().to_string(),
            expected,
        })
    }
}

impl fmt::Display for QuantitySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Split `"0.8 W/mK"` into `(0.8, " W/mK")`: the longest prefix that is a
/// finite number, and the rest. `None` if no prefix is a finite number.
fn split_number(text: &str) -> Option<(f64, &str)> {
    let ends = text
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .collect::<Vec<_>>();
    ends.into_iter().rev().find_map(|end| {
        let v: f64 = text[..end].trim().parse().ok()?;
        v.is_finite().then(|| (v, &text[end..]))
    })
}

/// Unit text without the characters that do not change its meaning.
fn normalize_unit(unit: &str) -> String {
    unit.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '·' | '⋅' | '*' | '(' | ')' | '-' | '.'))
        .map(|c| match c {
            '²' => '2',
            '³' => '3',
            '₂' => '2',
            c => c,
        })
        .collect()
}

/// Up to six significant digits, no trailing zeros, no `-0`
/// (`3.3`, `1200`, `0.035`).
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v.abs() < 1e-12 {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 9) as usize;
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> (QuantitySpec, f64) {
        QuantitySpec::parse(t, None).unwrap()
    }

    #[test]
    fn ids_are_unique_and_match_serde() {
        for s in QuantitySpec::ALL {
            assert_eq!(QuantitySpec::from_id(s.id()), Some(s));
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.id())
            );
            assert!(!s.display_units().is_empty(), "{s}");
            assert!(s.revit_spec().chars().all(|c| c.is_ascii_alphabetic()));
            assert!(s.ifc_measure().starts_with("Ifc"));
        }
        let mut ids: Vec<_> = QuantitySpec::ALL.iter().map(|s| s.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), QuantitySpec::ALL.len());
    }

    #[test]
    fn revit_and_ifc_names_map_back() {
        for s in QuantitySpec::ALL {
            let r = QuantitySpec::from_revit_spec(s.revit_spec()).unwrap();
            assert_eq!(r.revit_spec(), s.revit_spec());
            let i = QuantitySpec::from_ifc_measure(s.ifc_measure()).unwrap();
            assert_eq!(i.ifc_measure(), s.ifc_measure());
        }
        assert_eq!(
            QuantitySpec::from_revit_spec("HeatTransferCoefficient"),
            Some(QuantitySpec::UValue)
        );
        assert_eq!(
            QuantitySpec::from_revit_spec("Mass"),
            Some(QuantitySpec::Mass)
        );
        assert_eq!(QuantitySpec::from_ifc_measure("IfcFooMeasure"), None);
    }

    #[test]
    fn parse_reads_common_building_physics_units() {
        let cases = [
            ("0.8 W/mK", QuantitySpec::ThermalConductivity, 0.8),
            ("0.035 W/(m·K)", QuantitySpec::ThermalConductivity, 0.035),
            ("1200 kg/m3", QuantitySpec::Density, 1200.0),
            ("2400 kg/m³", QuantitySpec::Density, 2400.0),
            ("35 MPa", QuantitySpec::Stress, 35e6),
            ("35 N/mm2", QuantitySpec::Stress, 35e6),
            ("0.25 W/m2K", QuantitySpec::UValue, 0.25),
            ("4 m²K/W", QuantitySpec::ThermalResistance, 4.0),
            ("12 kN", QuantitySpec::Force, 12e3),
            ("3.5 kN·m", QuantitySpec::Moment, 3.5e3),
            ("120 kgCO2e", QuantitySpec::CarbonMass, 120.0),
            ("1.2 tCO2e", QuantitySpec::CarbonMass, 1200.0),
            ("500 lx", QuantitySpec::Illuminance, 500.0),
            ("2.5 kW", QuantitySpec::Power, 2500.0),
            ("101 kPa", QuantitySpec::Pressure, 101e3),
            ("10 L/s", QuantitySpec::Flow, 0.01),
            ("50%", QuantitySpec::Ratio, 0.5),
            ("2 h", QuantitySpec::Time, 7200.0),
            ("880 J/kgK", QuantitySpec::HeatCapacity, 880.0),
            ("3m", QuantitySpec::Length, 3000.0),
            ("12 m2", QuantitySpec::Area, 12e6),
            ("1.5 m3", QuantitySpec::Volume, 1.5e9),
            ("90°", QuantitySpec::Angle, std::f64::consts::FRAC_PI_2),
            ("-1.5e3 kg", QuantitySpec::Mass, -1500.0),
        ];
        for (t, spec, v) in cases {
            let (s, x) = parse(t);
            assert_eq!(s, spec, "{t}");
            assert!(s.eq_values(x, v), "{t}: {x} != {v}");
        }
    }

    #[test]
    fn parse_converts_temperature_with_offset() {
        let (s, k) = parse("20 °C");
        assert_eq!(s, QuantitySpec::Temperature);
        assert!(tol::eq_quantity(k, 293.15));
        let (_, k) = parse("68 °F");
        assert!(tol::eq_quantity(k, 293.15));
        assert_eq!(QuantitySpec::Temperature.format(293.15), "20 °C");
    }

    #[test]
    fn parse_with_expected_spec_uses_its_default_unit_for_bare_numbers() {
        let p = |t, s| QuantitySpec::parse(t, Some(s)).unwrap().1;
        assert!(tol::eq_quantity(
            p("0.8", QuantitySpec::ThermalConductivity),
            0.8
        ));
        assert!(tol::eq_quantity(p("20", QuantitySpec::Temperature), 293.15));
        assert!(tol::eq_len(p("3000", QuantitySpec::Length), 3000.0));
        assert!(tol::eq_len(p("10' 6\"", QuantitySpec::Length), 3200.4));
        // `Pa` is pressure by default but stress when expected.
        assert_eq!(parse("5 Pa").0, QuantitySpec::Pressure);
        assert_eq!(
            QuantitySpec::parse("5 Pa", Some(QuantitySpec::Stress))
                .unwrap()
                .0,
            QuantitySpec::Stress
        );
    }

    #[test]
    fn parse_is_case_sensitive_first_then_lenient() {
        // `W/mk` has no exact match, so the case-insensitive pass finds W/mK.
        assert_eq!(parse("0.8 W/mk").0, QuantitySpec::ThermalConductivity);
        assert!(tol::eq_quantity(parse("35 mpa").1, 35e6));
    }

    #[test]
    fn parse_rejects_degenerate_input() {
        let e = |t, s| QuantitySpec::parse(t, s).unwrap_err();
        assert!(matches!(e("", None), QuantityError::NotANumber(_)));
        assert!(matches!(e("W/mK", None), QuantityError::NotANumber(_)));
        assert!(matches!(e("inf W", None), QuantityError::NotANumber(_)));
        assert!(matches!(e("nan kg", None), QuantityError::NotANumber(_)));
        assert_eq!(e("0.8", None), QuantityError::MissingUnit);
        assert!(matches!(
            e("3 furlongs", None),
            QuantityError::UnknownUnit { .. }
        ));
        // A unit of another spec is rejected when a spec is expected.
        let err = e("35 MPa", Some(QuantitySpec::Density));
        assert!(err.to_string().contains("kg/m³"), "{err}");
    }

    #[test]
    fn display_units_round_trip_at_tolerance_edge() {
        for s in QuantitySpec::ALL {
            for u in s.display_units() {
                let stored = u.to_storage(1.234_567);
                assert!(
                    (u.from_storage(stored) - 1.234_567).abs() <= 1e-9,
                    "{s} {}",
                    u.symbol
                );
            }
        }
        // Inside / outside the relative tolerance.
        let d = QuantitySpec::Density;
        assert!(d.eq_values(1200.0, 1200.0 * (1.0 + 0.5 * tol::QUANTITY_REL_TOL)));
        assert!(!d.eq_values(1200.0, 1200.0 * (1.0 + 2.0 * tol::QUANTITY_REL_TOL)));
        assert!(QuantitySpec::Length.eq_values(1000.0, 1000.0 + tol::VERTEX_TOL));
        assert!(!QuantitySpec::Length.eq_values(1000.0, 1000.0 + 2.0 * tol::VERTEX_TOL));
    }

    #[test]
    fn format_uses_default_display_unit() {
        assert_eq!(QuantitySpec::ThermalConductivity.format(0.8), "0.8 W/(m·K)");
        assert_eq!(QuantitySpec::Density.format(1200.0), "1200 kg/m³");
        assert_eq!(QuantitySpec::Stress.format(35e6), "35 MPa");
        assert_eq!(QuantitySpec::Area.format(12.5e6), "12.5 m²");
        assert_eq!(
            QuantitySpec::Angle.format(std::f64::consts::FRAC_PI_2),
            "90°"
        );
        assert_eq!(QuantitySpec::Ratio.format(0.5), "0.5");
        assert_eq!(QuantitySpec::Number.format(-0.0), "0");
        assert_eq!(QuantitySpec::Length.format(1.0 / 3.0), "0.333333 mm");
    }

    #[test]
    fn to_storage_checks_the_unit() {
        let s = QuantitySpec::Density;
        assert!(s.to_storage(1.2, "t/m3").is_err());
        assert!(s.to_storage(1.0, "MPa").is_err());
        assert!(tol::eq_quantity(s.to_storage(1200.0, "").unwrap(), 1200.0));
        assert!(tol::eq_quantity(
            QuantitySpec::Stress.to_storage(35.0, "MPa").unwrap(),
            35e6
        ));
    }
}
