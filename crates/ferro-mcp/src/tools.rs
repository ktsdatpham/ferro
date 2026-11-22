//! The tool surface AI agents see. Adding a tool = add it to `list()` and
//! handle it in `call()`. Keep descriptions clear: agents read them.

use ferro_core::{EntityKind, Length, Model, Vec3};
use serde_json::{json, Value};

pub fn list() -> Value {
    let length = json!({
        "type": ["number", "string"],
        "description": "Length. Number = millimetres, or a string with units: \"3m\", \"300cm\", \"10'\"."
    });
    let point = json!({
        "type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3,
        "description": "[x, y, z] in millimetres"
    });

    json!([
        {
            "name": "create_rectangle",
            "description": "Draw a rectangular face on a horizontal plane, like SketchUp's Rectangle tool. Returns the new entity id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "origin": point, "width": length, "depth": length,
                    "name": {"type": "string"}
                },
                "required": ["width", "depth"]
            }
        },
        {
            "name": "create_face",
            "description": "Draw a planar polygon face from 3+ points (millimetres). Returns the new entity id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "points": {"type": "array", "items": point, "minItems": 3},
                    "name": {"type": "string"}
                },
                "required": ["points"]
            }
        },
        {
            "name": "push_pull",
            "description": "Extrude a face along its normal into a solid, like SketchUp's Push/Pull. Negative distance pulls the other way.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "integer"}, "distance": length},
                "required": ["id", "distance"]
            }
        },
        {
            "name": "set_tags",
            "description": "Attach BIM metadata to an entity, e.g. {\"category\":\"wall\",\"level\":\"L1\"}. These map to IFC / Revit categories on export.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer"},
                    "tags": {"type": "object", "additionalProperties": {"type": "string"}}
                },
                "required": ["id", "tags"]
            }
        },
        {
            "name": "delete_entity",
            "description": "Delete an entity by id.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}
        },
        {
            "name": "list_entities",
            "description": "Summarise everything in the model: id, name, type, tags, face count, area.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "export_json",
            "description": "Return the full model as JSON (the native .ferro format).",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn arg_len(args: &Value, key: &str) -> Result<f64, String> {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_f64().ok_or_else(|| format!("bad number for {key}")),
        Some(Value::String(s)) => Length::parse(s)
            .map(|l| l.mm())
            .ok_or_else(|| format!("can't read length {s:?} for {key}")),
        _ => Err(format!("missing {key}")),
    }
}

fn arg_point(v: &Value) -> Result<Vec3, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("point must be [x, y, z]")?;
    let f = |i: usize| a[i].as_f64().ok_or("point coordinates must be numbers");
    Ok(Vec3::new(f(0)?, f(1)?, f(2)?))
}

fn arg_id(args: &Value) -> Result<u64, String> {
    args.get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing id".into())
}

fn set_name(model: &mut Model, id: u64, args: &Value) {
    if let (Some(name), Some(e)) = (args.get("name").and_then(Value::as_str), model.get_mut(id)) {
        e.name = Some(name.to_string());
    }
}

/// Run a tool. Ok = text result for the agent, Err = tool error message.
pub fn call(model: &mut Model, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "create_rectangle" => {
            let origin = match args.get("origin") {
                Some(p) => arg_point(p)?,
                None => Vec3::ZERO,
            };
            let id = model
                .add_rectangle(origin, arg_len(args, "width")?, arg_len(args, "depth")?)
                .map_err(|e| e.to_string())?;
            set_name(model, id, args);
            Ok(json!({ "id": id }).to_string())
        }
        "create_face" => {
            let pts = args
                .get("points")
                .and_then(Value::as_array)
                .ok_or("missing points")?
                .iter()
                .map(arg_point)
                .collect::<Result<Vec<_>, _>>()?;
            let id = model.add_face(pts).map_err(|e| e.to_string())?;
            set_name(model, id, args);
            Ok(json!({ "id": id }).to_string())
        }
        "push_pull" => {
            let id = model
                .push_pull(arg_id(args)?, arg_len(args, "distance")?)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "id": id, "type": "solid" }).to_string())
        }
        "set_tags" => {
            let id = arg_id(args)?;
            let tags = args
                .get("tags")
                .and_then(Value::as_object)
                .ok_or("missing tags")?;
            let e = model.get_mut(id).ok_or(format!("entity {id} not found"))?;
            for (k, v) in tags {
                e.tags.insert(
                    k.clone(),
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string()),
                );
            }
            Ok(json!({ "id": id, "tags": e.tags }).to_string())
        }
        "delete_entity" => {
            let id = arg_id(args)?;
            model.remove(id).map_err(|e| e.to_string())?;
            Ok(json!({ "deleted": id }).to_string())
        }
        "list_entities" => {
            let items: Vec<Value> = model
                .entities()
                .map(|e| {
                    let (kind, faces, area) = match &e.kind {
                        EntityKind::Face(f) => ("face", 1, f.area()),
                        EntityKind::Solid(s) => (
                            "solid",
                            s.faces.len(),
                            s.faces.iter().map(|f| f.area()).sum(),
                        ),
                    };
                    json!({
                        "id": e.id, "name": e.name, "type": kind, "tags": e.tags,
                        "faces": faces, "area_m2": (area / 1e6 * 1000.0).round() / 1000.0
                    })
                })
                .collect();
            Ok(Value::Array(items).to_string())
        }
        "export_json" => Ok(model.to_json()),
        other => Err(format!("unknown tool: {other}")),
    }
}
