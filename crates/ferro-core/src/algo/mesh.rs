use super::AlgoError;
use crate::geom::{Plane, Surface};
use crate::math::{Point3, Vec3};
use crate::topo::{Body, FaceId};
use serde::{Deserialize, Serialize};

/// Triangle mesh for display and export. Tessellation is a cache, never the
/// source of truth (§Q-C7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<Point3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
    /// Source face of each triangle (for picking).
    pub triangle_faces: Vec<FaceId>,
}

/// Triangulate one face (holes supported) with ear clipping in the face plane.
pub fn triangulate_face(body: &Body, face: FaceId) -> Result<Vec<[Point3; 3]>, AlgoError> {
    let f = body.face(face)?;
    let n = body.face_normal(face)?;
    let outer = body.loop_points(f.outer);
    let plane = Plane::from_origin_normal(outer[0], n);
    let mut pts: Vec<Point3> = outer;
    let mut holes = Vec::new();
    for h in &f.holes {
        holes.push(pts.len());
        pts.extend(body.loop_points(*h));
    }
    let flat: Vec<f64> = pts
        .iter()
        .flat_map(|p| {
            let q = plane.to_local(*p);
            [q.x, q.y]
        })
        .collect();
    let idx = earcutr::earcut(&flat, &holes, 2).map_err(|_| AlgoError::Triangulation)?;
    let mut tris = Vec::with_capacity(idx.len() / 3);
    for t in idx.as_chunks::<3>().0 {
        let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
        // earcut output winding is not guaranteed; orient to the face normal.
        if (b - a).cross(c - a).dot(n.as_vec()) >= 0.0 {
            tris.push([a, b, c]);
        } else {
            tris.push([a, c, b]);
        }
    }
    Ok(tris)
}

/// Mesh of a whole body. Planar faces get flat normals; curved faces get the
/// exact surface normal per vertex (smooth shading for cylinders).
pub fn body_mesh(body: &Body) -> Result<Mesh, AlgoError> {
    let mut m = Mesh::default();
    for fid in body.face_ids() {
        let face = body.face(fid)?;
        let flat = body.face_normal(fid)?.as_vec();
        for tri in triangulate_face(body, fid)? {
            let base = m.positions.len() as u32;
            for p in tri {
                m.positions.push(p);
                let n = match face.surface {
                    Surface::Plane(_) => flat,
                    Surface::Cylinder(c) => c
                        .normal_at(p)
                        .map(|u| u.as_vec())
                        .filter(|v| v.dot(flat) > 0.0)
                        .unwrap_or(flat),
                };
                m.normals.push(n);
            }
            m.indices.push([base, base + 1, base + 2]);
            m.triangle_faces.push(fid);
        }
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topo::tests_support::box_body;

    #[test]
    fn box_mesh_has_twelve_outward_triangles() {
        let (b, _) = box_body(1000.0, 2000.0, 3000.0);
        let m = body_mesh(&b).unwrap();
        assert_eq!(m.indices.len(), 12);
        let c = Point3::new(500.0, 1000.0, 1500.0);
        for t in &m.indices {
            let [a, b2, d] = t.map(|i| m.positions[i as usize]);
            let n = (b2 - a).cross(d - a);
            assert!((a - c).dot(n) > 0.0);
        }
    }

    #[test]
    fn face_with_hole_triangulates_to_correct_area() {
        let p = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(3.0, 0.0, 0.0),
            Point3::new(3.0, 3.0, 0.0),
            Point3::new(0.0, 3.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(2.0, 2.0, 0.0),
            Point3::new(2.0, 1.0, 0.0),
        ];
        let mut b = Body::default();
        let v: Vec<_> = p.iter().map(|x| b.add_vertex(*x)).collect();
        let mut spec = crate::topo::FaceSpec::new(v[0..4].to_vec());
        spec.holes = vec![v[4..8].to_vec()];
        let ids = b.rebuild(b.vertex_map(), vec![spec]).unwrap();
        let area: f64 = triangulate_face(&b, ids[0])
            .unwrap()
            .iter()
            .map(|[a, b2, c]| (*b2 - *a).cross(*c - *a).length() * 0.5)
            .sum();
        assert!((area - 8.0).abs() < 1e-9);
        assert!((b.face_area(ids[0]).unwrap() - 8.0).abs() < 1e-9);
    }
}
