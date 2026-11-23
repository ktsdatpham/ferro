//! Section cut of a mesh by a plane.

use ferro_core::algo::Mesh;
use ferro_core::math::tol::VERTEX_TOL;
use ferro_core::{Point3, UnitVec3};

/// Segments (mm) where the plane through `p` with normal `n` crosses the
/// mesh's triangles. Vertices within `VERTEX_TOL` of the plane count as on
/// it; a triangle lying in the plane, or touching it at one point, gives no
/// segment. An edge lying in the plane is reported once per triangle using
/// it (harmless when drawn).
pub fn section_segments(m: &Mesh, p: Point3, n: UnitVec3) -> Vec<(Point3, Point3)> {
    let n = n.as_vec();
    // Signed distance; `None` means on the plane (within tolerance).
    let side = |q: Point3| {
        let d = n.dot(q - p);
        if d.abs() <= VERTEX_TOL {
            None
        } else {
            Some(d)
        }
    };
    let mut out = Vec::new();
    for t in &m.indices {
        let Some(v) = t
            .iter()
            .map(|i| m.positions.get(*i as usize).copied())
            .collect::<Option<Vec<Point3>>>()
        else {
            continue;
        };
        let d: Vec<Option<f64>> = v.iter().map(|q| side(*q)).collect();
        let mut pts: Vec<Point3> = Vec::with_capacity(2);
        for i in 0..3 {
            let (a, b) = (v[i], v[(i + 1) % 3]);
            match (d[i], d[(i + 1) % 3]) {
                (None, _) => pts.push(a),
                (Some(da), Some(db)) if da * db < 0.0 => {
                    pts.push(a + (b - a) * (da / (da - db)));
                }
                _ => {}
            }
        }
        pts.dedup_by(|x, y| x.is_almost_equal(*y));
        if let [a, b] = pts[..] {
            out.push((a, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::Vec3;

    /// One triangle with corners (0,0,0), (1000,0,0), (0,1000,0).
    fn tri() -> Mesh {
        Mesh {
            positions: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1000.0, 0.0, 0.0),
                Point3::new(0.0, 1000.0, 0.0),
            ],
            normals: vec![Vec3::Z; 3],
            indices: vec![[0, 1, 2]],
            triangle_faces: Vec::new(),
        }
    }

    #[test]
    fn plane_across_a_triangle_gives_one_segment() {
        let s = section_segments(&tri(), Point3::new(500.0, 0.0, 0.0), UnitVec3::X);
        let [(a, b)] = s[..] else { panic!("{s:?}") };
        let ends = [a, b];
        assert!(ends
            .iter()
            .any(|p| p.is_almost_equal(Point3::new(500.0, 0.0, 0.0))));
        assert!(ends
            .iter()
            .any(|p| p.is_almost_equal(Point3::new(500.0, 500.0, 0.0))));
    }

    #[test]
    fn plane_missing_or_containing_the_triangle_gives_nothing() {
        assert!(section_segments(&tri(), Point3::new(2000.0, 0.0, 0.0), UnitVec3::X).is_empty());
        assert!(section_segments(&tri(), Point3::new(0.0, 0.0, 0.0), UnitVec3::Z).is_empty());
        assert!(
            section_segments(&Mesh::default(), Point3::new(0.0, 0.0, 0.0), UnitVec3::X).is_empty()
        );
    }

    #[test]
    fn plane_through_a_corner_within_tolerance() {
        // Through vertex (1000,0,0) only: a single point, no segment.
        let p = Point3::new(1000.0 + 0.005, 0.0, 0.0);
        assert!(section_segments(&tri(), p, UnitVec3::X).is_empty());
        // Through the vertex (0,0,0) and across the opposite edge.
        let n = Vec3::new(1.0, -1.0, 0.0).normalize().unwrap();
        let s = section_segments(&tri(), Point3::new(0.0, 0.0, 0.0), n);
        assert_eq!(s.len(), 1);
        let (a, b) = s[0];
        assert!((a.distance_to(b) - 1000.0 / 2f64.sqrt()).abs() < 1e-6);
    }
}
