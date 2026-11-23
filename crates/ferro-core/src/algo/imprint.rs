//! Imprint: split a planar face by curves drawn on it (SketchUp's "sticky"
//! geometry). The face boundary and the new segments form a planar
//! arrangement; its bounded regions inside the face become the new faces.
//!
//! One algorithm covers a chord across a face, a shape inside it (it becomes
//! a hole plus a new face), shapes touching or crossing the boundary, and
//! crossing lines. Segments that do not close a region (dangling lines) are
//! dropped: Ferro bodies have no loose edges.

use super::AlgoError;
use crate::geom::{predicates, Plane};
use crate::math::{tol, Point2, Point3};
use crate::topo::{Body, FaceId, FaceSpec, Lineage, VertexId};
use std::collections::{BTreeMap, HashMap};

/// Result of [`imprint_face`].
#[derive(Debug, Clone, PartialEq)]
pub struct Imprinted {
    /// Faces covering the original face; the first keeps the original id.
    pub faces: Vec<FaceId>,
    /// Faces bounded (at least partly) by the new curves: the ones a user
    /// typically pushes or pulls next.
    pub new_faces: Vec<FaceId>,
}

/// 2D node index.
type Node = usize;

struct Arrangement {
    pts: Vec<Point2>,
    edges: Vec<(Node, Node)>,
}

impl Arrangement {
    fn node(&mut self, p: Point2) -> Node {
        if let Some(i) = self
            .pts
            .iter()
            .position(|q| q.distance_to(p) <= tol::VERTEX_TOL)
        {
            return i;
        }
        self.pts.push(p);
        self.pts.len() - 1
    }
}

fn seg_intersection(a: Point2, b: Point2, c: Point2, d: Point2) -> Vec<(f64, f64)> {
    // Parameters (t on ab, u on cd) of intersection points, including
    // collinear overlap end points.
    let r = b - a;
    let s = d - c;
    let den = r.perp_dot(s);
    let qp = c - a;
    let mut out = Vec::new();
    let eps = 1e-12;
    if den.abs() > eps * r.length().max(1.0) * s.length().max(1.0) {
        let t = qp.perp_dot(s) / den;
        let u = qp.perp_dot(r) / den;
        let tt = tol::VERTEX_TOL / r.length().max(1e-9);
        let tu = tol::VERTEX_TOL / s.length().max(1e-9);
        if t >= -tt && t <= 1.0 + tt && u >= -tu && u <= 1.0 + tu {
            out.push((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)));
        }
        return out;
    }
    // Parallel: collinear?
    let dist = qp.perp_dot(r).abs() / r.length().max(1e-9);
    if dist > tol::VERTEX_TOL {
        return out;
    }
    let rr = r.dot(r).max(1e-18);
    let ss = s.dot(s).max(1e-18);
    let t_c = qp.dot(r) / rr;
    let t_d = (d - a).dot(r) / rr;
    for t in [t_c, t_d] {
        if (-1e-9..=1.0 + 1e-9).contains(&t) {
            let p = a + r * t;
            out.push((t.clamp(0.0, 1.0), ((p - c).dot(s) / ss).clamp(0.0, 1.0)));
        }
    }
    for (u, p) in [(0.0, a), (1.0, b)] {
        let uu = (p - c).dot(s) / ss;
        if (-1e-9..=1.0 + 1e-9).contains(&uu) {
            out.push((u, uu.clamp(0.0, 1.0)));
        }
    }
    out
}

fn area(poly: &[Point2]) -> f64 {
    predicates::signed_area2(poly) * 0.5
}

/// A point strictly inside a simple polygon given counter-clockwise: just
/// left of the middle of its longest edge.
fn interior_point(poly: &[Point2]) -> Point2 {
    let n = poly.len();
    let (mut best, mut len) = (0, -1.0);
    for i in 0..n {
        let l = poly[i].distance_to(poly[(i + 1) % n]);
        if l > len {
            best = i;
            len = l;
        }
    }
    let a = poly[best];
    let b = poly[(best + 1) % n];
    let m = Point2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    let d = b - a;
    let left = crate::math::Vec2::new(-d.y, d.x) * (1.0 / len.max(1e-9));
    // Small enough to stay inside thin regions, large enough for robustness.
    m + left * (tol::VERTEX_TOL * 2.0).max(len * 1e-6)
}

fn inside(p: Point2, outer: &[Point2], holes: &[Vec<Point2>]) -> bool {
    predicates::point_in_polygon(p, outer)
        && !holes.iter().any(|h| predicates::point_in_polygon(p, h))
}

/// Split face `face` of `body` by the polyline `curve` (closed if `closed`),
/// whose points must lie on the face's plane. Neighbouring faces get the new
/// boundary vertices too, so the body stays manifold.
///
/// Fails with `Unsupported` for curved faces, `NotOnFace` style geometry
/// errors when points are off the plane, `Degenerate` when the curve
/// splits nothing, and `SelfIntersecting` (`ferro.geom.self_intersecting_profile`)
/// when a closed curve crosses or touches itself.
pub fn imprint_face(
    body: &mut Body,
    face: FaceId,
    curve: &[Point3],
    closed: bool,
) -> Result<(Imprinted, Lineage), AlgoError> {
    let f = body.face(face)?.clone();
    if !f.surface.is_planar() {
        return Err(AlgoError::Unsupported("imprint on curved faces"));
    }
    if curve.len() < 2 {
        return Err(AlgoError::Unsupported("a curve needs at least two points"));
    }
    let n = body.face_normal(face)?;
    let loops: Vec<Vec<VertexId>> = std::iter::once(f.outer)
        .chain(f.holes.iter().copied())
        .map(|l| body.loop_vertices(l))
        .collect();
    let p0 = body
        .point(loops[0][0])
        .ok_or(AlgoError::Unsupported("empty face"))?;
    let plane = Plane::from_origin_normal(p0, n);
    for p in curve {
        if plane.signed_distance_to(*p).abs() > tol::VERTEX_TOL * 10.0 {
            return Err(AlgoError::Unsupported("the curve is not on the face"));
        }
    }
    let to2 = |p: Point3| plane.to_local(p);
    if closed {
        // A closed curve that crosses or touches itself (a bow-tie) would
        // leave a pinched region: refuse it with exact predicates (D-53). A
        // repeated closing point is tolerated.
        let mut c2: Vec<Point2> = curve.iter().map(|p| to2(*p)).collect();
        if c2.len() > 3 && c2[0].distance_to(c2[c2.len() - 1]) <= tol::VERTEX_TOL {
            c2.pop();
        }
        if c2.len() >= 3
            && !predicates::points_are_collinear(&c2)
            && !predicates::polygon_is_simple(&c2)
        {
            return Err(crate::geom::GeomError::SelfIntersecting.into());
        }
    }
    let outer2: Vec<Point2> = loops[0]
        .iter()
        .map(|v| to2(body.point(*v).expect("v")))
        .collect();
    let holes2: Vec<Vec<Point2>> = loops[1..]
        .iter()
        .map(|l| l.iter().map(|v| to2(body.point(*v).expect("v"))).collect())
        .collect();
    // Orientation of the face loops in this frame must be CCW for the outer.
    let flip = area(&outer2) < 0.0;
    let fix = |mut p: Vec<Point2>, ccw: bool| {
        if (area(&p) > 0.0) != ccw {
            p.reverse();
        }
        p
    };
    let outer_ccw = fix(outer2.clone(), true);
    let holes_cw: Vec<Vec<Point2>> = holes2.iter().map(|h| fix(h.clone(), false)).collect();
    let holes_ccw: Vec<Vec<Point2>> = holes2.iter().map(|h| fix(h.clone(), true)).collect();
    let _ = flip;

    // Input segments.
    let mut segs: Vec<(Point2, Point2, bool)> = Vec::new(); // (a, b, is_new)
    for lp in std::iter::once(&outer_ccw).chain(holes_cw.iter()) {
        for i in 0..lp.len() {
            segs.push((lp[i], lp[(i + 1) % lp.len()], false));
        }
    }
    let c2: Vec<Point2> = curve.iter().map(|p| to2(*p)).collect();
    let m = c2.len();
    let count = if closed { m } else { m - 1 };
    for i in 0..count {
        let (a, b) = (c2[i], c2[(i + 1) % m]);
        if a.distance_to(b) > tol::VERTEX_TOL {
            segs.push((a, b, true));
        }
    }

    // Split every segment at all intersections.
    let mut cuts: Vec<Vec<f64>> = vec![vec![0.0, 1.0]; segs.len()];
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            for (t, u) in seg_intersection(segs[i].0, segs[i].1, segs[j].0, segs[j].1) {
                cuts[i].push(t);
                cuts[j].push(u);
            }
        }
    }
    let mut arr = Arrangement {
        pts: Vec::new(),
        edges: Vec::new(),
    };
    let mut is_new_edge: HashMap<(Node, Node), bool> = HashMap::new();
    for (i, (a, b, new)) in segs.iter().enumerate() {
        let mut ts = cuts[i].clone();
        ts.sort_by(|x, y| x.total_cmp(y));
        let pts: Vec<Point2> = ts.iter().map(|t| *a + (*b - *a) * *t).collect();
        for w in pts.windows(2) {
            let (na, nb) = (arr.node(w[0]), arr.node(w[1]));
            if na == nb {
                continue;
            }
            let key = (na.min(nb), na.max(nb));
            if let Some(flag) = is_new_edge.get_mut(&key) {
                *flag = *flag && *new; // a boundary edge stays a boundary edge
                continue;
            }
            // New pieces outside the face are dropped.
            if *new {
                let mid = Point2::new((w[0].x + w[1].x) * 0.5, (w[0].y + w[1].y) * 0.5);
                let on_boundary = segs.iter().any(|(sa, sb, sn)| {
                    !sn && seg_intersection(*sa, *sb, mid, mid).len() == 1
                        && (mid - *sa).perp_dot(*sb - *sa).abs() / (*sb - *sa).length().max(1e-9)
                            <= tol::VERTEX_TOL
                });
                if !on_boundary && !inside(mid, &outer_ccw, &holes_ccw) {
                    continue;
                }
            }
            is_new_edge.insert(key, *new);
            arr.edges.push(key);
        }
    }

    // Drop dangling edges (degree 1), repeatedly.
    loop {
        let mut deg = vec![0usize; arr.pts.len()];
        for (a, b) in &arr.edges {
            deg[*a] += 1;
            deg[*b] += 1;
        }
        let before = arr.edges.len();
        arr.edges.retain(|(a, b)| deg[*a] > 1 && deg[*b] > 1);
        if arr.edges.len() == before {
            break;
        }
    }
    if !arr
        .edges
        .iter()
        .any(|k| is_new_edge.get(k).copied().unwrap_or(false))
    {
        return Err(AlgoError::Unsupported("the curve does not split the face"));
    }

    // Half-edge face extraction: at each node, outgoing edges sorted by angle;
    // the next edge after arriving along (u→v) is the one just clockwise of
    // (v→u) around v, which traces regions counter-clockwise.
    let mut out_edges: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
    for (a, b) in &arr.edges {
        out_edges.entry(*a).or_default().push(*b);
        out_edges.entry(*b).or_default().push(*a);
    }
    let ang = |from: Node, to: Node| {
        let d = arr.pts[to] - arr.pts[from];
        d.y.atan2(d.x)
    };
    for (v, list) in out_edges.iter_mut() {
        let v = *v;
        list.sort_by(|a, b| ang(v, *a).total_cmp(&ang(v, *b)));
    }
    let mut used: std::collections::HashSet<(Node, Node)> = Default::default();
    let mut cycles: Vec<Vec<Node>> = Vec::new();
    let directed: Vec<(Node, Node)> = arr
        .edges
        .iter()
        .flat_map(|(a, b)| [(*a, *b), (*b, *a)])
        .collect();
    for start in directed {
        if used.contains(&start) {
            continue;
        }
        let mut cyc = Vec::new();
        let (mut u, mut v) = start;
        loop {
            used.insert((u, v));
            cyc.push(u);
            let list = &out_edges[&v];
            let i = list.iter().position(|x| *x == u).expect("edge listed");
            // Previous in CCW order = next clockwise.
            let w = list[(i + list.len() - 1) % list.len()];
            u = v;
            v = w;
            if (u, v) == start || cyc.len() > arr.edges.len() * 2 + 2 {
                break;
            }
        }
        cycles.push(cyc);
    }
    let poly = |c: &[Node]| c.iter().map(|i| arr.pts[*i]).collect::<Vec<_>>();
    let mut faces2: Vec<(Vec<Node>, Vec<Vec<Node>>)> = Vec::new();
    let mut outers: Vec<Vec<Node>> = Vec::new(); // negative cycles (component boundaries)
    for c in cycles {
        let p = poly(&c);
        let a = area(&p);
        if a > tol::VERTEX_TOL * tol::VERTEX_TOL {
            if inside(interior_point(&p), &outer_ccw, &holes_ccw) {
                faces2.push((c, Vec::new()));
            }
        } else if a < 0.0 {
            outers.push(c);
        }
    }
    // A component boundary lying inside a region is a hole of the smallest
    // such region (nested shapes, original holes).
    for c in outers {
        let p = poly(&c);
        let probe = p[0];
        let mut best: Option<(usize, f64)> = None;
        for (i, (fc, _)) in faces2.iter().enumerate() {
            if fc.iter().any(|n| c.contains(n)) {
                continue; // same component
            }
            let fp = poly(fc);
            if predicates::point_in_polygon(probe, &fp) {
                let a = area(&fp);
                if best.is_none_or(|(_, ba)| a < ba) {
                    best = Some((i, a));
                }
            }
        }
        if let Some((i, _)) = best {
            faces2[i].1.push(c);
        }
    }
    if faces2.len() < 2 {
        return Err(AlgoError::Unsupported("the curve does not split the face"));
    }

    // Back to 3D: reuse existing vertices where positions match.
    let mut vmap = body.vertex_map();
    let existing: Vec<(VertexId, Point3)> = body.vertices().collect();
    let mut node_vid: HashMap<Node, VertexId> = HashMap::new();
    let mut new_vertices: Vec<(VertexId, Point3)> = Vec::new();
    for (i, p2) in arr.pts.iter().enumerate() {
        let p3 = plane.to_world(*p2);
        let vid = match existing
            .iter()
            .find(|(_, q)| q.distance_to(p3) <= tol::VERTEX_TOL)
        {
            Some((v, _)) => *v,
            None => {
                let v = body.add_vertex(p3);
                vmap.insert(v, crate::topo::Vertex { point: p3 });
                new_vertices.push((v, p3));
                v
            }
        };
        node_vid.insert(i, vid);
    }
    // Keep the original id on the largest region.
    faces2.sort_by(|a, b| area(&poly(&b.0)).total_cmp(&area(&poly(&a.0))));
    let mut specs: Vec<FaceSpec> = body
        .face_specs()
        .into_iter()
        .filter(|s| s.id != Some(face))
        .collect();
    let first_new = specs.len();
    let mut new_flags = Vec::new();
    for (k, (c, holes)) in faces2.iter().enumerate() {
        let touches_new = c.iter().zip(c.iter().cycle().skip(1)).any(|(a, b)| {
            is_new_edge
                .get(&((*a).min(*b), (*a).max(*b)))
                .copied()
                .unwrap_or(false)
        }) || holes.iter().any(|h| {
            h.iter().zip(h.iter().cycle().skip(1)).any(|(a, b)| {
                is_new_edge
                    .get(&((*a).min(*b), (*a).max(*b)))
                    .copied()
                    .unwrap_or(false)
            })
        });
        new_flags.push(touches_new && k > 0);
        let mut spec = FaceSpec::new(c.iter().map(|i| node_vid[i]).collect());
        spec.holes = holes
            .iter()
            .map(|h| h.iter().map(|i| node_vid[i]).collect())
            .collect();
        spec.id = if k == 0 { Some(face) } else { None };
        spec.surface = Some(f.surface);
        spec.smooth_group = f.smooth_group;
        specs.push(spec);
    }
    // New vertices on the old boundary must also appear in neighbour loops.
    for s in specs[..first_new].iter_mut() {
        for lp in std::iter::once(&mut s.outer).chain(s.holes.iter_mut()) {
            let mut out = Vec::with_capacity(lp.len());
            for i in 0..lp.len() {
                let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                out.push(a);
                let (pa, pb) = (vmap[&a].point, vmap[&b].point);
                let ab = pb - pa;
                let l2 = ab.length_squared();
                if l2 <= 0.0 {
                    continue;
                }
                let mut on: Vec<(f64, VertexId)> = new_vertices
                    .iter()
                    .filter_map(|(v, p)| {
                        let t = (*p - pa).dot(ab) / l2;
                        let d = (pa + ab * t).distance_to(*p);
                        (t > 1e-9 && t < 1.0 - 1e-9 && d <= tol::VERTEX_TOL).then_some((t, *v))
                    })
                    .collect();
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                out.extend(on.into_iter().map(|(_, v)| v));
            }
            *lp = out;
        }
    }
    let ids = body.rebuild(vmap, specs)?;
    let faces: Vec<FaceId> = ids[first_new..].to_vec();
    let new_faces: Vec<FaceId> = faces
        .iter()
        .zip(&new_flags)
        .filter(|(_, n)| **n)
        .map(|(f, _)| *f)
        .collect();
    let lineage = Lineage {
        kept: vec![face],
        created: faces[1..].iter().map(|f2| (*f2, Some(face))).collect(),
        deleted: Vec::new(),
    };
    Ok((Imprinted { faces, new_faces }, lineage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topo::tests_support::box_body;

    fn top(b: &Body) -> FaceId {
        b.face_ids()
            .find(|f| b.face_normal(*f).unwrap().as_vec().z > 0.9)
            .unwrap()
    }

    #[test]
    fn rectangle_inside_a_face_becomes_hole_plus_face() {
        let (mut b, _) = box_body(4000.0, 3000.0, 1000.0);
        let t = top(&b);
        let r = [
            Point3::new(1000.0, 1000.0, 1000.0),
            Point3::new(2000.0, 1000.0, 1000.0),
            Point3::new(2000.0, 2000.0, 1000.0),
            Point3::new(1000.0, 2000.0, 1000.0),
        ];
        let (res, _) = imprint_face(&mut b, t, &r, true).unwrap();
        assert_eq!(res.faces.len(), 2);
        assert_eq!(res.faces[0], t, "original id kept on the larger face");
        assert_eq!(res.new_faces.len(), 1);
        b.validate_solid().unwrap();
        assert_eq!(b.face_count(), 7);
        assert!((b.face_area(res.new_faces[0]).unwrap() - 1e6).abs() < 1e-6);
        assert_eq!(b.face(t).unwrap().holes.len(), 1);
        assert!((b.volume() - 4000.0 * 3000.0 * 1000.0).abs() < 1.0);
    }

    #[test]
    fn chord_splits_a_face_in_two() {
        let (mut b, _) = box_body(4000.0, 3000.0, 1000.0);
        let t = top(&b);
        let line = [
            Point3::new(1500.0, 0.0, 1000.0),
            Point3::new(1500.0, 3000.0, 1000.0),
        ];
        let (res, _) = imprint_face(&mut b, t, &line, false).unwrap();
        assert_eq!(res.faces.len(), 2);
        b.validate_solid().unwrap();
        // Side faces got the new vertices (no T-junctions).
        let areas: Vec<f64> = res.faces.iter().map(|f| b.face_area(*f).unwrap()).collect();
        assert!((areas[0] - 2500.0 * 3000.0).abs() < 1e-6);
        assert!((areas[1] - 1500.0 * 3000.0).abs() < 1e-6);
    }

    #[test]
    fn shape_touching_the_boundary_and_crossing_lines() {
        let (mut b, _) = box_body(4000.0, 3000.0, 1000.0);
        let t = top(&b);
        // A rectangle sitting on the y = 0 edge (like a door outline).
        let r = [
            Point3::new(1000.0, 0.0, 1000.0),
            Point3::new(2000.0, 0.0, 1000.0),
            Point3::new(2000.0, 1000.0, 1000.0),
            Point3::new(1000.0, 1000.0, 1000.0),
        ];
        let (res, _) = imprint_face(&mut b, t, &r, true).unwrap();
        assert_eq!(res.faces.len(), 2);
        b.validate_solid().unwrap();
        // An X across the remaining big face makes four more regions.
        let big = res.faces[0];
        let x1 = [
            Point3::new(0.0, 3000.0, 1000.0),
            Point3::new(4000.0, 1500.0, 1000.0),
        ];
        imprint_face(&mut b, big, &x1, false).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - 4000.0 * 3000.0 * 1000.0).abs() < 1.0);
    }

    #[test]
    fn degenerate_and_off_plane_curves_fail() {
        let (mut b, _) = box_body(1000.0, 1000.0, 1000.0);
        let t = top(&b);
        // A dangling line that does not reach the boundary splits nothing.
        let dangling = [
            Point3::new(200.0, 200.0, 1000.0),
            Point3::new(500.0, 500.0, 1000.0),
        ];
        assert!(imprint_face(&mut b, t, &dangling, false).is_err());
        let off = [
            Point3::new(0.0, 500.0, 900.0),
            Point3::new(1000.0, 500.0, 900.0),
        ];
        assert!(imprint_face(&mut b, t, &off, false).is_err());
        // Tolerance edge: a chord ending 0.005 mm from the boundary still snaps.
        let near = [
            Point3::new(500.0, 0.005, 1000.0),
            Point3::new(500.0, 1000.0, 1000.0),
        ];
        imprint_face(&mut b, t, &near, false).unwrap();
        b.validate_solid().unwrap();
        assert_eq!(b.face_count(), 7);
    }
}
