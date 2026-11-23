//! Material commands (ADR 0004 §3): `create_material` and `set_material`.
//! Each `NAME` is also the MCP tool name. Asset values use the units named
//! in their fields (`model::material`).

use super::bim_commands::parse_color;
use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::model::material::{
    AppearanceAsset, CostAsset, EnvironmentalAsset, Material, PhysicalAsset, ThermalAsset,
    DEFAULT_ROUGHNESS,
};
use crate::model::{Category, ElementBody, ElementId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

fn invalid(msg: impl Into<String>) -> CommandError {
    CommandError::new("ferro.model.invalid_material", msg)
}

fn color(s: &str) -> Result<[f32; 3], CommandError> {
    parse_color(s).ok_or_else(|| {
        CommandError::new(
            "ferro.model.invalid_color",
            format!("{s:?} is not a #rrggbb colour"),
        )
    })
}

/// Create a material with a base colour (`"#rrggbb"`), optional roughness
/// (0..1, default 0.8) and optional assets. Fails with
/// `ferro.model.duplicate_name`, `ferro.model.invalid_color` or
/// `ferro.model.invalid_material` (a value out of range).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateMaterial {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub roughness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Box<AppearanceAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical: Option<Box<PhysicalAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<Box<ThermalAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environmental: Option<Box<EnvironmentalAsset>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Box<CostAsset>>,
}

impl Command for CreateMaterial {
    const NAME: &'static str = "create_material";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if tx.doc().elements().any(|e| {
            matches!(e.body, ElementBody::Material(_)) && e.name.as_deref() == Some(&self.name)
        }) {
            return Err(CommandError::new(
                "ferro.model.duplicate_name",
                format!("{:?} already exists", self.name),
            ));
        }
        let mut m = Material::new(
            color(&self.color)?,
            self.roughness.unwrap_or(DEFAULT_ROUGHNESS).clamp(0.0, 1.0),
        );
        m.appearance = self.appearance.as_deref().cloned();
        m.physical = self.physical.as_deref().cloned();
        m.thermal = self.thermal.as_deref().cloned();
        m.environmental = self.environmental.as_deref().cloned();
        m.cost = self.cost.as_deref().cloned();
        m.validate().map_err(invalid)?;
        let id = tx.create(
            Category::Materials,
            Some(self.name.clone()),
            None,
            ElementBody::Material(Box::new(m)),
        )?;
        Ok(json!({ "element": id }))
    }
}

/// An asset of a material, named as in the file and tool arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialAsset {
    Appearance,
    Physical,
    Thermal,
    Environmental,
    Cost,
}

/// Edit a material; omitted fields keep their value. `color` / `roughness`
/// replace those values. Each asset argument is a JSON merge patch
/// (RFC 7386) applied to the current asset (or to an empty one): given
/// fields are set, `null` fields are cleared, others kept. `clear` removes
/// whole assets first. Fails with `ferro.model.not_a_material`,
/// `ferro.model.invalid_color` or `ferro.model.invalid_material` (unknown
/// field, missing required field, value out of range).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetMaterial {
    pub material: ElementId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environmental: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<MaterialAsset>,
}

/// RFC 7386 JSON merge patch.
fn merge(target: &mut Value, patch: &Value) {
    let Value::Object(p) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    if let Value::Object(t) = target {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                merge(t.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
    }
}

/// Apply a merge patch to an optional asset.
fn patched<T: Serialize + DeserializeOwned>(
    name: &str,
    current: &Option<T>,
    patch: &Option<Value>,
) -> Result<Option<T>, CommandError> {
    let Some(patch) = patch else {
        return Ok(None);
    };
    if !patch.is_object() {
        return Err(invalid(format!("{name} must be an object of fields")));
    }
    let mut v = match current {
        Some(c) => serde_json::to_value(c).map_err(|e| invalid(e.to_string()))?,
        None => Value::Object(Map::new()),
    };
    merge(&mut v, patch);
    let asset: T = serde_json::from_value(v).map_err(|e| invalid(format!("{name}: {e}")))?;
    // Serde ignores unknown fields; a misspelt field must not vanish silently.
    let back = serde_json::to_value(&asset).map_err(|e| invalid(e.to_string()))?;
    if let (Value::Object(p), Value::Object(b)) = (patch, &back) {
        if let Some(k) = p.iter().find(|(k, v)| !v.is_null() && !b.contains_key(*k)) {
            return Err(invalid(format!("{name} has no field {:?}", k.0)));
        }
    }
    Ok(Some(asset))
}

impl Command for SetMaterial {
    const NAME: &'static str = "set_material";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let new_color = self.color.as_deref().map(color).transpose()?;
        let s = self.clone();
        tx.modify(self.material, |e| {
            let ElementBody::Material(m) = &mut e.body else {
                return Err(CommandError::new(
                    "ferro.model.not_a_material",
                    format!("{} is not a material", e.id),
                )
                .on(e.id));
            };
            if let Some(c) = new_color {
                m.color = c;
            }
            if let Some(r) = s.roughness {
                if !(0.0..=1.0).contains(&r) {
                    return Err(invalid(format!("roughness must be in [0, 1], got {r}")));
                }
                m.roughness = r;
            }
            for a in &s.clear {
                match a {
                    MaterialAsset::Appearance => m.appearance = None,
                    MaterialAsset::Physical => m.physical = None,
                    MaterialAsset::Thermal => m.thermal = None,
                    MaterialAsset::Environmental => m.environmental = None,
                    MaterialAsset::Cost => m.cost = None,
                }
            }
            if let Some(a) = patched("appearance", &m.appearance, &s.appearance)? {
                m.appearance = Some(a);
            }
            if let Some(a) = patched("physical", &m.physical, &s.physical)? {
                m.physical = Some(a);
            }
            if let Some(a) = patched("thermal", &m.thermal, &s.thermal)? {
                m.thermal = Some(a);
            }
            if let Some(a) = patched("environmental", &m.environmental, &s.environmental)? {
                m.environmental = Some(a);
            }
            if let Some(a) = patched("cost", &m.cost, &s.cost)? {
                m.cost = Some(a);
            }
            m.validate().map_err(invalid)
        })?;
        Ok(json!({ "element": self.material }))
    }
}
