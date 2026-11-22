use super::{GeomError, Plane};
use crate::math::{tol, Angle, Point2, Point3, UnitVec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// A circular arc on a plane: centre = plane origin, starting at `start_angle`
/// (from the plane X axis, counter-clockwise about the plane normal) and
/// sweeping `sweep` radians (> 0). A sweep of 2π is a full circle. Revit: `Arc`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Arc {
    plane: Plane,
    radius: f64,
    start_angle: f64,
    sweep: f64,
}

impl Arc {
    pub fn from_center(
        plane: Plane,
        radius: f64,
        start_angle: Angle,
        sweep: Angle,
    ) -> Result<Arc, GeomError> {
        if radius <= 0.0 || !radius.is_finite() {
            return Err(GeomError::InvalidRadius);
        }
        let sweep = sweep.rad();
        if sweep <= 0.0 || sweep > TAU + tol::ANGLE_TOL {
            return Err(GeomError::Degenerate);
        }
        if radius * sweep < tol::SHORT_CURVE_TOL {
            return Err(GeomError::ShortCurve);
        }
        Ok(Arc {
            plane,
            radius,
            start_angle: start_angle.rad(),
            sweep: sweep.min(TAU),
        })
    }

    /// A full circle.
    pub fn circle(plane: Plane, radius: f64) -> Result<Arc, GeomError> {
        Self::from_center(plane, radius, Angle::ZERO, Angle::from_rad(TAU))
    }

    /// Arc from `a` through `m` to `b`.
    pub fn from_three_points(a: Point3, m: Point3, b: Point3) -> Result<Arc, GeomError> {
        let pl = Plane::from_three_points(a, m, b)?;
        let (pa, pm, pb) = (pl.to_local(a), pl.to_local(m), pl.to_local(b));
        let center = circumcenter(pa, pm, pb).ok_or(GeomError::Degenerate)?;
        let radius = center.distance_to(pa);
        let ang = |p: Point2| (p.y - center.y).atan2(p.x - center.x);
        let (aa, am, ab) = (ang(pa), ang(pm), ang(pb));
        let ccw = |from: f64, to: f64| (to - from).rem_euclid(TAU);
        // In the plane frame a→m→b may run clockwise; then build on the
        // reversed plane so the arc is always counter-clockwise.
        let centre3 = pl.to_world(center);
        if ccw(aa, am) <= ccw(aa, ab) {
            let plane = Plane::from_frame(centre3, pl.x_axis().as_vec(), pl.y_axis().as_vec())?;
            Self::from_center(
                plane,
                radius,
                Angle::from_rad(aa),
                Angle::from_rad(ccw(aa, ab)),
            )
        } else {
            let plane = Plane::from_frame(centre3, pl.x_axis().as_vec(), -pl.y_axis().as_vec())?;
            let ang_r = |p: Point3| {
                let q = plane.to_local(p);
                q.y.atan2(q.x)
            };
            let (sa, sb) = (ang_r(a), ang_r(b));
            Self::from_center(
                plane,
                radius,
                Angle::from_rad(sa),
                Angle::from_rad(ccw(sa, sb)),
            )
        }
    }

    pub fn plane(&self) -> &Plane {
        &self.plane
    }
    pub fn center(&self) -> Point3 {
        self.plane.origin()
    }
    pub fn radius(&self) -> f64 {
        self.radius
    }
    pub fn normal(&self) -> UnitVec3 {
        self.plane.normal()
    }
    pub fn sweep(&self) -> Angle {
        Angle::from_rad(self.sweep)
    }
    pub fn length(&self) -> f64 {
        self.radius * self.sweep
    }
    pub fn is_closed(&self) -> bool {
        tol::eq_angle(self.sweep, TAU)
    }

    /// Point at normalised parameter `t` ∈ [0, 1].
    pub fn point_at(&self, t: f64) -> Point3 {
        let a = self.start_angle + self.sweep * t;
        self.plane
            .to_world(Point2::new(self.radius * a.cos(), self.radius * a.sin()))
    }
    pub fn start(&self) -> Point3 {
        self.point_at(0.0)
    }
    pub fn end(&self) -> Point3 {
        self.point_at(1.0)
    }

    /// Same arc traversed the other way.
    pub fn reversed(&self) -> Arc {
        Arc {
            plane: self.plane.reversed(),
            radius: self.radius,
            // Mirroring Y maps angle θ to −θ.
            start_angle: -(self.start_angle + self.sweep),
            sweep: self.sweep,
        }
    }

    /// Polyline approximation with segments subtending at most `max_angle`
    /// radians. Includes both end points.
    pub fn tessellate(&self, max_angle: f64) -> Vec<Point3> {
        let n = ((self.sweep / max_angle.max(1e-3)).ceil() as usize).max(1);
        (0..=n)
            .map(|i| self.point_at(i as f64 / n as f64))
            .collect()
    }
}

fn circumcenter(a: Point2, b: Point2, c: Point2) -> Option<Point2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (
        a.x * a.x + a.y * a.y,
        b.x * b.x + b.y * b.y,
        c.x * c.x + c.y * c.y,
    );
    Some(Point2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_is_closed_and_has_correct_length() {
        let c = Arc::circle(Plane::XY, 1000.0).unwrap();
        assert!(c.is_closed());
        assert!((c.length() - TAU * 1000.0).abs() < 1e-6);
        assert!(c.start().is_almost_equal(c.end()));
    }

    #[test]
    fn three_point_arc_passes_through_points_both_ways() {
        for (a, m, b) in [
            (
                Point3::new(1000.0, 0.0, 0.0),
                Point3::new(0.0, 1000.0, 0.0),
                Point3::new(-1000.0, 0.0, 0.0),
            ),
            (
                Point3::new(-1000.0, 0.0, 0.0),
                Point3::new(0.0, 1000.0, 0.0),
                Point3::new(1000.0, 0.0, 0.0),
            ),
        ] {
            let arc = Arc::from_three_points(a, m, b).unwrap();
            assert!(arc.start().is_almost_equal(a));
            assert!(arc.end().is_almost_equal(b));
            assert!(arc.point_at(0.5).is_almost_equal(m));
            assert!((arc.radius() - 1000.0).abs() < 1e-6);
        }
    }

    #[test]
    fn reversed_swaps_ends() {
        let arc = Arc::from_center(
            Plane::XY,
            500.0,
            Angle::from_deg(10.0),
            Angle::from_deg(120.0),
        )
        .unwrap();
        let r = arc.reversed();
        assert!(r.start().is_almost_equal(arc.end()));
        assert!(r.end().is_almost_equal(arc.start()));
        assert!(r.point_at(0.5).is_almost_equal(arc.point_at(0.5)));
    }

    #[test]
    fn invalid_arcs_fail() {
        assert_eq!(Arc::circle(Plane::XY, 0.0), Err(GeomError::InvalidRadius));
        assert_eq!(
            Arc::from_center(Plane::XY, 1.0, Angle::ZERO, Angle::from_rad(0.1)),
            Err(GeomError::ShortCurve)
        );
        assert!(Arc::from_three_points(
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(2.0, 0.0, 0.0)
        )
        .is_err());
    }
}
