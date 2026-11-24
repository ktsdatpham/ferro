//! Exact geometric predicates (D-53). Every topological decision (left / right,
//! inside / outside) goes through these, never through raw `f64` comparisons.

use crate::math::{Point2, Point3};
use robust::{Coord, Coord3D};

fn c(p: Point2) -> Coord<f64> {
    Coord { x: p.x, y: p.y }
}

fn c3(p: Point3) -> Coord3D<f64> {
    Coord3D {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

/// Sign of an exact predicate value: `1`, `-1` or `0`.
fn sign(x: f64) -> i8 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// Sign of the orientation of `(a, b, c)`: `> 0` counter-clockwise (c left of
/// a→b), `< 0` clockwise, `0` exactly collinear. Exact (Shewchuk).
pub fn orient2d(a: Point2, b: Point2, p: Point2) -> f64 {
    robust::orient2d(c(a), c(b), c(p))
}

/// Sign of the orientation of `d` relative to the plane through `a, b, c`:
/// opposite signs for points on opposite sides, `0` exactly coplanar.
/// Exact (Shewchuk).
pub fn orient3d(a: Point3, b: Point3, c: Point3, d: Point3) -> f64 {
    robust::orient3d(c3(a), c3(b), c3(c), c3(d))
}

/// `p` lies within the bounding box of segment `a b` (used once `p` is known
/// to be exactly collinear with it).
fn within_box(a: Point2, b: Point2, p: Point2) -> bool {
    p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
}

/// Exact: do the closed segments `a b` and `c d` share at least one point
/// (crossing, touching at an end point or overlapping)?
pub fn segments_intersect(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let d1 = sign(orient2d(c, d, a));
    let d2 = sign(orient2d(c, d, b));
    let d3 = sign(orient2d(a, b, c));
    let d4 = sign(orient2d(a, b, d));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true;
    }
    (d1 == 0 && within_box(c, d, a))
        || (d2 == 0 && within_box(c, d, b))
        || (d3 == 0 && within_box(a, b, c))
        || (d4 == 0 && within_box(a, b, d))
}

/// Exact: is the closed polygon `poly` simple (D-53)? Non-adjacent edges
/// must not touch, adjacent edges must not fold back onto each other, and no
/// vertex may repeat. Fewer than three points, or all points collinear, is
/// not simple. `O(n²)` with a bounding-box prefilter.
pub fn polygon_is_simple(poly: &[Point2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let edge = |i: usize| (poly[i], poly[(i + 1) % n]);
    let boxes: Vec<(f64, f64, f64, f64)> = (0..n)
        .map(|i| {
            let (a, b) = edge(i);
            (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
        })
        .collect();
    for i in 0..n {
        // Adjacent edges a→b, b→c may only share b.
        let (a, b) = edge(i);
        let c = poly[(i + 2) % n];
        if sign(orient2d(a, b, c)) == 0 && (within_box(a, b, c) || within_box(b, c, a)) {
            return false;
        }
        for j in (i + 2)..n {
            if i == 0 && j == n - 1 {
                continue; // adjacent through the closing edge
            }
            let (bi, bj) = (boxes[i], boxes[j]);
            if bi.2 < bj.0 || bj.2 < bi.0 || bi.3 < bj.1 || bj.3 < bi.1 {
                continue;
            }
            let (c, d) = edge(j);
            if segments_intersect(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// Exact: are all points on one line (or all coincident)? Such an outline
/// encloses no area.
pub fn points_are_collinear(pts: &[Point2]) -> bool {
    let Some(&a) = pts.first() else {
        return true;
    };
    let Some(&b) = pts.iter().find(|p| p.distance_to(a) > 0.0) else {
        return true;
    };
    pts.iter().all(|p| sign(orient2d(a, b, *p)) == 0)
}

/// Exact: do two closed polygons share at least one point (an edge of one
/// touches an edge of the other, or one lies inside the other)?
pub fn polygons_touch(a: &[Point2], b: &[Point2]) -> bool {
    match (a.first(), b.first()) {
        (Some(pa), Some(pb)) => {
            polygons_touch_boundary(a, b) || point_in_polygon(*pa, b) || point_in_polygon(*pb, a)
        }
        _ => false,
    }
}

/// Exact: does an edge of closed polygon `a` touch an edge of closed
/// polygon `b` (crossing, touching or overlapping)?
pub fn polygons_touch_boundary(a: &[Point2], b: &[Point2]) -> bool {
    let (na, nb) = (a.len(), b.len());
    (0..na)
        .any(|i| (0..nb).any(|j| segments_intersect(a[i], a[(i + 1) % na], b[j], b[(j + 1) % nb])))
}

/// Exact point-in-triangle test in 2D, boundary included.
fn point_in_triangle(p: Point2, t: [Point2; 3]) -> bool {
    let s = [
        sign(orient2d(t[0], t[1], p)),
        sign(orient2d(t[1], t[2], p)),
        sign(orient2d(t[2], t[0], p)),
    ];
    !(s.contains(&1) && s.contains(&-1))
}

/// Projection that keeps orientation tests meaningful for points exactly
/// coplanar with the triangle `t`: drop the axis of its largest normal
/// component.
fn projector(t: [Point3; 3]) -> impl Fn(Point3) -> Point2 {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
    let drop = if az >= ax && az >= ay {
        2
    } else if ax >= ay {
        0
    } else {
        1
    };
    move |p: Point3| match drop {
        0 => Point2::new(p.y, p.z),
        1 => Point2::new(p.z, p.x),
        _ => Point2::new(p.x, p.y),
    }
}

/// Exact: is the triangle degenerate (its three points collinear or
/// coincident)?
pub fn triangle_is_degenerate(t: [Point3; 3]) -> bool {
    let xy = |p: Point3| Point2::new(p.x, p.y);
    let yz = |p: Point3| Point2::new(p.y, p.z);
    let zx = |p: Point3| Point2::new(p.z, p.x);
    sign(orient2d(xy(t[0]), xy(t[1]), xy(t[2]))) == 0
        && sign(orient2d(yz(t[0]), yz(t[1]), yz(t[2]))) == 0
        && sign(orient2d(zx(t[0]), zx(t[1]), zx(t[2]))) == 0
}

/// Two triangles exactly in the same plane: any shared point?
fn coplanar_triangles_intersect(t: [Point3; 3], u: [Point3; 3]) -> bool {
    let pr = projector(t);
    let a = [pr(t[0]), pr(t[1]), pr(t[2])];
    let b = [pr(u[0]), pr(u[1]), pr(u[2])];
    for i in 0..3 {
        for j in 0..3 {
            if segments_intersect(a[i], a[(i + 1) % 3], b[j], b[(j + 1) % 3]) {
                return true;
            }
        }
    }
    point_in_triangle(a[0], b) || point_in_triangle(b[0], a)
}

/// Exact: does the closed segment `p q` meet the closed triangle `t`?
fn segment_meets_triangle(p: Point3, q: Point3, t: [Point3; 3]) -> bool {
    let sp = sign(orient3d(t[0], t[1], t[2], p));
    let sq = sign(orient3d(t[0], t[1], t[2], q));
    if sp == sq && sp != 0 {
        return false;
    }
    if sp == 0 && sq == 0 {
        let pr = projector(t);
        let (a, b) = (pr(p), pr(q));
        let tt = [pr(t[0]), pr(t[1]), pr(t[2])];
        return (0..3).any(|i| segments_intersect(a, b, tt[i], tt[(i + 1) % 3]))
            || point_in_triangle(a, tt);
    }
    // The segment reaches the plane: does its line pass through t?
    let s = [
        sign(orient3d(p, q, t[0], t[1])),
        sign(orient3d(p, q, t[1], t[2])),
        sign(orient3d(p, q, t[2], t[0])),
    ];
    !(s.contains(&1) && s.contains(&-1))
}

/// Exact triangle–triangle intersection test (D-53): `true` when the two
/// closed triangles share at least one point (crossing, touching or
/// overlapping in a common plane). Both triangles must be non-degenerate
/// (see [`triangle_is_degenerate`]); a degenerate one gives an unspecified
/// answer.
pub fn triangles_intersect(t: [Point3; 3], u: [Point3; 3]) -> bool {
    let side = |tri: [Point3; 3], p: Point3| sign(orient3d(tri[0], tri[1], tri[2], p));
    let su = [side(t, u[0]), side(t, u[1]), side(t, u[2])];
    if su.iter().all(|s| *s > 0) || su.iter().all(|s| *s < 0) {
        return false;
    }
    let st = [side(u, t[0]), side(u, t[1]), side(u, t[2])];
    if st.iter().all(|s| *s > 0) || st.iter().all(|s| *s < 0) {
        return false;
    }
    if su.iter().all(|s| *s == 0) {
        return coplanar_triangles_intersect(t, u);
    }
    // Not coplanar: the intersection (if any) is a segment whose end points
    // lie on edges of one triangle inside the other.
    (0..3).any(|i| segment_meets_triangle(t[i], t[(i + 1) % 3], u))
        || (0..3).any(|i| segment_meets_triangle(u[i], u[(i + 1) % 3], t))
}

/// Exact point-in-polygon (crossing number with exact orientation tests).
/// Points exactly on the boundary count as inside.
pub fn point_in_polygon(p: Point2, poly: &[Point2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        // On-segment check (exact collinearity + bounding box).
        if orient2d(a, b, p) == 0.0
            && p.x >= a.x.min(b.x)
            && p.x <= a.x.max(b.x)
            && p.y >= a.y.min(b.y)
            && p.y <= a.y.max(b.y)
        {
            return true;
        }
        let upward = a.y <= p.y && b.y > p.y;
        let downward = b.y <= p.y && a.y > p.y;
        if upward && orient2d(a, b, p) > 0.0 || downward && orient2d(a, b, p) < 0.0 {
            inside = !inside;
        }
    }
    inside
}

/// Twice the signed area of a 2D polygon (positive = counter-clockwise).
pub fn signed_area2(poly: &[Point2]) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq() -> Vec<Point2> {
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(10.0, 10.0),
            Point2::new(0.0, 10.0),
        ]
    }

    #[test]
    fn orientation_signs() {
        let (a, b) = (Point2::new(0.0, 0.0), Point2::new(1.0, 0.0));
        assert!(orient2d(a, b, Point2::new(0.5, 1.0)) > 0.0);
        assert!(orient2d(a, b, Point2::new(0.5, -1.0)) < 0.0);
        assert_eq!(orient2d(a, b, Point2::new(7.0, 0.0)), 0.0);
    }

    #[test]
    fn orientation_is_exact_for_nearly_collinear_points() {
        // A classic case where naive f64 evaluation gives the wrong sign.
        let a = Point2::new(0.5, 0.5);
        let b = Point2::new(12.0, 12.0);
        let p = Point2::new(24.0, 24.0 + f64::EPSILON * 24.0);
        assert!(orient2d(a, b, p) > 0.0);
    }

    #[test]
    fn point_in_polygon_inside_outside_and_boundary() {
        assert!(point_in_polygon(Point2::new(5.0, 5.0), &sq()));
        assert!(!point_in_polygon(Point2::new(15.0, 5.0), &sq()));
        assert!(point_in_polygon(Point2::new(10.0, 5.0), &sq()));
        assert!(point_in_polygon(Point2::new(0.0, 0.0), &sq()));
    }

    fn p2(pts: &[(f64, f64)]) -> Vec<Point2> {
        pts.iter().map(|(x, y)| Point2::new(*x, *y)).collect()
    }

    #[test]
    fn segments_cross_touch_overlap_and_miss() {
        let q = Point2::new;
        assert!(segments_intersect(
            q(0.0, 0.0),
            q(2.0, 2.0),
            q(0.0, 2.0),
            q(2.0, 0.0)
        ));
        // Touching at an end point; overlapping collinear.
        assert!(segments_intersect(
            q(0.0, 0.0),
            q(1.0, 0.0),
            q(1.0, 0.0),
            q(1.0, 5.0)
        ));
        assert!(segments_intersect(
            q(0.0, 0.0),
            q(2.0, 0.0),
            q(1.0, 0.0),
            q(3.0, 0.0)
        ));
        // Collinear but disjoint, parallel, a hair apart.
        assert!(!segments_intersect(
            q(0.0, 0.0),
            q(1.0, 0.0),
            q(2.0, 0.0),
            q(3.0, 0.0)
        ));
        assert!(!segments_intersect(
            q(0.0, 0.0),
            q(1.0, 0.0),
            q(0.0, 1.0),
            q(1.0, 1.0)
        ));
        assert!(!segments_intersect(
            q(0.0, 0.0),
            q(1.0, 0.0),
            q(0.5, 1e-12),
            q(0.5, 1.0)
        ));
    }

    #[test]
    fn simple_and_self_intersecting_polygons() {
        assert!(polygon_is_simple(&sq()));
        // The audit's bow-tie (M5-15.10).
        let bow = p2(&[(0.0, 0.0), (3000.0, 1000.0), (3000.0, 0.0), (0.0, 2000.0)]);
        assert!(!polygon_is_simple(&bow));
        let l = p2(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (0.0, 2.0),
        ]);
        assert!(polygon_is_simple(&l));
    }

    #[test]
    fn degenerate_polygons_are_not_simple() {
        assert!(!polygon_is_simple(&p2(&[(0.0, 0.0), (1.0, 0.0)])));
        // Collinear; a spike folding back; a repeated vertex (figure eight).
        assert!(!polygon_is_simple(&p2(&[
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0)
        ])));
        assert!(!polygon_is_simple(&p2(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0)
        ])));
        let eight = [
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (2.0, 1.0),
            (2.0, 2.0),
            (1.0, 2.0),
            (1.0, 1.0),
            (0.0, 1.0),
        ];
        assert!(!polygon_is_simple(&p2(&eight)));
    }

    #[test]
    fn vertex_touching_an_edge_within_a_hair_is_decided_exactly() {
        // A notch whose tip just misses, reaches or crosses the bottom edge.
        let make = |tip: f64| {
            p2(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (6.0, 10.0),
                (5.0, tip),
                (4.0, 10.0),
                (0.0, 10.0),
            ])
        };
        assert!(polygon_is_simple(&make(1e-12)));
        assert!(!polygon_is_simple(&make(0.0)));
        assert!(!polygon_is_simple(&make(-1e-12)));
    }

    #[test]
    fn polygons_touching_inside_or_apart() {
        let big = sq();
        let inner = p2(&[(2.0, 2.0), (4.0, 2.0), (4.0, 4.0)]);
        let outside = p2(&[(20.0, 2.0), (24.0, 2.0), (24.0, 4.0)]);
        let touching = p2(&[(10.0, 2.0), (14.0, 2.0), (14.0, 4.0)]);
        assert!(polygons_touch(&big, &inner));
        assert!(polygons_touch(&inner, &big));
        assert!(!polygons_touch(&big, &outside));
        assert!(polygons_touch(&big, &touching));
    }

    fn t3(a: (f64, f64, f64), b: (f64, f64, f64), c: (f64, f64, f64)) -> [Point3; 3] {
        let q = |p: (f64, f64, f64)| Point3::new(p.0, p.1, p.2);
        [q(a), q(b), q(c)]
    }

    #[test]
    fn triangles_crossing_touching_and_apart() {
        let flat = t3((0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (0.0, 10.0, 0.0));
        let pierce = t3((2.0, 2.0, -1.0), (2.0, 2.0, 1.0), (3.0, 3.0, 1.0));
        assert!(triangles_intersect(flat, pierce));
        assert!(triangles_intersect(pierce, flat));
        // Touching with one vertex; a hair above.
        let touch = t3((2.0, 2.0, 0.0), (2.0, 2.0, 1.0), (3.0, 3.0, 1.0));
        assert!(triangles_intersect(flat, touch));
        let above = t3((2.0, 2.0, 1e-12), (2.0, 2.0, 1.0), (3.0, 3.0, 1.0));
        assert!(!triangles_intersect(flat, above));
        // Coplanar: overlapping, apart, one inside the other.
        let co = t3((1.0, 1.0, 0.0), (20.0, 1.0, 0.0), (1.0, 20.0, 0.0));
        assert!(triangles_intersect(flat, co));
        let apart = t3((11.0, 11.0, 0.0), (20.0, 11.0, 0.0), (11.0, 20.0, 0.0));
        assert!(!triangles_intersect(flat, apart));
        let inner = t3((1.0, 1.0, 0.0), (2.0, 1.0, 0.0), (1.0, 2.0, 0.0));
        assert!(triangles_intersect(flat, inner));
        assert!(triangles_intersect(inner, flat));
    }

    #[test]
    fn degenerate_triangles_are_detected() {
        let line = t3((0.0, 0.0, 0.0), (1.0, 1.0, 1.0), (2.0, 2.0, 2.0));
        assert!(triangle_is_degenerate(line));
        let ok = t3((0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0));
        assert!(!triangle_is_degenerate(ok));
    }

    #[test]
    fn signed_area_sign_follows_orientation() {
        assert!(signed_area2(&sq()) > 0.0);
        let mut cw = sq();
        cw.reverse();
        assert!(signed_area2(&cw) < 0.0);
    }
}
