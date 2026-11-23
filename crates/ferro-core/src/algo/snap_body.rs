//! Snap one operand to another at Ferro's tolerance before a kernel call
//! (D-06): kernels keep a 0.001 mm gap or skin that Ferro treats as nothing,
//! so a face of the tool within `VERTEX_TOL` of a plane of the target is
//! moved onto that plane first.

use crate::geom::Surface;
use crate::math::{tol, Point3, Vec3};
use crate::topo::{Body, TopoError, VertexId};
use std::collections::BTreeMap;

/// A plane constraint `n · x = d` (unit `n`, mm).
#[derive(Debug, Clone, Copy)]
struct Constraint {
    n: Vec3,
    d: f64,
}

/// Solve the 3×3 system `m x = r` (Cramer's rule); `None` when singular.
fn solve3(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-9 {
        return None;
    }
    let mut out = [0.0; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let mut mc = m;
        for row in 0..3 {
            mc[row][c] = r[row];
        }
        *o = det(mc) / d;
    }
    Some(out)
}

/// The point nearest `p` on every plane of `cs` (independent planes only,
/// at most three): `x = p − Σ λᵢ nᵢ` with `(N Nᵀ) λ = N p − d`.
fn project(p: Point3, cs: &[Constraint]) -> Point3 {
    let mut chosen: Vec<Constraint> = Vec::new();
    for c in cs {
        if chosen.len() == 3 {
            break;
        }
        let independent = match chosen.as_slice() {
            [] => true,
            [a] => a.n.cross(c.n).length() > 1e-3,
            [a, b] => a.n.cross(b.n).dot(c.n).abs() > 1e-3,
            _ => false,
        };
        if independent {
            chosen.push(*c);
        }
    }
    let k = chosen.len();
    let mut m = [[0.0; 3]; 3];
    let mut r = [0.0; 3];
    for i in 0..3 {
        if i < k {
            for j in 0..k {
                m[i][j] = chosen[i].n.dot(chosen[j].n);
            }
            r[i] = chosen[i].n.dot(p.to_vec()) - chosen[i].d;
        } else {
            m[i][i] = 1.0;
        }
    }
    let Some(l) = solve3(m, r) else { return p };
    let mut q = p;
    for i in 0..k {
        q -= chosen[i].n * l[i];
    }
    q
}

/// `b` with its faces that lie within `VERTEX_TOL` (0.01 mm, D-06) of a
/// plane of `a` moved exactly onto that plane (vertices on several such
/// planes go to their common point), then its vertices within `VERTEX_TOL`
/// of a vertex of `a` moved onto it. Ids do not change; `a` is not touched.
/// A face counts as lying on a plane when every one of its vertices does
/// and its normal is parallel to the plane's (either side: a cutter flush
/// with a face faces the other way). Curved faces of `a` are ignored.
///
/// Deterministic; O(faces(a) × faces(b)).
///
/// # Errors
/// `TopoError::NotFound` only if `b` is corrupt (a face naming a missing
/// vertex).
pub fn snap_body_to_body(b: &Body, a: &Body) -> Result<Body, TopoError> {
    let planes: Vec<_> = a
        .face_ids()
        .filter_map(|f| match a.face(f).ok()?.surface {
            Surface::Plane(p) => Some(p),
            Surface::Cylinder(_) => None,
        })
        .collect();
    let mut constraints: BTreeMap<VertexId, Vec<Constraint>> = BTreeMap::new();
    for fb in b.face_ids() {
        let face = b.face(fb)?;
        let Ok(nb) = b.face_normal(fb) else { continue };
        let verts: Vec<VertexId> = std::iter::once(&face.outer)
            .chain(&face.holes)
            .flat_map(|l| b.loop_vertices(*l))
            .collect();
        for pa in &planes {
            if nb.dot(pa.normal()).abs() < 1.0 - 1e-6 {
                continue;
            }
            let on = verts.iter().all(|v| {
                b.point(*v)
                    .is_some_and(|p| pa.signed_distance_to(p).abs() <= tol::VERTEX_TOL)
            });
            if !on {
                continue;
            }
            let c = Constraint {
                n: pa.normal().as_vec(),
                d: pa.normal().as_vec().dot(pa.origin().to_vec()),
            };
            for v in &verts {
                let list = constraints.entry(*v).or_default();
                let same = list
                    .iter()
                    .any(|o| o.n.cross(c.n).length() < 1e-9 && (o.d - c.d * o.n.dot(c.n)).abs() < 1e-9);
                if !same {
                    list.push(c);
                }
            }
        }
    }
    let mut moves: Vec<(VertexId, Point3)> = Vec::new();
    for (v, p) in b.vertices() {
        let mut q = match constraints.get(&v) {
            Some(cs) => project(p, cs),
            None => p,
        };
        if let Some((_, pa)) = a
            .vertices()
            .filter(|(_, pa)| pa.distance_to(q) <= tol::VERTEX_TOL)
            .min_by(|x, y| x.1.distance_to(q).total_cmp(&y.1.distance_to(q)))
        {
            q = pa;
        }
        if q.distance_to(p) > 0.0 {
            moves.push((v, q));
        }
    }
    let mut out = b.clone();
    if !moves.is_empty() {
        out.move_vertices(&moves)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Angle, Transform, UnitVec3};
    use crate::topo::tests_support::box_body;

    /// A 1 m cube and a 200 mm cutter whose top is `dz` above the cube's top
    /// (negative: below), both turned `deg` about the Z axis through (500, 500).
    fn pair(dz: f64, deg: f64) -> (Body, Body) {
        let (mut a, _) = box_body(1000.0, 1000.0, 1000.0);
        let (mut b, _) = box_body(200.0, 200.0, 600.0);
        b.transform(&Transform::translation(Vec3::new(400.0, 400.0, 400.0 + dz)));
        let r = Transform::rotation(
            Point3::new(500.0, 500.0, 0.0),
            UnitVec3::Z,
            Angle::from_deg(deg),
        );
        a.transform(&r);
        b.transform(&r);
        (a, b)
    }

    fn top_z(b: &Body) -> f64 {
        b.bounding_box().unwrap().max.z
    }

    #[test]
    fn snap_moves_a_face_within_tolerance_onto_the_plane() {
        for deg in [0.0, 30.0, 45.0] {
            let (a, b) = pair(0.004, deg);
            let s = snap_body_to_body(&b, &a).unwrap();
            assert!((top_z(&s) - 1000.0).abs() < 1e-9, "{deg}: {}", top_z(&s));
            s.validate_solid().unwrap();
            // Ids are kept.
            assert_eq!(s.face_ids().collect::<Vec<_>>(), b.face_ids().collect::<Vec<_>>());
        }
    }

    #[test]
    fn snap_leaves_faces_beyond_tolerance_alone() {
        let (a, b) = pair(0.02, 30.0);
        let s = snap_body_to_body(&b, &a).unwrap();
        assert_eq!(s, b);
    }

    #[test]
    fn snap_boundary_is_inclusive() {
        // 0.009 mm: inside the tolerance (an exact 0.01 rounds either way).
        let (a, b) = pair(-0.009, 0.0);
        let s = snap_body_to_body(&b, &a).unwrap();
        assert!((top_z(&s) - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn snap_of_a_body_far_away_changes_nothing() {
        let (a, _) = box_body(1000.0, 1000.0, 1000.0);
        let (mut b, _) = box_body(10.0, 10.0, 10.0);
        b.transform(&Transform::translation(Vec3::new(5e6, 5e6, 0.0)));
        assert_eq!(snap_body_to_body(&b, &a).unwrap(), b);
    }

    #[test]
    fn snap_projects_a_corner_onto_three_planes() {
        let (a, _) = box_body(1000.0, 1000.0, 1000.0);
        let (mut b, _) = box_body(500.0, 500.0, 500.0);
        b.transform(&Transform::translation(Vec3::new(500.006, 500.003, 499.995)));
        let s = snap_body_to_body(&b, &a).unwrap();
        let bb = s.bounding_box().unwrap();
        assert!((bb.min.x - 500.006).abs() < 1e-9, "inner faces are not on a plane of a");
        assert!((bb.max.x - 1000.0).abs() < 1e-9);
        assert!((bb.max.y - 1000.0).abs() < 1e-9);
        assert!((bb.max.z - 1000.0).abs() < 1e-9);
    }
}
