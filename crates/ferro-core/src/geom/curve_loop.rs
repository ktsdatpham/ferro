use super::{polygon, predicates, Curve, GeomError, Line, Plane};
use crate::math::{tol, Point2, Point3};
use serde::{Deserialize, Serialize};

/// Winding of a loop seen from the side its plane normal points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    CounterClockwise,
    Clockwise,
}

/// A closed, connected chain of curves. Revit: `CurveLoop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveLoop {
    curves: Vec<Curve>,
}

impl CurveLoop {
    /// Validates connectivity (each end meets the next start within
    /// `VERTEX_TOL`) and closure.
    pub fn from_curves(curves: Vec<Curve>) -> Result<CurveLoop, GeomError> {
        if curves.is_empty() {
            return Err(GeomError::LoopNotClosed);
        }
        for w in curves.windows(2) {
            if !w[0].end().is_almost_equal(w[1].start()) {
                return Err(GeomError::LoopNotConnected);
            }
        }
        let first = curves[0].start();
        let last = curves[curves.len() - 1].end();
        if !first.is_almost_equal(last) {
            return Err(GeomError::LoopNotClosed);
        }
        Ok(CurveLoop { curves })
    }

    /// Closed polygon through `points` (do not repeat the first point).
    pub fn from_polygon(points: &[Point3]) -> Result<CurveLoop, GeomError> {
        if points.len() < 3 {
            return Err(GeomError::Degenerate);
        }
        let n = points.len();
        let curves = (0..n)
            .map(|i| Line::from_points(points[i], points[(i + 1) % n]).map(Curve::Line))
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_curves(curves)
    }

    pub fn curves(&self) -> &[Curve] {
        &self.curves
    }

    /// Polygon approximation (arcs tessellated), without repeating the first point.
    pub fn polygon(&self) -> Vec<Point3> {
        let mut pts = Vec::new();
        for c in &self.curves {
            let t = c.tessellate();
            pts.extend_from_slice(&t[..t.len() - 1]);
        }
        pts
    }

    /// Best-fit plane (Newell normal through the first point), if all curves
    /// lie on it within `VERTEX_TOL`.
    pub fn plane(&self) -> Result<Plane, GeomError> {
        let pts = self.polygon();
        let n = polygon::polygon_normal(&pts).ok_or(GeomError::Degenerate)?;
        let pl = Plane::from_origin_normal(pts[0], n);
        if pts.iter().all(|p| pl.contains(*p)) {
            Ok(pl)
        } else {
            Err(GeomError::NotPlanar)
        }
    }

    pub fn orientation(&self, plane: &Plane) -> Orientation {
        if self.signed_area(plane) >= 0.0 {
            Orientation::CounterClockwise
        } else {
            Orientation::Clockwise
        }
    }

    /// Signed area (mm²) in the plane's frame; positive = counter-clockwise.
    pub fn signed_area(&self, plane: &Plane) -> f64 {
        predicates::signed_area2(&self.local_polygon(plane)) * 0.5
    }

    pub fn local_polygon(&self, plane: &Plane) -> Vec<Point2> {
        self.polygon().iter().map(|p| plane.to_local(*p)).collect()
    }

    pub fn length(&self) -> f64 {
        self.curves.iter().map(Curve::length).sum()
    }

    pub fn reversed(&self) -> CurveLoop {
        CurveLoop {
            curves: self.curves.iter().rev().map(Curve::reversed).collect(),
        }
    }

    pub fn is_coplanar_with(&self, plane: &Plane) -> bool {
        self.polygon().iter().all(|p| plane.contains(*p))
            && self.curves.iter().all(|c| plane.contains(c.point_at(0.5)))
    }

    pub fn contains_point(&self, plane: &Plane, p: Point3) -> bool {
        tol::is_zero_len(plane.signed_distance_to(p))
            && predicates::point_in_polygon(plane.to_local(p), &self.local_polygon(plane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Vec<Point3> {
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(w, 0.0, 0.0),
            Point3::new(w, h, 0.0),
            Point3::new(0.0, h, 0.0),
        ]
    }

    #[test]
    fn polygon_loop_is_ccw_and_has_area() {
        let l = CurveLoop::from_polygon(&rect(4000.0, 3000.0)).unwrap();
        assert_eq!(l.orientation(&Plane::XY), Orientation::CounterClockwise);
        assert!((l.signed_area(&Plane::XY) - 12_000_000.0).abs() < 1e-3);
        assert_eq!(l.reversed().orientation(&Plane::XY), Orientation::Clockwise);
    }

    #[test]
    fn disconnected_curves_fail() {
        let a = Line::from_points(Point3::ORIGIN, Point3::new(10.0, 0.0, 0.0)).unwrap();
        let b = Line::from_points(Point3::new(10.0, 5.0, 0.0), Point3::ORIGIN).unwrap();
        assert_eq!(
            CurveLoop::from_curves(vec![a.into(), b.into()]),
            Err(GeomError::LoopNotConnected)
        );
    }

    #[test]
    fn non_planar_loop_is_detected() {
        let mut pts = rect(1000.0, 1000.0);
        pts[2].z = 10.0;
        let l = CurveLoop::from_polygon(&pts).unwrap();
        assert_eq!(l.plane(), Err(GeomError::NotPlanar));
    }

    #[test]
    fn duplicate_points_fail_as_short_curve() {
        let mut pts = rect(1000.0, 1000.0);
        pts.insert(1, Point3::new(0.1, 0.0, 0.0));
        assert_eq!(CurveLoop::from_polygon(&pts), Err(GeomError::ShortCurve));
    }
}
