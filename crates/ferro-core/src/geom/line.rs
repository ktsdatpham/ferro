use super::GeomError;
use crate::math::{tol, BoundingBox3, Point3, Transform, UnitVec3};
use serde::{Deserialize, Serialize};

/// A bounded straight segment (D-09). Revit: `Line.CreateBound`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Line {
    start: Point3,
    end: Point3,
}

impl Line {
    /// Fails with `ShortCurve` if the segment is shorter than `SHORT_CURVE_TOL`.
    pub fn from_points(start: Point3, end: Point3) -> Result<Line, GeomError> {
        if start.distance_to(end) < tol::SHORT_CURVE_TOL {
            return Err(GeomError::ShortCurve);
        }
        Ok(Line { start, end })
    }

    pub fn start(&self) -> Point3 {
        self.start
    }
    pub fn end(&self) -> Point3 {
        self.end
    }
    pub fn length(&self) -> f64 {
        self.start.distance_to(self.end)
    }
    pub fn direction(&self) -> UnitVec3 {
        (self.end - self.start)
            .normalize()
            .expect("lines are longer than SHORT_CURVE_TOL")
    }

    /// Point at normalised parameter `t` ∈ [0, 1].
    pub fn point_at(&self, t: f64) -> Point3 {
        self.start + (self.end - self.start) * t
    }

    /// Parameter of the closest point, clamped to [0, 1].
    pub fn closest_param(&self, p: Point3) -> f64 {
        let d = self.end - self.start;
        ((p - self.start).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    }

    pub fn reversed(&self) -> Line {
        Line {
            start: self.end,
            end: self.start,
        }
    }

    pub fn transformed(&self, t: &Transform) -> Result<Line, GeomError> {
        Line::from_points(t.apply(self.start), t.apply(self.end))
    }

    pub fn bounding_box(&self) -> BoundingBox3 {
        BoundingBox3::from_points([self.start, self.end]).expect("two points")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_curve() {
        let a = Point3::ORIGIN;
        assert_eq!(
            Line::from_points(a, Point3::new(tol::SHORT_CURVE_TOL * 0.9, 0.0, 0.0)),
            Err(GeomError::ShortCurve)
        );
        assert!(Line::from_points(a, Point3::new(tol::SHORT_CURVE_TOL, 0.0, 0.0)).is_ok());
    }

    #[test]
    fn point_at_and_closest_param() {
        let l = Line::from_points(Point3::ORIGIN, Point3::new(4000.0, 0.0, 0.0)).unwrap();
        assert!(l
            .point_at(0.25)
            .is_almost_equal(Point3::new(1000.0, 0.0, 0.0)));
        assert!(tol::eq_param(
            l.closest_param(Point3::new(3000.0, 50.0, 0.0)),
            0.75
        ));
        assert!(tol::eq_param(
            l.closest_param(Point3::new(-10.0, 0.0, 0.0)),
            0.0
        ));
    }
}
