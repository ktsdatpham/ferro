//! Quantities split by phase status (ADR 0006 §6): demolished, new and
//! existing volume and area per category, the basis for demolition and
//! new-build costs and carbon.

use super::phase::PhaseStatus;
use super::{keys, Category, Document, Element, ElementId, ParamValue};
use serde::Serialize;
use std::collections::BTreeMap;

/// Totals of one status within one category. Volumes in mm³, areas in mm².
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct StatusTotals {
    pub count: usize,
    /// Sum of the volumes of closed bodies (mm³).
    pub volume_mm3: f64,
    /// Floors: plan area; walls: gross length × height (one face);
    /// others: the `ferro.dimensions.area` parameter when present (mm²).
    pub area_mm2: f64,
}

/// Quantities of a document in the view of one phase: per category, per
/// shown status (`existing`, `new`, `demolished`, `temporary`). Future and
/// past elements are not counted. Deterministic (sorted maps).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PhaseQuantities {
    pub phase: Option<ElementId>,
    pub categories: BTreeMap<Category, BTreeMap<PhaseStatus, StatusTotals>>,
}

impl PhaseQuantities {
    /// Quantities of every phased element (Forms, walls, floors, doors,
    /// windows) of `doc` in phase `phase`. Returns `None` if `phase` is not a
    /// phase of the document.
    pub fn from_document(doc: &Document, phase: ElementId) -> Option<PhaseQuantities> {
        doc.phase_index(phase)?;
        let mut out = PhaseQuantities {
            phase: Some(phase),
            ..Default::default()
        };
        for e in doc.elements() {
            if !super::phase::is_phased_category(e) {
                continue;
            }
            let status = doc.phase_status(e, phase);
            if !status.is_shown() {
                continue;
            }
            let t = out
                .categories
                .entry(e.category)
                .or_default()
                .entry(status)
                .or_default();
            t.count += 1;
            t.volume_mm3 += element_volume(e);
            t.area_mm2 += element_area(e);
        }
        Some(out)
    }

    /// Totals of one status over every category.
    pub fn total(&self, status: PhaseStatus) -> StatusTotals {
        let mut t = StatusTotals::default();
        for s in self.categories.values().filter_map(|m| m.get(&status)) {
            t.count += s.count;
            t.volume_mm3 += s.volume_mm3;
            t.area_mm2 += s.area_mm2;
        }
        t
    }
}

fn element_volume(e: &Element) -> f64 {
    e.geometry()
        .filter(|b| b.is_closed())
        .map(|b| b.volume().abs())
        .unwrap_or(0.0)
}

fn element_area(e: &Element) -> f64 {
    let num = |k: &str| match e.param(k) {
        Some(ParamValue::Area(a)) => Some(*a),
        Some(ParamValue::Length(l)) => Some(l.mm()),
        _ => None,
    };
    if let Some(a) = num(keys::AREA) {
        return a;
    }
    if e.category == Category::Walls {
        if let (Some(l), Some(h)) = (num(keys::LENGTH), num(keys::HEIGHT)) {
            return l * h;
        }
    }
    0.0
}
