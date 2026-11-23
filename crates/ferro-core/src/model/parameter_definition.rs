//! Parameter definitions (ADR 0004 §2.3, Foundation Spec §3.5): what a
//! parameter key means, as data with a `Uid`, like a Revit shared parameter.
//!
//! - **Built-in** definitions (keys `ferro.*`) come from a fixed registry,
//!   [`builtin_definitions`]. Their `Uid` is [`ParameterDefinition::uid_for_key`],
//!   derived from the key, so it is never stored and never changes.
//! - **Project** definitions (user keys such as `acme.energy.u_value`) are
//!   stored in the document's project information and added with the
//!   `set_parameter_definition` command. A definition imported from a Revit
//!   shared parameter keeps that parameter's GUID as its `Uid`.
//!
//! Keys without a definition stay allowed (any typed value, D-15).
//! Round-trips match definitions by `Uid`, not by key or name.

use super::{keys, ParamValue, Uid};
use crate::math::QuantitySpec;
use serde::{Deserialize, Serialize};

/// What values a parameter holds (Revit: data type / `ForgeTypeId` spec).
/// Serialised as `"text"` or `{"quantity": "density"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterDataType {
    /// A physical quantity; values are `ParamValue::Quantity` with this spec,
    /// or an old variant reporting it (`Length`, `Area`…).
    Quantity(QuantitySpec),
    Integer,
    Bool,
    Text,
    Tags,
    ElementRef,
}

impl ParameterDataType {
    /// `true` if `value` has this data type (see [`ParamValue::spec`]).
    pub fn accepts(&self, value: &ParamValue) -> bool {
        match self {
            ParameterDataType::Quantity(s) => value.spec() == Some(*s),
            ParameterDataType::Integer => matches!(value, ParamValue::Integer(_)),
            ParameterDataType::Bool => matches!(value, ParamValue::Bool(_)),
            ParameterDataType::Text => matches!(value, ParamValue::Text(_)),
            ParameterDataType::Tags => matches!(value, ParamValue::Tags(_)),
            ParameterDataType::ElementRef => matches!(value, ParamValue::ElementRef(_)),
        }
    }

    /// Short name for messages, e.g. `"quantity (density)"`, `"text"`.
    pub fn describe(&self) -> String {
        match self {
            ParameterDataType::Quantity(s) => format!("quantity ({s})"),
            ParameterDataType::Integer => "integer".into(),
            ParameterDataType::Bool => "bool".into(),
            ParameterDataType::Text => "text".into(),
            ParameterDataType::Tags => "tags".into(),
            ParameterDataType::ElementRef => "element_ref".into(),
        }
    }
}

/// Where a parameter is shown in the properties panel (Revit: `GroupTypeId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterGroup {
    Constraints,
    Dimensions,
    IdentityData,
    Materials,
    /// Energy and building physics (U-value, conductivity…).
    Analysis,
    Structural,
    /// Anything else (Revit "Other" / Data).
    #[default]
    Data,
}

impl ParameterGroup {
    /// Name of the Revit `GroupTypeId` property, e.g. `"Geometry"` for
    /// dimensions (Revit shows it as "Dimensions").
    pub fn revit_group(self) -> &'static str {
        match self {
            ParameterGroup::Constraints => "Constraints",
            ParameterGroup::Dimensions => "Geometry",
            ParameterGroup::IdentityData => "IdentityData",
            ParameterGroup::Materials => "Materials",
            ParameterGroup::Analysis => "EnergyAnalysis",
            ParameterGroup::Structural => "Structural",
            ParameterGroup::Data => "Data",
        }
    }
}

/// Whether the parameter is set per element or per type (D-15; Revit:
/// `InstanceBinding` / `TypeBinding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterBinding {
    #[default]
    Instance,
    Type,
}

/// Where a parameter goes in IFC: a property in a property set, e.g.
/// `Pset_WallCommon.ThermalTransmittance`. The value is written with the
/// spec's IFC measure type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfcPropertyMapping {
    /// Property set name, e.g. `"Pset_WallCommon"`.
    pub property_set: String,
    /// Property name inside the set, e.g. `"ThermalTransmittance"`.
    pub property: String,
}

/// The definition of a parameter key (ADR 0004 §2.3). File keys are stable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterDefinition {
    /// Stable global id; Revit shared parameter GUID on export / import.
    pub uid: Uid,
    /// Parameter key, `namespace.group.name` (§5.6).
    pub key: String,
    pub data_type: ParameterDataType,
    #[serde(default)]
    pub group: ParameterGroup,
    #[serde(default)]
    pub binding: ParameterBinding,
    /// Human description (English), shown as a tooltip.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Computed by regeneration; commands may not write it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// Revit `BuiltInParameter` name when the parameter is a Revit built-in
    /// (e.g. `"CURVE_ELEM_LENGTH"`); otherwise it maps to a shared parameter
    /// with GUID = `uid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revit_builtin: Option<String>,
    /// IFC property set mapping; `None` = not exported as a property.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifc: Option<IfcPropertyMapping>,
}

impl ParameterDefinition {
    /// The deterministic `Uid` of the definition of `key`:
    /// `Uid::from_name("ferro.parameter_definition:" + key)`. Used for every
    /// built-in definition and as the default for project definitions.
    pub fn uid_for_key(key: &str) -> Uid {
        Uid::from_name(&format!("ferro.parameter_definition:{key}"))
    }

    /// `true` if `value` fits this definition's data type.
    pub fn accepts(&self, value: &ParamValue) -> bool {
        self.data_type.accepts(value)
    }

    /// `true` for keys reserved for Ferro's built-in definitions
    /// (namespace `ferro`).
    pub fn is_builtin_key(key: &str) -> bool {
        key.split('.').next() == Some("ferro")
    }
}

fn builtin(
    key: &str,
    data_type: ParameterDataType,
    group: ParameterGroup,
    description: &str,
    revit_builtin: Option<&str>,
) -> ParameterDefinition {
    ParameterDefinition {
        uid: ParameterDefinition::uid_for_key(key),
        key: key.into(),
        data_type,
        group,
        binding: ParameterBinding::Instance,
        description: description.into(),
        read_only: keys::is_read_only(key),
        revit_builtin: revit_builtin.map(String::from),
        ifc: None,
    }
}

/// Definitions of every built-in key in [`keys`], sorted by key. Pure and
/// deterministic: the same list on every call.
///
/// Computed dimensions are exported to IFC as base quantities
/// (`Qto_*BaseQuantities`) by the IFC writer, so they have no property-set
/// mapping here.
pub fn builtin_definitions() -> Vec<ParameterDefinition> {
    use ParameterDataType as T;
    use ParameterGroup as G;
    let q = T::Quantity;
    let mut v = vec![
        builtin(
            keys::FORM_AREA,
            q(QuantitySpec::Area),
            G::Dimensions,
            "Total surface area of a Form.",
            None,
        ),
        builtin(
            keys::FORM_VOLUME,
            q(QuantitySpec::Volume),
            G::Dimensions,
            "Volume of a closed Form.",
            None,
        ),
        builtin(keys::TAGS, T::Tags, G::IdentityData, "User tags.", None),
        builtin(
            keys::COMMENTS,
            T::Text,
            G::IdentityData,
            "Free comment.",
            Some("ALL_MODEL_INSTANCE_COMMENTS"),
        ),
        builtin(
            keys::LOD,
            T::Integer,
            G::IdentityData,
            "Development level, e.g. 100, 200, 300.",
            None,
        ),
        builtin(
            keys::MATERIAL,
            T::ElementRef,
            G::Materials,
            "Material painted on the element.",
            None,
        ),
        builtin(
            keys::CLASSIFICATION,
            T::Text,
            G::IdentityData,
            "Classification code (Uniclass, OmniClass, Vietnamese code).",
            None,
        ),
        builtin(
            keys::LENGTH,
            q(QuantitySpec::Length),
            G::Dimensions,
            "Length of a wall's location curve.",
            Some("CURVE_ELEM_LENGTH"),
        ),
        builtin(
            keys::HEIGHT,
            q(QuantitySpec::Length),
            G::Dimensions,
            "Height of a wall.",
            Some("WALL_USER_HEIGHT_PARAM"),
        ),
        builtin(
            keys::THICKNESS,
            q(QuantitySpec::Length),
            G::Dimensions,
            "Thickness from the element's type.",
            None,
        ),
        builtin(
            keys::AREA,
            q(QuantitySpec::Area),
            G::Dimensions,
            "Plan area of a floor.",
            Some("HOST_AREA_COMPUTED"),
        ),
        builtin(
            keys::VOLUME,
            q(QuantitySpec::Volume),
            G::Dimensions,
            "Volume of a wall or floor, openings deducted.",
            Some("HOST_VOLUME_COMPUTED"),
        ),
    ];
    v.sort_by(|a, b| a.key.cmp(&b.key));
    v
}

/// The built-in definition of `key`, or `None` if `key` is not built in.
pub fn builtin_definition(key: &str) -> Option<ParameterDefinition> {
    builtin_definitions().into_iter().find(|d| d.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Length;

    #[test]
    fn every_builtin_key_has_one_valid_definition() {
        let defs = builtin_definitions();
        assert_eq!(defs.len(), 12);
        for d in &defs {
            assert!(keys::is_valid(&d.key), "{}", d.key);
            assert!(ParameterDefinition::is_builtin_key(&d.key));
            assert_eq!(d.uid, ParameterDefinition::uid_for_key(&d.key));
            assert_eq!(d.uid.version(), 8);
            assert!(!d.description.is_empty());
        }
        let mut uids: Vec<_> = defs.iter().map(|d| d.uid).collect();
        uids.sort();
        uids.dedup();
        assert_eq!(uids.len(), defs.len());
        // Read-only flags agree with keys::READ_ONLY.
        for k in keys::READ_ONLY {
            assert!(builtin_definition(k).unwrap().read_only, "{k}");
        }
        assert!(!builtin_definition(keys::COMMENTS).unwrap().read_only);
        assert!(builtin_definition("acme.x.y").is_none());
    }

    #[test]
    fn builtin_uids_are_fixed_forever() {
        // Golden value: if this changes, every Revit / IFC round-trip breaks.
        assert_eq!(
            ParameterDefinition::uid_for_key(keys::COMMENTS),
            Uid::from_name("ferro.parameter_definition:ferro.common.comments")
        );
        assert_eq!(builtin_definitions(), builtin_definitions());
    }

    #[test]
    fn data_type_accepts_old_and_new_quantity_variants() {
        let len = ParameterDataType::Quantity(QuantitySpec::Length);
        assert!(len.accepts(&ParamValue::Length(Length::from_mm(200.0))));
        assert!(len.accepts(&ParamValue::Quantity {
            spec: QuantitySpec::Length,
            value: 200.0
        }));
        assert!(!len.accepts(&ParamValue::Area(1.0)));
        assert!(!len.accepts(&ParamValue::Text("200".into())));
        assert!(ParameterDataType::Text.accepts(&ParamValue::Text(String::new())));
        assert!(!ParameterDataType::Integer.accepts(&ParamValue::Number(1.0)));
    }

    #[test]
    fn definition_serde_is_compact_and_round_trips() {
        let d = ParameterDefinition {
            uid: ParameterDefinition::uid_for_key("acme.energy.u_value"),
            key: "acme.energy.u_value".into(),
            data_type: ParameterDataType::Quantity(QuantitySpec::UValue),
            group: ParameterGroup::Analysis,
            binding: ParameterBinding::Type,
            description: String::new(),
            read_only: false,
            revit_builtin: None,
            ifc: Some(IfcPropertyMapping {
                property_set: "Pset_WallCommon".into(),
                property: "ThermalTransmittance".into(),
            }),
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(s.contains(r#""data_type":{"quantity":"u_value"}"#), "{s}");
        assert!(
            !s.contains("read_only") && !s.contains("description"),
            "{s}"
        );
        assert_eq!(serde_json::from_str::<ParameterDefinition>(&s).unwrap(), d);
        // Minimal form: group and binding default.
        let min: ParameterDefinition = serde_json::from_str(&format!(
            r#"{{"uid":"{}","key":"acme.x.y","data_type":"text"}}"#,
            d.uid
        ))
        .unwrap();
        assert_eq!(min.group, ParameterGroup::Data);
        assert_eq!(min.binding, ParameterBinding::Instance);
    }
}
