//! M5-9.2 kernel spike: the same architectural experiments on `solidcore`
//! (tangency-labs, safe-Rust port of NVIDIA SMLib). Units: mm.
//! Run: `cargo run` (in this folder). Env: ONLY=<substring>, MESH_TOL=<mm>.

use solidcore_ops::boolean::boolean;
use solidcore_ops::sweep::{extrude, loft, Curve, DraftCorner};
use solidcore_ops::{blend_ordered, make_box, session, Blend, BlendOptions, Body, BooleanOp, Corners};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

type R<T> = std::result::Result<T, String>;

fn mesh_tol() -> f64 {
    std::env::var("MESH_TOL").ok().and_then(|s| s.parse().ok()).unwrap_or(0.5)
}

fn e<E: std::fmt::Debug>(what: &'static str) -> impl Fn(E) -> String {
    move |x| format!("{what}: {x:?}")
}

fn cuboid(p0: [f64; 3], p1: [f64; 3]) -> R<Body> {
    make_box(p0, [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]]).map_err(e("make_box"))
}

fn line(a: [f64; 3], b: [f64; 3]) -> Curve {
    Curve::Line { start: a, end: b }
}

fn arc(center: [f64; 3], r: f64, start: f64, end: f64) -> Curve {
    Curve::Arc { center, axis: [0.0, 0.0, 1.0], reference: [1.0, 0.0, 0.0], radius: r, start, end }
}

/// Cylinder along +Y: circle in the plane y = y0, extruded `len`.
fn cylinder_y(cx: f64, cz: f64, y0: f64, len: f64, r: f64) -> R<Body> {
    let c = Curve::Arc {
        center: [cx, y0, cz],
        axis: [0.0, 1.0, 0.0],
        reference: [1.0, 0.0, 0.0],
        radius: r,
        start: 0.0,
        end: std::f64::consts::TAU,
    };
    extrude(&[c], [0.0, 1.0, 0.0], len, 0.0, DraftCorner::Curvature, true).map_err(e("extrude circle"))
}

/// Rounded-corner wall profile extruded (the fillet substitute, as for truck).
fn rounded_wall(l: f64, t: f64, h: f64, r: f64) -> R<Body> {
    use std::f64::consts::{FRAC_PI_2 as Q, PI};
    let z = 0.0;
    let prof = vec![
        line([r, 0.0, z], [l - r, 0.0, z]),
        arc([l - r, r, z], r, -Q, 0.0),
        line([l, r, z], [l, t - r, z]),
        arc([l - r, t - r, z], r, 0.0, Q),
        line([l - r, t, z], [r, t, z]),
        arc([r, t - r, z], r, Q, PI),
        line([0.0, t - r, z], [0.0, r, z]),
        arc([r, r, z], r, PI, PI + Q),
    ];
    extrude(&prof, [0.0, 0.0, 1.0], h, 0.0, DraftCorner::Curvature, true).map_err(e("extrude profile"))
}

/// Roof slab: lofted (degree 3) through 4 closed sections in planes y = const; each section's
/// bottom is a cubic Bezier z = f(x) over x in [-500, 6500], its top the same moved up `thk`.
/// Vertical thickness is `thk` everywhere, so volumes follow Cavalieri as for truck.
fn roof(double: bool, thk: f64) -> R<Body> {
    let xs = [-500.0, 1833.333333333333, 4166.666666666667, 6500.0];
    let ys = [-1000.0, -266.6666666666667, 466.6666666666667, 1200.0];
    let bump = [0.0, 1.0, 1.0, 0.0];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let sections: Vec<Vec<Curve>> = (0..4)
        .map(|j| {
            let s = if double { 0.3 + 0.7 * bump[j] } else { 1.0 };
            let bot: Vec<[f64; 4]> = (0..4).map(|i| [xs[i], ys[j], 3000.0 + 1500.0 * bump[i] * s, 1.0]).collect();
            let top: Vec<[f64; 4]> = bot.iter().rev().map(|p| [p[0], p[1], p[2] + thk, 1.0]).collect();
            let (b0, b3, t0, t3) = (bot[0], bot[3], top[0], top[3]);
            vec![
                Curve::Nurbs { degree: 3, points: bot, knots: knots.clone() },
                line([b3[0], b3[1], b3[2]], [t0[0], t0[1], t0[2]]),
                Curve::Nurbs { degree: 3, points: top, knots: knots.clone() },
                line([t3[0], t3[1], t3[2]], [b0[0], b0[1], b0[2]]),
            ]
        })
        .collect();
    loft(&sections, 3, true).map_err(e("loft"))
}

struct Measure {
    counts: [usize; 3],
    manifold: bool,
    volume: f64,
    mesh_volume: f64,
    tris: usize,
    mesh_ms: f64,
}

fn mesh(b: &Body, tol: f64) -> R<(f64, usize, f64)> {
    let (rec, _) = b.to_record().map_err(e("to_record"))?;
    let t0 = Instant::now();
    let (plan, _stats) = solidcore_tess::plan(&rec, &solidcore_tess::MeshTolerance::chord(tol), 10_000.0)
        .map_err(e("tess plan"))?;
    let (m, refused) = plan.evaluate();
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    if !refused.is_empty() {
        return Err(format!("tess refused {} faces", refused.len()));
    }
    let mut v = 0.0;
    for t in &m.triangles {
        let p = |i: u32| m.positions[i as usize];
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        v += (a.x * (b.y * c.z - b.z * c.y) - a.y * (b.x * c.z - b.z * c.x) + a.z * (b.x * c.y - b.y * c.x)) / 6.0;
    }
    Ok((v, m.triangles.len(), ms))
}

fn measure(b: &Body) -> R<Measure> {
    let (mesh_volume, tris, mesh_ms) = mesh(b, mesh_tol())?;
    Ok(Measure {
        counts: b.counts().map_err(e("counts"))?,
        manifold: b.is_manifold_solid().map_err(e("manifold"))?,
        volume: b.volume(1e-9).map_err(e("volume"))?,
        mesh_volume,
        tris,
        mesh_ms,
    })
}

fn run<F: FnOnce() -> R<(Body, f64)>>(name: &str, f: F) {
    let filter = std::env::var("ONLY").unwrap_or_default();
    if !name.contains(filter.as_str()) {
        return;
    }
    eprintln!("start {name}");
    let t0 = Instant::now();
    let res = catch_unwind(AssertUnwindSafe(f));
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    match res {
        Ok(Ok((b, expected))) => match measure(&b) {
            Ok(m) => {
                let rel = (m.volume - expected) / expected;
                let mrel = (m.mesh_volume - expected) / expected;
                let ok = m.manifold && rel.abs() < 2e-3;
                println!(
                    "| {name} | {} | {ms:.1} | {:.1} | {:?} | {} | {:.6e} | {:.6e} | {rel:+.2e} | {mrel:+.2e} | {} |",
                    if ok { "PASS" } else { "FAIL" },
                    m.mesh_ms, m.counts, m.manifold, m.volume, expected, m.tris
                );
            }
            Err(err) => println!("| {name} | FAIL | {ms:.1} | - | - | - | - | - | - | - | measure: {err} |"),
        },
        Ok(Err(err)) => println!("| {name} | FAIL | {ms:.1} | - | - | - | - | - | - | - | {} |", err.replace('\n', " ")),
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

fn op(a: &Body, b: &Body, o: BooleanOp) -> R<Body> {
    let a = a.copy().map_err(e("copy"))?;
    let b = b.copy().map_err(e("copy"))?;
    boolean(a, b, o).map_err(e("boolean"))?.ok_or_else(|| "boolean: empty result".to_string())
}

fn translated(b: &Body, d: [f64; 3]) -> R<Body> {
    let c = b.copy().map_err(e("copy"))?;
    // Column-major: the fourth column (0, 0, 0, 1) is the homogeneous row, m[3] the translation.
    let m = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [d[0], d[1], d[2], 1.0]];
    c.transform(&m).map_err(e("transform"))?;
    Ok(c)
}

/// Fillet `n` vertical edges of `wall` (record edges whose two vertices differ only in z).
fn fillet_vertical(wall: &Body, n: usize, r: f64) -> R<Body> {
    let b = wall.copy().map_err(e("copy"))?;
    let (rec, order) = b.to_record().map_err(e("to_record"))?;
    let edges: Vec<usize> = rec
        .edges
        .iter()
        .enumerate()
        .filter(|(_, ed)| {
            let p = |i: u32| rec.vertices[i as usize].point;
            let (a, c) = (p(ed.vertices[0]), p(ed.vertices[1]));
            (a[0] - c[0]).abs() < 1e-6 && (a[1] - c[1]).abs() < 1e-6
        })
        .map(|(i, _)| i)
        .take(n)
        .collect();
    if edges.len() != n {
        return Err(format!("found {} vertical edges", edges.len()));
    }
    let sizes = vec![r; n];
    blend_ordered(&b, &order, Blend::Round, &edges, &sizes, &[], Corners::Filled, BlendOptions::default())
        .map_err(e("blend"))?;
    Ok(b)
}

fn step_round_trip(b: &Body, label: &str) {
    use solidcore_step::write::product::{products, ExportBody, ExportProduct, Header};
    let t0 = Instant::now();
    let res = (|| -> R<String> {
        let (rec, _) = b.to_record().map_err(e("to_record"))?;
        let text = products(
            &[ExportProduct {
                name: "spike".into(),
                bodies: vec![ExportBody { name: "body".into(), record: &rec, color: None, faces: vec![] }],
                children: vec![],
            }],
            0,
            &Header::default(),
        )
        .map_err(e("step write"))?;
        let w_ms = t0.elapsed().as_secs_f64() * 1e3;
        let ex = solidcore_step::p21::parse(&text).map_err(e("p21 parse"))?;
        let solids = solidcore_step::topology::read_solids(&ex);
        if !solids.failures.is_empty() {
            return Err(format!("read failures: {:?}", solids.failures));
        }
        let mut vol = 0.0;
        let mut faces = 0;
        for s in &solids.solids {
            faces += s.brep.faces.len();
            let (back, _) = Body::from_record(&s.brep, false).map_err(e("from_record"))?;
            vol += back.volume(1e-9).map_err(e("volume"))?;
        }
        let orig = b.volume(1e-9).map_err(e("volume"))?;
        Ok(format!(
            "{} bytes, write {w_ms:.1} ms, total {:.1} ms, {} solid(s), {faces} faces back, volume rel diff {:+.2e}",
            text.len(),
            t0.elapsed().as_secs_f64() * 1e3,
            solids.solids.len(),
            (vol - orig) / orig
        ))
    })();
    match res {
        Ok(s) => println!("E5 STEP round trip {label}: {s}"),
        Err(err) => println!("E5 STEP round trip {label} FAILED: {err}"),
    }
}

fn main() {
    // The kernel asks callers for an 8 MiB stack (its README, "WebAssembly stack size").
    let h = std::thread::Builder::new().stack_size(64 << 20).spawn(body).expect("thread");
    let _ = h.join();
}

fn body() {
    std::panic::set_hook(Box::new(|_| {}));
    // As SolidCraft and the kernel's own tests do: switch on every upstream bug-fix flag.
    if let Err(err) = session::set_thread_fixes(solidcore::kernel::UpstreamFixes::NAMES.iter().copied()) {
        println!("fixes refused: {err:?}");
    }
    println!("solidcore, exact volume from mass properties, mesh tol = {} mm", mesh_tol());
    println!("| experiment | result | op ms | mesh ms | counts [F,E,V] | manifold solid | volume mm3 | expected | rel err | mesh vol rel err | triangles |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    let (l, t, h) = (6000.0, 200.0, 3000.0);
    let pi = std::f64::consts::PI;
    let wv = l * t * h;
    let Ok(wall) = cuboid([0.0, 0.0, 0.0], [l, t, h]) else {
        println!("make_box failed");
        return;
    };
    run("E0 wall box", || Ok((wall.copy().map_err(e("copy"))?, wv)));
    run("E1 rounded-profile wall (extrude)", || Ok((rounded_wall(l, t, h, 50.0)?, (l * t - (4.0 - pi) * 2500.0) * h)));
    for n in [1usize, 4] {
        run(&format!("E1b fillet r50 on {n} vertical edge(s)"), || {
            Ok((fillet_vertical(&wall, n, 50.0)?, wv - n as f64 * (4.0 - pi) / 4.0 * 2500.0 * h))
        });
    }
    let win = || cuboid([2000.0, -100.0, 900.0], [3500.0, 300.0, 2100.0]);
    run("E2a wall - window box", || Ok((op(&wall, &win()?, BooleanOp::Subtract)?, wv - 1500.0 * 200.0 * 1200.0)));
    let cyl = || cylinder_y(4500.0, 1500.0, -100.0, 400.0, 400.0);
    run("E2b wall - cylinder r400", || Ok((op(&wall, &cyl()?, BooleanOp::Subtract)?, wv - pi * 160000.0 * 200.0)));
    run("E2c wall + wall (L corner)", || {
        let b = cuboid([0.0, 0.0, 0.0], [t, 4000.0, h])?;
        Ok((op(&wall, &b, BooleanOp::Union)?, wv + t * 4000.0 * h - t * t * h))
    });
    for double in [false, true] {
        let tag = if double { "double" } else { "single" };
        run(&format!("E3 {tag}-curved roof solid alone"), || Ok((roof(double, 200.0)?, 7000.0 * 2200.0 * 200.0)));
        let tall = || cuboid([0.0, 0.0, 0.0], [l, t, 5000.0]);
        run(&format!("E3 wall AND {tag}-curved roof"), || {
            Ok((op(&tall()?, &roof(double, 200.0)?, BooleanOp::Intersect)?, l * t * 200.0))
        });
        run(&format!("E3 wall - {tag}-curved roof"), || {
            Ok((op(&tall()?, &roof(double, 200.0)?, BooleanOp::Subtract)?, l * t * 5000.0 - l * t * 200.0))
        });
    }
    let a = || cuboid([0.0, 0.0, 0.0], [1000.0, 1000.0, 1000.0]);
    for (name, dx) in [("touching", 0.0), ("gap 0.001", 0.001), ("gap 0.01", 0.01), ("overlap 0.001", -0.001), ("overlap 0.01", -0.01)] {
        run(&format!("E4 union boxes {name}"), || {
            let b = cuboid([1000.0 + dx, 0.0, 0.0], [2000.0 + dx, 1000.0, 1000.0])?;
            Ok((op(&a()?, &b, BooleanOp::Union)?, 2e9 - if dx < 0.0 { -dx * 1e6 } else { 0.0 }))
        });
    }
    run("E4 wall - opening flush both faces", || {
        let f = cuboid([2000.0, 0.0, 900.0], [3500.0, 200.0, 2100.0])?;
        Ok((op(&wall, &f, BooleanOp::Subtract)?, wv - 1500.0 * 200.0 * 1200.0))
    });
    run("E4 wall - niche flush one face", || {
        let f = cuboid([2000.0, 100.0, 900.0], [3500.0, 200.0, 2100.0])?;
        Ok((op(&wall, &f, BooleanOp::Subtract)?, wv - 1500.0 * 100.0 * 1200.0))
    });
    let off = [5.0e6, 5.0e6, 0.0];
    run("E4 wall - window at 5 km", || {
        Ok((op(&translated(&wall, off)?, &translated(&win()?, off)?, BooleanOp::Subtract)?, wv - 1500.0 * 200.0 * 1200.0))
    });
    run("E4 wall - cylinder at 5 km", || {
        Ok((op(&translated(&wall, off)?, &translated(&cyl()?, off)?, BooleanOp::Subtract)?, wv - pi * 160000.0 * 200.0))
    });

    if std::env::var("ONLY").map(|f| !f.contains("E5")).unwrap_or(false) {
        return;
    }
    if let Ok(s) = cyl().and_then(|c| op(&wall, &c, BooleanOp::Subtract)) {
        let exact = wv - pi * 160000.0 * 200.0;
        for mt in [5.0, 1.0, 0.1] {
            match mesh(&s, mt) {
                Ok((v, n, ms)) => println!(
                    "E5 tessellate wall-cylinder tol {mt} mm: {ms:.1} ms, {n} triangles, volume err {:+.2e}",
                    (v - exact) / exact
                ),
                Err(err) => println!("E5 tessellate tol {mt} FAILED: {err}"),
            }
        }
        step_round_trip(&s, "wall-cylinder");
    }
    if let Ok(r) = roof(true, 200.0) {
        for mt in [5.0, 1.0, 0.1] {
            match mesh(&r, mt) {
                Ok((v, n, ms)) => println!(
                    "E5 tessellate double-curved roof tol {mt} mm: {ms:.1} ms, {n} triangles, volume err {:+.2e}",
                    (v - 3.08e9) / 3.08e9
                ),
                Err(err) => println!("E5 tessellate roof tol {mt} FAILED: {err}"),
            }
        }
        step_round_trip(&r, "double-curved roof");
    }
}
