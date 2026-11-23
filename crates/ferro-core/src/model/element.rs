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

/// Where an element's geometry comes from (ADR 0004 §1). The geometric
/// variants of [`ElementBody`] are the source: `Wall`, `Floor` and `Opening`
/// carry the parameters their geometry is generated from; `Form` is geometry
/// edited directly, whatever the element's category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometrySource {
    Parametric,
    Direct,
}

/// Category-specific data of an element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementBody {
    Level(Level),
    Form(Form),
    /// Boxed: a material with assets is much larger than other bodies.
    Material(Box<Material>),
    WallType(WallType),
    FloorType(FloorType),
    Wall(Wall),
    Floor(Floor),
    Opening(Opening),
    /// A construction stage (category `Phases`, ADR 0006).
    Phase(super::phase::Phase),
    /// A view: camera, cut, style, hidden elements, phase (category
    /// `Views`, D-33).
    View(super::view::View),
    /// An option set (category `DesignOptions`, ADR 0006).
    OptionSet(super::design_option::OptionSet),
    /// A design option: a layer of changes over the shared model (category
    /// `DesignOptions`, hosted by its option set; ADR 0006).
    DesignOption(super::design_option::DesignOption),
}

impl ElementBody {
    /// Stable name of the body kind, as written in files (the `kind` tag):
    /// `level`, `form`, `material`, `wall_type`, `floor_type`, `wall`,
    /// `floor`, `opening`, `phase`.
    pub fn kind(&self) -> &'static str {
        match self {
            ElementBody::Level(_) => "level",
            ElementBody::Form(_) => "form",
            ElementBody::Material(_) => "material",
            ElementBody::WallType(_) => "wall_type",
            ElementBody::FloorType(_) => "floor_type",
            ElementBody::Wall(_) => "wall",
            ElementBody::Floor(_) => "floor",
            ElementBody::Opening(_) => "opening",
            ElementBody::Phase(_) => "phase",
            ElementBody::View(_) => "view",
            ElementBody::OptionSet(_) => "option_set",
            ElementBody::DesignOption(_) => "design_option",
        }
    }
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
    /// Design option membership (D-32, ADR 0006): always `None` in the
    /// shared model, whose options are layers of changes; in a document
    /// resolved for a combination, the option whose layer added the element
    /// (a derived index, `model::design_option::apply_layers`).
    pub design_option: Option<ElementId>,
    /// Phase the element is built in (Revit: *Phase Created*; ADR 0006).
    /// `None` = existing in every phase (`model::phase::phase_status`).
    pub phase_created: Option<ElementId>,
    /// Phase the element is demolished in (Revit: *Phase Demolished*); the
    /// same phase as `phase_created` or a later one. `None` = never.
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
    /// Source of the element's geometry; `None` when it has none (levels,
    /// types, materials).
    pub fn geometry_source(&self) -> Option<GeometrySource> {
        match &self.body {
            ElementBody::Form(_) => Some(GeometrySource::Direct),
            ElementBody::Wall(_) | ElementBody::Floor(_) | ElementBody::Opening(_) => {
                Some(GeometrySource::Parametric)
            }
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
