//! Non-destructive cuts (ADR 0004 §5, M5-5.3) and phased cuts (ADR 0006 §5,
//! M5-23.6).
//!
//! A relationship row `cut [cutter, cut]` makes the geometry of the `cut`
//! element (the *target*) equal to its own body minus the cutter's body.
//! Nothing is stored but the row: the result is recomputed during
//! regeneration through the injected [`Kernel`], so moving, editing or
//! deleting the cutter, or deleting the row, regenerates the target.
//!
//! Rules (Foundation §3.8):
//! - **Targets** are walls, floors (generated geometry) and Forms of any
//!   category (direct geometry; the source body stays as drawn, the result
//!   is the Form's derived `cut_body`). Openings are not targets.
//! - **Cutters** are any element with geometry (Form, wall, floor, opening).
//!   A cutter contributes its *uncut* body (its own cuts do not chain).
//! - **Order.** Rows apply one after the other: rows without a phase first,
//!   then by phase order, each group in uid order. Without phases this is
//!   plain uid order.
//! - **Voids.** A Form of category `Forms` that is the cutter of at least
//!   one row is a *void* ([`is_void`], Revit: a void form): views and
//!   exports leave it out as an element of its own; the app draws it as a
//!   translucent outline so it stays selectable. Walls, floors and openings
//!   used as cutters stay ordinary elements.
//! - **Failures never fail a transaction.** Without a kernel the target
//!   keeps its uncut body and regeneration reports
//!   `ferro.kernel.unavailable` once; a kernel error on one row (empty
//!   result, non-manifold, not a solid) skips that row and reports a
//!   warning with the kernel's code.
//! - **Lineage.** The kernel keeps the ids of the target's surviving faces
//!   (`KernelResult`), so references to them survive re-cuts.

use super::generate::GenIssue;
use super::{Category, Document, Element, ElementBody, ElementId, Relationship, RelationshipKind};
use crate::algo::{BooleanOp, Kernel, KernelError};
use crate::topo::Body;

/// `true` when `e` can be the `cut` element of a cut row: walls, floors and
/// Forms (direct geometry of any category).
pub fn is_cut_target(e: &Element) -> bool {
    matches!(
        e.body,
        ElementBody::Wall(_) | ElementBody::Floor(_) | ElementBody::Form(_)
    )
}

/// `true` when `e` is a void: a Form of category `Forms` that cuts at least
/// one element. Views and exports leave voids out (Revit: void forms are
/// not shown); deleting its last cut row makes it an ordinary Form again.
pub fn is_void(doc: &Document, e: &Element) -> bool {
    e.category == Category::Forms
        && e.as_form().is_some()
        && doc
            .relationships_of(e.id)
            .any(|r| r.kind == RelationshipKind::Cut && r.elements.first() == Some(&e.id))
}

/// Position of a row's phase for ordering: rows without a phase come first
/// (`0`), then phase `i` as `i + 1`. A phase that no longer exists sorts
/// last.
fn phase_rank(doc: &Document, r: &Relationship) -> usize {
    match r.phase {
        None => 0,
        Some(p) => doc.phase_index(p).map(|i| i + 1).unwrap_or(usize::MAX),
    }
}

/// The cut rows whose target is `target`, in the order they apply: rows
/// without a phase first, then by phase order, each group in uid order.
pub fn cuts_of(doc: &Document, target: ElementId) -> Vec<&Relationship> {
    let mut v: Vec<&Relationship> = doc
        .relationships()
        .filter(|r| r.kind == RelationshipKind::Cut && r.elements.get(1) == Some(&target))
        .collect();
    v.sort_by_key(|r| (phase_rank(doc, r), r.uid));
    v
}

/// The cut rows of `target` that apply in a view of phase `phase`: rows
/// without a phase and rows made in `phase` or earlier. With a `phase` that
/// is not a phase of `doc`, every row applies.
pub fn cuts_in_phase(doc: &Document, target: ElementId, phase: ElementId) -> Vec<&Relationship> {
    let Some(v) = doc.phase_index(phase) else {
        return cuts_of(doc, target);
    };
    cuts_of(doc, target)
        .into_iter()
        .filter(|r| phase_rank(doc, r) <= v + 1)
        .collect()
}

/// The cut rows of `target` made before phase `phase`: rows without a
/// phase and rows of earlier phases (the state the phase starts from).
/// With a `phase` that is not a phase of `doc`, every row.
pub fn cuts_before_phase(
    doc: &Document,
    target: ElementId,
    phase: ElementId,
) -> Vec<&Relationship> {
    let Some(v) = doc.phase_index(phase) else {
        return cuts_of(doc, target);
    };
    cuts_of(doc, target)
        .into_iter()
        .filter(|r| phase_rank(doc, r) <= v)
        .collect()
}

/// `true` when `target` has a cut row made in a phase (ADR 0006 §5).
pub fn has_phased_cuts(doc: &Document, target: ElementId) -> bool {
    cuts_of(doc, target).iter().any(|r| r.phase.is_some())
}

/// What [`apply_cuts`] produced: the cut body and the warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct CutOutcome {
    /// The target's body after every row that applied (the uncut body when
    /// none did).
    pub body: Body,
    /// `ferro.kernel.*` warnings (one per failed row, or one
    /// `ferro.kernel.unavailable`).
    pub issues: Vec<GenIssue>,
}

/// Subtract the cutters of `rows` (in the given order) from `uncut`, the
/// body of `target`. `cutter_body` gives a cutter's uncut body. Without a
/// kernel the body stays uncut with one `ferro.kernel.unavailable` warning
/// (`warn_unavailable = false` leaves it out, so a regeneration reports it
/// once). A row whose cutter has no body, or whose boolean fails, is
/// skipped with a warning carrying the kernel's code. Never panics.
pub fn apply_cuts(
    kernel: Option<&dyn Kernel>,
    target: ElementId,
    uncut: &Body,
    rows: &[&Relationship],
    cutter_body: &dyn Fn(ElementId) -> Option<Body>,
    warn_unavailable: bool,
) -> CutOutcome {
    let mut out = CutOutcome {
        body: uncut.clone(),
        issues: Vec::new(),
    };
    if rows.is_empty() {
        return out;
    }
    let Some(kernel) = kernel else {
        if warn_unavailable {
            let e = KernelError::unavailable();
            out.issues.push(GenIssue::warning(
                e.code,
                format!(
                    "{}; cuts are not applied, the element keeps its uncut body",
                    e.message
                ),
                target,
            ));
        }
        return out;
    };
    for r in rows {
        let Some(&cutter) = r.elements.first() else {
            continue;
        };
        let Some(b) = cutter_body(cutter).filter(|b| b.face_count() > 0) else {
            out.issues.push(GenIssue::warning(
                KernelError::NOT_SOLID,
                format!("cut {}: the cutter {cutter} has no body", r.uid),
                target,
            ));
            continue;
        };
        match kernel.boolean(BooleanOp::Difference, &out.body, &b) {
            Ok(res) => out.body = res.body,
            Err(e) => out.issues.push(GenIssue::warning(
                e.code,
                format!("cut {} by {cutter} is not applied: {}", r.uid, e.message),
                target,
            )),
        }
    }
    out
}

/// The body of element `id` in a view of phase `phase` (ADR 0006 §5): its
/// uncut body less the cutters of the rows that apply in that phase
/// ([`cuts_in_phase`]). `uncut` is the element's uncut body
/// (`generate::uncut_body`). Without a kernel, or when a row fails, the
/// rows that cannot be applied are left out (see [`apply_cuts`]).
pub fn body_in_phase(
    doc: &Document,
    kernel: Option<&dyn Kernel>,
    id: ElementId,
    phase: ElementId,
    uncut: &Body,
    cutter_body: &dyn Fn(ElementId) -> Option<Body>,
) -> CutOutcome {
    let rows = cuts_in_phase(doc, id, phase);
    apply_cuts(kernel, id, uncut, &rows, cutter_body, true)
}

/// The part of element `id` that the cut rows made in phase `phase`
/// remove (ADR 0006 §5: shown and counted as demolished in that phase):
/// the body at the start of the phase less the body in the phase. `None`
/// when no row is made in `phase`, without a kernel, or when nothing is
/// removed (the kernel's `ferro.kernel.empty_result`).
pub fn removed_in_phase(
    doc: &Document,
    kernel: Option<&dyn Kernel>,
    id: ElementId,
    phase: ElementId,
    uncut: &Body,
    cutter_body: &dyn Fn(ElementId) -> Option<Body>,
) -> Option<Body> {
    let kernel = kernel?;
    let before_rows = cuts_before_phase(doc, id, phase);
    let rows = cuts_in_phase(doc, id, phase);
    if rows.len() == before_rows.len() {
        return None;
    }
    let before = apply_cuts(Some(kernel), id, uncut, &before_rows, cutter_body, false).body;
    let after = apply_cuts(Some(kernel), id, uncut, &rows, cutter_body, false).body;
    kernel
        .boolean(BooleanOp::Difference, &before, &after)
        .ok()
        .map(|r| r.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Uid;

    #[test]
    fn rows_without_a_phase_come_first_then_by_uid() {
        let doc = Document::new("t");
        let r = |u: u128, phase: Option<ElementId>| Relationship {
            uid: Uid(u),
            kind: RelationshipKind::Cut,
            elements: vec![ElementId(1), ElementId(2)],
            phase,
        };
        let a = r(3, None);
        let b = r(1, None);
        let c = r(2, Some(ElementId(99))); // not a phase: sorts last
        let mut v = [&a, &b, &c];
        v.sort_by_key(|r| (phase_rank(&doc, r), r.uid));
        assert_eq!(v.map(|r| r.uid.0), [1, 3, 2]);
    }

    #[test]
    fn without_rows_or_kernel_the_body_is_uncut() {
        let body = Body::default();
        let none = apply_cuts(None, ElementId(1), &body, &[], &|_| None, true);
        assert!(none.issues.is_empty());
        let row = Relationship {
            uid: Uid(1),
            kind: RelationshipKind::Cut,
            elements: vec![ElementId(2), ElementId(1)],
            phase: None,
        };
        let o = apply_cuts(None, ElementId(1), &body, &[&row], &|_| None, true);
        assert_eq!(o.body, body);
        assert_eq!(o.issues.len(), 1);
        assert_eq!(o.issues[0].code, "ferro.kernel.unavailable");
        assert!(!o.issues[0].is_error);
        let quiet = apply_cuts(None, ElementId(1), &body, &[&row], &|_| None, false);
        assert!(quiet.issues.is_empty());
    }
}
