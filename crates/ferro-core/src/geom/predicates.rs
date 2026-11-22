//! Exact geometric predicates (D-53). Every topological decision (left / right,
//! inside / outside) goes through these, never through raw `f64` comparisons.

use crate::math::Point2;
use robust::Coord;

fn c(p: Point2) -> Coord<f64> {
    Coord { x: p.x, y: p.y }
}

/// Sign of the orientation of `(a, b, c)`: `> 0` counter-clockwise (c left of
/// a→b), `< 0` clockwise, `0` exactly collinear. Exact (Shewchuk).
pub fn orient2d(a: Point2, b: Point2, p: Point2) -> f64 {
    robust::orient2d(c(a), c(b), c(p))
}

/// Exact point-in-polygon (crossing number with exact orientation tests).
/// Points exactly on the boundary count as inside.
pub fn point_in_polygon(p: Point2, poly: &[Point2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        // On-segment check (exact collinearity + bounding box).
        if orient2d(a, b, p) == 0.0
            && p.x >= a.x.min(b.x)
            && p.x <= a.x.max(b.x)
            && p.y >= a.y.min(b.y)
            && p.y <= a.y.max(b.y)
        {
            return true;
        }
        let upward = a.y <= p.y && b.y > p.y;
        let downward = b.y <= p.y && a.y > p.y;
        if upward && orient2d(a, b, p) > 0.0 || downward && orient2d(a, b, p) < 0.0 {
            inside = !inside;
        }
    }
    inside
}

/// Twice the signed area of a 2D polygon (positive = counter-clockwise).
pub fn signed_area2(poly: &[Point2]) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq() -> Vec<Point2> {
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(10.0, 10.0),
            Point2::new(0.0, 10.0),
        ]
    }

    #[test]
    fn orientation_signs() {
        let (a, b) = (Point2::new(0.0, 0.0), Point2::new(1.0, 0.0));
        assert!(orient2d(a, b, Point2::new(0.5, 1.0)) > 0.0);
        assert!(orient2d(a, b, Point2::new(0.5, -1.0)) < 0.0);
        assert_eq!(orient2d(a, b, Point2::new(7.0, 0.0)), 0.0);
    }

    #[test]
    fn orientation_is_exact_for_nearly_collinear_points() {
        // A classic case where naive f64 evaluation gives the wrong sign.
        let a = Point2::new(0.5, 0.5);
        let b = Point2::new(12.0, 12.0);
        let p = Point2::new(24.0, 24.0 + f64::EPSILON * 24.0);
        assert!(orient2d(a, b, p) > 0.0);
    }

    #[test]
    fn point_in_polygon_inside_outside_and_boundary() {
        assert!(point_in_polygon(Point2::new(5.0, 5.0), &sq()));
        assert!(!point_in_polygon(Point2::new(15.0, 5.0), &sq()));
        assert!(point_in_polygon(Point2::new(10.0, 5.0), &sq()));
        assert!(point_in_polygon(Point2::new(0.0, 0.0), &sq()));
    }

    #[test]
    fn signed_area_sign_follows_orientation() {
        assert!(signed_area2(&sq()) > 0.0);
        let mut cw = sq();
        cw.reverse();
        assert!(signed_area2(&cw) < 0.0);
    }
}
