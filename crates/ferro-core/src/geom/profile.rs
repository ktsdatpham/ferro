use super::{predicates, CurveLoop, GeomError, Orientation, Plane};
use serde::{Deserialize, Serialize};

/// One outer loop (counter-clockwise about the plane normal) and zero or more
/// holes (clockwise), all coplanar (D-10). Revit: the sketch of a floor / roof /
/// extrusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    plane: Plane,
    outer: CurveLoop,
    holes: Vec<CurveLoop>,
}

impl Profile {
    /// Normalises orientation and validates coplanarity and hole containment.
    /// The user may draw loops in any direction.
    pub fn from_loops(
        plane: Plane,
        outer: CurveLoop,
        holes: Vec<CurveLoop>,
    ) -> Result<Profile, GeomError> {
        if !outer.is_coplanar_with(&plane) || holes.iter().any(|h| !h.is_coplanar_with(&plane)) {
            return Err(GeomError::NotPlanar);
        }
        let outer = match outer.orientation(&plane) {
            Orientation::CounterClockwise => outer,
            Orientation::Clockwise => outer.reversed(),
        };
        let outer_poly = outer.local_polygon(&plane);
        let mut norm_holes = Vec::with_capacity(holes.len());
        for h in holes {
            let poly = h.local_polygon(&plane);
            // Every hole vertex must be strictly inside the outer loop.
            if poly
                .iter()
                .any(|p| !predicates::point_in_polygon(*p, &outer_poly))
            {
                return Err(GeomError::InvalidProfile);
            }
            norm_holes.push(match h.orientation(&plane) {
                Orientation::Clockwise => h,
                Orientation::CounterClockwise => h.reversed(),
            });
        }
        Ok(Profile {
            plane,
            outer,
            holes: norm_holes,
        })
    }

    /// Profile from a single outer loop on its own best-fit plane, with the
    /// plane normal chosen so the loop is counter-clockwise.
    pub fn from_outer(outer: CurveLoop) -> Result<Profile, GeomError> {
        let plane = outer.plane()?;
        Self::from_loops(plane, outer, Vec::new())
    }

    pub fn plane(&self) -> &Plane {
        &self.plane
    }
    pub fn outer(&self) -> &CurveLoop {
        &self.outer
    }
    pub fn holes(&self) -> &[CurveLoop] {
        &self.holes
    }

    /// Net area (mm²): outer minus holes.
    pub fn area(&self) -> f64 {
        self.outer.signed_area(&self.plane)
            + self
                .holes
                .iter()
                .map(|h| h.signed_area(&self.plane))
                .sum::<f64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Point3;

    fn sq(x0: f64, y0: f64, s: f64) -> CurveLoop {
        CurveLoop::from_polygon(&[
            Point3::new(x0, y0, 0.0),
            Point3::new(x0 + s, y0, 0.0),
            Point3::new(x0 + s, y0 + s, 0.0),
            Point3::new(x0, y0 + s, 0.0),
        ])
        .unwrap()
    }

    #[test]
    fn orientation_is_normalised_and_area_subtracts_holes() {
        let p = Profile::from_loops(
            Plane::XY,
            sq(0.0, 0.0, 10.0).reversed(),
            vec![sq(2.0, 2.0, 2.0)],
        )
        .unwrap();
        assert_eq!(
            p.outer().orientation(p.plane()),
            Orientation::CounterClockwise
        );
        assert_eq!(p.holes()[0].orientation(p.plane()), Orientation::Clockwise);
        assert!((p.area() - 96.0).abs() < 1e-9);
    }

    #[test]
    fn hole_outside_outer_fails() {
        assert_eq!(
            Profile::from_loops(Plane::XY, sq(0.0, 0.0, 10.0), vec![sq(20.0, 0.0, 2.0)]),
            Err(GeomError::InvalidProfile)
        );
    }

    #[test]
    fn off_plane_loop_fails() {
        assert_eq!(
            Profile::from_loops(Plane::horizontal(100.0), sq(0.0, 0.0, 10.0), vec![]),
            Err(GeomError::NotPlanar)
        );
    }
}
