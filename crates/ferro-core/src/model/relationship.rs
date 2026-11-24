//! Relationships between elements as data (ADR 0004 §5, F5; M5-5).
//!
//! A relationship is a row in the document's relationship table: a stable
//! [`Uid`], a [`RelationshipKind`] and the elements it links, in an order
//! whose meaning depends on the kind. Generation reads the table (a join
//! decides who cuts whom, a cut subtracts a void), so relationships survive
//! freezing, copying between documents and sync, unlike geometry inferred on
//! the fly. Revit has no equivalent object; IFC has `IfcRelationship`
//! subtypes (`IfcRelVoidsElement`, `IfcRelConnectsPathElements`, …).

use super::{Document, ElementId, Uid};
use serde::{Deserialize, Serialize};

/// What a relationship means. The order of [`Relationship::elements`] is
/// given per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    /// `[first, second]`: the two elements' geometry is joined; where they
    /// overlap, `first` wins and cuts `second` (Revit: Join Geometry and
    /// wall joins; IFC `IfcRelConnectsPathElements` for walls).
    Join,
    /// `[cutter, cut]`: the cutter's volume is removed from the cut element,
    /// non-destructively (Revit: Cut Geometry / voids; IFC
    /// `IfcRelVoidsElement`).
    Cut,
    /// `[host, hosted]`: the hosted element lives in the host (an opening
    /// in a wall). Revit: `FamilyInstance.Host`; IFC
    /// `IfcRelFillsElement` / `IfcRelVoidsElement`.
    Host,
    /// `[attached, target]`: the attached element's top or base follows the
    /// target (a wall attached to a roof). Revit: Attach Top/Base.
    Attach,
    /// `[space, boundary]`: the boundary element bounds the space (room
    /// bounding). IFC `IfcRelSpaceBoundary`.
    Bounds,
    /// `[a, b]`: a physical connection with no geometric effect
    /// (structural, MEP). IFC `IfcRelConnectsElements`.
    Connect,
    /// `[whole, part, part, …]`: the parts make up the whole (an assembly).
    /// IFC `IfcRelAggregates`.
    Aggregate,
}

impl RelationshipKind {
    pub const ALL: [RelationshipKind; 7] = [
        RelationshipKind::Join,
        RelationshipKind::Cut,
        RelationshipKind::Host,
        RelationshipKind::Attach,
        RelationshipKind::Bounds,
        RelationshipKind::Connect,
        RelationshipKind::Aggregate,
    ];

    /// Stable snake_case name, as in files and MCP.
    pub fn name(&self) -> &'static str {
        match self {
            RelationshipKind::Join => "join",
            RelationshipKind::Cut => "cut",
            RelationshipKind::Host => "host",
            RelationshipKind::Attach => "attach",
            RelationshipKind::Bounds => "bounds",
            RelationshipKind::Connect => "connect",
            RelationshipKind::Aggregate => "aggregate",
        }
    }

    /// `true` when swapping the two elements gives the same relationship.
    pub fn is_symmetric(&self) -> bool {
        matches!(self, RelationshipKind::Connect)
    }
}

/// One row of the relationship table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    /// Stable id, kept across files, sync and IFC (`GlobalId`).
    pub uid: Uid,
    pub kind: RelationshipKind,
    /// Linked elements, ordered as [`RelationshipKind`] says.
    pub elements: Vec<ElementId>,
    /// Phase the relationship is made in (a `Cut` made in a renovation
    /// phase shows the removed part as demolished; ADR 0006 §5). `None` =
    /// always.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ElementId>,
}

/// Why a relationship is refused, with its stable failure code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipError {
    /// `ferro.model.invalid_relationship`, `ferro.model.missing_element` or
    /// `ferro.model.duplicate_relationship`.
    pub code: &'static str,
    pub message: String,
}

impl RelationshipError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        RelationshipError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RelationshipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Relationship {
    /// `true` if `id` is one of the linked elements.
    pub fn involves(&self, id: ElementId) -> bool {
        self.elements.contains(&id)
    }

    /// `true` if `other` links the same elements with the same kind (in
    /// either order for symmetric kinds; for joins too, since two joins of
    /// one pair would disagree on who wins).
    pub fn same_link(&self, other: &Relationship) -> bool {
        if self.kind != other.kind {
            return false;
        }
        if self.elements == other.elements {
            return true;
        }
        let pair_kind = self.kind.is_symmetric() || self.kind == RelationshipKind::Join;
        pair_kind
            && self.elements.len() == 2
            && other.elements.len() == 2
            && self.elements[0] == other.elements[1]
            && self.elements[1] == other.elements[0]
    }

    /// Check the row against `doc`: the number of elements for the kind
    /// (two, or a whole and at least one part for `Aggregate`), all
    /// distinct, all existing elements with geometry (not levels, types,
    /// materials or phases), the `cut` element of a cut a wall, floor or
    /// Form, the phase a phase, and no other row linking the same
    /// elements the same way (rows with this row's uid are ignored, so an
    /// edit can be checked in place).
    pub fn validate(&self, doc: &Document) -> Result<(), RelationshipError> {
        let invalid = |m: String| RelationshipError::new("ferro.model.invalid_relationship", m);
        let n = self.elements.len();
        let ok_len = match self.kind {
            RelationshipKind::Aggregate => n >= 2,
            _ => n == 2,
        };
        if !ok_len {
            return Err(invalid(format!(
                "a {} relationship links {} elements, got {n}",
                self.kind.name(),
                if self.kind == RelationshipKind::Aggregate {
                    "at least 2"
                } else {
                    "2"
                }
            )));
        }
        for (i, id) in self.elements.iter().enumerate() {
            if self.elements[..i].contains(id) {
                return Err(invalid(format!("element {id} is listed twice")));
            }
            let e = doc.get(*id).ok_or_else(|| {
                RelationshipError::new(
                    "ferro.model.missing_element",
                    format!("element {id} does not exist"),
                )
            })?;
            if e.geometry_source().is_none() {
                return Err(invalid(format!(
                    "{} cannot take part in a relationship",
                    e.label()
                )));
            }
        }
        if self.kind == RelationshipKind::Cut {
            if let Some(t) = self.elements.get(1).and_then(|id| doc.get(*id)) {
                if !super::cut::is_cut_target(t) {
                    return Err(invalid(format!(
                        "{} cannot be cut: a cut row cuts a wall, a floor or a Form",
                        t.label()
                    )));
                }
            }
        }
        if let Some(p) = self.phase {
            if doc.get(p).and_then(|e| e.as_phase()).is_none() {
                return Err(invalid(format!("{p} is not a phase")));
            }
        }
        if let Some(o) = doc
            .relationships()
            .find(|o| o.uid != self.uid && o.same_link(self))
        {
            return Err(RelationshipError::new(
                "ferro.model.duplicate_relationship",
                format!(
                    "these elements already have a {} relationship ({})",
                    self.kind.name(),
                    o.uid
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(kind: RelationshipKind, ids: &[u64]) -> Relationship {
        Relationship {
            uid: Uid(ids.iter().map(|i| *i as u128).sum::<u128>() + 1),
            kind,
            elements: ids.iter().map(|i| ElementId(*i)).collect(),
            phase: None,
        }
    }

    #[test]
    fn same_link_follows_the_kind() {
        use RelationshipKind::*;
        assert!(rel(Join, &[1, 2]).same_link(&rel(Join, &[2, 1])));
        assert!(rel(Connect, &[1, 2]).same_link(&rel(Connect, &[2, 1])));
        // Order matters for a cut: the reverse cut is a different row.
        assert!(!rel(Cut, &[1, 2]).same_link(&rel(Cut, &[2, 1])));
        assert!(!rel(Cut, &[1, 2]).same_link(&rel(Join, &[1, 2])));
        assert!(!rel(Aggregate, &[1, 2, 3]).same_link(&rel(Aggregate, &[1, 3, 2])));
    }

    #[test]
    fn names_are_the_serde_names() {
        for k in RelationshipKind::ALL {
            assert_eq!(serde_json::to_value(k).unwrap(), k.name());
        }
    }
}
