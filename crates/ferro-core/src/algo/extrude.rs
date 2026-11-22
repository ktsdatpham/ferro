use super::AlgoError;
use crate::geom::{Curve, CurveLoop, Cylinder, Profile, Surface};
use crate::math::{tol, Point3, UnitVec3};
use crate::topo::{Body, FaceId, FaceSpec, VertexId};

/// Face ids produced by [`extrude_profile`].
#[derive(Debug, Clone, PartialEq)]
pub struct ExtrudeFaces {
    /// Face on the profile plane (facing away from the extrusion).
    pub base: FaceId,
    /// Face at the far end.
    pub cap: FaceId,
    /// Side faces, in loop order (outer loop first, then holes).
    pub sides: Vec<FaceId>,
}

/// One polygon vertex of a tessellated loop and the curve its outgoing
/// segment came from.
struct Seg {
    point: Point3,
    curve: usize,
}

fn segments(l: &CurveLoop) -> Vec<Seg> {
    let mut out = Vec::new();
    for (ci, c) in l.curves().iter().enumerate() {
        let pts = c.tessellate();
        for p in &pts[..pts.len() - 1] {
            out.push(Seg {
                point: *p,
                curve: ci,
            });
        }
    }
    out
}

/// Extrude a profile by `distance` (mm) along its plane normal (negative =
/// opposite direction) into a closed, outward-facing solid.
///
/// Arcs become faceted side faces that carry the exact `Cylinder` surface and
/// share a smooth group (D-39). Fails with `ZeroDistance` below
/// `SHORT_CURVE_TOL`.
pub fn extrude_profile(
    profile: &Profile,
    distance: f64,
) -> Result<(Body, ExtrudeFaces), AlgoError> {
    if distance.abs() < tol::SHORT_CURVE_TOL {
        return Err(AlgoError::ZeroDistance);
    }
    let n = profile.plane().normal();
    let dir: UnitVec3 = if distance > 0.0 { n } else { -n };
    let offset = dir * distance.abs();

    // Loops oriented counter-clockwise about `dir` (outer) / clockwise (holes).
    let flip = distance < 0.0;
    let loops: Vec<CurveLoop> = std::iter::once(profile.outer())
        .chain(profile.holes())
        .map(|l| if flip { l.reversed() } else { l.clone() })
        .collect();

    let mut body = Body::default();
    let mut specs = Vec::new();
    let mut base_loops: Vec<Vec<VertexId>> = Vec::new();
    let mut cap_loops: Vec<Vec<VertexId>> = Vec::new();
    let mut side_specs = Vec::new();

    for l in &loops {
        let segs = segments(l);
        let bottom: Vec<VertexId> = segs.iter().map(|s| body.add_vertex(s.point)).collect();
        let top: Vec<VertexId> = segs
            .iter()
            .map(|s| body.add_vertex(s.point + offset))
            .collect();
        let k = segs.len();
        let mut groups = std::collections::HashMap::new();
        for i in 0..k {
            let j = (i + 1) % k;
            let (surface, smooth_group) = match l.curves()[segs[i].curve] {
                Curve::Arc(a) => {
                    let g = *groups
                        .entry(segs[i].curve)
                        .or_insert_with(|| body.alloc_smooth_group());
                    (
                        Some(Surface::Cylinder(Cylinder {
                            axis_origin: a.center(),
                            axis: dir,
                            radius: a.radius(),
                        })),
                        Some(g),
                    )
                }
                Curve::Line(_) => (None, None),
            };
            side_specs.push(FaceSpec {
                id: None,
                outer: vec![bottom[i], bottom[j], top[j], top[i]],
                holes: vec![],
                surface,
                smooth_group,
            });
        }
        let mut rev = bottom.clone();
        rev.reverse();
        base_loops.push(rev);
        cap_loops.push(top);
    }

    let mut base = FaceSpec::new(base_loops[0].clone());
    base.holes = base_loops[1..].to_vec();
    let mut cap = FaceSpec::new(cap_loops[0].clone());
    cap.holes = cap_loops[1..].to_vec();
    specs.push(base);
    specs.push(cap);
    specs.extend(side_specs);

    let verts = body.vertex_map();
    let ids = body.rebuild(verts, specs)?;
    debug_assert!(body.validate_solid().is_ok(), "{:?}", body.validate_solid());
    Ok((
        body,
        ExtrudeFaces {
            base: ids[0],
            cap: ids[1],
            sides: ids[2..].to_vec(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Arc, Plane};
    use crate::math::Vec3;

    fn rect_profile(w: f64, d: f64) -> Profile {
        Profile::from_outer(
            CurveLoop::from_polygon(&[
                Point3::ORIGIN,
                Point3::new(w, 0.0, 0.0),
                Point3::new(w, d, 0.0),
                Point3::new(0.0, d, 0.0),
            ])
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn extrude_rectangle_up_and_down() {
        for d in [2800.0, -2800.0] {
            let (b, f) = extrude_profile(&rect_profile(4000.0, 3000.0), d).unwrap();
            b.validate_solid().unwrap();
            assert!((b.volume() - 4000.0 * 3000.0 * 2800.0).abs() < 1.0);
            assert_eq!(f.sides.len(), 4);
            let cap_n = b.face_normal(f.cap).unwrap().as_vec();
            assert!(cap_n.is_almost_equal(Vec3::Z * d.signum()));
        }
    }

    #[test]
    fn extrude_with_hole_subtracts_volume() {
        let hole = CurveLoop::from_polygon(&[
            Point3::new(1000.0, 1000.0, 0.0),
            Point3::new(2000.0, 1000.0, 0.0),
            Point3::new(2000.0, 2000.0, 0.0),
            Point3::new(1000.0, 2000.0, 0.0),
        ])
        .unwrap();
        let p = Profile::from_loops(
            Plane::XY,
            rect_profile(3000.0, 3000.0).outer().clone(),
            vec![hole],
        )
        .unwrap();
        let (b, f) = extrude_profile(&p, 100.0).unwrap();
        b.validate_solid().unwrap();
        assert_eq!(f.sides.len(), 8);
        assert!((b.volume() - (9e6 - 1e6) * 100.0).abs() < 1.0);
    }

    #[test]
    fn extrude_circle_makes_smooth_cylinder() {
        let p = Profile::from_outer(
            CurveLoop::from_curves(vec![Arc::circle(Plane::XY, 300.0).unwrap().into()]).unwrap(),
        )
        .unwrap();
        let (b, f) = extrude_profile(&p, 3000.0).unwrap();
        b.validate_solid().unwrap();
        let exact = std::f64::consts::PI * 300.0 * 300.0 * 3000.0;
        assert!((b.volume() - exact).abs() / exact < 0.02);
        let g = b.face(f.sides[0]).unwrap().smooth_group;
        assert!(g.is_some());
        assert!(f
            .sides
            .iter()
            .all(|s| b.face(*s).unwrap().smooth_group == g));
        assert!(matches!(
            b.face(f.sides[0]).unwrap().surface,
            Surface::Cylinder(_)
        ));
    }

    #[test]
    fn tiny_distance_is_rejected() {
        assert_eq!(
            extrude_profile(&rect_profile(1000.0, 1000.0), 0.1).map(|_| ()),
            Err(AlgoError::ZeroDistance)
        );
    }
}
