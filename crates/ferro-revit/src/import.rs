//! Revit → Ferro: apply a snapshot of the Revit model to a session as
//! ordinary commands (so it is undoable, journaled, and can be a proposal).

use crate::mm;
use ferro_core::model::bim::{OpeningKind, WallTop};
use ferro_core::model::generate;
use ferro_core::model::{Document, ElementBody, ElementId};
use ferro_core::ops::{AnyCommand, ExecContext};
use ferro_core::{Point2, Session};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Lengths in the snapshot are Revit feet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RevitSnapshot {
    /// Ferro uids the Revit model held after the last sync. Those missing
    /// from the snapshot were deleted in Revit.
    #[serde(default)]
    pub known_ferro_uids: Vec<String>,
    #[serde(default)]
    pub levels: Vec<RevitLevel>,
    #[serde(default)]
    pub wall_types: Vec<RevitType>,
    #[serde(default)]
    pub floor_types: Vec<RevitType>,
    #[serde(default)]
    pub walls: Vec<RevitWall>,
    #[serde(default)]
    pub floors: Vec<RevitFloor>,
    #[serde(default)]
    pub openings: Vec<RevitOpening>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitLevel {
    pub unique_id: String,
    #[serde(default)]
    pub ferro_uid: Option<String>,
    pub name: String,
    pub elevation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitLayer {
    /// `MaterialFunctionAssignment` name.
    pub function: String,
    pub width: f64,
    #[serde(default)]
    pub material_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitType {
    pub unique_id: String,
    #[serde(default)]
    pub ferro_uid: Option<String>,
    pub name: String,
    #[serde(default)]
    pub layers: Vec<RevitLayer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitCurve {
    pub start: [f64; 3],
    pub end: [f64; 3],
    #[serde(default)]
    pub mid: Option<[f64; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitWall {
    pub unique_id: String,
    #[serde(default)]
    pub ferro_uid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub type_unique_id: String,
    pub level_unique_id: String,
    pub curve: RevitCurve,
    #[serde(default)]
    pub base_offset: f64,
    /// Unconnected height (used when `top_level_unique_id` is absent).
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub top_level_unique_id: Option<String>,
    #[serde(default)]
    pub top_offset: f64,
    #[serde(default)]
    pub location_line: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitFloor {
    pub unique_id: String,
    #[serde(default)]
    pub ferro_uid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub type_unique_id: String,
    pub level_unique_id: String,
    /// Boundary loops in plan (z ignored), any order and winding.
    pub loops: Vec<Vec<[f64; 3]>>,
    #[serde(default)]
    pub offset: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevitOpening {
    pub unique_id: String,
    #[serde(default)]
    pub ferro_uid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// `door` or `window`.
    pub kind: String,
    pub host_unique_id: String,
    /// Insertion point (on the host's location line).
    pub point: [f64; 3],
    pub width: f64,
    pub height: f64,
    /// Above the level.
    #[serde(default)]
    pub sill: f64,
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    /// (Revit unique id, new Ferro uid) for elements created in Ferro; the
    /// add-in stores the uid on the Revit element.
    pub created: Vec<(String, String)>,
    pub updated: usize,
    pub deleted: usize,
    /// Problems that skipped one element (the rest still imported).
    pub warnings: Vec<String>,
}

/// Two lengths in mm differ by more than half a millimetre.
fn differs(a: f64, b: f64) -> bool {
    (a - b).abs() > 0.5
}

fn pt(p: [f64; 3]) -> Value {
    json!({"x": mm(p[0]), "y": mm(p[1]), "z": 0.0})
}

fn pt2(p: &[f64; 3]) -> Point2 {
    Point2::new(mm(p[0]), mm(p[1]))
}

fn function(name: &str) -> &'static str {
    match name {
        "Structure" => "structure",
        "Substrate" => "substrate",
        "Insulation" => "insulation",
        "Membrane" => "membrane",
        _ => "finish",
    }
}

fn location_line(v: i32) -> &'static str {
    match v {
        1 | 4 | 5 => "core_centerline",
        2 => "finish_exterior",
        3 => "finish_interior",
        _ => "wall_centerline",
    }
}

fn signed_area(pts: &[Point2]) -> f64 {
    let mut a = 0.0;
    for (i, p) in pts.iter().enumerate() {
        let q = pts[(i + 1) % pts.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

struct Importer<'a> {
    session: &'a mut Session,
    ctx: &'a ExecContext,
    report: ImportReport,
    /// Revit unique id → Ferro element.
    ids: HashMap<String, ElementId>,
}

impl Importer<'_> {
    fn doc(&self) -> &Document {
        self.session.doc()
    }

    fn by_uid(&self, uid: &Option<String>) -> Option<ElementId> {
        let uid = uid.as_ref()?;
        self.doc()
            .elements()
            .find(|e| e.uid.to_string() == *uid)
            .map(|e| e.id)
    }

    fn run(&mut self, tool: &str, args: Value) -> Option<Value> {
        let cmd = match AnyCommand::from_tool(tool, &args) {
            Ok(c) => c,
            Err(m) => {
                self.report.warnings.push(m);
                return None;
            }
        };
        match self.session.execute(cmd, self.ctx) {
            Ok(o) => Some(o.output),
            Err(f) => {
                let msg = f.first().map(|x| x.message.clone()).unwrap_or_default();
                self.report.warnings.push(format!("{tool}: {msg}"));
                None
            }
        }
    }

    /// Run a creating command and record the new element.
    fn create(&mut self, unique_id: &str, tool: &str, args: Value) -> Option<ElementId> {
        let out = self.run(tool, args)?;
        let id: ElementId = serde_json::from_value(out["element"].clone()).ok()?;
        let uid = self.doc().get(id)?.uid.to_string();
        self.report.created.push((unique_id.to_string(), uid));
        self.ids.insert(unique_id.to_string(), id);
        Some(id)
    }

    fn update(&mut self, tool: &str, args: Value) {
        if self.run(tool, args).is_some() {
            self.report.updated += 1;
        }
    }

    fn levels(&mut self, levels: &[RevitLevel]) {
        for l in levels {
            let elev = mm(l.elevation);
            let existing = self.by_uid(&l.ferro_uid).or_else(|| {
                // Same name already in Ferro: adopt it instead of duplicating.
                self.doc()
                    .levels()
                    .into_iter()
                    .find(|e| e.name.as_deref() == Some(l.name.as_str()))
                    .map(|e| e.id)
            });
            match existing {
                Some(id) => {
                    self.ids.insert(l.unique_id.clone(), id);
                    let Some(e) = self.doc().get(id) else {
                        continue;
                    };
                    let cur = e.as_level().map(|x| x.elevation.mm()).unwrap_or(0.0);
                    let name_differs = e.name.as_deref() != Some(l.name.as_str());
                    if differs(cur, elev) {
                        self.update(
                            "set_level_elevation",
                            json!({"level": id, "elevation": elev}),
                        );
                    }
                    if name_differs {
                        self.update("set_name", json!({"element": id, "name": l.name}));
                    }
                    if l.ferro_uid.is_none() {
                        let uid = self
                            .doc()
                            .get(id)
                            .map(|e| e.uid.to_string())
                            .unwrap_or_default();
                        self.report.created.push((l.unique_id.clone(), uid));
                    }
                }
                None => {
                    self.create(
                        &l.unique_id,
                        "create_level",
                        json!({"name": l.name, "elevation": elev}),
                    );
                }
            }
        }
    }

    fn material(&mut self, name: &str) -> Option<ElementId> {
        let found = self
            .doc()
            .elements()
            .find(|e| matches!(e.body, ElementBody::Material(_)) && e.name.as_deref() == Some(name))
            .map(|e| e.id);
        if found.is_some() {
            return found;
        }
        let out = self.run("create_material", json!({"name": name, "color": "#9a9a9a"}))?;
        serde_json::from_value(out["element"].clone()).ok()
    }

    fn types(&mut self, types: &[RevitType], floor: bool) {
        let tool = if floor {
            "create_floor_type"
        } else {
            "create_wall_type"
        };
        for t in types {
            let is_kind = |b: &ElementBody| {
                if floor {
                    matches!(b, ElementBody::FloorType(_))
                } else {
                    matches!(b, ElementBody::WallType(_))
                }
            };
            let existing = self.by_uid(&t.ferro_uid).or_else(|| {
                self.doc()
                    .elements()
                    .find(|e| is_kind(&e.body) && e.name.as_deref() == Some(t.name.as_str()))
                    .map(|e| e.id)
            });
            if let Some(id) = existing {
                self.ids.insert(t.unique_id.clone(), id);
                if t.ferro_uid.is_none() {
                    let uid = self
                        .doc()
                        .get(id)
                        .map(|e| e.uid.to_string())
                        .unwrap_or_default();
                    self.report.created.push((t.unique_id.clone(), uid));
                }
                continue;
            }
            let mut layers = Vec::new();
            for l in &t.layers {
                let material = l.material_name.as_deref().and_then(|n| self.material(n));
                layers.push(json!({"function": function(&l.function), "thickness": mm(l.width), "material": material}));
            }
            if layers
                .iter()
                .all(|l| l["thickness"].as_f64().unwrap_or(0.0) <= 0.0)
            {
                self.report
                    .warnings
                    .push(format!("type {:?} has no thickness", t.name));
                continue;
            }
            self.create(
                &t.unique_id,
                tool,
                json!({"name": t.name, "layers": layers}),
            );
        }
    }

    fn walls(&mut self, walls: &[RevitWall]) {
        for w in walls {
            let (Some(&level), Some(&ty)) = (
                self.ids.get(&w.level_unique_id),
                self.ids.get(&w.type_unique_id),
            ) else {
                self.report
                    .warnings
                    .push(format!("wall {}: unknown level or type", w.unique_id));
                continue;
            };
            let top_level = w
                .top_level_unique_id
                .as_ref()
                .and_then(|u| self.ids.get(u).copied());
            let mut args = json!({
                "start": pt(w.curve.start), "end": pt(w.curve.end),
                "base_offset": mm(w.base_offset),
                "location_line": location_line(w.location_line),
            });
            if let Some(m) = w.curve.mid {
                args["mid"] = pt(m);
            }
            match top_level {
                Some(l) => {
                    args["top_level"] = json!(l);
                    args["top_offset"] = json!(mm(w.top_offset));
                }
                None => args["height"] = json!(mm(w.height)),
            }
            match self.by_uid(&w.ferro_uid) {
                Some(id) => {
                    self.ids.insert(w.unique_id.clone(), id);
                    if self.wall_changed(id, w, top_level) {
                        args["wall"] = json!(id);
                        self.update("set_wall", args);
                    }
                    if self.doc().get(id).and_then(|e| e.type_id) != Some(ty) {
                        self.update(
                            "set_element_type",
                            json!({"element": id, "element_type": ty}),
                        );
                    }
                }
                None => {
                    args["level"] = json!(level);
                    args["wall_type"] = json!(ty);
                    args["name"] = json!(w.name);
                    let tool = if w.curve.mid.is_some() {
                        "create_arc_wall"
                    } else {
                        "create_wall"
                    };
                    self.create(&w.unique_id, tool, args);
                }
            }
        }
    }

    fn wall_changed(&self, id: ElementId, w: &RevitWall, top_level: Option<ElementId>) -> bool {
        let Some(e) = self.doc().get(id) else {
            return true;
        };
        let Some(cur) = e.as_wall() else { return true };
        let (s, en) = (cur.curve.start(), cur.curve.end());
        let moved = differs(s.x, mm(w.curve.start[0]))
            || differs(s.y, mm(w.curve.start[1]))
            || differs(en.x, mm(w.curve.end[0]))
            || differs(en.y, mm(w.curve.end[1]));
        let top = match (&cur.top, top_level) {
            (WallTop::Level { level, offset }, Some(l)) => {
                *level != l || differs(offset.mm(), mm(w.top_offset))
            }
            (WallTop::Unconnected { height }, None) => differs(height.mm(), mm(w.height)),
            _ => true,
        };
        moved
            || top
            || differs(cur.base_offset.mm(), mm(w.base_offset))
            || location_line(w.location_line)
                != serde_json::to_value(cur.location_line)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
    }

    fn floors(&mut self, floors: &[RevitFloor]) {
        for f in floors {
            let (Some(&level), Some(&ty)) = (
                self.ids.get(&f.level_unique_id),
                self.ids.get(&f.type_unique_id),
            ) else {
                self.report
                    .warnings
                    .push(format!("floor {}: unknown level or type", f.unique_id));
                continue;
            };
            let mut loops: Vec<Vec<Point2>> = f
                .loops
                .iter()
                .map(|l| l.iter().map(pt2).collect())
                .collect();
            loops.retain(|l| l.len() >= 3);
            if loops.is_empty() {
                continue;
            }
            // The largest loop is the outline; the others are holes.
            loops.sort_by(|a, b| signed_area(b).abs().total_cmp(&signed_area(a).abs()));
            let outer = loops.remove(0);
            let offset = mm(f.offset);
            match self.by_uid(&f.ferro_uid) {
                Some(id) => {
                    self.ids.insert(f.unique_id.clone(), id);
                    let changed = match self.doc().get(id).map(|e| &e.body) {
                        Some(ElementBody::Floor(cur)) => {
                            let same_ring = |a: &[Point2], b: &[Point2]| {
                                a.len() == b.len()
                                    && a.iter().all(|p| {
                                        b.iter().any(|q| !differs(p.x, q.x) && !differs(p.y, q.y))
                                    })
                            };
                            !same_ring(&cur.outer, &outer)
                                || cur.holes.len() != loops.len()
                                || differs(cur.offset.mm(), offset)
                        }
                        _ => true,
                    };
                    if changed {
                        self.update(
                            "set_floor",
                            json!({"floor": id, "points": outer, "holes": loops, "offset": offset}),
                        );
                    }
                    if self.doc().get(id).and_then(|e| e.type_id) != Some(ty) {
                        self.update(
                            "set_element_type",
                            json!({"element": id, "element_type": ty}),
                        );
                    }
                }
                None => {
                    self.create(
                        &f.unique_id,
                        "create_floor",
                        json!({"points": outer, "holes": loops, "level": level, "offset": offset, "floor_type": ty, "name": f.name}),
                    );
                }
            }
        }
    }

    fn openings(&mut self, openings: &[RevitOpening]) {
        for o in openings {
            let Some(&host) = self.ids.get(&o.host_unique_id) else {
                self.report
                    .warnings
                    .push(format!("opening {}: host wall not in Ferro", o.unique_id));
                continue;
            };
            let doc = self.doc();
            let Some(he) = doc.get(host) else { continue };
            let Some(lay) = he
                .as_wall()
                .and_then(|w| generate::wall_layout(doc, he, w).ok())
            else {
                continue;
            };
            let Some((s, d, _)) = lay.frame() else {
                self.report
                    .warnings
                    .push(format!("opening {}: curved host", o.unique_id));
                continue;
            };
            let position = (pt2(&o.point) - s).dot(d);
            let host_level = generate::level_elevation(doc, he.level_id);
            // Revit sill is above the level; Ferro's is above the wall base.
            let sill = (mm(o.sill) + host_level - lay.z0).max(0.0);
            let kind = if o.kind == "window" {
                OpeningKind::Window
            } else {
                OpeningKind::Door
            };
            let (w, h) = (mm(o.width), mm(o.height));
            match self.by_uid(&o.ferro_uid) {
                Some(id) => {
                    self.ids.insert(o.unique_id.clone(), id);
                    let changed = match self.doc().get(id).map(|e| &e.body) {
                        Some(ElementBody::Opening(cur)) => {
                            differs(cur.position.mm(), position)
                                || differs(cur.width.mm(), w)
                                || differs(cur.height.mm(), h)
                                || differs(cur.sill.mm(), sill)
                        }
                        _ => true,
                    };
                    if changed {
                        self.update(
                            "set_opening",
                            json!({"opening": id, "position": position, "width": w, "height": h, "sill": sill}),
                        );
                    }
                }
                None => {
                    self.create(
                        &o.unique_id,
                        "create_opening",
                        json!({"wall": host, "kind": kind, "position": position, "width": w, "height": h, "sill": sill, "name": o.name}),
                    );
                }
            }
        }
    }

    fn deletions(&mut self, snap: &RevitSnapshot) {
        let present: HashSet<&str> = snap
            .levels
            .iter()
            .map(|x| &x.ferro_uid)
            .chain(snap.wall_types.iter().map(|x| &x.ferro_uid))
            .chain(snap.floor_types.iter().map(|x| &x.ferro_uid))
            .chain(snap.walls.iter().map(|x| &x.ferro_uid))
            .chain(snap.floors.iter().map(|x| &x.ferro_uid))
            .chain(snap.openings.iter().map(|x| &x.ferro_uid))
            .filter_map(|u| u.as_deref())
            .collect();
        let doc = self.doc();
        // Types and materials are not deleted from Revit's side: a type
        // missing from the snapshot is usually just unused there.
        let gone: Vec<ElementId> = snap
            .known_ferro_uids
            .iter()
            .filter(|u| !present.contains(u.as_str()))
            .filter_map(|u| doc.elements().find(|e| e.uid.to_string() == *u))
            .filter(|e| {
                matches!(
                    e.body,
                    ElementBody::Wall(_)
                        | ElementBody::Floor(_)
                        | ElementBody::Opening(_)
                        | ElementBody::Level(_)
                )
            })
            .map(|e| e.id)
            .collect();
        // Openings of deleted walls go with them.
        let gone: Vec<ElementId> = gone
            .iter()
            .copied()
            .filter(|id| {
                doc.get(*id)
                    .and_then(|e| e.host_id)
                    .is_none_or(|h| !gone.contains(&h))
            })
            .collect();
        if !gone.is_empty()
            && self
                .run("delete_elements", json!({"elements": gone}))
                .is_some()
        {
            self.report.deleted += gone.len();
        }
    }
}

/// Apply a Revit snapshot to the session. Each change is a normal command,
/// so wrap the call in a transaction group to make it one undo step.
pub fn import(session: &mut Session, ctx: &ExecContext, snap: &RevitSnapshot) -> ImportReport {
    let mut imp = Importer {
        session,
        ctx,
        report: ImportReport::default(),
        ids: HashMap::new(),
    };
    imp.levels(&snap.levels);
    imp.types(&snap.wall_types, false);
    imp.types(&snap.floor_types, true);
    imp.walls(&snap.walls);
    imp.floors(&snap.floors);
    imp.openings(&snap.openings);
    imp.deletions(snap);
    imp.report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{plan, RevitOp, RevitState};
    use ferro_core::ops::SequentialIds;

    fn empty() -> Session {
        Session::new(Document::new("From Revit"), Box::new(SequentialIds::new(7)))
    }

    fn revit_house() -> RevitSnapshot {
        let lvl = |u: &str, n: &str, e: f64| RevitLevel {
            unique_id: u.into(),
            ferro_uid: None,
            name: n.into(),
            elevation: e,
        };
        let wall = |u: &str, s: [f64; 3], e: [f64; 3]| RevitWall {
            unique_id: u.into(),
            ferro_uid: None,
            name: None,
            type_unique_id: "wt".into(),
            level_unique_id: "l1".into(),
            curve: RevitCurve {
                start: s,
                end: e,
                mid: None,
            },
            base_offset: 0.0,
            height: 0.0,
            top_level_unique_id: Some("l2".into()),
            top_offset: 0.0,
            location_line: 0,
        };
        RevitSnapshot {
            known_ferro_uids: vec![],
            levels: vec![lvl("l1", "Level 1", 0.0), lvl("l2", "Level 2", 10.0)],
            wall_types: vec![RevitType {
                unique_id: "wt".into(),
                ferro_uid: None,
                name: "Generic - 8\"".into(),
                layers: vec![RevitLayer {
                    function: "Structure".into(),
                    width: 8.0 / 12.0,
                    material_name: Some("Concrete".into()),
                }],
            }],
            floor_types: vec![RevitType {
                unique_id: "ft".into(),
                ferro_uid: None,
                name: "Generic 12\"".into(),
                layers: vec![RevitLayer {
                    function: "Structure".into(),
                    width: 1.0,
                    material_name: None,
                }],
            }],
            walls: vec![
                wall("w1", [0.0, 0.0, 0.0], [0.0, 20.0, 0.0]),
                wall("w2", [0.0, 20.0, 0.0], [30.0, 20.0, 0.0]),
            ],
            floors: vec![RevitFloor {
                unique_id: "f1".into(),
                ferro_uid: None,
                name: None,
                type_unique_id: "ft".into(),
                level_unique_id: "l1".into(),
                loops: vec![
                    vec![
                        [5.0, 5.0, 0.0],
                        [6.0, 5.0, 0.0],
                        [6.0, 6.0, 0.0],
                        [5.0, 6.0, 0.0],
                    ],
                    vec![
                        [0.0, 0.0, 0.0],
                        [30.0, 0.0, 0.0],
                        [30.0, 20.0, 0.0],
                        [0.0, 20.0, 0.0],
                    ],
                ],
                offset: 0.0,
            }],
            openings: vec![RevitOpening {
                unique_id: "d1".into(),
                ferro_uid: None,
                name: None,
                kind: "door".into(),
                host_unique_id: "w1".into(),
                point: [0.0, 5.0, 0.0],
                width: 3.0,
                height: 7.0,
                sill: 0.0,
            }],
        }
    }

    #[test]
    fn importing_a_revit_model_creates_ferro_elements() {
        let mut s = empty();
        let r = import(&mut s, &ExecContext::user(), &revit_house());
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // 2 levels, 2 types, 2 walls, 1 floor, 1 door (+1 material, not reported).
        assert_eq!(r.created.len(), 8, "{:?}", r.created);
        let doc = s.doc();
        let walls: Vec<_> = doc.elements().filter(|e| e.as_wall().is_some()).collect();
        assert_eq!(walls.len(), 2);
        let h = match walls[0].param(ferro_core::model::keys::HEIGHT) {
            Some(ferro_core::ParamValue::Length(l)) => l.mm(),
            _ => panic!(),
        };
        assert!((h - 3048.0).abs() < 1e-6);
        let door = doc
            .elements()
            .find(|e| matches!(e.body, ElementBody::Opening(_)))
            .unwrap();
        let ElementBody::Opening(o) = &door.body else {
            unreachable!()
        };
        assert!((o.position.mm() - 5.0 * 304.8).abs() < 1e-6);
        assert!((o.width.mm() - 914.4).abs() < 1e-6);
        let floor = doc
            .elements()
            .find(|e| matches!(e.body, ElementBody::Floor(_)))
            .unwrap();
        let ElementBody::Floor(f) = &floor.body else {
            unreachable!()
        };
        assert_eq!(f.holes.len(), 1, "the small loop is a hole");
    }

    #[test]
    fn reimport_updates_moves_and_deletes() {
        let mut s = empty();
        let mut snap = revit_house();
        let r = import(&mut s, &ExecContext::user(), &snap);
        // The add-in tags Revit elements with the returned uids.
        let uid_of: HashMap<String, String> = r.created.into_iter().collect();
        for l in &mut snap.levels {
            l.ferro_uid = uid_of.get(&l.unique_id).cloned();
        }
        for t in snap
            .wall_types
            .iter_mut()
            .chain(snap.floor_types.iter_mut())
        {
            t.ferro_uid = uid_of.get(&t.unique_id).cloned();
        }
        for w in &mut snap.walls {
            w.ferro_uid = uid_of.get(&w.unique_id).cloned();
        }
        for f in &mut snap.floors {
            f.ferro_uid = uid_of.get(&f.unique_id).cloned();
        }
        for o in &mut snap.openings {
            o.ferro_uid = uid_of.get(&o.unique_id).cloned();
        }
        snap.known_ferro_uids = uid_of.values().cloned().collect();

        // Nothing changed in Revit: nothing changes in Ferro.
        let before = s.doc().clone();
        let r = import(&mut s, &ExecContext::user(), &snap);
        assert_eq!((r.created.len(), r.updated, r.deleted), (0, 0, 0), "{r:?}");
        assert_eq!(s.doc().len(), before.len());

        // In Revit: raise level 2, move the door, delete the second wall.
        snap.levels[1].elevation = 12.0;
        snap.openings[0].point = [0.0, 8.0, 0.0];
        snap.walls.pop();
        let r = import(&mut s, &ExecContext::user(), &snap);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.updated, 2);
        assert_eq!(r.deleted, 1);
        assert_eq!(
            s.doc().elements().filter(|e| e.as_wall().is_some()).count(),
            1
        );
        let door = s
            .doc()
            .elements()
            .find(|e| matches!(e.body, ElementBody::Opening(_)))
            .unwrap();
        let ElementBody::Opening(o) = &door.body else {
            unreachable!()
        };
        assert!((o.position.mm() - 8.0 * 304.8).abs() < 1e-6);
    }

    #[test]
    fn plan_and_import_round_trip() {
        // Ferro → Revit plan → (pretend Revit) snapshot → import into an
        // empty Ferro gives the same walls.
        let src = crate::plan::tests_support::house();
        let ops = plan(src.doc(), &RevitState::default());
        let mut snap = RevitSnapshot::default();
        for op in &ops {
            match op {
                RevitOp::Level {
                    uid,
                    name,
                    elevation,
                    ..
                } => snap.levels.push(RevitLevel {
                    unique_id: uid.clone(),
                    ferro_uid: None,
                    name: name.clone(),
                    elevation: *elevation,
                }),
                RevitOp::WallType {
                    uid, name, layers, ..
                } => snap.wall_types.push(RevitType {
                    unique_id: uid.clone(),
                    ferro_uid: None,
                    name: name.clone(),
                    layers: layers
                        .iter()
                        .map(|l| RevitLayer {
                            function: l.function.clone(),
                            width: l.width,
                            material_name: None,
                        })
                        .collect(),
                }),
                RevitOp::Wall {
                    uid,
                    type_uid,
                    level_uid,
                    curve,
                    base_offset,
                    height,
                    top_level_uid,
                    top_offset,
                    location_line,
                    ..
                } => snap.walls.push(RevitWall {
                    unique_id: uid.clone(),
                    ferro_uid: None,
                    name: None,
                    type_unique_id: type_uid.clone(),
                    level_unique_id: level_uid.clone(),
                    curve: RevitCurve {
                        start: curve.start,
                        end: curve.end,
                        mid: curve.mid,
                    },
                    base_offset: *base_offset,
                    height: *height,
                    top_level_unique_id: top_level_uid.clone(),
                    top_offset: *top_offset,
                    location_line: *location_line,
                }),
                _ => {}
            }
        }
        let mut dst = empty();
        let r = import(&mut dst, &ExecContext::user(), &snap);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let vol = |d: &Document| -> f64 {
            d.elements()
                .filter(|e| e.as_wall().is_some())
                .map(|e| e.geometry().unwrap().volume())
                .sum()
        };
        let (a, b, c) = (vol(src.doc()), vol(dst.doc()), door_volume(src.doc()));
        // Feet are rounded to 1e-9 ft in the plan: allow a relative 1e-8.
        assert!((a - b - c).abs() < a * 1e-8, "src {a} dst {b} door {c}");
    }

    fn door_volume(d: &Document) -> f64 {
        d.elements()
            .filter_map(|e| match &e.body {
                ElementBody::Opening(o) => Some(o.width.mm() * o.height.mm() * 230.0),
                _ => None,
            })
            .sum::<f64>()
            * -1.0
    }
}
