use super::GeomError;
use crate::math::{tol, Point2, Point3, Transform, UnitVec3, Vec3};
use serde::{Deserialize, Serialize};

/// A plane as a full frame: origin + in-plane X and Y axes; normal = X × Y (D-08).
/// Revit: `Plane` (`Origin`, `XVec`, `YVec`, `Normal`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    origin: Point3,
    x_axis: UnitVec3,
    y_axis: UnitVec3,
}

impl Plane {
    /// The world ground plane (Z = 0), X east, Y north.
    pub const XY: Plane = Plane {
        origin: Point3::ORIGIN,
        x_axis: UnitVec3::X,
        y_axis: UnitVec3::Y,
    };

    /// Horizontal plane at elevation `z` (mm).
    pub fn horizontal(z: f64) -> Plane {
        Plane {
            origin: Point3::new(0.0, 0.0, z),
            ..Self::XY
        }
    }

    /// Plane through `origin` with `normal`. The X axis is chosen by a fixed
    /// rule: the projection of world X onto the plane, or world Y when the
    /// normal is parallel to X. Deterministic, so sketches never rotate.
    pub fn from_origin_normal(origin: Point3, normal: UnitVec3) -> Plane {
        let n = normal.as_vec();
        let seed = if n.is_parallel(Vec3::X) {
            Vec3::Y
        } else {
            Vec3::X
        };
        let x = (seed - n * seed.dot(n))
            .normalize()
            .expect("seed axis is not parallel to the normal");
        let y = n.cross(x.as_vec()).normalize().expect("x ⟂ n");
        Plane {
            origin,
            x_axis: x,
            y_axis: y,
        }
    }

    /// Plane from an origin and two in-plane directions; `y_dir` is
    /// orthogonalised against `x_dir`. Fails if the directions are parallel.
    pub fn from_frame(origin: Point3, x_dir: Vec3, y_dir: Vec3) -> Result<Plane, GeomError> {
        let x = x_dir.normalize().ok_or(GeomError::Degenerate)?;
        let y_ortho = y_dir - x.as_vec() * y_dir.dot(x.as_vec());
        if y_ortho.length() <= tol::ANGLE_TOL * y_dir.length().max(1.0) {
            return Err(GeomError::Degenerate);
        }
        let y = y_ortho.normalize().ok_or(GeomError::Degenerate)?;
        Ok(Plane {
            origin,
            x_axis: x,
            y_axis: y,
        })
    }

    /// Plane through three points, X along a→b, normal by the right-hand rule.
    pub fn from_three_points(a: Point3, b: Point3, c: Point3) -> Result<Plane, GeomError> {
        if (b - a).cross(c - a).length() <= tol::VERTEX_TOL * tol::VERTEX_TOL {
            return Err(GeomError::Degenerate);
        }
        Self::from_frame(a, b - a, c - a)
    }

    pub fn origin(&self) -> Point3 {
        self.origin
    }
    pub fn x_axis(&self) -> UnitVec3 {
        self.x_axis
    }
    pub fn y_axis(&self) -> UnitVec3 {
        self.y_axis
    }
    pub fn normal(&self) -> UnitVec3 {
        self.x_axis
            .cross(self.y_axis)
            .expect("axes are orthonormal")
    }

    /// Signed distance (mm), positive on the normal side.
    pub fn signed_distance_to(&self, p: Point3) -> f64 {
        (p - self.origin).dot(self.normal().as_vec())
    }

    /// `true` if `p` lies on the plane within `VERTEX_TOL`.
    pub fn contains(&self, p: Point3) -> bool {
        tol::is_zero_len(self.signed_distance_to(p))
    }

    /// Orthogonal projection of `p` onto the plane.
    pub fn project(&self, p: Point3) -> Point3 {
        p - self.normal() * self.signed_distance_to(p)
    }

    /// 2D sketch coordinates of `p` (projected onto the plane).
    pub fn to_local(&self, p: Point3) -> Point2 {
        let d = p - self.origin;
        Point2::new(d.dot(self.x_axis.as_vec()), d.dot(self.y_axis.as_vec()))
    }

    /// World point from 2D sketch coordinates.
    pub fn to_world(&self, uv: Point2) -> Point3 {
        self.origin + self.x_axis * uv.x + self.y_axis * uv.y
    }

    /// The plane with its normal flipped (Y axis reversed, X kept).
    pub fn reversed(&self) -> Plane {
        Plane {
            y_axis: -self.y_axis,
            ..*self
        }
    }

    /// The plane moved by a conformal transform. Fails for transforms that
    /// collapse the frame.
    pub fn transformed(&self, t: &Transform) -> Result<Plane, GeomError> {
        Self::from_frame(
            t.apply(self.origin),
            t.apply_vec(self.x_axis.as_vec()),
            t.apply_vec(self.y_axis.as_vec()),
        )
    }

    /// The transform mapping local frame coordinates (x, y, z = along normal)
    /// to world coordinates.
    pub fn to_world_transform(&self) -> Transform {
        Transform {
            basis_x: self.x_axis.as_vec(),
            basis_y: self.y_axis.as_vec(),
            basis_z: self.normal().as_vec(),
            origin: self.origin.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xy_plane_normal_is_up() {
        assert!(Plane::XY.normal().as_vec().is_almost_equal(Vec3::Z));
    }

    #[test]
    fn local_world_roundtrip() {
        let pl = Plane::from_origin_normal(
            Point3::new(100.0, 200.0, 300.0),
            Vec3::new(1.0, 1.0, 1.0).normalize().unwrap(),
        );
        let p = pl.to_world(Point2::new(1234.0, -567.0));
        assert!(pl.contains(p));
        assert!(pl.to_local(p).is_almost_equal(Point2::new(1234.0, -567.0)));
    }

    #[test]
    fn from_origin_normal_is_deterministic_for_vertical_planes() {
        let pl = Plane::from_origin_normal(Point3::ORIGIN, UnitVec3::X);
        assert!(pl.x_axis().as_vec().is_almost_equal(Vec3::Y));
        assert!(pl.normal().as_vec().is_almost_equal(Vec3::X));
    }

    #[test]
    fn three_collinear_points_fail() {
        let r = Plane::from_three_points(
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0),
        );
        assert_eq!(r, Err(GeomError::Degenerate));
    }

    #[test]
    fn contains_tolerance_edge() {
        let pl = Plane::XY;
        assert!(pl.contains(Point3::new(5.0, 5.0, tol::VERTEX_TOL * 0.5)));
        assert!(!pl.contains(Point3::new(5.0, 5.0, tol::VERTEX_TOL * 2.0)));
    }

    #[test]
    fn reversed_flips_normal() {
        assert!(Plane::XY
            .reversed()
            .normal()
            .as_vec()
            .is_almost_equal(-Vec3::Z));
    }
}
