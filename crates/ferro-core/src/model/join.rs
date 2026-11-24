//! Joins (ADR 0004 §5, M5-5.2; Foundation §3.8): wall-end joins and
//! geometry joins.
//!
//! **Wall-end joins** stay automatic, like Revit's "Allow Join": straight
//! walls whose location ends meet (within [`JOIN_TOL`]) are mitred (L), a
//! wall end on the middle of another wall stops at its face (T), and three
//! or more ends at one point stay square (ADR 0001, 0002). Each wall end
//! has a join state ([`WallJoin`]): a `disallow` end stays square and is
//! not seen by the other walls (Revit `DisallowWallJoinAtEnd`). A `join`
//! row between two walls whose ends meet turns their mitre into a **butt**
//! join: `first` runs through to the far face of `second`, `second` stops
//! at the near face of `first` (Revit: the join order of a wall corner).
//!
//! **Geometry joins** (Revit: Join Geometry): a row `join [first, second]`
//! makes the overlap belong to `first`: `second`'s body becomes `second` −
//! `first` through the injected [`Kernel`] at regeneration, like a cut row
//! but without hiding `first`. Walls, floors and Forms take part. A wall
//! pair already settled by its ends (L mitre or butt, T whose stem loses)
//! needs no boolean. `first` contributes its unjoined, uncut body (joins do
//! not chain). Joins apply before cuts. Without a kernel, or on a kernel
//! error, `second` keeps its unjoined body and regeneration warns (never an
//! error), as for cuts.
//!
//! **Frozen walls.** A wall frozen into a Form (`set_geometry_source
//! direct`) keeps its id, so its rows stay; freezing turns its inferred
//! joins into `join` rows with the frozen wall first. A parametric wall end
//! that touches a frozen wall (a Form of category `Walls`) it has a `join`
//! row with is extended square to the far side of that Form, and the row
//! then trims it through the kernel: an L stays mitred, a T stem stops at
//! the face, exactly as before freezing.

use super::bim::WallJoin;
use super::cut::CutOutcome;
use super::generate::{GenIssue, JOIN_TOL};
use super::{Category, Document, Element, ElementBody, ElementId, Relationship, RelationshipKind};
use crate::algo::{BooleanOp, Kernel, KernelError};
use crate::math::{tol, Point2, Vec2};
use crate::topo::Body;
use std::collections::{BTreeSet, HashMap};

/// `true` when `e` can take part in a `join` row: walls, floors and Forms
/// of any category (Revit: Join Geometry takes walls, floors, columns,
/// beams and in-place elements; openings join through their host).
pub fn is_join_element(e: &Element) -> bool {
    matches!(
        e.body,
        ElementBody::Wall(_) | ElementBody::Floor(_) | ElementBody::Form(_)
    )
}

/// The `join` row linking `a` and `b` in either order, if any (one row per
/// pair, `Relationship::same_link`).
pub fn join_between(doc: &Document, a: ElementId, b: ElementId) -> Option<&Relationship> {
    doc.relationships_of(a)
        .find(|r| r.kind == RelationshipKind::Join && r.involves(b) && r.elements.len() == 2)
}

/// The `join` rows whose `second` is `id` (the rows that trim it), in uid
/// order.
pub fn joins_of(doc: &Document, id: ElementId) -> Vec<&Relationship> {
    let mut v: Vec<&Relationship> = doc
        .relationships_of(id)
        .filter(|r| r.kind == RelationshipKind::Join && r.elements.get(1) == Some(&id))
        .collect();
    v.sort_by_key(|r| r.uid);
    v
}

/// The join state of the end of wall `e` (`at_start` = the curve start);
/// `Auto` for anything that is not a wall.
pub fn wall_join_at(e: &Element, at_start: bool) -> WallJoin {
    match e.as_wall() {
        Some(w) if at_start => w.join_start,
        Some(w) => w.join_end,
        None => WallJoin::Auto,
    }
}

// ------------------------------------------------------------ wall ends

/// Corner points of a straight wall end in plan: (exterior corner, interior
/// corner).
pub(crate) type EndCorners = (Point2, Point2);

/// A straight wall reduced to what end joins need (plan, mm).
#[derive(Debug, Clone, Copy)]
pub(crate) struct StraightWall {
    pub id: ElementId,
    pub s: Point2,
    pub e: Point2,
    /// Unit direction from `s` to `e`.
    pub d: Vec2,
    /// Offsets of the exterior / interior faces along the left normal.
    pub ext: f64,
    pub int: f64,
    /// Bottom and top elevations.
    pub z0: f64,
    pub z1: f64,
    /// `disallow` at the start / the end.
    pub disallow: [bool; 2],
}

/// End corners of every straight wall, and the wall pairs whose overlap
/// the end shapes already settle (no geometry join needed for their row).
#[derive(Debug, Default)]
pub(crate) struct WallEnds {
    pub corners: HashMap<ElementId, [Option<EndCorners>; 2]>,
    /// The wall each end is joined to (L or T), when it is a wall.
    pub partners: HashMap<ElementId, [Option<ElementId>; 2]>,
    pub settled: BTreeSet<(ElementId, ElementId)>,
}

fn pair(a: ElementId, b: ElementId) -> (ElementId, ElementId) {
    (a.min(b), a.max(b))
}

fn intersect(p1: Point2, d1: Vec2, p2: Point2, d2: Vec2) -> Option<Point2> {
    let cross = d1.perp_dot(d2);
    if cross.abs() < 1e-3 {
        return None; // parallel within ~0.06°
    }
    let t = (p2 - p1).perp_dot(d2) / cross;
    Some(p1 + d1 * t)
}

/// Mitred corners for wall `x` at its end `x_at_start` joined to wall `y`.
fn miter(
    x: &StraightWall,
    x_at_start: bool,
    y: &StraightWall,
    y_at_start: bool,
) -> Option<EndCorners> {
    let j = if x_at_start { x.s } else { x.e };
    let out = |w: &StraightWall, at_start: bool| -> (Vec2, f64, f64) {
        // Outgoing direction and (left, right) offsets along its left normal.
        if at_start {
            (w.d, w.ext, w.int)
        } else {
            (w.d * -1.0, -w.int, -w.ext)
        }
    };
    let (ox, lx, rx) = out(x, x_at_start);
    let (oy, ly, ry) = out(y, y_at_start);
    let perp = |o: Vec2| Vec2::new(-o.y, o.x);
    let ql = intersect(j + perp(ox) * lx, ox, j + perp(oy) * ry, oy)?;
    let qr = intersect(j + perp(ox) * rx, ox, j + perp(oy) * ly, oy)?;
    let limit = 5.0 * (x.ext - x.int).max(y.ext - y.int);
    if (ql - j).length() > limit || (qr - j).length() > limit {
        return None;
    }
    // left(ox) is the exterior face at a start, the interior face at an end.
    Some(if x_at_start { (ql, qr) } else { (qr, ql) })
}

/// Corners for wall `x` whose end butts against wall `y`: `x`'s faces stop
/// at the face of `y` that looks towards the rest of `x` (`far = false`),
/// or run through to `y`'s other face (`far = true`). `None` when parallel
/// or the corners are more than 5 × the thicker wall away.
fn butt(x: &StraightWall, x_at_start: bool, y: &StraightWall, far: bool) -> Option<EndCorners> {
    let j = if x_at_start { x.s } else { x.e };
    let other = if x_at_start { x.e } else { x.s };
    if x.d.perp_dot(y.d).abs() < 1e-3 {
        return None; // parallel: no butt
    }
    let ny = Vec2::new(-y.d.y, y.d.x);
    let side = (other - y.s).dot(ny);
    let near_is_ext = side >= 0.0;
    let face = if near_is_ext != far { y.ext } else { y.int };
    let face_pt = y.s + ny * face;
    let nx = Vec2::new(-x.d.y, x.d.x);
    let pe = intersect(x.s + nx * x.ext, x.d, face_pt, y.d)?;
    let pi = intersect(x.s + nx * x.int, x.d, face_pt, y.d)?;
    let limit = 5.0 * (x.ext - x.int).max(y.ext - y.int);
    if (pe - j).length() > limit || (pi - j).length() > limit {
        return None;
    }
    Some((pe, pi))
}

/// Corners for wall `x` whose end lies on the location line of wall `y`,
/// away from `y`'s ends (a T): see [`butt`].
fn tee(x: &StraightWall, x_at_start: bool, y: &StraightWall, far: bool) -> Option<EndCorners> {
    let j = if x_at_start { x.s } else { x.e };
    let len = (y.e - y.s).length();
    let u = (j - y.s).dot(y.d);
    let ny = Vec2::new(-y.d.y, y.d.x);
    let off = (j - y.s).dot(ny);
    let margin = JOIN_TOL.max(1.0);
    if u <= margin || u >= len - margin || off.abs() > JOIN_TOL {
        return None;
    }
    butt(x, x_at_start, y, far)
}

/// Square end of wall `x` extended to the far side of a frozen wall it has
/// a `join` row with (see the module doc). `None` when no such Form
/// touches the end.
fn frozen_end(doc: &Document, x: &StraightWall, at_start: bool) -> Option<EndCorners> {
    let j = if at_start { x.s } else { x.e };
    let o = if at_start { x.d * -1.0 } else { x.d };
    let n = Vec2::new(-x.d.y, x.d.x);
    let thick = x.ext - x.int;
    let limit = (5.0 * thick).max(1000.0);
    let mut rows: Vec<&Relationship> = doc
        .relationships_of(x.id)
        .filter(|r| r.kind == RelationshipKind::Join && r.elements.len() == 2)
        .collect();
    rows.sort_by_key(|r| r.uid);
    for r in rows {
        let Some(other) = r.elements.iter().find(|i| **i != x.id) else {
            continue;
        };
        let Some(e) = doc.get(*other) else { continue };
        let (Some(f), Category::Walls) = (e.as_form(), e.category) else {
            continue;
        };
        let Some(bb) = f.body.bounding_box() else {
            continue;
        };
        if bb.max.z.min(x.z1) - bb.min.z.max(x.z0) <= tol::VERTEX_TOL {
            continue; // not at the same height
        }
        let mut hits: Vec<f64> = Vec::new();
        for q in [j + n * x.ext, j + n * x.int] {
            for (_, a, b) in f.body.edges() {
                let (a, b) = (Point2::new(a.x, a.y), Point2::new(b.x, b.y));
                let ed = b - a;
                if ed.length() < tol::VERTEX_TOL {
                    continue; // a vertical edge
                }
                let cross = o.perp_dot(ed);
                if cross.abs() < 1e-9 * ed.length() {
                    continue; // along the ray
                }
                let t = (a - q).perp_dot(ed) / cross;
                let s = (a - q).perp_dot(o) / cross;
                let eps = tol::VERTEX_TOL / ed.length();
                if (-eps..=1.0 + eps).contains(&s) && t.abs() <= limit {
                    hits.push(t);
                }
            }
        }
        let lo = hits.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = hits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if lo > JOIN_TOL || hi <= tol::VERTEX_TOL {
            continue; // not touching, or nothing beyond the end
        }
        let k = j + o * hi;
        return Some((k + n * x.ext, k + n * x.int));
    }
    None
}

/// Infer the end shape of every straight wall (module doc): L mitres (or
/// butts when a `join` row orders the pair), T stops, square ends where
/// three or more ends meet or an end is disallowed, and ends extended into
/// joined frozen walls.
pub(crate) fn wall_ends(doc: &Document, walls: &[StraightWall]) -> WallEnds {
    let winner = |a: ElementId, b: ElementId| {
        join_between(doc, a, b).and_then(|r| r.elements.first().copied())
    };
    let mut out = WallEnds::default();
    for (i, x) in walls.iter().enumerate() {
        let mut ends = [None, None];
        let mut partners = [None, None];
        for (k, at_start) in [true, false].into_iter().enumerate() {
            if x.disallow[k] {
                continue; // square, sees nothing
            }
            let p = if at_start { x.s } else { x.e };
            let mut found = Vec::new();
            for (jdx, y) in walls.iter().enumerate() {
                if jdx == i {
                    continue;
                }
                if !y.disallow[0] && y.s.distance_to(p) <= JOIN_TOL {
                    found.push((y, true));
                }
                if !y.disallow[1] && y.e.distance_to(p) <= JOIN_TOL {
                    found.push((y, false));
                }
            }
            // (corners, partner, settled by the ends alone)
            let tee_join = || {
                walls
                    .iter()
                    .enumerate()
                    .filter(|(jdx, _)| *jdx != i)
                    .find_map(|(_, y)| {
                        // The stem runs through only when its row puts it first.
                        let wins = winner(x.id, y.id) == Some(x.id);
                        tee(x, at_start, y, wins).map(|c| (c, y.id, !wins))
                    })
            };
            let r = match found.as_slice() {
                // A collinear continuation cannot be mitred; it may still
                // butt against a third wall it crosses.
                [(y, y_start)] => match winner(x.id, y.id) {
                    None => miter(x, at_start, y, *y_start).map(|c| (c, y.id, true)),
                    Some(w) => butt(x, at_start, y, w == x.id).map(|c| (c, y.id, true)),
                }
                .or_else(tee_join),
                [] => tee_join(),
                _ => None,
            };
            ends[k] = match r {
                Some((c, y, settled)) => {
                    if settled {
                        out.settled.insert(pair(x.id, y));
                    }
                    partners[k] = Some(y);
                    Some(c)
                }
                None if found.is_empty() => frozen_end(doc, x, at_start),
                None => None,
            };
        }
        out.corners.insert(x.id, ends);
        out.partners.insert(x.id, partners);
    }
    out
}

/// The walls joined to wall `id` by its ends (L or T, either wall's end),
/// as inferred now; what freezing `id` turns into `join` rows. Sorted.
pub fn wall_join_partners(doc: &Document, id: ElementId) -> Vec<ElementId> {
    let ends = wall_ends(doc, &super::generate::straight_walls(doc));
    let mut out: BTreeSet<ElementId> = BTreeSet::new();
    for (w, ps) in &ends.partners {
        for p in ps.iter().flatten() {
            if *w == id {
                out.insert(*p);
            } else if *p == id {
                out.insert(*w);
            }
        }
    }
    out.into_iter().collect()
}

/// The wall pairs (lower id first) whose end shapes already settle their
/// overlap (an L, mitred or butt, or a T whose stem loses): a `join` row
/// between them needs no boolean, and Revit joins them as walls.
pub fn settled_wall_pairs(doc: &Document) -> BTreeSet<(ElementId, ElementId)> {
    wall_ends(doc, &super::generate::straight_walls(doc)).settled
}

/// Where a wall connection sits on a wall (IFC `IfcConnectionTypeEnum`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallConnectionAt {
    /// The start of the location curve (`ATSTART`).
    Start,
    /// The end of the location curve (`ATEND`).
    End,
    /// Along the wall, away from its ends (`ATPATH`, the crossed wall of a T).
    Path,
}

/// One joined pair of walls (L or T), for exports (IFC
/// `IfcRelConnectsPathElements`). `relating` is the `first` of the pair's
/// `join` row when there is one (the wall that runs through), else the
/// lower id; `row` is that row.
#[derive(Debug, Clone, PartialEq)]
pub struct WallConnection {
    pub relating: ElementId,
    pub relating_at: WallConnectionAt,
    pub related: ElementId,
    pub related_at: WallConnectionAt,
    pub row: Option<super::Uid>,
}

/// Every wall–wall end join of `doc`, as inferred now (disallowed ends and
/// frozen walls take no part), once per pair and joint, sorted.
pub fn wall_connections(doc: &Document) -> Vec<WallConnection> {
    let walls = super::generate::straight_walls(doc);
    let ends = wall_ends(doc, &walls);
    let by_id: HashMap<ElementId, &StraightWall> = walls.iter().map(|w| (w.id, w)).collect();
    let mut seen: BTreeSet<(ElementId, ElementId, i64, i64)> = BTreeSet::new();
    let mut out = Vec::new();
    let mut ids: Vec<&ElementId> = ends.partners.keys().collect();
    ids.sort();
    for xid in ids {
        let (Some(x), Some(ps)) = (by_id.get(xid), ends.partners.get(xid)) else {
            continue;
        };
        for (k, p) in ps.iter().enumerate() {
            let Some(y) = p.and_then(|p| by_id.get(&p)) else {
                continue;
            };
            let j = if k == 0 { x.s } else { x.e };
            let key = (
                x.id.min(y.id),
                x.id.max(y.id),
                j.x.round() as i64,
                j.y.round() as i64,
            );
            if !seen.insert(key) {
                continue;
            }
            let x_at = if k == 0 {
                WallConnectionAt::Start
            } else {
                WallConnectionAt::End
            };
            let y_at = if y.s.distance_to(j) <= JOIN_TOL {
                WallConnectionAt::Start
            } else if y.e.distance_to(j) <= JOIN_TOL {
                WallConnectionAt::End
            } else {
                WallConnectionAt::Path
            };
            let row = join_between(doc, x.id, y.id);
            let x_first = match row.and_then(|r| r.elements.first()) {
                Some(f) => *f == x.id,
                None => x.id < y.id,
            };
            let (relating, relating_at, related, related_at) = if x_first {
                (x.id, x_at, y.id, y_at)
            } else {
                (y.id, y_at, x.id, x_at)
            };
            out.push(WallConnection {
                relating,
                relating_at,
                related,
                related_at,
                row: row.map(|r| r.uid),
            });
        }
    }
    out.sort_by_key(|c| (c.relating, c.related));
    out
}

// --------------------------------------------------------- geometry joins

/// Applies the geometry joins of one regeneration (built once per
/// regeneration by `generate::regenerate_with`).
pub(crate) struct Joiner<'a> {
    doc: &'a Document,
    settled: BTreeSet<(ElementId, ElementId)>,
    warned: bool,
}

impl<'a> Joiner<'a> {
    pub(crate) fn new(doc: &'a Document) -> Joiner<'a> {
        let any = doc
            .relationships()
            .any(|r| r.kind == RelationshipKind::Join);
        let settled = if any {
            wall_ends(doc, &super::generate::straight_walls(doc)).settled
        } else {
            BTreeSet::new()
        };
        Joiner {
            doc,
            settled,
            warned: false,
        }
    }

    /// `id`'s body after its `join` rows (`None` when no row applies);
    /// problems go to `issues` as warnings.
    pub(crate) fn apply(
        &mut self,
        kernel: Option<&dyn Kernel>,
        id: ElementId,
        body: &Body,
        first_body: &dyn Fn(ElementId) -> Option<Body>,
        issues: &mut Vec<GenIssue>,
    ) -> Option<Body> {
        let rows: Vec<&Relationship> = joins_of(self.doc, id)
            .into_iter()
            .filter(|r| match r.elements.first() {
                Some(f) => !self.settled.contains(&pair(*f, id)),
                None => false,
            })
            .collect();
        if rows.is_empty() {
            return None;
        }
        let o = apply_joins(kernel, id, body, &rows, first_body, !self.warned);
        if kernel.is_none() && !o.issues.is_empty() {
            self.warned = true;
        }
        issues.extend(o.issues);
        (o.body != *body).then_some(o.body)
    }
}

/// Subtract the `first` element of each row (in the given order) from
/// `body`, the body of `second`. `first_body` gives an element's unjoined,
/// uncut body. A row whose bodies do not overlap (bounding boxes apart or
/// only touching) changes nothing and needs no kernel. Without a kernel the
/// body stays as it is with one `ferro.kernel.unavailable` warning
/// (`warn_unavailable = false` leaves it out); a row whose boolean fails is
/// skipped with a warning carrying the kernel's code. Never panics.
pub fn apply_joins(
    kernel: Option<&dyn Kernel>,
    second: ElementId,
    body: &Body,
    rows: &[&Relationship],
    first_body: &dyn Fn(ElementId) -> Option<Body>,
    warn_unavailable: bool,
) -> CutOutcome {
    let mut out = CutOutcome {
        body: body.clone(),
        issues: Vec::new(),
    };
    let overlapping: Vec<(&Relationship, ElementId, Body)> = rows
        .iter()
        .filter_map(|r| {
            let first = *r.elements.first()?;
            let b = first_body(first).filter(|b| b.face_count() > 0)?;
            overlaps(body, &b).then_some((*r, first, b))
        })
        .collect();
    if overlapping.is_empty() {
        return out;
    }
    let Some(kernel) = kernel else {
        if warn_unavailable {
            let e = KernelError::unavailable();
            out.issues.push(GenIssue::warning(
                e.code,
                format!(
                    "{}; joins are not applied, the element keeps its unjoined body",
                    e.message
                ),
                second,
            ));
        }
        return out;
    };
    for (r, first, b) in overlapping {
        match kernel.boolean(BooleanOp::Difference, &out.body, &b) {
            Ok(res) => out.body = res.body,
            Err(e) => out.issues.push(GenIssue::warning(
                e.code,
                format!("join {} with {first} is not applied: {}", r.uid, e.message),
                second,
            )),
        }
    }
    out
}

/// `true` when the bounding boxes of `a` and `b` overlap by more than
/// `VERTEX_TOL` in every direction (touching bodies have nothing to share).
fn overlaps(a: &Body, b: &Body) -> bool {
    let (Some(x), Some(y)) = (a.bounding_box(), b.bounding_box()) else {
        return false;
    };
    let t = tol::VERTEX_TOL;
    x.max.x.min(y.max.x) - x.min.x.max(y.min.x) > t
        && x.max.y.min(y.max.y) - x.min.y.max(y.min.y) > t
        && x.max.z.min(y.max.z) - x.min.z.max(y.min.z) > t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(id: u64, s: (f64, f64), e: (f64, f64)) -> StraightWall {
        let (s, e) = (Point2::new(s.0, s.1), Point2::new(e.0, e.1));
        let d = e - s;
        let d = d * (1.0 / d.length());
        StraightWall {
            id: ElementId(id),
            s,
            e,
            d,
            ext: 100.0,
            int: -100.0,
            z0: 0.0,
            z1: 3000.0,
            disallow: [false, false],
        }
    }

    #[test]
    fn butt_stops_at_the_near_face_or_runs_to_the_far_face() {
        // x: (-4000, 0) → (0, 0); y: (0, 0) → (0, 4000).
        let x = wall(1, (-4000.0, 0.0), (0.0, 0.0));
        let y = wall(2, (0.0, 0.0), (0.0, 4000.0));
        let (pe, pi) = butt(&x, false, &y, false).unwrap();
        assert!((pe.x + 100.0).abs() < 1e-9 && (pi.x + 100.0).abs() < 1e-9);
        let (pe, pi) = butt(&x, false, &y, true).unwrap();
        assert!((pe.x - 100.0).abs() < 1e-9 && (pi.x - 100.0).abs() < 1e-9);
        // Parallel walls never butt (degenerate).
        let z = wall(3, (0.0, 0.0), (4000.0, 0.0));
        assert!(butt(&x, false, &z, false).is_none());
    }

    #[test]
    fn a_disallowed_end_neither_joins_nor_is_seen() {
        let doc = Document::new("t");
        let x = wall(1, (-4000.0, 0.0), (0.0, 0.0));
        let mut y = wall(2, (0.0, 0.0), (0.0, 4000.0));
        let ends = wall_ends(&doc, &[x, y]);
        assert!(ends.corners[&x.id][1].is_some(), "mitred by default");
        y.disallow = [true, false];
        let ends = wall_ends(&doc, &[x, y]);
        assert!(
            ends.corners[&x.id][1].is_none(),
            "square: its partner disallows"
        );
        assert!(ends.corners[&y.id][0].is_none());
    }

    #[test]
    fn partners_record_l_and_t_joins() {
        let doc = Document::new("t");
        let x = wall(1, (-4000.0, 0.0), (4000.0, 0.0));
        let stem = wall(2, (0.0, 0.0), (0.0, 3000.0));
        let leg = wall(3, (4000.0, 0.0), (4000.0, 3000.0));
        let ends = wall_ends(&doc, &[x, stem, leg]);
        assert_eq!(ends.partners[&stem.id], [Some(x.id), None], "T");
        assert_eq!(ends.partners[&x.id], [None, Some(leg.id)], "L");
        assert!(ends.settled.contains(&(x.id, stem.id)));
    }

    #[test]
    fn touching_bounding_boxes_do_not_overlap() {
        use crate::algo::extrude_profile;
        use crate::geom::{CurveLoop, Plane, Profile};
        use crate::math::Point3;
        let cube = |x0: f64| {
            let pts = [
                Point3::new(x0, 0.0, 0.0),
                Point3::new(x0 + 1000.0, 0.0, 0.0),
                Point3::new(x0 + 1000.0, 1000.0, 0.0),
                Point3::new(x0, 1000.0, 0.0),
            ];
            let lp = CurveLoop::from_polygon(&pts).unwrap();
            let pr = Profile::from_loops(Plane::horizontal(0.0), lp, vec![]).unwrap();
            extrude_profile(&pr, 1000.0).unwrap().0
        };
        assert!(!overlaps(&cube(0.0), &cube(1000.0)));
        assert!(!overlaps(&cube(0.0), &cube(1000.0 - tol::VERTEX_TOL * 0.5)));
        assert!(overlaps(&cube(0.0), &cube(999.0)));
    }
}
