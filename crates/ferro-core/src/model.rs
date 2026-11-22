use crate::geom::{polygon_area, polygon_normal};
use crate::math::{Point3, UnitVec3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type EntityId = u64;

/// A planar polygon face. Vertices are ordered counter-clockwise when seen
/// from the side the normal points to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub vertices: Vec<Point3>,
}

impl Face {
    pub fn normal(&self) -> Option<UnitVec3> {
        polygon_normal(&self.vertices)
    }
    pub fn area(&self) -> f64 {
        polygon_area(&self.vertices)
    }
}

/// A closed boundary-represented solid: a set of faces.
/// (v0.1 keeps it simple; a half-edge mesh replaces this in the roadmap.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solid {
    pub faces: Vec<Face>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntityKind {
    Face(Face),
    Solid(Solid),
}

/// Anything in the model. `tags` is free-form BIM-ish metadata
/// ("category": "wall", "level": "L1") that later maps to IFC / Revit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub name: Option<String>,
    pub tags: BTreeMap<String, String>,
    pub kind: EntityKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    NotFound(EntityId),
    NotAFace(EntityId),
    DegenerateFace,
    ZeroDistance,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::NotFound(id) => write!(f, "entity {id} not found"),
            ModelError::NotAFace(id) => write!(f, "entity {id} is not a face"),
            ModelError::DegenerateFace => write!(f, "face is degenerate (zero area)"),
            ModelError::ZeroDistance => write!(f, "push-pull distance must not be zero"),
        }
    }
}

impl std::error::Error for ModelError {}

/// The document. Single source of truth for the UI, the MCP server and the
/// Revit bridge.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Model {
    next_id: EntityId,
    entities: BTreeMap<EntityId, Entity>,
}

impl Model {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            entities: BTreeMap::new(),
        }
    }

    fn insert(&mut self, kind: EntityKind) -> EntityId {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.entities.insert(
            id,
            Entity {
                id,
                name: None,
                tags: BTreeMap::new(),
                kind,
            },
        );
        id
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn remove(&mut self, id: EntityId) -> Result<Entity, ModelError> {
        self.entities.remove(&id).ok_or(ModelError::NotFound(id))
    }

    /// Draw a face from a closed polygon.
    pub fn add_face(&mut self, vertices: Vec<Point3>) -> Result<EntityId, ModelError> {
        let face = Face { vertices };
        if face.vertices.len() < 3 || face.area() < 1e-6 {
            return Err(ModelError::DegenerateFace);
        }
        Ok(self.insert(EntityKind::Face(face)))
    }

    /// SketchUp's rectangle tool, on the ground plane (z = elevation).
    pub fn add_rectangle(
        &mut self,
        origin: Point3,
        width: f64,
        depth: f64,
    ) -> Result<EntityId, ModelError> {
        let o = origin;
        self.add_face(vec![
            o,
            o + Vec3::new(width, 0.0, 0.0),
            o + Vec3::new(width, depth, 0.0),
            o + Vec3::new(0.0, depth, 0.0),
        ])
    }

    /// SketchUp's push-pull: extrude a face along its normal into a solid.
    /// The face entity is replaced by the solid, keeping id, name and tags.
    pub fn push_pull(&mut self, id: EntityId, distance: f64) -> Result<EntityId, ModelError> {
        if distance.abs() < 1e-9 {
            return Err(ModelError::ZeroDistance);
        }
        let entity = self.entities.get(&id).ok_or(ModelError::NotFound(id))?;
        let EntityKind::Face(face) = &entity.kind else {
            return Err(ModelError::NotAFace(id));
        };
        let n = face.normal().ok_or(ModelError::DegenerateFace)?;
        let offset = n * distance;

        let base: Vec<Point3> = face.vertices.clone();
        let top: Vec<Point3> = base.iter().map(|&v| v + offset).collect();

        // Keep every face pointing outward whichever way we pulled.
        let (bottom_loop, top_loop) = if distance > 0.0 {
            (base.iter().rev().copied().collect::<Vec<_>>(), top.clone())
        } else {
            (base.clone(), top.iter().rev().copied().collect::<Vec<_>>())
        };

        let mut faces = vec![
            Face {
                vertices: bottom_loop,
            },
            Face { vertices: top_loop },
        ];
        let k = base.len();
        for i in 0..k {
            let j = (i + 1) % k;
            let quad = if distance > 0.0 {
                vec![base[i], base[j], top[j], top[i]]
            } else {
                vec![base[j], base[i], top[i], top[j]]
            };
            faces.push(Face { vertices: quad });
        }

        let entity = self.entities.get_mut(&id).expect("checked above");
        entity.kind = EntityKind::Solid(Solid { faces });
        Ok(id)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("model is always serialisable")
    }

    pub fn from_json(s: &str) -> serde_json::Result<Model> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_then_push_pull_makes_a_box() {
        let mut m = Model::new();
        let id = m.add_rectangle(Point3::ORIGIN, 4000.0, 3000.0).unwrap();
        m.push_pull(id, 2800.0).unwrap();

        let EntityKind::Solid(s) = &m.get(id).unwrap().kind else {
            panic!("not a solid")
        };
        assert_eq!(s.faces.len(), 6);

        // Every face normal must point away from the box centre.
        let c = Point3::new(2000.0, 1500.0, 1400.0);
        for f in &s.faces {
            let n = f.normal().unwrap();
            assert!(
                (f.vertices[0] - c).dot(n.as_vec()) > 0.0,
                "inward face: {f:?}"
            );
        }
    }

    #[test]
    fn negative_push_pull_also_points_outward() {
        let mut m = Model::new();
        let id = m.add_rectangle(Point3::ORIGIN, 1000.0, 1000.0).unwrap();
        m.push_pull(id, -500.0).unwrap();
        let EntityKind::Solid(s) = &m.get(id).unwrap().kind else {
            panic!()
        };
        let c = Point3::new(500.0, 500.0, -250.0);
        for f in &s.faces {
            assert!((f.vertices[0] - c).dot(f.normal().unwrap().as_vec()) > 0.0);
        }
    }

    #[test]
    fn json_round_trip() {
        let mut m = Model::new();
        m.add_rectangle(Point3::ORIGIN, 1.0, 1.0).unwrap();
        let back = Model::from_json(&m.to_json()).unwrap();
        assert_eq!(back.entities().count(), 1);
    }
}
