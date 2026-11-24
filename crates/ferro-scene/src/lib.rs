//! Display scene derived from a `Document` snapshot (ADR 0004 §6).
//!
//! The [`DisplayScene`] holds one [`DrawRecord`] per element with geometry:
//! which mesh to draw, where (a translation), its display colour and its
//! bounding box. Meshes live in a shared store ([`SharedMesh`]) so copies of
//! one element share a single mesh (see *Instancing* below).
//!
//! [`DisplayScene::sync`] moves the scene to a new document version. The
//! document is immutable and structurally shared (D-24), so an element whose
//! storage is shared with the previous version ([`Document::same_element`])
//! keeps its mesh; only changed or new elements are re-meshed.
//!
//! View state (hidden elements, section plane, selection, presentation
//! styles) is **not** part of the scene: toggling it never re-meshes. The
//! renderer combines records with view state when it builds its buffers.
//!
//! # Instancing
//!
//! Each mesh is stored in local coordinates, relative to an anchor (the
//! minimum corner of its bounding box); the record carries the anchor as a
//! translation. Two elements share a mesh when their local meshes match:
//! same triangles, same source faces, same visible edges, positions equal
//! within [`INSTANCE_TOL`] and normals within `1e-9`. Lookup is by a hash of
//! the local mesh quantised to [`INSTANCE_TOL`], then verified.
//!
//! This finds copies made by copy / paste and arrays (translation-only).
//! Limits: a rotated or mirrored copy gets its own mesh; two values that
//! straddle a quantisation step are not shared (correct, just not shared);
//! a copy is still re-meshed once when it changes, since the document does
//! not say how it changed. Components (D-71) will provide exact instance
//! transforms and plug in here.
//!
//! Output is deterministic: records are ordered by `ElementId` and mesh ids
//! are assigned in that order, so the same document synced into an empty
//! scene always gives the same scene.

mod color;
mod edges;
mod section;

pub use color::element_color;
pub use edges::visible_edges;
pub use section::section_segments;

use ferro_core::algo::{body_mesh, Mesh};
use ferro_core::model::{Category, Document, ElementId};
use ferro_core::{BoundingBox3, Point3, UnitVec3, Vec3};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

/// Position tolerance for sharing a mesh between translated copies, in mm.
/// Much tighter than `math::tol::VERTEX_TOL`, so a shared mesh is drawn
/// within this distance of each copy's own geometry.
pub const INSTANCE_TOL: f64 = 1e-6;

/// Normal tolerance for sharing a mesh (unitless, per component).
const NORMAL_TOL: f64 = 1e-9;

/// Id of a mesh in the scene's shared store. Local to one [`DisplayScene`];
/// never saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshId(pub u64);

/// A mesh in local coordinates, shared by every element drawn with it.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedMesh {
    /// Triangles in local coordinates (mm, relative to each user's offset).
    pub mesh: Mesh,
    /// Edges to draw (smooth-group interiors removed), local coordinates.
    pub edges: Vec<[Point3; 2]>,
    /// Local bounding box; `None` when the mesh has no positions or edges.
    pub bounds: Option<BoundingBox3>,
    /// Number of draw records using this mesh (1 = not instanced).
    pub users: usize,
    hash: u64,
}

/// What to draw for one element. Derived data, never saved.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawRecord {
    /// The element drawn.
    pub element: ElementId,
    /// Its category (for view filters).
    pub category: Category,
    /// Shared mesh, in local coordinates.
    pub mesh: MeshId,
    /// Translation from local to model coordinates (mm): world = local + offset.
    pub offset: Vec3,
    /// Display colour (sRGB + alpha, 0–1): the element's material, else its
    /// type's core material, else a category colour. Selection and style
    /// tints are view state and are applied by the renderer.
    pub color: [f32; 4],
    /// Bounding box in model coordinates (mm).
    pub bounds: Option<BoundingBox3>,
    /// Scene revision at which this record last changed (mesh, offset or
    /// colour). Lets renderers cache per-element buffers.
    pub revision: u64,
}

/// What one [`DisplayScene::sync`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Elements meshed (new or changed).
    pub meshed: usize,
    /// Elements whose mesh was kept (storage shared with the previous version).
    pub reused: usize,
    /// Records removed (element deleted or lost its geometry).
    pub removed: usize,
    /// Kept records whose colour changed (e.g. their material was edited).
    pub recolored: usize,
    /// Elements whose body could not be triangulated: drawn with edges only.
    pub failed: usize,
}

/// Scene derived from a document snapshot. See the crate docs.
#[derive(Debug, Clone, Default)]
pub struct DisplayScene {
    doc: Option<Document>,
    records: BTreeMap<ElementId, DrawRecord>,
    meshes: BTreeMap<MeshId, SharedMesh>,
    /// Quantised content hash → meshes with that hash (lookup only, never
    /// iterated into output).
    by_hash: HashMap<u64, Vec<MeshId>>,
    next_mesh: u64,
    revision: u64,
}

impl DisplayScene {
    /// Empty scene.
    pub fn new() -> DisplayScene {
        DisplayScene::default()
    }

    /// Scene of `doc`, built from scratch.
    pub fn from_document(doc: &Document) -> DisplayScene {
        let mut s = DisplayScene::new();
        s.sync(doc);
        s
    }

    /// Move the scene to `doc`. Elements whose storage is shared with the
    /// previously synced document keep their mesh; others are re-meshed.
    /// Colours are re-evaluated for all records (they depend on materials
    /// and types, which are other elements). A body that fails to
    /// triangulate gets an empty mesh (edges only) and counts in
    /// [`SyncStats::failed`]; nothing panics.
    pub fn sync(&mut self, doc: &Document) -> SyncStats {
        let mut stats = SyncStats::default();
        if self.doc.as_ref().is_some_and(|p| same_doc(p, doc)) {
            return stats;
        }
        let rev = self.revision + 1;
        let mut changed = false;

        let gone: Vec<ElementId> = self
            .records
            .keys()
            .copied()
            .filter(|id| doc.get(*id).and_then(|e| e.geometry()).is_none())
            .collect();
        for id in gone {
            if let Some(r) = self.records.remove(&id) {
                self.release(r.mesh);
                stats.removed += 1;
                changed = true;
            }
        }

        for e in doc.elements() {
            let Some(body) = e.geometry() else { continue };
            let color = element_color(doc, e);
            let kept = self.doc.as_ref().is_some_and(|p| p.same_element(doc, e.id));
            if kept {
                if let Some(r) = self.records.get_mut(&e.id) {
                    stats.reused += 1;
                    if !same_color(r.color, color) {
                        r.color = color;
                        r.revision = rev;
                        stats.recolored += 1;
                        changed = true;
                    }
                    continue;
                }
            }
            stats.meshed += 1;
            changed = true;
            let mesh = match body_mesh(body) {
                Ok(m) => m,
                Err(_) => {
                    stats.failed += 1;
                    Mesh::default()
                }
            };
            let edges: Vec<[Point3; 2]> = visible_edges(body)
                .into_iter()
                .map(|(a, b)| [a, b])
                .collect();
            let (local, offset) = localize(mesh, edges);
            let bounds = local.bounds.map(|b| BoundingBox3 {
                min: b.min + offset,
                max: b.max + offset,
            });
            let id = self.intern(local);
            if let Some(old) = self.records.get(&e.id).map(|r| r.mesh) {
                self.release(old);
            }
            self.records.insert(
                e.id,
                DrawRecord {
                    element: e.id,
                    category: e.category,
                    mesh: id,
                    offset,
                    color,
                    bounds,
                    revision: rev,
                },
            );
        }
        self.doc = Some(doc.clone());
        if changed {
            self.revision = rev;
        }
        stats
    }

    /// Increases whenever a sync changed any record; unchanged otherwise.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The document last synced, if any.
    pub fn document(&self) -> Option<&Document> {
        self.doc.as_ref()
    }

    /// Draw records in `ElementId` order.
    pub fn records(&self) -> impl Iterator<Item = &DrawRecord> {
        self.records.values()
    }

    /// Number of draw records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// No draw records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Draw record of one element; `None` if it has no geometry.
    pub fn record(&self, id: ElementId) -> Option<&DrawRecord> {
        self.records.get(&id)
    }

    /// A mesh of the shared store.
    pub fn shared_mesh(&self, id: MeshId) -> Option<&SharedMesh> {
        self.meshes.get(&id)
    }

    /// Shared meshes in `MeshId` order.
    pub fn shared_meshes(&self) -> impl Iterator<Item = (MeshId, &SharedMesh)> {
        self.meshes.iter().map(|(k, v)| (*k, v))
    }

    /// The element's record and its local mesh.
    pub fn local_mesh(&self, id: ElementId) -> Option<(&DrawRecord, &SharedMesh)> {
        let r = self.records.get(&id)?;
        Some((r, self.meshes.get(&r.mesh)?))
    }

    /// The element's mesh in model coordinates (allocates a copy).
    pub fn world_mesh(&self, id: ElementId) -> Option<Mesh> {
        let (r, m) = self.local_mesh(id)?;
        let mut out = m.mesh.clone();
        for p in &mut out.positions {
            *p += r.offset;
        }
        Some(out)
    }

    /// Union of all record bounds (mm); `None` for an empty scene.
    pub fn bounds(&self) -> Option<BoundingBox3> {
        self.records
            .values()
            .filter_map(|r| r.bounds)
            .reduce(|a, b| a.union(b))
    }

    /// Section cut: segments (model coordinates) where the plane through
    /// `point` with normal `normal` crosses the triangles of every record
    /// for which `skip` is false. Ordered by element.
    pub fn section_segments(
        &self,
        point: Point3,
        normal: UnitVec3,
        skip: impl Fn(ElementId) -> bool,
    ) -> Vec<(Point3, Point3)> {
        let mut out = Vec::new();
        for r in self.records.values() {
            if skip(r.element) {
                continue;
            }
            let Some(m) = self.meshes.get(&r.mesh) else {
                continue;
            };
            if let Some(b) = r.bounds {
                if !plane_crosses(b, point, normal) {
                    continue;
                }
            }
            // Cut the local mesh with the plane moved into local coordinates.
            for (a, b) in section_segments(&m.mesh, point - r.offset, normal) {
                out.push((a + r.offset, b + r.offset));
            }
        }
        out
    }

    /// Store `m`, or reuse an equal mesh already stored; returns its id with
    /// one more user.
    fn intern(&mut self, mut m: SharedMesh) -> MeshId {
        if let Some(ids) = self.by_hash.get(&m.hash) {
            for id in ids {
                if let Some(s) = self.meshes.get_mut(id) {
                    if same_local_mesh(s, &m) {
                        s.users += 1;
                        return *id;
                    }
                }
            }
        }
        let id = MeshId(self.next_mesh);
        self.next_mesh += 1;
        m.users = 1;
        self.by_hash.entry(m.hash).or_default().push(id);
        self.meshes.insert(id, m);
        id
    }

    /// Drop one user of a mesh; remove it when none is left.
    fn release(&mut self, id: MeshId) {
        let Some(m) = self.meshes.get_mut(&id) else {
            return;
        };
        m.users = m.users.saturating_sub(1);
        if m.users > 0 {
            return;
        }
        let hash = m.hash;
        self.meshes.remove(&id);
        if let Some(v) = self.by_hash.get_mut(&hash) {
            v.retain(|x| *x != id);
            if v.is_empty() {
                self.by_hash.remove(&hash);
            }
        }
    }
}

/// Cheap identity check: same version, same elements, every element shares
/// storage (D-24).
pub fn same_doc(a: &Document, b: &Document) -> bool {
    a.version() == b.version()
        && a.len() == b.len()
        && a.elements().all(|e| a.same_element(b, e.id))
}

fn same_color(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Whether the plane may cross the box (corners on both sides, or touching).
fn plane_crosses(b: BoundingBox3, p: Point3, n: UnitVec3) -> bool {
    let n = n.as_vec();
    let tol = ferro_core::math::tol::VERTEX_TOL;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for x in [b.min.x, b.max.x] {
        for y in [b.min.y, b.max.y] {
            for z in [b.min.z, b.max.z] {
                let d = n.dot(Point3::new(x, y, z) - p);
                lo = lo.min(d);
                hi = hi.max(d);
            }
        }
    }
    lo <= tol && hi >= -tol
}

/// Move a mesh and its edges into local coordinates (relative to the minimum
/// corner of their bounding box). Returns the local mesh and the offset.
fn localize(mut mesh: Mesh, mut edges: Vec<[Point3; 2]>) -> (SharedMesh, Vec3) {
    let all = mesh
        .positions
        .iter()
        .copied()
        .chain(edges.iter().flatten().copied());
    let world = BoundingBox3::from_points(all);
    let anchor = world.map(|b| b.min).unwrap_or(Point3::ORIGIN);
    let offset = anchor - Point3::ORIGIN;
    for p in &mut mesh.positions {
        *p -= offset;
    }
    for e in &mut edges {
        for p in e.iter_mut() {
            *p -= offset;
        }
    }
    let bounds = world.map(|b| BoundingBox3 {
        min: b.min - offset,
        max: b.max - offset,
    });
    let hash = content_hash(&mesh, &edges);
    (
        SharedMesh {
            mesh,
            edges,
            bounds,
            users: 0,
            hash,
        },
        offset,
    )
}

fn quantize(v: f64, step: f64) -> i64 {
    // `as` saturates for huge or NaN values, which only makes a false
    // match possible, and matches are verified.
    (v / step).round() as i64
}

fn content_hash(m: &Mesh, edges: &[[Point3; 2]]) -> u64 {
    let mut h = DefaultHasher::new();
    let point = |p: &Point3, h: &mut DefaultHasher| {
        quantize(p.x, INSTANCE_TOL).hash(h);
        quantize(p.y, INSTANCE_TOL).hash(h);
        quantize(p.z, INSTANCE_TOL).hash(h);
    };
    m.positions.len().hash(&mut h);
    for p in &m.positions {
        point(p, &mut h);
    }
    for n in &m.normals {
        quantize(n.x, NORMAL_TOL).hash(&mut h);
        quantize(n.y, NORMAL_TOL).hash(&mut h);
        quantize(n.z, NORMAL_TOL).hash(&mut h);
    }
    m.indices.hash(&mut h);
    m.triangle_faces.hash(&mut h);
    edges.len().hash(&mut h);
    for e in edges {
        point(&e[0], &mut h);
        point(&e[1], &mut h);
    }
    h.finish()
}

fn same_local_mesh(a: &SharedMesh, b: &SharedMesh) -> bool {
    let near = |p: &Point3, q: &Point3| {
        (p.x - q.x).abs() <= INSTANCE_TOL
            && (p.y - q.y).abs() <= INSTANCE_TOL
            && (p.z - q.z).abs() <= INSTANCE_TOL
    };
    let (x, y) = (&a.mesh, &b.mesh);
    x.indices == y.indices
        && x.triangle_faces == y.triangle_faces
        && x.positions.len() == y.positions.len()
        && x.normals.len() == y.normals.len()
        && a.edges.len() == b.edges.len()
        && x.positions
            .iter()
            .zip(&y.positions)
            .all(|(p, q)| near(p, q))
        && x.normals.iter().zip(&y.normals).all(|(n, o)| {
            (n.x - o.x).abs() <= NORMAL_TOL
                && (n.y - o.y).abs() <= NORMAL_TOL
                && (n.z - o.z).abs() <= NORMAL_TOL
        })
        && a.edges
            .iter()
            .zip(&b.edges)
            .all(|(e, f)| near(&e[0], &f[0]) && near(&e[1], &f[1]))
}

#[cfg(test)]
mod tests;
