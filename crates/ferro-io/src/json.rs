use crate::IoError;
use ferro_core::model::{Document, Element, ElementId, ProjectInfo, Uid};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Version of the logical file schema (D-21). Bump with a migration.
pub const FORMAT_VERSION: u32 = 1;

const REF_FIELDS: [&str; 6] = [
    "level_id",
    "type_id",
    "host_id",
    "design_option",
    "phase_created",
    "phase_demolished",
];

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
    let ver = root["format_version"].as_u64().unwrap_or(0) as u32;
    if ver == 0 || ver > FORMAT_VERSION {
        return Err(IoError::new(
            "ferro.io.version",
            format!("file format {ver} is newer than this app supports ({FORMAT_VERSION})"),
        ));
    }
    let project: ProjectInfo = serde_json::from_value(root["project"].clone())?;
    let values = root["elements"].as_array().cloned().unwrap_or_default();
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
