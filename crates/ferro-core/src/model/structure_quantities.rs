//! Quantities of a wall / floor type derived from its `CompoundStructure`
//! and the assets of its layer materials (ADR 0004 §3): thermal
//! transmittance (U-value), mass, embodied carbon and cost, all per m² of
//! wall / floor.
//!
//! These are computed when read, not stored: they depend on other elements
//! (the materials), so storing them on the type would go stale when a
//! material changes. A quantity is only given when **every** layer has the
//! data it needs; otherwise it is `None` and [`StructureQuantities::gaps`]
//! says which layer lacks what. Nothing is silently counted as zero.

use super::bim::CompoundStructure;
use super::material::{Material, MaterialUnit};
use super::{Document, ElementBody, ElementId};
use crate::math::tol;
use serde::{Deserialize, Serialize};

/// External surface resistance Rse (m²·K/W), ISO 6946:2017 Table 7, any
/// heat-flow direction, surface exposed to outside air.
pub const RSE_M2K_W: f64 = 0.04;

/// Direction of heat flow through the element, which sets the internal
/// surface resistance Rsi (ISO 6946:2017 Table 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeatFlow {
    /// Walls (within ±30° of horizontal): Rsi = 0.13 m²·K/W.
    Horizontal,
    /// Roofs, ceilings with heat flowing up: Rsi = 0.10 m²·K/W.
    Upward,
    /// Floors with heat flowing down: Rsi = 0.17 m²·K/W.
    Downward,
}

impl HeatFlow {
    /// Internal surface resistance Rsi (m²·K/W), ISO 6946:2017 Table 7.
    pub fn rsi_m2k_w(self) -> f64 {
        match self {
            HeatFlow::Horizontal => 0.13,
            HeatFlow::Upward => 0.10,
            HeatFlow::Downward => 0.17,
        }
    }
}

/// A quantity derived from the layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureQuantity {
    UValue,
    Mass,
    Gwp,
    Cost,
}

/// Why a layer could not contribute to a quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    /// The layer has no material, or its id is not a material.
    NoMaterial,
    /// The material has no thermal conductivity.
    NoConductivity,
    /// The material has no density (physical or thermal asset).
    NoDensity,
    /// The material has no environmental asset with an A1–A3 figure.
    NoGwp,
    /// The material has no cost asset.
    NoCost,
    /// The material's price is in another currency than an earlier layer's.
    CurrencyMismatch,
}

/// One layer that lacks data for one quantity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerGap {
    /// Index of the layer in the structure (0 = exterior / top).
    pub layer: usize,
    /// The layer's material, if any.
    pub material: Option<ElementId>,
    pub quantity: StructureQuantity,
    pub reason: GapReason,
}

/// Per-m² quantities of a compound structure. Each value is `None` unless
/// every layer had the data for it; see `gaps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureQuantities {
    /// Heat-flow direction used for Rsi.
    pub heat_flow: HeatFlow,
    /// Total thermal resistance Rsi + Σ dᵢ/λᵢ + Rse (m²·K/W).
    pub thermal_resistance_m2k_w: Option<f64>,
    /// Thermal transmittance U = 1 / R_total (W/(m²·K)), ISO 6946 for
    /// homogeneous plane layers (no thermal bridges, air layers or
    /// fixings corrections).
    pub u_value_w_m2k: Option<f64>,
    /// Mass per m² of element, Σ ρᵢ·dᵢ (kg/m²).
    pub mass_kg_m2: Option<f64>,
    /// Embodied carbon, product stage A1–A3, per m² of element (kg CO₂e/m²).
    pub gwp_a1_a3_kgco2e_m2: Option<f64>,
    /// Material cost per m² of element, in `currency`.
    pub cost_per_m2: Option<f64>,
    /// ISO 4217 currency of `cost_per_m2` (that of the first priced layer).
    pub currency: Option<String>,
    /// Layers that lack data, in layer order.
    pub gaps: Vec<LayerGap>,
}

impl StructureQuantities {
    /// Compute the quantities of `structure`, looking layer materials up with
    /// `material`. Layer thickness d is in mm (converted to m here).
    ///
    /// Layers thinner than `tol::VERTEX_TOL` count as zero thickness: they
    /// add no thermal resistance or mass and need no data for those, but a
    /// per-m² GWP or price still applies to them (membranes). Conductivity
    /// must be > 0 (enforced by `Material::validate`).
    pub fn from_structure<'a>(
        structure: &CompoundStructure,
        heat_flow: HeatFlow,
        material: impl Fn(ElementId) -> Option<&'a Material>,
    ) -> Self {
        let mut gaps = Vec::new();
        let mut r_layers = Some(0.0);
        let mut mass = Some(0.0);
        let mut gwp = Some(0.0);
        let mut cost = Some(0.0);
        let mut currency: Option<String> = None;

        // Mark a quantity incomplete and record why.
        fn gap(
            gaps: &mut Vec<LayerGap>,
            total: &mut Option<f64>,
            layer: usize,
            material: Option<ElementId>,
            quantity: StructureQuantity,
            reason: GapReason,
        ) {
            *total = None;
            gaps.push(LayerGap {
                layer,
                material,
                quantity,
                reason,
            });
        }
        fn add(total: &mut Option<f64>, v: f64) {
            if let Some(t) = total {
                *t += v;
            }
        }

        for (i, layer) in structure.layers.iter().enumerate() {
            let thin = tol::is_zero_len(layer.thickness.mm());
            let d_m = if thin {
                0.0
            } else {
                layer.thickness.mm() / 1000.0
            };
            let id = layer.material;
            let Some(m) = id.and_then(&material) else {
                use StructureQuantity::*;
                let needs: &[StructureQuantity] = if thin {
                    &[Gwp, Cost]
                } else {
                    &[UValue, Mass, Gwp, Cost]
                };
                for q in needs {
                    let total = match q {
                        UValue => &mut r_layers,
                        Mass => &mut mass,
                        Gwp => &mut gwp,
                        Cost => &mut cost,
                    };
                    gap(&mut gaps, total, i, id, *q, GapReason::NoMaterial);
                }
                continue;
            };
            let density = m.density_kg_m3();

            // Thermal resistance d / λ.
            if !thin {
                match m.thermal.as_ref().and_then(|t| t.conductivity_w_mk) {
                    Some(k) if k > 0.0 => add(&mut r_layers, d_m / k),
                    _ => gap(
                        &mut gaps,
                        &mut r_layers,
                        i,
                        id,
                        StructureQuantity::UValue,
                        GapReason::NoConductivity,
                    ),
                }
                match density {
                    Some(rho) => add(&mut mass, rho * d_m),
                    None => gap(
                        &mut gaps,
                        &mut mass,
                        i,
                        id,
                        StructureQuantity::Mass,
                        GapReason::NoDensity,
                    ),
                }
            }

            // Amount of declared units per m² of element.
            let per_m2 = |unit: MaterialUnit| -> Result<f64, GapReason> {
                match unit {
                    MaterialUnit::SquareMetre => Ok(1.0),
                    MaterialUnit::CubicMetre => Ok(d_m),
                    MaterialUnit::Kilogram if thin => Ok(0.0),
                    MaterialUnit::Kilogram => density.map(|r| r * d_m).ok_or(GapReason::NoDensity),
                }
            };

            match m
                .environmental
                .as_ref()
                .and_then(|e| e.gwp_a1_a3.map(|g| (g, e.declared_unit)))
            {
                Some((g, unit)) => match per_m2(unit) {
                    Ok(n) => add(&mut gwp, g * n),
                    Err(r) => gap(&mut gaps, &mut gwp, i, id, StructureQuantity::Gwp, r),
                },
                None => gap(
                    &mut gaps,
                    &mut gwp,
                    i,
                    id,
                    StructureQuantity::Gwp,
                    GapReason::NoGwp,
                ),
            }

            match &m.cost {
                Some(c) => {
                    if currency.as_deref().is_some_and(|cur| cur != c.currency) {
                        gap(
                            &mut gaps,
                            &mut cost,
                            i,
                            id,
                            StructureQuantity::Cost,
                            GapReason::CurrencyMismatch,
                        );
                    } else {
                        currency.get_or_insert_with(|| c.currency.clone());
                        match per_m2(c.unit) {
                            Ok(n) => add(&mut cost, c.unit_price * n),
                            Err(r) => gap(&mut gaps, &mut cost, i, id, StructureQuantity::Cost, r),
                        }
                    }
                }
                None => gap(
                    &mut gaps,
                    &mut cost,
                    i,
                    id,
                    StructureQuantity::Cost,
                    GapReason::NoCost,
                ),
            }
        }

        let r_total = r_layers.map(|r| heat_flow.rsi_m2k_w() + r + RSE_M2K_W);
        StructureQuantities {
            heat_flow,
            thermal_resistance_m2k_w: r_total,
            // R_total ≥ Rsi + Rse > 0, so the division is safe.
            u_value_w_m2k: r_total.map(|r| 1.0 / r),
            mass_kg_m2: mass,
            gwp_a1_a3_kgco2e_m2: gwp,
            cost_per_m2: cost,
            currency: if cost.is_some() { currency } else { None },
            gaps,
        }
    }

    /// Quantities of a wall type (heat flow horizontal) or floor type (heat
    /// flow downward) in `doc`. `None` if `type_id` is not a wall / floor type.
    pub fn from_type(doc: &Document, type_id: ElementId) -> Option<Self> {
        let (structure, flow) = match &doc.get(type_id)?.body {
            ElementBody::WallType(t) => (&t.structure, HeatFlow::Horizontal),
            ElementBody::FloorType(t) => (&t.structure, HeatFlow::Downward),
            _ => return None,
        };
        Some(Self::from_structure(structure, flow, |id| {
            match doc.get(id).map(|e| &e.body) {
                Some(ElementBody::Material(m)) => Some(&**m),
                _ => None,
            }
        }))
    }

    /// `true` if every quantity could be computed.
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Length;
    use crate::model::bim::{CompoundLayer, LayerFunction};
    use crate::model::material::{CostAsset, EnvironmentalAsset, PhysicalAsset, ThermalAsset};
    use std::collections::HashMap;

    fn near(a: Option<f64>, b: f64, eps: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() <= eps)
    }

    fn layer(mm: f64, m: Option<u64>) -> CompoundLayer {
        CompoundLayer {
            function: LayerFunction::Structure,
            thickness: Length::from_mm(mm),
            material: m.map(ElementId),
        }
    }

    /// Brick (λ 0.77, ρ 1800) and mineral wool (λ 0.035, ρ 30), with GWP
    /// and prices in VND.
    fn library() -> HashMap<ElementId, Material> {
        let mut brick = Material::new([0.7, 0.3, 0.2], 0.9);
        brick.thermal = Some(ThermalAsset {
            conductivity_w_mk: Some(0.77),
            ..Default::default()
        });
        brick.physical = Some(PhysicalAsset {
            density_kg_m3: Some(1800.0),
            ..Default::default()
        });
        brick.environmental = Some(EnvironmentalAsset {
            declared_unit: MaterialUnit::Kilogram,
            gwp_a1_a3: Some(0.24),
            gwp_a4: None,
            gwp_a5: None,
            gwp_b: None,
            gwp_c: None,
            gwp_d: None,
            epd_source: None,
        });
        brick.cost = Some(CostAsset {
            unit_price: 1_500_000.0,
            currency: "VND".into(),
            unit: MaterialUnit::CubicMetre,
        });
        let mut wool = Material::new([0.9, 0.9, 0.5], 1.0);
        wool.thermal = Some(ThermalAsset {
            conductivity_w_mk: Some(0.035),
            density_kg_m3: Some(30.0),
            ..Default::default()
        });
        wool.environmental = Some(EnvironmentalAsset {
            declared_unit: MaterialUnit::SquareMetre,
            gwp_a1_a3: Some(2.5),
            gwp_a4: None,
            gwp_a5: None,
            gwp_b: None,
            gwp_c: None,
            gwp_d: None,
            epd_source: Some("test".into()),
        });
        wool.cost = Some(CostAsset {
            unit_price: 90_000.0,
            currency: "VND".into(),
            unit: MaterialUnit::SquareMetre,
        });
        HashMap::from([(ElementId(1), brick), (ElementId(2), wool)])
    }

    fn compute(
        layers: Vec<CompoundLayer>,
        lib: &HashMap<ElementId, Material>,
    ) -> StructureQuantities {
        StructureQuantities::from_structure(
            &CompoundStructure { layers },
            HeatFlow::Horizontal,
            |id| lib.get(&id),
        )
    }

    #[test]
    fn two_layer_wall_matches_hand_calculation() {
        // 200 mm brick + 100 mm mineral wool, horizontal heat flow:
        // R = 0.13 + 0.2/0.77 + 0.1/0.035 + 0.04
        //   = 0.13 + 0.259740 + 2.857143 + 0.04 = 3.286883 m²K/W
        // U = 1 / 3.286883 = 0.304240 W/m²K
        // mass = 1800·0.2 + 30·0.1 = 363 kg/m²
        // GWP = 0.24 kg/kg · 360 kg + 2.5 /m² = 86.4 + 2.5 = 88.9 kgCO2e/m²
        // cost = 1 500 000 · 0.2 + 90 000 = 390 000 VND/m²
        let lib = library();
        let q = compute(vec![layer(200.0, Some(1)), layer(100.0, Some(2))], &lib);
        assert!(q.is_complete(), "{:?}", q.gaps);
        assert!(near(q.thermal_resistance_m2k_w, 3.286883, 1e-6));
        assert!(near(q.u_value_w_m2k, 0.304240, 1e-6));
        assert!(near(q.mass_kg_m2, 363.0, 1e-9));
        assert!(near(q.gwp_a1_a3_kgco2e_m2, 88.9, 1e-9));
        assert!(near(q.cost_per_m2, 390_000.0, 1e-6));
        assert_eq!(q.currency.as_deref(), Some("VND"));
    }

    #[test]
    fn heat_flow_changes_internal_surface_resistance() {
        let lib = library();
        let st = CompoundStructure {
            layers: vec![layer(200.0, Some(1))],
        };
        let r = |f| {
            StructureQuantities::from_structure(&st, f, |id| lib.get(&id))
                .thermal_resistance_m2k_w
                .unwrap()
        };
        assert!((r(HeatFlow::Downward) - r(HeatFlow::Horizontal) - 0.04).abs() < 1e-12);
        assert!((r(HeatFlow::Horizontal) - r(HeatFlow::Upward) - 0.03).abs() < 1e-12);
    }

    #[test]
    fn layer_without_conductivity_is_reported_not_zero() {
        let mut lib = library();
        lib.get_mut(&ElementId(2)).unwrap().thermal = None;
        let q = compute(vec![layer(200.0, Some(1)), layer(100.0, Some(2))], &lib);
        assert!(q.u_value_w_m2k.is_none());
        assert!(q.thermal_resistance_m2k_w.is_none());
        // Wool had its density only in the thermal asset.
        assert!(q.mass_kg_m2.is_none());
        assert!(q.gwp_a1_a3_kgco2e_m2.is_some() && q.cost_per_m2.is_some());
        assert_eq!(
            q.gaps,
            vec![
                LayerGap {
                    layer: 1,
                    material: Some(ElementId(2)),
                    quantity: StructureQuantity::UValue,
                    reason: GapReason::NoConductivity
                },
                LayerGap {
                    layer: 1,
                    material: Some(ElementId(2)),
                    quantity: StructureQuantity::Mass,
                    reason: GapReason::NoDensity
                },
            ]
        );
    }

    #[test]
    fn layer_without_material_blocks_every_quantity() {
        let lib = library();
        let q = compute(vec![layer(200.0, Some(1)), layer(15.0, None)], &lib);
        assert!(q.u_value_w_m2k.is_none() && q.mass_kg_m2.is_none());
        assert!(q.gwp_a1_a3_kgco2e_m2.is_none() && q.cost_per_m2.is_none());
        assert!(q.currency.is_none());
        assert_eq!(q.gaps.len(), 4);
        assert!(q
            .gaps
            .iter()
            .all(|g| g.layer == 1 && g.reason == GapReason::NoMaterial));
        // An id that is not in the library counts the same.
        let q = compute(vec![layer(200.0, Some(99))], &lib);
        assert_eq!(q.gaps.len(), 4);
    }

    #[test]
    fn zero_thickness_layer_adds_nothing_but_per_m2_carbon() {
        // A membrane without thermal / physical data: no resistance or
        // mass needed; its per-m² GWP and price still count.
        let mut lib = library();
        let wool = lib.get_mut(&ElementId(2)).unwrap();
        wool.thermal = None;
        let q = compute(vec![layer(200.0, Some(1)), layer(0.0, Some(2))], &lib);
        let brick_only = compute(vec![layer(200.0, Some(1))], &lib);
        assert!(q.is_complete(), "{:?}", q.gaps);
        assert!(near(
            q.u_value_w_m2k,
            brick_only.u_value_w_m2k.unwrap(),
            1e-12
        ));
        assert!(near(q.mass_kg_m2, 360.0, 1e-9));
        assert!(near(q.gwp_a1_a3_kgco2e_m2, 86.4 + 2.5, 1e-9));
        // A zero-thickness layer with no material still lacks GWP and cost.
        let q = compute(vec![layer(200.0, Some(1)), layer(0.0, None)], &lib);
        assert!(q.u_value_w_m2k.is_some() && q.mass_kg_m2.is_some());
        let lacking: Vec<_> = q.gaps.iter().map(|g| g.quantity).collect();
        assert_eq!(
            lacking,
            vec![StructureQuantity::Gwp, StructureQuantity::Cost]
        );
    }

    #[test]
    fn thickness_at_vertex_tolerance_is_zero() {
        // Within VERTEX_TOL a layer is zero-thickness (no conductivity
        // needed); just above it the layer must have data.
        let mut lib = library();
        lib.get_mut(&ElementId(2)).unwrap().thermal = None;
        let within = compute(
            vec![layer(200.0, Some(1)), layer(tol::VERTEX_TOL * 0.5, Some(2))],
            &lib,
        );
        assert!(within.u_value_w_m2k.is_some());
        let above = compute(
            vec![layer(200.0, Some(1)), layer(tol::VERTEX_TOL * 2.0, Some(2))],
            &lib,
        );
        assert!(above.u_value_w_m2k.is_none());
    }

    #[test]
    fn per_kg_figures_need_a_density() {
        let mut lib = library();
        let brick = lib.get_mut(&ElementId(1)).unwrap();
        brick.physical = None;
        let q = compute(vec![layer(200.0, Some(1))], &lib);
        assert!(q.gwp_a1_a3_kgco2e_m2.is_none());
        assert!(q
            .gaps
            .iter()
            .any(|g| g.quantity == StructureQuantity::Gwp && g.reason == GapReason::NoDensity));
        // Cost is per m³, so it does not need the density.
        assert!(near(q.cost_per_m2, 300_000.0, 1e-6));
    }

    #[test]
    fn mixed_currencies_are_reported() {
        let mut lib = library();
        lib.get_mut(&ElementId(2))
            .unwrap()
            .cost
            .as_mut()
            .unwrap()
            .currency = "USD".into();
        let q = compute(vec![layer(200.0, Some(1)), layer(100.0, Some(2))], &lib);
        assert!(q.cost_per_m2.is_none() && q.currency.is_none());
        assert_eq!(q.gaps.len(), 1);
        assert_eq!(q.gaps[0].reason, GapReason::CurrencyMismatch);
    }

    #[test]
    fn empty_structure_is_surface_resistances_only() {
        let q = compute(vec![], &library());
        assert!(near(q.thermal_resistance_m2k_w, 0.17, 1e-12));
        assert!(near(q.mass_kg_m2, 0.0, 1e-12));
        assert!(q.currency.is_none());
    }
}
