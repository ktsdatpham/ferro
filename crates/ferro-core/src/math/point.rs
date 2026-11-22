use super::{tol, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Sub, SubAssign};

/// A location in model space, millimetres from the Internal Origin (D-04, D-07).
///
/// | Expression | Result |
/// |---|---|
/// | `Point3 - Point3` | `Vec3` |
/// | `Point3 ± Vec3` | `Point3` |
/// | `Point3 + Point3` | does not compile |
///
/// ```compile_fail
/// use ferro_core::Point3;
/// let _ = Point3::ORIGIN + Point3::ORIGIN; // adding two locations is meaningless
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    /// The Internal Origin.
    pub const ORIGIN: Point3 = Point3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Vector from the origin to this point.
    pub fn to_vec(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    pub fn distance_to(self, o: Point3) -> f64 {
        (self - o).length()
    }

    /// Same point within [`tol::VERTEX_TOL`].
    pub fn is_almost_equal(self, o: Point3) -> bool {
        self.distance_to(o) <= tol::VERTEX_TOL
    }

    pub fn midpoint(a: Point3, b: Point3) -> Point3 {
        a + (b - a) * 0.5
    }

    /// Arithmetic mean of the points; `None` for an empty slice.
    pub fn centroid(points: &[Point3]) -> Option<Point3> {
        let first = *points.first()?;
        let sum = points.iter().fold(Vec3::ZERO, |acc, p| acc + (*p - first));
        Some(first + sum / points.len() as f64)
    }
}

impl Sub for Point3 {
    type Output = Vec3;
    fn sub(self, o: Point3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Add<Vec3> for Point3 {
    type Output = Point3;
    fn add(self, v: Vec3) -> Point3 {
        Point3::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}
impl AddAssign<Vec3> for Point3 {
    fn add_assign(&mut self, v: Vec3) {
        *self = *self + v;
    }
}
impl Sub<Vec3> for Point3 {
    type Output = Point3;
    fn sub(self, v: Vec3) -> Point3 {
        Point3::new(self.x - v.x, self.y - v.y, self.z - v.z)
    }
}
impl SubAssign<Vec3> for Point3 {
    fn sub_assign(&mut self, v: Vec3) {
        *self = *self - v;
    }
}

/// A location in a 2D sketch or parameter space (Revit: `UV`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Point2 {
    pub const ORIGIN: Point2 = Point2::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn distance_to(self, o: Point2) -> f64 {
        (self - o).length()
    }
    pub fn is_almost_equal(self, o: Point2) -> bool {
        self.distance_to(o) <= tol::VERTEX_TOL
    }
}

impl Sub for Point2 {
    type Output = Vec2;
    fn sub(self, o: Point2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Add<Vec2> for Point2 {
    type Output = Point2;
    fn add(self, v: Vec2) -> Point2 {
        Point2::new(self.x + v.x, self.y + v.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_minus_point_is_vector() {
        let v: Vec3 = Point3::new(5.0, 7.0, 9.0) - Point3::new(1.0, 2.0, 3.0);
        assert!(v.is_almost_equal(Vec3::new(4.0, 5.0, 6.0)));
    }

    #[test]
    fn centroid_of_square_is_center() {
        let sq = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4000.0, 0.0, 0.0),
            Point3::new(4000.0, 2000.0, 0.0),
            Point3::new(0.0, 2000.0, 0.0),
        ];
        assert!(Point3::centroid(&sq)
            .unwrap()
            .is_almost_equal(Point3::new(2000.0, 1000.0, 0.0)));
    }

    #[test]
    fn centroid_of_empty_slice_is_none() {
        assert!(Point3::centroid(&[]).is_none());
    }

    #[test]
    fn far_coordinates_keep_precision_within_extent() {
        // D-05: 10 km extent. Midpoint far from origin must stay exact to tolerance.
        let a = Point3::new(9_999_000.0, -9_999_000.0, 0.0);
        let b = a + Vec3::new(0.02, 0.0, 0.0);
        let m = Point3::midpoint(a, b);
        assert!(m.is_almost_equal(a + Vec3::new(0.01, 0.0, 0.0)));
    }

    #[test]
    fn almost_equal_tolerance_edge() {
        let a = Point3::ORIGIN;
        assert!(a.is_almost_equal(Point3::new(tol::VERTEX_TOL * 0.9, 0.0, 0.0)));
        assert!(!a.is_almost_equal(Point3::new(tol::VERTEX_TOL * 1.1, 0.0, 0.0)));
    }
}
