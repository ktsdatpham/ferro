use super::bim::{Floor, FloorType, Material, Opening, Wall, WallType};
use super::{Category, ElementId, ParamValue, Uid};
use crate::math::{BoundingBox3, Length};
use crate::topo::Body;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A named elevation (D-16). `is_building_storey` marks real storeys
/// (`IfcBuildingStorey`), as in Q-E2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub elevation: Length,
    pub is_building_storey: bool,
}

/// Free-form geometry edited directly (D-12, D-38). Geometry is stored in
/// model coordinates (relative to the Internal Origin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub body: Body,
}

/// Category-specific data of an element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementBody {
    Level(Level),
    Form(Form),
    Material(Material),
    WallType(WallType),
    FloorType(FloorType),
    Wall(Wall),
    Floor(Floor),
    Opening(Opening),
}

/// Anything in the document with identity (Foundation Spec §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    pub uid: Uid,
    pub category: Category,
    pub name: Option<String>,
    /// `FamilyType` (reserved until families land in milestone 2).
    pub type_id: Option<ElementId>,
    /// Home level (D-16, Q-E2).
    pub level_id: Option<ElementId>,
    /// Hosting element (doors in walls; reserved).
    pub host_id: Option<ElementId>,
    /// Design option membership (D-32); `None` = shared model.
    pub design_option: Option<ElementId>,
    /// Phase created / demolished (D-32, reserved).
    pub phase_created: Option<ElementId>,
    pub phase_demolished: Option<ElementId>,
    pub params: BTreeMap<String, ParamValue>,
    pub body: ElementBody,
}

impl Element {
    pub fn as_form(&self) -> Option<&Form> {
        match &self.body {
            ElementBody::Form(f) => Some(f),
            _ => None,
        }
    }
    pub fn as_level(&self) -> Option<&Level> {
        match &self.body {
            ElementBody::Level(l) => Some(l),
            _ => None,
        }
    }
    /// Model geometry of the element, if it has any (Forms, walls, floors,
    /// openings).
    pub fn geometry(&self) -> Option<&Body> {
        match &self.body {
            ElementBody::Form(f) => Some(&f.body),
            ElementBody::Wall(w) => Some(&w.geometry),
            ElementBody::Floor(f) => Some(&f.geometry),
            ElementBody::Opening(o) => Some(&o.geometry),
            _ => None,
        }
    }
    /// `true` for type and material definitions (not placed in the model).
    pub fn is_definition(&self) -> bool {
        matches!(
            self.body,
            ElementBody::Material(_) | ElementBody::WallType(_) | ElementBody::FloorType(_)
        )
    }
    pub fn as_wall(&self) -> Option<&Wall> {
        match &self.body {
            ElementBody::Wall(w) => Some(w),
            _ => None,
        }
    }
    pub fn param(&self, key: &str) -> Option<&ParamValue> {
        self.params.get(key)
    }
    pub fn bounding_box(&self) -> Option<BoundingBox3> {
        self.geometry().and_then(|b| b.bounding_box())
    }
    /// Display label: name, else category and id.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("{} {}", self.category, self.id))
    }
}
