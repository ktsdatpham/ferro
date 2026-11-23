//! Scenario audit of every command (M5-15.1, M5-15.2): basic, degenerate
//! and tolerance-edge cases through the tool path (`from_tool` + `execute`),
//! rollback on failure, and exact undo / redo.

use super::scenario_common_tests::*;
use super::*;
use crate::math::Vec3;
use crate::model::{ElementId, GeometrySource};
use crate::topo::FaceId;
use serde_json::{json, Value};

/// A document with one element of every kind.
struct Fx {
    s: Session,
    l1: ElementId,
    l2: ElementId,
    mat: ElementId,
    wt: ElementId,
    ft: ElementId,
    /// Straight wall (0,0)→(6000,0) on L1, 3 m high, type `wt`.
    wall: ElementId,
    /// Wall (6000,0)→(6000,4000) joined to `wall`, top attached to L2.
    wall2: ElementId,
    door: ElementId,
    window: ElementId,
    floor: ElementId,
    arc: ElementId,
    /// 2 m cube at (10 m, 0, 0).
    bx: ElementId,
    /// Open 1 m square sheet at (20 m, 0, 0) and its face.
    sheet: ElementId,
    sheet_face: FaceId,
    /// Phases "Existing" and "New Construction" (used by no element).
    ph_old: ElementId,
    ph_new: ElementId,
    /// A `connect` relationship between `wall` and `bx` (its uid).
    rel: String,
    /// A 3D view hiding nothing (M5-22).
    view: ElementId,
}

impl Fx {
    fn new() -> Fx {
        let mut s = session();
        let l1 = id(&run(
            &mut s,
            "create_level",
            json!({"name": "L1", "elevation": 0}),
        ));
        let l2 = id(&run(
            &mut s,
            "create_level",
            json!({"name": "L2", "elevation": 3000}),
        ));
        let mat = id(&run(
            &mut s,
            "create_material",
            json!({"name": "Concrete", "color": "#808080"}),
        ));
        let wt = id(&run(
            &mut s,
            "create_wall_type",
            json!({"name": "Ext 300", "layers": [
                {"function": "finish", "thickness": 20, "material": mat},
                {"function": "structure", "thickness": 260, "material": mat},
                {"function": "finish", "thickness": 20}]}),
        ));
        let ft = id(&run(
            &mut s,
            "create_floor_type",
            json!({"name": "Slab 250", "thickness": 250, "material": mat}),
        ));
        let wall = id(&run(
            &mut s,
            "create_wall",
            json!({"start": p(0.0, 0.0, 0.0), "end": p(6000.0, 0.0, 0.0), "level": l1, "wall_type": wt, "height": 3000, "name": "South"}),
        ));
        let wall2 = id(&run(
            &mut s,
            "create_wall",
            json!({"start": p(6000.0, 0.0, 0.0), "end": p(6000.0, 4000.0, 0.0), "level": l1, "wall_type": wt, "top_level": l2}),
        ));
        let door = id(&run(
            &mut s,
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 1500}),
        ));
        let window = id(&run(
            &mut s,
            "create_opening",
            json!({"wall": wall, "kind": "window", "position": 4000}),
        ));
        let floor = id(&run(
            &mut s,
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":6000,"y":0},{"x":6000,"y":4000},{"x":0,"y":4000}],
                   "holes": [[{"x":1000,"y":1000},{"x":1000,"y":2000},{"x":2000,"y":2000},{"x":2000,"y":1000}]],
                   "level": l1, "floor_type": ft}),
        ));
        let arc = id(&run(
            &mut s,
            "create_arc_wall",
            json!({"start": p(0.0, 8000.0, 0.0), "mid": p(3000.0, 9000.0, 0.0), "end": p(6000.0, 8000.0, 0.0)}),
        ));
        let bx = boxed(&mut s, [10_000.0, 0.0, 0.0], 2000.0, 2000.0, 2000.0);
        let o = run(
            &mut s,
            "create_rectangle",
            json!({"origin": p(20_000.0, 0.0, 0.0), "width": 1000, "depth": 1000}),
        );
        let (sheet, sheet_face) = (id(&o), face_of(&o));
        let ph_old = id(&run(&mut s, "create_phase", json!({"name": "Existing"})));
        let ph_new = id(&run(
            &mut s,
            "create_phase",
            json!({"name": "New Construction"}),
        ));
        let rel = run(
            &mut s,
            "create_relationship",
            json!({"kind": "connect", "elements": [wall, bx]}),
        )
        .output["relationship"]
            .as_str()
            .expect("uid")
            .to_string();
        let view = id(&run(
            &mut s,
            "create_view",
            json!({"name": "3D", "view_type": "view_3d"}),
        ));
        check_document(s.doc()).expect("fixture is valid");
        Fx {
            s,
            l1,
            l2,
            mat,
            wt,
            ft,
            wall,
            wall2,
            door,
            window,
            floor,
            arc,
            bx,
            sheet,
            sheet_face,
            ph_old,
            ph_new,
            rel,
            view,
        }
    }

    fn top(&self, e: ElementId) -> FaceId {
        face_towards(&geometry(&self.s, e), Vec3::Z)
    }
    fn side(&self, e: ElementId, d: Vec3) -> FaceId {
        face_towards(&geometry(&self.s, e), d)
    }
    /// Every element id in the fixture plus one that does not exist.
    fn all_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.s.doc().elements().map(|e| e.id.0).collect();
        v.push(9999);
        v
    }
}

// ------------------------------------------------------------ 1. failures

/// Degenerate inputs, wrong / missing elements, wrong kinds: each fails
/// with its documented code and leaves the document untouched.
#[test]
fn every_command_rejects_degenerate_and_wrong_inputs_with_rollback() {
    let mut f = Fx::new();
    let (wall, door, floor, bx, sheet, arc) = (f.wall, f.door, f.floor, f.bx, f.sheet, f.arc);
    let (l1, l2, mat, wt, ft) = (f.l1, f.l2, f.mat, f.wt, f.ft);
    let bx_top = f.top(bx);
    let floor_side = f.side(floor, Vec3::X);
    let wall_top = f.top(wall);
    let sq = |x: f64| {
        json!([
            p(x, 0.0, 0.0),
            p(x + 1000.0, 0.0, 0.0),
            p(x + 1000.0, 1000.0, 0.0),
            p(x, 1000.0, 0.0)
        ])
    };
    let cases: Vec<(&str, Value, &str)> = vec![
        // levels
        (
            "create_level",
            json!({"name": "L1", "elevation": 500}),
            "ferro.model.duplicate_name",
        ),
        (
            "set_level_elevation",
            json!({"level": wall, "elevation": 100}),
            "ferro.model.not_a_level",
        ),
        (
            "set_level_elevation",
            json!({"level": 9999, "elevation": 100}),
            "ferro.model.not_found",
        ),
        // wall2's top is attached to L2: moving L2 below L1 inverts it.
        (
            "set_level_elevation",
            json!({"level": l2, "elevation": -100}),
            "ferro.wall.no_height",
        ),
        // rectangles / polygons / boxes
        (
            "create_rectangle",
            json!({"origin": p(0.0, 0.0, 0.0), "width": 0, "depth": 1000}),
            "ferro.geom.short_curve",
        ),
        (
            "create_rectangle",
            json!({"origin": p(0.0, 0.0, 0.0), "width": 1000, "depth": 0.49}),
            "ferro.geom.short_curve",
        ),
        (
            "create_rectangle",
            json!({"origin": p(0.0, 0.0, 0.0), "width": 1000, "depth": 1000, "level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "create_rectangle",
            json!({"origin": p(0.0, 0.0, 0.0), "width": 1000, "depth": 1000, "level": 9999}),
            "ferro.model.not_found",
        ),
        (
            "create_rectangle",
            json!({"origin": p(9_999_500.0, 0.0, 0.0), "width": 1000, "depth": 1000}),
            "ferro.model.out_of_extent",
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0)]}),
            "ferro.geom.degenerate",
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0), p(2000.0,0.0,0.0)]}),
            "ferro.geom.degenerate",
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0), p(1000.0,1000.0,0.1), p(0.0,1000.0,0.0)]}),
            "ferro.geom.not_planar",
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0), p(1000.0,0.005,0.0), p(1000.0,1000.0,0.0)]}),
            "ferro.geom.short_curve",
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0), p(1000.0,0.02,0.0), p(1000.0,1000.0,0.0)]}),
            "ferro.geom.short_curve",
        ),
        (
            "create_polygon",
            json!({"points": sq(0.0), "level": wall, "relative_to_level": true}),
            "ferro.model.not_a_level",
        ),
        (
            "create_box",
            json!({"origin": p(0.0,0.0,0.0), "width": 1000, "depth": 1000, "height": 0}),
            "ferro.algo.zero_distance",
        ),
        (
            "create_box",
            json!({"origin": p(0.0,0.0,0.0), "width": 1000, "depth": 1000, "height": 0.49}),
            "ferro.algo.zero_distance",
        ),
        (
            "create_box",
            json!({"origin": p(0.0,0.0,9_999_500.0), "width": 1000, "depth": 1000, "height": 1000}),
            "ferro.model.out_of_extent",
        ),
        // push / pull
        (
            "push_pull_face",
            json!({"element": bx, "face": bx_top, "distance": 0.49}),
            "ferro.algo.zero_distance",
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": bx_top, "distance": -0.49}),
            "ferro.algo.zero_distance",
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": 9999, "distance": 100}),
            "ferro.topo.not_found",
        ),
        (
            "push_pull_face",
            json!({"element": 9999, "face": 1, "distance": 100}),
            "ferro.model.not_found",
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": bx_top, "distance": -2000}),
            "ferro.algo.collapse",
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": bx_top, "distance": -2500}),
            "ferro.algo.collapse",
        ),
        (
            "push_pull_face",
            json!({"element": l1, "face": 1, "distance": 100}),
            "ferro.model.no_geometry",
        ),
        (
            "push_pull_face",
            json!({"element": mat, "face": 1, "distance": 100}),
            "ferro.model.no_geometry",
        ),
        (
            "push_pull_face",
            json!({"element": floor, "face": floor_side, "distance": 100}),
            "ferro.ops.unsupported_push_pull",
        ),
        (
            "push_pull_face",
            json!({"element": wall, "face": wall_top, "distance": -3000}),
            "ferro.wall.no_height",
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": bx_top, "distance": 1e7}),
            "ferro.model.out_of_extent",
        ),
        // edits
        (
            "move_elements",
            json!({"elements": [l1], "vector": p(0.0,0.0,100.0)}),
            "ferro.model.not_movable",
        ),
        (
            "move_elements",
            json!({"elements": [mat], "vector": p(100.0,0.0,0.0)}),
            "ferro.model.not_movable",
        ),
        (
            "move_elements",
            json!({"elements": [wt], "vector": p(100.0,0.0,0.0)}),
            "ferro.model.not_movable",
        ),
        (
            "move_elements",
            json!({"elements": [bx, 9999], "vector": p(100.0,0.0,0.0)}),
            "ferro.model.not_found",
        ),
        (
            "move_elements",
            json!({"elements": [bx], "vector": p(1e7,0.0,0.0)}),
            "ferro.model.out_of_extent",
        ),
        (
            "move_elements",
            json!({"elements": [wall], "vector": p(1e7,0.0,0.0)}),
            "ferro.model.out_of_extent",
        ),
        (
            "rotate_elements",
            json!({"elements": [l1], "center": p(0.0,0.0,0.0), "angle": 0.5}),
            "ferro.model.not_movable",
        ),
        (
            "rotate_elements",
            json!({"elements": [9999], "center": p(0.0,0.0,0.0), "angle": 0.5}),
            "ferro.model.not_found",
        ),
        (
            "copy_elements",
            json!({"elements": [bx], "vector": p(100.0,0.0,0.0), "count": 0}),
            "ferro.ops.invalid_count",
        ),
        (
            "copy_elements",
            json!({"elements": [bx], "vector": p(100.0,0.0,0.0), "count": 1001}),
            "ferro.ops.invalid_count",
        ),
        (
            "copy_elements",
            json!({"elements": [9999], "vector": p(100.0,0.0,0.0)}),
            "ferro.model.not_found",
        ),
        (
            "copy_elements",
            json!({"elements": [l1], "vector": p(100.0,0.0,0.0)}),
            "ferro.model.not_movable",
        ),
        (
            "copy_elements",
            json!({"elements": [bx], "vector": p(5e6,0.0,0.0), "count": 3}),
            "ferro.model.out_of_extent",
        ),
        (
            "delete_elements",
            json!({"elements": [9999]}),
            "ferro.model.not_found",
        ),
        (
            "delete_elements",
            json!({"elements": [bx, 9999]}),
            "ferro.model.not_found",
        ),
        (
            "delete_elements",
            json!({"elements": [wt]}),
            "ferro.model.type_in_use",
        ),
        (
            "delete_elements",
            json!({"elements": [ft]}),
            "ferro.model.type_in_use",
        ),
        // data
        (
            "set_name",
            json!({"element": 9999, "name": "x"}),
            "ferro.model.not_found",
        ),
        (
            "set_parameter",
            json!({"element": bx, "key": "nokey", "value": {"type": "number", "value": 1}}),
            "ferro.model.invalid_parameter_key",
        ),
        (
            "set_parameter",
            json!({"element": bx, "key": "ferro.form.volume", "value": {"type": "number", "value": 1}}),
            "ferro.model.read_only_parameter",
        ),
        (
            "set_parameter",
            json!({"element": 9999, "key": "acme.x.y", "value": {"type": "number", "value": 1}}),
            "ferro.model.not_found",
        ),
        (
            "set_parameter",
            json!({"element": bx, "key": "acme.x.y", "value": {"type": "element_ref", "value": 9999}}),
            "ferro.model.not_found",
        ),
        (
            "set_category",
            json!({"element": bx, "category": "levels"}),
            "ferro.model.invalid_category",
        ),
        (
            "set_category",
            json!({"element": wall, "category": "walls"}),
            "ferro.model.not_a_form",
        ),
        (
            "set_category",
            json!({"element": 9999, "category": "walls"}),
            "ferro.model.not_found",
        ),
        (
            "set_geometry_source",
            json!({"element": l1, "source": "direct"}),
            "ferro.model.no_geometry",
        ),
        (
            "set_geometry_source",
            json!({"element": door, "source": "direct"}),
            "ferro.model.hosted_opening",
        ),
        (
            "set_geometry_source",
            json!({"element": bx, "source": "parametric"}),
            "ferro.model.not_recognised",
        ),
        (
            "set_geometry_source",
            json!({"element": 9999, "source": "direct"}),
            "ferro.model.not_found",
        ),
        // materials and types
        (
            "set_material",
            json!({"material": wall, "color": "#ffffff"}),
            "ferro.model.not_a_material",
        ),
        (
            "set_material",
            json!({"material": 9999, "color": "#ffffff"}),
            "ferro.model.not_found",
        ),
        (
            "set_material",
            json!({"material": mat, "color": "red"}),
            "ferro.model.invalid_color",
        ),
        (
            "set_material",
            json!({"material": mat, "roughness": 1.5}),
            "ferro.model.invalid_material",
        ),
        (
            "set_material",
            json!({"material": mat, "thermal": {"conductivity_w_mk": -1}}),
            "ferro.model.invalid_material",
        ),
        (
            "set_material",
            json!({"material": mat, "thermal": {"conductivity": 1}}),
            "ferro.model.invalid_material",
        ),
        (
            "set_material",
            json!({"material": mat, "thermal": 3}),
            "ferro.model.invalid_material",
        ),
        (
            "set_parameter_definition",
            json!({"key": "bad key", "data_type": "text"}),
            "ferro.model.invalid_parameter_key",
        ),
        (
            "set_parameter_definition",
            json!({"key": "ferro.common.comments", "data_type": "text"}),
            "ferro.model.builtin_parameter_definition",
        ),
        (
            "set_parameter_definition",
            json!({"key": "acme.x.y", "data_type": "text", "ifc": {"property_set": " ", "property": "P"}}),
            "ferro.model.invalid_parameter_definition",
        ),
        (
            "create_material",
            json!({"name": "Concrete", "color": "#ffffff"}),
            "ferro.model.duplicate_name",
        ),
        (
            "create_material",
            json!({"name": "Bad", "color": "#12345"}),
            "ferro.model.invalid_color",
        ),
        (
            "create_material",
            json!({"name": "Bad", "color": "#gg0000"}),
            "ferro.model.invalid_color",
        ),
        (
            "create_material",
            json!({"name": "Bad", "color": "#ééé"}),
            "ferro.model.invalid_color",
        ),
        (
            "create_wall_type",
            json!({"name": "Ext 300", "thickness": 100}),
            "ferro.model.duplicate_name",
        ),
        (
            "create_wall_type",
            json!({"name": "T0", "thickness": 0.49}),
            "ferro.model.invalid_structure",
        ),
        (
            "create_wall_type",
            json!({"name": "T0"}),
            "ferro.model.invalid_structure",
        ),
        (
            "create_wall_type",
            json!({"name": "T0", "layers": []}),
            "ferro.model.invalid_structure",
        ),
        (
            "create_wall_type",
            json!({"name": "T0", "layers": [{"function": "structure", "thickness": -10}, {"function": "finish", "thickness": 300}]}),
            "ferro.model.invalid_structure",
        ),
        (
            "create_wall_type",
            json!({"name": "T0", "thickness": 100, "material": wall}),
            "ferro.model.not_a_material",
        ),
        (
            "create_wall_type",
            json!({"name": "T0", "thickness": 100, "material": 9999}),
            "ferro.model.not_found",
        ),
        (
            "create_floor_type",
            json!({"name": "Slab 250", "thickness": 100}),
            "ferro.model.duplicate_name",
        ),
        (
            "create_floor_type",
            json!({"name": "F0", "thickness": 0}),
            "ferro.model.invalid_structure",
        ),
        // walls
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(0.0,0.0,0.0)}),
            "ferro.geom.short_curve",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(0.49,0.0,0.0)}),
            "ferro.geom.short_curve",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(0.0,0.0,3000.0)}),
            "ferro.geom.short_curve",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "height": 0}),
            "ferro.wall.no_height",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "height": 0.49}),
            "ferro.wall.no_height",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "wall_type": ft}),
            "ferro.model.wrong_type",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "wall_type": 9999}),
            "ferro.model.not_found",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "top_level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "create_wall",
            json!({"start": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0), "level": l2, "top_level": l1}),
            "ferro.wall.no_height",
        ),
        (
            "create_wall",
            json!({"start": p(9_999_000.0,0.0,0.0), "end": p(10_000_500.0,0.0,0.0)}),
            "ferro.model.out_of_extent",
        ),
        (
            "create_walls",
            json!({"points": [p(0.0,0.0,0.0)]}),
            "ferro.geom.degenerate",
        ),
        (
            "create_walls",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0)], "closed": true}),
            "ferro.geom.degenerate",
        ),
        (
            "create_walls",
            json!({"points": [p(0.0,0.0,0.0), p(1000.0,0.0,0.0), p(1000.0,0.005,0.0)]}),
            "ferro.geom.short_curve",
        ),
        (
            "create_arc_wall",
            json!({"start": p(0.0,0.0,0.0), "mid": p(500.0,0.0,0.0), "end": p(1000.0,0.0,0.0)}),
            "ferro.geom.degenerate",
        ),
        (
            "create_arc_wall",
            json!({"start": p(0.0,0.0,0.0), "mid": p(0.0,0.0,0.0), "end": p(1000.0,0.0,0.0)}),
            "ferro.geom.degenerate",
        ),
        (
            "set_wall_height",
            json!({"wall": wall, "height": 0}),
            "ferro.wall.no_height",
        ),
        (
            "set_wall_height",
            json!({"wall": wall, "height": -100}),
            "ferro.wall.no_height",
        ),
        (
            "set_wall_height",
            json!({"wall": bx, "height": 1000}),
            "ferro.model.not_a_wall",
        ),
        (
            "set_wall_height",
            json!({"wall": 9999, "height": 1000}),
            "ferro.model.not_found",
        ),
        // the door is 2100 high: a 2 m wall no longer holds it.
        (
            "set_element_type",
            json!({"element": wall, "element_type": ft}),
            "ferro.model.wrong_type",
        ),
        (
            "set_element_type",
            json!({"element": floor, "element_type": wt}),
            "ferro.model.wrong_type",
        ),
        (
            "set_element_type",
            json!({"element": bx, "element_type": wt}),
            "ferro.model.wrong_type",
        ),
        (
            "set_element_type",
            json!({"element": wall, "element_type": 9999}),
            "ferro.model.not_found",
        ),
        (
            "set_element_type",
            json!({"element": wall, "element_type": mat}),
            "ferro.model.wrong_type",
        ),
        // floors
        (
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":1000,"y":0}]}),
            "ferro.geom.degenerate",
        ),
        (
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":1000,"y":0},{"x":2000,"y":0}]}),
            // Same code as create_polygon (M5-15.12).
            "ferro.geom.degenerate",
        ),
        (
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":1000,"y":0},{"x":1000,"y":0.005},{"x":0,"y":1000}]}),
            "ferro.geom.short_curve",
        ),
        (
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":1000,"y":0},{"x":1000,"y":1000}], "floor_type": wt}),
            "ferro.model.wrong_type",
        ),
        (
            "create_floor",
            json!({"points": [{"x":0,"y":0},{"x":1000,"y":0},{"x":1000,"y":1000}], "level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "set_floor",
            json!({"floor": floor, "points": [{"x":0,"y":0},{"x":1000,"y":0}]}),
            "ferro.geom.degenerate",
        ),
        (
            "set_floor",
            json!({"floor": wall, "offset": 100}),
            "ferro.model.not_a_floor",
        ),
        (
            "set_floor",
            json!({"floor": 9999, "offset": 100}),
            "ferro.model.not_found",
        ),
        // openings
        (
            "create_opening",
            json!({"wall": bx, "kind": "door", "position": 500}),
            "ferro.model.not_a_wall",
        ),
        (
            "create_opening",
            json!({"wall": 9999, "kind": "door", "position": 500}),
            "ferro.model.not_found",
        ),
        (
            "create_opening",
            json!({"wall": arc, "kind": "window", "position": 2000}),
            "ferro.opening.curved_host",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 5900}),
            "ferro.opening.outside_host",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": -500}),
            "ferro.opening.outside_host",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 1600}),
            "ferro.opening.overlap",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 3000, "height": 3500}),
            "ferro.opening.too_tall",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 3000, "width": 0}),
            "ferro.opening.too_small",
        ),
        (
            "create_opening",
            json!({"wall": wall, "kind": "door", "position": 3000, "width": -900}),
            "ferro.opening.too_small",
        ),
        (
            "set_opening",
            json!({"opening": wall, "width": 100}),
            "ferro.model.not_an_opening",
        ),
        (
            "set_opening",
            json!({"opening": 9999, "width": 100}),
            "ferro.model.not_found",
        ),
        (
            "set_wall",
            json!({"wall": wall, "start": p(0.0,0.0,0.0)}),
            "ferro.model.invalid_location",
        ),
        (
            "set_wall",
            json!({"wall": wall, "start": p(0.0,0.0,0.0), "end": p(0.0,0.0,0.0)}),
            "ferro.geom.short_curve",
        ),
        (
            "set_wall",
            json!({"wall": wall, "top_level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "set_wall",
            json!({"wall": floor, "height": 1000}),
            "ferro.model.not_a_wall",
        ),
        (
            "set_wall",
            json!({"wall": wall, "height": -5}),
            "ferro.wall.no_height",
        ),
        // Forms
        (
            "imprint_face",
            json!({"element": wall, "face": 1, "points": [p(0.0,0.0,0.0), p(1.0,0.0,0.0)]}),
            "ferro.model.not_a_form",
        ),
        (
            "imprint_face",
            json!({"element": bx, "face": 9999, "points": [p(10_500.0,500.0,2000.0), p(11_000.0,500.0,2000.0)]}),
            "ferro.topo.not_found",
        ),
        (
            "create_circle",
            json!({"center": p(0.0,0.0,0.0), "radius": 0}),
            "ferro.geom.short_curve",
        ),
        (
            "create_circle",
            json!({"center": p(0.0,0.0,0.0), "radius": 0.49}),
            "ferro.geom.short_curve",
        ),
        (
            "create_circle",
            json!({"center": p(0.0,0.0,0.0), "radius": 100, "normal": p(0.0,0.0,0.0)}),
            "ferro.geom.degenerate",
        ),
        (
            "create_circle",
            json!({"center": p(0.0,0.0,0.0), "radius": 100, "element": wall, "face": 1}),
            "ferro.model.not_a_form",
        ),
        (
            "create_circle",
            json!({"center": p(0.0,0.0,0.0), "radius": 100, "level": bx}),
            "ferro.model.not_a_level",
        ),
        (
            "offset_face",
            json!({"element": bx, "face": bx_top, "distance": 0}),
            "ferro.geom.short_curve",
        ),
        (
            "offset_face",
            json!({"element": bx, "face": bx_top, "distance": 0.49}),
            "ferro.geom.short_curve",
        ),
        (
            "offset_face",
            json!({"element": bx, "face": bx_top, "distance": -100}),
            "ferro.geom.short_curve",
        ),
        (
            "offset_face",
            json!({"element": bx, "face": bx_top, "distance": 1000}),
            "ferro.geom.offset_too_large",
        ),
        (
            "offset_face",
            json!({"element": wall, "face": 1, "distance": 10}),
            "ferro.model.not_a_form",
        ),
        (
            "move_geometry",
            json!({"element": bx, "vector": p(100.0,0.0,0.0)}),
            "ferro.ops.nothing_selected",
        ),
        (
            "move_geometry",
            json!({"element": bx, "edges": [9999], "vector": p(100.0,0.0,0.0)}),
            "ferro.topo.not_found",
        ),
        (
            "move_geometry",
            json!({"element": bx, "faces": [9999], "vector": p(100.0,0.0,0.0)}),
            "ferro.topo.not_found",
        ),
        (
            "move_geometry",
            json!({"element": bx, "faces": [bx_top], "vector": p(0.0,0.0,-2000.0)}),
            "ferro.geom.degenerate",
        ),
        (
            "move_geometry",
            json!({"element": bx, "faces": [bx_top], "vector": p(0.0,0.0,-3000.0)}),
            "ferro.algo.collapse",
        ),
        (
            "move_geometry",
            json!({"element": wall, "faces": [1], "vector": p(0.0,0.0,100.0)}),
            "ferro.model.not_a_form",
        ),
        (
            "scale_elements",
            json!({"elements": [bx], "center": p(0.0,0.0,0.0)}),
            "ferro.ops.invalid_scale",
        ),
        (
            "scale_elements",
            json!({"elements": [bx], "center": p(0.0,0.0,0.0), "factor": 0}),
            "ferro.ops.invalid_scale",
        ),
        (
            "scale_elements",
            json!({"elements": [bx], "center": p(0.0,0.0,0.0), "factors": [1, 1, 0]}),
            "ferro.ops.invalid_scale",
        ),
        (
            "scale_elements",
            json!({"elements": [wall], "center": p(0.0,0.0,0.0), "factor": 2}),
            "ferro.ops.not_scalable",
        ),
        (
            "scale_elements",
            json!({"elements": [9999], "center": p(0.0,0.0,0.0), "factor": 2}),
            "ferro.model.not_found",
        ),
        (
            "scale_elements",
            json!({"elements": [bx], "center": p(0.0,0.0,0.0), "factor": 1e4}),
            "ferro.model.out_of_extent",
        ),
        (
            "erase_edges",
            json!({"element": bx, "edges": [9999]}),
            "ferro.topo.not_found",
        ),
        (
            "erase_edges",
            json!({"element": wall, "edges": [1]}),
            "ferro.model.not_a_form",
        ),
        (
            "erase_faces",
            json!({"element": bx, "faces": [9999]}),
            "ferro.topo.not_found",
        ),
        (
            "erase_faces",
            json!({"element": floor, "faces": [1]}),
            "ferro.model.not_a_form",
        ),
        (
            "erase_faces",
            json!({"element": sheet, "faces": [9999]}),
            "ferro.topo.not_found",
        ),
    ];
    let mut failures = Vec::new();
    for (tool, args, code) in cases {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            expect_code(&mut f.s, tool, args.clone(), code)
        }));
        if r.is_err() {
            failures.push(format!("{tool} {args}"));
        }
    }
    assert!(
        failures.is_empty(),
        "failing cases:\n{}",
        failures.join("\n")
    );
}

// ------------------------------------------------- 1b. tolerance edges

/// Values just below a tolerance fail; just above succeed.
#[test]
fn tolerance_edges_accept_just_above_the_limit() {
    let mut f = Fx::new();
    let bx = f.bx;
    let top = f.top(bx);
    let ok: Vec<(&str, Value)> = vec![
        (
            "create_rectangle",
            json!({"origin": p(0.0, 20_000.0, 0.0), "width": 0.51, "depth": 1000}),
        ),
        (
            "create_box",
            json!({"origin": p(0.0, 22_000.0, 0.0), "width": 1000, "depth": 1000, "height": 0.51}),
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": top, "distance": 0.51}),
        ),
        (
            "push_pull_face",
            json!({"element": bx, "face": top, "distance": -0.51}),
        ),
        (
            "create_wall",
            json!({"start": p(0.0, 30_000.0, 0.0), "end": p(1000.0, 30_000.0, 0.0), "height": 0.51}),
        ),
        (
            "create_wall_type",
            json!({"name": "Thin", "thickness": 0.51}),
        ),
        (
            "create_floor_type",
            json!({"name": "Thin", "thickness": 0.51}),
        ),
        (
            "offset_face",
            json!({"element": bx, "face": top, "distance": 0.51}),
        ),
        // Coplanar within VERTEX_TOL: a 0.005 mm wobble is the same plane.
        (
            "create_polygon",
            json!({"points": [p(0.0,40_000.0,0.0), p(1000.0,40_000.0,0.0), p(1000.0,41_000.0,0.005), p(0.0,41_000.0,0.0)]}),
        ),
        // An edge just over 0.5 mm is allowed.
        (
            "create_polygon",
            json!({"points": [p(0.0,50_000.0,0.0), p(1000.0,50_000.0,0.0), p(1000.0,50_000.51,0.0), p(0.0,51_000.0,0.0)]}),
        ),
        (
            "create_rectangle",
            json!({"origin": p(9_998_000.0, 0.0, 0.0), "width": 1000, "depth": 1000}),
        ),
    ];
    for (tool, args) in ok {
        expect_ok(&mut f.s, tool, args);
    }
}

/// D-06 rule 3: no command may create an edge shorter than 0.5 mm. A circle
/// of radius 1 mm with 360 segments would have 0.017 mm edges.
#[test]
fn circles_never_create_edges_below_short_curve_tolerance() {
    let mut f = Fx::new();
    expect_code(
        &mut f.s,
        "create_circle",
        json!({"center": p(0.0, 60_000.0, 0.0), "radius": 1, "segments": 360}),
        "ferro.geom.short_curve",
    );
    let top = f.top(f.bx);
    expect_code(
        &mut f.s,
        "create_circle",
        json!({"center": p(11_000.0, 1000.0, 2000.0), "radius": 2, "segments": 360, "element": f.bx, "face": top}),
        "ferro.geom.short_curve",
    );
    // Segment length 2 r sin(pi / n) >= 0.5 mm is fine.
    expect_ok(
        &mut f.s,
        "create_circle",
        json!({"center": p(0.0, 60_000.0, 0.0), "radius": 100, "segments": 360}),
    );
}

/// Scaling must not shrink edges below 0.5 mm either (D-06 rule 3).
#[test]
fn scaling_never_creates_edges_below_short_curve_tolerance() {
    let mut f = Fx::new();
    // 2000 mm × 1e-4 = 0.2 mm edges.
    expect_code(
        &mut f.s,
        "scale_elements",
        json!({"elements": [f.bx], "center": p(10_000.0, 0.0, 0.0), "factor": 1e-4}),
        "ferro.geom.short_curve",
    );
    expect_ok(
        &mut f.s,
        "scale_elements",
        json!({"elements": [f.bx], "center": p(10_000.0, 0.0, 0.0), "factor": 0.001}),
    );
}

/// An offset past the middle of a face mirrors the outline through its
/// centre (same orientation, every edge reversed): it is too large, not an
/// imprint that misses the face.
#[test]
fn offset_past_the_middle_is_too_large() {
    let mut f = Fx::new();
    let top = f.top(f.bx);
    for d in [1500.0, 5000.0] {
        expect_code(
            &mut f.s,
            "offset_face",
            json!({"element": f.bx, "face": top, "distance": d}),
            "ferro.geom.offset_too_large",
        );
    }
}

/// Offsetting to within less than 0.5 mm of the opposite side would leave a
/// sliver face with short edges.
#[test]
fn offset_leaving_a_sliver_is_rejected() {
    let mut f = Fx::new();
    let top = f.top(f.bx);
    // 2000 wide: an inset of 999.9 leaves a 0.2 mm square.
    expect_code(
        &mut f.s,
        "offset_face",
        json!({"element": f.bx, "face": top, "distance": 999.9}),
        "ferro.geom.short_curve",
    );
}

/// Imprinting near existing vertices: within VERTEX_TOL a point is the
/// vertex; 0.02 mm away it is a distinct point and the 0.02 mm edge is
/// rejected.
#[test]
fn imprint_points_snap_within_vertex_tolerance() {
    let mut f = Fx::new();
    let bx = f.bx;
    let top = f.top(bx);
    // A line across the top from corner (10000,0) to the far edge, the
    // start 0.005 mm off the corner: it must be treated as the corner.
    let o = expect_ok(
        &mut f.s,
        "imprint_face",
        json!({"element": bx, "face": top, "points": [p(10_000.005, 0.0, 2000.0), p(12_000.0, 2000.0, 2000.0)]}),
    );
    let b = geometry(&f.s, bx);
    assert_eq!(b.vertex_count(), 8, "{}", o.output);
    assert_eq!(b.face_count(), 7);
    // 0.02 mm from a corner: an edge of 0.02 mm would appear.
    let mut f = Fx::new();
    let top = f.top(f.bx);
    let r = checked(
        &mut f.s,
        "imprint_face",
        json!({"element": f.bx, "face": top, "points": [p(10_000.02, 0.0, 2000.0), p(12_000.0, 2000.0, 2000.0)]}),
    );
    assert!(r.is_err(), "a 0.02 mm edge was created");
}

// ------------------------------------------------------- 1c. JSON parsing

/// Missing required arguments, wrong types or non-object arguments give a
/// clean error from `from_tool`, never a panic.
#[test]
fn bad_tool_arguments_are_clean_errors() {
    let bad: Vec<(&str, Value)> = vec![
        ("create_level", json!({"elevation": 0})),
        ("create_level", json!({"name": "L", "elevation": "3m"})),
        ("create_level", json!({"name": 5, "elevation": 0})),
        (
            "create_rectangle",
            json!({"origin": p(0.0, 0.0, 0.0), "width": null, "depth": 1}),
        ),
        (
            "create_box",
            json!({"origin": p(0.0, 0.0, 0.0), "width": 1, "depth": 1}),
        ),
        (
            "push_pull_face",
            json!({"element": -1, "face": 1, "distance": 10}),
        ),
        (
            "push_pull_face",
            json!({"element": 1.5, "face": 1, "distance": 10}),
        ),
        (
            "push_pull_face",
            json!({"element": 1, "face": 1, "distance": true}),
        ),
        (
            "move_elements",
            json!({"elements": 1, "vector": p(0.0, 0.0, 0.0)}),
        ),
        (
            "move_elements",
            json!({"elements": [1], "vector": {"x": "a"}}),
        ),
        (
            "rotate_elements",
            json!({"elements": [1], "center": p(0.0, 0.0, 0.0)}),
        ),
        (
            "copy_elements",
            json!({"elements": [1], "vector": p(0.0, 0.0, 0.0), "count": -1}),
        ),
        (
            "copy_elements",
            json!({"elements": [1], "vector": p(0.0, 0.0, 0.0), "count": 1e20}),
        ),
        (
            "set_parameter",
            json!({"element": 1, "key": "a.b.c", "value": 3}),
        ),
        (
            "set_parameter",
            json!({"element": 1, "key": "a.b.c", "value": {"type": "colour", "value": 3}}),
        ),
        (
            "set_category",
            json!({"element": 1, "category": "spaceships"}),
        ),
        (
            "set_geometry_source",
            json!({"element": 1, "source": "magic"}),
        ),
        (
            "create_wall_type",
            json!({"name": "T", "layers": [{"function": "glue", "thickness": 10}]}),
        ),
        (
            "create_wall_type",
            json!({"name": "T", "layers": [{"thickness": 10}]}),
        ),
        ("create_wall", json!({"start": p(0.0, 0.0, 0.0)})),
        (
            "create_wall",
            json!({"start": p(0.0, 0.0, 0.0), "end": p(1.0, 0.0, 0.0), "location_line": "middle"}),
        ),
        ("create_floor", json!({"points": "square"})),
        (
            "create_opening",
            json!({"wall": 1, "kind": "hatch", "position": 1}),
        ),
        ("create_opening", json!({"wall": 1, "position": 1})),
        (
            "scale_elements",
            json!({"elements": [1], "center": p(0.0, 0.0, 0.0), "factors": [1, 2]}),
        ),
        ("erase_edges", json!({"element": 1})),
        (
            "imprint_face",
            json!({"element": 1, "face": 1, "points": [1, 2]}),
        ),
        ("set_project_name", json!({})),
        ("set_project_name", json!(["name"])),
        // `name` is required (null clears); a missing one used to clear the
        // name silently (M5-15.12).
        ("set_name", json!({"element": 1})),
        ("set_name", json!({"element": 1, "name": 5})),
        ("create_box", json!(null)),
        ("create_box", json!("box")),
    ];
    for (tool, args) in bad {
        assert!(
            AnyCommand::from_tool(tool, &args).is_err(),
            "{tool} {args} was accepted"
        );
    }
    assert!(AnyCommand::from_tool("", &json!({})).is_err());
    assert!(AnyCommand::from_tool("CREATE_BOX", &json!({})).is_err());
    // An argument named "command" cannot redirect to another command.
    let c = AnyCommand::from_tool(
        "set_project_name",
        &json!({"name": "x", "command": "delete_elements"}),
    )
    .expect("command key is overwritten by the tool name");
    assert_eq!(c.name(), "set_project_name");
}

/// Every tool given empty or junk arguments: no panic, either a clean
/// argument error or a clean command failure.
#[test]
fn every_tool_survives_empty_and_junk_arguments() {
    let junk = [
        json!({}),
        json!(null),
        json!([]),
        json!({"element": 9999, "elements": [9999], "face": 9999, "wall": 9999, "floor": 9999, "opening": 9999, "level": 9999}),
    ];
    for name in AnyCommand::NAMES {
        for args in &junk {
            if let Ok(cmd) = AnyCommand::from_tool(name, args) {
                let mut f = Fx::new();
                let before = snapshot(&f.s);
                if let Err(fails) = f.s.execute(cmd, &ExecContext::user()) {
                    assert!(fails.iter().all(|x| x.code.starts_with("ferro.")));
                    assert_eq!(snapshot(&f.s), before);
                }
            }
        }
    }
}

// ----------------------------------------------- 1d. wrong element kinds

/// One valid call per command (built on the fixture).
fn valid_calls(f: &Fx) -> Vec<(&'static str, Value)> {
    let bx_top = f.top(f.bx);
    let wall_top = f.top(f.wall);
    let bx_edge = geometry(&f.s, f.bx).edges().next().expect("edge").0;
    vec![
        (
            "create_level",
            json!({"name": "L3", "elevation": 6000, "is_building_storey": false}),
        ),
        (
            "set_level_elevation",
            json!({"level": f.l2, "elevation": 3300}),
        ),
        (
            "create_rectangle",
            json!({"origin": p(0.0, -5000.0, 0.0), "width": 1000, "depth": -2000, "level": f.l2, "name": "R"}),
        ),
        (
            "create_polygon",
            json!({"points": [p(0.0,-9000.0,100.0), p(1000.0,-9000.0,100.0), p(500.0,-8000.0,100.0)], "level": f.l2, "relative_to_level": true}),
        ),
        (
            "create_box",
            json!({"origin": p(0.0, -12_000.0, 0.0), "width": -1000, "depth": 1000, "height": 500, "level": f.l1}),
        ),
        (
            "push_pull_face",
            json!({"element": f.bx, "face": bx_top, "distance": 500}),
        ),
        (
            "move_elements",
            json!({"elements": [f.bx, f.wall, f.floor, f.door], "vector": p(100.0, 200.0, 0.0)}),
        ),
        (
            "rotate_elements",
            json!({"elements": [f.bx, f.wall, f.door, f.window], "center": p(0.0, 0.0, 0.0), "angle": 0.3}),
        ),
        (
            "copy_elements",
            json!({"elements": [f.wall, f.bx], "vector": p(0.0, -20_000.0, 0.0), "count": 2}),
        ),
        ("delete_elements", json!({"elements": [f.l2, f.wall]})),
        ("set_name", json!({"element": f.wall, "name": null})),
        (
            "set_parameter",
            json!({"element": f.wall, "key": "acme.notes.ref", "value": {"type": "element_ref", "value": f.floor}}),
        ),
        (
            "set_category",
            json!({"element": f.bx, "category": "walls"}),
        ),
        ("set_project_name", json!({"name": "Renamed"})),
        (
            "create_material",
            json!({"name": "Brick", "color": "a0522d", "roughness": 7.0}),
        ),
        (
            "create_wall_type",
            json!({"name": "Int 100", "thickness": 100, "material": f.mat}),
        ),
        (
            "create_floor_type",
            json!({"name": "Deck", "layers": [{"function": "finish", "thickness": 20}, {"function": "structure", "thickness": 180}]}),
        ),
        (
            "create_wall",
            json!({"start": p(0.0, 20_000.0, 0.0), "end": p(0.0, 25_000.0, 0.0), "level": f.l1, "top_level": f.l2, "top_offset": -100, "base_offset": 50, "location_line": "finish_exterior"}),
        ),
        (
            "create_walls",
            json!({"points": [p(0.0,30_000.0,0.0), p(5000.0,30_000.0,0.0), p(5000.0,34_000.0,0.0)], "closed": true, "wall_type": f.wt}),
        ),
        (
            "create_arc_wall",
            json!({"start": p(0.0, 40_000.0, 0.0), "mid": p(2000.0, 42_000.0, 0.0), "end": p(4000.0, 40_000.0, 0.0), "height": 2500}),
        ),
        ("set_wall_height", json!({"wall": f.wall2, "height": 4000})),
        (
            "set_element_type",
            json!({"element": f.arc, "element_type": f.wt}),
        ),
        (
            "create_floor",
            json!({"points": [{"x":0,"y":-30_000},{"x":3000,"y":-30_000},{"x":0,"y":-27_000}], "offset": 300}),
        ),
        (
            "create_opening",
            json!({"wall": f.wall, "kind": "window", "position": 2900, "width": 600, "height": 600, "sill": 1200, "name": "W2"}),
        ),
        (
            "set_opening",
            json!({"opening": f.window, "width": 1000, "sill": 1000}),
        ),
        (
            "set_wall",
            json!({"wall": f.wall, "start": p(-500.0, 0.0, 0.0), "end": p(6000.0, 0.0, 0.0), "location_line": "core_centerline"}),
        ),
        (
            "set_floor",
            json!({"floor": f.floor, "holes": [], "offset": -50}),
        ),
        (
            "imprint_face",
            json!({"element": f.bx, "face": bx_top, "points": [p(10_500.0,500.0,2000.0), p(11_500.0,500.0,2000.0), p(11_500.0,1500.0,2000.0), p(10_500.0,1500.0,2000.0)], "closed": true}),
        ),
        (
            "create_circle",
            json!({"center": p(11_000.0, 1000.0, 2000.0), "radius": 400, "segments": 16, "element": f.bx, "face": bx_top}),
        ),
        (
            "offset_face",
            json!({"element": f.bx, "face": bx_top, "distance": 300}),
        ),
        (
            "move_geometry",
            json!({"element": f.bx, "faces": [bx_top], "vector": p(0.0, 0.0, 100.0)}),
        ),
        (
            "scale_elements",
            json!({"elements": [f.bx, f.sheet], "center": p(0.0, 0.0, 0.0), "factors": [1.5, 1.0, 0.5]}),
        ),
        ("erase_edges", json!({"element": f.bx, "edges": [bx_edge]})),
        (
            "erase_faces",
            json!({"element": f.sheet, "faces": [f.sheet_face]}),
        ),
        (
            "set_geometry_source",
            json!({"element": f.wall, "source": "direct"}),
        ),
        (
            "set_material",
            json!({"material": f.mat, "color": "#909090", "thermal": {"conductivity_w_mk": 1.8}}),
        ),
        (
            "set_parameter_definition",
            json!({"key": "acme.fire.rating", "data_type": "text", "description": "Fire rating"}),
        ),
        (
            "set_project_units",
            json!({"spec": "length", "unit": "m", "decimals": 2}),
        ),
        (
            "set_site_location",
            json!({"latitude": 0.28, "longitude": 1.89, "utc_offset_minutes": 420}),
        ),
        ("set_project_currency", json!({"currency": "VND"})),
        // phases (M5-23)
        (
            "create_phase",
            json!({"name": "Stage 2", "before": f.ph_new, "start_date": "2027-01-01"}),
        ),
        (
            "set_phase",
            json!({"phase": f.ph_new, "name": "Works", "position": 0, "end_date": "2027-06-30"}),
        ),
        ("delete_phase", json!({"phase": f.ph_new})),
        (
            "set_element_phase",
            json!({"elements": [f.wall], "created": f.ph_old, "demolished": f.ph_new}),
        ),
        (
            "set_renovation_status",
            json!({"elements": [f.bx, f.door], "status": "demolish"}),
        ),
        // relationships (M5-5)
        (
            "create_relationship",
            json!({"kind": "join", "elements": [f.wall2, f.bx], "phase": f.ph_new}),
        ),
        ("delete_relationship", json!({"relationship": f.rel})),
        // views (M5-22)
        (
            "create_view",
            json!({"name": "L1 plan", "view_type": "plan_view", "level": f.l1, "phase": f.ph_old}),
        ),
        (
            "set_view",
            json!({"view": f.view, "hide": [f.bx], "display_style": "wireframe", "phase": f.ph_new}),
        ),
        ("delete_view", json!({"view": f.view})),
        // a semantic push / pull on a wall (height)
        (
            "push_pull_face",
            json!({"element": f.wall, "face": wall_top, "distance": 200}),
        ),
    ]
}

#[test]
fn valid_calls_cover_every_command() {
    let f = Fx::new();
    let calls = valid_calls(&f);
    for name in AnyCommand::NAMES {
        assert!(
            calls.iter().any(|(t, _)| t == name),
            "{name} has no scenario"
        );
    }
}

/// Each element-id argument of each command replaced by every element of
/// the fixture (every kind) and by a missing id: either a valid commit or a
/// clean `ferro.*` failure with rollback. Never a panic.
#[test]
fn every_id_argument_accepts_any_element_kind_without_panicking() {
    let id_fields = [
        "element",
        "level",
        "wall",
        "opening",
        "floor",
        "wall_type",
        "floor_type",
        "top_level",
        "material",
        "element_type",
        "phase",
        "before",
        "merge_into",
        "created",
        "demolished",
        "view",
    ];
    let base = Fx::new();
    let all = base.all_ids();
    let calls = valid_calls(&base);
    let mut tried = 0;
    for (tool, args) in &calls {
        let obj = args.as_object().expect("object");
        for field in id_fields.iter().filter(|k| obj.contains_key(**k)) {
            for v in &all {
                let mut a = obj.clone();
                a.insert((*field).into(), json!(v));
                let mut f = Fx::new();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = checked(&mut f.s, tool, Value::Object(a.clone()));
                }));
                assert!(r.is_ok(), "{tool} with {field} = {v} panicked");
                tried += 1;
            }
        }
        if let Some(Value::Array(_)) = obj.get("elements") {
            for v in &all {
                let mut a = obj.clone();
                a.insert("elements".into(), json!([v]));
                let mut f = Fx::new();
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = checked(&mut f.s, tool, Value::Object(a.clone()));
                }));
                assert!(r.is_ok(), "{tool} with elements = [{v}] panicked");
                tried += 1;
            }
        }
    }
    assert!(tried > 300, "{tried}");
}

// ------------------------------------------------------- 2. undo / redo

/// Every command: execute, undo gives the exact previous document, redo the
/// exact after state; a second undo / redo cycle is stable.
#[test]
fn undo_and_redo_restore_exact_documents_for_every_command() {
    let base = Fx::new();
    for (tool, args) in valid_calls(&base) {
        let mut f = Fx::new();
        let before = snapshot(&f.s);
        let journal = f.s.journal().len();
        expect_ok(&mut f.s, tool, args.clone());
        let after = snapshot(&f.s);
        assert_ne!(before, after, "{tool} changed nothing");
        assert_eq!(f.s.journal().len(), journal + 1);
        assert_eq!(f.s.undo_name(), Some(tool));
        f.s.undo().expect("undo");
        assert_eq!(snapshot(&f.s), before, "{tool}: undo");
        f.s.redo().expect("redo");
        assert_eq!(snapshot(&f.s), after, "{tool}: redo");
        f.s.undo().expect("undo again");
        assert_eq!(snapshot(&f.s), before, "{tool}: second undo");
        f.s.redo().expect("redo again");
        assert_eq!(snapshot(&f.s), after, "{tool}: second redo");
        // A new command after undo clears redo.
        f.s.undo();
        run(&mut f.s, "set_project_name", json!({"name": "branch"}));
        assert!(!f.s.can_redo(), "{tool}: redo survived a new command");
    }
}

/// Undoing the whole fixture history returns the empty document; redoing it
/// all returns the same document.
#[test]
fn full_history_undo_redo_roundtrip() {
    let mut f = Fx::new();
    let full = snapshot(&f.s);
    let n = f.s.journal().len();
    for _ in 0..n {
        f.s.undo().expect("undo");
    }
    assert!(f.s.doc().is_empty());
    assert!(f.s.undo().is_none());
    for _ in 0..n {
        f.s.redo().expect("redo");
    }
    assert!(f.s.redo().is_none());
    assert_eq!(snapshot(&f.s), full);
}

/// A failed command in the middle of history does not disturb undo / redo.
#[test]
fn failures_do_not_touch_redo() {
    let mut f = Fx::new();
    run(&mut f.s, "set_project_name", json!({"name": "A"}));
    f.s.undo();
    assert!(f.s.can_redo());
    expect_code(
        &mut f.s,
        "set_wall_height",
        json!({"wall": f.wall, "height": 0}),
        "ferro.wall.no_height",
    );
    assert!(f.s.can_redo(), "a failed command cleared redo");
    assert_eq!(f.s.redo_name(), Some("set_project_name"));
}

// ------------------------------------------------------- semantics

/// Moving the fixture by a vector and back gives the same geometry within
/// tolerance; rotating 4 × 90° does too.
#[test]
fn move_and_rotate_are_reversible() {
    let mut f = Fx::new();
    let els = json!([f.bx, f.wall, f.wall2, f.floor, f.arc, f.sheet]);
    let before = geometry(&f.s, f.bx).bounding_box().expect("bbox");
    expect_ok(
        &mut f.s,
        "move_elements",
        json!({"elements": els, "vector": p(1234.5, -987.25, 0.0)}),
    );
    expect_ok(
        &mut f.s,
        "move_elements",
        json!({"elements": els, "vector": p(-1234.5, 987.25, 0.0)}),
    );
    let back = geometry(&f.s, f.bx).bounding_box().expect("bbox");
    assert!(before.min.is_almost_equal(back.min) && before.max.is_almost_equal(back.max));
    let vol = geometry(&f.s, f.wall).volume();
    for _ in 0..4 {
        expect_ok(
            &mut f.s,
            "rotate_elements",
            json!({"elements": els, "center": p(3000.0, 2000.0, 0.0), "angle": std::f64::consts::FRAC_PI_2}),
        );
    }
    let back = geometry(&f.s, f.bx).bounding_box().expect("bbox");
    assert!(
        before.min.distance_to(back.min) < 1e-6 && before.max.distance_to(back.max) < 1e-6,
        "{before:?} vs {back:?}"
    );
    assert!((geometry(&f.s, f.wall).volume() - vol).abs() < 1.0);
}

/// Frozen (direct) wall: Form commands work on it, parametric ones refuse.
#[test]
fn frozen_wall_takes_form_commands_and_refuses_wall_commands() {
    let mut f = Fx::new();
    expect_ok(
        &mut f.s,
        "set_geometry_source",
        json!({"element": f.wall, "source": "direct"}),
    );
    let e = f.s.doc().get(f.wall).expect("wall");
    assert_eq!(e.geometry_source(), Some(GeometrySource::Direct));
    expect_code(
        &mut f.s,
        "set_wall_height",
        json!({"wall": f.wall, "height": 1000}),
        "ferro.model.not_a_wall",
    );
    expect_code(
        &mut f.s,
        "create_opening",
        json!({"wall": f.wall, "kind": "door", "position": 3000}),
        "ferro.model.not_a_wall",
    );
    let top = f.top(f.wall);
    expect_ok(
        &mut f.s,
        "push_pull_face",
        json!({"element": f.wall, "face": top, "distance": 300}),
    );
    expect_ok(
        &mut f.s,
        "scale_elements",
        json!({"elements": [f.wall], "center": p(0.0, 0.0, 0.0), "factor": 1.1}),
    );
    // Deleting the frozen wall's type is still refused.
    expect_code(
        &mut f.s,
        "delete_elements",
        json!({"elements": [f.wt]}),
        "ferro.model.type_in_use",
    );
}

/// Deleting levels keeps walls and floors where they were.
#[test]
fn deleting_levels_keeps_geometry_in_place() {
    let mut f = Fx::new();
    run(
        &mut f.s,
        "set_level_elevation",
        json!({"level": f.l1, "elevation": 500}),
    );
    let w2 = geometry(&f.s, f.wall2).bounding_box().expect("bbox");
    let fl = geometry(&f.s, f.floor).bounding_box().expect("bbox");
    let o = expect_ok(
        &mut f.s,
        "delete_elements",
        json!({"elements": [f.l1, f.l2]}),
    );
    assert!(o
        .warnings
        .iter()
        .all(|w| w.code == "ferro.model.reference_cleared"));
    let w2b = geometry(&f.s, f.wall2).bounding_box().expect("bbox");
    let flb = geometry(&f.s, f.floor).bounding_box().expect("bbox");
    assert!(w2.min.is_almost_equal(w2b.min) && w2.max.is_almost_equal(w2b.max));
    assert!(fl.min.is_almost_equal(flb.min) && fl.max.is_almost_equal(flb.max));
}

/// `preview` gives the same document `execute` commits, without changing
/// the session.
#[test]
fn preview_matches_execute() {
    let base = Fx::new();
    for (tool, args) in valid_calls(&base) {
        let mut f = Fx::new();
        let cmd = AnyCommand::from_tool(tool, &args).expect("args");
        let before = snapshot(&f.s);
        let (doc, _) = f.s.preview(&cmd).expect("preview");
        assert_eq!(
            snapshot(&f.s),
            before,
            "{tool}: preview changed the session"
        );
        f.s.execute(cmd, &ExecContext::user()).expect("execute");
        // Uids differ (preview uses its own id source); compare the shape.
        let strip = |d: &crate::model::Document| {
            d.elements()
                .map(|e| (e.id, e.category, e.name.clone(), e.geometry().cloned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(&doc), strip(f.s.doc()), "{tool}");
    }
}

/// A self-intersecting (bow-tie) outline is not a valid face or floor
/// (M5-15.10).
#[test]
fn self_intersecting_outlines_are_rejected() {
    let mut f = Fx::new();
    let bow = [(0.0, 0.0), (3000.0, 1000.0), (3000.0, 0.0), (0.0, 2000.0)];
    expect_code(
        &mut f.s,
        "create_polygon",
        json!({"points": bow.iter().map(|(x, y)| p(*x, *y + 70_000.0, 0.0)).collect::<Vec<_>>()}),
        "ferro.geom.self_intersecting_profile",
    );
    // Vertical bow-tie (any plane) and a figure eight through a repeated point.
    expect_code(
        &mut f.s,
        "create_polygon",
        json!({"points": bow.iter().map(|(x, z)| p(*x, 70_000.0, *z)).collect::<Vec<_>>()}),
        "ferro.geom.self_intersecting_profile",
    );
    let eight = [
        (0.0, 0.0),
        (1000.0, 0.0),
        (1000.0, 1000.0),
        (2000.0, 1000.0),
        (2000.0, 2000.0),
        (1000.0, 2000.0),
        (1000.0, 1000.0),
        (0.0, 1000.0),
    ];
    expect_code(
        &mut f.s,
        "create_polygon",
        json!({"points": eight.iter().map(|(x, y)| p(*x, *y + 70_000.0, 0.0)).collect::<Vec<_>>()}),
        "ferro.geom.self_intersecting_profile",
    );
    // Tolerance edge: a notch whose tip stops 1 mm short of the opposite
    // edge is simple; reaching it is not.
    let notch = |tip: f64| {
        [
            (0.0, 0.0),
            (3000.0, 0.0),
            (3000.0, 2000.0),
            (1600.0, 2000.0),
            (1500.0, tip),
            (1400.0, 2000.0),
            (0.0, 2000.0),
        ]
        .iter()
        .map(|(x, y)| p(*x, *y + 80_000.0, 0.0))
        .collect::<Vec<_>>()
    };
    expect_ok(&mut f.s, "create_polygon", json!({ "points": notch(1.0) }));
    expect_code(
        &mut f.s,
        "create_polygon",
        json!({ "points": notch(0.0) }),
        "ferro.geom.self_intersecting_profile",
    );
}

/// A self-intersecting (bow-tie) floor outline is rejected (M5-15.10), and so
/// are holes outside the outline, touching it, or overlapping each other.
#[test]
fn self_intersecting_floor_outlines_are_rejected() {
    let mut f = Fx::new();
    let xy = |pts: &[(f64, f64)]| {
        pts.iter()
            .map(|(x, y)| json!({"x": x, "y": y + 70_000.0}))
            .collect::<Vec<_>>()
    };
    let bow = [(0.0, 0.0), (3000.0, 1000.0), (3000.0, 0.0), (0.0, 2000.0)];
    expect_code(
        &mut f.s,
        "create_floor",
        json!({ "points": xy(&bow) }),
        "ferro.geom.self_intersecting_profile",
    );
    let sq = |x: f64, y: f64, s: f64| xy(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)]);
    let outer = sq(0.0, 0.0, 6000.0);
    for holes in [
        json!([sq(7000.0, 0.0, 500.0)]), // outside
        json!([sq(0.0, 1000.0, 500.0)]), // touching the outline
        json!([sq(1000.0, 1000.0, 1000.0), sq(1500.0, 1500.0, 1000.0)]), // overlapping
        json!([xy(&[
            (1000.0, 1000.0),
            (2000.0, 1300.0),
            (2000.0, 1000.0),
            (1000.0, 1600.0)
        ])]), // bow-tie hole
    ] {
        assert!(
            checked(
                &mut f.s,
                "create_floor",
                json!({"points": outer, "holes": holes})
            )
            .is_err(),
            "{holes}"
        );
    }
    expect_ok(
        &mut f.s,
        "create_floor",
        json!({"points": outer, "holes": [sq(1000.0, 1000.0, 1000.0), sq(3000.0, 3000.0, 1000.0)]}),
    );
}

/// Collinear outlines give the same code from `create_polygon` and
/// `create_floor` (M5-15.12).
#[test]
fn collinear_outlines_fail_with_the_same_code() {
    let mut f = Fx::new();
    let line = [(0.0, 0.0), (1000.0, 0.0), (2500.0, 0.0)];
    expect_code(
        &mut f.s,
        "create_polygon",
        json!({"points": line.iter().map(|(x, y)| p(*x, *y + 70_000.0, 0.0)).collect::<Vec<_>>()}),
        "ferro.geom.degenerate",
    );
    expect_code(
        &mut f.s,
        "create_floor",
        json!({"points": line.iter().map(|(x, y)| json!({"x": x, "y": y + 70_000.0})).collect::<Vec<_>>()}),
        "ferro.geom.degenerate",
    );
}

/// A closed imprint curve that crosses itself (bow-tie) is refused with the
/// self-intersection code; a closed curve repeating its first point is still
/// fine (M5-15.10).
#[test]
fn imprinting_a_bow_tie_is_refused() {
    let mut f = Fx::new();
    let e = boxed(&mut f.s, [0.0, 70_000.0, 0.0], 4000.0, 3000.0, 1000.0);
    let top = face_towards(&geometry(&f.s, e), Vec3::Z);
    expect_code(
        &mut f.s,
        "imprint_face",
        json!({"element": e, "face": top, "closed": true, "points": [
            p(1000.0, 71_000.0, 1000.0), p(3000.0, 72_000.0, 1000.0),
            p(3000.0, 71_000.0, 1000.0), p(1000.0, 72_000.0, 1000.0)]}),
        "ferro.geom.self_intersecting_profile",
    );
    expect_ok(
        &mut f.s,
        "imprint_face",
        json!({"element": e, "face": top, "closed": true, "points": [
            p(1000.0, 71_000.0, 1000.0), p(3000.0, 71_000.0, 1000.0),
            p(3000.0, 72_000.0, 1000.0), p(1000.0, 72_000.0, 1000.0),
            p(1000.0, 71_000.0, 1000.0)]}),
    );
}
