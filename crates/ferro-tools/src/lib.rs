//! Shared tool surface (D-17, D-44) for MCP and Ferro Link: every kernel
//! command is a tool with the same name, plus read-only tools. Descriptions are written
//! for an AI reader. User units are converted here, at the edge (D-01, D-02):
//! lengths accept numbers (mm) or strings ("3m", "300cm", "10'"), angles are
//! in degrees, points are [x, y, z] in mm.

use ferro_core::model::structure_quantities::StructureQuantities;
use ferro_core::model::{keys, Category, ElementBody};
use ferro_core::ops::{AnyCommand, ExecContext, Failure};
use ferro_core::{Angle, Document, ElementId, Length, Session};
use serde_json::{json, Map, Value};

mod design_option;
pub mod link;
mod parameter;

pub use parameter::param_value;

const LENGTH_FIELDS: [&str; 12] = [
    "radius",
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
const POINT_FIELDS: [&str; 7] = [
    "origin", "center", "vector", "start", "end", "mid", "normal",
];

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
/// Material asset properties (`create_material`, `set_material`) added to
/// `base`. With `partial`, no asset field is required (merge patch).
fn material_props(mut base: Value, partial: bool) -> Value {
    let num = |d: &str| json!({"type": ["number", "null"], "description": d});
    let unit = json!({"type":"string","enum":["m3","kg","m2"],"description":"Per cubic metre, kilogram or square metre of material"});
    let image = json!({"type":["object","null"],"description":"Image laid on faces at real-world size (placeholder, not rendered yet)","properties":{"path":{"type":"string"},"width_mm":{"type":"number"},"height_mm":{"type":"number"},"rotation_rad":{"type":"number"}},"required":["path","width_mm","height_mm"]});
    let req = |r: &[&str]| -> Value {
        if partial {
            json!([])
        } else {
            json!(r)
        }
    };
    let assets = json!({
        "appearance": {"type":"object","description":"OpenPBR surface parameters beyond colour (= base_color) and roughness (= specular_roughness)","properties":{
            "base_metalness": num("0..1, OpenPBR base_metalness"),
            "specular_ior": num("Index of refraction, default 1.5"),
            "transmission_weight": num("0..1, glass ≈ 1"),
            "emission_color": {"type":["array","null"],"items":{"type":"number"},"minItems":3,"maxItems":3,"description":"sRGB 0..1"},
            "emission_luminance": num("cd/m² (nits)"),
            "geometry_opacity": num("0..1, 1 = opaque"),
            "coat_weight": num("0..1, varnish / lacquer"),
            "base_color_image": image.clone(),
            "normal_image": image}},
        "physical": {"type":"object","description":"Mechanical data, SI units","properties":{
            "density_kg_m3": num("kg/m³"),
            "youngs_modulus_pa": num("Young's modulus, Pa (concrete ≈ 3.1e10)"),
            "poisson_ratio": num("-1..0.5"),
            "compressive_strength_pa": num("Pa"),
            "tensile_strength_pa": num("Pa"),
            "thermal_expansion_per_k": num("1/K")}},
        "thermal": {"type":"object","description":"Thermal data, SI units; conductivity gives wall / floor U-values","properties":{
            "conductivity_w_mk": num("λ, W/(m·K), > 0"),
            "specific_heat_j_kgk": num("J/(kg·K)"),
            "density_kg_m3": num("kg/m³ (used when physical has none)"),
            "emissivity": num("0..1"),
            "solar_absorptance": num("0..1"),
            "vapour_resistance_factor": num("μ, ≥ 1")}},
        "environmental": {"type":"object","description":"Embodied carbon (GWP-total, EN 15804 stages) in kg CO2e per declared unit","properties":{
            "declared_unit": unit.clone(),
            "gwp_a1_a3": num("Product stage A1-A3"),
            "gwp_a4": num("A4 transport"),
            "gwp_a5": num("A5 installation"),
            "gwp_b": num("B1-B7 use"),
            "gwp_c": num("C1-C4 end of life"),
            "gwp_d": num("D beyond system boundary"),
            "epd_source": {"type":["string","null"],"description":"EPD number / database"}},
            "required": req(&["declared_unit"])},
        "cost": {"type":"object","description":"Unit price","properties":{
            "unit_price": {"type":"number"},
            "currency": {"type":"string","description":"ISO 4217, e.g. VND, USD"},
            "unit": unit},
            "required": req(&["unit_price", "currency", "unit"])},
    });
    if let (Some(b), Some(a)) = (base.as_object_mut(), assets.as_object()) {
        for (k, v) in a {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

/// Comma-separated quantity spec ids, for tool descriptions.
fn spec_ids() -> String {
    ferro_core::QuantitySpec::ALL
        .iter()
        .map(|s| s.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// View properties shared by `create_view` and `set_view`, added to `base`.
fn view_props(mut base: Value) -> Value {
    let plane = json!({"type":["object","null"],"description":"Cutting plane: geometry on the side the normal points to is not drawn","properties":{"point": point("A point on the plane"), "normal": point("Normal of the side cut away")},"required":["point","normal"]});
    let common = json!({
        "camera": {"type":"object","description":"Camera; points in mm","properties":{"eye": point("Camera position"), "target": point("Point looked at"), "up": point("Up direction"), "projection": {"type":"string","enum":["perspective","parallel"]}, "fov_y": {"type":["number","string"],"description":"Vertical field of view, degrees (default 45)"}},"required":["eye","target","up"]},
        "section_plane": plane,
        "section_box": {"type":["object","null"],"description":"3D views: only what is inside is drawn (stored; not drawn by the app yet)","properties":{"min": point("Lowest corner"), "max": point("Highest corner")},"required":["min","max"]},
        "view_range": {"type":"object","description":"Plan views (Revit View Range), offsets from the level; view_depth ≤ bottom ≤ cut_height ≤ top","properties":{"top": length(), "cut_height": length(), "bottom": length(), "view_depth": length()},"required":["top","cut_height","bottom","view_depth"]},
        "depth": length(),
        "display_style": {"type":"string","enum":["shaded","x_ray","wireframe","hidden_line"]},
        "phase": {"type":["integer","null"],"description":"Phase shown by the view (null = the last phase)"},
        "phase_filter": {"type":"string","enum":["show_all","show_complete","show_new","show_previous_and_demo","show_previous_and_new","show_demo_and_new","no_overrides"],"description":"Revit phase filter: which statuses are shown and with status graphics (default show_all)"},
    });
    if let (Some(b), Some(c)) = (base.as_object_mut(), common.as_object()) {
        for (k, v) in c {
            b.insert(k.clone(), v.clone());
        }
    }
    base
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
        tool("set_parameter", &format!("Set a typed parameter. Key format namespace.group.name (e.g. ferro.common.comments, acme.energy.conductivity). Value forms: (1) a quantity with a unit as text: \"0.8 W/mK\", \"1200 kg/m3\", \"35 MPa\", \"0.25 W/m2K\", \"20 °C\", \"120 kgCO2e\", \"3m\" (if the key has a quantity definition, a bare number in text uses its default unit); (2) {{\"spec\": \"density\", \"value\": 1200, \"unit\": \"kg/m3\"}} (no unit = SI storage unit; length mm, angle rad); (3) {{\"type\": \"text\"|\"integer\"|\"bool\"|\"tags\"|\"element_ref\"|\"number\", \"value\": ...}}; also plain numbers, booleans and string arrays (tags). Specs: {}. Fails if the key's definition has another type. Computed keys (ferro.form.*, ferro.dimensions.*) are read-only.", spec_ids()),
            json!({"element": id("Element id"), "key": {"type":"string"}, "value": {"type":["object","string","number","boolean","array"]}}), &["element", "key", "value"]),
        tool("set_parameter_definition", "Define a project parameter key (like a Revit shared parameter): data type, group, instance / type binding, description and optional IFC property set mapping. Values set later must fit the type. Built-in ferro.* keys cannot be defined. Returns {key, uid}; pass `uid` (a Revit shared parameter GUID) to match an existing Revit parameter.",
            json!({"key": {"type":"string"},
                   "data_type": {"type":["string","object"], "description": format!("\"text\"|\"integer\"|\"bool\"|\"tags\"|\"element_ref\" or {{\"quantity\": \"<spec>\"}}, e.g. {{\"quantity\": \"u_value\"}}. Specs: {}", spec_ids())},
                   "group": {"type":"string","enum":["constraints","dimensions","identity_data","materials","analysis","structural","data"]},
                   "binding": {"type":"string","enum":["instance","type"]},
                   "description": {"type":"string"},
                   "uid": {"type":"string","description":"Optional UUID (8-4-4-4-12)"},
                   "ifc": {"type":"object","properties":{"property_set":{"type":"string"},"property":{"type":"string"}},"required":["property_set","property"]}}),
            &["key", "data_type"]),
        tool("set_category", "Classify directly edited geometry (a Form, or a frozen wall / floor) as a BIM category: a free-form wall keeps its shape and becomes a Wall with type, materials and parameters, exported to Revit / IFC as a wall. A type that no longer fits is removed.",
            json!({"element": id("Element with direct geometry"), "category": {"type":"string","enum": Category::ALL.iter().filter(|c| c.classifiable_form()).map(|c| c.to_string()).collect::<Vec<_>>()}}), &["element", "category"]),
        tool("set_geometry_source", "Switch where an element's geometry comes from. 'direct': freeze a parametric wall / floor so it can be edited freely like a SketchUp group (keeps category, type, parameters; hosted doors / windows freeze with it). 'parametric': turn a directly edited wall (rectangular box) or floor (flat slab with vertical sides) back into a parametric one, using a type of the same thickness; fails with ferro.model.not_recognised or ferro.model.no_matching_type. Returns {element, changed}.",
            json!({"element": id("Element id"), "source": {"type":"string","enum":["direct","parametric"]}}), &["element", "source"]),
        tool("set_project_units", "Choose how a quantity is shown and typed in this project, e.g. {spec: length, unit: m, decimals: 2}. unit null = back to the default (mm for length). Values are always stored in mm / SI; this changes display only.",
            json!({"spec": {"type":"string","enum": ferro_core::QuantitySpec::ALL.iter().map(|s| s.id()).collect::<Vec<_>>()}, "unit": {"type":["string","null"],"description":"A unit of that quantity, e.g. mm, m, ft, W/(m·K)"}, "decimals": {"type":["integer","null"],"minimum":0,"maximum":9}}), &["spec"]),
        tool("set_site_location", "Set where the project is (sun, energy, weather): latitude / longitude in degrees (north / east positive), elevation of the internal origin above sea level (length), UTC offset in minutes (Vietnam 420), place name, weather file (EPW). Omitted fields keep their value; clear: true removes the location.",
            json!({"latitude": {"type":"number","description":"Degrees, -90..90"}, "longitude": {"type":"number","description":"Degrees, -180..180"}, "elevation": length(), "utc_offset_minutes": {"type":"integer"}, "place": {"type":"string"}, "weather_file": {"type":"string"}, "clear": {"type":"boolean"}}), &[]),
        tool("set_project_currency", "Set the currency for costs (ISO 4217, e.g. VND, USD); null clears it.",
            json!({"currency": {"type":["string","null"]}}), &[]),
        tool("set_project_name", "Rename the project.", json!({"name":{"type":"string"}}), &["name"]),
        // BIM elements (milestone 2).
        tool("create_material", "Create a material with a base colour and optional assets (all optional, SI units named in each field): appearance (OpenPBR), physical, thermal, environmental (GWP from an EPD), cost. Wall / floor types using it then report U-value, mass, carbon and cost per m² (get_element on the type). Returns {element}.",
            material_props(json!({"name": {"type":"string"}, "color": {"type":"string","description":"#rrggbb"}, "roughness": {"type":"number","minimum":0,"maximum":1,"description":"OpenPBR specular_roughness, default 0.8"}}), false), &["name", "color"]),
        tool("set_material", "Edit a material; omitted fields keep their value. Each asset is a partial update: given fields are set, fields set to null are cleared, others kept (e.g. {\"material\": 3, \"thermal\": {\"conductivity_w_mk\": 0.8}}). `clear` removes whole assets. Unknown field names fail.",
            material_props(json!({"material": id("Material id"), "color": {"type":"string","description":"#rrggbb"}, "roughness": {"type":"number","minimum":0,"maximum":1}, "clear": {"type":"array","items":{"type":"string","enum":["appearance","physical","thermal","environmental","cost"]},"description":"Assets to remove before applying the updates"}}), true), &["material"]),
        tool("create_wall_type", "Create a wall type: either one layer of `thickness` (+ optional material id) or `layers` listed from the exterior face to the interior face, each {function: structure|substrate|insulation|finish|membrane, thickness, material?}.",
            json!({"name": {"type":"string"}, "thickness": length(), "material": id("Optional material id"), "layers": layers()}), &["name"]),
        tool("create_floor_type", "Create a floor type (layers from top to bottom), like create_wall_type.",
            json!({"name": {"type":"string"}, "thickness": length(), "material": id("Optional material id"), "layers": layers()}), &["name"]),
        tool("create_wall", "Create a straight wall from start to end (z ignored; the base is the level elevation + base_offset). The exterior face is on the LEFT of the direction of travel, so draw outer walls clockwise seen from above (as in Revit). Height: top_level (+ top_offset), else height (default 3 m). Default type \"Generic - 200mm\". Returns {element}.",
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
        tool("set_wall", "Change a wall: start + end (+ mid for a curved wall), base_offset, height (fixed top) or top_level (+ top_offset), location_line. Omitted fields keep their value.",
            json!({"wall": id("Wall id"), "start": point("Start"), "end": point("End"), "mid": point("Point on the arc (curved walls)"), "base_offset": length(), "height": length(), "top_level": id("Level the top attaches to"), "top_offset": length(), "location_line": {"type":"string","enum":["wall_centerline","core_centerline","finish_exterior","finish_interior"]}}), &["wall"]),
        tool("set_floor", "Change a floor's outline (points [x, y]), holes or offset. Omitted fields keep their value.",
            json!({"floor": id("Floor id"), "points": {"type":"array","items": point("Vertex"), "minItems": 3}, "holes": {"type":"array","items":{"type":"array","items": point("Vertex")}}, "offset": length()}), &["floor"]),
        // SketchUp-style Form editing.
        tool("imprint_face", "Draw on a face of a Form (lines or a closed shape, points on the face plane): the face splits, like drawing on a face in SketchUp. Returns new_faces, ready for push_pull_face (push in to make a pocket, push to the opposite face to cut a hole).",
            json!({"element": id("Form id"), "face": id("Face id"), "points": {"type":"array","items": point("Point on the face"), "minItems": 2}, "closed": {"type":"boolean"}}), &["element", "face", "points"]),
        tool("create_circle", "Draw a circle (segments, default 24). With element + face it is drawn on that face; otherwise a new circular face (normal defaults to up). Pulling it gives a smooth cylinder.",
            json!({"center": point("Centre"), "radius": length(), "normal": point("Normal direction"), "segments": {"type":"integer","minimum":3,"maximum":360}, "element": id("Form id"), "face": id("Face id"), "level": id("Optional level id"), "name": {"type":"string"}}), &["center", "radius"]),
        tool("offset_face", "Offset a face's outline inwards by a distance (SketchUp Offset); the inner copy splits the face.",
            json!({"element": id("Form id"), "face": id("Face id"), "distance": length()}), &["element", "face", "distance"]),
        tool("move_geometry", "Move vertices, edges or faces of a Form (connected faces stretch), e.g. move a top edge sideways to make a sloped roof. Faces must stay flat.",
            json!({"element": id("Form id"), "vertices": ids("Vertex ids"), "edges": ids("Edge ids"), "faces": ids("Face ids"), "vector": point("Displacement")}), &["element", "vector"]),
        tool("scale_elements", "Scale Forms about a centre: factor (uniform) or factors [x, y, z].",
            json!({"elements": ids("Form ids"), "center": point("Centre"), "factor": {"type":"number"}, "factors": {"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}}), &["elements", "center"]),
        tool("erase_edges", "Erase edges of a Form: an edge between faces in one plane merges them; other edges remove their faces.",
            json!({"element": id("Form id"), "edges": ids("Edge ids")}), &["element", "edges"]),
        tool("erase_faces", "Erase faces of a Form (it becomes open); erasing all faces deletes it.",
            json!({"element": id("Form id"), "faces": ids("Face ids")}), &["element", "faces"]),
        // Phases (ADR 0006).
        tool("create_phase", "Create a construction phase (Revit Phase), appended at the end or inserted before `before`. Phases are ordered; elements are created / demolished in phases. Dates are YYYY-MM-DD. Returns {element, phases}.",
            json!({"name": {"type":"string"}, "before": id("Optional phase to insert before"), "start_date": {"type":"string","description":"YYYY-MM-DD"}, "end_date": {"type":"string","description":"YYYY-MM-DD"}}), &["name"]),
        tool("set_phase", "Rename a phase, move it to `position` (0 = first) or change its dates (\"\" clears a date). Fails with ferro.model.invalid_phase_order if some element would be demolished before it is created.",
            json!({"phase": id("Phase id"), "name": {"type":"string"}, "position": {"type":"integer","minimum":0}, "start_date": {"type":"string"}, "end_date": {"type":"string"}}), &["phase"]),
        tool("delete_phase", "Delete a phase. A phase used by elements needs `merge_into`: the phase just before or after it, which its elements then use (Revit: combine with previous / next).",
            json!({"phase": id("Phase id"), "merge_into": id("Neighbouring phase that takes over its elements")}), &["phase"]),
        tool("set_element_phase", "Set the phase elements are created in and / or demolished in (Revit Phase Created / Phase Demolished). Omit a field to keep it, null clears it. Demolished must be the same phase as created or later. Elements without a created phase count as existing in every phase.",
            json!({"elements": ids("Element ids (walls, floors, doors, windows, Forms)"), "created": {"type":["integer","null"],"description":"Phase id"}, "demolished": {"type":["integer","null"],"description":"Phase id"}}), &["elements"]),
        tool("set_renovation_status", "Simple renovation mode (ArchiCAD-like): mark elements existing, to be demolished, or new. Uses the first and last phase; creates \"Existing\" and \"New Construction\" if the project has fewer than two phases.",
            json!({"elements": ids("Element ids"), "status": {"type":"string","enum":["existing","demolish","new"]}}), &["elements", "status"]),
        tool("get_phase_quantities", "Quantities in the view of a phase, per category split by status (existing / new / demolished / temporary): count, volume (m³) and area (m²; floors plan area, walls gross length × height). The basis for demolition and new-build costs. `phase` defaults to the last phase.",
            json!({"phase": id("Phase id (default: last phase)")}), &[]),
        // Relationships (ADR 0004 §5).
        tool("create_relationship", "Link elements with a relationship row (stored as data, saved, undoable). Order of `elements` by kind: join [first, second] (first wins where they overlap), cut [cutter, cut], host [host, hosted], attach [attached, target], bounds [space, boundary], connect [a, b], aggregate [whole, part, ...]. Only elements with geometry. `phase`: the phase it is made in. Returns {relationship: uid}. Fails with ferro.model.invalid_relationship, missing_element or duplicate_relationship.",
            json!({"kind": {"type":"string","enum":["join","cut","host","attach","bounds","connect","aggregate"]}, "elements": ids("Element ids, ordered as the kind says"), "phase": id("Optional phase id")}), &["kind", "elements"]),
        tool("delete_relationship", "Remove a relationship row by uid; the elements stay. Deleting an element removes its rows automatically.",
            json!({"relationship": {"type":"string","description":"Relationship uid (from create_relationship or list_relationships)"}}), &["relationship"]),
        // Design options (ADR 0006) and change lists (M5-26).
    ]
    .into_iter()
    .chain(design_option::tools())
    .chain(vec![
        tool("list_relationships", "Relationship rows, all or those linking `element`: [{relationship, kind, elements, phase}].",
            json!({"element": id("Optional element id")}), &[]),
        // Views (D-33, M5-22).
        tool("create_view", "Create a view (Revit View): `view_3d` (camera, optional section plane / section box), `plan_view` (needs `level`; cut at view_range.cut_height above it, default 1200 mm) or `section_view` (needs `section_plane`; shows `depth` beyond it, default 10 m). View names are unique. A view keeps camera, display style, hidden elements / tags and its phase; the Ferro app shows the active view. Returns {element}.",
            view_props(json!({"name": {"type":"string"}, "view_type": {"type":"string","enum":["view_3d","plan_view","section_view"]}, "level": id("Level of a plan view")})), &["name", "view_type"]),
        tool("set_view", "Change a view. Omit a field to keep it. `section_plane`, `section_box` and `phase` accept null to clear (a section view keeps its plane). `hide` / `unhide` add / remove elements from the view's hidden list (`unhide_all` empties it first); `hidden_tags` replaces the tags hidden in the view. Deleting an element removes it from every view.",
            view_props(json!({"view": id("View id"), "name": {"type":"string"}, "hide": ids("Elements to hide in the view"), "unhide": ids("Elements to show again"), "unhide_all": {"type":"boolean"}, "hidden_tags": {"type":"array","items":{"type":"string"},"description":"Tags whose elements are not shown"}})), &["view"]),
        tool("delete_view", "Delete a view (deleting a level also deletes its plan views).",
            json!({"view": id("View id")}), &["view"]),
        // App-only (needs the Ferro app open, through Ferro Link).
        tool("get_selection", "Elements the user has selected in the Ferro app, with their summaries. Use it for requests like \"make the selected wall taller\".", json!({}), &[]),
        tool("select_elements", "Select elements in the Ferro app so the user sees what you mean.", json!({"elements": ids("Element ids (empty clears)")}), &["elements"]),
        // Revit link (used by the Ferro add-in for Revit).
        tool("get_document_version", "Version number of the open document; it increases with every change. Revit polls it to pull changes live.", json!({}), &[]),
        tool("revit_plan", "Ferro → Revit: the operations (in feet) that bring a Revit model up to date. `state` lists the Revit elements already tagged with Ferro uids and their signatures.",
            json!({"state": {"type":"object","description":"{elements: [{ferro_uid, unique_id, signature}]}"}}), &[]),
        tool("revit_import", "Revit → Ferro: apply a snapshot of the Revit model (levels, types, walls, floors, doors, windows; lengths in feet). Returns the Ferro uids of created elements.",
            json!({"snapshot": {"type":"object"}}), &["snapshot"]),
        // Read-only and session tools.
        tool("list_elements", "Summarise the model: id, uid, name, category, level, faces, area (m²), volume (m³). Optional category filter. `combination` shows the model as seen with design options (elements added by an option carry design_option).",
            json!({"category": {"type":"string"}, "combination": design_option::combination_schema("Optional view (default: the shared model)")}), &[]),
        tool("get_element", "Full detail of one element, including each face id with its normal, area and centre (use for push_pull_face).",
            json!({"element": id("Element id")}), &["element"]),
        tool("undo", "Undo the last change.", json!({}), &[]),
        tool("redo", "Redo the last undone change.", json!({}), &[]),
        tool("get_journal", "Recent history: who changed what, with which commands.", json!({"limit":{"type":"integer"}}), &[]),
    ])
    .map(with_option_argument)
    .collect()
}

/// Every command tool takes an optional `in_design_option`: run it inside
/// that design option (the change goes into the option's layer, the shared
/// model is untouched).
fn with_option_argument(mut t: Value) -> Value {
    let is_command = t["name"]
        .as_str()
        .is_some_and(|n| AnyCommand::NAMES.contains(&n) && !DESIGN_OPTION_COMMANDS.contains(&n));
    if is_command {
        if let Some(p) = t["inputSchema"]["properties"].as_object_mut() {
            p.insert("in_design_option".into(), json!({"type": "integer", "description": "Optional design option id: make this change inside that option (its layer), not in the shared model"}));
        }
    }
    t
}

/// Commands on option sets themselves, which cannot run inside an option.
const DESIGN_OPTION_COMMANDS: [&str; 8] = [
    "create_option_set",
    "set_option_set",
    "delete_option_set",
    "create_design_option",
    "set_design_option",
    "delete_design_option",
    "accept_design_option",
    "apply_changes",
];

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
    // Views (M5-22): points, lengths and the field of view inside the
    // nested camera / cut objects.
    for (key, lengths, points) in [
        ("camera", &[][..], &["eye", "target", "up"][..]),
        ("section_plane", &[][..], &["point", "normal"][..]),
        ("section_box", &[][..], &["min", "max"][..]),
        (
            "view_range",
            &["top", "cut_height", "bottom", "view_depth"][..],
            &[][..],
        ),
    ] {
        let Some(Value::Object(mut inner)) = obj.get(key).cloned() else {
            continue;
        };
        for f in lengths {
            if let Some(v) = inner.get(*f).cloned() {
                inner.insert((*f).into(), json!(parse_length(&v, f)?));
            }
        }
        for f in points {
            if let Some(v) = inner.get(*f).cloned() {
                inner.insert((*f).into(), parse_point(&v, f)?);
            }
        }
        if let Some(v) = inner.get("fov_y").cloned() {
            inner.insert("fov_y".into(), json!(degrees(&v, "fov_y")?));
        }
        obj.insert(key.into(), Value::Object(inner));
    }
    // Angles are degrees at this edge, radians inside (D-02).
    for f in ["angle", "latitude", "longitude"] {
        if let Some(v) = obj.get(f).cloned() {
            let deg = match &v {
                Value::Number(n) => n.as_f64().ok_or(format!("{f} must be a number"))?,
                Value::String(s) => Angle::parse(s)
                    .ok_or(format!("cannot read {f} {s:?}"))?
                    .deg(),
                _ => return Err(format!("{f} must be degrees")),
            };
            obj.insert(f.into(), json!(Angle::from_deg(deg).rad()));
        }
    }
    Ok(Value::Object(obj))
}

/// Degrees (number or text with a unit) → radians.
fn degrees(v: &Value, f: &str) -> Result<f64, String> {
    let deg = match v {
        Value::Number(n) => n.as_f64().ok_or(format!("{f} must be a number"))?,
        Value::String(s) => Angle::parse(s)
            .ok_or(format!("cannot read {f} {s:?}"))?
            .deg(),
        _ => return Err(format!("{f} must be degrees")),
    };
    Ok(Angle::from_deg(deg).rad())
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
    summary_of(doc, category, None)
}

/// Summary restricted to some elements (selection).
pub fn summary_of(doc: &Document, category: Option<&str>, only: Option<&[ElementId]>) -> Value {
    let items: Vec<Value> = doc
        .elements()
        .filter(|e| category.is_none_or(|c| e.category.to_string() == c))
        .filter(|e| only.is_none_or(|ids| ids.contains(&e.id)))
        .map(|e| {
            let mut m = Map::new();
            m.insert("id".into(), json!(e.id));
            m.insert("uid".into(), json!(e.uid.to_string()));
            m.insert("name".into(), json!(e.name));
            m.insert("category".into(), json!(e.category));
            m.insert("body".into(), json!(e.body.kind()));
            m.insert("level".into(), json!(e.level_id));
            if e.design_option.is_some() {
                m.insert("design_option".into(), json!(e.design_option));
            }
            if e.phase_created.is_some() || e.phase_demolished.is_some() {
                m.insert("phase_created".into(), json!(e.phase_created));
                m.insert("phase_demolished".into(), json!(e.phase_demolished));
            }
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
                    m.insert("roughness".into(), json!(mat.roughness));
                    // Assets as stored (SI units named in each field).
                    if let Ok(Value::Object(all)) = serde_json::to_value(mat) {
                        for (k, v) in all {
                            if k != "color" && k != "roughness" {
                                m.insert(k, v);
                            }
                        }
                    }
                }
                ElementBody::WallType(t) => {
                    m.insert("thickness_mm".into(), json!(t.structure.thickness()));
                    m.insert("layers".into(), json!(t.structure.layers));
                    type_quantities(doc, e.id, &mut m);
                }
                ElementBody::FloorType(t) => {
                    m.insert("thickness_mm".into(), json!(t.structure.thickness()));
                    m.insert("layers".into(), json!(t.structure.layers));
                    type_quantities(doc, e.id, &mut m);
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
                ElementBody::OptionSet(s) => {
                    m.insert("order".into(), json!(s.order));
                    m.insert("primary".into(), json!(doc.primary_option(e.id)));
                }
                ElementBody::DesignOption(o) => {
                    let c = ferro_core::model::design_option::layer_counts(doc, &o.changes);
                    m.insert("set".into(), json!(e.host_id));
                    m.insert("is_primary".into(), json!(o.is_primary));
                    m.insert("status".into(), json!(o.status.as_str()));
                    m.insert("changes".into(), json!(o.changes.len()));
                    m.insert("added".into(), json!(c.added));
                    m.insert("removed".into(), json!(c.removed));
                    m.insert("overridden".into(), json!(c.overridden));
                }
                ElementBody::Phase(p) => {
                    m.insert("order".into(), json!(doc.phase_index(e.id)));
                    m.insert("start_date".into(), json!(p.start_date));
                    m.insert("end_date".into(), json!(p.end_date));
                }
                ElementBody::View(v) => {
                    m.insert("view_type".into(), json!(v.view_type));
                    let c = &v.camera;
                    m.insert("camera".into(), json!({"eye": [c.eye.x, c.eye.y, c.eye.z], "target": [c.target.x, c.target.y, c.target.z], "up": c.up, "projection": c.projection, "fov_y_deg": round(c.fov_y.deg())}));
                    m.insert("display_style".into(), json!(v.display_style));
                    m.insert("hidden_elements".into(), json!(v.hidden_elements));
                    m.insert("hidden_tags".into(), json!(v.hidden_tags));
                    m.insert("phase".into(), json!(v.phase));
                    m.insert("phase_filter".into(), json!(v.phase_filter));
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

/// Computed, read-only quantities of a wall / floor type from its layer
/// materials: U-value (W/m²K, ISO 6946 surface resistances), mass (kg/m²),
/// GWP A1–A3 (kg CO2e/m²), cost per m², and the layers that lack data.
/// A value is null when some layer lacks the data for it.
fn type_quantities(doc: &Document, id: ElementId, m: &mut Map<String, Value>) {
    if let Some(q) = StructureQuantities::from_type(doc, id) {
        let r = |x: Option<f64>, k: f64| x.map(|v| (v * k).round() / k);
        m.insert(
            "quantities".into(),
            json!({
                "u_value_w_m2k": r(q.u_value_w_m2k, 1e4),
                "thermal_resistance_m2k_w": r(q.thermal_resistance_m2k_w, 1e4),
                "heat_flow": q.heat_flow,
                "mass_kg_m2": r(q.mass_kg_m2, 1e3),
                "gwp_a1_a3_kgco2e_m2": r(q.gwp_a1_a3_kgco2e_m2, 1e3),
                "cost_per_m2": r(q.cost_per_m2, 1e2),
                "currency": q.currency,
                "missing": q.gaps,
            }),
        );
    }
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
    // Start from the summary row: type data, material assets, quantities.
    let mut v = summary_of(doc, None, Some(&[id]))["elements"]
        .get(0)
        .cloned()
        .unwrap_or_else(|| summary_one(e));
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
    // Quantities in display units, e.g. "0.8 W/(m·K)" (key order, deterministic).
    let shown: Map<String, Value> = e
        .params
        .iter()
        .filter(|(_, p)| p.spec().is_some())
        .map(|(k, p)| (k.clone(), json!(p.display())))
        .collect();
    if !shown.is_empty() {
        v["parameters_display"] = Value::Object(shown);
    }
    Ok(v)
}

/// `list_relationships`: rows (all, or those linking `element`), in uid
/// order.
fn relationships(doc: &Document, args: &Value) -> Value {
    let only = args.get("element").and_then(Value::as_u64).map(ElementId);
    let rows: Vec<Value> = doc
        .relationships()
        .filter(|r| only.is_none_or(|e| r.involves(e)))
        .map(|r| {
            json!({"relationship": r.uid.to_string(), "kind": r.kind.name(), "elements": r.elements, "phase": r.phase})
        })
        .collect();
    json!({ "relationships": rows })
}

/// `get_phase_quantities`: [`PhaseQuantities`] in m³ / m², keyed by category
/// then status. `phase` defaults to the last phase.
///
/// [`PhaseQuantities`]: ferro_core::model::PhaseQuantities
pub fn phase_quantities(doc: &Document, args: &Value) -> Result<Value, String> {
    let phase = match args.get("phase").and_then(Value::as_u64) {
        Some(p) => ElementId(p),
        None => doc
            .phases()
            .last()
            .map(|p| p.id)
            .ok_or("the project has no phases yet")?,
    };
    let q = ferro_core::model::PhaseQuantities::from_document(doc, phase)
        .ok_or_else(|| format!("{phase} is not a phase"))?;
    let totals = |t: &ferro_core::model::phase_quantities::StatusTotals| json!({"count": t.count, "volume_m3": (t.volume_mm3 / 1e9 * 1000.0).round() / 1000.0, "area_m2": (t.area_mm2 / 1e6 * 1000.0).round() / 1000.0});
    let mut cats = Map::new();
    for (c, by) in &q.categories {
        let mut m = Map::new();
        for (st, t) in by {
            m.insert(st.as_str().into(), totals(t));
        }
        cats.insert(c.to_string(), Value::Object(m));
    }
    let mut total = Map::new();
    for st in ferro_core::model::PhaseStatus::ALL
        .iter()
        .filter(|s| s.is_shown())
    {
        total.insert(st.as_str().into(), totals(&q.total(*st)));
    }
    let name = doc.get(phase).map(|e| e.label());
    Ok(json!({"phase": phase, "phase_name": name, "categories": cats, "total": total}))
}

fn summary_one(e: &ferro_core::Element) -> Value {
    json!({"id": e.id, "uid": e.uid.to_string(), "name": e.name, "category": e.category, "body": e.body.kind(), "level": e.level_id})
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
        let mut args = args.clone();
        let in_option = match args
            .as_object_mut()
            .and_then(|o| o.remove("in_design_option"))
        {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_u64() {
                Some(n) if !DESIGN_OPTION_COMMANDS.contains(&name) => Some(ElementId(n)),
                Some(_) => return Some(err(format!("{name} cannot run inside a design option"))),
                None => return Some(err("in_design_option must be an option id".into())),
            },
        };
        let mut norm = match normalize(&args) {
            Ok(v) => v,
            Err(m) => return Some(err(m)),
        };
        // `position` of a phase / option set is an index, not a length.
        if matches!(name, "set_phase" | "set_option_set") {
            if let Some(p) = args.get("position") {
                norm["position"] = p.clone();
            }
        }
        if name == "set_parameter" {
            let key = norm
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(v) = norm.get("value").cloned() {
                match param_value(session.doc(), &key, &v) {
                    Ok(v) => norm["value"] = v,
                    Err(m) => return Some(err(m)),
                }
            }
        }
        let cmd = match AnyCommand::from_tool(name, &norm) {
            Ok(c) => c,
            Err(m) => return Some(err(m)),
        };
        let result = match in_option {
            Some(o) => {
                session.execute_in_option(o, &ferro_core::model::Combination::primary(), cmd, ctx)
            }
            None => session.execute(cmd, ctx),
        };
        return Some(match result {
            Ok(o) => Ok(
                json!({"result": o.output, "changed": o.changed, "warnings": failures_json(&o.warnings)}),
            ),
            Err(f) => Err(
                json!({"error": "command failed; nothing was changed", "failures": failures_json(&f)}),
            ),
        });
    }
    Some(match name {
        "list_elements" => {
            let combo = match design_option::combination_arg(args.get("combination"), true) {
                Ok(c) => c,
                Err(m) => return Some(err(m)),
            };
            let category = args.get("category").and_then(Value::as_str);
            match combo {
                None => Ok(summary(session.doc(), category)),
                Some(_) => design_option::seen(session.doc(), &combo)
                    .map(|d| summary(&d, category)),
            }
        }
        "list_design_options" => Ok(design_option::list_design_options(session.doc())),
        "diff_documents" => design_option::diff_documents(session.doc(), args),
        "get_element" => match args.get("element").and_then(Value::as_u64) {
            Some(id) => detail(session.doc(), ElementId(id)).map_err(|m| {
                json!({"error": m, "failures": [{"severity": "error", "code": "ferro.model.not_found", "message": m, "elements": [id]}]})
            }),
            None => err("missing element".into()),
        },
        "get_phase_quantities" => phase_quantities(session.doc(), args).or_else(err),
        "list_relationships" => Ok(relationships(session.doc(), args)),
        "undo" => match session.undo() {
            Some(c) => Ok(json!({"changed": c})),
            None => err("nothing to undo".into()),
        },
        "redo" => match session.redo() {
            Some(c) => Ok(json!({"changed": c})),
            None => err("nothing to redo".into()),
        },
        "get_document_version" => {
            Ok(json!({"version": session.doc().version(), "project": session.doc().project().name}))
        }
        "revit_plan" => {
            let state: ferro_revit::RevitState = match args.get("state") {
                Some(v) if !v.is_null() => match serde_json::from_value(v.clone()) {
                    Ok(s) => s,
                    Err(e) => return Some(err(format!("invalid state: {e}"))),
                },
                _ => Default::default(),
            };
            Ok(
                json!({"version": session.doc().version(), "ops": ferro_revit::plan(session.doc(), &state)}),
            )
        }
        "revit_import" => {
            let snap: ferro_revit::RevitSnapshot =
                match serde_json::from_value(args.get("snapshot").cloned().unwrap_or_default()) {
                    Ok(s) => s,
                    Err(e) => return Some(err(format!("invalid snapshot: {e}"))),
                };
            Ok(json!({"report": ferro_revit::import(session, ctx, &snap)}))
        }
        "get_journal" => {
            let n = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let j = session.journal();
            Ok(json!(j[j.len().saturating_sub(n)..]))
        }
        "get_selection" | "select_elements" => {
            err("this tool needs the Ferro app open (Ferro Link)".into())
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
            "Export the model geometry to Wavefront OBJ (Y-up, metres).",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        tool(
            "export_glb",
            "Export the model geometry to binary glTF (Y-up, metres).",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
        tool(
            "export_ifc",
            "Export to IFC4 (walls, slabs, doors, windows with openings, types with material layers, storeys, base quantities; millimetres).",
            json!({"path":{"type":"string"}}),
            &["path"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::SequentialIds;
    use ferro_core::QuantitySpec;

    fn session() -> Session {
        Session::new(Document::new("T"), Box::new(SequentialIds::new(1)))
    }

    fn ok(s: &mut Session, name: &str, args: Value) -> Value {
        call(s, &ExecContext::user(), name, &args)
            .expect("handled")
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn summaries_name_the_body_kind_and_missing_elements_have_a_code() {
        let mut s = session();
        ok(
            &mut s,
            "create_level",
            json!({"name": "L1", "elevation": 0}),
        );
        ok(
            &mut s,
            "create_walls",
            json!({"points": [[0, 0], ["4m", 0]]}),
        );
        ok(
            &mut s,
            "create_box",
            json!({"origin": [0, "9m", 0], "width": "1m", "depth": "1m", "height": "1m"}),
        );
        let all = ok(&mut s, "list_elements", json!({}));
        let kinds: std::collections::BTreeSet<&str> = all["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["body"].as_str().expect("every row has a body kind"))
            .collect();
        for k in ["level", "wall", "wall_type", "form"] {
            assert!(kinds.contains(k), "{k} in {kinds:?}");
        }
        for e in all["elements"].as_array().unwrap() {
            let d = ok(&mut s, "get_element", json!({"element": e["id"]}));
            assert_eq!(d["body"], e["body"]);
        }
        let r = call(
            &mut s,
            &ExecContext::user(),
            "get_element",
            &json!({"element": 999}),
        )
        .expect("handled")
        .unwrap_err();
        assert_eq!(r["failures"][0]["code"], "ferro.model.not_found");
    }

    #[test]
    fn relationship_tools_round_trip_through_mcp() {
        let tools = list();
        let t = tools
            .iter()
            .find(|t| t["name"] == "create_relationship")
            .expect("schema");
        assert_eq!(
            t["inputSchema"]["properties"]["kind"]["enum"],
            json!([
                "join",
                "cut",
                "host",
                "attach",
                "bounds",
                "connect",
                "aggregate"
            ])
        );
        let mut s = session();
        let mut b = || {
            ok(
                &mut s,
                "create_box",
                json!({"origin": [0, 0, 0], "width": "1m", "depth": "1m", "height": "1m"}),
            )["result"]["element"]
                .as_u64()
                .unwrap()
        };
        let (x, y, z) = (b(), b(), b());
        let u = ok(
            &mut s,
            "create_relationship",
            json!({"kind": "join", "elements": [x, y]}),
        )["result"]["relationship"]
            .as_str()
            .unwrap()
            .to_string();
        ok(
            &mut s,
            "create_relationship",
            json!({"kind": "connect", "elements": [y, z]}),
        );
        let all = ok(&mut s, "list_relationships", json!({}));
        assert_eq!(all["relationships"].as_array().unwrap().len(), 2);
        let of_x = ok(&mut s, "list_relationships", json!({"element": x}));
        assert_eq!(
            of_x["relationships"],
            json!([{"relationship": u, "kind": "join", "elements": [x, y], "phase": null}])
        );
        ok(&mut s, "delete_relationship", json!({"relationship": u}));
        let of_x = ok(&mut s, "list_relationships", json!({"element": x}));
        assert_eq!(of_x["relationships"], json!([]));
        // A bad kind is an argument error, not a crash.
        let r = call(
            &mut s,
            &ExecContext::user(),
            "create_relationship",
            &json!({"kind": "glue", "elements": [x, y]}),
        )
        .expect("handled");
        assert!(r.is_err());
    }

    #[test]
    fn phase_tools_have_schemas_and_quantities_in_metres() {
        let tools = list();
        for name in [
            "create_phase",
            "set_phase",
            "delete_phase",
            "set_element_phase",
            "set_renovation_status",
            "get_phase_quantities",
        ] {
            let t = tools.iter().find(|t| t["name"] == name).expect(name);
            assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        }
        let t = tools
            .iter()
            .find(|t| t["name"] == "set_renovation_status")
            .unwrap();
        assert_eq!(
            t["inputSchema"]["properties"]["status"]["enum"],
            json!(["existing", "demolish", "new"])
        );
        let mut s = session();
        let r = call(
            &mut s,
            &ExecContext::user(),
            "get_phase_quantities",
            &json!({}),
        )
        .expect("handled")
        .unwrap_err();
        assert!(r["error"].as_str().unwrap().contains("no phases"), "{r}");
        let b = ok(
            &mut s,
            "create_box",
            json!({"origin": [0, 0, 0], "width": "1m", "depth": "2m", "height": "1m"}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        ok(
            &mut s,
            "set_renovation_status",
            json!({"elements": [b], "status": "demolish"}),
        );
        let q = ok(&mut s, "get_phase_quantities", json!({}));
        assert_eq!(q["phase_name"], "New Construction");
        assert_eq!(q["categories"]["forms"]["demolished"]["volume_m3"], 2.0);
        assert_eq!(q["total"]["demolished"]["count"], 1);
        let d = detail(s.doc(), ElementId(b)).unwrap();
        assert!(d["phase_demolished"].is_u64(), "{d}");
    }

    #[test]
    fn view_tools_take_user_units_and_report_views() {
        let tools = list();
        for name in ["create_view", "set_view", "delete_view"] {
            let t = tools.iter().find(|t| t["name"] == name).expect(name);
            assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        }
        let mut s = session();
        let l = ok(
            &mut s,
            "create_level",
            json!({"name": "L1", "elevation": "3m"}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        let p = ok(
            &mut s,
            "create_view",
            json!({"name": "Plan", "view_type": "plan_view", "level": l,
                   "view_range": {"top": "2.5m", "cut_height": "1.1m", "bottom": 0, "view_depth": 0}}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        let v = ok(
            &mut s,
            "create_view",
            json!({"name": "3D", "view_type": "view_3d",
                   "camera": {"eye": ["10m", 0, "2m"], "target": [0, 0, 0], "up": [0, 0, 1], "fov_y": 60},
                   "section_plane": {"point": [0, 0, "1.2m"], "normal": [0, 0, 1]}}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        let b = ok(
            &mut s,
            "create_box",
            json!({"origin": [0, 0, 0], "width": "1m", "depth": "1m", "height": "1m"}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        ok(&mut s, "set_view", json!({"view": v, "hide": [b]}));
        let doc = s.doc();
        let view = doc.get(ElementId(v)).unwrap().as_view().unwrap();
        assert!((view.camera.eye.x - 10_000.0).abs() < 1e-9);
        assert!(
            (view.camera.fov_y.deg() - 60.0).abs() < 1e-9,
            "degrees at the edge"
        );
        let cut = doc.view_cut_plane(doc.get(ElementId(p)).unwrap()).unwrap();
        assert!((cut.point.z - 4100.0).abs() < 1e-9, "3 m + 1.1 m");
        let d = detail(doc, ElementId(v)).unwrap();
        assert_eq!(d["hidden_elements"], json!([b]), "{d}");
        assert_eq!(d["camera"]["fov_y_deg"], 60.0);
        assert_eq!(d["view_type"]["type"], "view_3d");
        ok(&mut s, "delete_view", json!({"view": v}));
        assert!(s.doc().get(ElementId(v)).is_none());
    }

    #[test]
    fn set_parameter_takes_quantities_with_units_and_definitions() {
        let mut s = session();
        let b = ok(
            &mut s,
            "create_box",
            json!({"origin": [0, 0, 0], "width": "1m", "depth": "1m", "height": "1m"}),
        )["result"]["element"]
            .as_u64()
            .unwrap();
        ok(
            &mut s,
            "set_parameter",
            json!({"element": b, "key": "acme.energy.k", "value": "0.8 W/mK"}),
        );
        ok(
            &mut s,
            "set_parameter_definition",
            json!({"key": "acme.energy.u", "data_type": {"quantity": "u_value"}}),
        );
        // Bare number in text = default unit of the defined spec.
        ok(
            &mut s,
            "set_parameter",
            json!({"element": b, "key": "acme.energy.u", "value": "0.25"}),
        );
        let e = s.doc().get(ElementId(b)).unwrap();
        assert_eq!(
            e.param("acme.energy.u").and_then(|p| p.as_quantity()),
            Some((QuantitySpec::UValue, 0.25))
        );
        // A unit of another spec is refused at the edge.
        let r = call(
            &mut s,
            &ExecContext::user(),
            "set_parameter",
            &json!({"element": b, "key": "acme.energy.u", "value": "1200 kg/m3"}),
        )
        .expect("handled")
        .unwrap_err();
        assert!(r["error"].as_str().unwrap().contains("W/(m²·K)"), "{r}");
        let d = detail(s.doc(), ElementId(b)).unwrap();
        assert_eq!(
            d["parameters_display"]["acme.energy.k"],
            json!("0.8 W/(m·K)")
        );
        assert_eq!(
            d["parameters_display"]["acme.energy.u"],
            json!("0.25 W/(m²·K)")
        );
    }

    #[test]
    fn agents_edit_inside_design_options_and_diff_combinations() {
        let tools = list();
        let wall = tools.iter().find(|t| t["name"] == "create_box").unwrap();
        assert!(wall["inputSchema"]["properties"]["in_design_option"].is_object());
        let acc = tools
            .iter()
            .find(|t| t["name"] == "accept_design_option")
            .unwrap();
        assert!(acc["inputSchema"]["properties"]["in_design_option"].is_null());
        let mut s = session();
        let o = ok(&mut s, "create_option_set", json!({"name": "Roof"}));
        let set = o["result"]["element"].as_u64().unwrap();
        let b = ok(&mut s, "create_design_option", json!({"set": set}))["result"]["element"]
            .as_u64()
            .unwrap();
        ok(
            &mut s,
            "create_box",
            json!({"origin": [0, 0, 0], "width": "2m", "depth": "2m", "height": "2m", "in_design_option": b}),
        );
        assert_eq!(
            s.doc().len(),
            3,
            "shared model has the set and two options only"
        );
        let l = ok(&mut s, "list_design_options", json!({}));
        assert_eq!(l["sets"][0]["options"][1]["added"], 1, "{l}");
        assert_eq!(l["failures"], json!([]));
        let seen = ok(&mut s, "list_elements", json!({"combination": [b]}));
        let forms: Vec<&Value> = seen["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["category"] == "forms")
            .collect();
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0]["design_option"], b);
        let d = ok(&mut s, "diff_documents", json!({"to": [b]}));
        assert_eq!(d["count"], 1, "{d}");
        let none = ok(&mut s, "diff_documents", json!({}));
        assert_eq!(none["count"], 0, "primary option is empty");
        // Position is an index, not a length.
        ok(&mut s, "set_option_set", json!({"set": set, "position": 0}));
        let e = call(
            &mut s,
            &ExecContext::user(),
            "accept_design_option",
            &json!({"option": b, "in_design_option": b}),
        )
        .unwrap()
        .unwrap_err();
        assert!(e["error"].as_str().unwrap().contains("cannot run inside"));
        // Applying the diff to the shared model reproduces the option.
        ok(&mut s, "apply_changes", json!({"changes": d["changes"]}));
        assert_eq!(s.doc().len(), 4);
    }
}
