//! M5-9.2 kernel spike: architectural experiments on `truck` (ricosjp).
//! Units: mm. Run: `ONLY=<substring> MESH_TOL=<mm> truck-eval <boolean tol mm>`.
//! Prints one Markdown table row per experiment.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;
use truck_meshalgo::prelude::*;
use truck_modeling::*;

/// Chordal tolerance for tessellation used to measure volume (mm); env MESH_TOL.
fn mesh_tol() -> f64 {
    std::env::var("MESH_TOL").ok().and_then(|s| s.parse().ok()).unwrap_or(0.5)
}

fn cuboid(p0: [f64; 3], p1: [f64; 3]) -> Solid {
    let v = builder::vertex(Point3::new(p0[0], p0[1], p0[2]));
    let e = builder::tsweep(&v, Vector3::unit_x() * (p1[0] - p0[0]));
    let f = builder::tsweep(&e, Vector3::unit_y() * (p1[1] - p0[1]));
    builder::tsweep(&f, Vector3::unit_z() * (p1[2] - p0[2]))
}

/// Cylinder with axis along +Y through (cx, y0, cz).
fn cylinder_y(cx: f64, cz: f64, y0: f64, len: f64, r: f64) -> Solid {
    let v = builder::vertex(Point3::new(cx + r, y0, cz));
    let w = builder::rsweep(&v, Point3::new(cx, y0, cz), Vector3::unit_y(), Rad(7.0));
    let f = builder::try_attach_plane(&[w]).expect("circle face");
    builder::tsweep(&f, Vector3::unit_y() * len)
}

/// Wall l x t x h with vertical edges rounded (r), built as a rounded
/// profile extruded (truck has no fillet operation).
fn rounded_wall(l: f64, t: f64, h: f64, r: f64) -> Solid {
    let s = std::f64::consts::FRAC_1_SQRT_2 * r;
    let p = |x: f64, y: f64| Point3::new(x, y, 0.0);
    let v: Vec<Vertex> = [
        p(r, 0.0),
        p(l - r, 0.0),
        p(l, r),
        p(l, t - r),
        p(l - r, t),
        p(r, t),
        p(0.0, t - r),
        p(0.0, r),
    ]
    .into_iter()
    .map(builder::vertex)
    .collect();
    let wire: Wire = vec![
        builder::line(&v[0], &v[1]),
        builder::circle_arc(&v[1], &v[2], p(l - r + s, r - s)),
        builder::line(&v[2], &v[3]),
        builder::circle_arc(&v[3], &v[4], p(l - r + s, t - r + s)),
        builder::line(&v[4], &v[5]),
        builder::circle_arc(&v[5], &v[6], p(r - s, t - r + s)),
        builder::line(&v[6], &v[7]),
        builder::circle_arc(&v[7], &v[0], p(r - s, r - s)),
    ]
    .into();
    let f = builder::try_attach_plane(&[wire]).expect("profile face");
    builder::tsweep(&f, Vector3::unit_z() * h)
}

/// Bicubic Bezier roof sheet over x in [-500, 6500], y in [-1000, 1200],
/// thickened by a vertical sweep of `thk`. `double` = doubly curved.
/// Vertical thickness is constant, so volumes follow Cavalieri.
fn roof(double: bool, thk: f64) -> Solid {
    let xs = [-500.0, 1833.333333333333, 4166.666666666667, 6500.0];
    let ys = [-1000.0, -266.6666666666667, 466.6666666666667, 1200.0];
    let bump = [0.0, 1.0, 1.0, 0.0];
    let cp: Vec<Vec<Point3>> = (0..4)
        .map(|i| {
            (0..4)
                .map(|j| {
                    let k = if double { bump[i] * bump[j] } else { bump[i] };
                    Point3::new(xs[i], ys[j], 3000.0 + 1500.0 * k)
                })
                .collect()
        })
        .collect();
    let surf = BSplineSurface::new((KnotVec::bezier_knot(3), KnotVec::bezier_knot(3)), cp.clone());
    let v: Vec<Vertex> = [cp[0][0], cp[3][0], cp[3][3], cp[0][3]].into_iter().map(builder::vertex).collect();
    let bc = |pts: Vec<Point3>| Curve::BSplineCurve(BSplineCurve::new(KnotVec::bezier_knot(3), pts));
    let e0 = Edge::new(&v[0], &v[1], bc((0..4).map(|i| cp[i][0]).collect()));
    let e1 = Edge::new(&v[1], &v[2], bc((0..4).map(|j| cp[3][j]).collect()));
    let e2 = Edge::new(&v[2], &v[3], bc((0..4).map(|i| cp[3 - i][3]).collect()));
    let e3 = Edge::new(&v[3], &v[0], bc((0..4).map(|j| cp[0][3 - j]).collect()));
    let wire: Wire = vec![e0, e1, e2, e3].into();
    let face = Face::new(vec![wire], Surface::BSplineSurface(surf));
    builder::tsweep(&face, Vector3::unit_z() * thk)
}

struct Measure {
    shells: usize,
    closed: bool,
    mesh_closed: String,
    volume: f64,
    tris: usize,
}

fn measure(s: &Solid, tol: f64) -> Measure {
    let closed = s
        .boundaries()
        .iter()
        .all(|sh| format!("{:?}", sh.shell_condition()) == "Closed");
    let mut poly = s.triangulation(tol).to_polygon();
    let volume = poly.volume();
    let tris = poly.faces().triangle_iter().count();
    poly.put_together_same_attrs(1e-3);
    Measure {
        shells: s.boundaries().len(),
        closed,
        mesh_closed: format!("{:?}", poly.shell_condition()),
        volume,
        tris,
    }
}

fn run<F: FnOnce() -> std::result::Result<(Solid, f64), String>>(name: &str, f: F) {
    let filter = std::env::var("ONLY").unwrap_or_default();
    if !name.contains(filter.as_str()) {
        return;
    }
    eprintln!("start {name}");
    let t0 = Instant::now();
    let res = catch_unwind(AssertUnwindSafe(f));
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    match res {
        Ok(Ok((s, expected))) => {
            let t1 = Instant::now();
            let m = measure(&s, mesh_tol());
            let mesh_ms = t1.elapsed().as_secs_f64() * 1e3;
            let rel = (m.volume - expected) / expected;
            let ok = m.closed && rel.abs() < 2e-3;
            println!(
                "| {name} | {} | {ms:.1} | {mesh_ms:.1} | {} | {} | {} | {:.6e} | {:.6e} | {:+.2e} | {} |",
                if ok { "PASS" } else { "FAIL" },
                m.shells, m.closed, m.mesh_closed, m.volume, expected, rel, m.tris
            );
        }
        Ok(Err(e)) => println!("| {name} | FAIL | {ms:.1} | - | - | - | - | - | - | - | error: {e} |"),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            println!("| {name} | PANIC | {ms:.1} | - | - | - | - | - | - | - | {} |", msg.replace('\n', " "));
        }
    }
}

fn diff(a: &Solid, b: &Solid, tol: f64) -> std::result::Result<Solid, String> {
    let mut nb = b.clone();
    nb.not();
    truck_shapeops::and(a, &nb, tol).ok_or_else(|| "and(a, !b) returned None".into())
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let tol: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    println!("truck, boolean tol = {tol} mm, mesh tol = {} mm", mesh_tol());
    println!("| experiment | result | op ms | mesh ms | shells | B-rep closed | mesh condition | volume mm3 | expected | rel err | triangles |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    let (l, t, h) = (6000.0, 200.0, 3000.0);
    let wall = cuboid([0.0, 0.0, 0.0], [l, t, h]);
    let wv = l * t * h;
    let pi = std::f64::consts::PI;

    // E9 scale check: the README / upstream test case (unit cube minus a cylinder r0.3,
    // tol 0.05) at scale 1 and at scale 1000 (mm-sized).
    for k in [1.0, 1000.0] {
        run(&format!("E9 unit-cube - cylinder, scale {k}"), || {
            let c = cuboid([0.0, 0.0, 0.0], [k, k, k]);
            let cy = cylinder_y(0.5 * k, 0.5 * k, -0.1 * k, 1.2 * k, 0.3 * k);
            // scale 1: the upstream tol 0.05; scale 1000: the run's mm tol (CLI arg).
            let bt = if k < 2.0 { 0.05 } else { tol };
            Ok((diff(&c, &cy, bt)?, k * k * k * (1.0 - pi * 0.09)))
        });
    }
    // E0 sanity
    run("E0 wall box", || Ok((wall.clone(), wv)));
    // E1 fillet substitute
    run("E1 rounded-profile wall (no fillet API)", || {
        Ok((rounded_wall(l, t, h, 50.0), (l * t - (4.0 - pi) * 2500.0) * h))
    });
    // E2 booleans
    let win = cuboid([2000.0, -100.0, 900.0], [3500.0, 300.0, 2100.0]);
    run("E2a wall - window box", || Ok((diff(&wall, &win, tol)?, wv - 1500.0 * 200.0 * 1200.0)));
    let cyl = cylinder_y(4500.0, 1500.0, -100.0, 400.0, 400.0);
    run("E2b wall - cylinder r400", || Ok((diff(&wall, &cyl, tol)?, wv - pi * 160000.0 * 200.0)));
    let wall_b = cuboid([0.0, 0.0, 0.0], [t, 4000.0, h]);
    run("E2c wall + wall (L corner)", || {
        let u = truck_shapeops::or(&wall, &wall_b, tol).ok_or("or returned None")?;
        Ok((u, wv + t * 4000.0 * h - t * t * h))
    });
    // E3 NURBS roof
    for double in [false, true] {
        let tag = if double { "double" } else { "single" };
        let r = roof(double, 200.0);
        run(&format!("E3 {tag}-curved roof solid alone"), || Ok((r.clone(), 7000.0 * 2200.0 * 200.0)));
        let tall = cuboid([0.0, 0.0, 0.0], [l, t, 5000.0]);
        run(&format!("E3 wall AND {tag}-curved roof"), || {
            let s = truck_shapeops::and(&tall, &r, tol).ok_or("and returned None")?;
            Ok((s, l * t * 200.0))
        });
        run(&format!("E3 wall - {tag}-curved roof"), || {
            Ok((diff(&tall, &r, tol)?, l * t * 5000.0 - l * t * 200.0))
        });
    }
    // E4 robustness
    let a = cuboid([0.0, 0.0, 0.0], [1000.0, 1000.0, 1000.0]);
    for (name, dx) in [("touching", 0.0), ("gap 0.001", 0.001), ("gap 0.01", 0.01), ("overlap 0.001", -0.001), ("overlap 0.01", -0.01)] {
        let b = cuboid([1000.0 + dx, 0.0, 0.0], [2000.0 + dx, 1000.0, 1000.0]);
        run(&format!("E4 union boxes {name}"), || {
            let u = truck_shapeops::or(&a, &b, tol).ok_or("or returned None")?;
            Ok((u, 2e9 - if dx < 0.0 { -dx * 1e6 } else { 0.0 }))
        });
    }
    let flush = cuboid([2000.0, 0.0, 900.0], [3500.0, 200.0, 2100.0]);
    run("E4 wall - opening flush both faces", || Ok((diff(&wall, &flush, tol)?, wv - 1500.0 * 200.0 * 1200.0)));
    let niche = cuboid([2000.0, 100.0, 900.0], [3500.0, 200.0, 2100.0]);
    run("E4 wall - niche flush one face", || Ok((diff(&wall, &niche, tol)?, wv - 1500.0 * 100.0 * 1200.0)));
    let off = Vector3::new(5.0e6, 5.0e6, 0.0);
    let wall_far = builder::translated(&wall, off);
    let win_far = builder::translated(&win, off);
    run("E4 wall - window at 5 km", || Ok((diff(&wall_far, &win_far, tol)?, wv - 1500.0 * 200.0 * 1200.0)));
    let cyl_far = builder::translated(&cyl, off);
    run("E4 wall - cylinder at 5 km", || Ok((diff(&wall_far, &cyl_far, tol)?, wv - pi * 160000.0 * 200.0)));

    // E5 tessellation timing and STEP round trip
    if std::env::var("ONLY").map(|f| !f.contains("E5")).unwrap_or(false) {
        return;
    }
    if let Ok(s) = diff(&wall, &cyl, tol) {
        for mt in [5.0, 1.0, 0.1] {
            let t0 = Instant::now();
            let poly = s.triangulation(mt).to_polygon();
            println!(
                "E5 tessellate wall-cylinder tol {mt} mm: {:.1} ms, {} triangles, volume err {:+.2e}",
                t0.elapsed().as_secs_f64() * 1e3,
                poly.faces().triangle_iter().count(),
                (poly.volume() - (wv - pi * 160000.0 * 200.0)) / (wv - pi * 160000.0 * 200.0)
            );
        }
        step_round_trip(&s, "wall-cylinder");
    }
    step_round_trip(&roof(true, 200.0), "double-curved roof");
}

fn step_round_trip(s: &Solid, label: &str) {
    use truck_stepio::out;
    let t0 = Instant::now();
    let res = catch_unwind(AssertUnwindSafe(|| {
        let step = out::CompleteStepDisplay::new(out::StepModel::from(&s.compress()), Default::default()).to_string();
        let w_ms = t0.elapsed().as_secs_f64() * 1e3;
        let ex = truck_stepio::r#in::ruststep::parser::parse(&step).map_err(|e| format!("parse: {e:?}"))?;
        let table = truck_stepio::r#in::Table::from_data_section(&ex.data[0]);
        let mut vol = 0.0;
        let mut faces = 0;
        for sh in table.shell.values() {
            let cs = table.to_compressed_shell(sh).map_err(|e| format!("convert: {e:?}"))?;
            faces += cs.faces.len();
            vol += cs.robust_triangulation(mesh_tol()).to_polygon().volume();
        }
        std::result::Result::<_, String>::Ok((step.len(), w_ms, faces, vol))
    }));
    let orig = s.triangulation(mesh_tol()).to_polygon().volume();
    match res {
        Ok(Ok((bytes, w_ms, faces, vol))) => println!(
            "E5 STEP round trip {label}: {bytes} bytes, write {w_ms:.1} ms, total {:.1} ms, {faces} faces back, volume rel diff {:+.2e}",
            t0.elapsed().as_secs_f64() * 1e3,
            (vol - orig) / orig
        ),
        Ok(Err(e)) => println!("E5 STEP round trip {label} FAILED: {e}"),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            println!("E5 STEP round trip {label} PANIC: {}", msg.replace('\n', " "));
        }
    }
}
