use super::{Arc, GeomError, Line};
use crate::math::{Point3, Transform};
use serde::{Deserialize, Serialize};

/// Angular step used when curves are approximated by polylines (radians).
/// 7.5° keeps facets visually smooth for architectural radii.
pub const TESSELLATION_ANGLE: f64 = std::f64::consts::PI / 24.0;

/// Any bounded curve. Revit: `Curve`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Curve {
    Line(Line),
    Arc(Arc),
}

impl Curve {
    pub fn start(&self) -> Point3 {
        match self {
            Curve::Line(l) => l.start(),
            Curve::Arc(a) => a.start(),
        }
    }
    pub fn end(&self) -> Point3 {
        match self {
            Curve::Line(l) => l.end(),
            Curve::Arc(a) => a.end(),
        }
    }
    pub fn length(&self) -> f64 {
        match self {
            Curve::Line(l) => l.length(),
            Curve::Arc(a) => a.length(),
        }
    }
    pub fn point_at(&self, t: f64) -> Point3 {
        match self {
            Curve::Line(l) => l.point_at(t),
            Curve::Arc(a) => a.point_at(t),
        }
    }
    pub fn reversed(&self) -> Curve {
        match self {
            Curve::Line(l) => Curve::Line(l.reversed()),
            Curve::Arc(a) => Curve::Arc(a.reversed()),
        }
    }
    pub fn is_closed(&self) -> bool {
        matches!(self, Curve::Arc(a) if a.is_closed())
    }

    /// Polyline approximation including both ends.
    pub fn tessellate(&self) -> Vec<Point3> {
        match self {
            Curve::Line(l) => vec![l.start(), l.end()],
            Curve::Arc(a) => a.tessellate(TESSELLATION_ANGLE),
        }
    }

    /// Lines transform exactly; arcs are only defined under conformal transforms.
    pub fn transformed(&self, t: &Transform) -> Result<Curve, GeomError> {
        match self {
            Curve::Line(l) => Ok(Curve::Line(l.transformed(t)?)),
            Curve::Arc(a) => {
                if !t.is_conformal() {
                    return Err(GeomError::Degenerate);
                }
                let scale = t.basis_x.length();
                let plane = a.plane().transformed(t)?;
                let mut arc = Arc::from_center(
                    plane,
                    a.radius() * scale,
                    crate::math::Angle::ZERO,
                    a.sweep(),
                )?;
                // Re-anchor the start angle from the transformed start point.
                let s = plane.to_local(t.apply(a.start()));
                arc = Arc::from_center(
                    plane,
                    arc.radius(),
                    crate::math::Angle::from_rad(s.y.atan2(s.x)),
                    a.sweep(),
                )?;
                Ok(Curve::Arc(arc))
            }
        }
    }
}

impl From<Line> for Curve {
    fn from(l: Line) -> Curve {
        Curve::Line(l)
    }
}
impl From<Arc> for Curve {
    fn from(a: Arc) -> Curve {
        Curve::Arc(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Plane;
    use crate::math::{Angle, UnitVec3, Vec3};

    #[test]
    fn arc_survives_rotation() {
        let a = Arc::from_center(Plane::XY, 1000.0, Angle::ZERO, Angle::from_deg(90.0)).unwrap();
        let t = Transform::rotation(Point3::ORIGIN, UnitVec3::Z, Angle::from_deg(90.0))
            .then(&Transform::translation(Vec3::new(0.0, 0.0, 3000.0)));
        let c = Curve::Arc(a).transformed(&t).unwrap();
        assert!(c.start().is_almost_equal(t.apply(a.start())));
        assert!(c.end().is_almost_equal(t.apply(a.end())));
    }

    #[test]
    fn tessellated_arc_ends_match() {
        let a = Curve::Arc(Arc::circle(Plane::XY, 500.0).unwrap());
        let pts = a.tessellate();
        assert!(pts.len() > 8);
        assert!(pts[0].is_almost_equal(*pts.last().unwrap()));
    }
}
