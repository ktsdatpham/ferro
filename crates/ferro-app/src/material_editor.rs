//! Minimal material editor for the Materials panel (ADR 0004 §3): colour,
//! roughness and the thermal / physical / environmental / cost numbers.
//! The panel edits a draft; Apply turns it into one `set_material` command
//! (and `set_name` if the name changed). Values are typed in the units shown
//! and converted to the stored SI units here, at the UI edge.

use eframe::egui;
use ferro_core::model::material::Material;
use ferro_core::ops::material_commands::{MaterialAsset, SetMaterial};
use ferro_core::ElementId;
use serde_json::{Map, Value};

/// One numeric field: label, unit shown, stored key, factor display → SI.
struct Field {
    label: &'static str,
    unit: &'static str,
    key: &'static str,
    to_si: f64,
}

const fn field(label: &'static str, unit: &'static str, key: &'static str, to_si: f64) -> Field {
    Field {
        label,
        unit,
        key,
        to_si,
    }
}

const THERMAL: [Field; 6] = [
    field("Conductivity λ", "W/(m·K)", "conductivity_w_mk", 1.0),
    field("Specific heat", "J/(kg·K)", "specific_heat_j_kgk", 1.0),
    field("Density", "kg/m³", "density_kg_m3", 1.0),
    field("Emissivity", "0–1", "emissivity", 1.0),
    field("Solar absorptance", "0–1", "solar_absorptance", 1.0),
    field("Vapour resistance μ", "", "vapour_resistance_factor", 1.0),
];

const PHYSICAL: [Field; 6] = [
    field("Density", "kg/m³", "density_kg_m3", 1.0),
    field("Young's modulus", "GPa", "youngs_modulus_pa", 1e9),
    field("Poisson ratio", "", "poisson_ratio", 1.0),
    field(
        "Compressive strength",
        "MPa",
        "compressive_strength_pa",
        1e6,
    ),
    field("Tensile strength", "MPa", "tensile_strength_pa", 1e6),
    field("Thermal expansion", "1/K", "thermal_expansion_per_k", 1.0),
];

/// Declared / price units offered in the editor (stable keys).
const UNITS: [(&str, &str); 3] = [("m3", "per m³"), ("kg", "per kg"), ("m2", "per m²")];

/// What the user did in the editor this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftAction {
    None,
    Apply,
    Close,
}

/// Editable copy of a material; numbers are kept as typed text.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialDraft {
    pub material: ElementId,
    pub name: String,
    pub color: [f32; 3],
    pub roughness: f32,
    thermal: [String; 6],
    physical: [String; 6],
    gwp_a1_a3: String,
    gwp_unit: String,
    epd_source: String,
    price: String,
    currency: String,
    price_unit: String,
}

fn text(v: Option<f64>, to_si: f64) -> String {
    v.map(|x| format!("{}", x / to_si)).unwrap_or_default()
}

/// Read an asset (as stored JSON) into the text fields of `fields`.
fn fields_text(asset: Option<Value>, fields: &[Field; 6]) -> [String; 6] {
    std::array::from_fn(|i| {
        let v = asset
            .as_ref()
            .and_then(|a| a.get(fields[i].key))
            .and_then(Value::as_f64);
        text(v, fields[i].to_si)
    })
}

/// Parse a number typed by the user; empty = absent. Accepts a decimal comma.
fn number(label: &str, s: &str) -> Result<Option<f64>, String> {
    let t = s.trim().replace(',', ".");
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(Some)
        .ok_or_else(|| format!("{label}: {s:?} is not a number"))
}

impl MaterialDraft {
    /// A draft showing the current values of material `id`.
    pub fn from_material(id: ElementId, name: String, m: &Material) -> Self {
        let env = m.environmental.as_ref();
        let cost = m.cost.as_ref();
        MaterialDraft {
            material: id,
            name,
            color: m.color,
            roughness: m.roughness,
            thermal: fields_text(
                m.thermal
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok()),
                &THERMAL,
            ),
            physical: fields_text(
                m.physical
                    .as_ref()
                    .and_then(|a| serde_json::to_value(a).ok()),
                &PHYSICAL,
            ),
            gwp_a1_a3: text(env.and_then(|e| e.gwp_a1_a3), 1.0),
            gwp_unit: env.map(|e| e.declared_unit.as_str()).unwrap_or("m3").into(),
            epd_source: env.and_then(|e| e.epd_source.clone()).unwrap_or_default(),
            price: cost
                .map(|c| format!("{}", c.unit_price))
                .unwrap_or_default(),
            currency: cost
                .map(|c| c.currency.clone())
                .unwrap_or_else(|| "VND".into()),
            price_unit: cost.map(|c| c.unit.as_str()).unwrap_or("m3").into(),
        }
    }

    /// Patch of an asset from text fields: typed values set, empty ones
    /// cleared (`null`). `None` when every field is empty (asset removed).
    fn patch(texts: &[String; 6], fields: &[Field; 6]) -> Result<Option<Value>, String> {
        let mut obj = Map::new();
        let mut any = false;
        for (t, f) in texts.iter().zip(fields) {
            let v = number(f.label, t)?;
            any |= v.is_some();
            obj.insert(
                f.key.into(),
                v.map(|x| Value::from(x * f.to_si)).unwrap_or(Value::Null),
            );
        }
        Ok(any.then_some(Value::Object(obj)))
    }

    /// The `set_material` command for this draft. Fails with a message
    /// when a number cannot be read (the kernel checks value ranges).
    pub fn to_command(&self) -> Result<SetMaterial, String> {
        let [r, g, b] = self
            .color
            .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let mut clear = Vec::new();
        let thermal = Self::patch(&self.thermal, &THERMAL)?;
        if thermal.is_none() {
            clear.push(MaterialAsset::Thermal);
        }
        let physical = Self::patch(&self.physical, &PHYSICAL)?;
        if physical.is_none() {
            clear.push(MaterialAsset::Physical);
        }
        let environmental = match number("GWP A1–A3", &self.gwp_a1_a3)? {
            Some(g) => {
                let epd = self.epd_source.trim();
                Some(serde_json::json!({
                    "declared_unit": self.gwp_unit,
                    "gwp_a1_a3": g,
                    "epd_source": if epd.is_empty() { Value::Null } else { Value::from(epd) },
                }))
            }
            None => {
                clear.push(MaterialAsset::Environmental);
                None
            }
        };
        let cost = match number("Unit price", &self.price)? {
            Some(p) => Some(serde_json::json!({
                "unit_price": p,
                "currency": self.currency.trim().to_uppercase(),
                "unit": self.price_unit,
            })),
            None => {
                clear.push(MaterialAsset::Cost);
                None
            }
        };
        Ok(SetMaterial {
            material: self.material,
            color: Some(format!("#{r:02x}{g:02x}{b:02x}")),
            roughness: Some(self.roughness.clamp(0.0, 1.0)),
            appearance: None,
            physical,
            thermal,
            environmental,
            cost,
            clear,
        })
    }

    /// Draw the editor; returns what the user clicked.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> DraftAction {
        let mut action = DraftAction::None;
        let num_row = |ui: &mut egui::Ui, label: &str, unit: &str, s: &mut String| {
            ui.label(label);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(s).desired_width(80.0));
                ui.label(egui::RichText::new(unit).weak());
            });
            ui.end_row();
        };
        let unit_combo = |ui: &mut egui::Ui, salt: &str, unit: &mut String| {
            let shown = UNITS
                .iter()
                .find(|(k, _)| *k == unit.as_str())
                .map(|(_, l)| *l)
                .unwrap_or("");
            egui::ComboBox::from_id_salt(salt)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (k, l) in UNITS {
                        ui.selectable_value(unit, k.to_string(), l);
                    }
                });
        };
        egui::Grid::new("material_editor")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label("Colour");
                ui.color_edit_button_rgb(&mut self.color);
                ui.end_row();
                ui.label("Roughness");
                ui.add(egui::Slider::new(&mut self.roughness, 0.0..=1.0));
                ui.end_row();

                ui.label(egui::RichText::new("Thermal").strong());
                ui.end_row();
                for (f, s) in THERMAL.iter().zip(self.thermal.iter_mut()) {
                    num_row(ui, f.label, f.unit, s);
                }
                ui.label(egui::RichText::new("Physical").strong());
                ui.end_row();
                for (f, s) in PHYSICAL.iter().zip(self.physical.iter_mut()) {
                    num_row(ui, f.label, f.unit, s);
                }
                ui.label(egui::RichText::new("Environmental").strong());
                ui.end_row();
                ui.label("GWP A1–A3");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.gwp_a1_a3).desired_width(80.0));
                    ui.label(egui::RichText::new("kg CO₂e").weak());
                    unit_combo(ui, "gwp_unit", &mut self.gwp_unit);
                });
                ui.end_row();
                ui.label("EPD source");
                ui.text_edit_singleline(&mut self.epd_source);
                ui.end_row();
                ui.label(egui::RichText::new("Cost").strong());
                ui.end_row();
                ui.label("Unit price");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.price).desired_width(80.0));
                    ui.add(egui::TextEdit::singleline(&mut self.currency).desired_width(36.0));
                    unit_combo(ui, "price_unit", &mut self.price_unit);
                });
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                action = DraftAction::Apply;
            }
            if ui.button("Close").clicked() {
                action = DraftAction::Close;
            }
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::model::material::{CostAsset, MaterialUnit, PhysicalAsset, ThermalAsset};

    fn brick() -> Material {
        let mut m = Material::new([1.0, 0.5, 0.0], 0.9);
        m.thermal = Some(ThermalAsset {
            conductivity_w_mk: Some(0.77),
            ..Default::default()
        });
        m.physical = Some(PhysicalAsset {
            youngs_modulus_pa: Some(5e9),
            ..Default::default()
        });
        m.cost = Some(CostAsset {
            unit_price: 1500.0,
            currency: "VND".into(),
            unit: MaterialUnit::CubicMetre,
        });
        m
    }

    #[test]
    fn draft_shows_display_units_and_round_trips() {
        let d = MaterialDraft::from_material(ElementId(3), "Brick".into(), &brick());
        assert_eq!(d.thermal[0], "0.77");
        assert_eq!(d.physical[1], "5", "Young's modulus shown in GPa");
        let c = d.to_command().unwrap();
        assert_eq!(c.color.as_deref(), Some("#ff8000"));
        let e = c.physical.unwrap()["youngs_modulus_pa"].as_f64().unwrap();
        assert!((e - 5e9).abs() < 1e-3);
        let t = c.thermal.unwrap();
        assert!(t["specific_heat_j_kgk"].is_null(), "empty field clears");
        assert_eq!(c.clear, vec![MaterialAsset::Environmental]);
        assert_eq!(c.cost.unwrap()["unit"], "m3");
    }

    #[test]
    fn emptied_asset_is_cleared_and_bad_numbers_fail() {
        let mut d = MaterialDraft::from_material(ElementId(3), "Brick".into(), &brick());
        d.thermal[0].clear();
        d.price = " ".into();
        let c = d.to_command().unwrap();
        assert!(c.thermal.is_none() && c.cost.is_none());
        assert!(c.clear.contains(&MaterialAsset::Thermal));
        assert!(c.clear.contains(&MaterialAsset::Cost));
        d.physical[0] = "1,8e3".into();
        let c = d.to_command().unwrap();
        let rho = c.physical.unwrap()["density_kg_m3"].as_f64().unwrap();
        assert!((rho - 1800.0).abs() < 1e-9, "decimal comma accepted");
        d.physical[0] = "heavy".into();
        assert!(d.to_command().unwrap_err().contains("Density"));
    }
}
