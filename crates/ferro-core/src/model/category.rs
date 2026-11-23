use serde::{Deserialize, Serialize};

/// Fixed classification of elements (D-14, D-40). Adding a category is a spec
/// change. Each maps to a Revit `BuiltInCategory` and an IFC4 class.
///
/// Milestone 1 implements `Levels` and `Forms`; the rest are reserved so
/// classification of Forms (D-12) already works and the mapping is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Levels,
    Grids,
    ReferencePlanes,
    Forms,
    Walls,
    Floors,
    Roofs,
    Ceilings,
    Columns,
    Doors,
    Windows,
    Stairs,
    Railings,
    Rooms,
    Furniture,
    GenericModels,
    Materials,
    /// Construction stages (ADR 0006). Not IFC products: a phase maps to
    /// `Pset_*Common.Status` on the exported elements.
    Phases,
    /// Views (D-33, M5-22): `View3D`, `PlanView`, `SectionView`. Not IFC
    /// products and not exported.
    Views,
    /// Option sets and design options (ADR 0006). Not IFC products: the
    /// export writes one combination.
    DesignOptions,
}

impl Category {
    pub const ALL: [Category; 20] = [
        Category::Levels,
        Category::Grids,
        Category::ReferencePlanes,
        Category::Forms,
        Category::Walls,
        Category::Floors,
        Category::Roofs,
        Category::Ceilings,
        Category::Columns,
        Category::Doors,
        Category::Windows,
        Category::Stairs,
        Category::Railings,
        Category::Rooms,
        Category::Furniture,
        Category::GenericModels,
        Category::Materials,
        Category::Phases,
        Category::Views,
        Category::DesignOptions,
    ];

    /// Revit `BuiltInCategory` name.
    pub fn revit_builtin(&self) -> &'static str {
        match self {
            Category::Levels => "OST_Levels",
            Category::Grids => "OST_Grids",
            Category::ReferencePlanes => "OST_CLines",
            Category::Forms => "OST_Mass",
            Category::Walls => "OST_Walls",
            Category::Floors => "OST_Floors",
            Category::Roofs => "OST_Roofs",
            Category::Ceilings => "OST_Ceilings",
            Category::Columns => "OST_Columns",
            Category::Doors => "OST_Doors",
            Category::Windows => "OST_Windows",
            Category::Stairs => "OST_Stairs",
            Category::Railings => "OST_StairsRailing",
            Category::Rooms => "OST_Rooms",
            Category::Furniture => "OST_Furniture",
            Category::GenericModels => "OST_GenericModel",
            Category::Materials => "OST_Materials",
            Category::Phases => "OST_Phases",
            Category::Views => "OST_Views",
            Category::DesignOptions => "OST_DesignOptions",
        }
    }

    /// IFC4 entity class.
    pub fn ifc_class(&self) -> &'static str {
        match self {
            Category::Levels => "IfcBuildingStorey",
            Category::Grids => "IfcGrid",
            Category::ReferencePlanes => "IfcVirtualElement",
            Category::Forms => "IfcBuildingElementProxy",
            Category::Walls => "IfcWall",
            Category::Floors => "IfcSlab",
            Category::Roofs => "IfcRoof",
            Category::Ceilings => "IfcCovering",
            Category::Columns => "IfcColumn",
            Category::Doors => "IfcDoor",
            Category::Windows => "IfcWindow",
            Category::Stairs => "IfcStair",
            Category::Railings => "IfcRailing",
            Category::Rooms => "IfcSpace",
            Category::Furniture => "IfcFurniture",
            Category::GenericModels => "IfcBuildingElementProxy",
            Category::Materials => "IfcMaterial",
            // Not an IFC product: exported as `Pset_*Common.Status` values
            // (ADR 0006); `IfcWorkSchedule` is not used.
            Category::Phases => "IfcPropertySingleValue",
            // Not exported: views have no geometry and IFC4 has no view
            // product. `IfcAnnotation` is the nearest presentation class.
            Category::Views => "IfcAnnotation",
            // Not exported: IFC gets one combination (ADR 0006).
            Category::DesignOptions => "IfcGroup",
        }
    }

    /// Categories a `Form` may be classified as (D-12).
    pub fn classifiable_form(&self) -> bool {
        !matches!(
            self,
            Category::Levels
                | Category::Grids
                | Category::ReferencePlanes
                | Category::Rooms
                | Category::Materials
                | Category::Phases
                | Category::Views
                | Category::DesignOptions
        )
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        write!(f, "{s}")
    }
}
