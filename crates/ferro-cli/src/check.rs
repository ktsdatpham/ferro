//! `info` (model summary) and `check` (self-acceptance: regenerate, validate
//! every body, round trip, deterministic export). Output is deterministic:
//! elements in id order, categories sorted, no uids, lengths in mm rounded
//! to 0.001.

use crate::codes;
use ferro_core::math::BoundingBox3;
use ferro_core::model::generate::regenerate_with;
use ferro_core::{Category, Document};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn problem(code: &str, message: impl Into<String>, element: Option<u64>) -> Value {
    json!({"code": code, "message": message.into(), "element": element})
}

/// Validate every element body: `validate_solid` for closed bodies and for
/// walls and floors (always solids), `validate` for the rest. Returns the
/// number of bodies checked and one problem (`ferro.topo.*`) per bad body.
pub fn body_problems(doc: &Document) -> (usize, Vec<Value>) {
    let mut n = 0;
    let mut out = Vec::new();
    for e in doc.elements() {
        let Some(b) = e.geometry() else { continue };
        n += 1;
        let solid = b.is_closed() || matches!(e.category, Category::Walls | Category::Floors);
        let r = if solid {
            b.validate_solid()
        } else {
            b.validate()
        };
        if let Err(err) = r {
            out.push(problem(
                err.code(),
                format!("{} {}: {err}", e.category, e.id),
                Some(e.id.0),
            ));
        }
    }
    (n, out)
}

/// Regeneration errors (their own codes) and elements whose stored geometry
/// differs from the regenerated one (`ferro.cli.stale_geometry`).
/// Regeneration uses the geometry kernel, so `cut` rows (M5-5.3) are
/// applied as in the app; their warnings are reported as warnings.
pub fn regen_problems(doc: &Document) -> (Vec<Value>, Vec<Value>) {
    let kernel = ferro_kernel::SolidcoreKernel::new();
    let r = regenerate_with(doc, Some(&kernel));
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    for i in &r.issues {
        let p = problem(i.code, i.message.clone(), Some(i.element.0));
        if i.is_error {
            problems.push(p);
        } else {
            warnings.push(p);
        }
    }
    for i in &r.cut_issues {
        warnings.push(problem(i.code, i.message.clone(), Some(i.element.0)));
    }
    let cut_forms = r.cut_updates.iter().map(|(id, _)| id);
    for id in r.updates.iter().map(|(id, _)| id).chain(cut_forms) {
        let what = doc
            .get(*id)
            .map(|e| e.category.to_string())
            .unwrap_or_default();
        problems.push(problem(
            codes::STALE_GEOMETRY,
            format!("{what} {id}: saved geometry differs from regenerated geometry"),
            Some(id.0),
        ));
    }
    (problems, warnings)
}

/// Bounding box of all placed geometry (mm), or `None` for a model without.
pub fn bounding_box(doc: &Document) -> Option<BoundingBox3> {
    doc.elements()
        .filter(|e| !e.is_definition())
        .filter_map(|e| e.bounding_box())
        .reduce(BoundingBox3::union)
}

/// `ferro-cli info`: project name, element counts by category (placed
/// elements in `categories`, types and materials in `definitions`), levels,
/// bounding box (mm) and validation problems.
pub fn info(doc: &Document) -> Value {
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut definitions: BTreeMap<String, usize> = BTreeMap::new();
    for e in doc.elements() {
        let map = if e.is_definition() {
            &mut definitions
        } else {
            &mut categories
        };
        *map.entry(e.category.to_string()).or_default() += 1;
    }
    let levels: Vec<Value> = doc
        .levels()
        .iter()
        .map(|l| {
            let elevation = l.as_level().map(|x| x.elevation.mm()).unwrap_or(0.0);
            json!({"id": l.id, "name": l.label(), "elevation_mm": round(elevation)})
        })
        .collect();
    let p = |p: ferro_core::Point3| json!([round(p.x), round(p.y), round(p.z)]);
    let bbox = bounding_box(doc).map(|b| json!({"min": p(b.min), "max": p(b.max)}));
    let (_, mut problems) = regen_problems(doc);
    problems.extend(body_problems(doc).1);
    json!({
        "project": doc.project().name,
        "elements": doc.len(),
        "relationships": doc.relationships().count(),
        "categories": categories,
        "definitions": definitions,
        "levels": levels,
        "bounding_box_mm": bbox,
        "problems": problems,
    })
}

fn entry(name: &str, problems: Vec<Value>) -> Value {
    json!({"check": name, "ok": problems.is_empty(), "problems": problems})
}

fn roundtrip_json(doc: &Document) -> Result<String, Vec<Value>> {
    let fail = |m: String| vec![problem(codes::ROUNDTRIP, m, None)];
    let a = ferro_io::document_to_json(doc).map_err(|e| vec![problem(e.code, e.message, None)])?;
    let back =
        ferro_io::document_from_json(&a).map_err(|e| vec![problem(e.code, e.message, None)])?;
    let b =
        ferro_io::document_to_json(&back).map_err(|e| vec![problem(e.code, e.message, None)])?;
    if a != b {
        return Err(fail(
            "saving, reopening and saving again as .fro.json gives a different file".into(),
        ));
    }
    if back.len() != doc.len() {
        return Err(fail(format!(
            "{} elements saved, {} reopened",
            doc.len(),
            back.len()
        )));
    }
    Ok(a)
}

fn roundtrip_fro(
    doc: &Document,
    journal: &[ferro_core::ops::JournalEntry],
    json: &str,
) -> Vec<Value> {
    let io = |e: ferro_io::IoError| vec![problem(e.code, e.message, None)];
    let reopened = ferro_io::FroStore::open(Path::new(":memory:"))
        .and_then(|mut s| s.write_all(doc, journal).and_then(|_| s.load()));
    match reopened.and_then(|(d, _)| ferro_io::document_to_json(&d)) {
        Ok(t) if t == json => vec![],
        Ok(_) => vec![problem(
            codes::ROUNDTRIP,
            "saving to .fro and reopening changes the document",
            None,
        )],
        Err(e) => io(e),
    }
}

/// An exporter producing file bytes.
type Export = fn(&Document) -> Result<Vec<u8>, ferro_io::IoError>;

fn deterministic(doc: &Document) -> Vec<Value> {
    let mut out = Vec::new();
    let io = |e: ferro_io::IoError| problem(e.code, e.message, None);
    let exports: [(&str, Export); 3] = [
        ("IFC", |d| {
            // The session's kernel kind: design-option combinations keep
            // their cuts (M5-5.6).
            let k: std::sync::Arc<dyn ferro_core::algo::Kernel> =
                std::sync::Arc::new(ferro_kernel::SolidcoreKernel::new());
            ferro_io::export_ifc(d, Some(&k)).map(String::into_bytes)
        }),
        ("OBJ", |d| ferro_io::export_obj(d).map(String::into_bytes)),
        ("GLB", ferro_io::export_glb),
    ];
    for (name, f) in exports {
        match (f(doc), f(doc)) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(_), Ok(_)) => out.push(problem(
                codes::NONDETERMINISTIC,
                format!("two {name} exports of the same document differ"),
                None,
            )),
            (Err(e), _) | (_, Err(e)) => out.push(io(e)),
        }
    }
    out
}

/// `ferro-cli check`: open, regenerate, validate bodies, round trip through
/// `.fro.json` and `.fro` (in memory), export IFC / OBJ / GLB twice and
/// compare. Returns the report and whether every check passed.
pub fn check(path: &Path) -> (Value, bool) {
    let server = match crate::script::open(path) {
        Ok(s) => s,
        Err(e) => {
            let checks = vec![entry("open", vec![problem(&e.code, e.message, None)])];
            return (json!({"file": path, "ok": false, "checks": checks}), false);
        }
    };
    let doc = server.session.doc();
    let mut checks = vec![entry("open", vec![])];
    let (regen, warnings) = regen_problems(doc);
    let mut c = entry("regenerate", regen);
    c["warnings"] = json!(warnings);
    checks.push(c);
    let (n, bodies) = body_problems(doc);
    let mut c = entry("bodies", bodies);
    c["validated"] = json!(n);
    checks.push(c);
    match roundtrip_json(doc) {
        Ok(text) => {
            checks.push(entry("roundtrip_json", vec![]));
            checks.push(entry(
                "roundtrip_fro",
                roundtrip_fro(doc, server.session.journal(), &text),
            ));
        }
        Err(p) => {
            checks.push(entry("roundtrip_json", p));
        }
    }
    checks.push(entry("deterministic_export", deterministic(doc)));
    let ok = checks.iter().all(|c| c["ok"] == true);
    (
        json!({"file": path, "ok": ok, "elements": doc.len(), "checks": checks}),
        ok,
    )
}
