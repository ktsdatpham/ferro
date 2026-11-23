//! Edges to draw for a body.

use ferro_core::topo::Body;
use ferro_core::Point3;
use std::collections::HashMap;

/// Bit pattern of an unordered segment (exact endpoint match).
type SegBits = (u64, u64, u64, u64, u64, u64);

fn seg_bits(a: Point3, b: Point3) -> SegBits {
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
}

/// Edges to draw (model coordinates, mm): every edge of the body except one
/// shared by exactly two faces of the same smooth group (facets of one
/// curved surface). Loose edges and edges of open bodies are kept. Output
/// follows the body's edge order (deterministic).
pub fn visible_edges(body: &Body) -> Vec<(Point3, Point3)> {
    // Lookup only; the output order comes from `body.edges()`.
    let mut groups: HashMap<SegBits, Vec<Option<u32>>> = HashMap::new();
    for fid in body.face_ids() {
        let Ok(face) = body.face(fid) else { continue };
        let g = face.smooth_group;
        for l in std::iter::once(&face.outer).chain(&face.holes) {
            let pts = body.loop_points(*l);
            for i in 0..pts.len() {
                groups
                    .entry(seg_bits(pts[i], pts[(i + 1) % pts.len()]))
                    .or_default()
                    .push(g);
            }
        }
    }
    body.edges()
        .filter(|(_, a, b)| {
            let gs = groups.get(&seg_bits(*a, *b));
            !matches!(gs.map(|v| v.as_slice()), Some([Some(x), Some(y)]) if x == y)
        })
        .map(|(_, a, b)| (a, b))
        .collect()
}
