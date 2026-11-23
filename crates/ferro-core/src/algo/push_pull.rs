use super::AlgoError;
use crate::math::{tol, Point3};
use crate::topo::{Body, FaceId, FaceSpec, Lineage, VertexId};

/// How a push / pull was carried out (useful for UI hints and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPullMode {
    /// A lone face (open sheet) became a solid.
    Extrude,
    /// Every neighbour was perpendicular: the face moved and its neighbours
    /// stretched (SketchUp behaviour on boxes). No new faces.
    Stretch,
    /// New side faces were added between the old and the moved face.
    Prism,
    /// The face was pushed exactly to an opposite parallel face: a hole now
    /// goes through (the pushed face no longer exists).
    Through,
}

/// Push / pull face `face` of `body` by `distance` mm along its normal
/// (negative pushes inward). The pulled face keeps its id (D-25).
///
/// * Open single face → extruded into a closed solid.
/// * Closed solid, all neighbours perpendicular → face moves, neighbours stretch.
///   Pulled back flush (moved corners within `VERTEX_TOL` of the far corners
///   of the stretched faces), those faces vanish; a pocket floor that now
///   fills its hole merges into the surrounding face, so the face is in
///   `Lineage::deleted` and the surrounding face keeps its id. Further than
///   flush, the pocket turns into a boss (SketchUp behaviour).
/// * Closed solid, no neighbour perpendicular → a prism (boss or pocket) is added.
/// * Mixed neighbours (e.g. half of a split top face) → a prism whose side
///   faces that lie in the plane of a perpendicular neighbour are merged into
///   it, so that neighbour grows or shrinks (SketchUp behaviour).
///
/// * Closed solid, a face with one loop pushed exactly onto an opposite
///   parallel face (within `VERTEX_TOL`) → a hole through (`Through`).
///
/// The result is validated and the body is left unchanged on failure:
/// collapse or inversion returns `Collapse`; a result whose faces cross,
/// e.g. a pocket pushed deeper than the solid (past `VERTEX_TOL` beyond the
/// opposite face), returns `Topo(SelfIntersecting)`
/// (`ferro.topo.self_intersecting`).
pub fn push_pull_face(
    body: &mut Body,
    face: FaceId,
    distance: f64,
) -> Result<(PushPullMode, Lineage), AlgoError> {
    if distance.abs() < tol::SHORT_CURVE_TOL {
        return Err(AlgoError::ZeroDistance);
    }
    let n = body.face_normal(face)?;
    let offset = n * distance;
    let f = body.face(face)?.clone();
    if !f.surface.is_planar() {
        return Err(AlgoError::Unsupported("push / pull of curved faces"));
    }
    let loops: Vec<Vec<VertexId>> = std::iter::once(f.outer)
        .chain(f.holes.iter().copied())
        .map(|l| body.loop_vertices(l))
        .collect();

    let mut work = body.clone();
    if distance < 0.0 && work.is_closed() && loops.len() == 1 {
        if let Some(r) = push_through(&mut work, face, &loops[0], n, -distance)? {
            check_solid(&work)?;
            *body = work;
            return Ok(r);
        }
    }
    let result = if !work.is_closed() {
        if work.face_count() != 1 {
            return Err(AlgoError::Unsupported(
                "push / pull of a face that is part of an open multi-face body",
            ));
        }
        extrude_sheet(&mut work, face, &loops, offset, distance)?
    } else {
        let neighbors = work.neighbors(face)?;
        let perpendicular = |nf: FaceId| -> Result<bool, AlgoError> {
            let nn = work.face_normal(nf)?;
            Ok(work.face(nf)?.surface.is_planar() && nn.dot(n).abs() <= 1e-6)
        };
        let mut flags = Vec::new();
        for nb in neighbors.iter().flatten() {
            flags.push(perpendicular(*nb)?);
        }
        let has_holes = loops.len() > 1;
        if flags.iter().all(|x| *x) && !has_holes {
            let moves: Vec<(VertexId, Point3)> = loops[0]
                .iter()
                .map(|v| (*v, work.point(*v).expect("loop vertex") + offset))
                .collect();
            work.move_vertices(&moves)?;
            // Pulled back flush (a pocket floor to the top, a step to its
            // neighbour): the moved corners land on the far corners of the
            // stretched faces, which vanish; a filled hole heals.
            let deleted = merge_flush(&mut work, face, &loops[0], &neighbors, n)?;
            // A step pulled back below an old corner leaves that corner as
            // the tip of a spike in its faces: drop it.
            remove_spikes(&mut work)?;
            // A neighbour may legitimately turn around (a step pulled past its
            // neighbour flips), but it must not vanish or cross itself; the
            // solid check below catches a turned-inside-out body.
            for nf in neighbors.iter().flatten() {
                if deleted.contains(nf) {
                    continue;
                }
                let a = work.face_area_vector(*nf)?;
                if a.length() <= tol::VERTEX_TOL * tol::VERTEX_TOL * 100.0 {
                    return Err(AlgoError::Collapse);
                }
                let pts = work.face_points(*nf)?;
                let nn = work.face_normal(*nf)?;
                if !is_simple(&pts, nn) {
                    return Err(AlgoError::Collapse);
                }
            }
            (
                PushPullMode::Stretch,
                Lineage {
                    kept: work.face_ids().collect(),
                    created: Vec::new(),
                    deleted,
                },
            )
        } else if flags.iter().all(|x| !*x) {
            prism(&mut work, face, &loops, offset, false)?
        } else {
            prism(&mut work, face, &loops, offset, true)?
        }
    };

    check_solid(&work)?;
    *body = work;
    Ok(result)
}

/// After a stretch: merge each moved vertex of `outer` (the loop of `face`)
/// that landed within `VERTEX_TOL` of another vertex of a stretched
/// neighbour into that vertex, drop loops that collapse to fewer than three
/// vertices, and, when `face` now exactly fills a hole of a coplanar face
/// with the same normal, remove that hole and `face` (the pocket is gone, one
/// face again). Returns the deleted faces; no merge leaves the body alone.
fn merge_flush(
    work: &mut Body,
    face: FaceId,
    outer: &[VertexId],
    neighbors: &[Option<FaceId>],
    n: crate::math::UnitVec3,
) -> Result<Vec<FaceId>, AlgoError> {
    use std::collections::{BTreeMap, BTreeSet};
    let moved: BTreeSet<VertexId> = outer.iter().copied().collect();
    let mut candidates = BTreeSet::new();
    for nf in neighbors.iter().flatten() {
        let f = work.face(*nf)?;
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            candidates.extend(
                work.loop_vertices(*l)
                    .into_iter()
                    .filter(|v| !moved.contains(v)),
            );
        }
    }
    let mut map: BTreeMap<VertexId, VertexId> = BTreeMap::new();
    for v in outer {
        let p = work.point(*v).ok_or(AlgoError::Collapse)?;
        let mut best: Option<(f64, VertexId)> = None;
        for w in &candidates {
            let d = work.point(*w).ok_or(AlgoError::Collapse)?.distance_to(p);
            if d <= tol::VERTEX_TOL && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, *w));
            }
        }
        if let Some((_, w)) = best {
            map.insert(*v, w);
        }
    }
    if map.is_empty() {
        return Ok(Vec::new());
    }
    let fix = |lp: &[VertexId]| -> Vec<VertexId> {
        let mut out: Vec<VertexId> = Vec::with_capacity(lp.len());
        for v in lp.iter().map(|v| *map.get(v).unwrap_or(v)) {
            if out.last() != Some(&v) {
                out.push(v);
            }
        }
        while out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    };
    let mut deleted = Vec::new();
    let mut specs = Vec::new();
    for mut s in work.face_specs() {
        s.outer = fix(&s.outer);
        s.holes = s
            .holes
            .iter()
            .map(|h| fix(h))
            .filter(|h| h.len() >= 3)
            .collect();
        match s.id {
            Some(id) if s.outer.len() < 3 => deleted.push(id),
            None if s.outer.len() < 3 => {}
            _ => specs.push(s),
        }
    }
    // Heal: `face` fills a hole of a coplanar face pointing the same way.
    let same_cycle = |a: &[VertexId], b: &[VertexId]| {
        a.len() == b.len()
            && (0..b.len()).any(|k| (0..a.len()).all(|i| a[i] == b[(i + k) % b.len()]))
    };
    if let Some(fi) = specs.iter().position(|s| s.id == Some(face)) {
        if specs[fi].holes.is_empty() {
            let mut rev = specs[fi].outer.clone();
            rev.reverse();
            let host = specs.iter().enumerate().find_map(|(si, s)| {
                let coplanar = s.id != Some(face)
                    && matches!(s.surface, Some(crate::geom::Surface::Plane(pl))
                        if pl.normal().dot(n) >= 1.0 - tol::ANGLE_TOL);
                let hole = s.holes.iter().position(|h| same_cycle(h, &rev))?;
                coplanar.then_some((si, hole))
            });
            if let Some((si, hole)) = host {
                specs[si].holes.remove(hole);
                specs.remove(fi);
                deleted.push(face);
            }
        }
    }
    let verts = work.vertex_map();
    work.rebuild(verts, specs)
        .map_err(|_| AlgoError::Collapse)?;
    // Refit the planes of faces whose corners snapped onto the merged vertices.
    work.move_vertices(&[])?;
    Ok(deleted)
}

/// Distance (mm) from `p` to the line through `a` and `b` (to `a` when the
/// two coincide).
fn distance_to_line(p: Point3, a: Point3, b: Point3) -> f64 {
    let d = b - a;
    let len = d.length();
    if len <= tol::PARAM_TOL {
        return p.distance_to(a);
    }
    (p - a).cross(d).length() / len
}

/// Remove vertices left as spike tips by a stretch: a vertex where a loop
/// runs out and straight back (previous, vertex and next on one line within
/// `VERTEX_TOL`, direction reversed), provided every loop using it passes
/// straight through it or doubles back there. Loops left with fewer than
/// three vertices are a collapse.
fn remove_spikes(work: &mut Body) -> Result<(), AlgoError> {
    use std::collections::BTreeSet;
    // Each pass removes at least one vertex; bounded by the vertex count.
    for _ in 0..=work.vertex_count() {
        let specs = work.face_specs();
        let mut tips = BTreeSet::new();
        let mut bent = BTreeSet::new();
        for s in &specs {
            for lp in std::iter::once(&s.outer).chain(&s.holes) {
                let k = lp.len();
                for i in 0..k {
                    let (a, v, b) = (lp[(i + k - 1) % k], lp[i], lp[(i + 1) % k]);
                    let (Some(pa), Some(pv), Some(pb)) =
                        (work.point(a), work.point(v), work.point(b))
                    else {
                        return Err(AlgoError::Collapse);
                    };
                    if distance_to_line(pv, pa, pb) > tol::VERTEX_TOL {
                        bent.insert(v);
                    } else if (pv - pa).dot(pb - pv) < 0.0 {
                        tips.insert(v);
                    }
                }
            }
        }
        let remove: BTreeSet<VertexId> = tips.difference(&bent).copied().collect();
        if remove.is_empty() {
            return Ok(());
        }
        let mut out = Vec::with_capacity(specs.len());
        for mut s in specs {
            s.outer.retain(|v| !remove.contains(v));
            s.holes
                .iter_mut()
                .for_each(|h| h.retain(|v| !remove.contains(v)));
            if s.outer.len() < 3 || s.holes.iter().any(|h| h.len() < 3) {
                return Err(AlgoError::Collapse);
            }
            out.push(s);
        }
        let verts = work.vertex_map();
        work.rebuild(verts, out)?;
    }
    Ok(())
}

/// Validate the result as a solid: self-intersection keeps its own code,
/// every other failure is a collapse.
fn check_solid(work: &Body) -> Result<(), AlgoError> {
    match work.validate_solid() {
        Ok(()) => Ok(()),
        Err(e @ crate::topo::TopoError::SelfIntersecting(_)) => Err(AlgoError::Topo(e)),
        Err(_) => Err(AlgoError::Collapse),
    }
}

/// Push `face` inward by `depth` when that lands exactly on an opposite,
/// parallel face that contains it: cut a hole through. `Ok(None)` when the
/// situation does not apply.
fn push_through(
    work: &mut Body,
    face: FaceId,
    outer: &[VertexId],
    n: crate::math::UnitVec3,
    depth: f64,
) -> Result<Option<(PushPullMode, Lineage)>, AlgoError> {
    let offset = n * -depth;
    let target: Vec<Point3> = outer
        .iter()
        .map(|v| work.point(*v).expect("loop vertex") + offset)
        .collect();
    // The opposite face: anti-parallel normal, its plane through the target.
    let candidates: Vec<FaceId> = work.face_ids().filter(|f| *f != face).collect();
    let mut opposite = None;
    for b in candidates {
        let fb = work.face(b)?;
        let crate::geom::Surface::Plane(pl) = fb.surface else {
            continue;
        };
        let nb = work.face_normal(b)?;
        if nb.dot(n) > -1.0 + 1e-9 || !target.iter().all(|p| pl.contains(*p)) {
            continue;
        }
        // Every target point inside the face (or on its boundary).
        let pts = work.face_points(b)?;
        let plane = crate::geom::Plane::from_origin_normal(pts[0], nb);
        let poly: Vec<crate::math::Point2> = pts.iter().map(|p| plane.to_local(*p)).collect();
        let centre = Point3::centroid(&target).expect("loop");
        if crate::geom::predicates::point_in_polygon(plane.to_local(centre), &poly) {
            opposite = Some(b);
            break;
        }
    }
    let Some(b) = opposite else { return Ok(None) };
    // Imprint the target loop on the opposite face (it may already be there).
    let region = match crate::algo::imprint_face(work, b, &target, true) {
        Ok((imp, _)) => imp.faces.into_iter().find(|f| {
            work.face_points(*f).is_ok_and(|pts| {
                pts.len() == target.len()
                    && pts
                        .iter()
                        .all(|p| target.iter().any(|q| q.distance_to(*p) <= tol::VERTEX_TOL))
            })
        }),
        Err(_) => None,
    };
    let Some(region) = region else {
        return Ok(None);
    };
    let find = |p: Point3| {
        work.vertices()
            .find(|(_, q)| q.distance_to(p) <= tol::VERTEX_TOL)
            .map(|(v, _)| v)
    };
    let bottom: Vec<VertexId> = target
        .iter()
        .map(|p| find(*p))
        .collect::<Option<_>>()
        .ok_or(AlgoError::Collapse)?;
    let mut specs: Vec<FaceSpec> = work
        .face_specs()
        .into_iter()
        .filter(|s| s.id != Some(face) && s.id != Some(region))
        .collect();
    let first_side = specs.len();
    let k = outer.len();
    for i in 0..k {
        let j = (i + 1) % k;
        specs.push(FaceSpec::new(vec![
            outer[i], outer[j], bottom[j], bottom[i],
        ]));
    }
    let verts = work.vertex_map();
    let ids = work.rebuild(verts, specs)?;
    Ok(Some((
        PushPullMode::Through,
        Lineage {
            kept: ids[..first_side].to_vec(),
            created: ids[first_side..].iter().map(|f| (*f, Some(face))).collect(),
            deleted: vec![face, region],
        },
    )))
}

fn extrude_sheet(
    work: &mut Body,
    face: FaceId,
    loops: &[Vec<VertexId>],
    offset: crate::math::Vec3,
    distance: f64,
) -> Result<(PushPullMode, Lineage), AlgoError> {
    let up = distance > 0.0;
    let mut specs = Vec::new();
    let mut cap_loops = Vec::new();
    let mut base_loops = Vec::new();
    for l in loops {
        let top: Vec<VertexId> = l
            .iter()
            .map(|v| {
                let p = work.point(*v).expect("loop vertex");
                work.add_vertex(p + offset)
            })
            .collect();
        let k = l.len();
        let groups = smooth_runs(work, l);
        for i in 0..k {
            let j = (i + 1) % k;
            let quad = if up {
                vec![l[i], l[j], top[j], top[i]]
            } else {
                vec![l[j], l[i], top[i], top[j]]
            };
            let mut spec = FaceSpec::new(quad);
            spec.smooth_group = groups[i];
            specs.push(spec);
        }
        let (mut cap, mut base) = (top, l.clone());
        if up {
            base.reverse();
        } else {
            cap.reverse();
        }
        cap_loops.push(cap);
        base_loops.push(base);
    }
    let mut cap = FaceSpec::new(cap_loops[0].clone());
    cap.id = Some(face);
    cap.holes = cap_loops[1..].to_vec();
    let mut base = FaceSpec::new(base_loops[0].clone());
    base.holes = base_loops[1..].to_vec();
    let mut all = vec![cap, base];
    all.extend(specs);
    let verts = work.vertex_map();
    let ids = work.rebuild(verts, all)?;
    let created = ids[1..].iter().map(|f| (*f, Some(face))).collect();
    Ok((
        PushPullMode::Extrude,
        Lineage {
            kept: vec![face],
            created,
            deleted: vec![],
        },
    ))
}

/// Smooth groups for the side faces of a loop: runs of edges turning by less
/// than 20° (a circle or arc drawn as segments) share a group, so their
/// sides render as one curved surface. `None` for sharp corners.
fn smooth_runs(work: &mut Body, lp: &[VertexId]) -> Vec<Option<u32>> {
    let k = lp.len();
    let mut out = vec![None; k];
    if k < 8 {
        return out;
    }
    let pts: Vec<Point3> = lp.iter().map(|v| work.point(*v).expect("v")).collect();
    let dir = |i: usize| (pts[(i + 1) % k] - pts[i]).normalize();
    // smooth[i]: the joint between edge i and edge i+1 is smooth.
    let smooth: Vec<bool> = (0..k)
        .map(|i| match (dir(i), dir((i + 1) % k)) {
            (Some(a), Some(b)) => a.dot(b) > 20f64.to_radians().cos(),
            _ => false,
        })
        .collect();
    if smooth.iter().all(|x| *x) {
        let g = work.alloc_smooth_group();
        return vec![Some(g); k];
    }
    // Start after a sharp joint and walk once around.
    let start = (0..k)
        .find(|i| !smooth[*i])
        .map(|i| (i + 1) % k)
        .unwrap_or(0);
    let mut current: Option<u32> = None;
    for step in 0..k {
        let i = (start + step) % k;
        let prev_smooth = smooth[(i + k - 1) % k];
        let next_smooth = smooth[i];
        if prev_smooth {
            out[i] = current;
        } else if next_smooth {
            current = Some(work.alloc_smooth_group());
            out[i] = current;
        } else {
            current = None;
        }
    }
    out
}

/// Replace the directed edge `a → b` in whichever loop of `spec` holds it by
/// the path `a → mid… → b`. Returns false when the edge is not there.
fn splice(spec: &mut FaceSpec, a: VertexId, b: VertexId, mid: &[VertexId]) -> bool {
    for lp in std::iter::once(&mut spec.outer).chain(spec.holes.iter_mut()) {
        let n = lp.len();
        if let Some(i) = (0..n).find(|&i| lp[i] == a && lp[(i + 1) % n] == b) {
            let at = i + 1;
            for (k, v) in mid.iter().enumerate() {
                lp.insert(at + k, *v);
            }
            return true;
        }
    }
    false
}

/// `true` if the closed polygon is simple (exact test in its plane, D-53)
/// and has no two vertices within `VERTEX_TOL`.
fn is_simple(pts: &[Point3], normal: crate::math::UnitVec3) -> bool {
    let Some(&first) = pts.first() else {
        return false;
    };
    let plane = crate::geom::Plane::from_origin_normal(first, normal);
    let p: Vec<crate::math::Point2> = pts.iter().map(|q| plane.to_local(*q)).collect();
    let repeated = (0..p.len()).any(|i| {
        p.iter()
            .skip(i + 1)
            .any(|q| q.distance_to(p[i]) <= tol::VERTEX_TOL)
    });
    !repeated && crate::geom::predicates::polygon_is_simple(&p)
}

fn prism(
    work: &mut Body,
    face: FaceId,
    loops: &[Vec<VertexId>],
    offset: crate::math::Vec3,
    heal: bool,
) -> Result<(PushPullMode, Lineage), AlgoError> {
    let mut specs: Vec<FaceSpec> = work
        .face_specs()
        .into_iter()
        .filter(|s| s.id != Some(face))
        .collect();
    let kept: Vec<FaceId> = specs.iter().filter_map(|s| s.id).chain([face]).collect();
    let mut cap_loops = Vec::new();
    let mut sides = Vec::new();
    for l in loops {
        let top: Vec<VertexId> = l
            .iter()
            .map(|v| {
                let p = work.point(*v).expect("loop vertex");
                work.add_vertex(p + offset)
            })
            .collect();
        let k = l.len();
        let groups = smooth_runs(work, l);
        for i in 0..k {
            let j = (i + 1) % k;
            let quad = vec![l[i], l[j], top[j], top[i]];
            if heal {
                // A side lying in the plane of the neighbour across edge
                // l[i]→l[j] is merged into it: the neighbour holds l[j]→l[i];
                // replace that edge by l[j] → top[j] → top[i] → l[i].
                let quad_pts: Vec<Point3> =
                    quad.iter().map(|v| work.point(*v).expect("v")).collect();
                let neighbour = specs.iter().position(|sp| {
                    let lps = std::iter::once(&sp.outer).chain(sp.holes.iter());
                    let has = lps.clone().any(|lp| {
                        let n = lp.len();
                        (0..n).any(|x| lp[x] == l[j] && lp[(x + 1) % n] == l[i])
                    });
                    has && sp.surface.is_some_and(|s| match s {
                        crate::geom::Surface::Plane(pl) => quad_pts.iter().all(|p| pl.contains(*p)),
                        _ => false,
                    })
                });
                if let Some(ni) = neighbour {
                    if splice(&mut specs[ni], l[j], l[i], &[top[j], top[i]]) {
                        continue;
                    }
                }
            }
            let mut spec = FaceSpec::new(quad);
            spec.smooth_group = groups[i];
            sides.push(spec);
        }
        cap_loops.push(top);
    }
    let mut cap = FaceSpec::new(cap_loops[0].clone());
    cap.id = Some(face);
    cap.holes = cap_loops[1..].to_vec();
    let side_start = specs.len() + 1;
    specs.push(cap);
    specs.extend(sides);
    let verts = work.vertex_map();
    let ids = work.rebuild(verts, specs)?;
    if heal {
        // A merged side pushed below an old corner of its neighbour leaves
        // that corner as a spike tip (e.g. half of a split top pushed down):
        // drop it, then every grown neighbour must be a simple polygon.
        remove_spikes(work)?;
        let fids: Vec<FaceId> = work.face_ids().collect();
        for f in fids {
            let pts = work.face_points(f)?;
            if !is_simple(&pts, work.face_normal(f)?) {
                return Err(AlgoError::Collapse);
            }
        }
    }
    let created = ids[side_start..].iter().map(|f| (*f, Some(face))).collect();
    Ok((
        PushPullMode::Prism,
        Lineage {
            kept,
            created,
            deleted: vec![],
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::topo::tests_support::box_body;

    fn rect_sheet(w: f64, d: f64) -> (Body, FaceId) {
        let (b, ids) = Body::from_polygons(
            &[
                Point3::ORIGIN,
                Point3::new(w, 0.0, 0.0),
                Point3::new(w, d, 0.0),
                Point3::new(0.0, d, 0.0),
            ],
            &[vec![0, 1, 2, 3]],
        )
        .unwrap();
        (b, ids[0])
    }

    #[test]
    fn rectangle_pulled_up_becomes_box_keeping_face_id() {
        let (mut b, f) = rect_sheet(4000.0, 3000.0);
        let (mode, lin) = push_pull_face(&mut b, f, 2800.0).unwrap();
        assert_eq!(mode, PushPullMode::Extrude);
        b.validate_solid().unwrap();
        assert_eq!(b.face_count(), 6);
        assert!((b.volume() - 4000.0 * 3000.0 * 2800.0).abs() < 1.0);
        assert_eq!(lin.kept, vec![f]);
        assert_eq!(lin.created.len(), 5);
        // The kept face is now the top.
        assert!(b
            .face_points(f)
            .unwrap()
            .iter()
            .all(|p| (p.z - 2800.0).abs() < 1e-9));
    }

    #[test]
    fn rectangle_pushed_down_becomes_box_below() {
        let (mut b, f) = rect_sheet(1000.0, 1000.0);
        push_pull_face(&mut b, f, -500.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - 1000.0 * 1000.0 * 500.0).abs() < 1.0);
        assert!(b.face_normal(f).unwrap().as_vec().is_almost_equal(-Vec3::Z));
    }

    #[test]
    fn box_top_stretches_both_ways() {
        let (mut b, ids) = box_body(1000.0, 1000.0, 1000.0);
        let top = ids[1];
        let (mode, lin) = push_pull_face(&mut b, top, 500.0).unwrap();
        assert_eq!(mode, PushPullMode::Stretch);
        assert_eq!(b.face_count(), 6);
        assert!(lin.created.is_empty());
        assert!((b.volume() - 1.5e9).abs() < 1.0);
        push_pull_face(&mut b, top, -1200.0).unwrap();
        assert!((b.volume() - 0.3e9).abs() < 1.0);
    }

    #[test]
    fn pushing_through_the_box_collapses_and_leaves_body_unchanged() {
        let (mut b, ids) = box_body(1000.0, 1000.0, 1000.0);
        let before = b.clone();
        assert_eq!(
            push_pull_face(&mut b, ids[1], -1000.0).map(|_| ()),
            Err(AlgoError::Collapse)
        );
        assert_eq!(b, before);
        assert_eq!(
            push_pull_face(&mut b, ids[1], -1500.0).map(|_| ()),
            Err(AlgoError::Collapse)
        );
    }

    #[test]
    fn half_of_a_split_top_pulls_up_and_down() {
        use crate::algo::imprint_face;
        let (mut b, ids) = box_body(4000.0, 3000.0, 1000.0);
        let top = ids[1];
        let chord = [
            Point3::new(1500.0, 0.0, 1000.0),
            Point3::new(1500.0, 3000.0, 1000.0),
        ];
        let (imp, _) = imprint_face(&mut b, top, &chord, false).unwrap();
        let small = imp.faces[1]; // the 1500-wide half
        let (mode, _) = push_pull_face(&mut b, small, 500.0).unwrap();
        assert_eq!(mode, PushPullMode::Prism);
        b.validate_solid().unwrap();
        let v = 4000.0 * 3000.0 * 1000.0;
        assert!((b.volume() - (v + 1500.0 * 3000.0 * 500.0)).abs() < 1.0);
        // Back down past the original top: a step down.
        push_pull_face(&mut b, small, -800.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - (v - 1500.0 * 3000.0 * 300.0)).abs() < 1.0);
        // The side faces were merged, not added: same face count as a step.
        assert_eq!(b.face_count(), 8);
        // The old top corners are gone (no spike left in the side faces).
        assert!(b
            .vertices()
            .all(|(_, p)| p.x > 1.0 || (p.z - 1000.0).abs() > 1.0));
        for f in b.face_ids() {
            let pts = b.face_points(f).unwrap();
            assert!(is_simple(&pts, b.face_normal(f).unwrap()), "{f}");
        }
        // Too deep collapses and leaves the body alone.
        let before = b.clone();
        assert!(push_pull_face(&mut b, small, -800.0).is_err());
        assert_eq!(b, before);
    }

    #[test]
    fn pocket_and_boss_on_an_imprinted_rectangle() {
        use crate::algo::imprint_face;
        let (mut b, ids) = box_body(4000.0, 3000.0, 1000.0);
        let r = [
            Point3::new(1000.0, 1000.0, 1000.0),
            Point3::new(2000.0, 1000.0, 1000.0),
            Point3::new(2000.0, 2000.0, 1000.0),
            Point3::new(1000.0, 2000.0, 1000.0),
        ];
        let (imp, _) = imprint_face(&mut b, ids[1], &r, true).unwrap();
        let f = imp.new_faces[0];
        push_pull_face(&mut b, f, -300.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - (1.2e10 - 3e8)).abs() < 1.0);
        // The pocket floor moves back up (walls stretch).
        push_pull_face(&mut b, f, 100.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - (1.2e10 - 2e8)).abs() < 1.0);
        // A boss on a second rectangle.
        let big = imp.faces[0];
        let r2 = [
            Point3::new(2500.0, 500.0, 1000.0),
            Point3::new(3500.0, 500.0, 1000.0),
            Point3::new(3500.0, 2500.0, 1000.0),
            Point3::new(2500.0, 2500.0, 1000.0),
        ];
        let (imp2, _) = imprint_face(&mut b, big, &r2, true).unwrap();
        push_pull_face(&mut b, imp2.new_faces[0], 1000.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - (1.2e10 - 2e8 + 2e9)).abs() < 1.0);
    }

    fn pocket(depth: f64) -> (Body, FaceId, FaceId) {
        use crate::algo::imprint_face;
        let (mut b, ids) = box_body(4000.0, 3000.0, 2000.0);
        let r = [
            Point3::new(1000.0, 1000.0, 2000.0),
            Point3::new(2000.0, 1000.0, 2000.0),
            Point3::new(2000.0, 2000.0, 2000.0),
            Point3::new(1000.0, 2000.0, 2000.0),
        ];
        let (imp, _) = imprint_face(&mut b, ids[1], &r, true).unwrap();
        let f = imp.new_faces[0];
        push_pull_face(&mut b, f, -depth).unwrap();
        (b, ids[1], f)
    }

    #[test]
    fn pocket_pulled_back_flush_heals_the_top() {
        let v = 4000.0 * 3000.0 * 2000.0;
        // Tolerance edge: exactly flush and 0.005 mm short or past.
        for d in [800.0, 799.995, 800.005] {
            let (mut b, top, f) = pocket(800.0);
            let (mode, lin) = push_pull_face(&mut b, f, d).unwrap();
            assert_eq!(mode, PushPullMode::Stretch);
            b.validate_solid().unwrap();
            assert!((b.volume() - v).abs() < 1.0, "{d}");
            assert_eq!(b.face_count(), 6, "{d}");
            assert!(b.face(top).unwrap().holes.is_empty());
            assert!(b.face(f).is_err());
            // The pocket floor and its four walls are gone; the top is kept.
            assert_eq!(lin.deleted.len(), 5);
            assert!(lin.deleted.contains(&f) && lin.kept.contains(&top));
            // The square's corners went with the hole.
            assert_eq!(b.vertex_count(), 8);
        }
    }

    #[test]
    fn pocket_pulled_past_flush_becomes_a_boss() {
        let (mut b, top, f) = pocket(800.0);
        let (_, lin) = push_pull_face(&mut b, f, 1000.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - (4000.0 * 3000.0 * 2000.0 + 1e6 * 200.0)).abs() < 1.0);
        assert!(lin.deleted.is_empty() && lin.kept.contains(&f));
        assert_eq!(b.face(top).unwrap().holes.len(), 1);
        // 0.02 mm short of flush: a 0.02 mm pocket stays (the transaction
        // then refuses its sub-0.5 mm edges).
        let (mut b, _, f) = pocket(800.0);
        let (_, lin) = push_pull_face(&mut b, f, 799.98).unwrap();
        assert!(lin.deleted.is_empty());
        assert_eq!(b.face_count(), 11);
    }

    #[test]
    fn step_pulled_back_flush_restores_the_split_top() {
        use crate::algo::imprint_face;
        let (mut b, ids) = box_body(4000.0, 3000.0, 1000.0);
        let chord = [
            Point3::new(1500.0, 0.0, 1000.0),
            Point3::new(1500.0, 3000.0, 1000.0),
        ];
        let (imp, _) = imprint_face(&mut b, ids[1], &chord, false).unwrap();
        let small = imp.faces[1];
        push_pull_face(&mut b, small, -400.0).unwrap();
        assert_eq!(b.face_count(), 8);
        let (_, lin) = push_pull_face(&mut b, small, 400.0).unwrap();
        b.validate_solid().unwrap();
        assert!((b.volume() - 4000.0 * 3000.0 * 1000.0).abs() < 1.0);
        // Back to the imprinted box: two top halves and the step wall gone.
        assert_eq!(b.face_count(), 7);
        assert_eq!(lin.deleted.len(), 1);
        assert!(b.face(small).is_ok());
    }

    #[test]
    fn pushing_a_rectangle_through_makes_a_hole() {
        use crate::algo::imprint_face;
        let (mut b, ids) = box_body(4000.0, 3000.0, 300.0);
        let r = [
            Point3::new(1000.0, 1000.0, 300.0),
            Point3::new(2000.0, 1000.0, 300.0),
            Point3::new(2000.0, 2000.0, 300.0),
            Point3::new(1000.0, 2000.0, 300.0),
        ];
        let (imp, _) = imprint_face(&mut b, ids[1], &r, true).unwrap();
        let f = imp.new_faces[0];
        let (mode, lin) = push_pull_face(&mut b, f, -300.0).unwrap();
        assert_eq!(mode, PushPullMode::Through);
        assert!(lin.deleted.contains(&f));
        b.validate_solid().unwrap();
        assert!((b.volume() - (4000.0 * 3000.0 - 1e6) * 300.0).abs() < 1.0);
        // 6 box faces + 4 hole walls (top and bottom have holes).
        assert_eq!(b.face_count(), 10);
        // Not quite through (tolerance edge): an ordinary pocket.
        let (mut b2, ids2) = box_body(4000.0, 3000.0, 300.0);
        let (imp2, _) = imprint_face(&mut b2, ids2[1], &r, true).unwrap();
        let (mode2, _) = push_pull_face(&mut b2, imp2.new_faces[0], -299.0).unwrap();
        assert_eq!(mode2, PushPullMode::Prism);
    }

    #[test]
    fn tolerance_edge_distance() {
        let (mut b, f) = rect_sheet(1000.0, 1000.0);
        assert_eq!(
            push_pull_face(&mut b, f, tol::SHORT_CURVE_TOL * 0.9).map(|_| ()),
            Err(AlgoError::ZeroDistance)
        );
        assert!(push_pull_face(&mut b, f, tol::SHORT_CURVE_TOL).is_ok());
    }

    #[test]
    fn prism_on_a_tilted_face() {
        // A wedge: pulling its sloped face adds a prism (no perpendicular neighbours).
        let p = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1000.0, 0.0, 0.0),
            Point3::new(1000.0, 1000.0, 0.0),
            Point3::new(0.0, 1000.0, 0.0),
            Point3::new(0.0, 0.0, 1000.0),
            Point3::new(0.0, 1000.0, 1000.0),
        ];
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![1, 2, 5, 4], // slope
            vec![0, 1, 4],
            vec![3, 5, 2],
            vec![0, 4, 5, 3],
        ];
        let (mut b, ids) = Body::from_polygons(&p, &faces).unwrap();
        b.validate_solid().unwrap();
        let v0 = b.volume();
        // Slope's neighbours: bottom (not ⟂), two triangles (⟂ to slope? no) …
        let r = push_pull_face(&mut b, ids[1], 200.0);
        match r {
            Ok((mode, lin)) => {
                assert_eq!(mode, PushPullMode::Prism);
                assert!(b.volume() > v0);
                // The two triangle sides are perpendicular: their prism sides
                // merge into them; bottom and top sides are new.
                assert_eq!(lin.created.len(), 2);
            }
            Err(AlgoError::Unsupported(_)) => {} // mixed neighbours: acceptable in M1
            Err(e) => panic!("{e}"),
        }
    }
}
