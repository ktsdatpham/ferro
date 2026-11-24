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
//! - **Cache** (M5-5.7). A [`CutCache`] passed in by the caller (the
//!   session keeps one) remembers each row's result, so a regeneration
//!   calls the kernel only for rows whose cutter, target source body, row
//!   or earlier rows changed. It never lives in the `Document`: results
//!   are the same with or without it.

use super::generate::GenIssue;
use super::{Category, Document, Element, ElementBody, ElementId, Relationship, RelationshipKind};
use crate::algo::{BooleanOp, Kernel, KernelError};
use crate::topo::Body;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

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
/// [`apply_cuts_cached`] without a cache.
pub fn apply_cuts(
    kernel: Option<&dyn Kernel>,
    target: ElementId,
    uncut: &Body,
    rows: &[&Relationship],
    cutter_body: &dyn Fn(ElementId) -> Option<Body>,
    warn_unavailable: bool,
) -> CutOutcome {
    apply_cuts_cached(
        kernel,
        target,
        uncut,
        rows,
        cutter_body,
        warn_unavailable,
        None,
    )
}

/// [`apply_cuts`] through `cache` (M5-5.7): a row whose result is in the
/// cache, for the same target source body, the same earlier rows and the
/// same cutter body, does not call the kernel; a failed row is cached
/// with its error, so its warning is reported again. The outcome is the
/// same as without the cache (for a deterministic kernel).
pub fn apply_cuts_cached(
    kernel: Option<&dyn Kernel>,
    target: ElementId,
    uncut: &Body,
    rows: &[&Relationship],
    cutter_body: &dyn Fn(ElementId) -> Option<Body>,
    warn_unavailable: bool,
    cache: Option<&CutCache>,
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
    // Key of the rows applied so far: starts from the source body.
    let mut chain = cache.map(|_| body_key(uncut));
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
            // The chain changes: a later body must not reuse this result.
            chain = chain.map(|k| step_key(k, r, [0, 0]));
            continue;
        };
        let key = chain.map(|k| step_key(k, r, body_key(&b)));
        let cached = match (cache, key) {
            (Some(c), Some(k)) => c.get(k),
            _ => None,
        };
        let result = match cached {
            Some(res) => res,
            None => {
                let res = kernel
                    .boolean(BooleanOp::Difference, &out.body, &b)
                    .map(|k| k.body);
                if let (Some(c), Some(k)) = (cache, key) {
                    c.put(k, res.clone());
                }
                res
            }
        };
        chain = key;
        match result {
            Ok(body) => out.body = body,
            Err(e) => out.issues.push(GenIssue::warning(
                e.code,
                format!("cut {} by {cutter} is not applied: {}", r.uid, e.message),
                target,
            )),
        }
    }
    out
}

/// A 128-bit key: two independent 64-bit hashes.
type CutKey = [u64; 2];

/// Session-level cache of cut results (M5-5.7), keyed by the row's uid and
/// phase, the cutter's body, and the body the row applies to (the target's
/// source body and the rows before it). Pass it to
/// `generate::regenerate_cached` / [`apply_cuts_cached`]; it is shared
/// between transactions, previews and resolved design-option combinations
/// of one session.
///
/// It lives outside the `Document` (which stays plain data and
/// deterministic): a hit returns exactly what the kernel returned for the
/// same inputs. Keys are 128-bit hashes of the inputs (a collision is
/// ~2^-128 per pair). The cache holds at most [`CutCache::MAX_ENTRIES`]
/// results and is emptied when full. Safe to share between threads; a
/// poisoned lock is recovered, never a panic.
#[derive(Debug, Default)]
pub struct CutCache {
    entries: Mutex<HashMap<CutKey, Result<Body, KernelError>>>,
}

impl CutCache {
    /// Most results kept before the cache is emptied.
    pub const MAX_ENTRIES: usize = 4096;

    /// An empty cache.
    pub fn new() -> CutCache {
        CutCache::default()
    }

    /// Number of cached row results.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// `true` when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every result.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CutKey, Result<Body, KernelError>>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn get(&self, k: CutKey) -> Option<Result<Body, KernelError>> {
        self.lock().get(&k).cloned()
    }

    fn put(&self, k: CutKey, v: Result<Body, KernelError>) {
        let mut m = self.lock();
        if m.len() >= Self::MAX_ENTRIES {
            m.clear();
        }
        m.insert(k, v);
    }
}

/// Feeds bytes to two hashers seeded differently (a 128-bit key).
struct KeyWriter(DefaultHasher, DefaultHasher);

impl KeyWriter {
    fn new() -> KeyWriter {
        let a = DefaultHasher::new();
        let mut b = DefaultHasher::new();
        0x9e37_79b9_7f4a_7c15_u64.hash(&mut b);
        KeyWriter(a, b)
    }
    fn key(&self) -> CutKey {
        [self.0.finish(), self.1.finish()]
    }
}

impl std::io::Write for KeyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        self.1.write(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Key of a body: a hash of its serialised form (every id, coordinate bit
/// and surface), so any change gives another key.
fn body_key(b: &Body) -> CutKey {
    let mut w = KeyWriter::new();
    // Writing into a hasher cannot fail; on a serialiser error the key
    // still differs from a successful one by the marker below.
    if serde_json::to_writer(&mut w, b).is_err() {
        std::io::Write::write(&mut w, b"!").ok();
    }
    w.key()
}

/// Key after applying row `r` with a cutter of key `cutter` to a body of
/// key `prev`.
fn step_key(prev: CutKey, r: &Relationship, cutter: CutKey) -> CutKey {
    let mut w = KeyWriter::new();
    for h in [&mut w.0, &mut w.1] {
        prev.hash(h);
        r.uid.hash(h);
        r.phase.hash(h);
        cutter.hash(h);
    }
    w.key()
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
