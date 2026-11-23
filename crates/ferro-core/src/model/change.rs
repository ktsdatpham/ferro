//! The change format (M5-26, F10 of the foundation review, D-34, D-55,
//! ADR 0006): every property of a document has a stable path, and a change
//! is a list of edits on those paths. Diff, merge, sync between computers
//! and design-option layers all use it.
//!
//! * A **path** is a JSON Pointer (RFC 6901) into the document's canonical
//!   JSON: `/project/name`, `/project/settings/currency`,
//!   `/elements/<uid>` (a whole element), `/elements/<uid>/params/<key>`,
//!   `/elements/<uid>/body/top/height`. Elements are addressed by `Uid`,
//!   never by the document-local `ElementId`, and references between
//!   elements are written as uids, so a change means the same thing in
//!   every copy of a project.
//! * An **edit** sets a value at a path or removes it.
//! * [`diff`] gives the edits from one document to another, deterministic
//!   (sorted); [`apply`] replays them; [`conflicts`] finds where two change
//!   lists touch the same property (or one touches what the other removes).
//!
//! Arrays (points of an outline, layers of a type) are replaced whole: a
//! property is the smallest unit two people can change independently.

use super::{Document, Element, ElementId, ProjectInfo, Relationship, Uid};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

/// Fields of an element that hold another element's id.
pub const REF_FIELDS: [&str; 6] = [
    "level_id",
    "type_id",
    "host_id",
    "design_option",
    "phase_created",
    "phase_demolished",
];

/// A failed diff or apply, with a stable code (`ferro.change.*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeError {
    pub code: &'static str,
    pub message: String,
}

impl ChangeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ChangeError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// One edit: set the value at `path`, or remove it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Set { path: String, value: Value },
    Remove { path: String },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Set { path, .. } | Change::Remove { path } => path,
        }
    }
}

/// Call `f` on every place in an element's JSON that holds an element
/// reference: the [`REF_FIELDS`], a wall's top level, the materials of a
/// type's layers, and `element_ref` parameters. Null slots are skipped.
/// The flag passed to `f` is `true` for `element_ref` parameters: those may
/// point at an element that was deleted since, and are kept as they are
/// instead of failing.
pub fn visit_element_refs<E>(
    obj: &mut Map<String, Value>,
    f: &mut dyn FnMut(&mut Value, bool) -> Result<(), E>,
) -> Result<(), E> {
    for k in REF_FIELDS {
        if let Some(v) = obj.get_mut(k).filter(|v| !v.is_null()) {
            f(v, false)?;
        }
    }
    if let Some(body) = obj.get_mut("body").and_then(Value::as_object_mut) {
        if let Some(v) = body
            .get_mut("top")
            .and_then(|t| t.get_mut("level"))
            .filter(|v| !v.is_null())
        {
            f(v, false)?;
        }
        if let Some(layers) = body
            .get_mut("structure")
            .and_then(|s| s.get_mut("layers"))
            .and_then(Value::as_array_mut)
        {
            for l in layers {
                if let Some(m) = l.get_mut("material").filter(|m| !m.is_null()) {
                    f(m, false)?;
                }
            }
        }
    }
    if let Some(params) = obj.get_mut("params").and_then(Value::as_object_mut) {
        for p in params.values_mut() {
            if p["type"] == "element_ref" {
                if let Some(v) = p.get_mut("value").filter(|v| !v.is_null()) {
                    f(v, true)?;
                }
            }
        }
    }
    Ok(())
}

/// An element as canonical JSON: local `id` dropped, references as uids.
/// Fails with `ferro.change.dangling_reference` when a reference points at
/// no element of `doc`.
pub fn element_value(doc: &Document, e: &Element) -> Result<Value, ChangeError> {
    let mut v = serde_json::to_value(e)
        .map_err(|err| ChangeError::new("ferro.change.encode", err.to_string()))?;
    let Some(obj) = v.as_object_mut() else {
        return Err(ChangeError::new(
            "ferro.change.encode",
            "element is not an object",
        ));
    };
    obj.remove("id");
    visit_element_refs(obj, &mut |r: &mut Value, lenient| {
        let id = r.as_u64().ok_or_else(|| {
            ChangeError::new("ferro.change.encode", format!("reference {r} is not an id"))
        })?;
        let found = doc.get(ElementId(id)).map(|t| t.uid);
        if found.is_none() && lenient {
            return Ok(());
        }
        let uid = found.ok_or_else(|| {
            ChangeError::new(
                "ferro.change.dangling_reference",
                format!("{} refers to missing #{id}", e.label()),
            )
        })?;
        *r = Value::String(uid.to_string());
        Ok(())
    })?;
    Ok(v)
}

/// A relationship row as JSON with its element and phase references as
/// uids (`uid_of` maps a local id to its uid). Fails with
/// `ferro.change.dangling_reference` when a reference has no uid.
pub fn relationship_value(
    r: &Relationship,
    uid_of: &dyn Fn(ElementId) -> Option<Uid>,
) -> Result<Value, ChangeError> {
    let to_uid = |id: ElementId| {
        uid_of(id)
            .map(|u| Value::String(u.to_string()))
            .ok_or_else(|| {
                ChangeError::new(
                    "ferro.change.dangling_reference",
                    format!("relationship {} refers to missing #{id}", r.uid),
                )
            })
    };
    let mut m = Map::new();
    m.insert("uid".into(), Value::String(r.uid.to_string()));
    m.insert("kind".into(), Value::String(r.kind.name().to_string()));
    m.insert(
        "elements".into(),
        Value::Array(
            r.elements
                .iter()
                .map(|e| to_uid(*e))
                .collect::<Result<_, _>>()?,
        ),
    );
    if let Some(p) = r.phase {
        m.insert("phase".into(), to_uid(p)?);
    }
    Ok(Value::Object(m))
}

/// Read a row written by [`relationship_value`]; `id_of` maps a uid string
/// to the local id. Fails with `ferro.change.invalid` on bad data and
/// `ferro.change.dangling_reference` on an unknown uid.
pub fn relationship_from_value(
    v: &Value,
    id_of: &dyn Fn(&str) -> Option<ElementId>,
) -> Result<Relationship, ChangeError> {
    let invalid = |m: String| ChangeError::new("ferro.change.invalid", m);
    let to_id = |x: &Value| -> Result<ElementId, ChangeError> {
        let s = x
            .as_str()
            .ok_or_else(|| invalid(format!("relationship reference {x} is not a uid")))?;
        id_of(s).ok_or_else(|| {
            ChangeError::new(
                "ferro.change.dangling_reference",
                format!("relationship refers to missing {s}"),
            )
        })
    };
    let uid: Uid = serde_json::from_value(v["uid"].clone())
        .map_err(|e| invalid(format!("relationship uid: {e}")))?;
    let kind = serde_json::from_value(v["kind"].clone())
        .map_err(|e| invalid(format!("relationship {uid} kind: {e}")))?;
    let elements = v["elements"]
        .as_array()
        .ok_or_else(|| invalid(format!("relationship {uid} has no elements")))?
        .iter()
        .map(to_id)
        .collect::<Result<_, _>>()?;
    let phase = match v.get("phase") {
        None | Some(Value::Null) => None,
        Some(p) => Some(to_id(p)?),
    };
    Ok(Relationship {
        uid,
        kind,
        elements,
        phase,
    })
}

/// The document as `{project, elements: {uid: element}, relationships:
/// {uid: row}}`.
fn document_value(doc: &Document) -> Result<Value, ChangeError> {
    let mut elements = Map::new();
    for e in doc.elements() {
        elements.insert(e.uid.to_string(), element_value(doc, e)?);
    }
    let mut rows = Map::new();
    let uid_of = |id: ElementId| doc.get(id).map(|e| e.uid);
    for r in doc.relationships() {
        rows.insert(r.uid.to_string(), relationship_value(r, &uid_of)?);
    }
    let project = serde_json::to_value(doc.project())
        .map_err(|err| ChangeError::new("ferro.change.encode", err.to_string()))?;
    let mut root = Map::new();
    root.insert("project".into(), project);
    root.insert("elements".into(), Value::Object(elements));
    root.insert("relationships".into(), Value::Object(rows));
    Ok(Value::Object(root))
}

/// RFC 6901 escaping of one path segment (`~` → `~0`, `/` → `~1`).
pub fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

fn segments(path: &str) -> Result<Vec<String>, ChangeError> {
    let rest = path.strip_prefix('/').ok_or_else(|| {
        ChangeError::new(
            "ferro.change.bad_path",
            format!("`{path}` must start with /"),
        )
    })?;
    if rest.is_empty() {
        return Err(ChangeError::new(
            "ferro.change.bad_path",
            "the whole document cannot be replaced",
        ));
    }
    Ok(rest.split('/').map(unescape).collect())
}

fn diff_values(path: &str, a: &Value, b: &Value, out: &mut Vec<Change>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            // BTreeMap order keeps the output deterministic.
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = format!("{path}/{}", escape(k));
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => diff_values(&p, u, v, out),
                    (Some(_), None) => out.push(Change::Remove { path: p }),
                    (None, Some(v)) => out.push(Change::Set {
                        path: p,
                        value: v.clone(),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ if a == b => {}
        _ => out.push(Change::Set {
            path: path.to_string(),
            value: b.clone(),
        }),
    }
}

/// The edits that turn `a` into `b`, sorted by path. `diff(a, a)` is empty.
/// Added and removed elements are one edit each (`/elements/<uid>`);
/// changed elements give one edit per changed property.
pub fn diff(a: &Document, b: &Document) -> Result<Vec<Change>, ChangeError> {
    let (va, vb) = (document_value(a)?, document_value(b)?);
    let mut out = Vec::new();
    diff_values("", &va, &vb, &mut out);
    Ok(out)
}

fn apply_value(root: &mut Value, change: &Change) -> Result<(), ChangeError> {
    let segs = segments(change.path())?;
    let (last, parents) = segs
        .split_last()
        .ok_or_else(|| ChangeError::new("ferro.change.bad_path", "empty path"))?;
    let mut cur = root;
    for s in parents {
        cur = cur.get_mut(s.as_str()).ok_or_else(|| {
            ChangeError::new(
                "ferro.change.missing_parent",
                format!("`{}`: `{s}` does not exist", change.path()),
            )
        })?;
    }
    let obj = cur.as_object_mut().ok_or_else(|| {
        ChangeError::new(
            "ferro.change.bad_path",
            format!(
                "`{}` goes through a value that is not an object",
                change.path()
            ),
        )
    })?;
    match change {
        Change::Set { value, .. } => {
            obj.insert(last.clone(), value.clone());
        }
        Change::Remove { path } => {
            obj.remove(last.as_str()).ok_or_else(|| {
                ChangeError::new("ferro.change.missing", format!("`{path}` does not exist"))
            })?;
        }
    }
    Ok(())
}

/// Apply edits to `doc`. Elements keep their `ElementId`; new elements get
/// ids after the highest one, in uid order. Fails (and changes nothing)
/// with `ferro.change.*` on a bad path, a missing target, a reference to an
/// element that does not exist, or data that is not a valid element.
pub fn apply(doc: &Document, changes: &[Change]) -> Result<Document, ChangeError> {
    let mut root = document_value(doc)?;
    for c in changes {
        apply_value(&mut root, c)?;
    }
    let project: ProjectInfo = serde_json::from_value(root["project"].clone())
        .map_err(|e| ChangeError::new("ferro.change.invalid", format!("project: {e}")))?;
    let elements = root["elements"]
        .as_object()
        .cloned()
        .ok_or_else(|| ChangeError::new("ferro.change.invalid", "elements is not an object"))?;
    // Ids: kept for existing uids, fresh for new ones.
    let old: HashMap<String, ElementId> =
        doc.elements().map(|e| (e.uid.to_string(), e.id)).collect();
    let mut next = doc.elements().map(|e| e.id.0).max().unwrap_or(0) + 1;
    let mut id_of: BTreeMap<String, u64> = BTreeMap::new();
    for uid in elements.keys() {
        let id = match old.get(uid) {
            Some(i) => i.0,
            None => {
                next += 1;
                next - 1
            }
        };
        id_of.insert(uid.clone(), id);
    }
    let mut out = Vec::with_capacity(elements.len());
    for (uid, mut v) in elements {
        let obj = v.as_object_mut().ok_or_else(|| {
            ChangeError::new(
                "ferro.change.invalid",
                format!("element {uid} is not an object"),
            )
        })?;
        obj.insert("id".into(), Value::from(id_of[&uid]));
        if obj.get("uid").and_then(Value::as_str) != Some(uid.as_str()) {
            return Err(ChangeError::new(
                "ferro.change.invalid",
                format!("element at /elements/{uid} has a different uid"),
            ));
        }
        visit_element_refs(obj, &mut |r: &mut Value, lenient| {
            if lenient && r.is_u64() {
                return Ok(());
            }
            let s = r.as_str().ok_or_else(|| {
                ChangeError::new(
                    "ferro.change.invalid",
                    format!("reference {r} is not a uid"),
                )
            })?;
            let id = id_of.get(s).ok_or_else(|| {
                ChangeError::new(
                    "ferro.change.dangling_reference",
                    format!("{uid} refers to missing {s}"),
                )
            })?;
            *r = Value::from(*id);
            Ok(())
        })?;
        let e: Element = serde_json::from_value(v)
            .map_err(|err| ChangeError::new("ferro.change.invalid", format!("{uid}: {err}")))?;
        let _: Uid = e.uid;
        out.push(e);
    }
    let rows = match root.get("relationships") {
        Some(Value::Object(m)) => m.clone(),
        Some(_) => {
            return Err(ChangeError::new(
                "ferro.change.invalid",
                "relationships is not an object",
            ))
        }
        None => Map::new(),
    };
    let lookup = |s: &str| id_of.get(s).map(|i| ElementId(*i));
    let mut table = Vec::with_capacity(rows.len());
    for (uid, v) in &rows {
        let r = relationship_from_value(v, &lookup)?;
        if r.uid.to_string() != *uid {
            return Err(ChangeError::new(
                "ferro.change.invalid",
                format!("relationship at /relationships/{uid} has a different uid"),
            ));
        }
        table.push(r);
    }
    Document::from_parts(project, out)
        .and_then(|d| d.with_relationships(table))
        .map_err(|m| ChangeError::new("ferro.change.invalid", m))
}

/// Paths where `ours` and `theirs` disagree: both edit the same path with
/// different results, or one edits inside (or above) a path the other
/// edits. Identical edits on both sides are not conflicts. Sorted, unique.
pub fn conflicts(ours: &[Change], theirs: &[Change]) -> Vec<String> {
    let related = |p: &str, q: &str| {
        p == q || q.starts_with(&format!("{p}/")) || p.starts_with(&format!("{q}/"))
    };
    let mut out: Vec<String> = Vec::new();
    for a in ours {
        for b in theirs {
            if related(a.path(), b.path()) && a != b {
                let p = if a.path().len() <= b.path().len() {
                    a.path()
                } else {
                    b.path()
                };
                out.push(p.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_with_slashes_and_tildes_are_escaped() {
        assert_eq!(escape("a/b~c"), "a~1b~0c");
        assert_eq!(unescape(&escape("a/b~c")), "a/b~c");
        assert_eq!(unescape("~01"), "~1", "decode ~1 before ~0");
    }
}
