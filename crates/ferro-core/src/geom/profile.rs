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
    /// Normalises orientation and validates the loops with exact predicates
    /// (D-53). The user may draw loops in any direction.
    ///
    /// Fails with `NotPlanar` (a loop off `plane`), `Degenerate` (a loop with
    /// all points on one line), `SelfIntersecting` (a loop crosses or touches
    /// itself, e.g. a bow-tie) or `InvalidProfile` (a hole outside the outer
    /// loop or touching it, or two holes touching / overlapping).
    pub fn from_loops(
        plane: Plane,
        outer: CurveLoop,
        holes: Vec<CurveLoop>,
    ) -> Result<Profile, GeomError> {
        if !outer.is_coplanar_with(&plane) || holes.iter().any(|h| !h.is_coplanar_with(&plane)) {
            return Err(GeomError::NotPlanar);
        }
        let check = |poly: &[crate::math::Point2]| {
            if predicates::points_are_collinear(poly) {
                Err(GeomError::Degenerate)
            } else if !predicates::polygon_is_simple(poly) {
                Err(GeomError::SelfIntersecting)
            } else {
                Ok(())
            }
        };
        let outer = match outer.orientation(&plane) {
            Orientation::CounterClockwise => outer,
            Orientation::Clockwise => outer.reversed(),
        };
        let outer_poly = outer.local_polygon(&plane);
        check(&outer_poly)?;
        let mut hole_polys: Vec<Vec<crate::math::Point2>> = Vec::with_capacity(holes.len());
        let mut norm_holes = Vec::with_capacity(holes.len());
        for h in holes {
            let poly = h.local_polygon(&plane);
            check(&poly)?;
            // Every hole vertex inside the outer loop, no edge touching it,
            // and no contact with another hole.
            if poly
                .iter()
                .any(|p| !predicates::point_in_polygon(*p, &outer_poly))
                || predicates::polygons_touch_boundary(&poly, &outer_poly)
                || hole_polys
                    .iter()
                    .any(|other| predicates::polygons_touch(&poly, other))
            {
                return Err(GeomError::InvalidProfile);
            }
            hole_polys.push(poly);
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

    fn poly(pts: &[(f64, f64)]) -> CurveLoop {
        let p: Vec<Point3> = pts.iter().map(|(x, y)| Point3::new(*x, *y, 0.0)).collect();
        CurveLoop::from_polygon(&p).unwrap()
    }

    #[test]
    fn bow_tie_outer_or_hole_is_self_intersecting() {
        let bow = poly(&[(0.0, 0.0), (3000.0, 1000.0), (3000.0, 0.0), (0.0, 2000.0)]);
        assert_eq!(
            Profile::from_loops(Plane::XY, bow, vec![]),
            Err(GeomError::SelfIntersecting)
        );
        let small_bow = poly(&[(1.0, 1.0), (4.0, 2.0), (4.0, 1.0), (1.0, 3.0)]);
        assert_eq!(
            Profile::from_loops(Plane::XY, sq(0.0, 0.0, 10.0), vec![small_bow]),
            Err(GeomError::SelfIntersecting)
        );
    }

    #[test]
    fn collinear_outline_is_degenerate() {
        let line = poly(&[(0.0, 0.0), (1000.0, 0.0), (2000.0, 0.0)]);
        assert_eq!(
            Profile::from_loops(Plane::XY, line, vec![]),
            Err(GeomError::Degenerate)
        );
    }

    #[test]
    fn overlapping_or_touching_holes_fail() {
        let outer = sq(0.0, 0.0, 10.0);
        // Two holes overlapping; one inside the other; sharing an edge.
        for (a, b) in [
            (sq(2.0, 2.0, 3.0), sq(4.0, 4.0, 3.0)),
            (sq(2.0, 2.0, 6.0), sq(3.0, 3.0, 1.0)),
            (sq(2.0, 2.0, 2.0), sq(4.0, 2.0, 2.0)),
        ] {
            assert_eq!(
                Profile::from_loops(Plane::XY, outer.clone(), vec![a, b]),
                Err(GeomError::InvalidProfile)
            );
        }
        // Apart: fine.
        assert!(Profile::from_loops(
            Plane::XY,
            outer.clone(),
            vec![sq(1.0, 1.0, 2.0), sq(5.0, 5.0, 2.0)]
        )
        .is_ok());
    }

    #[test]
    fn hole_touching_or_crossing_the_outer_loop_fails() {
        // Tolerance edge: a hole edge exactly on the outer edge, or a hair inside.
        assert_eq!(
            Profile::from_loops(Plane::XY, sq(0.0, 0.0, 10.0), vec![sq(0.0, 2.0, 2.0)]),
            Err(GeomError::InvalidProfile)
        );
        assert!(
            Profile::from_loops(Plane::XY, sq(0.0, 0.0, 10.0), vec![sq(1e-9, 2.0, 2.0)]).is_ok()
        );
        // Every hole vertex inside a concave outer, but an edge crosses out.
        let u = poly(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (6.0, 10.0),
            (6.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]);
        let across = poly(&[(2.0, 6.0), (8.0, 6.0), (8.0, 7.0), (2.0, 7.0)]);
        assert_eq!(
            Profile::from_loops(Plane::XY, u, vec![across]),
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
