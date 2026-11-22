use super::{tol, Angle, Point3, UnitVec3, Vec3};
use serde::{Deserialize, Serialize};

/// Affine transform: 3×3 linear part (columns = images of X, Y, Z) plus a
/// translation. Revit: `Transform` with `BasisX/Y/Z` and `Origin`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub basis_x: Vec3,
    pub basis_y: Vec3,
    pub basis_z: Vec3,
    pub origin: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        basis_x: Vec3::X,
        basis_y: Vec3::Y,
        basis_z: Vec3::Z,
        origin: Vec3::ZERO,
    };

    pub fn translation(v: Vec3) -> Transform {
        Transform {
            origin: v,
            ..Self::IDENTITY
        }
    }

    /// Rotation by `angle` (counter-clockwise, right-hand rule) about the axis
    /// through `axis_origin` with direction `axis`.
    pub fn rotation(axis_origin: Point3, axis: UnitVec3, angle: Angle) -> Transform {
        let k = axis.as_vec();
        let (s, c) = angle.rad().sin_cos();
        // Rodrigues: R v = v c + (k × v) s + k (k·v)(1 − c)
        let rot = |v: Vec3| v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c));
        let linear = Transform {
            basis_x: rot(Vec3::X),
            basis_y: rot(Vec3::Y),
            basis_z: rot(Vec3::Z),
            origin: Vec3::ZERO,
        };
        Self::about(axis_origin, linear)
    }

    /// Uniform scale by `factor` about `center`.
    pub fn scale_about(center: Point3, factor: f64) -> Transform {
        let linear = Transform {
            basis_x: Vec3::X * factor,
            basis_y: Vec3::Y * factor,
            basis_z: Vec3::Z * factor,
            origin: Vec3::ZERO,
        };
        Self::about(center, linear)
    }

    /// Reflection across the plane through `origin` with normal `normal`.
    pub fn mirror(origin: Point3, normal: UnitVec3) -> Transform {
        let n = normal.as_vec();
        let refl = |v: Vec3| v - n * (2.0 * v.dot(n));
        let linear = Transform {
            basis_x: refl(Vec3::X),
            basis_y: refl(Vec3::Y),
            basis_z: refl(Vec3::Z),
            origin: Vec3::ZERO,
        };
        Self::about(origin, linear)
    }

    /// Conjugate a linear transform so it acts about `center`.
    fn about(center: Point3, linear: Transform) -> Transform {
        let c = center.to_vec();
        Transform {
            origin: c - linear.apply_vec(c),
            ..linear
        }
    }

    pub fn apply(&self, p: Point3) -> Point3 {
        Point3::ORIGIN + self.apply_vec(p.to_vec()) + self.origin
    }

    /// Transform a vector (no translation).
    pub fn apply_vec(&self, v: Vec3) -> Vec3 {
        self.basis_x * v.x + self.basis_y * v.y + self.basis_z * v.z
    }

    /// Transform a direction; `None` if the transform collapses it.
    pub fn apply_dir(&self, d: UnitVec3) -> Option<UnitVec3> {
        self.apply_vec(d.as_vec()).normalize()
    }

    /// Composition: apply `self`, then `next`.
    pub fn then(&self, next: &Transform) -> Transform {
        Transform {
            basis_x: next.apply_vec(self.basis_x),
            basis_y: next.apply_vec(self.basis_y),
            basis_z: next.apply_vec(self.basis_z),
            origin: next.apply_vec(self.origin) + next.origin,
        }
    }

    pub fn determinant(&self) -> f64 {
        self.basis_x.dot(self.basis_y.cross(self.basis_z))
    }

    pub fn has_reflection(&self) -> bool {
        self.determinant() < 0.0
    }

    /// Angle- and shape-preserving (rotation, uniform scale, reflection,
    /// translation; no shear or non-uniform scale).
    pub fn is_conformal(&self) -> bool {
        let (x, y, z) = (self.basis_x, self.basis_y, self.basis_z);
        let lx = x.length();
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * lx.max(1.0);
        lx > 0.0
            && close(y.length(), lx)
            && close(z.length(), lx)
            && x.dot(y).abs() <= 1e-9 * lx * lx
            && y.dot(z).abs() <= 1e-9 * lx * lx
            && z.dot(x).abs() <= 1e-9 * lx * lx
    }

    /// Inverse transform; `None` if the transform is singular.
    pub fn inverse(&self) -> Option<Transform> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c) = (self.basis_x, self.basis_y, self.basis_z);
        // Rows of the inverse linear part are the cross products / det.
        let r0 = b.cross(c) / det;
        let r1 = c.cross(a) / det;
        let r2 = a.cross(b) / det;
        let linear = Transform {
            basis_x: Vec3::new(r0.x, r1.x, r2.x),
            basis_y: Vec3::new(r0.y, r1.y, r2.y),
            basis_z: Vec3::new(r0.z, r1.z, r2.z),
            origin: Vec3::ZERO,
        };
        Some(Transform {
            origin: -linear.apply_vec(self.origin),
            ..linear
        })
    }

    /// Same transform within tolerance (basis within `ANGLE_TOL`-scale, origin
    /// within `VERTEX_TOL`).
    pub fn is_almost_equal(&self, o: &Transform) -> bool {
        let b = |a: Vec3, b: Vec3| (a - b).length() <= tol::ANGLE_TOL;
        b(self.basis_x, o.basis_x)
            && b(self.basis_y, o.basis_y)
            && b(self.basis_z, o.basis_z)
            && self.origin.is_almost_equal(o.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_quarter_turn_about_z_through_point() {
        let t = Transform::rotation(
            Point3::new(1000.0, 0.0, 0.0),
            UnitVec3::Z,
            Angle::from_deg(90.0),
        );
        let p = t.apply(Point3::new(2000.0, 0.0, 500.0));
        assert!(p.is_almost_equal(Point3::new(1000.0, 1000.0, 500.0)));
    }

    #[test]
    fn then_applies_left_first() {
        let a = Transform::translation(Vec3::new(1000.0, 0.0, 0.0));
        let b = Transform::rotation(Point3::ORIGIN, UnitVec3::Z, Angle::from_deg(90.0));
        let p = a.then(&b).apply(Point3::ORIGIN);
        assert!(p.is_almost_equal(Point3::new(0.0, 1000.0, 0.0)));
    }

    #[test]
    fn inverse_roundtrips() {
        let t = Transform::rotation(
            Point3::new(5.0, -3.0, 2.0),
            Vec3::new(1.0, 1.0, 1.0).normalize().unwrap(),
            Angle::from_deg(37.0),
        )
        .then(&Transform::scale_about(Point3::new(1.0, 2.0, 3.0), 2.5));
        let p = Point3::new(1234.0, -567.0, 89.0);
        let back = t.inverse().unwrap().apply(t.apply(p));
        assert!(back.is_almost_equal(p));
        assert!(t
            .then(&t.inverse().unwrap())
            .is_almost_equal(&Transform::IDENTITY));
    }

    #[test]
    fn singular_transform_has_no_inverse() {
        assert!(Transform::scale_about(Point3::ORIGIN, 0.0)
            .inverse()
            .is_none());
    }

    #[test]
    fn mirror_reflects_and_flags_reflection() {
        let m = Transform::mirror(Point3::new(0.0, 0.0, 1000.0), UnitVec3::Z);
        assert!(m
            .apply(Point3::new(5.0, 5.0, 0.0))
            .is_almost_equal(Point3::new(5.0, 5.0, 2000.0)));
        assert!(m.has_reflection());
        assert!(m.is_conformal());
    }

    #[test]
    fn non_uniform_scale_is_not_conformal() {
        let t = Transform {
            basis_x: Vec3::X * 2.0,
            ..Transform::IDENTITY
        };
        assert!(!t.is_conformal());
    }
}
