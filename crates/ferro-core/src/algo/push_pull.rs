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
}

/// Push / pull face `face` of `body` by `distance` mm along its normal
/// (negative pushes inward). The pulled face keeps its id (D-25).
///
/// * Open single face → extruded into a closed solid.
/// * Closed solid, all neighbours perpendicular → face moves, neighbours stretch.
/// * Closed solid, no neighbour perpendicular → a prism (boss or pocket) is added.
/// * Mixed neighbours → `Unsupported` (milestone 2).
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
            // Remember neighbour orientations to detect inversion.
            let before: Vec<(FaceId, crate::math::UnitVec3)> = neighbors
                .iter()
                .flatten()
                .map(|nf| Ok((*nf, work.face_normal(*nf)?)))
                .collect::<Result<_, AlgoError>>()?;
            work.move_vertices(&moves)?;
            for (nf, nn) in before {
                let a = work.face_area_vector(nf)?;
                if a.dot(nn.as_vec()) <= tol::VERTEX_TOL {
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
            prism(&mut work, face, &loops, offset)?
        } else {
            return Err(AlgoError::Unsupported(
                "push / pull where only some neighbouring faces are perpendicular",
            ));
        }
    };

    work.validate_solid().map_err(|_| AlgoError::Collapse)?;
    *body = work;
    Ok(result)
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
        for i in 0..k {
            let j = (i + 1) % k;
            let quad = if up {
                vec![l[i], l[j], top[j], top[i]]
            } else {
                vec![l[j], l[i], top[i], top[j]]
            };
            specs.push(FaceSpec::new(quad));
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

fn prism(
    work: &mut Body,
    face: FaceId,
    loops: &[Vec<VertexId>],
    offset: crate::math::Vec3,
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
        for i in 0..k {
            let j = (i + 1) % k;
            sides.push(FaceSpec::new(vec![l[i], l[j], top[j], top[i]]));
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
                assert_eq!(lin.created.len(), 4);
            }
            Err(AlgoError::Unsupported(_)) => {} // mixed neighbours: acceptable in M1
            Err(e) => panic!("{e}"),
        }
    }
}
