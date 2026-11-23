use super::ElementId;
use crate::math::{Angle, Length, QuantitySpec};
use serde::{Deserialize, Serialize};

/// A typed parameter value (D-15, ADR 0004 §2). Lengths in mm, angles in
/// radians, areas in mm², volumes in mm³.
///
/// New parameters with a physical quantity use [`ParamValue::Quantity`].
/// `Length`, `Angle`, `Area`, `Volume` and `Number` are kept so files from
/// M1–M4 still load and computed parameters keep their shape; they report
/// their spec through [`ParamValue::spec`] / [`ParamValue::as_quantity`], so
/// readers should use those instead of matching the variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ParamValue {
    /// Length in mm. Prefer `Quantity { spec: Length, .. }` for new keys.
    Length(Length),
    /// Angle in radians. Prefer `Quantity { spec: Angle, .. }` for new keys.
    Angle(Angle),
    /// Area in mm². Prefer `Quantity { spec: Area, .. }` for new keys.
    Area(f64),
    /// Volume in mm³. Prefer `Quantity { spec: Volume, .. }` for new keys.
    Volume(f64),
    /// Unitless real number.
    Number(f64),
    Integer(i64),
    Bool(bool),
    Text(String),
    /// User tags (D-74), e.g. `["phase-1", "client-review"]`.
    Tags(Vec<String>),
    ElementRef(ElementId),
    /// A physical quantity, `value` in the spec's storage unit (SI; mm for
    /// length, rad for angle; see [`QuantitySpec::storage_unit`]).
    /// Serialised as `{"type": "quantity", "value": {"spec": "density",
    /// "value": 1200.0}}`.
    Quantity {
        spec: QuantitySpec,
        value: f64,
    },
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
            ParamValue::Quantity { .. } => "quantity",
        }
    }

    /// The physical quantity of this value: `Length` / `Angle` / `Area` /
    /// `Volume` / `Number` report their spec, `Quantity` its own; `None` for
    /// integers, booleans, text, tags and element references.
    pub fn spec(&self) -> Option<QuantitySpec> {
        self.as_quantity().map(|(s, _)| s)
    }

    /// The spec and the value in storage units, for any quantity-like
    /// variant (see [`ParamValue::spec`]). `None` for non-quantities.
    pub fn as_quantity(&self) -> Option<(QuantitySpec, f64)> {
        match self {
            ParamValue::Length(l) => Some((QuantitySpec::Length, l.mm())),
            ParamValue::Angle(a) => Some((QuantitySpec::Angle, a.rad())),
            ParamValue::Area(a) => Some((QuantitySpec::Area, *a)),
            ParamValue::Volume(v) => Some((QuantitySpec::Volume, *v)),
            ParamValue::Number(n) => Some((QuantitySpec::Number, *n)),
            ParamValue::Quantity { spec, value } => Some((*spec, *value)),
            _ => None,
        }
    }

    /// `false` if a numeric value is NaN or infinite. Commands reject such
    /// values (`ferro.model.invalid_parameter_value`).
    pub fn is_finite(&self) -> bool {
        self.as_quantity().is_none_or(|(_, v)| v.is_finite())
    }

    /// The value for display, e.g. `"0.8 W/(m·K)"` for quantities (default
    /// display unit), the text itself for text, `"#12"` for references.
    pub fn display(&self) -> String {
        match self {
            ParamValue::Integer(i) => i.to_string(),
            ParamValue::Bool(b) => b.to_string(),
            ParamValue::Text(t) => t.clone(),
            ParamValue::Tags(t) => t.join(", "),
            ParamValue::ElementRef(r) => r.to_string(),
            v => match v.as_quantity() {
                Some((s, x)) => s.format(x),
                None => String::new(),
            },
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

    #[test]
    fn quantity_serde_is_tagged_with_spec_id() {
        let v = ParamValue::Quantity {
            spec: QuantitySpec::ThermalConductivity,
            value: 0.8,
        };
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(
            s,
            r#"{"type":"quantity","value":{"spec":"thermal_conductivity","value":0.8}}"#
        );
        assert_eq!(serde_json::from_str::<ParamValue>(&s).unwrap(), v);
        assert!(serde_json::from_str::<ParamValue>(
            r#"{"type":"quantity","value":{"spec":"furlong","value":1}}"#
        )
        .is_err());
    }

    #[test]
    fn old_variants_still_load_and_report_their_spec() {
        // Shapes written by M1–M4 files.
        let old = [
            (
                r#"{"type":"length","value":200.0}"#,
                QuantitySpec::Length,
                200.0,
            ),
            (r#"{"type":"angle","value":1.5}"#, QuantitySpec::Angle, 1.5),
            (
                r#"{"type":"area","value":2000000.0}"#,
                QuantitySpec::Area,
                2e6,
            ),
            (
                r#"{"type":"volume","value":3.0e9}"#,
                QuantitySpec::Volume,
                3e9,
            ),
            (r#"{"type":"number","value":4}"#, QuantitySpec::Number, 4.0),
        ];
        for (json, spec, value) in old {
            let v: ParamValue = serde_json::from_str(json).unwrap();
            let (s, x) = v.as_quantity().unwrap();
            assert_eq!(s, spec, "{json}");
            assert!(s.eq_values(x, value), "{json}");
        }
        for json in [
            r#"{"type":"text","value":"hi"}"#,
            r#"{"type":"integer","value":300}"#,
            r#"{"type":"tags","value":["a"]}"#,
            r#"{"type":"element_ref","value":7}"#,
            r#"{"type":"bool","value":true}"#,
        ] {
            let v: ParamValue = serde_json::from_str(json).unwrap();
            assert_eq!(v.spec(), None, "{json}");
            assert!(v.is_finite());
        }
    }

    #[test]
    fn non_finite_quantities_are_flagged() {
        let v = |x| ParamValue::Quantity {
            spec: QuantitySpec::Density,
            value: x,
        };
        assert!(v(1200.0).is_finite());
        assert!(!v(f64::NAN).is_finite());
        assert!(!ParamValue::Area(f64::INFINITY).is_finite());
    }

    #[test]
    fn display_uses_default_units() {
        let v = ParamValue::Quantity {
            spec: QuantitySpec::Density,
            value: 1200.0,
        };
        assert_eq!(v.display(), "1200 kg/m³");
        assert_eq!(ParamValue::Area(2.5e6).display(), "2.5 m²");
        assert_eq!(ParamValue::Integer(300).display(), "300");
        assert_eq!(
            ParamValue::Tags(vec!["a".into(), "b".into()]).display(),
            "a, b"
        );
    }
}
