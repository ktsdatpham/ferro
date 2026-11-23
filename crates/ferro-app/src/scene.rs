//! Turns the display scene (`ferro-scene`) into GPU vertex lists.
//!
//! Meshes come from `ferro_scene::DisplayScene`, which re-meshes only the
//! elements that changed (D-24). This module keeps one vertex chunk per
//! element, rebuilt only when its draw record, its selection state or a
//! presentation style changes; the flat lists are the visible chunks
//! concatenated. Toggling hidden elements or the section plane never
//! re-meshes and never rebuilds chunks.

use crate::render::Vertex;
use ferro_core::algo::Mesh;
use ferro_core::model::{Document, ElementId, PhaseStatus};
use ferro_core::topo::FaceId;
use ferro_core::{BoundingBox3, Point3, UnitVec3};
use ferro_scene::{DisplayScene, DrawRecord, SharedMesh};
use std::collections::{HashMap, HashSet};

pub use ferro_scene::same_doc;

pub const EDGE: [f32; 4] = [0.08, 0.09, 0.11, 1.0];
pub const SELECT: [f32; 4] = [0.18, 0.45, 0.95, 1.0];
/// Section cut lines.
pub const CUT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Demolished / temporary faces (translucent) and their dashed edges (M5-23).
pub const DEMOLISHED: [f32; 4] = [0.86, 0.32, 0.26, 0.35];
pub const DEMOLISHED_EDGE: [f32; 4] = [0.70, 0.12, 0.08, 1.0];
/// Existing elements: faces blended towards this grey, lighter edges.
pub const EXISTING_GREY: [f32; 4] = [0.72, 0.72, 0.72, 1.0];
pub const EXISTING_EDGE: [f32; 4] = [0.45, 0.46, 0.48, 1.0];
/// Dash and gap of demolished edges, mm.
const DASH_MM: f64 = 120.0;
const GAP_MM: f64 = 80.0;

/// What every chunk depends on besides its record: origin bits, X-ray,
/// wireframe, hidden line.
type StyleKey = (u64, bool, bool, bool);

/// Vertices of one element, for one record revision and selection state.
struct Chunk {
    revision: u64,
    selected: bool,
    phase: Option<PhaseStatus>,
    tris: Vec<Vertex>,
    lines: Vec<Vertex>,
}

/// What the flat lists were built for.
#[derive(PartialEq)]
struct ListKey {
    revision: u64,
    style: StyleKey,
    selection: Vec<ElementId>,
    hidden: Vec<ElementId>,
    phase: Vec<(ElementId, PhaseStatus)>,
}

/// What the cut lines were built for: scene revision, origin, hidden, plane.
type CutKey = (u64, u64, Vec<ElementId>, Vec<u64>);

#[derive(Default)]
pub struct SceneCache {
    /// Derived scene: meshes, colours, instances.
    pub scene: DisplayScene,
    chunks: HashMap<ElementId, Chunk>,
    style: Option<StyleKey>,
    list_key: Option<ListKey>,
    cut_key: Option<CutKey>,
    /// Bumped whenever `tris` / `lines` change (the renderer re-uploads).
    pub version: u64,
    pub tris: Vec<Vertex>,
    pub lines: Vec<Vertex>,
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
    /// Chunks rebuilt by the last `update` (for tests and diagnostics).
    pub rebuilt: usize,
    /// Phase status of elements drawn differently in the view's phase
    /// (existing greyed, demolished / temporary red and dashed). View state.
    pub phase: HashMap<ElementId, PhaseStatus>,
}

fn sorted(set: &HashSet<ElementId>) -> Vec<ElementId> {
    let mut v: Vec<ElementId> = set.iter().copied().collect();
    v.sort();
    v
}

impl SceneCache {
    /// Bring the vertex lists up to date with `doc`, the selection and the
    /// view flags. Only changed elements are re-meshed (by the scene) and
    /// only their chunks, plus chunks whose selection changed, are rebuilt.
    pub fn update(&mut self, doc: &Document, origin: Point3, selection: &HashSet<ElementId>) {
        self.rebuilt = 0;
        self.scene.sync(doc);
        let style: StyleKey = (
            origin.x.to_bits()
                ^ origin.y.to_bits().rotate_left(21)
                ^ origin.z.to_bits().rotate_left(42),
            self.xray,
            self.wireframe,
            self.hidden_line,
        );
        let hidden = sorted(&self.hidden);
        self.update_cut(origin, &style, &hidden);

        let key = ListKey {
            revision: self.scene.revision(),
            style,
            selection: sorted(selection),
            hidden,
            phase: {
                let mut v: Vec<(ElementId, PhaseStatus)> =
                    self.phase.iter().map(|(k, v)| (*k, *v)).collect();
                v.sort();
                v
            },
        };
        if self.list_key.as_ref() == Some(&key) {
            return;
        }
        if self.style != Some(style) {
            self.chunks.clear();
            self.style = Some(style);
        }
        let scene = &self.scene;
        self.chunks.retain(|id, _| scene.record(*id).is_some());
        self.tris.clear();
        self.lines.clear();
        let mut translucent = Vec::new();
        for r in scene.records() {
            let selected = selection.contains(&r.element);
            let phase = self.phase.get(&r.element).copied();
            let fresh = self.chunks.get(&r.element).is_some_and(|c| {
                c.revision == r.revision && c.selected == selected && c.phase == phase
            });
            if !fresh {
                if let Some(m) = scene.shared_mesh(r.mesh) {
                    self.chunks.insert(
                        r.element,
                        chunk(
                            r,
                            m,
                            selected,
                            origin,
                            (self.xray, self.wireframe, self.hidden_line),
                            phase,
                        ),
                    );
                    self.rebuilt += 1;
                }
            }
            if self.hidden.contains(&r.element) {
                continue;
            }
            // Translucent (demolished) faces go last so opaque faces behind
            // them are already drawn when they blend.
            if matches!(
                phase,
                Some(PhaseStatus::Demolished | PhaseStatus::Temporary)
            ) {
                translucent.push(r.element);
                continue;
            }
            if let Some(c) = self.chunks.get(&r.element) {
                self.tris.extend_from_slice(&c.tris);
                self.lines.extend_from_slice(&c.lines);
            }
        }
        for id in translucent {
            if let Some(c) = self.chunks.get(&id) {
                self.tris.extend_from_slice(&c.tris);
                self.lines.extend_from_slice(&c.lines);
            }
        }
        self.list_key = Some(key);
        self.version += 1;
    }

    fn update_cut(&mut self, origin: Point3, style: &StyleKey, hidden: &[ElementId]) {
        let plane: Vec<u64> = self
            .section
            .map(|(p, n)| {
                let n = n.as_vec();
                [p.x, p.y, p.z, n.x, n.y, n.z]
                    .into_iter()
                    .map(f64::to_bits)
                    .collect()
            })
            .unwrap_or_default();
        let key = (self.scene.revision(), style.0, hidden.to_vec(), plane);
        if self.cut_key.as_ref() == Some(&key) {
            return;
        }
        self.cut.clear();
        if let Some((p, n)) = self.section {
            let rel = |q: Point3| {
                let v = q - origin;
                [v.x as f32, v.y as f32, v.z as f32]
            };
            for (a, b) in self
                .scene
                .section_segments(p, n, |e| self.hidden.contains(&e))
            {
                self.cut.push(line_vertex(rel(a), CUT));
                self.cut.push(line_vertex(rel(b), CUT));
            }
        }
        self.cut_key = Some(key);
    }

    /// The element's mesh in model coordinates (a copy; instanced meshes
    /// are stored once in local coordinates).
    pub fn mesh(&self, id: ElementId) -> Option<Mesh> {
        self.scene.world_mesh(id)
    }
}

/// Vertices of one element: faces (unless wireframe) and visible edges,
/// camera-relative to `origin` (D-05).
fn chunk(
    r: &DrawRecord,
    m: &SharedMesh,
    selected: bool,
    origin: Point3,
    (xray, wireframe, hidden_line): (bool, bool, bool),
    phase: Option<PhaseStatus>,
) -> Chunk {
    // Local → camera-relative: add the record's offset, subtract the origin.
    let shift = r.offset - (origin - Point3::ORIGIN);
    let rel = |p: Point3| {
        let v = (p - Point3::ORIGIN) + shift;
        [v.x as f32, v.y as f32, v.z as f32]
    };
    let demolished = matches!(
        phase,
        Some(PhaseStatus::Demolished | PhaseStatus::Temporary)
    );
    let c = match phase {
        _ if demolished => DEMOLISHED,
        Some(PhaseStatus::Existing) => {
            let g = EXISTING_GREY;
            let k = 0.25;
            [
                r.color[0] * k + g[0] * (1.0 - k),
                r.color[1] * k + g[1] * (1.0 - k),
                r.color[2] * k + g[2] * (1.0 - k),
                r.color[3],
            ]
        }
        _ => r.color,
    };
    let color = if selected {
        [
            c[0] * 0.7 + 0.3 * SELECT[0],
            c[1] * 0.7 + 0.3 * SELECT[1],
            c[2] * 0.7 + 0.3 * SELECT[2],
            c[3],
        ]
    } else {
        c
    };
    let color = if hidden_line {
        [1.0, 1.0, 1.0, 1.0]
    } else {
        color
    };
    let color = if xray {
        [color[0], color[1], color[2], 0.3]
    } else {
        color
    };
    let mut tris = Vec::new();
    if !wireframe {
        let mesh = &m.mesh;
        tris.reserve(mesh.indices.len() * 3);
        for t in &mesh.indices {
            for &i in t {
                let (Some(p), Some(n)) =
                    (mesh.positions.get(i as usize), mesh.normals.get(i as usize))
                else {
                    continue;
                };
                tris.push(Vertex {
                    pos: rel(*p),
                    normal: [n.x as f32, n.y as f32, n.z as f32],
                    color,
                });
            }
        }
    }
    let edge = match phase {
        _ if selected => SELECT,
        _ if demolished => DEMOLISHED_EDGE,
        Some(PhaseStatus::Existing) => EXISTING_EDGE,
        _ => EDGE,
    };
    let mut lines = Vec::with_capacity(m.edges.len() * 2);
    for [a, b] in &m.edges {
        if demolished {
            for (p, q) in dashes(*a, *b) {
                lines.push(line_vertex(rel(p), edge));
                lines.push(line_vertex(rel(q), edge));
            }
        } else {
            lines.push(line_vertex(rel(*a), edge));
            lines.push(line_vertex(rel(*b), edge));
        }
    }
    Chunk {
        revision: r.revision,
        selected,
        phase,
        tris,
        lines,
    }
}

/// Dashes along the edge `a`–`b` (model mm): about [`DASH_MM`] on and
/// [`GAP_MM`] off, scaled so the edge starts and ends with a dash; an edge
/// shorter than one dash and gap stays whole.
/// At most 200 dashes per edge.
fn dashes(a: Point3, b: Point3) -> Vec<(Point3, Point3)> {
    let len = a.distance_to(b);
    let period = DASH_MM + GAP_MM;
    if len <= DASH_MM + GAP_MM || !len.is_finite() {
        return vec![(a, b)];
    }
    // n dashes and n - 1 gaps, scaled to fit exactly: both ends are dashed.
    let n = (((len + GAP_MM) / period).round() as usize).clamp(1, 200);
    let unit = len / (n as f64 * DASH_MM + (n - 1) as f64 * GAP_MM);
    let (on, step) = (DASH_MM * unit, period * unit);
    let d = (b - a) * (1.0 / len);
    (0..n)
        .map(|i| {
            let s = step * i as f64;
            (a + d * s, a + d * (s + on))
        })
        .collect()
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
mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    fn boxes(n: usize) -> (Session, Vec<ElementId>) {
        let mut s = Session::new(Document::new("t"), Box::new(SequentialIds::new(1)));
        let mut ids = Vec::new();
        for i in 0..n {
            let (x, y) = ((i % 50) as f64 * 3000.0, (i / 50) as f64 * 3000.0);
            let args = json!({"origin":{"x":x,"y":y,"z":0},"width":1000.0 + i as f64,"depth":1000,"height":3000});
            let out = s
                .execute(
                    AnyCommand::from_tool("create_box", &args).unwrap(),
                    &ExecContext::user(),
                )
                .unwrap()
                .output;
            ids.push(serde_json::from_value(out["element"].clone()).unwrap());
        }
        (s, ids)
    }

    #[test]
    fn view_flags_and_selection_rebuild_only_what_they_touch() {
        let (s, ids) = boxes(3);
        let mut c = SceneCache::default();
        let none = HashSet::new();
        c.update(s.doc(), Point3::ORIGIN, &none);
        assert_eq!(c.rebuilt, 3);
        assert_eq!(c.tris.len(), 3 * 36);
        assert_eq!(c.lines.len(), 3 * 24);
        let v = c.version;

        // Nothing changed: no new version.
        c.update(s.doc(), Point3::ORIGIN, &none);
        assert_eq!((c.version, c.rebuilt), (v, 0));

        // Selecting one element rebuilds its chunk only.
        let sel: HashSet<ElementId> = [ids[1]].into_iter().collect();
        c.update(s.doc(), Point3::ORIGIN, &sel);
        assert_eq!(c.rebuilt, 1);
        assert!(c.version > v);

        // Hiding one element drops its vertices without rebuilding anything.
        c.hidden.insert(ids[0]);
        c.update(s.doc(), Point3::ORIGIN, &sel);
        assert_eq!(c.rebuilt, 0);
        assert_eq!(c.tris.len(), 2 * 36);

        // A section plane makes cut lines and leaves the lists alone.
        let v = c.version;
        c.section = Some((Point3::new(0.0, 0.0, 1500.0), UnitVec3::Z));
        c.update(s.doc(), Point3::ORIGIN, &sel);
        assert_eq!(c.version, v);
        assert_eq!(c.cut.len(), 2 * 8 * 2);

        // A style change rebuilds every chunk, but re-meshes nothing.
        let rev = c.scene.revision();
        c.wireframe = true;
        c.update(s.doc(), Point3::ORIGIN, &sel);
        assert_eq!(c.rebuilt, 3);
        assert!(c.tris.is_empty());
        assert_eq!(c.scene.revision(), rev);
    }

    #[test]
    fn phase_looks_grey_existing_and_dash_demolished_last() {
        let (s, ids) = boxes(2);
        let mut c = SceneCache::default();
        let none = HashSet::new();
        c.update(s.doc(), Point3::ORIGIN, &none);
        let plain_lines = c.lines.len();
        c.phase.insert(ids[0], PhaseStatus::Demolished);
        c.phase.insert(ids[1], PhaseStatus::Existing);
        c.update(s.doc(), Point3::ORIGIN, &none);
        assert_eq!(c.rebuilt, 2, "both looks changed");
        // Demolished faces are translucent and drawn after the opaque ones.
        assert_eq!(c.tris.len(), 2 * 36);
        assert!(c.tris[..36].iter().all(|v| v.color[3] > 0.99));
        assert!(c.tris[36..].iter().all(|v| v.color == DEMOLISHED));
        // Its edges are dashed: more, shorter segments in the edge colour.
        assert!(c.lines.len() > plain_lines);
        assert!(c.lines.iter().any(|v| v.color == DEMOLISHED_EDGE));
        assert!(c.lines.iter().any(|v| v.color == EXISTING_EDGE));
        // Same looks again: nothing rebuilt.
        let v = c.version;
        c.update(s.doc(), Point3::ORIGIN, &none);
        assert_eq!((c.version, c.rebuilt), (v, 0));
    }

    #[test]
    fn dashes_cover_the_edge_from_end_to_end() {
        let a = Point3::ORIGIN;
        let b = Point3::new(1000.0, 0.0, 0.0);
        let d = dashes(a, b);
        assert_eq!(d.len(), 5);
        assert!(d[0].0.distance_to(a) < 1e-9);
        assert!(d[4].1.distance_to(b) < 1e-9);
        assert!(d.iter().all(|(p, q)| p.distance_to(*q) < DASH_MM * 1.2));
        // Short and zero-length edges stay whole.
        assert_eq!(dashes(a, Point3::new(50.0, 0.0, 0.0)).len(), 1);
        assert_eq!(dashes(a, a), vec![(a, a)]);
        // Very long edges are capped.
        assert_eq!(dashes(a, Point3::new(1e9, 0.0, 0.0)).len(), 200);
    }

    #[test]
    fn empty_document_gives_empty_lists() {
        let mut c = SceneCache {
            section: Some((Point3::ORIGIN, UnitVec3::Z)),
            ..Default::default()
        };
        c.update(&Document::new("t"), Point3::ORIGIN, &HashSet::new());
        assert!(c.tris.is_empty() && c.lines.is_empty() && c.cut.is_empty());
        assert!(c.scene.is_empty());
    }

    /// Run with `cargo test -p ferro-app --release scene -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn benchmark_vertex_lists_two_thousand_boxes() {
        use std::time::Instant;
        let (s, ids) = boxes(2000);
        let mut c = SceneCache::default();
        let none = HashSet::new();
        let t = Instant::now();
        c.update(s.doc(), Point3::ORIGIN, &none);
        let full = t.elapsed();
        let sel: HashSet<ElementId> = [ids[7]].into_iter().collect();
        let t = Instant::now();
        c.update(s.doc(), Point3::ORIGIN, &sel);
        let select = t.elapsed();
        c.hidden.insert(ids[9]);
        let t = Instant::now();
        c.update(s.doc(), Point3::ORIGIN, &sel);
        let hide = t.elapsed();
        println!("ferro-app vertex lists, 2,000 boxes: full {full:?}, selection change {select:?}, hide one {hide:?}");
    }
}
