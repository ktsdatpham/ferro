//! BIM element data (Foundation Spec ch. 3, D-41, D-70): system family types
//! with compound structures, walls, floors and hosted openings. `Material`
//! lives in `model::material` and is re-exported here.
//! Geometry is derived by regeneration (`model::generate`) and stored on the
//! element as a cache; parameters are the source of truth.

use super::ElementId;
use crate::geom::Curve;
use crate::math::{Length, Point2};
use crate::topo::Body;
use serde::{Deserialize, Serialize};

/// Re-export: `bim::Material` keeps working (milestone 2 path).
pub use super::material::Material;

/// Function of a layer in a compound structure (Revit: `MaterialFunctionAssignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerFunction {
    Structure,
    Substrate,
    Insulation,
    Finish,
    Membrane,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundLayer {
    pub function: LayerFunction,
    pub thickness: Length,
    #[serde(default)]
    pub material: Option<ElementId>,
}

/// Layers listed from the exterior face to the interior face (Revit:
/// `CompoundStructure`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundStructure {
    pub layers: Vec<CompoundLayer>,
}

impl CompoundStructure {
    pub fn single(thickness: Length) -> Self {
        CompoundStructure {
            layers: vec![CompoundLayer {
                function: LayerFunction::Structure,
                thickness,
                material: None,
            }],
        }
    }

    pub fn thickness(&self) -> f64 {
        self.layers.iter().map(|l| l.thickness.mm()).sum()
    }

    /// Distance from the exterior face to the centre of the structural core
    /// (the structure layers). Falls back to the wall centre.
    pub fn core_center_from_exterior(&self) -> f64 {
        let mut d = 0.0;
        let (mut a, mut b) = (None, 0.0);
        for l in &self.layers {
            if l.function == LayerFunction::Structure {
                a.get_or_insert(d);
                b = d + l.thickness.mm();
            }
            d += l.thickness.mm();
        }
        match a {
            Some(a) => (a + b) * 0.5,
            None => d * 0.5,
        }
    }

    /// Material of the first structure layer (display colour).
    pub fn core_material(&self) -> Option<ElementId> {
        self.layers
            .iter()
            .find(|l| l.function == LayerFunction::Structure)
            .or(self.layers.first())
            .and_then(|l| l.material)
    }
}

/// A wall type (system family type, D-70).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallType {
    pub structure: CompoundStructure,
}

/// A floor type (system family type, D-70).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorType {
    pub structure: CompoundStructure,
}

/// Which line of the wall the location curve runs along (Revit: `WALL_KEY_REF_PARAM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationLine {
    #[default]
    WallCenterline,
    CoreCenterline,
    FinishExterior,
    FinishInterior,
}

/// Top constraint of a wall (D-16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WallTop {
    /// Fixed height above the base.
    Unconnected { height: Length },
    /// Attached to a level plus offset.
    Level { level: ElementId, offset: Length },
}

/// A wall instance. Base level = `Element::level_id`; type = `Element::type_id`.
/// The location curve is horizontal (its z is ignored); the exterior face is
/// on the **left** of the curve direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub curve: Curve,
    #[serde(default)]
    pub base_offset: Length,
    pub top: WallTop,
    #[serde(default)]
    pub location_line: LocationLine,
    /// Derived geometry (regenerated).
    #[serde(default)]
    pub geometry: Body,
}

/// A floor instance: a sketch profile (plan coordinates, mm) whose top sits at
/// level + offset. Type = `Element::type_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    pub outer: Vec<Point2>,
    #[serde(default)]
    pub holes: Vec<Vec<Point2>>,
    #[serde(default)]
    pub offset: Length,
    #[serde(default)]
    pub geometry: Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpeningKind {
    Door,
    Window,
}

/// A door or window hosted by a wall (`Element::host_id`), cutting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opening {
    /// Serialised as `opening_kind`: `kind` is the element body's own tag.
    #[serde(rename = "opening_kind")]
    pub kind: OpeningKind,
    /// Distance along the host wall's location curve from its start to the
    /// opening centre (mm).
    pub position: Length,
    pub width: Length,
    pub height: Length,
    /// Height of the bottom of the opening above the wall base (0 for doors).
    #[serde(default)]
    pub sill: Length,
    /// Derived display geometry (frame / panel).
    #[serde(default)]
    pub geometry: Body,
}
