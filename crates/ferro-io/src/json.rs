use crate::IoError;
use ferro_core::model::{Document, Element, ElementId, ProjectInfo, Uid};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Version of the logical file schema (D-21). Bump with a migration.
pub use crate::migrate::FORMAT_VERSION;

const REF_FIELDS: [&str; 6] = [
    "level_id",
    "type_id",
    "host_id",
    "design_option",
    "phase_created",
    "phase_demolished",
];

/// Element references stored inside the category-specific body: a wall's top
/// level and the materials of a type's layers.
fn body_refs(obj: &mut Map<String, Value>) -> Vec<&mut Value> {
    let mut out = Vec::new();
    let Some(body) = obj.get_mut("body").and_then(Value::as_object_mut) else {
        return out;
    };
    let mut top_level = None;
    let mut layers = None;
    for (k, v) in body.iter_mut() {
        match k.as_str() {
            "top" => top_level = v.get_mut("level"),
            "structure" => layers = v.get_mut("layers").and_then(Value::as_array_mut),
            _ => {}
        }
    }
    if let Some(l) = top_level {
        out.push(l);
    }
    if let Some(ls) = layers {
        for l in ls {
            if let Some(m) = l.get_mut("material").filter(|m| !m.is_null()) {
                out.push(m);
            }
        }
    }
    out
}

/// Element → JSON with references rewritten as uids and the local id dropped.
pub(crate) fn element_to_value(
    e: &Element,
    uid_of: &HashMap<ElementId, Uid>,
) -> Result<Value, IoError> {
    let mut v = serde_json::to_value(e)?;
    let obj = v.as_object_mut().expect("element is an object");
    obj.remove("id");
    for f in REF_FIELDS {
        if let Some(r) = obj.get(f).and_then(Value::as_u64) {
            let uid = uid_of.get(&ElementId(r)).ok_or_else(|| {
                IoError::new("ferro.io.dangling_reference", format!("{f} → #{r}"))
            })?;
            obj.insert(f.into(), Value::String(uid.to_string()));
        }
    }
    for r in body_refs(obj) {
        if let Some(id) = r.as_u64() {
            let uid = uid_of.get(&ElementId(id)).ok_or_else(|| {
                IoError::new("ferro.io.dangling_reference", format!("body → #{id}"))
            })?;
            *r = Value::String(uid.to_string());
        }
    }
    if let Some(params) = obj.get_mut("params").and_then(Value::as_object_mut) {
        for p in params.values_mut() {
            if p["type"] == "element_ref" {
                if let Some(r) = p["value"].as_u64() {
                    if let Some(uid) = uid_of.get(&ElementId(r)) {
                        p["value"] = Value::String(uid.to_string());
                    }
                }
            }
        }
    }
    Ok(v)
}

/// JSON values (uid references) → elements with fresh local ids in order.
pub(crate) fn elements_from_values(values: Vec<Value>) -> Result<Vec<Element>, IoError> {
    let mut id_of: HashMap<String, u64> = HashMap::new();
    for (i, v) in values.iter().enumerate() {
        let uid = v["uid"]
            .as_str()
            .ok_or_else(|| IoError::new("ferro.io.schema", "element without uid"))?;
        if id_of.insert(uid.to_string(), i as u64 + 1).is_some() {
            return Err(IoError::new(
                "ferro.io.schema",
                format!("duplicate uid {uid}"),
            ));
        }
    }
    let lookup = |s: &str| -> Result<u64, IoError> {
        id_of.get(s).copied().ok_or_else(|| {
            IoError::new(
                "ferro.io.dangling_reference",
                format!("missing element {s}"),
            )
        })
    };
    let mut out = Vec::with_capacity(values.len());
    for (i, mut v) in values.into_iter().enumerate() {
        let obj = v
            .as_object_mut()
            .ok_or_else(|| IoError::new("ferro.io.schema", "element is not an object"))?;
        obj.insert("id".into(), json!(i as u64 + 1));
        for f in REF_FIELDS {
            if let Some(s) = obj.get(f).and_then(Value::as_str).map(str::to_string) {
                obj.insert(f.into(), json!(lookup(&s)?));
            }
        }
        for r in body_refs(obj) {
            if let Some(s) = r.as_str().map(str::to_string) {
                *r = json!(lookup(&s)?);
            }
        }
        if let Some(params) = obj.get_mut("params").and_then(Value::as_object_mut) {
            for p in params.values_mut() {
                if p["type"] == "element_ref" {
                    if let Some(s) = p["value"].as_str().map(str::to_string) {
                        p["value"] = json!(lookup(&s)?);
                    }
                }
            }
        }
        out.push(serde_json::from_value(v)?);
    }
    Ok(out)
}

pub(crate) fn uid_map(doc: &Document) -> HashMap<ElementId, Uid> {
    doc.elements().map(|e| (e.id, e.uid)).collect()
}

/// Serialise a document to the `.fro.json` interchange format.
pub fn document_to_json(doc: &Document) -> Result<String, IoError> {
    let uids = uid_map(doc);
    let elements = doc
        .elements()
        .map(|e| element_to_value(e, &uids))
        .collect::<Result<Vec<_>, _>>()?;
    let mut root = Map::new();
    root.insert("format".into(), json!("ferro"));
    root.insert("format_version".into(), json!(FORMAT_VERSION));
    root.insert("app_version".into(), json!(env!("CARGO_PKG_VERSION")));
    root.insert("project".into(), serde_json::to_value(doc.project())?);
    root.insert("elements".into(), Value::Array(elements));
    Ok(serde_json::to_string_pretty(&Value::Object(root))?)
}

/// Parse `.fro.json`. Unknown top-level keys are ignored (forward compatible).
pub fn document_from_json(text: &str) -> Result<Document, IoError> {
    let root: Value = serde_json::from_str(text)?;
    if root["format"] != "ferro" {
        return Err(IoError::new("ferro.io.not_ferro", "not a Ferro file"));
    }
    let ver = root["format_version"]
        .as_u64()
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32;
    let mut project_v = root["project"].clone();
    let mut values = root["elements"].as_array().cloned().unwrap_or_default();
    crate::migrate::migrate(ver, &mut project_v, &mut values)?;
    let project: ProjectInfo = serde_json::from_value(project_v)?;
    let elements = elements_from_values(values)?;
    Document::from_parts(project, elements).map_err(|e| IoError::new("ferro.io.schema", e))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;

    pub(crate) fn sample() -> Session {
        let mut s = Session::new(Document::new("Nhà phố"), Box::new(SequentialIds::new(1)));
        let ctx = ExecContext::user();
        for (name, args) in [
            ("create_level", json!({"name":"L1","elevation":0})),
            ("create_level", json!({"name":"L2","elevation":3300})),
            (
                "create_box",
                json!({"origin":{"x":0,"y":0,"z":0},"width":4000,"depth":16000,"height":3300,"level":2,"name":"Tầng 2"}),
            ),
            (
                "set_parameter",
                json!({"element":3,"key":"acme.ref.level","value":{"type":"element_ref","value":1}}),
            ),
        ] {
            s.execute(AnyCommand::from_tool(name, &args).unwrap(), &ctx)
                .unwrap();
        }
        s
    }

    #[test]
    fn json_roundtrip_preserves_document() {
        let s = sample();
        let text = document_to_json(s.doc()).unwrap();
        assert!(
            text.contains("\"level_id\": \""),
            "refs are written as uids"
        );
        let back = document_from_json(&text).unwrap();
        assert_eq!(back.len(), 3);
        for e in s.doc().elements() {
            let b = back.find_uid(e.uid).unwrap();
            assert_eq!(b.name, e.name);
            assert_eq!(b.body, e.body);
            assert_eq!(
                b.level_id.map(|l| back.get(l).unwrap().uid),
                e.level_id.map(|l| s.doc().get(l).unwrap().uid)
            );
        }
    }

    #[test]
    fn bim_elements_roundtrip_with_body_references() {
        let mut s = Session::new(Document::new("BIM"), Box::new(SequentialIds::new(1)));
        let ctx = ExecContext::user();
        // A level first so ids shift when the file is reloaded in uid order.
        for (name, args) in [
            ("create_material", json!({"name":"Brick","color":"#aa5533"})),
            ("create_level", json!({"name":"L1","elevation":0})),
            ("create_level", json!({"name":"Roof","elevation":3200})),
            (
                "create_wall_type",
                json!({"name":"Brick 200","thickness":200,"material":1}),
            ),
            (
                "create_walls",
                json!({"points":[{"x":0,"y":0,"z":0},{"x":6000,"y":0,"z":0},{"x":6000,"y":4000,"z":0}],"level":2,"top_level":3,"wall_type":4}),
            ),
            (
                "create_opening",
                json!({"wall":5,"kind":"window","position":3000}),
            ),
            (
                "create_floor",
                json!({"points":[{"x":0,"y":0},{"x":6000,"y":0},{"x":6000,"y":4000}],"level":2}),
            ),
        ] {
            s.execute(AnyCommand::from_tool(name, &args).unwrap(), &ctx)
                .unwrap_or_else(|f| panic!("{name}: {f:?}"));
        }
        let text = document_to_json(s.doc()).unwrap();
        let back = document_from_json(&text).unwrap();
        assert_eq!(back.len(), s.doc().len());
        for e in s.doc().elements() {
            let b = back.find_uid(e.uid).unwrap();
            assert_eq!(b.category, e.category);
            assert_eq!(b.params, e.params);
            assert_eq!(b.geometry(), e.geometry());
        }
        // The wall's top level and the type's material still point at the
        // same elements by uid.
        let wall = s.doc().elements().find(|e| e.as_wall().is_some()).unwrap();
        let wb = back.find_uid(wall.uid).unwrap();
        let top = |doc: &Document, e: &ferro_core::Element| match &e.as_wall().unwrap().top {
            ferro_core::model::bim::WallTop::Level { level, .. } => doc.get(*level).unwrap().uid,
            _ => panic!("expected level top"),
        };
        assert_eq!(top(s.doc(), wall), top(&back, wb));
        assert_eq!(
            back.get(wb.host_id.unwrap_or(wb.id)).map(|e| e.uid),
            Some(wall.uid)
        );
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert_eq!(
            document_from_json("{}").unwrap_err().code,
            "ferro.io.not_ferro"
        );
        let future = json!({"format":"ferro","format_version":999,"project":{},"elements":[]});
        assert_eq!(
            document_from_json(&future.to_string()).unwrap_err().code,
            "ferro.io.version"
        );
    }
}
