//! A kernel's planar result turned into a Ferro [`Body`] in operand `a`'s id
//! space, merged at Ferro's tolerances (D-06) and with lineage (D-25).
//! Shared by every [`Kernel`](super::Kernel) implementation, so the merge and
//! id rules do not depend on the kernel library.

use super::{FaceSource, KernelError, KernelResult};
use crate::geom::{polygon, predicates, Plane, Surface};
use crate::math::{tol, Point2, Point3, UnitVec3, Vec3};
use crate::topo::{Body, FaceId, FaceSpec, Lineage, TopoError, Vertex, VertexId};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One planar face of a kernel result, before merging.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelFace {
    /// Outer loop first, then holes (mm). The outer loop runs
    /// counter-clockwise about the outward normal, holes clockwise.
    pub loops: Vec<Vec<Point3>>,
    /// Input faces it came from (any order; empty for a new face).
    pub sources: Vec<FaceSource>,
}

/// A face during merging: loops as indices into the merged point list.
#[derive(Debug, Clone)]
struct Work {
    loops: Vec<Vec<usize>>,
    sources: BTreeSet<FaceSource>,
}

/// Merged points: the first `seeds` are `a`'s vertices, so result points
/// within `VERTEX_TOL` of one take its exact position and id.
struct Points {
    pts: Vec<Point3>,
    grid: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl Points {
    fn cell(p: Point3) -> (i64, i64, i64) {
        let c = |x: f64| (x / tol::VERTEX_TOL).floor() as i64;
        (c(p.x), c(p.y), c(p.z))
    }

    fn push(&mut self, p: Point3) -> usize {
        let i = self.pts.len();
        self.pts.push(p);
        self.grid.entry(Self::cell(p)).or_default().push(i);
        i
    }

    /// The nearest point within `VERTEX_TOL` (lowest index on a tie), or a
    /// new one.
    fn find_or_push(&mut self, p: Point3) -> usize {
        let (cx, cy, cz) = Self::cell(p);
        let mut best: Option<(f64, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = self.grid.get(&(cx + dx, cy + dy, cz + dz)) else {
                        continue;
                    };
                    for &i in list {
                        let d = self.pts[i].distance_to(p);
                        if d <= tol::VERTEX_TOL
                            && best.is_none_or(|(bd, bi)| d < bd || (d <= bd && i < bi))
                        {
                            best = Some((d, i));
                        }
                    }
                }
            }
        }
        match best {
            Some((_, i)) => i,
            None => self.push(p),
        }
    }
}

fn invalid(message: impl Into<String>) -> KernelError {
    KernelError::new(KernelError::INVALID_RESULT, message)
}

fn from_topo(e: TopoError) -> KernelError {
    match e {
        TopoError::NonManifold => KernelError::new(
            KernelError::NON_MANIFOLD_RESULT,
            "the result is not manifold: solids touch only along an edge or at a vertex",
        ),
        e => invalid(format!("the result is not a valid solid: {e} ({})", e.code())),
    }
}

/// Remove repeated consecutive vertices and back-and-forth spikes.
fn clean_loop(mut l: Vec<usize>) -> Vec<usize> {
    loop {
        let n = l.len();
        if n < 3 {
            return l;
        }
        let mut cut: Option<Vec<usize>> = None;
        for i in 0..n {
            let next = (i + 1) % n;
            if l[i] == l[next] {
                cut = Some(vec![i]);
                break;
            }
            if l[(i + n - 1) % n] == l[next] {
                cut = Some(vec![i, next]);
                break;
            }
        }
        match cut {
            None => return l,
            Some(mut idx) => {
                idx.sort_unstable();
                for i in idx.into_iter().rev() {
                    l.remove(i);
                }
            }
        }
    }
}

/// Newell area vector of all loops (holes subtract), mm².
fn area_vector(pts: &[Point3], loops: &[Vec<usize>]) -> Vec3 {
    let mut a = Vec3::ZERO;
    for l in loops {
        let Some(&o) = l.first() else { continue };
        let o = pts[o];
        for i in 1..l.len().saturating_sub(1) {
            a += (pts[l[i]] - o).cross(pts[l[i + 1]] - o) * 0.5;
        }
    }
    a
}

/// The plane of a face: through the centroid of its outer loop, normal by
/// Newell's method. `None` for a degenerate face.
fn face_plane(pts: &[Point3], loops: &[Vec<usize>]) -> Option<Plane> {
    let outer: Vec<Point3> = loops.first()?.iter().map(|&i| pts[i]).collect();
    let n = polygon::polygon_normal(&outer)?;
    let c = Point3::centroid(&outer)?;
    Some(Plane::from_origin_normal(c, n))
}

fn on_plane(pl: &Plane, pts: &[Point3], loops: &[Vec<usize>]) -> bool {
    loops.iter().flatten().all(|&i| pl.contains(pts[i]))
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.0[hi] = lo;
        }
    }
}

/// Angle of `v` in `plane` (radians, (-π, π]).
fn angle_in(plane: &Plane, v: Vec3) -> f64 {
    let x = v.dot(plane.x_axis().as_vec());
    let y = v.dot(plane.y_axis().as_vec());
    y.atan2(x)
}

/// Merge faces of one plane: drop the edges they share and trace what is
/// left into outer loops (counter-clockwise about `normal`) and holes.
fn merge_group(
    pts: &[Point3],
    members: &[&Work],
    normal: UnitVec3,
) -> Result<Vec<Work>, KernelError> {
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for m in members {
        for l in &m.loops {
            for i in 0..l.len() {
                edges.push((l[i], l[(i + 1) % l.len()]));
            }
        }
    }
    let all: BTreeSet<(usize, usize)> = edges.iter().copied().collect();
    let left: Vec<(usize, usize)> = edges
        .into_iter()
        .filter(|(u, v)| !all.contains(&(*v, *u)))
        .collect();
    let mut outgoing: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (u, v) in &left {
        outgoing.entry(*u).or_default().push(*v);
    }
    let origin = pts[left.first().map(|e| e.0).unwrap_or(0)];
    let plane = Plane::from_origin_normal(origin, normal);
    let mut used: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut loops: Vec<Vec<usize>> = Vec::new();
    for &(u0, v0) in &left {
        if used.contains(&(u0, v0)) {
            continue;
        }
        let mut lp = vec![u0];
        let (mut u, mut v) = (u0, v0);
        used.insert((u, v));
        let mut guard = 0;
        while v != u0 {
            guard += 1;
            if guard > left.len() {
                return Err(invalid("coplanar face merge did not close a loop"));
            }
            lp.push(v);
            let back = pts[u] - pts[v];
            let cands: Vec<usize> = outgoing
                .get(&v)
                .map(|c| {
                    c.iter()
                        .copied()
                        .filter(|w| !used.contains(&(v, *w)))
                        .collect()
                })
                .unwrap_or_default();
            // DCEL rule: the first outgoing edge clockwise from the way back.
            let a0 = angle_in(&plane, back);
            let next = cands
                .iter()
                .copied()
                .map(|w| {
                    let mut d = a0 - angle_in(&plane, pts[w] - pts[v]);
                    while d <= 1e-12 {
                        d += std::f64::consts::TAU;
                    }
                    (d, w)
                })
                .min_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)))
                .map(|(_, w)| w)
                .ok_or_else(|| invalid("coplanar face merge found an open boundary"))?;
            used.insert((v, next));
            u = v;
            v = next;
        }
        loops.push(lp);
    }
    let local = |l: &Vec<usize>| -> Vec<Point2> { l.iter().map(|&i| plane.to_local(pts[i])).collect() };
    let mut outers: Vec<Vec<usize>> = Vec::new();
    let mut holes: Vec<Vec<usize>> = Vec::new();
    for l in loops {
        let l = clean_loop(l);
        if l.len() < 3 {
            continue;
        }
        if predicates::signed_area2(&local(&l)) > 0.0 {
            outers.push(l);
        } else {
            holes.push(l);
        }
    }
    if outers.is_empty() {
        return Err(invalid("coplanar face merge left no outer loop"));
    }
    let sources: BTreeSet<FaceSource> = members
        .iter()
        .flat_map(|m| m.sources.iter().copied())
        .collect();
    let mut out: Vec<Work> = outers
        .into_iter()
        .map(|o| Work {
            loops: vec![o],
            sources: sources.clone(),
        })
        .collect();
    for h in holes {
        let p = plane.to_local(pts[h[0]]);
        let host = out
            .iter()
            .position(|w| predicates::point_in_polygon(p, &local(&w.loops[0])))
            .ok_or_else(|| invalid("coplanar face merge left a hole outside every face"))?;
        out[host].loops.push(h);
    }
    Ok(out)
}

/// Drop vertices that sit on a straight edge between exactly two
/// neighbours (within `VERTEX_TOL` of the line through them).
fn drop_collinear(pts: &[Point3], faces: &mut [Work]) {
    loop {
        let mut nb: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for f in faces.iter() {
            for l in &f.loops {
                for i in 0..l.len() {
                    let (a, b) = (l[i], l[(i + 1) % l.len()]);
                    nb.entry(a).or_default().insert(b);
                    nb.entry(b).or_default().insert(a);
                }
            }
        }
        let mut gone: BTreeSet<usize> = BTreeSet::new();
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        for (&v, ns) in &nb {
            if ns.len() != 2 || touched.contains(&v) {
                continue;
            }
            let mut it = ns.iter();
            let (Some(&a), Some(&b)) = (it.next(), it.next()) else {
                continue;
            };
            if touched.contains(&a) || touched.contains(&b) {
                continue;
            }
            let ab = pts[b] - pts[a];
            let len = ab.length();
            if len <= tol::VERTEX_TOL {
                continue;
            }
            let av = pts[v] - pts[a];
            let t = av.dot(ab) / (len * len);
            if t <= 0.0 || t >= 1.0 || ab.cross(av).length() / len > tol::VERTEX_TOL {
                continue;
            }
            gone.insert(v);
            touched.extend([v, a, b]);
        }
        if gone.is_empty() {
            return;
        }
        for f in faces.iter_mut() {
            for l in &mut f.loops {
                l.retain(|i| !gone.contains(i));
            }
        }
    }
}

/// Turn a kernel's planar faces into a [`KernelResult`] in `a`'s id space.
///
/// * Points within `VERTEX_TOL` (0.01 mm, D-06) become one vertex; a point
///   within it of a vertex of `a` takes that vertex's id and exact position.
/// * Faces left without area are dropped; neighbouring faces in one plane
///   (every vertex of each within `VERTEX_TOL` of the other's plane, same
///   side) merge into one face carrying both faces' sources; vertices left
///   on a straight edge between two faces are removed.
/// * Ids: for each face of `a` (in id order), the largest unclaimed result
///   face that came from it keeps its id; other faces get new ids. Vertex
///   and edge ids of `a` are kept where the same points survive.
///
/// Deterministic: the same `a` and `faces` give the same result.
///
/// # Errors
/// `ferro.kernel.empty_result` when no face is left;
/// `ferro.kernel.non_manifold_result` when an edge is used by more than two
/// faces; `ferro.kernel.invalid_result` for non-finite points, loops the
/// merge cannot close, or a body that fails [`Body::validate_solid`].
pub fn assemble_kernel_result(a: &Body, faces: &[KernelFace]) -> Result<KernelResult, KernelError> {
    let mut points = Points {
        pts: Vec::new(),
        grid: HashMap::new(),
    };
    let seeds: Vec<VertexId> = a.vertices().map(|(id, _)| id).collect();
    for (_, p) in a.vertices() {
        points.push(p);
    }
    let mut work: Vec<Work> = Vec::new();
    for f in faces {
        let mut loops = Vec::new();
        for (k, l) in f.loops.iter().enumerate() {
            if l.iter().any(|p| !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite())) {
                return Err(invalid("the kernel returned a non-finite point"));
            }
            let idx = clean_loop(l.iter().map(|p| points.find_or_push(*p)).collect());
            if idx.len() >= 3 {
                loops.push(idx);
            } else if k == 0 {
                break; // the outer loop collapsed: the face is a sliver
            }
        }
        if loops.is_empty() {
            continue;
        }
        if area_vector(&points.pts, &loops).length() < tol::VERTEX_TOL * tol::VERTEX_TOL {
            continue;
        }
        work.push(Work {
            loops,
            sources: f.sources.iter().copied().collect(),
        });
    }
    let pts = points.pts;

    // Coplanar neighbours.
    let planes: Vec<Option<Plane>> = work.iter().map(|w| face_plane(&pts, &w.loops)).collect();
    let mut owner: HashMap<(usize, usize), usize> = HashMap::new();
    for (fi, w) in work.iter().enumerate() {
        for l in &w.loops {
            for i in 0..l.len() {
                owner.insert((l[i], l[(i + 1) % l.len()]), fi);
            }
        }
    }
    let mut uf = UnionFind((0..work.len()).collect());
    for (fi, w) in work.iter().enumerate() {
        for l in &w.loops {
            for i in 0..l.len() {
                let Some(&gi) = owner.get(&(l[(i + 1) % l.len()], l[i])) else {
                    continue;
                };
                if gi <= fi {
                    continue;
                }
                let (Some(pf), Some(pg)) = (&planes[fi], &planes[gi]) else {
                    continue;
                };
                if pf.normal().dot(pg.normal()) > 0.0
                    && on_plane(pf, &pts, &work[gi].loops)
                    && on_plane(pg, &pts, &work[fi].loops)
                {
                    uf.union(fi, gi);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..work.len() {
        let r = uf.find(i);
        groups.entry(r).or_default().push(i);
    }
    let mut merged: Vec<Work> = Vec::new();
    for members in groups.values() {
        if let [one] = members.as_slice() {
            merged.push(work[*one].clone());
            continue;
        }
        let mut sum = Vec3::ZERO;
        for &m in members {
            sum += area_vector(&pts, &work[m].loops);
        }
        let normal = sum
            .normalize()
            .ok_or_else(|| invalid("coplanar faces cancel out"))?;
        let ms: Vec<&Work> = members.iter().map(|&m| &work[m]).collect();
        merged.extend(merge_group(&pts, &ms, normal)?);
    }
    drop_collinear(&pts, &mut merged);
    merged.retain(|w| w.loops.first().is_some_and(|l| l.len() >= 3));
    for w in &mut merged {
        w.loops.retain(|l| l.len() >= 3);
    }
    if merged.is_empty() {
        return Err(KernelError::new(
            KernelError::EMPTY_RESULT,
            "the result is empty",
        ));
    }

    // Ids: the largest unclaimed piece of each face of `a` keeps its id.
    let areas: Vec<f64> = merged
        .iter()
        .map(|w| area_vector(&pts, &w.loops).length())
        .collect();
    let mut keep: Vec<Option<FaceId>> = vec![None; merged.len()];
    let mut lineage = Lineage::default();
    for fa in a.face_ids() {
        let best = merged
            .iter()
            .enumerate()
            .filter(|(i, w)| keep[*i].is_none() && w.sources.contains(&FaceSource::FromA(fa)))
            .max_by(|x, y| areas[x.0].total_cmp(&areas[y.0]).then(y.0.cmp(&x.0)))
            .map(|(i, _)| i);
        match best {
            Some(i) => {
                keep[i] = Some(fa);
                lineage.kept.push(fa);
            }
            None => lineage.deleted.push(fa),
        }
    }

    let mut body = a.clone();
    let mut vid: BTreeMap<usize, VertexId> = BTreeMap::new();
    let mut vmap: BTreeMap<VertexId, Vertex> = BTreeMap::new();
    let mut specs = Vec::with_capacity(merged.len());
    for (i, w) in merged.iter().enumerate() {
        let mut ids: Vec<Vec<VertexId>> = Vec::with_capacity(w.loops.len());
        for l in &w.loops {
            let mut out = Vec::with_capacity(l.len());
            for &p in l {
                let id = match vid.get(&p) {
                    Some(id) => *id,
                    None => {
                        let id = match seeds.get(p) {
                            Some(id) => *id,
                            None => body.add_vertex(pts[p]),
                        };
                        vid.insert(p, id);
                        vmap.insert(id, Vertex { point: pts[p] });
                        id
                    }
                };
                out.push(id);
            }
            ids.push(out);
        }
        let plane = face_plane(&pts, &w.loops).ok_or_else(|| invalid("a result face is degenerate"))?;
        let smooth = {
            let groups: BTreeSet<Option<u32>> = w
                .sources
                .iter()
                .map(|s| match s {
                    FaceSource::FromA(f) => a.face(*f).ok().and_then(|x| x.smooth_group),
                    FaceSource::FromB(_) => None,
                })
                .collect();
            match groups.into_iter().collect::<Vec<_>>().as_slice() {
                [Some(g)] => Some(*g),
                _ => None,
            }
        };
        let mut it = ids.into_iter();
        let outer = it.next().unwrap_or_default();
        specs.push(FaceSpec {
            id: keep[i],
            outer,
            holes: it.collect(),
            surface: Some(Surface::Plane(plane)),
            smooth_group: smooth,
        });
    }
    let fids = body.rebuild(vmap, specs).map_err(from_topo)?;
    let mut out_faces = BTreeMap::new();
    for (i, fid) in fids.iter().enumerate() {
        let w = &merged[i];
        out_faces.insert(*fid, w.sources.iter().copied().collect::<Vec<_>>());
        if keep[i].is_none() {
            let from = w.sources.iter().find_map(|s| match s {
                FaceSource::FromA(f) => Some(*f),
                FaceSource::FromB(_) => None,
            });
            lineage.created.push((*fid, from));
        }
    }
    body.validate_solid().map_err(from_topo)?;
    Ok(KernelResult {
        body,
        faces: out_faces,
        lineage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Transform;
    use crate::topo::tests_support::box_body;

    /// Faces of `b` as kernel faces, `src` naming their source, skipping
    /// `skip`.
    fn faces_of(b: &Body, src: fn(FaceId) -> FaceSource, skip: &[FaceId]) -> Vec<KernelFace> {
        b.face_ids()
            .filter(|f| !skip.contains(f))
            .map(|f| {
                let face = b.face(f).unwrap();
                let loops = std::iter::once(&face.outer)
                    .chain(&face.holes)
                    .map(|l| b.loop_points(*l))
                    .collect();
                KernelFace {
                    loops,
                    sources: vec![src(f)],
                }
            })
            .collect()
    }

    /// Two 1 m cubes side by side, as a kernel would give their union before
    /// merging: A's east face and B's west face gone, the rest split.
    fn union_faces(dx: f64) -> (Body, Body, Vec<KernelFace>) {
        let (a, fa) = box_body(1000.0, 1000.0, 1000.0);
        let (mut b, fb) = box_body(1000.0, 1000.0, 1000.0);
        b.transform(&Transform::translation(Vec3::new(1000.0 + dx, 0.0, 0.0)));
        let mut faces = faces_of(&a, FaceSource::FromA, &[fa[3]]);
        faces.extend(faces_of(&b, FaceSource::FromB, &[fb[5]]));
        (a, b, faces)
    }

    #[test]
    fn assemble_merges_coplanar_faces_and_keeps_ids_of_a() {
        let (a, _, faces) = union_faces(0.0);
        let r = assemble_kernel_result(&a, &faces).unwrap();
        r.body.validate_solid().unwrap();
        assert_eq!(r.body.face_count(), 6);
        assert_eq!(r.body.vertex_count(), 8);
        assert!((r.body.volume() - 2e9).abs() < 1e-3);
        // A's five surviving faces keep their ids; its east face is gone.
        let (_, fa) = box_body(1000.0, 1000.0, 1000.0);
        assert_eq!(r.lineage.deleted, vec![fa[3]]);
        assert_eq!(r.lineage.kept.len(), 5);
        for f in &r.lineage.kept {
            assert!(r.body.face(*f).is_ok());
        }
        // The merged top carries both tops; B's east face is new.
        let top = r.body.face_ids().find(|f| r.body.face_normal(*f).unwrap().as_vec().z > 0.9).unwrap();
        assert_eq!(top, fa[1]);
        assert_eq!(r.faces[&top].len(), 2);
        assert_eq!(r.lineage.created.len(), 1);
        assert_eq!(r.lineage.created[0].1, None);
    }

    #[test]
    fn assemble_merges_points_within_vertex_tolerance() {
        // B 0.004 mm away: its points snap onto A's, the result is one box.
        let (a, _, faces) = union_faces(0.004);
        let r = assemble_kernel_result(&a, &faces).unwrap();
        assert_eq!(r.body.face_count(), 6);
        // 0.02 mm away: points stay apart; the faces no longer close up.
        let (a, _, faces) = union_faces(0.02);
        let e = assemble_kernel_result(&a, &faces).unwrap_err();
        assert_eq!(e.code, KernelError::INVALID_RESULT);
    }

    #[test]
    fn assemble_of_nothing_is_empty() {
        let (a, _) = box_body(1000.0, 1000.0, 1000.0);
        let e = assemble_kernel_result(&a, &[]).unwrap_err();
        assert_eq!(e.code, KernelError::EMPTY_RESULT);
        // A sliver below tolerance is dropped, which leaves nothing.
        let sliver = KernelFace {
            loops: vec![vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1000.0, 0.0, 0.0),
                Point3::new(1000.0, 0.005, 0.0),
            ]],
            sources: vec![],
        };
        let e = assemble_kernel_result(&a, &[sliver]).unwrap_err();
        assert_eq!(e.code, KernelError::EMPTY_RESULT);
    }

    #[test]
    fn assemble_refuses_non_finite_points() {
        let (a, _) = box_body(1000.0, 1000.0, 1000.0);
        let bad = KernelFace {
            loops: vec![vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(f64::NAN, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ]],
            sources: vec![],
        };
        let e = assemble_kernel_result(&a, &[bad]).unwrap_err();
        assert_eq!(e.code, KernelError::INVALID_RESULT);
    }

    #[test]
    fn assemble_is_deterministic() {
        let (a, _, faces) = union_faces(0.0);
        let r1 = assemble_kernel_result(&a, &faces).unwrap();
        let r2 = assemble_kernel_result(&a, &faces).unwrap();
        assert_eq!(r1, r2);
    }
}
