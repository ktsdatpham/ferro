//! Ferro `Body` ↔ solidcore B-rep record (schema 4, STEP names) for planar
//! bodies: every face a plane, every edge a straight segment. The record is
//! written in its canonical order (regions → shells → faces → edges →
//! vertices by first appearance), so the kernel takes it as it is.

use ferro_core::algo::KernelError;
use ferro_core::geom::{polygon, Plane};
use ferro_core::math::{tol, Point3, Vec3};
use ferro_core::topo::{Body, FaceId, VertexId};
use solidcore_record::brep::{
    Brep, Curve, Curve2, Edge, EdgeUseRef, Face, Loop, OrientedEdge, OrientedFace, Region, Shell,
    Surface, Vertex, VERSION,
};
use std::collections::{BTreeMap, HashMap};

/// The tolerance solidcore gives its own primitives (mm); the record's
/// vertices and edges get at least this, more where a Ferro face is flat
/// only within `VERTEX_TOL`.
const KERNEL_TOL: f64 = 2e-5;

fn arr(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn pt(a: [f64; 3]) -> Point3 {
    Point3::new(a[0], a[1], a[2])
}

fn index(i: usize) -> Result<u32, KernelError> {
    u32::try_from(i).map_err(|_| KernelError::new(KernelError::UNSUPPORTED, "the body is too large"))
}

fn not_solid(e: impl std::fmt::Display) -> KernelError {
    KernelError::new(KernelError::NOT_SOLID, format!("the operand is not a valid solid: {e}"))
}

/// A record of `body` and, for each record face, the Ferro face it is.
pub(crate) struct Record {
    pub(crate) brep: Brep,
    pub(crate) faces: Vec<FaceId>,
}

/// Faces of `body` grouped into connected shells (by shared edges), each
/// sorted by id, shells ordered by their smallest face id.
fn shells(loops: &BTreeMap<FaceId, Vec<Vec<VertexId>>>) -> Vec<Vec<FaceId>> {
    let ids: Vec<FaceId> = loops.keys().copied().collect();
    let pos: HashMap<FaceId, usize> = ids.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut by_edge: HashMap<(VertexId, VertexId), usize> = HashMap::new();
    for (f, ls) in loops {
        for l in ls {
            for k in 0..l.len() {
                let (a, b) = (l[k], l[(k + 1) % l.len()]);
                let key = if a < b { (a, b) } else { (b, a) };
                let me = pos[f];
                match by_edge.get(&key) {
                    Some(&other) => {
                        let (ra, rb) = (find(&mut parent, me), find(&mut parent, other));
                        if ra != rb {
                            parent[ra.max(rb)] = ra.min(rb);
                        }
                    }
                    None => {
                        by_edge.insert(key, me);
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    for (i, f) in ids.iter().enumerate() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(*f);
    }
    groups.into_values().collect()
}

/// `body` as a solidcore record.
///
/// # Errors
/// `ferro.kernel.not_solid` when `body` fails `validate_solid`;
/// `ferro.kernel.unsupported` for a face that is not flat within
/// `VERTEX_TOL` (curved) or a body with an inner void shell.
pub(crate) fn body_to_record(body: &Body) -> Result<Record, KernelError> {
    body.validate_solid().map_err(not_solid)?;
    let mut loops: BTreeMap<FaceId, Vec<Vec<VertexId>>> = BTreeMap::new();
    for f in body.face_ids() {
        let face = body.face(f).map_err(not_solid)?;
        let ls = std::iter::once(&face.outer)
            .chain(&face.holes)
            .map(|l| body.loop_vertices(*l))
            .collect();
        loops.insert(f, ls);
    }
    let point = |v: VertexId| body.point(v).ok_or_else(|| not_solid(format!("vertex {v} missing")));

    // Each face's plane: through the outer loop's centroid, Newell normal.
    let mut planes: BTreeMap<FaceId, Plane> = BTreeMap::new();
    let mut slack: BTreeMap<VertexId, f64> = BTreeMap::new();
    for (f, ls) in &loops {
        let outer: Vec<Point3> = ls[0].iter().map(|v| point(*v)).collect::<Result<_, _>>()?;
        let n = polygon::polygon_normal(&outer).ok_or_else(|| not_solid(format!("face {f} is degenerate")))?;
        let c = Point3::centroid(&outer).ok_or_else(|| not_solid(format!("face {f} is empty")))?;
        let pl = Plane::from_origin_normal(c, n);
        for v in ls.iter().flatten() {
            let d = pl.signed_distance_to(point(*v)?).abs();
            if d > tol::VERTEX_TOL {
                return Err(KernelError::new(
                    KernelError::UNSUPPORTED,
                    format!("face {f} is curved; the kernel seam carries flat faces only"),
                ));
            }
            let s = slack.entry(*v).or_insert(0.0);
            *s = s.max(d);
        }
        planes.insert(*f, pl);
    }
    let vtol = |v: VertexId| KERNEL_TOL.max(slack.get(&v).copied().unwrap_or(0.0) * 1.5);

    let shells_f = shells(&loops);
    // A shell whose own volume is negative is an inner void: not carried yet.
    for s in &shells_f {
        let mut vol = 0.0;
        for f in s {
            let a = body.face_area_vector(*f).map_err(not_solid)?;
            let p = point(loops[f][0][0])?;
            vol += p.to_vec().dot(a) / 3.0;
        }
        if vol <= 0.0 {
            return Err(KernelError::new(
                KernelError::UNSUPPORTED,
                "bodies with inner voids are not supported by the kernel seam yet",
            ));
        }
    }

    let mut faces_out: Vec<Face> = Vec::new();
    let mut face_ids: Vec<FaceId> = Vec::new();
    // Edge by undirected vertex pair: (record index, start vertex, end vertex).
    let mut edge_of: HashMap<(VertexId, VertexId), usize> = HashMap::new();
    let mut edges: Vec<(VertexId, VertexId, Vec<EdgeUseRef>)> = Vec::new();
    let mut shells_out: Vec<Shell> = Vec::new();
    for s in &shells_f {
        let mut oriented = Vec::with_capacity(s.len());
        for f in s {
            let fi = faces_out.len();
            let pl = planes[f];
            let (o, x, y) = (pl.origin(), pl.x_axis().as_vec(), pl.y_axis().as_vec());
            let uv = |p: Point3| -> [f64; 2] {
                let d: Vec3 = p - o;
                [d.dot(x), d.dot(y)]
            };
            let mut dom = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
            let mut rec_loops = Vec::new();
            for (li, l) in loops[f].iter().enumerate() {
                let mut uses = Vec::with_capacity(l.len());
                for k in 0..l.len() {
                    let (a, b) = (l[k], l[(k + 1) % l.len()]);
                    let key = if a < b { (a, b) } else { (b, a) };
                    let e = match edge_of.get(&key) {
                        Some(e) => *e,
                        None => {
                            edges.push((a, b, Vec::new()));
                            edge_of.insert(key, edges.len() - 1);
                            edges.len() - 1
                        }
                    };
                    let (start, end) = (edges[e].0, edges[e].1);
                    let forward = start == a;
                    let (ps, pe) = (uv(point(start)?), uv(point(end)?));
                    for q in [ps, pe] {
                        for (c, dd) in dom.iter_mut().enumerate() {
                            dd[0] = dd[0].min(q[c]);
                            dd[1] = dd[1].max(q[c]);
                        }
                    }
                    edges[e].2.push(EdgeUseRef {
                        face: index(fi)?,
                        r#loop: index(li)?,
                        position: index(k)?,
                        upper: !forward,
                    });
                    uses.push(OrientedEdge {
                        edge: index(e)?,
                        forward,
                        pcurve: Curve2::Spline {
                            knots: vec![0.0, 0.0, 1.0, 1.0],
                            points: vec![ps, pe],
                        },
                    });
                }
                rec_loops.push(Loop::Edges { edges: uses });
            }
            faces_out.push(Face {
                surface: Surface::Plane {
                    origin: arr(o),
                    u_axis: [x.x, x.y, x.z],
                    v_axis: [y.x, y.y, y.z],
                },
                nurbs: None,
                domain: dom,
                loops: rec_loops,
            });
            face_ids.push(*f);
            oriented.push(OrientedFace {
                face: index(fi)?,
                forward: true,
            });
        }
        shells_out.push(Shell::Faces {
            faces: oriented,
            wires: Vec::new(),
        });
    }

    // Vertices by first appearance walking edges (start, end).
    let mut vi: HashMap<VertexId, u32> = HashMap::new();
    let mut verts: Vec<Vertex> = Vec::new();
    for (a, b, _) in &edges {
        for v in [*a, *b] {
            if !vi.contains_key(&v) {
                vi.insert(v, index(verts.len())?);
                verts.push(Vertex {
                    point: arr(point(v)?),
                    tolerance: vtol(v),
                });
            }
        }
    }
    let mut rec_edges = Vec::with_capacity(edges.len());
    for (a, b, uses) in edges {
        rec_edges.push(Edge {
            curve: Curve::Line {
                start: arr(point(a)?),
                end: arr(point(b)?),
            },
            nurbs: None,
            interval: [0.0, 1.0],
            vertices: [vi[&a], vi[&b]],
            tolerance: vtol(a).max(vtol(b)),
            uses,
        });
    }
    let n = shells_out.len();
    let mut regions = vec![Region {
        shells: (0..n).map(index).collect::<Result<_, _>>()?,
        void: false,
    }];
    for s in 0..n {
        regions.push(Region {
            shells: vec![index(s)?],
            void: false,
        });
    }
    Ok(Record {
        brep: Brep {
            version: VERSION,
            tolerance: KERNEL_TOL,
            regions,
            shells: shells_out,
            faces: faces_out,
            edges: rec_edges,
            vertices: verts,
        },
        faces: face_ids,
    })
}

fn bad(what: &str) -> KernelError {
    KernelError::new(
        KernelError::INVALID_RESULT,
        format!("the kernel's result record is inconsistent ({what})"),
    )
}

/// The loops of every face of a planar result record (outer loop first,
/// counter-clockwise about the outward normal; holes clockwise), in record
/// order.
///
/// # Errors
/// `ferro.kernel.unsupported` for a curved face or edge, or a face used on
/// both sides (a sheet); `ferro.kernel.invalid_result` for indices out of
/// range or a face without an outer loop.
pub(crate) fn record_faces(rec: &Brep) -> Result<Vec<Vec<Vec<Point3>>>, KernelError> {
    let mut side: Vec<Option<bool>> = vec![None; rec.faces.len()];
    for s in &rec.shells {
        if let Shell::Faces { faces, .. } = s {
            for of in faces {
                let slot = side.get_mut(of.face as usize).ok_or_else(|| bad("shell face"))?;
                if slot.is_some() {
                    return Err(KernelError::new(
                        KernelError::UNSUPPORTED,
                        "the result has a sheet face (used on both sides)",
                    ));
                }
                *slot = Some(of.forward);
            }
        }
    }
    let mut out = Vec::with_capacity(rec.faces.len());
    for (fi, face) in rec.faces.iter().enumerate() {
        let natural = match &face.surface {
            Surface::Plane { u_axis, v_axis, .. } => pt(*u_axis).to_vec().cross(pt(*v_axis).to_vec()),
            Surface::PlaneThrough {
                origin,
                u_point,
                v_point,
            } => (pt(*u_point) - pt(*origin)).cross(pt(*v_point) - pt(*origin)),
            _ => {
                return Err(KernelError::new(
                    KernelError::UNSUPPORTED,
                    "the result has a curved face; the kernel seam carries flat faces only",
                ))
            }
        };
        let forward = side
            .get(fi)
            .copied()
            .flatten()
            .ok_or_else(|| bad("face in no shell"))?;
        let normal = if forward { natural } else { natural * -1.0 };
        let mut loops: Vec<Vec<Point3>> = Vec::new();
        for l in &face.loops {
            let Loop::Edges { edges } = l else { continue };
            let mut pts = Vec::with_capacity(edges.len());
            for u in edges {
                let e = rec.edges.get(u.edge as usize).ok_or_else(|| bad("edge"))?;
                if !matches!(e.curve, Curve::Line { .. }) {
                    return Err(KernelError::new(
                        KernelError::UNSUPPORTED,
                        "the result has a curved edge; the kernel seam carries straight edges only",
                    ));
                }
                let v = if u.forward { e.vertices[0] } else { e.vertices[1] };
                let p = rec.vertices.get(v as usize).ok_or_else(|| bad("vertex"))?;
                pts.push(pt(p.point));
            }
            if !forward {
                pts.reverse();
            }
            loops.push(pts);
        }
        // The outer loop runs counter-clockwise about the outward normal.
        let area = |l: &Vec<Point3>| -> f64 {
            let Some(&o) = l.first() else { return 0.0 };
            let mut a = Vec3::ZERO;
            for i in 1..l.len().saturating_sub(1) {
                a += (l[i] - o).cross(l[i + 1] - o);
            }
            a.dot(normal)
        };
        let outer = loops
            .iter()
            .enumerate()
            .filter(|(_, l)| area(l) > 0.0)
            .max_by(|x, y| area(x.1).total_cmp(&area(y.1)))
            .map(|(i, _)| i)
            .ok_or_else(|| bad("face without an outer loop"))?;
        let o = loops.remove(outer);
        loops.insert(0, o);
        out.push(loops);
    }
    Ok(out)
}
