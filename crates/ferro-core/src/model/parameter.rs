use super::ElementId;
use crate::math::{Angle, Length};
use serde::{Deserialize, Serialize};

/// A typed parameter value (D-15). Lengths in mm, angles in radians,
/// areas in mm², volumes in mm³.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ParamValue {
    Length(Length),
    Angle(Angle),
    Area(f64),
    Volume(f64),
    Number(f64),
    Integer(i64),
    Bool(bool),
    Text(String),
    /// User tags (D-74), e.g. `["phase-1", "client-review"]`.
    Tags(Vec<String>),
    ElementRef(ElementId),
}

impl ParamValue {
    /// Short type name, matches the serde tag.
    pub fn type_name(&self) -> &'static str {
        match self {
            ParamValue::Length(_) => "length",
            ParamValue::Angle(_) => "angle",
            ParamValue::Area(_) => "area",
            ParamValue::Volume(_) => "volume",
            ParamValue::Number(_) => "number",
            ParamValue::Integer(_) => "integer",
            ParamValue::Bool(_) => "bool",
            ParamValue::Text(_) => "text",
            ParamValue::Tags(_) => "tags",
            ParamValue::ElementRef(_) => "element_ref",
        }
    }
}

/// Built-in parameter keys. Stable forever once released (D-15).
pub mod keys {
    /// Computed: total surface area of a Form (mm²). Read-only.
    pub const FORM_AREA: &str = "ferro.form.area";
    /// Computed: volume of a closed Form (mm³). Read-only.
    pub const FORM_VOLUME: &str = "ferro.form.volume";
    /// User tags (D-74).
    pub const TAGS: &str = "ferro.common.tags";
    /// Free comment.
    pub const COMMENTS: &str = "ferro.common.comments";
    /// Development level (D-69), e.g. 100, 200, 300.
    pub const LOD: &str = "ferro.common.lod";
    /// Material painted on an element (element reference to a material).
    pub const MATERIAL: &str = "ferro.common.material";
    /// Classification code (Uniclass / OmniClass / Vietnamese code), D-40.
    pub const CLASSIFICATION: &str = "ferro.common.classification";

    /// Computed: length of a wall's location curve (mm). Read-only.
    pub const LENGTH: &str = "ferro.dimensions.length";
    /// Computed: height of a wall (mm). Read-only.
    pub const HEIGHT: &str = "ferro.dimensions.height";
    /// Computed: thickness from the element's type (mm). Read-only.
    pub const THICKNESS: &str = "ferro.dimensions.thickness";
    /// Computed: plan area of a floor (mm²). Read-only.
    pub const AREA: &str = "ferro.dimensions.area";
    /// Computed: volume of a wall or floor, openings deducted (mm³). Read-only.
    pub const VOLUME: &str = "ferro.dimensions.volume";

    /// Keys computed by regeneration; commands may not write them.
    pub const READ_ONLY: [&str; 7] = [
        FORM_AREA,
        FORM_VOLUME,
        LENGTH,
        HEIGHT,
        THICKNESS,
        AREA,
        VOLUME,
    ];

    /// A parameter key is `namespace.group.name`, lowercase snake case.
    pub fn is_valid(key: &str) -> bool {
        let parts: Vec<&str> = key.split('.').collect();
        parts.len() >= 3
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
    }

    pub fn is_read_only(key: &str) -> bool {
        READ_ONLY.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_format() {
        assert!(keys::is_valid("ferro.wall.base_offset"));
        assert!(keys::is_valid("acme.fengshui.score_2"));
        assert!(!keys::is_valid("Ferro.wall.x"));
        assert!(!keys::is_valid("ferro.wall"));
        assert!(!keys::is_valid("ferro..x"));
        assert!(!keys::is_valid("ferro.wall.base offset"));
    }

    #[test]
    fn value_serde_is_tagged() {
        let v = ParamValue::Length(Length::from_mm(200.0));
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#"{"type":"length","value":200.0}"#);
        assert_eq!(serde_json::from_str::<ParamValue>(&s).unwrap(), v);
    }
}
