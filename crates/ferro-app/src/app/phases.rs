//! Phases in the app (ADR 0006, M5-23): the active view's phase and phase
//! filter (stored in the view element, M5-22 / M5-23.8), the simple
//! renovation buttons, phase pickers in Properties and the per-status
//! graphics handed to the scene.
//!
//! Rules:
//! * a document without phases shows everything as usual;
//! * with phases, the view shows one phase (the last by default): future
//!   and already demolished elements are hidden; the view's phase filter
//!   (Revit *Phase Filter*) decides which other statuses are shown and
//!   which get status graphics (existing grey, demolished / temporary
//!   translucent red with dashed edges);
//! * elements created while phasing is used get the view's phase (Revit
//!   behaviour), in the same undo step as the command that made them;
//! * phased cuts (ADR 0006 §5, M5-23.6): an element whose `cut` row is
//!   made in a later phase than the view's is drawn uncut (its body in the
//!   view phase, through the session kernel); from the row's phase on it is
//!   drawn cut. The removed part is counted as demolished volume
//!   (`get_phase_quantities`);
//! * demolished parts (M5-23.10): in the phase of the row, when the element
//!   existed before that phase, the removed part
//!   (`model::cut::removed_in_phase`) follows the phase filter like a
//!   demolished element: overridden (Show All, Show Previous + Demo, Show
//!   Demo + New) it is drawn as a display-only solid with the demolished
//!   look (translucent red, dashed edges, not pickable); hidden (Show
//!   Complete, Show New, Show Previous + New) only the cut element is drawn;
//!   normal (No overrides) the element is drawn as it was before the
//!   phase's rows. Hiding the element hides its removed part.

use super::FerroApp;
use eframe::egui::{self, RichText, Vec2};
use ferro_core::algo::Kernel;
use ferro_core::model::phase::{is_phased_category, phase_status};
use ferro_core::model::{cut, generate, ElementBody, RelationshipKind, Uid};
use ferro_core::model::{PhaseFilter, PhaseGraphics, PhaseStatus};
use ferro_core::ops::phase_commands::{
    CreatePhase, RenovationStatus, SetElementPhase, SetRenovationStatus,
};
use ferro_core::{Document, ElementId};
use std::collections::{HashMap, HashSet};

/// What the view shows for phases: elements to hide and how to draw the
/// others (only statuses drawn differently from normal are listed).
#[derive(Clone, Default, PartialEq)]
pub struct PhaseDisplay {
    pub hidden: HashSet<ElementId>,
    pub looks: HashMap<ElementId, PhaseStatus>,
}

/// Phase view state of the app: a working copy of the active view's
/// phase and phase filter (`views` keeps it in sync with the view).
#[derive(Default)]
pub struct PhaseState {
    /// Phase picked for the view; `None` = the last phase.
    pub(super) pick: Option<ElementId>,
    /// The view's phase filter.
    pub(super) filter: PhaseFilter,
    /// Display computed for (document, view phase, filter).
    cache: Option<(Document, Option<ElementId>, PhaseFilter, PhaseDisplay)>,
    /// Phased-cut display computed for (document, view phase, filter).
    cut_cache: Option<(
        Document,
        Option<ElementId>,
        PhaseFilter,
        Option<PhaseCutDisplay>,
    )>,
}

/// Voids of `doc`: Forms of category `Forms` that cut (M5-5.3), drawn
/// faint so they stay selectable.
pub fn voids(doc: &Document) -> HashSet<ElementId> {
    doc.relationships()
        .filter(|r| r.kind == RelationshipKind::Cut)
        .filter_map(|r| r.elements.first().copied())
        .filter(|id| doc.get(*id).is_some_and(|e| cut::is_void(doc, e)))
        .collect()
}

/// Voids to leave out of the view, as Revit does once a void has cut: all
/// of them, except when `show` (View > Show voids) is on, when the void
/// itself is selected, or when an element it cuts is selected (so clicking
/// a wall reveals its voids, and a second click picks one).
pub fn hidden_voids(
    doc: &Document,
    voids: &HashSet<ElementId>,
    selection: &HashSet<ElementId>,
    show: bool,
) -> Vec<ElementId> {
    if show {
        return Vec::new();
    }
    let mut out: Vec<ElementId> = voids
        .iter()
        .copied()
        .filter(|v| {
            !selection.contains(v)
                && !doc.relationships().any(|r| {
                    r.kind == RelationshipKind::Cut
                        && r.elements.first() == Some(v)
                        && r.elements.iter().skip(1).any(|t| selection.contains(t))
                })
        })
        .collect();
    out.sort();
    out
}

/// What phased cuts change in the view (M5-23.6, M5-23.10): the document
/// to draw and the display-only elements holding demolished parts.
#[derive(Clone, PartialEq)]
pub struct PhaseCutDisplay {
    /// `doc` as drawn in the view phase: bodies of elements with phased
    /// cuts, plus one display-only element per demolished part.
    pub doc: Document,
    /// Display-only elements of demolished parts, each with the element
    /// it was removed from (`(part, element)`). Draw them with the
    /// demolished look; they are not in the session's document.
    pub parts: Vec<(ElementId, ElementId)>,
}

/// Seed mixed into the uid of a display-only demolished part.
const PART_UID_SEED: u128 = 0x6465_6d6f_6c69_7368_6564_2d70_6172_7400;

/// `doc` as drawn in view phase `view` under `filter` (M5-23.6,
/// M5-23.10): elements with `cut` rows made after `view` get their body
/// in that phase (uncut before the row's phase); an element that existed
/// before `view` and has rows made in `view` shows its removed part as
/// the filter shows demolished elements (see the module docs). `None`
/// when nothing differs from `doc`. Without a kernel the rows that do not
/// apply are left out (bodies stay uncut) and no removed part is drawn.
pub fn phased_cut_doc(
    doc: &Document,
    view: Option<ElementId>,
    filter: PhaseFilter,
    kernel: Option<&dyn Kernel>,
) -> Option<PhaseCutDisplay> {
    let view = view?;
    let targets: HashSet<ElementId> = doc
        .relationships()
        .filter(|r| r.kind == RelationshipKind::Cut && r.phase.is_some())
        .filter_map(|r| r.elements.get(1).copied())
        .collect();
    // Elements drawn with fewer rows than stored (rows of later phases).
    let later: HashSet<ElementId> = targets
        .iter()
        .copied()
        .filter(|t| cut::cuts_in_phase(doc, *t, view).len() != cut::cuts_of(doc, *t).len())
        .collect();
    // Elements existing in `view` with rows made in `view`.
    let demo = filter.graphics(PhaseStatus::Demolished);
    let removing: Vec<ElementId> = if kernel.is_none() || demo == PhaseGraphics::Hidden {
        Vec::new()
    } else {
        let mut v: Vec<ElementId> = targets
            .iter()
            .copied()
            .filter(|t| {
                doc.get(*t)
                    .is_some_and(|e| doc.phase_status(e, view) == PhaseStatus::Existing)
                    && cut::cuts_in_phase(doc, *t, view).len()
                        != cut::cuts_before_phase(doc, *t, view).len()
            })
            .collect();
        v.sort();
        v
    };
    if later.is_empty() && removing.is_empty() {
        return None;
    }
    let bodies = generate::uncut_bodies(doc);
    let cutter = |c: ElementId| bodies.get(&c).cloned();
    let mut elements: Vec<ferro_core::Element> = doc.elements().cloned().collect();
    let mut parts = Vec::new();
    let mut next = doc.elements().map(|e| e.id.0).max().unwrap_or(0) + 1;
    let mut uids: HashSet<Uid> = doc.elements().map(|e| e.uid).collect();
    let mut extra = Vec::new();
    for e in elements.iter_mut() {
        let Some(uncut) = bodies.get(&e.id) else {
            continue;
        };
        let show = |e: &mut ferro_core::Element, b: ferro_core::topo::Body| match &mut e.body {
            ElementBody::Form(f) => f.cut_body = (b != f.body).then_some(b),
            _ => generate::set_geometry(e, b),
        };
        if removing.contains(&e.id) {
            if demo == PhaseGraphics::Normal {
                // No overrides: drawn as it was before the phase's rows.
                let rows = cut::cuts_before_phase(doc, e.id, view);
                let b = cut::apply_cuts(kernel, e.id, uncut, &rows, &cutter, false).body;
                show(e, b);
                continue;
            }
            let removed = cut::removed_in_phase(doc, kernel, e.id, view, uncut, &cutter);
            if later.contains(&e.id) {
                let b = cut::body_in_phase(doc, kernel, e.id, view, uncut, &cutter).body;
                show(e, b);
            }
            let Some(removed) = removed.filter(|b| b.face_count() > 0) else {
                continue;
            };
            let mut uid = Uid(e.uid.0 ^ PART_UID_SEED);
            while !uids.insert(uid) {
                uid = Uid(uid.0.wrapping_add(1));
            }
            let mut part = e.clone();
            part.id = ElementId(next);
            next += 1;
            part.uid = uid;
            part.host_id = None;
            part.pinned = false;
            match &mut part.body {
                ElementBody::Form(f) => {
                    f.body = removed;
                    f.cut_body = None;
                }
                _ => generate::set_geometry(&mut part, removed),
            }
            parts.push((part.id, e.id));
            extra.push(part);
        } else if later.contains(&e.id) {
            let b = cut::body_in_phase(doc, kernel, e.id, view, uncut, &cutter).body;
            show(e, b);
        }
    }
    elements.extend(extra);
    Document::from_parts(doc.project().clone(), elements)
        .and_then(|d| d.with_relationships(doc.relationships().cloned().collect()))
        .ok()
        .map(|doc| PhaseCutDisplay { doc, parts })
}

/// Give demolished parts ([`PhaseCutDisplay::parts`]) their look: the
/// demolished status, or hidden with the element they come from.
pub fn mark_parts(
    parts: &[(ElementId, ElementId)],
    hidden: &mut HashSet<ElementId>,
    looks: &mut HashMap<ElementId, PhaseStatus>,
) {
    for (part, element) in parts {
        if hidden.contains(element) {
            hidden.insert(*part);
        } else {
            looks.insert(*part, PhaseStatus::Demolished);
        }
    }
}

/// Phase display of `doc` in view phase `view` under `filter`: pure, used
/// by the app and its tests.
pub fn phase_display(doc: &Document, view: Option<ElementId>, filter: PhaseFilter) -> PhaseDisplay {
    let mut out = PhaseDisplay::default();
    let Some(view) = view else {
        return out;
    };
    let phases: Vec<ElementId> = doc.phases().iter().map(|p| p.id).collect();
    let pos = |id: Option<ElementId>| id.and_then(|i| phases.iter().position(|p| *p == i));
    let Some(v) = pos(Some(view)) else {
        return out;
    };
    for e in doc.elements().filter(|e| is_phased_category(e)) {
        let st = phase_status(pos(e.phase_created), pos(e.phase_demolished), v);
        match filter.graphics(st) {
            PhaseGraphics::Hidden => {
                out.hidden.insert(e.id);
            }
            // New elements have no status look of their own.
            PhaseGraphics::Overridden if st != PhaseStatus::New => {
                out.looks.insert(e.id, st);
            }
            _ => {}
        }
    }
    out
}

impl FerroApp {
    /// Phase the view shows: the picked one if it still exists, else the
    /// last phase; `None` when the document has no phases.
    pub(super) fn view_phase(&self) -> Option<ElementId> {
        let doc = self.session.doc();
        self.phasing
            .pick
            .filter(|p| doc.phase_index(*p).is_some())
            .or_else(|| doc.phases().last().map(|p| p.id))
    }

    /// Hidden elements and looks for the current document and view phase,
    /// recomputed only when either changes.
    pub(super) fn phase_display(&mut self) -> PhaseDisplay {
        let view = self.view_phase();
        let filter = self.phasing.filter;
        let doc = self.session.doc();
        if let Some((d, v, f, shown)) = &self.phasing.cache {
            if *v == view && *f == filter && crate::scene::same_doc(d, doc) {
                return shown.clone();
            }
        }
        let shown = phase_display(doc, view, filter);
        self.phasing.cache = Some((doc.clone(), view, filter, shown.clone()));
        shown
    }

    /// The view document as drawn in the view phase when phased cuts make
    /// it differ, with its demolished parts ([`phased_cut_doc`]),
    /// recomputed only when the document, the view phase or the phase
    /// filter changes.
    pub(super) fn phase_cut_display(&mut self) -> Option<PhaseCutDisplay> {
        let view = self.view_phase();
        let filter = self.phasing.filter;
        let doc = self.doc().clone();
        if let Some((d, v, f, shown)) = &self.phasing.cut_cache {
            if *v == view && *f == filter && crate::scene::same_doc(d, &doc) {
                return shown.clone();
            }
        }
        let kernel = self.session.kernel().map(|k| k.as_ref());
        let shown = phased_cut_doc(&doc, view, filter, kernel);
        self.phasing.cut_cache = Some((doc, view, filter, shown.clone()));
        shown
    }

    /// Phase given to elements created now: the view phase while phasing is
    /// used.
    pub(super) fn phase_for_new_elements(&self) -> Option<ElementId> {
        self.view_phase()
    }

    /// After a command created elements, give the phased ones the view
    /// phase (inside the command's undo group).
    pub(super) fn assign_view_phase(&mut self, added: &[ElementId]) {
        let Some(phase) = self.phase_for_new_elements() else {
            return;
        };
        let doc = self.session.doc();
        let elements: Vec<ElementId> = added
            .iter()
            .copied()
            .filter(|id| {
                doc.get(*id)
                    .is_some_and(|e| is_phased_category(e) && e.phase_created.is_none())
            })
            .collect();
        if elements.is_empty() {
            return;
        }
        let cmd = SetElementPhase {
            elements,
            created: Some(Some(phase)),
            demolished: None,
        };
        let ctx = self.ctx();
        let _ = self.session.execute(cmd.into(), &ctx);
    }

    fn renovate(&mut self, status: RenovationStatus) {
        let mut elements: Vec<ElementId> = self.selection.iter().copied().collect();
        elements.sort();
        if elements.is_empty() {
            self.say("Select elements first", true);
            return;
        }
        self.run(SetRenovationStatus { elements, status }.into());
    }

    /// Simple renovation group (ArchiCAD-like): Existing / Demolish / New on
    /// the selection (ADR 0006 §4).
    pub(super) fn renovation_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new("Renovation").strong());
        let any = !self.selection.is_empty();
        for (label, status, tip) in [
            (
                "Existing",
                RenovationStatus::Existing,
                "Mark the selection as existing (created in the first phase)",
            ),
            (
                "Demolish",
                RenovationStatus::Demolish,
                "Mark the selection as demolished in the last phase",
            ),
            (
                "New",
                RenovationStatus::New,
                "Mark the selection as new (created in the last phase)",
            ),
        ] {
            let b = egui::Button::new(label).min_size(Vec2::new(124.0, 26.0));
            if ui
                .add_enabled(any, b)
                .on_hover_text(tip)
                .on_disabled_hover_text("Select elements first")
                .clicked()
            {
                self.renovate(status);
            }
        }
    }

    /// Side panel section: the view's phase and the phase list.
    pub(super) fn phases_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Phases");
        let phases: Vec<(ElementId, String)> = self
            .session
            .doc()
            .phases()
            .iter()
            .map(|p| (p.id, p.label()))
            .collect();
        if phases.is_empty() {
            ui.label("No phases. Use Existing / Demolish / New, or add one.");
        } else {
            let current = self.view_phase();
            let label = phases
                .iter()
                .find(|(id, _)| Some(*id) == current)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            let mut pick = current;
            ui.horizontal(|ui| {
                ui.label("View phase");
                egui::ComboBox::from_id_salt("view_phase")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (id, name) in &phases {
                            ui.selectable_value(&mut pick, Some(*id), name);
                        }
                    });
            });
            if pick != current {
                self.set_view_phase(pick);
            }
            let mut filter = self.phasing.filter;
            ui.horizontal(|ui| {
                ui.label("Phase filter");
                egui::ComboBox::from_id_salt("phase_filter")
                    .selected_text(filter.label())
                    .show_ui(ui, |ui| {
                        for f in PhaseFilter::ALL {
                            ui.selectable_value(&mut filter, f, f.label());
                        }
                    });
            });
            if filter != self.phasing.filter {
                self.set_view_phase_filter(filter);
            }
            ui.label(
                RichText::new("Existing: grey · Demolished: red, dashed · Future: hidden")
                    .weak()
                    .small(),
            );
        }
        if ui.button("Add phase").clicked() {
            let n = phases.len() + 1;
            let mut name = format!("Phase {n}");
            let mut k = n;
            while phases.iter().any(|(_, p)| *p == name) {
                k += 1;
                name = format!("Phase {k}");
            }
            self.run(
                CreatePhase {
                    name,
                    before: None,
                    start_date: None,
                    end_date: None,
                }
                .into(),
            );
        }
    }

    /// Properties rows: phase created / demolished pickers (inside the
    /// Properties grid). Only for phased elements in a phased document.
    pub(super) fn element_phase_rows(&mut self, ui: &mut egui::Ui, e: &ferro_core::Element) {
        if !is_phased_category(e) {
            return;
        }
        let phases: Vec<(ElementId, String)> = self
            .session
            .doc()
            .phases()
            .iter()
            .map(|p| (p.id, p.label()))
            .collect();
        if phases.is_empty() {
            return;
        }
        let name = |id: Option<ElementId>, none: &str| {
            id.and_then(|i| phases.iter().find(|(p, _)| *p == i))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| none.to_string())
        };
        let mut created = e.phase_created;
        let mut demolished = e.phase_demolished;
        for (label, value, none, salt) in [
            (
                "Phase created",
                &mut created,
                "— (existing)",
                "phase_created",
            ),
            (
                "Phase demolished",
                &mut demolished,
                "None",
                "phase_demolished",
            ),
        ] {
            ui.label(label);
            egui::ComboBox::from_id_salt(salt)
                .selected_text(name(*value, none))
                .show_ui(ui, |ui| {
                    ui.selectable_value(value, None, none);
                    for (id, n) in &phases {
                        ui.selectable_value(value, Some(*id), n);
                    }
                });
            ui.end_row();
        }
        if let Some(st) = self
            .view_phase()
            .map(|v| self.session.doc().phase_status(e, v))
        {
            ui.label("Phase status");
            ui.label(st.as_str());
            ui.end_row();
        }
        if created != e.phase_created || demolished != e.phase_demolished {
            self.run(
                SetElementPhase {
                    elements: vec![e.id],
                    created: Some(created),
                    demolished: Some(demolished),
                }
                .into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    #[test]
    fn voids_and_phased_cuts_for_display() {
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        let mut run = |tool: &str, args: serde_json::Value| {
            s.execute(
                AnyCommand::from_tool(tool, &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap()
            .output
        };
        let id = |o: serde_json::Value| ElementId(o["element"].as_u64().unwrap());
        let p1 = id(run("create_phase", json!({"name": "Existing"})));
        let p2 = id(run("create_phase", json!({"name": "New Construction"})));
        let a = id(run(
            "create_box",
            json!({"origin": {"x": 0, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
        ));
        let c = id(run(
            "create_box",
            json!({"origin": {"x": 200, "y": 200, "z": 500}, "width": 200, "depth": 200, "height": 900}),
        ));
        run(
            "create_relationship",
            json!({"kind": "cut", "elements": [c, a], "phase": p2}),
        );
        let doc = s.doc();
        assert_eq!(voids(doc), [c].into_iter().collect());
        // Hidden unless shown, selected, or what it cuts is selected.
        let vs = voids(doc);
        let none = HashSet::new();
        assert_eq!(hidden_voids(doc, &vs, &none, false), [c]);
        assert!(hidden_voids(doc, &vs, &none, true).is_empty());
        assert!(hidden_voids(doc, &vs, &[c].into_iter().collect(), false).is_empty());
        assert!(hidden_voids(doc, &vs, &[a].into_iter().collect(), false).is_empty());
        assert_eq!(
            hidden_voids(doc, &vs, &[p1].into_iter().collect(), false),
            [c]
        );
        // No kernel here: the stored Form is uncut, and so is every phase.
        let all = PhaseFilter::ShowAll;
        assert!(phased_cut_doc(doc, None, all, None).is_none());
        assert!(
            phased_cut_doc(doc, Some(p2), all, None).is_none(),
            "the cut's phase, no kernel: no removed part"
        );
        let early = phased_cut_doc(doc, Some(p1), all, None).expect("differs before the cut");
        assert_eq!(early.doc.get(a).unwrap().as_form().unwrap().cut_body, None);
        assert_eq!(early.doc.relationships().count(), 1);
        assert!(early.parts.is_empty());
    }

    /// An existing wall (4000 x 200 x 3000) cut in the second phase by a
    /// box through it: a 1000 x 1200 window.
    fn phased_window() -> (Session, ElementId, ElementId, ElementId) {
        let k = std::sync::Arc::new(ferro_kernel::SolidcoreKernel::new());
        let mut s =
            Session::new(Document::new("T"), Box::new(SequentialIds::new(1))).with_kernel(k);
        let mut run = |tool: &str, args: serde_json::Value| {
            s.execute(
                AnyCommand::from_tool(tool, &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap()
            .output
        };
        let id = |o: serde_json::Value| ElementId(o["element"].as_u64().unwrap());
        let p1 = id(run("create_phase", json!({"name": "Existing"})));
        let p2 = id(run("create_phase", json!({"name": "New Construction"})));
        let w = id(run(
            "create_wall",
            json!({"start": {"x": 0, "y": 0, "z": 0}, "end": {"x": 4000, "y": 0, "z": 0}}),
        ));
        let c = id(run(
            "create_box",
            json!({"origin": {"x": 1000, "y": -500, "z": 800}, "width": 1000, "depth": 1000, "height": 1200}),
        ));
        run(
            "create_relationship",
            json!({"kind": "cut", "elements": [c, w], "phase": p2}),
        );
        (s, p1, p2, w)
    }

    fn volume_of(doc: &Document, id: ElementId) -> f64 {
        doc.get(id)
            .and_then(|e| e.geometry())
            .map(|b| b.volume().abs())
            .unwrap_or(0.0)
    }

    #[test]
    fn the_removed_part_follows_the_phase_filter_as_demolished() {
        let (s, p1, p2, w) = phased_window();
        let k = s.kernel().map(|k| k.as_ref());
        let doc = s.doc();
        let wall = 4000.0 * 200.0 * 3000.0;
        let window = 1000.0 * 200.0 * 1200.0;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-3 * b;
        assert!(close(volume_of(doc, w), wall - window), "stored: cut");
        // Show All in the cut's phase: one display-only part, the window.
        let shown = phased_cut_doc(doc, Some(p2), PhaseFilter::ShowAll, k).expect("a part");
        assert_eq!(shown.parts.len(), 1);
        let (part, from) = shown.parts[0];
        assert_eq!(from, w);
        assert!(!doc.contains(part), "not in the session's document");
        assert!(close(volume_of(&shown.doc, part), window));
        assert!(close(volume_of(&shown.doc, w), wall - window));
        assert_eq!(
            shown.doc.get(part).unwrap().category,
            doc.get(w).unwrap().category
        );
        // Show Previous + Demo and Show Demo + New show it too.
        for f in [
            PhaseFilter::ShowPreviousAndDemo,
            PhaseFilter::ShowDemoAndNew,
        ] {
            let d = phased_cut_doc(doc, Some(p2), f, k).expect("a part");
            assert_eq!(d.parts.len(), 1, "{f:?}");
        }
        // Show Complete / Show New / Show Previous + New: hidden, nothing
        // differs from the stored (cut) document.
        for f in [
            PhaseFilter::ShowComplete,
            PhaseFilter::ShowNew,
            PhaseFilter::ShowPreviousAndNew,
        ] {
            assert!(phased_cut_doc(doc, Some(p2), f, k).is_none(), "{f:?}");
        }
        // No overrides: drawn as it was before the phase, no part.
        let plain = phased_cut_doc(doc, Some(p2), PhaseFilter::NoOverrides, k).expect("uncut");
        assert!(plain.parts.is_empty());
        assert!(close(volume_of(&plain.doc, w), wall));
        // Before the cut's phase: uncut, no part. Without a kernel: none.
        let early = phased_cut_doc(doc, Some(p1), PhaseFilter::ShowAll, k).expect("uncut");
        assert!(early.parts.is_empty());
        assert!(close(volume_of(&early.doc, w), wall));
        assert!(phased_cut_doc(doc, Some(p2), PhaseFilter::ShowAll, None).is_none());
    }

    #[test]
    fn a_wall_new_in_the_cuts_phase_has_no_demolished_part() {
        let (mut s, _, p2, w) = phased_window();
        s.execute(
            AnyCommand::from_tool(
                "set_element_phase",
                &json!({"elements": [w], "created": p2}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let k = s.kernel().map(|k| k.as_ref());
        assert!(phased_cut_doc(s.doc(), Some(p2), PhaseFilter::ShowAll, k).is_none());
    }

    #[test]
    fn demolished_parts_are_drawn_red_dashed_and_hidden_with_their_element() {
        use crate::scene::{SceneCache, DEMOLISHED, DEMOLISHED_EDGE};
        let (s, _, p2, w) = phased_window();
        let k = s.kernel().map(|k| k.as_ref());
        let shown = phased_cut_doc(s.doc(), Some(p2), PhaseFilter::ShowAll, k).unwrap();
        let part = shown.parts[0].0;
        let mut c = SceneCache::default();
        let none = HashSet::new();
        mark_parts(&shown.parts, &mut c.hidden, &mut c.phase);
        assert_eq!(c.phase.get(&part), Some(&PhaseStatus::Demolished));
        c.update(&shown.doc, ferro_core::Point3::ORIGIN, &none);
        assert!(c.tris.iter().any(|v| v.color == DEMOLISHED));
        assert!(c.lines.iter().any(|v| v.color == DEMOLISHED_EDGE));
        // Translucent faces come last.
        assert_eq!(c.tris.last().map(|v| v.color), Some(DEMOLISHED));
        // Hiding the wall hides its removed part.
        let mut hidden: HashSet<ElementId> = [w].into_iter().collect();
        let mut looks = HashMap::new();
        mark_parts(&shown.parts, &mut hidden, &mut looks);
        assert!(hidden.contains(&part) && looks.is_empty());
    }

    #[test]
    fn phase_display_hides_future_and_marks_existing_and_demolished() {
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        let mut run = |tool: &str, args: serde_json::Value| {
            s.execute(
                AnyCommand::from_tool(tool, &args).unwrap(),
                &ExecContext::user(),
            )
            .unwrap()
            .output
        };
        let mut boxes = Vec::new();
        for x in [0, 2000, 4000, 6000] {
            let o = run(
                "create_box",
                json!({"origin": {"x": x, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
            );
            boxes.push(ElementId(o["element"].as_u64().unwrap()));
        }
        // No phases: nothing changes.
        assert!(
            phase_display(&Document::new("x"), None, PhaseFilter::ShowAll)
                == PhaseDisplay::default()
        );
        for (b, st) in boxes.iter().zip(["existing", "demolish", "new"]) {
            run(
                "set_renovation_status",
                json!({"elements": [b], "status": st}),
            );
        }
        let doc = s.doc();
        let ph: Vec<ElementId> = doc.phases().iter().map(|p| p.id).collect();
        let last = phase_display(doc, Some(ph[1]), PhaseFilter::ShowAll);
        assert_eq!(last.looks.get(&boxes[0]), Some(&PhaseStatus::Existing));
        assert_eq!(last.looks.get(&boxes[1]), Some(&PhaseStatus::Demolished));
        assert_eq!(last.looks.get(&boxes[2]), None, "new is drawn normally");
        assert_eq!(last.looks.get(&boxes[3]), Some(&PhaseStatus::Existing));
        assert!(last.hidden.is_empty());
        let first = phase_display(doc, Some(ph[0]), PhaseFilter::ShowAll);
        assert!(first.hidden.contains(&boxes[2]), "new box is future");
        assert_eq!(first.hidden.len(), 1);
    }

    #[test]
    fn phase_filters_hide_and_override_per_status() {
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        let mut boxes = Vec::new();
        for (x, st) in [(0, "existing"), (2000, "demolish"), (4000, "new")] {
            let o = s
                .execute(
                    AnyCommand::from_tool(
                        "create_box",
                        &json!({"origin": {"x": x, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000}),
                    )
                    .unwrap(),
                    &ExecContext::user(),
                )
                .unwrap();
            let b = ElementId(o.output["element"].as_u64().unwrap());
            s.execute(
                AnyCommand::from_tool(
                    "set_renovation_status",
                    &json!({"elements": [b], "status": st}),
                )
                .unwrap(),
                &ExecContext::user(),
            )
            .unwrap();
            boxes.push(b);
        }
        let doc = s.doc();
        let last = doc.phases().last().map(|p| p.id);
        let complete = phase_display(doc, last, PhaseFilter::ShowComplete);
        assert!(complete.hidden.contains(&boxes[1]), "demolished not shown");
        assert!(complete.looks.is_empty(), "drawn normally");
        let demo = phase_display(doc, last, PhaseFilter::ShowPreviousAndDemo);
        assert!(demo.hidden.contains(&boxes[2]), "new not shown");
        assert_eq!(demo.looks.get(&boxes[1]), Some(&PhaseStatus::Demolished));
        let plain = phase_display(doc, last, PhaseFilter::NoOverrides);
        assert!(plain.hidden.is_empty() && plain.looks.is_empty());
    }
}
