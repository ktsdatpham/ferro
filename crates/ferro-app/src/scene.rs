//! Turns a `Document` into GPU vertex lists. Meshes are cached per element and
//! reused while the element's storage is shared between versions (D-24), so
//! previews only re-mesh what changed.

use crate::render::Vertex;
use ferro_core::algo::{body_mesh, Mesh};
use ferro_core::model::{Category, Document, ElementId};
use ferro_core::topo::FaceId;
use ferro_core::{BoundingBox3, Point3, UnitVec3};
use std::collections::{HashMap, HashSet};

pub const EDGE: [f32; 4] = [0.08, 0.09, 0.11, 1.0];
pub const SELECT: [f32; 4] = [0.18, 0.45, 0.95, 1.0];
/// Section cut lines.
pub const CUT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Display colour: the material of the element's type (core layer), else a
/// category colour.
fn element_color(doc: &Document, e: &ferro_core::Element) -> [f32; 4] {
    use ferro_core::model::{keys, ElementBody, ParamValue};
    if let Some(ParamValue::ElementRef(m)) = e.param(keys::MATERIAL) {
        if let Some(ElementBody::Material(mat)) = doc.get(*m).map(|x| &x.body) {
            let [r, g, b] = mat.color;
            return [r, g, b, 1.0];
        }
    }
    let structure = e
        .type_id
        .and_then(|t| doc.get(t))
        .and_then(|t| match &t.body {
            ElementBody::WallType(w) => Some(&w.structure),
            ElementBody::FloorType(f) => Some(&f.structure),
            _ => None,
        });
    let material = structure
        .and_then(|s| s.core_material())
        .and_then(|m| doc.get(m))
        .and_then(|m| match &m.body {
            ElementBody::Material(m) => Some(m.color),
            _ => None,
        });
    match material {
        Some([r, g, b]) => [r, g, b, 1.0],
        None => face_color(e.category),
    }
}

fn face_color(c: Category) -> [f32; 4] {
    match c {
        Category::Walls => [0.93, 0.90, 0.84, 1.0],
        Category::Floors => [0.82, 0.80, 0.76, 1.0],
        Category::Roofs => [0.72, 0.44, 0.36, 1.0],
        Category::Columns => [0.80, 0.80, 0.80, 1.0],
        Category::Furniture => [0.74, 0.63, 0.50, 1.0],
        Category::Doors => [0.62, 0.45, 0.30, 1.0],
        Category::Windows => [0.62, 0.78, 0.90, 1.0],
        _ => [0.90, 0.90, 0.88, 1.0],
    }
}

/// What the vertex lists were built for: origin bits, selection, X-ray,
/// wireframe, hidden line, hidden elements, section plane bits.
type SceneKey = (
    u64,
    Vec<ElementId>,
    bool,
    bool,
    bool,
    Vec<ElementId>,
    Vec<u64>,
);

struct Cached {
    doc: Document,
}

#[derive(Default)]
pub struct SceneCache {
    meshes: HashMap<ElementId, Mesh>,
    last: Option<Cached>,
    pub version: u64,
    pub tris: Vec<Vertex>,
    pub lines: Vec<Vertex>,
    key: Option<SceneKey>,
    /// Translucent faces (X-ray style).
    pub xray: bool,
    /// Edges only (Wireframe style).
    pub wireframe: bool,
    /// Elements not drawn (hidden, or on a hidden Tag). View state only.
    pub hidden: HashSet<ElementId>,
    /// White faces (Hidden line style).
    pub hidden_line: bool,
    /// Section plane (point, normal of the side cut away). View state only.
    pub section: Option<(Point3, UnitVec3)>,
    /// Where the section plane cuts faces (drawn over everything else).
    pub cut: Vec<Vertex>,
}

impl SceneCache {
    /// Rebuild vertex lists if `doc` (or the selection) differs from last time.
    pub fn update(&mut self, doc: &Document, origin: Point3, selection: &HashSet<ElementId>) {
        let mut sel: Vec<ElementId> = selection.iter().copied().collect();
        sel.sort();
        let same_doc = self.last.as_ref().is_some_and(|c| same_doc(&c.doc, doc));
        let key = (
            origin.x.to_bits()
                ^ origin.y.to_bits().rotate_left(21)
                ^ origin.z.to_bits().rotate_left(42),
            sel.clone(),
            self.xray,
            self.wireframe,
            self.hidden_line,
            {
                let mut h: Vec<ElementId> = self.hidden.iter().copied().collect();
                h.sort();
                h
            },
            self.section
                .map(|(p, n)| {
                    let n = n.as_vec();
                    vec![p.x, p.y, p.z, n.x, n.y, n.z]
                        .into_iter()
                        .map(f64::to_bits)
                        .collect()
                })
                .unwrap_or_default(),
        );
        if same_doc && self.key.as_ref() == Some(&key) {
            return;
        }
        // Re-mesh changed elements only.
        let mut fresh = HashMap::new();
        for e in doc.elements() {
            let Some(g) = e.geometry() else { continue };
            let reuse = self
                .last
                .as_ref()
                .filter(|c| c.doc.same_element(doc, e.id))
                .and_then(|_| self.meshes.remove(&e.id));
            let m = match reuse {
                Some(m) => m,
                None => body_mesh(g).unwrap_or_default(),
            };
            fresh.insert(e.id, m);
        }
        self.meshes = fresh;
        self.last = Some(Cached { doc: doc.clone() });
        self.key = Some(key);
        self.version += 1;

        self.tris.clear();
        self.lines.clear();
        self.cut.clear();
        let rel = |p: Point3| {
            let v = p - origin;
            [v.x as f32, v.y as f32, v.z as f32]
        };
        for e in doc.elements() {
            let Some(g) = e.geometry() else { continue };
            if self.hidden.contains(&e.id) {
                continue;
            }
            let selected = selection.contains(&e.id);
            let color = if selected {
                let c = element_color(doc, e);
                [
                    c[0] * 0.7 + 0.3 * SELECT[0],
                    c[1] * 0.7 + 0.3 * SELECT[1],
                    c[2] * 0.7 + 0.3 * SELECT[2],
                    1.0,
                ]
            } else {
                element_color(doc, e)
            };
            let color = if self.hidden_line {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                color
            };
            let color = if self.xray {
                [color[0], color[1], color[2], 0.3]
            } else {
                color
            };
            if let Some(m) = self.meshes.get(&e.id).filter(|_| !self.wireframe) {
                for t in &m.indices {
                    for &i in t {
                        let n = m.normals[i as usize];
                        self.tris.push(Vertex {
                            pos: rel(m.positions[i as usize]),
                            normal: [n.x as f32, n.y as f32, n.z as f32],
                            color,
                        });
                    }
                }
            }
            if let (Some((p, n)), Some(m)) = (self.section, self.meshes.get(&e.id)) {
                for (a, b) in section_segments(m, p, n) {
                    self.cut.push(line_vertex(rel(a), CUT));
                    self.cut.push(line_vertex(rel(b), CUT));
                }
            }
            let edge = if selected { SELECT } else { EDGE };
            // Hide edges inside smooth groups (facets of one curved surface).
            for (a, b) in visible_edges(g) {
                self.lines.push(line_vertex(rel(a), edge));
                self.lines.push(line_vertex(rel(b), edge));
            }
        }
    }

    pub fn mesh(&self, id: ElementId) -> Option<&Mesh> {
        self.meshes.get(&id)
    }
}

/// Segments where the plane through `p` with normal `n` crosses the mesh's
/// triangles (the section cut). Triangles lying in the plane give none.
pub fn section_segments(m: &Mesh, p: Point3, n: UnitVec3) -> Vec<(Point3, Point3)> {
    let n = n.as_vec();
    let side = |q: Point3| {
        let d = n.dot(q - p);
        if d.abs() <= ferro_core::math::tol::VERTEX_TOL {
            0.0
        } else {
            d
        }
    };
    let mut out = Vec::new();
    for t in &m.indices {
        let v = t.map(|i| m.positions[i as usize]);
        let d = v.map(side);
        let mut pts: Vec<Point3> = Vec::with_capacity(2);
        for i in 0..3 {
            let (a, b, da, db) = (v[i], v[(i + 1) % 3], d[i], d[(i + 1) % 3]);
            if da == 0.0 {
                pts.push(a);
            } else if da * db < 0.0 {
                pts.push(a + (b - a) * (da / (da - db)));
            }
        }
        pts.dedup_by(|x, y| x.is_almost_equal(*y));
        if let [a, b] = pts[..] {
            // An edge lying in the plane belongs to two triangles: the cut
            // shows it once each, which is harmless when drawn.
            out.push((a, b));
        }
    }
    out
}

/// Bit pattern of an unordered segment (exact endpoint match).
type SegKey = (u64, u64, u64, u64, u64, u64);

/// Edges to draw: all edges except those between two faces of the same smooth group.
fn visible_edges(body: &ferro_core::topo::Body) -> Vec<(Point3, Point3)> {
    // Map each edge segment to the smooth groups of the faces using it.
    let mut groups: HashMap<SegKey, Vec<Option<u32>>> = HashMap::new();
    let key = |a: Point3, b: Point3| {
        let (p, q) = if (a.x, a.y, a.z) < (b.x, b.y, b.z) {
            (a, b)
        } else {
            (b, a)
        };
        (
            p.x.to_bits(),
            p.y.to_bits(),
            p.z.to_bits(),
            q.x.to_bits(),
            q.y.to_bits(),
            q.z.to_bits(),
        )
    };
    for fid in body.face_ids() {
        let Ok(face) = body.face(fid) else { continue };
        let g = face.smooth_group;
        for l in std::iter::once(&face.outer).chain(&face.holes) {
            let pts = body.loop_points(*l);
            for i in 0..pts.len() {
                groups
                    .entry(key(pts[i], pts[(i + 1) % pts.len()]))
                    .or_default()
                    .push(g);
            }
        }
    }
    body.edges()
        .filter(|(_, a, b)| {
            let gs = groups.get(&key(*a, *b));
            !matches!(gs.map(|v| v.as_slice()), Some([Some(x), Some(y)]) if x == y)
        })
        .map(|(_, a, b)| (a, b))
        .collect()
}

/// Cheap identity check: same version and every element shares storage.
pub fn same_doc(a: &Document, b: &Document) -> bool {
    a.version() == b.version()
        && a.len() == b.len()
        && a.elements().all(|e| a.same_element(b, e.id))
}

/// An edge vertex (drawn over its faces).
pub fn line_vertex(pos: [f32; 3], color: [f32; 4]) -> Vertex {
    Vertex {
        pos,
        normal: [1.0, 0.0, 0.0],
        color,
    }
}

/// A grid / guide vertex (depth-tested normally, hidden by faces).
pub fn guide_vertex(pos: [f32; 3], color: [f32; 4]) -> Vertex {
    Vertex {
        pos,
        normal: [0.0, 0.0, 0.0],
        color,
    }
}

/// Ground grid (major every 5 cells) and the red / green / blue axes.
///
/// Lines are split per cell and any segment reaching behind the camera is
/// dropped, so no primitive crosses the near plane (some drivers rasterise
/// such lines badly).
pub fn grid_lines(
    origin: Point3,
    center: Point3,
    extent: f64,
    eye: Point3,
    forward: ferro_core::Vec3,
    near: f64,
) -> Vec<Vertex> {
    let mut v = Vec::new();
    // Keep roughly 60 lines per direction whatever the zoom.
    let step = [1000.0, 5000.0, 10_000.0, 50_000.0]
        .into_iter()
        .find(|s| extent / s <= 60.0)
        .unwrap_or(100_000.0);
    let n = (extent / step).ceil() as i64;
    let cx = (center.x / step).round() * step;
    let cy = (center.y / step).round() * step;
    let in_front = |p: Point3| (p - eye).dot(forward) > near;
    let seg = |a: Point3, b: Point3, c: [f32; 4], v: &mut Vec<Vertex>| {
        if in_front(a) && in_front(b) {
            let r = |p: Point3| {
                [
                    (p.x - origin.x) as f32,
                    (p.y - origin.y) as f32,
                    (p.z - origin.z) as f32,
                ]
            };
            v.push(guide_vertex(r(a), c));
            v.push(guide_vertex(r(b), c));
        }
    };
    for i in -n..=n {
        let major = ((cx / step) as i64 + i).rem_euclid(5) == 0;
        let c = if major {
            [0.60, 0.63, 0.68, 0.45]
        } else {
            [0.72, 0.74, 0.78, 0.22]
        };
        let x = cx + i as f64 * step;
        let y = cy + i as f64 * step;
        for k in -n..n {
            let t0 = k as f64 * step;
            let t1 = t0 + step;
            seg(
                Point3::new(x, cy + t0, 0.0),
                Point3::new(x, cy + t1, 0.0),
                c,
                &mut v,
            );
            seg(
                Point3::new(cx + t0, y, 0.0),
                Point3::new(cx + t1, y, 0.0),
                c,
                &mut v,
            );
        }
    }
    let l = extent * 1.2;
    let pieces = 48;
    for (d, c) in [
        (ferro_core::Vec3::X, [0.85, 0.15, 0.15, 1.0]),
        (ferro_core::Vec3::Y, [0.15, 0.65, 0.20, 1.0]),
        (ferro_core::Vec3::Z, [0.15, 0.30, 0.90, 1.0]),
    ] {
        for k in 0..pieces {
            let a = Point3::ORIGIN + d * (l * k as f64 / pieces as f64);
            let b = Point3::ORIGIN + d * (l * (k + 1) as f64 / pieces as f64);
            seg(a, b, c, &mut v);
        }
    }
    v
}

/// Triangles of one face, for hover highlighting.
pub fn face_triangles(mesh: &Mesh, face: FaceId) -> Vec<[Point3; 3]> {
    mesh.indices
        .iter()
        .zip(&mesh.triangle_faces)
        .filter(|(_, f)| **f == face)
        .map(|(t, _)| t.map(|i| mesh.positions[i as usize]))
        .collect()
}

pub fn doc_bounds(doc: &Document) -> Option<BoundingBox3> {
    doc.elements()
        .filter_map(|e| e.bounding_box())
        .reduce(|a, b| a.union(b))
}

#[cfg(test)]
mod section_tests {
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
