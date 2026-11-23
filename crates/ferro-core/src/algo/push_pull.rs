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
/// * Closed solid, no neighbour perpendicular → a prism (boss or pocket) is added.
/// * Mixed neighbours (e.g. half of a split top face) → a prism whose side
///   faces that lie in the plane of a perpendicular neighbour are merged into
///   it, so that neighbour grows or shrinks (SketchUp behaviour).
///
/// The result is validated; collapse or inversion returns `Collapse` and the
/// body is left unchanged.
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
            work.validate_solid().map_err(|_| AlgoError::Collapse)?;
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
            // A neighbour may legitimately turn around (a step pulled past its
            // neighbour flips), but it must not vanish or cross itself; the
            // solid check below catches a turned-inside-out body.
            for nf in neighbors.iter().flatten() {
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
                    ..Default::default()
                },
            )
        } else if flags.iter().all(|x| !*x) {
            prism(&mut work, face, &loops, offset, false)?
        } else {
            prism(&mut work, face, &loops, offset, true)?
        }
    };

    work.validate_solid().map_err(|_| AlgoError::Collapse)?;
    *body = work;
    Ok(result)
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

/// `true` if the closed polygon has no two non-adjacent edges that touch.
fn is_simple(pts: &[Point3], normal: crate::math::UnitVec3) -> bool {
    let plane = crate::geom::Plane::from_origin_normal(pts[0], normal);
    let p: Vec<crate::math::Point2> = pts.iter().map(|q| plane.to_local(*q)).collect();
    let n = p.len();
    let cross = |a: crate::math::Point2, b: crate::math::Point2, c: crate::math::Point2| {
        (b - a).perp_dot(c - a)
    };
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (c, d) = (p[j], p[(j + 1) % n]);
            let d1 = cross(a, b, c);
            let d2 = cross(a, b, d);
            let d3 = cross(c, d, a);
            let d4 = cross(c, d, b);
            if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
                return false;
            }
        }
        // Repeated vertices also break simplicity.
        if p.iter()
            .skip(i + 1)
            .any(|q| q.distance_to(p[i]) <= tol::VERTEX_TOL)
        {
            return false;
        }
    }
    true
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
    if heal {
        for sp in &specs {
            let pts: Vec<Point3> = sp
                .outer
                .iter()
                .map(|v| work.point(*v).expect("v"))
                .collect();
            let n = crate::geom::polygon::polygon_normal(&pts).ok_or(AlgoError::Collapse)?;
            if !is_simple(&pts, n) {
                return Err(AlgoError::Collapse);
            }
        }
    }
    let mut cap = FaceSpec::new(cap_loops[0].clone());
    cap.id = Some(face);
    cap.holes = cap_loops[1..].to_vec();
    let side_start = specs.len() + 1;
    specs.push(cap);
    specs.extend(sides);
    let verts = work.vertex_map();
    let ids = work.rebuild(verts, specs)?;
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
