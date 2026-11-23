//! `Material` and its assets (ADR 0004 §3, D-66). A material is an element
//! (category `Materials`). Besides its display colour it may carry optional
//! assets, as Revit does (Appearance / Physical / Thermal assets):
//!
//! | Asset | Holds | Used for |
//! |---|---|---|
//! | [`AppearanceAsset`] | OpenPBR parameters beyond colour and roughness, image mappings | rendering, glTF / USD |
//! | [`PhysicalAsset`] | density, elastic moduli, strengths, thermal expansion | structure, mass |
//! | [`ThermalAsset`] | conductivity, specific heat, density, emissivity, … | U-value |
//! | [`EnvironmentalAsset`] | GWP per life-cycle stage, declared unit, EPD source | carbon |
//! | [`CostAsset`] | unit price, currency, unit | cost estimates |
//!
//! Every asset is optional and omitted from files when absent, so files from
//! milestones 2–4 (`color`, `roughness` only) load and save unchanged.
//!
//! Units: physical quantities are SI and named in the field
//! (`conductivity_w_mk` = W/(m·K)); lengths are mm (D-01). Unknown values are
//! `None`, never zero: a missing conductivity is not an insulator.

use serde::{Deserialize, Serialize};

/// A material (Revit: `Material`). `color` and `roughness` are the OpenPBR
/// `base_color` and `specular_roughness`; the other assets are optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// OpenPBR `base_color`: sRGB, each component 0..1.
    pub color: [f32; 3],
    /// OpenPBR `specular_roughness`, 0 (mirror) ..1 (matte).
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    /// Further OpenPBR parameters and image mappings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<AppearanceAsset>,
    /// Structural / mechanical data (Revit: Physical asset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical: Option<PhysicalAsset>,
    /// Thermal data (Revit: Thermal asset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<ThermalAsset>,
    /// Embodied carbon from an EPD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environmental: Option<EnvironmentalAsset>,
    /// Unit price for estimates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostAsset>,
}

/// Default `roughness` of a material created without one.
pub const DEFAULT_ROUGHNESS: f32 = 0.8;

fn default_roughness() -> f32 {
    DEFAULT_ROUGHNESS
}

impl Material {
    /// A material with only a colour (sRGB 0..1) and roughness (0..1), no assets.
    pub fn new(color: [f32; 3], roughness: f32) -> Self {
        Material {
            color,
            roughness,
            appearance: None,
            physical: None,
            thermal: None,
            environmental: None,
            cost: None,
        }
    }

    /// Density in kg/m³: the physical asset's, else the thermal asset's.
    /// `None` when neither gives one.
    pub fn density_kg_m3(&self) -> Option<f64> {
        self.physical
            .as_ref()
            .and_then(|p| p.density_kg_m3)
            .or_else(|| self.thermal.as_ref().and_then(|t| t.density_kg_m3))
    }

    /// Check value ranges. Fails with a message naming the first bad field
    /// (e.g. `"thermal.conductivity_w_mk must be > 0"`). Non-finite numbers
    /// are always rejected.
    pub fn validate(&self) -> Result<(), String> {
        let mut c = Check::default();
        for (i, v) in self.color.iter().enumerate() {
            c.range(&format!("color[{i}]"), Some(*v as f64), 0.0, 1.0);
        }
        c.range("roughness", Some(self.roughness as f64), 0.0, 1.0);
        if let Some(a) = &self.appearance {
            a.check(&mut c);
        }
        if let Some(p) = &self.physical {
            c.positive("physical.density_kg_m3", p.density_kg_m3);
            c.positive("physical.youngs_modulus_pa", p.youngs_modulus_pa);
            c.range("physical.poisson_ratio", p.poisson_ratio, -1.0, 0.5);
            c.positive(
                "physical.compressive_strength_pa",
                p.compressive_strength_pa,
            );
            c.positive("physical.tensile_strength_pa", p.tensile_strength_pa);
            c.finite(
                "physical.thermal_expansion_per_k",
                p.thermal_expansion_per_k,
            );
        }
        if let Some(t) = &self.thermal {
            c.positive("thermal.conductivity_w_mk", t.conductivity_w_mk);
            c.positive("thermal.specific_heat_j_kgk", t.specific_heat_j_kgk);
            c.positive("thermal.density_kg_m3", t.density_kg_m3);
            c.range("thermal.emissivity", t.emissivity, 0.0, 1.0);
            c.range("thermal.solar_absorptance", t.solar_absorptance, 0.0, 1.0);
            c.range(
                "thermal.vapour_resistance_factor",
                t.vapour_resistance_factor,
                1.0,
                f64::MAX,
            );
        }
        if let Some(e) = &self.environmental {
            // GWP may be negative (biogenic carbon, module D credits).
            for (k, v) in e.gwp_stages() {
                c.finite(&format!("environmental.{k}"), v);
            }
        }
        if let Some(k) = &self.cost {
            c.range("cost.unit_price", Some(k.unit_price), 0.0, f64::MAX);
            if !is_currency_code(&k.currency) {
                c.fail(format!(
                    "cost.currency must be an ISO 4217 code such as \"VND\" or \"USD\", got {:?}",
                    k.currency
                ));
            }
        }
        c.done()
    }
}

/// `true` for three upper-case ASCII letters (ISO 4217 form, e.g. `VND`).
pub fn is_currency_code(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())
}

#[derive(Default)]
struct Check(Option<String>);

impl Check {
    fn fail(&mut self, msg: String) {
        self.0.get_or_insert(msg);
    }
    fn finite(&mut self, name: &str, v: Option<f64>) {
        if let Some(v) = v.filter(|v| !v.is_finite()) {
            self.fail(format!("{name} must be a finite number, got {v}"));
        }
    }
    fn range(&mut self, name: &str, v: Option<f64>, lo: f64, hi: f64) {
        if let Some(v) = v {
            if !v.is_finite() || v < lo || v > hi {
                let hi = if hi >= f64::MAX {
                    "∞".to_string()
                } else {
                    hi.to_string()
                };
                self.fail(format!("{name} must be in [{lo}, {hi}], got {v}"));
            }
        }
    }
    fn positive(&mut self, name: &str, v: Option<f64>) {
        if let Some(v) = v {
            if !v.is_finite() || v <= 0.0 {
                self.fail(format!("{name} must be > 0, got {v}"));
            }
        }
    }
    fn done(self) -> Result<(), String> {
        match self.0 {
            Some(m) => Err(m),
            None => Ok(()),
        }
    }
}

// ------------------------------------------------------------ appearance

/// OpenPBR Surface parameters (OpenPBR 1.1, Academy Software Foundation)
/// beyond `Material::color` (= `base_color`) and `Material::roughness`
/// (= `specular_roughness`). Each field names the OpenPBR parameter it maps
/// to; defaults are OpenPBR's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceAsset {
    /// OpenPBR `base_metalness`, 0 (dielectric) ..1 (metal).
    #[serde(default)]
    pub base_metalness: f64,
    /// OpenPBR `specular_ior`, index of refraction (default 1.5).
    #[serde(default = "default_ior")]
    pub specular_ior: f64,
    /// OpenPBR `transmission_weight`, 0..1 (glass ≈ 1).
    #[serde(default)]
    pub transmission_weight: f64,
    /// OpenPBR `emission_color`, sRGB 0..1.
    #[serde(default = "white")]
    pub emission_color: [f32; 3],
    /// OpenPBR `emission_luminance` in cd/m² (nits); 0 = not emissive.
    #[serde(default)]
    pub emission_luminance: f64,
    /// OpenPBR `geometry_opacity`, 0 (invisible) ..1 (opaque).
    #[serde(default = "one")]
    pub geometry_opacity: f64,
    /// OpenPBR `coat_weight`, 0..1 (varnish, lacquer).
    #[serde(default)]
    pub coat_weight: f64,
    /// Image for OpenPBR `base_color` (placeholder: stored, not yet rendered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color_image: Option<ImageMapping>,
    /// Normal map for OpenPBR `geometry_normal` (placeholder: stored, not yet rendered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_image: Option<ImageMapping>,
}

fn default_ior() -> f64 {
    1.5
}
fn white() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn one() -> f64 {
    1.0
}

impl Default for AppearanceAsset {
    fn default() -> Self {
        AppearanceAsset {
            base_metalness: 0.0,
            specular_ior: default_ior(),
            transmission_weight: 0.0,
            emission_color: white(),
            emission_luminance: 0.0,
            geometry_opacity: 1.0,
            coat_weight: 0.0,
            base_color_image: None,
            normal_image: None,
        }
    }
}

impl AppearanceAsset {
    fn check(&self, c: &mut Check) {
        c.range(
            "appearance.base_metalness",
            Some(self.base_metalness),
            0.0,
            1.0,
        );
        c.range("appearance.specular_ior", Some(self.specular_ior), 1.0, 5.0);
        c.range(
            "appearance.transmission_weight",
            Some(self.transmission_weight),
            0.0,
            1.0,
        );
        for (i, v) in self.emission_color.iter().enumerate() {
            c.range(
                &format!("appearance.emission_color[{i}]"),
                Some(*v as f64),
                0.0,
                1.0,
            );
        }
        c.range(
            "appearance.emission_luminance",
            Some(self.emission_luminance),
            0.0,
            f64::MAX,
        );
        c.range(
            "appearance.geometry_opacity",
            Some(self.geometry_opacity),
            0.0,
            1.0,
        );
        c.range("appearance.coat_weight", Some(self.coat_weight), 0.0, 1.0);
        for (name, img) in [
            ("appearance.base_color_image", &self.base_color_image),
            ("appearance.normal_image", &self.normal_image),
        ] {
            if let Some(img) = img {
                if img.path.trim().is_empty() {
                    c.fail(format!("{name}.path must not be empty"));
                }
                c.positive(&format!("{name}.width_mm"), Some(img.width_mm));
                c.positive(&format!("{name}.height_mm"), Some(img.height_mm));
                c.finite(&format!("{name}.rotation_rad"), Some(img.rotation_rad));
            }
        }
    }
}

/// How an image is laid on faces: real-world size of one tile (Revit:
/// texture "Scale"; SketchUp: texture size), so a 600 × 600 mm tile image
/// repeats every 600 mm whatever the face size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageMapping {
    /// Image file path, relative to the project file or absolute.
    pub path: String,
    /// Real-world width of one tile (mm, > 0).
    pub width_mm: f64,
    /// Real-world height of one tile (mm, > 0).
    pub height_mm: f64,
    /// Rotation of the tile on the face (radians, counter-clockwise).
    #[serde(default)]
    pub rotation_rad: f64,
}

// --------------------------------------------------------------- physical

/// Mechanical data (Revit: Physical / structural asset; IFC:
/// `Pset_MaterialCommon`, `Pset_MaterialMechanical`). SI units.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalAsset {
    /// Density ρ (kg/m³, > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_kg_m3: Option<f64>,
    /// Young's modulus E (Pa, > 0); concrete C25/30 ≈ 31e9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youngs_modulus_pa: Option<f64>,
    /// Poisson ratio ν (dimensionless, −1..0.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poisson_ratio: Option<f64>,
    /// Characteristic compressive strength (Pa, > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compressive_strength_pa: Option<f64>,
    /// Characteristic tensile strength (Pa, > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tensile_strength_pa: Option<f64>,
    /// Linear thermal expansion coefficient α (1/K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_expansion_per_k: Option<f64>,
}

// ---------------------------------------------------------------- thermal

/// Thermal data (Revit: Thermal asset; IFC: `Pset_MaterialThermal`). SI units.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThermalAsset {
    /// Design thermal conductivity λ (W/(m·K), > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conductivity_w_mk: Option<f64>,
    /// Specific heat capacity c (J/(kg·K), > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specific_heat_j_kgk: Option<f64>,
    /// Density ρ (kg/m³, > 0), used when the physical asset has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density_kg_m3: Option<f64>,
    /// Long-wave (thermal) emissivity ε, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissivity: Option<f64>,
    /// Solar absorptance α, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solar_absorptance: Option<f64>,
    /// Water vapour resistance factor μ (dimensionless, ≥ 1; air = 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vapour_resistance_factor: Option<f64>,
}

// ---------------------------------------------------------- environmental

/// Unit a GWP figure or a price refers to (EPD "declared unit").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialUnit {
    /// Per cubic metre of material.
    #[serde(rename = "m3")]
    CubicMetre,
    /// Per kilogram of material (needs a density for area quantities).
    #[serde(rename = "kg")]
    Kilogram,
    /// Per square metre of layer, whatever its thickness (membranes, boards).
    #[serde(rename = "m2")]
    SquareMetre,
}

impl MaterialUnit {
    /// Stable short name: `m3`, `kg` or `m2`.
    pub fn as_str(self) -> &'static str {
        match self {
            MaterialUnit::CubicMetre => "m3",
            MaterialUnit::Kilogram => "kg",
            MaterialUnit::SquareMetre => "m2",
        }
    }
}

/// Embodied carbon from an EPD (EN 15804 life-cycle modules). Global warming
/// potential (GWP-total) in kg CO₂e per `declared_unit`. Stages that the EPD
/// does not declare stay `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentalAsset {
    /// What one GWP figure refers to (1 m³, 1 kg or 1 m²).
    pub declared_unit: MaterialUnit,
    /// Product stage A1–A3 (raw material, transport, manufacturing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_a1_a3: Option<f64>,
    /// A4 transport to site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_a4: Option<f64>,
    /// A5 construction / installation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_a5: Option<f64>,
    /// B1–B7 use stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_b: Option<f64>,
    /// C1–C4 end of life.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_c: Option<f64>,
    /// D benefits beyond the system boundary (usually negative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gwp_d: Option<f64>,
    /// Where the figures come from: EPD number, programme operator, database
    /// (e.g. "EPD-IES-0001234, Environdec, 2025").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epd_source: Option<String>,
}

impl EnvironmentalAsset {
    /// The stages as `(key, value)` pairs in life-cycle order; keys are the
    /// field names (`gwp_a1_a3`, …).
    pub fn gwp_stages(&self) -> [(&'static str, Option<f64>); 6] {
        [
            ("gwp_a1_a3", self.gwp_a1_a3),
            ("gwp_a4", self.gwp_a4),
            ("gwp_a5", self.gwp_a5),
            ("gwp_b", self.gwp_b),
            ("gwp_c", self.gwp_c),
            ("gwp_d", self.gwp_d),
        ]
    }
}

// ------------------------------------------------------------------- cost

/// Unit price for estimates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostAsset {
    /// Price of one `unit` of material, in `currency` (≥ 0).
    pub unit_price: f64,
    /// ISO 4217 currency code, e.g. `VND`, `USD`.
    pub currency: String,
    /// What `unit_price` is for (1 m³, 1 kg or 1 m²).
    pub unit: MaterialUnit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_material_json_loads_and_saves_unchanged() {
        let old = r#"{"color":[0.5,0.25,0.125],"roughness":0.6}"#;
        let m: Material = serde_json::from_str(old).unwrap();
        assert!(m.appearance.is_none() && m.thermal.is_none() && m.cost.is_none());
        assert_eq!(serde_json::to_string(&m).unwrap(), old);
        // Roughness was optional in M2 files.
        let m: Material = serde_json::from_str(r#"{"color":[1,1,1]}"#).unwrap();
        assert!((m.roughness - DEFAULT_ROUGHNESS).abs() < 1e-6);
    }

    #[test]
    fn assets_round_trip_with_defaults() {
        let json = r#"{"color":[1,1,1],"appearance":{"base_metalness":1.0},
            "thermal":{"conductivity_w_mk":0.8},
            "environmental":{"declared_unit":"m3","gwp_a1_a3":250.0},
            "cost":{"unit_price":1200000,"currency":"VND","unit":"m3"}}"#;
        let m: Material = serde_json::from_str(json).unwrap();
        let a = m.appearance.as_ref().unwrap();
        assert!((a.specular_ior - 1.5).abs() < 1e-12);
        assert!((a.geometry_opacity - 1.0).abs() < 1e-12);
        assert_eq!(
            m.environmental.as_ref().unwrap().declared_unit,
            MaterialUnit::CubicMetre
        );
        let back: Material = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_values() {
        let mut m = Material::new([0.5; 3], 0.5);
        m.thermal = Some(ThermalAsset {
            conductivity_w_mk: Some(0.0),
            ..Default::default()
        });
        assert!(m.validate().unwrap_err().contains("conductivity_w_mk"));
        m.thermal = Some(ThermalAsset {
            conductivity_w_mk: Some(f64::NAN),
            ..Default::default()
        });
        assert!(m.validate().is_err());
        m.thermal = None;
        m.cost = Some(CostAsset {
            unit_price: 10.0,
            currency: "dong".into(),
            unit: MaterialUnit::SquareMetre,
        });
        assert!(m.validate().unwrap_err().contains("currency"));
        m.cost = None;
        m.physical = Some(PhysicalAsset {
            poisson_ratio: Some(0.6),
            ..Default::default()
        });
        assert!(m.validate().unwrap_err().contains("poisson_ratio"));
    }

    #[test]
    fn validate_range_edges_are_inclusive() {
        let mut m = Material::new([0.0, 1.0, 0.0], 1.0);
        m.thermal = Some(ThermalAsset {
            emissivity: Some(1.0),
            solar_absorptance: Some(0.0),
            vapour_resistance_factor: Some(1.0),
            ..Default::default()
        });
        m.physical = Some(PhysicalAsset {
            poisson_ratio: Some(0.5),
            ..Default::default()
        });
        assert!(m.validate().is_ok());
        m.thermal.as_mut().unwrap().conductivity_w_mk = Some(1e-12);
        assert!(
            m.validate().is_ok(),
            "tiny positive conductivity is allowed"
        );
    }

    #[test]
    fn density_prefers_physical_asset() {
        let mut m = Material::new([0.5; 3], 0.5);
        assert!(m.density_kg_m3().is_none());
        m.thermal = Some(ThermalAsset {
            density_kg_m3: Some(1800.0),
            ..Default::default()
        });
        assert!((m.density_kg_m3().unwrap() - 1800.0).abs() < 1e-9);
        m.physical = Some(PhysicalAsset {
            density_kg_m3: Some(2400.0),
            ..Default::default()
        });
        assert!((m.density_kg_m3().unwrap() - 2400.0).abs() < 1e-9);
    }
}
