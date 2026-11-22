//! Quantities with units fixed by the spec (D-01, D-02). Unit conversion happens
//! only at the edges (UI, MCP, files); the kernel stores mm and radians.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A length in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Length(f64);

impl Length {
    pub const ZERO: Length = Length(0.0);

    pub const fn from_mm(mm: f64) -> Self {
        Length(mm)
    }
    pub fn from_m(m: f64) -> Self {
        Length(m * 1000.0)
    }
    pub fn from_feet(ft: f64) -> Self {
        Length(ft * 304.8)
    }
    pub fn mm(self) -> f64 {
        self.0
    }
    pub fn m(self) -> f64 {
        self.0 / 1000.0
    }
    pub fn feet(self) -> f64 {
        self.0 / 304.8
    }
    pub fn abs(self) -> Length {
        Length(self.0.abs())
    }

    /// Parse user / agent input. A bare number means millimetres.
    ///
    /// Accepts `3000`, `3m`, `300cm`, `3000mm`, `10'`, `6"`, `10in`, `10ft`,
    /// and feet-inches such as `10' 6"` or `10'6"`. Returns `None` if the text
    /// is not a length.
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            return None;
        }
        // Feet-inches: 10' 6"
        if let Some((ft, rest)) = s.split_once('\'') {
            let rest = rest.trim();
            if !rest.is_empty() {
                let inch = rest.strip_suffix('"').or_else(|| rest.strip_suffix("in"))?;
                let ft: f64 = ft.trim().parse().ok()?;
                let inch: f64 = inch.trim().parse().ok()?;
                return Some(Length((ft * 12.0 + inch) * 25.4));
            }
        }
        let (num, factor) = if let Some(n) = s.strip_suffix("mm") {
            (n, 1.0)
        } else if let Some(n) = s.strip_suffix("cm") {
            (n, 10.0)
        } else if let Some(n) = s.strip_suffix('m') {
            (n, 1000.0)
        } else if let Some(n) = s.strip_suffix('"').or_else(|| s.strip_suffix("in")) {
            (n, 25.4)
        } else if let Some(n) = s.strip_suffix('\'').or_else(|| s.strip_suffix("ft")) {
            (n, 304.8)
        } else {
            (s.as_str(), 1.0)
        };
        let v: f64 = num.trim().parse().ok()?;
        v.is_finite().then_some(Length(v * factor))
    }
}

impl Add for Length {
    type Output = Length;
    fn add(self, o: Length) -> Length {
        Length(self.0 + o.0)
    }
}
impl Sub for Length {
    type Output = Length;
    fn sub(self, o: Length) -> Length {
        Length(self.0 - o.0)
    }
}
impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, s: f64) -> Length {
        Length(self.0 * s)
    }
}
impl Div<f64> for Length {
    type Output = Length;
    fn div(self, s: f64) -> Length {
        Length(self.0 / s)
    }
}
impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length {
        Length(-self.0)
    }
}

/// An angle in radians. Positive is counter-clockwise (D-03).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Angle(f64);

impl Angle {
    pub const ZERO: Angle = Angle(0.0);

    pub const fn from_rad(rad: f64) -> Self {
        Angle(rad)
    }
    pub fn from_deg(deg: f64) -> Self {
        Angle(deg.to_radians())
    }
    pub fn rad(self) -> f64 {
        self.0
    }
    pub fn deg(self) -> f64 {
        self.0.to_degrees()
    }

    /// Parse agent / user input in degrees (`"90"`, `"90deg"`, `"90°"`) or
    /// radians (`"1.57rad"`).
    pub fn parse(s: &str) -> Option<Angle> {
        let s = s.trim().to_lowercase();
        let (num, rad) = if let Some(n) = s.strip_suffix("rad") {
            (n, true)
        } else if let Some(n) = s.strip_suffix("deg").or_else(|| s.strip_suffix('°')) {
            (n, false)
        } else {
            (s.as_str(), false)
        };
        let v: f64 = num.trim().parse().ok()?;
        v.is_finite()
            .then(|| if rad { Angle(v) } else { Angle::from_deg(v) })
    }
}

impl Add for Angle {
    type Output = Angle;
    fn add(self, o: Angle) -> Angle {
        Angle(self.0 + o.0)
    }
}
impl Sub for Angle {
    type Output = Angle;
    fn sub(self, o: Angle) -> Angle {
        Angle(self.0 - o.0)
    }
}
impl Neg for Angle {
    type Output = Angle;
    fn neg(self) -> Angle {
        Angle(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::tol;

    #[test]
    fn length_parses_common_units() {
        assert!(tol::eq_len(Length::parse("3000").unwrap().mm(), 3000.0));
        assert!(tol::eq_len(Length::parse("3m").unwrap().mm(), 3000.0));
        assert!(tol::eq_len(Length::parse("300 cm").unwrap().mm(), 3000.0));
        assert!(tol::eq_len(Length::parse("10'").unwrap().mm(), 3048.0));
        assert!(tol::eq_len(Length::parse("10' 6\"").unwrap().mm(), 3200.4));
    }

    #[test]
    fn length_parse_rejects_garbage() {
        for bad in ["", "abc", "m", "3mx", "nan", "inf"] {
            assert!(Length::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn length_serializes_as_plain_mm() {
        assert_eq!(
            serde_json::to_string(&Length::from_m(2.8)).unwrap(),
            "2800.0"
        );
    }

    #[test]
    fn angle_parse_defaults_to_degrees() {
        assert!(tol::eq_angle(
            Angle::parse("90").unwrap().rad(),
            std::f64::consts::FRAC_PI_2
        ));
        assert!(tol::eq_angle(Angle::parse("1.5rad").unwrap().rad(), 1.5));
        assert!(Angle::parse("ninety").is_none());
    }
}
