//! Temporary polygon helpers used by the v0.1 prototype model.
//! Replaced by the full geometry layer (`Plane`, `CurveLoop`, `Profile`) in M1-2.

use crate::math::{Point3, UnitVec3, Vec3};

/// Newell's method: robust unit normal of a planar polygon, following the
/// counter-clockwise vertex order. `None` for degenerate polygons.
pub fn polygon_normal(pts: &[Point3]) -> Option<UnitVec3> {
    let mut n = Vec3::ZERO;
    for (i, a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n.normalize()
}

/// Area of a planar polygon (mm²), independent of orientation.
pub fn polygon_area(pts: &[Point3]) -> f64 {
    let Some(&first) = pts.first() else {
        return 0.0;
    };
    let mut s = Vec3::ZERO;
    for i in 1..pts.len().saturating_sub(1) {
        s += (pts[i] - first).cross(pts[i + 1] - first);
    }
    s.length() * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Point3> {
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(2000.0, 0.0, 0.0),
            Point3::new(2000.0, 2000.0, 0.0),
            Point3::new(0.0, 2000.0, 0.0),
        ]
    }

    #[test]
    fn ccw_square_normal_points_up() {
        let n = polygon_normal(&square()).unwrap();
        assert!(n.as_vec().is_almost_equal(Vec3::Z));
    }

    #[test]
    fn area_of_square() {
        assert!((polygon_area(&square()) - 4_000_000.0).abs() < 1e-6);
    }

    #[test]
    fn collinear_points_are_degenerate() {
        let line = [
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        ];
        assert!(polygon_normal(&line).is_none());
        assert!(polygon_area(&line).abs() < 1e-9);
    }
}
