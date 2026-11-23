use super::tol;
use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A displacement or direction with length, in millimetres (D-07).
/// Not a location: see [`super::Point3`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    /// `true` if the length is zero within [`tol::VERTEX_TOL`].
    pub fn is_zero(self) -> bool {
        tol::is_zero_len(self.length())
    }

    /// Unit direction, or `None` if the vector is (near) zero.
    pub fn normalize(self) -> Option<UnitVec3> {
        let l = self.length();
        // A direction needs a meaningful magnitude; use a tiny absolute floor so
        // that short but valid vectors (e.g. 0.001 mm) still normalise.
        (l > 1e-12).then(|| UnitVec3(self / l))
    }

    /// Angle to another vector in radians, in [0, π]. `None` if either is zero.
    pub fn angle_to(self, o: Vec3) -> Option<f64> {
        let (a, b) = (self.normalize()?, o.normalize()?);
        Some(a.dot(b).clamp(-1.0, 1.0).acos())
    }

    /// Parallel or anti-parallel within [`tol::ANGLE_TOL`]. `false` if either is zero.
    pub fn is_parallel(self, o: Vec3) -> bool {
        match (self.normalize(), o.normalize()) {
            (Some(a), Some(b)) => a.as_vec().cross(b.as_vec()).length() <= tol::ANGLE_TOL,
            _ => false,
        }
    }

    /// Perpendicular within [`tol::ANGLE_TOL`]. `false` if either is zero.
    pub fn is_perpendicular(self, o: Vec3) -> bool {
        match (self.normalize(), o.normalize()) {
            (Some(a), Some(b)) => a.dot(b).abs() <= tol::ANGLE_TOL,
            _ => false,
        }
    }

    /// Equal component-wise within [`tol::VERTEX_TOL`].
    pub fn is_almost_equal(self, o: Vec3) -> bool {
        (self - o).length() <= tol::VERTEX_TOL
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A direction of length 1. Only constructible through [`Vec3::normalize`] or
/// the axis constants, so the invariant always holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(into = "Vec3", try_from = "Vec3")]
pub struct UnitVec3(Vec3);

impl UnitVec3 {
    pub const X: UnitVec3 = UnitVec3(Vec3::X);
    pub const Y: UnitVec3 = UnitVec3(Vec3::Y);
    pub const Z: UnitVec3 = UnitVec3(Vec3::Z);

    pub fn as_vec(self) -> Vec3 {
        self.0
    }

    pub fn dot(self, o: UnitVec3) -> f64 {
        self.0.dot(o.0)
    }

    /// Cross product of two unit vectors; `None` if they are parallel.
    pub fn cross(self, o: UnitVec3) -> Option<UnitVec3> {
        self.0.cross(o.0).normalize()
    }

    pub fn is_parallel(self, o: UnitVec3) -> bool {
        self.0.is_parallel(o.0)
    }
}

impl From<UnitVec3> for Vec3 {
    fn from(u: UnitVec3) -> Vec3 {
        u.0
    }
}

impl TryFrom<Vec3> for UnitVec3 {
    type Error = &'static str;
    /// Already-unit vectors (as written by Ferro) are kept bit for bit so
    /// files round-trip exactly; anything else is normalised.
    fn try_from(v: Vec3) -> Result<Self, Self::Error> {
        if (v.length_squared() - 1.0).abs() < 1e-12 {
            return Ok(UnitVec3(v));
        }
        v.normalize().ok_or("zero-length vector is not a direction")
    }
}

impl Neg for UnitVec3 {
    type Output = UnitVec3;
    fn neg(self) -> UnitVec3 {
        UnitVec3(-self.0)
    }
}

impl Mul<f64> for UnitVec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        self.0 * s
    }
}

/// A 2D displacement in a sketch or parameter space (Revit: `UV`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    /// Z component of the 3D cross product (signed parallelogram area).
    pub fn perp_dot(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }
    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_gives_unit_length() {
        let u = Vec3::new(3.0, 4.0, 0.0).normalize().unwrap();
        assert!((u.as_vec().length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn normalize_rejects_zero_vector() {
        assert!(Vec3::ZERO.normalize().is_none());
    }

    #[test]
    fn short_but_valid_vector_still_normalizes() {
        assert!(Vec3::new(0.001, 0.0, 0.0).normalize().is_some());
    }

    #[test]
    fn cross_follows_right_hand_rule() {
        assert!(Vec3::X.cross(Vec3::Y).is_almost_equal(Vec3::Z));
    }

    #[test]
    fn parallel_detects_opposite_directions() {
        assert!(Vec3::new(1.0, 2.0, 3.0).is_parallel(Vec3::new(-2.0, -4.0, -6.0)));
        assert!(!Vec3::X.is_parallel(Vec3::Y));
        assert!(!Vec3::ZERO.is_parallel(Vec3::X));
    }

    #[test]
    fn parallel_tolerance_edge() {
        let almost = Vec3::new(1.0, tol::ANGLE_TOL * 0.5, 0.0);
        let off = Vec3::new(1.0, tol::ANGLE_TOL * 10.0, 0.0);
        assert!(Vec3::X.is_parallel(almost));
        assert!(!Vec3::X.is_parallel(off));
    }

    #[test]
    fn unit_vec_roundtrips_through_serde_and_rejects_zero() {
        let json = serde_json::to_string(&UnitVec3::Z).unwrap();
        let back: UnitVec3 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, UnitVec3::Z);
        assert!(serde_json::from_str::<UnitVec3>(r#"{"x":0,"y":0,"z":0}"#).is_err());
    }
}
