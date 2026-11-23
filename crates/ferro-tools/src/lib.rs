//! Shared tool surface (D-17, D-44) for MCP and Ferro Link: every kernel
//! command is a tool with the same name, plus read-only tools. Descriptions are written
//! for an AI reader. User units are converted here, at the edge (D-01, D-02):
//! lengths accept numbers (mm) or strings ("3m", "300cm", "10'"), angles are
//! in degrees, points are [x, y, z] in mm.

use ferro_core::model::{keys, Category, ElementBody};
use ferro_core::ops::{AnyCommand, ExecContext, Failure};
use ferro_core::{Angle, Document, ElementId, Length, Session};
use serde_json::{json, Map, Value};

pub mod link;

const LENGTH_FIELDS: [&str; 11] = [
    "width",
    "depth",
    "height",
    "elevation",
    "distance",
    "position",
    "sill",
    "offset",
    "top_offset",
    "base_offset",
    "thickness",
];
const POINT_FIELDS: [&str; 6] = ["origin", "center", "vector", "start", "end", "mid"];

fn length() -> Value {
    json!({"type": ["number", "string"], "description": "Length: number = millimetres, or a string with units such as \"3m\", \"300cm\", \"10'\"."})
}
fn point(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": ["number", "string"]}, "minItems": 2, "maxItems": 3, "description": format!("{desc} [x, y, z] (z optional): numbers in millimetres or strings with units (\"3m\"). Z is up, X east, Y north.")})
}
fn id(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}
fn ids(desc: &str) -> Value {
    json!({"type": "array", "items": {"type": "integer"}, "description": desc})
}
fn layers() -> Value {
    json!({"type":"array","items":{"type":"object","properties":{"function":{"type":"string","enum":["structure","substrate","insulation","finish","membrane"]},"thickness": length(),"material": id("Material id")},"required":["function","thickness"]}})
}
fn wall_props(mut extra: Value) -> Value {
    let common = json!({"level": id("Base level id"), "wall_type": id("Wall type id"), "height": length(), "top_level": id("Level the top is attached to"), "top_offset": length(), "base_offset": length(), "location_line": {"type":"string","enum":["wall_centerline","core_centerline","finish_exterior","finish_interior"]}, "name": {"type":"string"}});
    if let (Some(e), Some(c)) = (extra.as_object_mut(), common.as_object()) {
        for (k, v) in c {
            e.insert(k.clone(), v.clone());
        }
    }
    extra
}
fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

/// Tool definitions (name, description, JSON schema) for commands and the
/// read-only tools.
pub fn list() -> Vec<Value> {
    vec![
        tool("create_level", "Create a level (named elevation, e.g. a storey). Elements placed on a level move with it.",
            json!({"name": {"type":"string"}, "elevation": length(), "is_building_storey": {"type":"boolean","description":"Default true."}}), &["name", "elevation"]),
        tool("set_level_elevation", "Change a level's elevation. Elements on that level move with it.",
            json!({"level": id("Level element id"), "elevation": length()}), &["level", "elevation"]),
        tool("create_rectangle", "Draw a horizontal rectangular face (SketchUp Rectangle tool). Returns {element, face}. With `level`, origin z is relative to the level elevation.",
            json!({"origin": point("Corner"), "width": length(), "depth": length(), "level": id("Optional level id"), "name": {"type":"string"}}), &["origin", "width", "depth"]),
        tool("create_polygon", "Draw a planar polygon face from 3+ points. Returns {element, face}.",
            json!({"points": {"type":"array","items": point("Vertex"), "minItems": 3}, "level": id("Optional level id"), "name": {"type":"string"}, "relative_to_level": {"type":"boolean"}}), &["points"]),
        tool("create_box", "Create a box (rectangle pulled up). Fastest way to block out a volume. Returns {element}.",
            json!({"origin": point("Bottom corner"), "width": length(), "depth": length(), "height": length(), "level": id("Optional level id"), "name": {"type":"string"}}), &["origin", "width", "depth", "height"]),
        tool("push_pull_face", "Push / pull a face along its normal (SketchUp Push/Pull). Negative distance pushes inward. On a Form it edits geometry; on a wall, floor, door or window it edits parameters instead (top face → height, end face → length, side face → moves the wall, opening edges → size). Get face ids from get_element.",
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
        // BIM elements (milestone 2).
        tool("create_material", "Create a material with a base colour. Returns {element}.",
            json!({"name": {"type":"string"}, "color": {"type":"string","description":"#rrggbb"}, "roughness": {"type":"number","minimum":0,"maximum":1}}), &["name", "color"]),
        tool("create_wall_type", "Create a wall type: either one layer of `thickness` (+ optional material id) or `layers` listed from the exterior face to the interior face, each {function: structure|substrate|insulation|finish|membrane, thickness, material?}.",
            json!({"name": {"type":"string"}, "thickness": length(), "material": id("Optional material id"), "layers": layers()}), &["name"]),
        tool("create_floor_type", "Create a floor type (layers from top to bottom), like create_wall_type.",
            json!({"name": {"type":"string"}, "thickness": length(), "material": id("Optional material id"), "layers": layers()}), &["name"]),
        tool("create_wall", "Create a straight wall from start to end (z ignored; the base is the level elevation + base_offset). The exterior face is on the LEFT of the direction of travel, so draw outer walls counter-clockwise seen from above. Height: top_level (+ top_offset), else height (default 3 m). Default type \"Generic - 200mm\". Returns {element}.",
            wall_props(json!({"start": point("Start"), "end": point("End")})), &["start", "end"]),
        tool("create_walls", "Create a chain of walls through points (closed adds the closing wall). Corners are mitred automatically. Returns {elements}.",
            wall_props(json!({"points": {"type":"array","items": point("Vertex"), "minItems": 2}, "closed": {"type":"boolean"}})), &["points"]),
        tool("create_arc_wall", "Create a curved wall along the arc start → mid → end. Openings in curved walls are not supported yet.",
            wall_props(json!({"start": point("Start"), "mid": point("A point on the arc"), "end": point("End")})), &["start", "mid", "end"]),
        tool("set_wall_height", "Give a wall a fixed (unconnected) height.",
            json!({"wall": id("Wall id"), "height": length()}), &["wall", "height"]),
        tool("set_element_type", "Change the type of a wall (wall type) or floor (floor type).",
            json!({"element": id("Wall or floor id"), "element_type": id("Type id")}), &["element", "element_type"]),
        tool("create_floor", "Create a floor slab from a plan outline: points [x, y] in mm, optional holes. Its top sits at level elevation + offset; it extends down by the type thickness. Returns {element}.",
            json!({"points": {"type":"array","items": point("Vertex (z ignored)"), "minItems": 3}, "holes": {"type":"array","items":{"type":"array","items": point("Vertex")}}, "level": id("Optional level id"), "offset": length(), "floor_type": id("Optional floor type id"), "name": {"type":"string"}}), &["points"]),
        tool("create_opening", "Place a door or window in a straight wall. position = distance from the wall start to the opening centre. Defaults: door 900 × 2100; window 1200 × 1200 with 900 sill. Fails if it does not fit. Returns {element}.",
            json!({"wall": id("Host wall id"), "kind": {"type":"string","enum":["door","window"]}, "position": length(), "width": length(), "height": length(), "sill": length(), "name": {"type":"string"}}), &["wall", "kind", "position"]),
        tool("set_opening", "Change a door or window: position along the wall, width, height, sill. Omitted fields keep their value.",
            json!({"opening": id("Door or window id"), "position": length(), "width": length(), "height": length(), "sill": length()}), &["opening"]),
        // Read-only and session tools.
        tool("list_elements", "Summarise the model: id, uid, name, category, level, faces, area (m²), volume (m³). Optional category filter.",
            json!({"category": {"type":"string"}}), &[]),
        tool("get_element", "Full detail of one element, including each face id with its normal, area and centre (use for push_pull_face).",
            json!({"element": id("Element id")}), &["element"]),
        tool("undo", "Undo the last change.", json!({}), &[]),
        tool("redo", "Redo the last undone change.", json!({}), &[]),
        tool("get_journal", "Recent history: who changed what, with which commands.", json!({"limit":{"type":"integer"}}), &[]),
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
    if let Some(Value::Array(holes)) = obj.get("holes").cloned() {
        let mut conv = Vec::new();
        for h in holes {
            let Value::Array(pts) = h else {
                return Err("holes must be lists of points".into());
            };
            conv.push(Value::Array(
                pts.iter()
                    .map(|p| parse_point(p, "holes"))
                    .collect::<Result<Vec<_>, _>>()?,
            ));
        }
        obj.insert("holes".into(), Value::Array(conv));
    }
    if let Some(Value::Array(layers)) = obj.get("layers").cloned() {
        let mut conv = Vec::new();
        for l in layers {
            let mut l = l
                .as_object()
                .cloned()
                .ok_or("each layer must be an object")?;
            if let Some(t) = l.get("thickness").cloned() {
                l.insert("thickness".into(), json!(parse_length(&t, "thickness")?));
            }
            conv.push(Value::Object(l));
        }
        obj.insert("layers".into(), Value::Array(conv));
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
    // Coordinates accept the same forms as lengths: 1200 or "1.2m".
    let get = |x: &Value| parse_length(x, field);
    match v {
        Value::Array(a) if a.len() == 3 => {
            Ok(json!({"x": get(&a[0])?, "y": get(&a[1])?, "z": get(&a[2])?}))
        }
        Value::Array(a) if a.len() == 2 => {
            Ok(json!({"x": get(&a[0])?, "y": get(&a[1])?, "z": 0.0}))
        }
        Value::Object(o) => Ok(json!({
            "x": get(o.get("x").unwrap_or(&json!(0)))?,
            "y": get(o.get("y").unwrap_or(&json!(0)))?,
            "z": get(o.get("z").unwrap_or(&json!(0)))?,
        })),
        _ => Err(format!("{field} must be [x, y, z]")),
    }
}

/// Failures as JSON for tool results.
pub fn failures_json(f: &[Failure]) -> Value {
    json!(f.iter().map(|x| json!({"severity": x.severity, "code": x.code, "message": x.message, "elements": x.elements})).collect::<Vec<_>>())
}

/// Model summary (`list_elements`).
pub fn summary(doc: &Document, category: Option<&str>) -> Value {
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
                ElementBody::Material(mat) => {
                    let [r, g, b] = mat.color.map(|c| (c * 255.0).round() as u8);
                    m.insert("color".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
                }
                ElementBody::WallType(t) => {
                    m.insert("thickness_mm".into(), json!(t.structure.thickness()));
                    m.insert("layers".into(), json!(t.structure.layers));
                }
                ElementBody::FloorType(t) => {
                    m.insert("thickness_mm".into(), json!(t.structure.thickness()));
                    m.insert("layers".into(), json!(t.structure.layers));
                }
                ElementBody::Wall(w) => {
                    m.insert("type".into(), json!(e.type_id));
                    m.insert("curve".into(), json!(w.curve));
                    m.insert("top".into(), json!(w.top));
                    m.insert("base_offset_mm".into(), json!(w.base_offset.mm()));
                    m.insert("location_line".into(), json!(w.location_line));
                    bim_dims(e, &mut m);
                }
                ElementBody::Floor(f) => {
                    m.insert("type".into(), json!(e.type_id));
                    m.insert("outline_mm".into(), json!(f.outer));
                    m.insert("offset_mm".into(), json!(f.offset.mm()));
                    bim_dims(e, &mut m);
                }
                ElementBody::Opening(o) => {
                    m.insert("host".into(), json!(e.host_id));
                    m.insert("kind".into(), json!(o.kind));
                    m.insert("position_mm".into(), json!(o.position.mm()));
                    m.insert("width_mm".into(), json!(o.width.mm()));
                    m.insert("height_mm".into(), json!(o.height.mm()));
                    m.insert("sill_mm".into(), json!(o.sill.mm()));
                }
            }
            Value::Object(m)
        })
        .collect();
    json!({"project": doc.project().name, "elements": items})
}

/// Computed dimensions of walls and floors in metres.
fn bim_dims(e: &ferro_core::Element, m: &mut Map<String, Value>) {
    use ferro_core::ParamValue;
    for (k, label) in [
        (keys::LENGTH, "length_m"),
        (keys::HEIGHT, "height_m"),
        (keys::THICKNESS, "thickness_m"),
    ] {
        if let Some(ParamValue::Length(l)) = e.param(k) {
            m.insert(label.into(), json!((l.m() * 1000.0).round() / 1000.0));
        }
    }
    if let Some(ParamValue::Area(a)) = e.param(keys::AREA) {
        m.insert("area_m2".into(), json!((a / 1e6 * 1000.0).round() / 1000.0));
    }
    if let Some(ParamValue::Volume(v)) = e.param(keys::VOLUME) {
        m.insert(
            "volume_m3".into(),
            json!((v / 1e9 * 1000.0).round() / 1000.0),
        );
    }
}

/// Full element detail (`get_element`).
pub fn detail(doc: &Document, id: ElementId) -> Result<Value, String> {
    let e = doc
        .get(id)
        .ok_or_else(|| format!("element {id} not found"))?;
    let mut v = summary_one(e);
    if let Some(g) = e.geometry() {
        let faces: Vec<Value> = g
            .face_ids()
            .map(|fid| {
                let n = g.face_normal(fid).map(|u| u.as_vec()).unwrap_or_default();
                let pts = g.face_points(fid).unwrap_or_default();
                let c = ferro_core::Point3::centroid(&pts).unwrap_or_default();
                json!({"face": fid, "normal": [round(n.x), round(n.y), round(n.z)], "area_m2": (g.face_area(fid).unwrap_or(0.0) / 1e6 * 1000.0).round() / 1000.0, "center_mm": [round(c.x), round(c.y), round(c.z)]})
            })
            .collect();
        v["faces"] = json!(faces);
        if let Some(bb) = g.bounding_box() {
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

/// Run a command or read-only tool against a session. Ok = JSON result,
/// Err = JSON with `error` and optional `failures`. Returns `None` for tools
/// this function does not handle (file tools, link tools).
pub fn call(
    session: &mut Session,
    ctx: &ExecContext,
    name: &str,
    args: &Value,
) -> Option<Result<Value, Value>> {
    let err = |m: String| Err(json!({"error": m}));
    if AnyCommand::NAMES.contains(&name) {
        let norm = match normalize(args) {
            Ok(v) => v,
            Err(m) => return Some(err(m)),
        };
        let cmd = match AnyCommand::from_tool(name, &norm) {
            Ok(c) => c,
            Err(m) => return Some(err(m)),
        };
        return Some(match session.execute(cmd, ctx) {
            Ok(o) => Ok(
                json!({"result": o.output, "changed": o.changed, "warnings": failures_json(&o.warnings)}),
            ),
            Err(f) => Err(
                json!({"error": "command failed; nothing was changed", "failures": failures_json(&f)}),
            ),
        });
    }
    Some(match name {
        "list_elements" => Ok(summary(
            session.doc(),
            args.get("category").and_then(Value::as_str),
        )),
        "get_element" => match args.get("element").and_then(Value::as_u64) {
            Some(id) => detail(session.doc(), ElementId(id)).or_else(err),
            None => err("missing element".into()),
        },
        "undo" => match session.undo() {
            Some(c) => Ok(json!({"changed": c})),
            None => err("nothing to undo".into()),
        },
        "redo" => match session.redo() {
            Some(c) => Ok(json!({"changed": c})),
            None => err("nothing to redo".into()),
        },
        "get_journal" => {
            let n = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let j = session.journal();
            Ok(json!(j[j.len().saturating_sub(n)..]))
        }
        _ => return None,
    })
}

/// File tools offered by a standalone tool host (ferro-mcp without an open app).
pub fn file_tools() -> Vec<Value> {
    vec![
        tool(
            "new_document",
            "Start a new empty project.",
            json!({"name":{"type":"string"}}),
            &[],
        ),
        tool(
            "open_document",
            "Open a .fro or .fro.json file.",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        tool(
            "save_document",
            "Save to .fro (SQLite) or .fro.json. Without path, saves to the opened file.",
            json!({"path":{"type":"string"}}),
            &[],
        ),
        tool(
            "export_obj",
            "Export Forms to Wavefront OBJ (Y-up, metres).",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        tool(
            "export_glb",
            "Export Forms to binary glTF (Y-up, metres).",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
    ]
}
