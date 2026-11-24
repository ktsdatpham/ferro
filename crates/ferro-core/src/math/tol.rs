//! Global tolerances (D-06). Never compare `f64` with `==`; use these.

/// Two points closer than this (mm) are the same point.
pub const VERTEX_TOL: f64 = 0.01;
/// Shortest allowed edge or curve (mm). Revit: `ShortCurveTolerance` ≈ 0.8 mm.
pub const SHORT_CURVE_TOL: f64 = 0.5;
/// Two directions within this angle (radians) are parallel.
pub const ANGLE_TOL: f64 = 1e-6;
/// Curve parameter equality on the normalised range [0, 1].
pub const PARAM_TOL: f64 = 1e-9;
/// Relative tolerance for quantity values other than lengths and angles
/// (density, stress, U-value…, ADR 0004 §2). Unitless.
pub const QUANTITY_REL_TOL: f64 = 1e-9;

/// `true` if two lengths (mm) are equal within [`VERTEX_TOL`].
pub fn eq_len(a: f64, b: f64) -> bool {
    (a - b).abs() <= VERTEX_TOL
}

/// `true` if a length (mm) is zero within [`VERTEX_TOL`].
pub fn is_zero_len(a: f64) -> bool {
    a.abs() <= VERTEX_TOL
}

/// `true` if two angles (radians) are equal within [`ANGLE_TOL`].
pub fn eq_angle(a: f64, b: f64) -> bool {
    (a - b).abs() <= ANGLE_TOL
}

/// `true` if two normalised curve parameters are equal within [`PARAM_TOL`].
pub fn eq_param(a: f64, b: f64) -> bool {
    (a - b).abs() <= PARAM_TOL
}

/// `true` if two quantity values in the same storage unit are equal within
/// [`QUANTITY_REL_TOL`] relative to the larger magnitude (absolute below 1).
pub fn eq_quantity(a: f64, b: f64) -> bool {
    (a - b).abs() <= QUANTITY_REL_TOL * a.abs().max(b.abs()).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_len_accepts_values_inside_tolerance() {
        assert!(eq_len(1000.0, 1000.0 + VERTEX_TOL * 0.5));
    }

    #[test]
    fn eq_len_rejects_values_outside_tolerance() {
        assert!(!eq_len(1000.0, 1000.0 + VERTEX_TOL * 2.0));
    }

    #[test]
    fn eq_len_boundary_is_inclusive() {
        assert!(eq_len(0.0, VERTEX_TOL));
        assert!(is_zero_len(-VERTEX_TOL));
    }

    #[test]
    fn eq_quantity_is_relative_for_large_values() {
        assert!(eq_quantity(35e6, 35e6 * (1.0 + 0.5 * QUANTITY_REL_TOL)));
        assert!(!eq_quantity(35e6, 35e6 * (1.0 + 2.0 * QUANTITY_REL_TOL)));
        assert!(eq_quantity(0.0, QUANTITY_REL_TOL));
        assert!(!eq_quantity(0.0, 2.0 * QUANTITY_REL_TOL));
    }
}
