//! `ferro-cli fingerprint`: a canonical description of a model for the
//! regression corpus (M5-17, SolidCraft's naming corpus).
//!
//! Elements are keyed by their local id, which is deterministic for a script
//! (uids are random, so they are left out). For every element with geometry
//! the fingerprint lists each face by its persistent id with its vertex
//! count, area and normal, plus counts, volume and bounding box. A kernel
//! change that renames faces, loses lineage or changes a quantity shows up
//! as a difference. Lengths in mm, areas in mm², volumes in mm³.

use ferro_core::model::{Document, Element};
use serde_json::{json, Map, Value};

/// Relative tolerance used by `--expect` (numbers are rounded to 0.001 too).
pub const TOLERANCE: f64 = 1e-6;

/// Numbers are rounded to this many decimals so the text is stable across
/// platforms; [`same`] compares with a tolerance on top.
fn r(x: f64) -> f64 {
    let v = (x * 1e3).round() / 1e3;
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

fn element(doc: &Document, e: &Element) -> Value {
    let mut m = Map::new();
    m.insert("category".into(), json!(e.category));
    m.insert("body".into(), json!(e.body.kind()));
    if let Some(n) = &e.name {
        m.insert("name".into(), json!(n));
    }
    for (k, v) in [
        ("level", e.level_id),
        ("type", e.type_id),
        ("host", e.host_id),
        ("phase_created", e.phase_created),
        ("phase_demolished", e.phase_demolished),
    ] {
        if let Some(id) = v {
            m.insert(k.into(), json!(id));
        }
    }
    if let Some(g) = e.geometry() {
        let faces: Vec<Value> = g
            .face_ids()
            .map(|f| {
                let n = g.face_normal(f).map(|u| u.as_vec()).unwrap_or_default();
                json!({
                    "face": f,
                    "vertices": g.face_points(f).map(|p| p.len()).unwrap_or(0),
                    "area": r(g.face_area(f).unwrap_or(0.0)),
                    "normal": [r(n.x), r(n.y), r(n.z)],
                })
            })
            .collect();
        m.insert("faces".into(), Value::Array(faces));
        m.insert(
            "counts".into(),
            json!({"faces": g.face_count(), "edges": g.edge_count(), "vertices": g.vertex_count()}),
        );
        m.insert("closed".into(), json!(g.is_closed()));
        if g.is_closed() {
            m.insert("volume".into(), json!(r(g.volume())));
        }
        if let Some(b) = g.bounding_box() {
            m.insert(
                "bounding_box".into(),
                json!([
                    [r(b.min.x), r(b.min.y), r(b.min.z)],
                    [r(b.max.x), r(b.max.y), r(b.max.z)]
                ]),
            );
        }
    }
    let rels: Vec<Value> = doc
        .relationships_of(e.id)
        .map(|x| json!({"kind": x.kind.name(), "elements": x.elements}))
        .collect();
    if !rels.is_empty() {
        m.insert("relationships".into(), Value::Array(rels));
    }
    Value::Object(m)
}

/// The fingerprint of a document: `{elements: {"<id>": {...}}}` in id
/// order, plus the project's settings.
pub fn fingerprint(doc: &Document) -> Value {
    let mut els = Map::new();
    for e in doc.elements() {
        els.insert(e.id.0.to_string(), element(doc, e));
    }
    json!({
        "elements": Value::Object(els),
        "settings": doc.project().settings,
    })
}

/// Differences between two fingerprints as `path: expected → actual` lines;
/// numbers equal within `tol` (absolute, or relative for big values) count
/// as the same. Empty = same.
pub fn differences(expected: &Value, actual: &Value, tol: f64) -> Vec<String> {
    let mut out = Vec::new();
    walk("", expected, actual, tol, &mut out);
    out
}

fn walk(path: &str, a: &Value, b: &Value, tol: f64, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = format!("{path}/{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => walk(&p, u, v, tol, out),
                    (Some(u), None) => out.push(format!("{p}: {u} → (missing)")),
                    (None, Some(v)) => out.push(format!("{p}: (missing) → {v}")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: {} items → {}", x.len(), y.len()));
            }
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                walk(&format!("{path}/{i}"), u, v, tol, out);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            let (u, v) = (
                x.as_f64().unwrap_or(f64::NAN),
                y.as_f64().unwrap_or(f64::NAN),
            );
            let same = (u - v).abs() <= tol * u.abs().max(v.abs()).max(1.0);
            if !same {
                out.push(format!("{path}: {u} → {v}"));
            }
        }
        _ if a == b => {}
        _ => out.push(format!("{path}: {a} → {b}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_with_tolerance_and_structure_exactly() {
        let a = json!({"x": 1000.0, "f": [1, 2], "k": "a"});
        assert!(
            differences(&a, &json!({"x": 1000.0000001, "f": [1, 2], "k": "a"}), 1e-9).is_empty()
        );
        let d = differences(&a, &json!({"x": 1001.0, "f": [1], "k": "b", "n": 1}), 1e-9);
        assert_eq!(d.len(), 4, "{d:?}");
        assert_eq!(r(-0.0001), 0.0, "no negative zero");
    }
}
