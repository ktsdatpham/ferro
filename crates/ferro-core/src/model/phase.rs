//! Phases (ADR 0006 §Phases, D-77): the project's ordered construction
//! stages and the status an element has in a view's phase.
//!
//! A [`Phase`] is the body of an element of category `Phases` (Revit:
//! `Phase`, `OST_Phases`). Elements point at phases through
//! `phase_created` / `phase_demolished` (Revit: *Phase Created*, *Phase
//! Demolished*). The status of an element in a view is derived, never stored
//! ([`phase_status`]).

use super::{Category, Document, Element, ElementBody, ElementId};
use serde::{Deserialize, Serialize};

/// Name of the first default phase (created the first time phasing is used).
pub const EXISTING_PHASE_NAME: &str = "Existing";
/// Name of the second default phase.
pub const NEW_CONSTRUCTION_PHASE_NAME: &str = "New Construction";

/// A construction stage (Revit: `Phase`). Phases are ordered by `order`
/// (then by element id, so a gap or a tie never breaks determinism); the
/// commands keep `order` equal to the position `0..n`.
///
/// Dates are optional ISO 8601 calendar dates (`YYYY-MM-DD`), for
/// information and schedules only; they never decide the order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
}

/// Status of an element in a view's phase (ADR 0006 §3; Revit phase
/// statuses). Derived from the element's two phase fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    /// Created in an earlier phase and still standing.
    Existing,
    /// Created in this phase.
    New,
    /// Created earlier and demolished in this phase.
    Demolished,
    /// Created and demolished in this phase.
    Temporary,
    /// Created in a later phase; hidden in this view.
    Future,
    /// Demolished in an earlier phase; hidden in this view (gone).
    Past,
}

impl PhaseStatus {
    /// All statuses, in display order.
    pub const ALL: [PhaseStatus; 6] = [
        PhaseStatus::Existing,
        PhaseStatus::New,
        PhaseStatus::Demolished,
        PhaseStatus::Temporary,
        PhaseStatus::Future,
        PhaseStatus::Past,
    ];

    /// `true` if the element is drawn in a view of this phase (not future,
    /// not already demolished).
    pub fn is_shown(self) -> bool {
        !matches!(self, PhaseStatus::Future | PhaseStatus::Past)
    }

    /// Stable snake_case name (`"existing"`, `"new"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseStatus::Existing => "existing",
            PhaseStatus::New => "new",
            PhaseStatus::Demolished => "demolished",
            PhaseStatus::Temporary => "temporary",
            PhaseStatus::Future => "future",
            PhaseStatus::Past => "past",
        }
    }

    /// IFC4 `Pset_*Common.Status` value (`PEnum_ElementStatus`), or `None`
    /// when the element is not exported in that phase (future / past).
    pub fn ifc_status(self) -> Option<&'static str> {
        match self {
            PhaseStatus::Existing => Some("EXISTING"),
            PhaseStatus::New => Some("NEW"),
            PhaseStatus::Demolished => Some("DEMOLISH"),
            PhaseStatus::Temporary => Some("TEMPORARY"),
            PhaseStatus::Future | PhaseStatus::Past => None,
        }
    }
}

/// Status of an element in the view phase at position `view`, given the
/// positions of its created / demolished phases in the ordered phase list.
///
/// Rules (ADR 0006 §3, Revit behaviour):
/// * no `created` = created before every phase (unphased data such as
///   imports and elements drawn before phasing was used): it is *existing*
///   in every phase until demolished;
/// * no `demolished` = never demolished;
/// * created after the view → `Future`; demolished before it → `Past`;
/// * created and demolished in the view → `Temporary`; demolished in it →
///   `Demolished`; created in it → `New`; otherwise `Existing`.
///
/// Pure and total: inconsistent input (demolished before created) still
/// gives an answer (`Future` wins over `Past`).
pub fn phase_status(created: Option<usize>, demolished: Option<usize>, view: usize) -> PhaseStatus {
    if created.is_some_and(|c| c > view) {
        return PhaseStatus::Future;
    }
    if demolished.is_some_and(|d| d < view) {
        return PhaseStatus::Past;
    }
    let made_here = created == Some(view);
    let gone_here = demolished == Some(view);
    match (made_here, gone_here) {
        (true, true) => PhaseStatus::Temporary,
        (false, true) => PhaseStatus::Demolished,
        (true, false) => PhaseStatus::New,
        (false, false) => PhaseStatus::Existing,
    }
}

/// `true` for elements that carry phase fields: elements placed in the
/// model with geometry (Forms, walls, floors, doors, windows). Levels,
/// types, materials and phases themselves are in every phase.
pub fn is_phased_category(e: &Element) -> bool {
    e.geometry_source().is_some()
}

/// Check an ISO 8601 calendar date `YYYY-MM-DD` (Gregorian, leap years).
pub fn is_valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        s.get(r)
            .filter(|p| p.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|p| p.parse().ok())
    };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

impl Element {
    /// The phase data, if this element is a phase.
    pub fn as_phase(&self) -> Option<&Phase> {
        match &self.body {
            ElementBody::Phase(p) => Some(p),
            _ => None,
        }
    }
}

impl Document {
    /// The project's phases in order (by `order`, then id). Empty until
    /// phasing is used.
    pub fn phases(&self) -> Vec<&Element> {
        let mut v: Vec<&Element> = self
            .by_category(Category::Phases)
            .filter(|e| e.as_phase().is_some())
            .collect();
        v.sort_by_key(|e| (e.as_phase().map(|p| p.order).unwrap_or(u32::MAX), e.id));
        v
    }

    /// Position of a phase in [`Document::phases`]; `None` if `id` is not a
    /// phase.
    pub fn phase_index(&self, id: ElementId) -> Option<usize> {
        self.phases().iter().position(|p| p.id == id)
    }

    /// Status of element `e` in a view of phase `view_phase` ([`phase_status`]).
    /// Elements without phase fields (levels, types) and unknown view phases
    /// give `Existing`; a phase field pointing at a non-phase counts as
    /// missing.
    pub fn phase_status(&self, e: &Element, view_phase: ElementId) -> PhaseStatus {
        let phases = self.phases();
        let pos = |id: Option<ElementId>| id.and_then(|i| phases.iter().position(|p| p.id == i));
        match (is_phased_category(e), pos(Some(view_phase))) {
            (true, Some(v)) => phase_status(pos(e.phase_created), pos(e.phase_demolished), v),
            _ => PhaseStatus::Existing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhaseStatus::*;
    use super::*;

    #[test]
    fn status_table_covers_every_combination() {
        // Three phases 0, 1, 2; view in phase 1. `None` = missing field.
        let cases: &[(Option<usize>, Option<usize>, PhaseStatus)] = &[
            (None, None, Existing),
            (Some(0), None, Existing),
            (Some(1), None, New),
            (Some(2), None, Future),
            (None, Some(0), Past),
            (None, Some(1), Demolished),
            (None, Some(2), Existing),
            (Some(0), Some(0), Past),
            (Some(0), Some(1), Demolished),
            (Some(0), Some(2), Existing),
            (Some(1), Some(1), Temporary),
            (Some(1), Some(2), New),
            (Some(2), Some(2), Future),
        ];
        for (c, d, want) in cases {
            assert_eq!(phase_status(*c, *d, 1), *want, "created {c:?} demolished {d:?}");
        }
    }

    #[test]
    fn status_at_the_ends_of_the_phase_list() {
        // First phase: elements created in it are new, nothing is existing
        // except unphased data.
        assert_eq!(phase_status(Some(0), None, 0), New);
        assert_eq!(phase_status(None, None, 0), Existing);
        assert_eq!(phase_status(None, Some(0), 0), Demolished);
        // Last phase of the two-phase renovation (ADR §4).
        assert_eq!(phase_status(Some(0), None, 1), Existing);
        assert_eq!(phase_status(Some(0), Some(1), 1), Demolished);
        assert_eq!(phase_status(Some(1), None, 1), New);
    }

    #[test]
    fn inconsistent_fields_still_give_an_answer() {
        // Demolished before created (refused by the commands): future wins.
        assert_eq!(phase_status(Some(2), Some(0), 1), Future);
        assert_eq!(phase_status(Some(1), Some(0), 1), Past);
    }

    #[test]
    fn shown_and_ifc_values() {
        for s in PhaseStatus::ALL {
            assert_eq!(s.is_shown(), s.ifc_status().is_some(), "{s:?}");
        }
        assert_eq!(Demolished.ifc_status(), Some("DEMOLISH"));
        assert_eq!(Temporary.as_str(), "temporary");
    }

    #[test]
    fn dates_are_iso_calendar_dates() {
        for ok in ["2026-11-24", "2024-02-29", "2000-02-29", "1999-12-31"] {
            assert!(is_valid_date(ok), "{ok}");
        }
        for bad in [
            "",
            "2026-1-24",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "2023-02-29",
            "1900-02-29",
            "2026/11/24",
            "２０２６-11-24",
            "+026-11-24",
        ] {
            assert!(!is_valid_date(bad), "{bad}");
        }
    }
}
