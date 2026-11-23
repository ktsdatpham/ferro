//! Shared helpers for the scenario, stress and fuzz tests (M5-15): drive
//! commands through `AnyCommand::from_tool` + `Session::execute` and check
//! the invariants every command must keep.

use super::*;
use crate::math::{tol, Point3, Vec3};
use crate::model::{Document, ElementBody, ElementId};
use crate::topo::{Body, FaceId};
use serde_json::{json, Value};

pub fn session() -> Session {
    Session::new(
        Document::new("Scenario"),
        Box::new(SequentialIds::new(1_764_000_000_000)),
    )
}

/// A point as tool JSON.
pub fn p(x: f64, y: f64, z: f64) -> Value {
    json!({"x": x, "y": y, "z": z})
}

pub fn try_run(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    match AnyCommand::from_tool(tool, &args) {
        Ok(cmd) => s.execute(cmd, &ExecContext::user()),
        Err(m) => panic!("{tool}: arguments rejected: {m} ({args})"),
    }
}

pub fn run(s: &mut Session, tool: &str, args: Value) -> Outcome {
    match try_run(s, tool, args.clone()) {
        Ok(o) => o,
        Err(f) => panic!("{tool} {args} failed: {f:?}"),
    }
}

pub fn id(o: &Outcome) -> ElementId {
    serde_json::from_value(o.output["element"].clone()).expect("element in output")
}

pub fn face_of(o: &Outcome) -> FaceId {
    serde_json::from_value(o.output["face"].clone()).expect("face in output")
}

pub fn new_face(o: &Outcome) -> FaceId {
    serde_json::from_value(o.output["new_faces"][0].clone()).expect("new face in output")
}

/// The document as JSON text: exact comparison of two versions.
pub fn snapshot(s: &Session) -> String {
    serde_json::to_string(s.doc()).expect("document serialises")
}

pub fn geometry(s: &Session, e: ElementId) -> Body {
    s.doc()
        .get(e)
        .and_then(|x| x.geometry())
        .cloned()
        .unwrap_or_else(|| panic!("{e} has no geometry"))
}

/// Face whose normal is closest to `dir`; ties broken by the largest
/// centroid coordinate along `dir`.
pub fn face_towards(b: &Body, dir: Vec3) -> FaceId {
    let mut best: Option<(f64, FaceId)> = None;
    for f in b.face_ids() {
        let n = b.face_normal(f).expect("normal").as_vec();
        let c = Point3::centroid(&b.face_points(f).expect("points")).expect("centroid");
        let score = (n.dot(dir) * 1000.0).round() * 1e9 + c.to_vec().dot(dir);
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, f));
        }
    }
    best.expect("a face").1
}

/// Shortest edge of a body (mm).
pub fn min_edge(b: &Body) -> f64 {
    b.edges()
        .map(|(_, a, c)| a.distance_to(c))
        .fold(f64::INFINITY, f64::min)
}

/// Invariants every committed document keeps: Forms are structurally
/// valid, closed bodies are outward-facing solids, no edge is shorter than
/// `SHORT_CURVE_TOL` (D-06 rule 3) and all geometry is inside the 10 km
/// extent (D-05).
pub fn check_document(doc: &Document) -> Result<(), String> {
    for e in doc.elements() {
        let Some(b) = e.geometry() else { continue };
        if b.face_count() == 0 {
            continue;
        }
        b.validate().map_err(|x| format!("{}: {x}", e.label()))?;
        if b.is_closed() {
            b.validate_solid()
                .map_err(|x| format!("{} solid: {x}", e.label()))?;
        }
        let m = min_edge(b);
        if m < tol::SHORT_CURVE_TOL - 1e-9 {
            return Err(format!("{}: edge of {m:.4} mm", e.label()));
        }
        if let Some(bb) = b.bounding_box() {
            let lim = crate::model::EXTENT_LIMIT_MM;
            if [bb.min, bb.max]
                .iter()
                .any(|q| q.x.abs() > lim || q.y.abs() > lim || q.z.abs() > lim)
            {
                return Err(format!("{} outside the extent", e.label()));
            }
        }
        if let ElementBody::Form(_) = e.body {
            // Computed parameters follow the body.
            let has_volume = e.param(crate::model::keys::FORM_VOLUME).is_some();
            if has_volume != b.is_closed() {
                return Err(format!("{}: stale volume parameter", e.label()));
            }
        }
    }
    Ok(())
}

/// Execute `tool` and check the outcome contract: success keeps the
/// document invariants; failure returns `ferro.*` error codes and leaves the
/// document, undo stack and journal exactly as they were.
pub fn checked(s: &mut Session, tool: &str, args: Value) -> Result<Outcome, Vec<Failure>> {
    let before = snapshot(s);
    let journal = s.journal().len();
    let undo = s.undo_name().map(str::to_string);
    let r = try_run(s, tool, args.clone());
    match &r {
        Ok(_) => {
            if let Err(m) = check_document(s.doc()) {
                panic!("{tool} {args} committed an invalid document: {m}");
            }
        }
        Err(f) => {
            assert!(!f.is_empty(), "{tool}: empty failure list");
            for x in f {
                assert!(
                    x.code.starts_with("ferro.") && x.code.split('.').count() == 3,
                    "{tool}: bad failure code {:?}",
                    x.code
                );
                assert_eq!(x.severity, Severity::Error);
            }
            assert_eq!(
                snapshot(s),
                before,
                "{tool} {args}: failure changed the document"
            );
            assert_eq!(s.journal().len(), journal, "{tool}: failure journaled");
            assert_eq!(
                s.undo_name().map(str::to_string),
                undo,
                "{tool}: undo stack"
            );
        }
    }
    r
}

/// Execute and expect a failure with `code` (rollback checked).
pub fn expect_code(s: &mut Session, tool: &str, args: Value, code: &str) {
    match checked(s, tool, args.clone()) {
        Ok(_) => panic!("{tool} {args}: expected {code}, it succeeded"),
        Err(f) => assert_eq!(f[0].code, code, "{tool} {args}: {f:?}"),
    }
}

/// Execute and expect success (invariants checked).
pub fn expect_ok(s: &mut Session, tool: &str, args: Value) -> Outcome {
    match checked(s, tool, args.clone()) {
        Ok(o) => o,
        Err(f) => panic!("{tool} {args}: expected success, got {f:?}"),
    }
}

/// Box `w × d × h` with its corner at `(x, y, z)`.
pub fn boxed(s: &mut Session, o: [f64; 3], w: f64, d: f64, h: f64) -> ElementId {
    id(&run(
        s,
        "create_box",
        json!({"origin": p(o[0], o[1], o[2]), "width": w, "depth": d, "height": h}),
    ))
}

/// Tiny deterministic generator (xorshift64*), no dependencies.
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * u
    }
    pub fn pick(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}
