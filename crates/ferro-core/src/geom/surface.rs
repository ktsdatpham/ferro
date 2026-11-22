use super::Plane;
use crate::math::{Point3, UnitVec3};
use serde::{Deserialize, Serialize};

/// An infinite circular cylinder (curved walls, round columns, barrel vaults).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cylinder {
    pub axis_origin: Point3,
    pub axis: UnitVec3,
    pub radius: f64,
}

impl Cylinder {
    /// Outward unit normal at the point of the surface closest to `p`.
    pub fn normal_at(&self, p: Point3) -> Option<UnitVec3> {
        let d = p - self.axis_origin;
        let radial = d - self.axis * d.dot(self.axis.as_vec());
        radial.normalize()
    }
}

/// The exact surface carried by a B-rep face (D-39). v1: `Plane` and `Cylinder`;
/// NURBS and SubD plug in later without changing topology.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Surface {
    Plane(Plane),
    Cylinder(Cylinder),
}

impl Surface {
    pub fn normal_at(&self, p: Point3) -> Option<UnitVec3> {
        match self {
            Surface::Plane(pl) => Some(pl.normal()),
            Surface::Cylinder(c) => c.normal_at(p),
        }
    }

    pub fn is_planar(&self) -> bool {
        matches!(self, Surface::Plane(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    #[test]
    fn cylinder_normal_is_radial() {
        let c = Cylinder {
            axis_origin: Point3::ORIGIN,
            axis: UnitVec3::Z,
            radius: 1000.0,
        };
        let n = c.normal_at(Point3::new(0.0, 1000.0, 500.0)).unwrap();
        assert!(n.as_vec().is_almost_equal(Vec3::Y));
        assert!(c.normal_at(Point3::new(0.0, 0.0, 5.0)).is_none());
    }
}
