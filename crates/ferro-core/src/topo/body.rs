use super::{EdgeId, FaceId, HalfEdgeId, LoopId, TopoError, VertexId};
use crate::geom::{polygon, Plane, Surface};
use crate::math::{tol, BoundingBox3, Point3, Transform, UnitVec3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub point: Point3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HalfEdge {
    pub origin: VertexId,
    pub twin: Option<HalfEdgeId>,
    pub next: HalfEdgeId,
    pub prev: HalfEdgeId,
    pub loop_id: LoopId,
    pub edge: EdgeId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loop {
    pub first: HalfEdgeId,
    pub face: FaceId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub outer: LoopId,
    pub holes: Vec<LoopId>,
    pub surface: Surface,
    /// Faces sharing a smooth group render with shared normals (facets of one
    /// curved surface).
    pub smooth_group: Option<u32>,
}

/// Input for building a face: vertex loops in counter-clockwise order about
/// the outward normal (holes clockwise).
#[derive(Debug, Clone, PartialEq)]
pub struct FaceSpec {
    /// Keep this id (when rebuilding); `None` allocates a new one.
    pub id: Option<FaceId>,
    pub outer: Vec<VertexId>,
    pub holes: Vec<Vec<VertexId>>,
    /// `None` = plane through the outer loop.
    pub surface: Option<Surface>,
    pub smooth_group: Option<u32>,
}

impl FaceSpec {
    pub fn new(outer: Vec<VertexId>) -> Self {
        FaceSpec {
            id: None,
            outer,
            holes: Vec::new(),
            surface: None,
            smooth_group: None,
        }
    }
}

/// A half-edge B-rep body. Maps are ordered for deterministic iteration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Body {
    vertices: BTreeMap<VertexId, Vertex>,
    half_edges: BTreeMap<HalfEdgeId, HalfEdge>,
    edges: BTreeMap<EdgeId, [Option<HalfEdgeId>; 2]>,
    loops: BTreeMap<LoopId, Loop>,
    faces: BTreeMap<FaceId, Face>,
    /// Single counter for all ids: an id is never reused (D-25).
    next_id: u32,
}

impl Default for Body {
    fn default() -> Self {
        Body {
            vertices: BTreeMap::new(),
            half_edges: BTreeMap::new(),
            edges: BTreeMap::new(),
            loops: BTreeMap::new(),
            faces: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl Body {
    // ---------------------------------------------------------------- building

    /// Build a body from points and face loops given as indices into `points`.
    /// Returns the body and the face ids in input order.
    pub fn from_polygons(
        points: &[Point3],
        faces: &[Vec<usize>],
    ) -> Result<(Body, Vec<FaceId>), TopoError> {
        let mut b = Body::default();
        let vids: Vec<VertexId> = points.iter().map(|p| b.add_vertex(*p)).collect();
        let specs = faces
            .iter()
            .map(|f| {
                f.iter()
                    .map(|&i| {
                        vids.get(i)
                            .copied()
                            .ok_or_else(|| TopoError::NotFound(format!("point index {i}")))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(FaceSpec::new)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let vertices = b.vertices.clone();
        let ids = b.rebuild(vertices, specs)?;
        Ok((b, ids))
    }

    /// Allocate a new vertex (not yet used by any face).
    pub fn add_vertex(&mut self, p: Point3) -> VertexId {
        let id = VertexId(self.alloc());
        self.vertices.insert(id, Vertex { point: p });
        id
    }

    fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Replace all faces. Vertices not referenced by any face are dropped.
    /// Edge, half-edge and loop ids are reused where the same vertex pair /
    /// face loop existed before, so ids stay stable across edits.
    pub fn rebuild(
        &mut self,
        vertices: BTreeMap<VertexId, Vertex>,
        specs: Vec<FaceSpec>,
    ) -> Result<Vec<FaceId>, TopoError> {
        // Previous ids keyed by geometry-independent keys.
        let old_edge: HashMap<(VertexId, VertexId), EdgeId> = self
            .half_edges
            .values()
            .map(|h| (self.edge_key(h), h.edge))
            .collect();
        let old_he: HashMap<(VertexId, VertexId), HalfEdgeId> = self
            .half_edges
            .iter()
            .map(|(id, h)| ((h.origin, self.half_edges[&h.next].origin), *id))
            .collect();
        let old_loops: HashMap<FaceId, Vec<LoopId>> = self
            .faces
            .iter()
            .map(|(fid, f)| {
                let mut v = vec![f.outer];
                v.extend(&f.holes);
                (*fid, v)
            })
            .collect();

        let mut nb = Body {
            vertices: BTreeMap::new(),
            half_edges: BTreeMap::new(),
            edges: BTreeMap::new(),
            loops: BTreeMap::new(),
            faces: BTreeMap::new(),
            next_id: self.next_id,
        };
        let mut directed: HashMap<(VertexId, VertexId), HalfEdgeId> = HashMap::new();
        let mut face_ids = Vec::with_capacity(specs.len());

        for spec in specs {
            let fid = match spec.id {
                Some(id) => {
                    if nb.faces.contains_key(&id) {
                        return Err(TopoError::Corrupt(format!("duplicate face {id}")));
                    }
                    id
                }
                None => FaceId(nb.alloc()),
            };
            let reuse_loops = old_loops.get(&fid).cloned().unwrap_or_default();
            let mut loop_ids = Vec::new();
            for (li, verts) in std::iter::once(&spec.outer).chain(&spec.holes).enumerate() {
                Self::check_loop(verts, &vertices)?;
                let lid = match reuse_loops.get(li) {
                    Some(l) if !nb.loops.contains_key(l) => *l,
                    _ => LoopId(nb.alloc()),
                };
                let n = verts.len();
                let mut hes = Vec::with_capacity(n);
                for i in 0..n {
                    let key = (verts[i], verts[(i + 1) % n]);
                    if directed.contains_key(&key) {
                        return Err(TopoError::NonManifold);
                    }
                    let hid = match old_he.get(&key) {
                        Some(h) if !nb.half_edges.contains_key(h) => *h,
                        _ => HalfEdgeId(nb.alloc()),
                    };
                    directed.insert(key, hid);
                    hes.push(hid);
                }
                for i in 0..n {
                    nb.half_edges.insert(
                        hes[i],
                        HalfEdge {
                            origin: verts[i],
                            twin: None,
                            next: hes[(i + 1) % n],
                            prev: hes[(i + n - 1) % n],
                            loop_id: lid,
                            edge: EdgeId(0), // assigned below
                        },
                    );
                }
                nb.loops.insert(
                    lid,
                    Loop {
                        first: hes[0],
                        face: fid,
                    },
                );
                loop_ids.push(lid);
            }
            let surface = match spec.surface {
                Some(s) => s,
                None => {
                    let pts: Vec<Point3> = spec.outer.iter().map(|v| vertices[v].point).collect();
                    let n = polygon::polygon_normal(&pts).ok_or(TopoError::DegenerateLoop)?;
                    Surface::Plane(Plane::from_origin_normal(pts[0], n))
                }
            };
            nb.faces.insert(
                fid,
                Face {
                    outer: loop_ids[0],
                    holes: loop_ids[1..].to_vec(),
                    surface,
                    smooth_group: spec.smooth_group,
                },
            );
            face_ids.push(fid);
        }

        // Twins and edges.
        let keys: Vec<((VertexId, VertexId), HalfEdgeId)> =
            directed.iter().map(|(k, v)| (*k, *v)).collect();
        let mut sorted = keys;
        sorted.sort();
        for ((a, b), hid) in sorted {
            if nb.half_edges[&hid].edge != EdgeId(0) {
                continue;
            }
            let twin = directed.get(&(b, a)).copied();
            let key = if a < b { (a, b) } else { (b, a) };
            let eid = match old_edge.get(&key) {
                Some(e) if !nb.edges.contains_key(e) => *e,
                _ => EdgeId(nb.alloc()),
            };
            nb.edges.insert(eid, [Some(hid), twin]);
            if let Some(h) = nb.half_edges.get_mut(&hid) {
                h.edge = eid;
                h.twin = twin;
            }
            if let Some(t) = twin.and_then(|t| nb.half_edges.get_mut(&t)) {
                t.edge = eid;
                t.twin = Some(hid);
            }
        }

        // Keep only referenced vertices.
        for h in nb.half_edges.values() {
            if let Some(v) = vertices.get(&h.origin) {
                nb.vertices.insert(h.origin, v.clone());
            }
        }
        // Never move the counter backwards (allocations by the caller).
        nb.next_id = nb.next_id.max(self.next_id);
        *self = nb;
        Ok(face_ids)
    }

    fn check_loop(
        verts: &[VertexId],
        vertices: &BTreeMap<VertexId, Vertex>,
    ) -> Result<(), TopoError> {
        if verts.len() < 3 {
            return Err(TopoError::DegenerateLoop);
        }
        let mut seen = std::collections::HashSet::new();
        for v in verts {
            if !vertices.contains_key(v) {
                return Err(TopoError::NotFound(format!("vertex {v}")));
            }
            if !seen.insert(*v) {
                return Err(TopoError::DegenerateLoop);
            }
        }
        Ok(())
    }

    fn edge_key(&self, h: &HalfEdge) -> (VertexId, VertexId) {
        let a = h.origin;
        let b = self.half_edges[&h.next].origin;
        if a < b {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// Current faces as specs (with ids), for algorithms that rebuild.
    pub fn face_specs(&self) -> Vec<FaceSpec> {
        self.faces
            .iter()
            .map(|(fid, f)| FaceSpec {
                id: Some(*fid),
                outer: self.loop_vertices(f.outer),
                holes: f.holes.iter().map(|l| self.loop_vertices(*l)).collect(),
                surface: Some(f.surface),
                smooth_group: f.smooth_group,
            })
            .collect()
    }

    pub fn vertex_map(&self) -> BTreeMap<VertexId, Vertex> {
        self.vertices.clone()
    }

    /// Allocate a fresh smooth-group number.
    pub fn alloc_smooth_group(&mut self) -> u32 {
        self.alloc()
    }

    // ---------------------------------------------------------------- queries

    pub fn face_ids(&self) -> impl Iterator<Item = FaceId> + '_ {
        self.faces.keys().copied()
    }
    pub fn face(&self, id: FaceId) -> Result<&Face, TopoError> {
        self.faces
            .get(&id)
            .ok_or_else(|| TopoError::NotFound(format!("face {id}")))
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
    pub fn point(&self, v: VertexId) -> Option<Point3> {
        self.vertices.get(&v).map(|x| x.point)
    }
    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, Point3)> + '_ {
        self.vertices.iter().map(|(id, v)| (*id, v.point))
    }

    pub fn loop_vertices(&self, l: LoopId) -> Vec<VertexId> {
        let Some(lp) = self.loops.get(&l) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut h = lp.first;
        loop {
            let he = &self.half_edges[&h];
            out.push(he.origin);
            h = he.next;
            if h == lp.first || out.len() > self.half_edges.len() {
                break;
            }
        }
        out
    }

    pub fn loop_points(&self, l: LoopId) -> Vec<Point3> {
        self.loop_vertices(l)
            .iter()
            .map(|v| self.vertices[v].point)
            .collect()
    }

    /// Outer loop points of a face.
    pub fn face_points(&self, f: FaceId) -> Result<Vec<Point3>, TopoError> {
        Ok(self.loop_points(self.face(f)?.outer))
    }

    /// Area vector of a face: Σ over loops of ½ Σ pᵢ × pᵢ₊₁ (holes subtract
    /// because they run clockwise).
    pub fn face_area_vector(&self, f: FaceId) -> Result<Vec3, TopoError> {
        let face = self.face(f)?;
        let mut a = Vec3::ZERO;
        for l in std::iter::once(&face.outer).chain(&face.holes) {
            let pts = self.loop_points(*l);
            let o = pts[0];
            for i in 1..pts.len() - 1 {
                a += (pts[i] - o).cross(pts[i + 1] - o) * 0.5;
            }
        }
        Ok(a)
    }

    pub fn face_normal(&self, f: FaceId) -> Result<UnitVec3, TopoError> {
        match self.face(f)?.surface {
            Surface::Plane(p) => Ok(p.normal()),
            Surface::Cylinder(_) => self
                .face_area_vector(f)?
                .normalize()
                .ok_or(TopoError::DegenerateLoop),
        }
    }

    pub fn face_area(&self, f: FaceId) -> Result<f64, TopoError> {
        Ok(self.face_area_vector(f)?.length())
    }

    /// Triangles covering face `f` (holes supported), as vertex ids,
    /// counter-clockwise about the face normal: ear clipping in the face
    /// plane. Collinear boundary vertices may be left out of every triangle.
    ///
    /// Fails with `NotFound` for an unknown face and `DegenerateLoop` when
    /// the face cannot be triangulated.
    pub fn face_triangles(&self, f: FaceId) -> Result<Vec<[VertexId; 3]>, TopoError> {
        let face = self.face(f)?;
        let n = self.face_normal(f)?;
        let mut ids = self.loop_vertices(face.outer);
        let first = self
            .point(*ids.first().ok_or(TopoError::DegenerateLoop)?)
            .ok_or(TopoError::DegenerateLoop)?;
        let plane = Plane::from_origin_normal(first, n);
        let mut holes = Vec::new();
        for h in &face.holes {
            holes.push(ids.len());
            ids.extend(self.loop_vertices(*h));
        }
        let pts: Vec<Point3> = ids
            .iter()
            .map(|v| self.point(*v).ok_or(TopoError::DegenerateLoop))
            .collect::<Result<_, _>>()?;
        let flat: Vec<f64> = pts
            .iter()
            .flat_map(|p| {
                let q = plane.to_local(*p);
                [q.x, q.y]
            })
            .collect();
        let idx = earcutr::earcut(&flat, &holes, 2).map_err(|_| TopoError::DegenerateLoop)?;
        let mut tris = Vec::with_capacity(idx.len() / 3);
        for t in idx.chunks_exact(3) {
            let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            // earcut output winding is not guaranteed; orient to the normal.
            if (b - a).cross(c - a).dot(n.as_vec()) >= 0.0 {
                tris.push([ids[t[0]], ids[t[1]], ids[t[2]]]);
            } else {
                tris.push([ids[t[0]], ids[t[2]], ids[t[1]]]);
            }
        }
        Ok(tris)
    }

    /// Faces adjacent to `f` across its outer-loop edges, in loop order
    /// (`None` for open edges).
    pub fn neighbors(&self, f: FaceId) -> Result<Vec<Option<FaceId>>, TopoError> {
        let face = self.face(f)?;
        let first = self.loops[&face.outer].first;
        let mut out = Vec::new();
        let mut h = first;
        loop {
            let he = &self.half_edges[&h];
            out.push(he.twin.map(|t| {
                let lid = self.half_edges[&t].loop_id;
                self.loops[&lid].face
            }));
            h = he.next;
            if h == first {
                break;
            }
        }
        Ok(out)
    }

    /// All edges as segments, for rendering and snapping.
    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, Point3, Point3)> + '_ {
        self.edges.iter().filter_map(|(eid, hs)| {
            let h = &self.half_edges[&hs[0]?];
            let a = self.vertices[&h.origin].point;
            let b = self.vertices[&self.half_edges[&h.next].origin].point;
            Some((*eid, a, b))
        })
    }

    /// `true` if every edge has two faces.
    pub fn is_closed(&self) -> bool {
        !self.half_edges.is_empty() && self.half_edges.values().all(|h| h.twin.is_some())
    }

    /// Signed volume (mm³) by the divergence theorem; meaningful when closed.
    pub fn volume(&self) -> f64 {
        self.faces
            .keys()
            .filter_map(|f| {
                let p = self.face_points(*f).ok()?[0];
                Some(p.to_vec().dot(self.face_area_vector(*f).ok()?))
            })
            .sum::<f64>()
            / 3.0
    }

    pub fn bounding_box(&self) -> Option<BoundingBox3> {
        BoundingBox3::from_points(self.vertices.values().map(|v| v.point))
    }

    // ---------------------------------------------------------------- edits

    /// Move vertices (ids unchanged). Planar face surfaces are refitted.
    pub fn move_vertices(&mut self, moves: &[(VertexId, Point3)]) -> Result<(), TopoError> {
        for (v, p) in moves {
            self.vertices
                .get_mut(v)
                .ok_or_else(|| TopoError::NotFound(format!("vertex {v}")))?
                .point = *p;
        }
        self.refit_planes();
        Ok(())
    }

    /// Apply a transform to all geometry.
    pub fn transform(&mut self, t: &Transform) {
        for v in self.vertices.values_mut() {
            v.point = t.apply(v.point);
        }
        for f in self.faces.values_mut() {
            // Carry the plane frame along so its in-plane axis follows the
            // rotation (refit below snaps it to the moved points).
            if let Surface::Plane(pl) = &mut f.surface {
                if let Ok(moved) = pl.transformed(t) {
                    *pl = moved;
                }
            }
            if let Surface::Cylinder(c) = &mut f.surface {
                c.axis_origin = t.apply(c.axis_origin);
                if let Some(a) = t.apply_dir(c.axis) {
                    c.axis = a;
                }
                c.radius *= t.basis_x.length();
            }
        }
        if t.has_reflection() {
            // Reflection flips orientation: reverse every loop to stay outward.
            let specs: Vec<FaceSpec> = self
                .face_specs()
                .into_iter()
                .map(|mut s| {
                    s.outer.reverse();
                    s.holes.iter_mut().for_each(|h| h.reverse());
                    s.surface = match s.surface {
                        Some(Surface::Plane(_)) => None,
                        other => other,
                    };
                    s
                })
                .collect();
            let verts = self.vertices.clone();
            let _ = self.rebuild(verts, specs);
        }
        self.refit_planes();
    }

    fn refit_planes(&mut self) {
        let fids: Vec<FaceId> = self.faces.keys().copied().collect();
        for f in fids {
            if !self.faces[&f].surface.is_planar() {
                continue;
            }
            let pts = self.loop_points(self.faces[&f].outer);
            if let Some(n) = polygon::polygon_normal(&pts) {
                let old = match self.faces[&f].surface {
                    Surface::Plane(p) => p,
                    _ => unreachable!(),
                };
                // Keep the in-plane X axis when possible so textures don't
                // spin: the old axis projected into the new plane.
                let ox = old.x_axis().as_vec();
                let x = ox - n.as_vec() * ox.dot(n.as_vec());
                let pl = Plane::from_frame(pts[0], x, n.as_vec().cross(x))
                    .ok()
                    .filter(|p| p.normal().dot(n) > 1.0 - tol::ANGLE_TOL)
                    .unwrap_or_else(|| Plane::from_origin_normal(pts[0], n));
                if let Some(face) = self.faces.get_mut(&f) {
                    face.surface = Surface::Plane(pl);
                }
            }
        }
    }

    // ---------------------------------------------------------------- checks

    /// Validity of the body: links consistent, manifold, planar faces planar,
    /// no degenerate face, and no self-intersection (faces sharing no vertex
    /// do not touch; exact predicates, D-53). Fails with the matching
    /// `TopoError` (`SelfIntersecting` for the last rule).
    pub fn validate(&self) -> Result<(), TopoError> {
        let corrupt = |s: String| Err(TopoError::Corrupt(s));
        for (hid, h) in &self.half_edges {
            let Some(next) = self.half_edges.get(&h.next) else {
                return corrupt(format!("{hid}.next"));
            };
            if next.prev != *hid {
                return corrupt(format!("{hid}.next.prev"));
            }
            if next.loop_id != h.loop_id {
                return corrupt(format!("{hid} loop"));
            }
            if let Some(t) = h.twin {
                let Some(tw) = self.half_edges.get(&t) else {
                    return corrupt(format!("{hid}.twin"));
                };
                if tw.twin != Some(*hid) || tw.origin != next.origin || tw.edge != h.edge {
                    return Err(TopoError::NonManifold);
                }
            }
            if !self.vertices.contains_key(&h.origin) {
                return corrupt(format!("{hid}.origin"));
            }
        }
        for (fid, f) in &self.faces {
            for l in std::iter::once(&f.outer).chain(&f.holes) {
                if self.loops.get(l).map(|x| x.face) != Some(*fid) {
                    return corrupt(format!("{fid} loop {l}"));
                }
            }
            if let Surface::Plane(pl) = f.surface {
                for l in std::iter::once(&f.outer).chain(&f.holes) {
                    if self.loop_points(*l).iter().any(|p| !pl.contains(*p)) {
                        return Err(TopoError::NonPlanarFace(fid.to_string()));
                    }
                }
            }
            if self.face_area(*fid)? < tol::VERTEX_TOL * tol::VERTEX_TOL {
                return Err(TopoError::DegenerateLoop);
            }
        }
        self.check_self_intersection()
    }

    /// Faces meet only along shared edges and vertices (D-11): two planar
    /// faces that share no vertex must not touch at all. Decided with exact
    /// predicates (D-53) on the faces' triangles, after a sweep over face
    /// bounding boxes and a per-triangle box test.
    ///
    /// Limitations: faces sharing a vertex are not compared, curved faces
    /// are skipped, and so are faces that cannot be triangulated.
    fn check_self_intersection(&self) -> Result<(), TopoError> {
        struct Item {
            face: FaceId,
            verts: std::collections::BTreeSet<VertexId>,
            tris: Vec<([Point3; 3], BoundingBox3)>,
            bb: BoundingBox3,
        }
        let mut items: Vec<Item> = Vec::new();
        for (fid, f) in &self.faces {
            if !f.surface.is_planar() {
                continue;
            }
            let Ok(tris) = self.face_triangles(*fid) else {
                continue;
            };
            let mut out = Vec::with_capacity(tris.len());
            for t in tris {
                let (Some(a), Some(b), Some(c)) =
                    (self.point(t[0]), self.point(t[1]), self.point(t[2]))
                else {
                    continue;
                };
                let p = [a, b, c];
                if crate::geom::predicates::triangle_is_degenerate(p) {
                    continue;
                }
                if let Some(bb) = BoundingBox3::from_points(p) {
                    out.push((p, bb));
                }
            }
            let Some(bb) = out.iter().map(|t| t.1).reduce(BoundingBox3::union) else {
                continue;
            };
            let verts = std::iter::once(&f.outer)
                .chain(&f.holes)
                .flat_map(|l| self.loop_vertices(*l))
                .collect();
            items.push(Item {
                face: *fid,
                verts,
                tris: out,
                bb,
            });
        }
        // Sweep along x (stable sort: deterministic order).
        items.sort_by(|a, b| a.bb.min.x.total_cmp(&b.bb.min.x));
        for i in 0..items.len() {
            let a = &items[i];
            for b in &items[i + 1..] {
                if b.bb.min.x > a.bb.max.x {
                    break;
                }
                if !a.bb.intersects(b.bb) || a.verts.intersection(&b.verts).next().is_some() {
                    continue;
                }
                for (ta, ba) in &a.tris {
                    for (tb, bbx) in &b.tris {
                        if ba.intersects(*bbx)
                            && crate::geom::predicates::triangles_intersect(*ta, *tb)
                        {
                            let (x, y) = if a.face < b.face {
                                (a.face, b.face)
                            } else {
                                (b.face, a.face)
                            };
                            return Err(TopoError::SelfIntersecting(format!("faces {x} and {y}")));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Validity as a solid: structurally valid, closed and outward-facing (D-11).
    pub fn validate_solid(&self) -> Result<(), TopoError> {
        self.validate()?;
        if !self.is_closed() {
            return Err(TopoError::NotClosed);
        }
        if self.volume() <= 0.0 {
            return Err(TopoError::InwardFacing);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// Axis-aligned box `w × d × h` at the origin, outward faces.
    pub(crate) fn box_body(w: f64, d: f64, h: f64) -> (Body, Vec<FaceId>) {
        let p = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(w, 0.0, 0.0),
            Point3::new(w, d, 0.0),
            Point3::new(0.0, d, 0.0),
            Point3::new(0.0, 0.0, h),
            Point3::new(w, 0.0, h),
            Point3::new(w, d, h),
            Point3::new(0.0, d, h),
        ];
        let faces = vec![
            vec![0, 3, 2, 1], // bottom
            vec![4, 5, 6, 7], // top
            vec![0, 1, 5, 4], // south
            vec![1, 2, 6, 5], // east
            vec![2, 3, 7, 6], // north
            vec![3, 0, 4, 7], // west
        ];
        Body::from_polygons(&p, &faces).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::box_body;
    use super::*;

    #[test]
    fn box_is_a_valid_solid() {
        let (b, _) = box_body(4000.0, 3000.0, 2800.0);
        b.validate_solid().unwrap();
        assert_eq!(
            (b.vertex_count(), b.edge_count(), b.face_count()),
            (8, 12, 6)
        );
        assert!((b.volume() - 4000.0 * 3000.0 * 2800.0).abs() < 1.0);
    }

    #[test]
    fn inverted_box_is_rejected() {
        let (b, _) = box_body(1000.0, 1000.0, 1000.0);
        let specs: Vec<FaceSpec> = b
            .face_specs()
            .into_iter()
            .map(|mut s| {
                s.outer.reverse();
                s.surface = None;
                s
            })
            .collect();
        let mut inv = b.clone();
        inv.rebuild(b.vertex_map(), specs).unwrap();
        assert_eq!(inv.validate_solid(), Err(TopoError::InwardFacing));
    }

    #[test]
    fn single_face_is_open_but_valid() {
        let (b, _) = Body::from_polygons(
            &[
                Point3::ORIGIN,
                Point3::new(1000.0, 0.0, 0.0),
                Point3::new(1000.0, 1000.0, 0.0),
            ],
            &[vec![0, 1, 2]],
        )
        .unwrap();
        b.validate().unwrap();
        assert!(!b.is_closed());
        assert_eq!(b.validate_solid(), Err(TopoError::NotClosed));
    }

    #[test]
    fn same_direction_edge_twice_is_non_manifold() {
        let p = [
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
            Point3::new(0.0, -1.0, 0.0),
        ];
        assert_eq!(
            Body::from_polygons(&p, &[vec![0, 1, 2], vec![0, 1, 3]]).map(|_| ()),
            Err(TopoError::NonManifold)
        );
    }

    #[test]
    fn degenerate_loops_fail() {
        let p = [Point3::ORIGIN, Point3::new(1.0, 0.0, 0.0)];
        assert_eq!(
            Body::from_polygons(&p, &[vec![0, 1]]).map(|_| ()),
            Err(TopoError::DegenerateLoop)
        );
    }

    #[test]
    fn rebuild_keeps_face_and_edge_ids() {
        let (mut b, ids) = box_body(1000.0, 1000.0, 1000.0);
        let edges_before: Vec<EdgeId> = b.edges().map(|e| e.0).collect();
        let specs = b.face_specs();
        b.rebuild(b.vertex_map(), specs).unwrap();
        assert_eq!(b.face_ids().collect::<Vec<_>>(), {
            let mut s = ids.clone();
            s.sort();
            s
        });
        assert_eq!(b.edges().map(|e| e.0).collect::<Vec<_>>(), edges_before);
    }

    #[test]
    fn neighbors_of_box_top_are_the_four_sides() {
        let (b, ids) = box_body(1000.0, 1000.0, 1000.0);
        let n = b.neighbors(ids[1]).unwrap();
        assert_eq!(n.len(), 4);
        assert!(n.iter().all(|f| f.is_some() && *f != Some(ids[0])));
    }

    #[test]
    fn reflection_keeps_solid_outward() {
        let (mut b, _) = box_body(1000.0, 2000.0, 3000.0);
        b.transform(&Transform::mirror(Point3::ORIGIN, UnitVec3::X));
        b.validate_solid().unwrap();
        assert!(b.volume() > 0.0);
    }

    /// Two boxes as one body (two shells), the second shifted by `dx`.
    fn two_boxes(dx: f64) -> Body {
        let (a, _) = box_body(1000.0, 1000.0, 1000.0);
        let mut p: Vec<Point3> = Vec::new();
        let mut faces: Vec<Vec<usize>> = Vec::new();
        for shift in [0.0, dx] {
            let mut map = std::collections::BTreeMap::new();
            for (v, q) in a.vertices() {
                map.insert(v, p.len());
                p.push(q + Vec3::new(shift, 0.0, 0.0));
            }
            for s in a.face_specs() {
                faces.push(s.outer.iter().map(|v| map[v]).collect());
            }
        }
        Body::from_polygons(&p, &faces).unwrap().0
    }

    #[test]
    fn interpenetrating_shells_are_self_intersecting() {
        let b = two_boxes(500.0);
        assert!(b.is_closed() && b.volume() > 0.0);
        assert!(matches!(
            b.validate_solid(),
            Err(TopoError::SelfIntersecting(_))
        ));
        assert_eq!(
            b.validate().map_err(|e| e.code()),
            Err("ferro.topo.self_intersecting")
        );
    }

    #[test]
    fn separate_shells_are_valid_and_touching_ones_are_not() {
        two_boxes(2000.0).validate_solid().unwrap();
        // Tolerance edge: exactly touching (coplanar faces, no shared vertex)
        // vs a hair apart.
        assert!(matches!(
            two_boxes(1000.0).validate(),
            Err(TopoError::SelfIntersecting(_))
        ));
        two_boxes(1000.0 + 1e-9).validate_solid().unwrap();
    }

    #[test]
    fn face_triangles_follow_the_face_normal() {
        let (b, ids) = box_body(1000.0, 1000.0, 1000.0);
        let t = b.face_triangles(ids[1]).unwrap();
        assert_eq!(t.len(), 2);
        let n = b.face_normal(ids[1]).unwrap().as_vec();
        for tri in t {
            let p: Vec<Point3> = tri.iter().map(|v| b.point(*v).unwrap()).collect();
            assert!((p[1] - p[0]).cross(p[2] - p[0]).dot(n) > 0.0);
        }
    }

    #[test]
    fn body_roundtrips_through_json() {
        let (b, _) = box_body(1000.0, 1000.0, 1000.0);
        let back: Body = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(back, b);
    }
}
