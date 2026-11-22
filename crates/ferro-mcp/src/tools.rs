//! MCP tool surface (D-17, D-44): every kernel command is a tool with the same
//! name; a few read-only and file tools are added. Descriptions are written
//! for an AI reader. User units are converted here, at the edge (D-01, D-02):
//! lengths accept numbers (mm) or strings ("3m", "300cm", "10'"), angles are
//! in degrees, points are [x, y, z] in mm.

use ferro_core::model::{keys, Category, ElementBody};
use ferro_core::ops::{Actor, AnyCommand, ExecContext, Failure};
use ferro_core::{Angle, Document, ElementId, Length, Session};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const LENGTH_FIELDS: [&str; 5] = ["width", "depth", "height", "elevation", "distance"];
const POINT_FIELDS: [&str; 3] = ["origin", "center", "vector"];

fn length() -> Value {
    json!({"type": ["number", "string"], "description": "Length: number = millimetres, or a string with units such as \"3m\", \"300cm\", \"10'\"."})
}
fn point(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": format!("{desc} [x, y, z] in millimetres. Z is up, X east, Y north.")})
}
fn id(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}
fn ids(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": "integer"}, "description": desc})
}
fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

pub fn list() -> Vec<Value> {
    vec![
        tool("create_level", "Create a level (named elevation, e.g. a storey). Elements placed on a level move with it.",
            json!({"name": {"type":"string"}, "elevation": length(), "is_building_storey": {"type":"boolean","description":"Default true."}}), &["name", "elevation"]),
        tool("set_level_elevation", "Change a level's elevation. Forms on that level move with it.",
            json!({"level": id("Level element id"), "elevation": length()}), &["level", "elevation"]),
        tool("create_rectangle", "Draw a horizontal rectangular face (SketchUp Rectangle tool). Returns {element, face}. With `level`, origin z is relative to the level elevation.",
            json!({"origin": point("Corner"), "width": length(), "depth": length(), "level": id("Optional level id"), "name": {"type":"string"}}), &["origin", "width", "depth"]),
        tool("create_polygon", "Draw a planar polygon face from 3+ points. Returns {element, face}.",
            json!({"points": {"type":"array","items": point("Vertex"), "minItems": 3}, "level": id("Optional level id"), "name": {"type":"string"}, "relative_to_level": {"type":"boolean"}}), &["points"]),
        tool("create_box", "Create a box (rectangle pulled up). Fastest way to block out a volume. Returns {element}.",
            json!({"origin": point("Bottom corner"), "width": length(), "depth": length(), "height": length(), "level": id("Optional level id"), "name": {"type":"string"}}), &["origin", "width", "depth", "height"]),
        tool("push_pull_face", "Push / pull a face of a Form along its normal (SketchUp Push/Pull). Negative distance pushes inward. Get face ids from get_element.",
            json!({"element": id("Form element id"), "face": id("Face id within the Form"), "distance": length()}), &["element", "face", "distance"]),
        tool("move_elements", "Move elements by a vector.",
            json!({"elements": ids("Element ids"), "vector": point("Displacement")}), &["elements", "vector"]),
        tool("rotate_elements", "Rotate elements about a vertical axis through `center`. Angle in degrees, counter-clockwise positive.",
            json!({"elements": ids("Element ids"), "center": point("Axis point"), "angle": {"type":"number","description":"Degrees"}}), &["elements", "center", "angle"]),
        tool("copy_elements", "Copy elements offset by a vector; count > 1 makes a linear array. Returns new ids.",
            json!({"elements": ids("Element ids"), "vector": point("Offset per copy"), "count": {"type":"integer","minimum":1,"maximum":1000}}), &["elements", "vector"]),
        tool("delete_elements", "Delete elements. References from other elements are cleared with a warning.",
            json!({"elements": ids("Element ids")}), &["elements"]),
        tool("set_name", "Rename an element (null clears).",
            json!({"element": id("Element id"), "name": {"type":["string","null"]}}), &["element", "name"]),
        tool("set_parameter", "Set a typed parameter. Key format namespace.group.name (e.g. ferro.common.comments, acme.fengshui.score). Value: {\"type\": \"length\"|\"angle\"|\"area\"|\"volume\"|\"number\"|\"integer\"|\"bool\"|\"text\"|\"tags\"|\"element_ref\", \"value\": ...}. Lengths in mm, angles in radians here. Computed keys (ferro.form.area, ferro.form.volume) are read-only.",
            json!({"element": id("Element id"), "key": {"type":"string"}, "value": {"type":"object"}}), &["element", "key", "value"]),
        tool("set_category", "Classify a Form as a BIM category so it exports to Revit / IFC correctly.",
            json!({"element": id("Form element id"), "category": {"type":"string","enum": Category::ALL.iter().filter(|c| c.classifiable_form()).map(|c| c.to_string()).collect::<Vec<_>>()}}), &["element", "category"]),
        tool("set_project_name", "Rename the project.", json!({"name":{"type":"string"}}), &["name"]),
        // Read-only and session tools.
        tool("list_elements", "Summarise the model: id, uid, name, category, level, faces, area (m²), volume (m³). Optional category filter.",
            json!({"category": {"type":"string"}}), &[]),
        tool("get_element", "Full detail of one element, including each face id with its normal, area and centre (use for push_pull_face).",
            json!({"element": id("Element id")}), &["element"]),
        tool("undo", "Undo the last change.", json!({}), &[]),
        tool("redo", "Redo the last undone change.", json!({}), &[]),
        tool("get_journal", "Recent history: who changed what, with which commands.", json!({"limit":{"type":"integer"}}), &[]),
        tool("new_document", "Start a new empty project.", json!({"name":{"type":"string"}}), &[]),
        tool("open_document", "Open a .fro or .fro.json file.", json!({"path":{"type":"string"}}), &["path"]),
        tool("save_document", "Save to .fro (SQLite) or .fro.json. Without path, saves to the opened file.", json!({"path":{"type":"string"}}), &[]),
        tool("export_obj", "Export Forms to Wavefront OBJ (Y-up, metres).", json!({"path":{"type":"string"}}), &["path"]),
        tool("export_glb", "Export Forms to binary glTF (Y-up, metres).", json!({"path":{"type":"string"}}), &["path"]),
    ]
}

/// Convert agent-friendly arguments to kernel units.
pub fn normalize(args: &Value) -> Result<Value, String> {
    let mut obj = args.as_object().cloned().unwrap_or_default();
    for f in LENGTH_FIELDS {
        if let Some(v) = obj.get(f).cloned() {
            obj.insert(f.into(), json!(parse_length(&v, f)?));
        }
    }
    for f in POINT_FIELDS {
        if let Some(v) = obj.get(f).cloned() {
            obj.insert(f.into(), parse_point(&v, f)?);
        }
    }
    if let Some(Value::Array(pts)) = obj.get("points").cloned() {
        let conv = pts
            .iter()
            .map(|p| parse_point(p, "points"))
            .collect::<Result<Vec<_>, _>>()?;
        obj.insert("points".into(), Value::Array(conv));
    }
    if let Some(v) = obj.get("angle").cloned() {
        let deg = match &v {
            Value::Number(n) => n.as_f64().ok_or("angle must be a number")?,
            Value::String(s) => Angle::parse(s).ok_or("cannot read angle")?.deg(),
            _ => return Err("angle must be degrees".into()),
        };
        obj.insert("angle".into(), json!(Angle::from_deg(deg).rad()));
    }
    Ok(Value::Object(obj))
}

fn parse_length(v: &Value, field: &str) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("bad number for {field}")),
        Value::String(s) => Length::parse(s)
            .map(|l| l.mm())
            .ok_or_else(|| format!("cannot read length {s:?} for {field}")),
        _ => Err(format!("{field} must be a length")),
    }
}

fn parse_point(v: &Value, field: &str) -> Result<Value, String> {
    let get = |x: &Value| {
        x.as_f64()
            .ok_or_else(|| format!("{field}: coordinates must be numbers"))
    };
    match v {
        Value::Array(a) if a.len() == 3 => {
            Ok(json!({"x": get(&a[0])?, "y": get(&a[1])?, "z": get(&a[2])?}))
        }
        Value::Object(o) => Ok(json!({
            "x": get(o.get("x").unwrap_or(&json!(0)))?,
            "y": get(o.get("y").unwrap_or(&json!(0)))?,
            "z": get(o.get("z").unwrap_or(&json!(0)))?,
        })),
        _ => Err(format!("{field} must be [x, y, z]")),
    }
}

fn failures_json(f: &[Failure]) -> Value {
    json!(f.iter().map(|x| json!({"severity": x.severity, "code": x.code, "message": x.message, "elements": x.elements})).collect::<Vec<_>>())
}

fn summary(doc: &Document, category: Option<&str>) -> Value {
    let items: Vec<Value> = doc
        .elements()
        .filter(|e| category.is_none_or(|c| e.category.to_string() == c))
        .map(|e| {
            let mut m = Map::new();
            m.insert("id".into(), json!(e.id));
            m.insert("uid".into(), json!(e.uid.to_string()));
            m.insert("name".into(), json!(e.name));
            m.insert("category".into(), json!(e.category));
            m.insert("level".into(), json!(e.level_id));
            match &e.body {
                ElementBody::Level(l) => {
                    m.insert("elevation_mm".into(), json!(l.elevation.mm()));
                }
                ElementBody::Form(f) => {
                    m.insert("faces".into(), json!(f.body.face_count()));
                    m.insert("closed".into(), json!(f.body.is_closed()));
                    if let Some(ferro_core::ParamValue::Area(a)) = e.param(keys::FORM_AREA) {
                        m.insert("area_m2".into(), json!((a / 1e6 * 1000.0).round() / 1000.0));
                    }
                    if let Some(ferro_core::ParamValue::Volume(v)) = e.param(keys::FORM_VOLUME) {
                        m.insert(
                            "volume_m3".into(),
                            json!((v / 1e9 * 1000.0).round() / 1000.0),
                        );
                    }
                }
            }
            Value::Object(m)
        })
        .collect();
    json!({"project": doc.project().name, "elements": items})
}

fn detail(doc: &Document, id: ElementId) -> Result<Value, String> {
    let e = doc
        .get(id)
        .ok_or_else(|| format!("element {id} not found"))?;
    let mut v = summary_one(e);
    if let ElementBody::Form(f) = &e.body {
        let faces: Vec<Value> = f
            .body
            .face_ids()
            .map(|fid| {
                let n = f.body.face_normal(fid).map(|u| u.as_vec()).unwrap_or_default();
                let pts = f.body.face_points(fid).unwrap_or_default();
                let c = ferro_core::Point3::centroid(&pts).unwrap_or_default();
                json!({"face": fid, "normal": [round(n.x), round(n.y), round(n.z)], "area_m2": (f.body.face_area(fid).unwrap_or(0.0) / 1e6 * 1000.0).round() / 1000.0, "center_mm": [round(c.x), round(c.y), round(c.z)]})
            })
            .collect();
        v["faces"] = json!(faces);
        if let Some(bb) = f.body.bounding_box() {
            v["bounding_box_mm"] = json!({"min": [bb.min.x, bb.min.y, bb.min.z], "max": [bb.max.x, bb.max.y, bb.max.z]});
        }
    }
    v["parameters"] = json!(e.params);
    Ok(v)
}

fn summary_one(e: &ferro_core::Element) -> Value {
    json!({"id": e.id, "uid": e.uid.to_string(), "name": e.name, "category": e.category, "level": e.level_id})
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Server state: one session (the open project) plus its file path.
pub struct Server {
    pub session: Session,
    pub path: Option<PathBuf>,
    pub agent: Actor,
}

impl Server {
    pub fn new() -> Server {
        Server {
            session: Session::new(Document::new("Untitled"), Box::new(ferro_io::SystemIds)),
            path: None,
            agent: Actor::agent("mcp-client", "unknown"),
        }
    }

    fn ctx(&self) -> ExecContext {
        ExecContext {
            actor: self.agent.clone(),
            prompt: None,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Run a tool. Ok = JSON result, Err = (message, failures JSON).
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Value, Value> {
        let err = |m: String| Err(json!({"error": m}));
        if AnyCommand::NAMES.contains(&name) {
            let norm = match normalize(args) {
                Ok(v) => v,
                Err(m) => return err(m),
            };
            let cmd = match AnyCommand::from_tool(name, &norm) {
                Ok(c) => c,
                Err(m) => return err(m),
            };
            let ctx = self.ctx();
            return match self.session.execute(cmd, &ctx) {
                Ok(o) => Ok(
                    json!({"result": o.output, "changed": o.changed, "warnings": failures_json(&o.warnings)}),
                ),
                Err(f) => Err(
                    json!({"error": "command failed; nothing was changed", "failures": failures_json(&f)}),
                ),
            };
        }
        let path_arg = || args.get("path").and_then(Value::as_str).map(PathBuf::from);
        match name {
            "list_elements" => Ok(summary(
                self.session.doc(),
                args.get("category").and_then(Value::as_str),
            )),
            "get_element" => {
                let Some(id) = args.get("element").and_then(Value::as_u64) else {
                    return err("missing element".into());
                };
                detail(self.session.doc(), ElementId(id)).or_else(err)
            }
            "undo" => Ok(json!({"changed": self.session.undo()})),
            "redo" => Ok(json!({"changed": self.session.redo()})),
            "get_journal" => {
                let n = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                let j = self.session.journal();
                Ok(json!(j[j.len().saturating_sub(n)..]))
            }
            "new_document" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled");
                self.session.replace_document(Document::new(name));
                self.path = None;
                Ok(json!({"ok": true}))
            }
            "open_document" => {
                let Some(p) = path_arg() else {
                    return err("missing path".into());
                };
                let doc = if p.to_string_lossy().ends_with(".json") {
                    std::fs::read_to_string(&p)
                        .map_err(|e| e.to_string())
                        .and_then(|t| ferro_io::document_from_json(&t).map_err(|e| e.to_string()))
                } else {
                    ferro_io::load_fro(&p)
                        .map(|(d, _)| d)
                        .map_err(|e| e.to_string())
                };
                match doc {
                    Ok(d) => {
                        self.session.replace_document(d);
                        self.path = Some(p);
                        Ok(summary(self.session.doc(), None))
                    }
                    Err(m) => err(m),
                }
            }
            "save_document" => {
                let Some(p) = path_arg().or_else(|| self.path.clone()) else {
                    return err("no path: give one, or open a file first".into());
                };
                let r = if p.to_string_lossy().ends_with(".json") {
                    ferro_io::document_to_json(self.session.doc())
                        .map_err(|e| e.to_string())
                        .and_then(|t| std::fs::write(&p, t).map_err(|e| e.to_string()))
                } else {
                    ferro_io::save_fro(&p, self.session.doc(), self.session.journal())
                        .map_err(|e| e.to_string())
                };
                match r {
                    Ok(()) => {
                        self.path = Some(p.clone());
                        Ok(json!({"saved": p}))
                    }
                    Err(m) => err(m),
                }
            }
            "export_obj" | "export_glb" => {
                let Some(p) = path_arg() else {
                    return err("missing path".into());
                };
                let r = if name == "export_obj" {
                    ferro_io::export_obj(self.session.doc())
                        .map_err(|e| e.to_string())
                        .and_then(|s| std::fs::write(&p, s).map_err(|e| e.to_string()))
                } else {
                    ferro_io::export_glb(self.session.doc())
                        .map_err(|e| e.to_string())
                        .and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string()))
                };
                r.map(|_| json!({"exported": p})).or_else(err)
            }
            other => err(format!("unknown tool: {other}")),
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
