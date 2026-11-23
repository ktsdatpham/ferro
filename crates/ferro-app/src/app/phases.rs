//! Phases in the app (ADR 0006, M5-23): the view's phase (app state until
//! views become elements, M5-22), the simple renovation buttons, phase
//! pickers in Properties and the per-status graphics handed to the scene.
//!
//! Rules:
//! * a document without phases shows everything as usual;
//! * with phases, the view shows one phase (the last by default): future
//!   and already demolished elements are hidden, existing ones greyed,
//!   demolished / temporary ones translucent red with dashed edges;
//! * elements created while phasing is used get the view's phase (Revit
//!   behaviour), in the same undo step as the command that made them.

use super::FerroApp;
use eframe::egui::{self, RichText, Vec2};
use ferro_core::model::phase::{is_phased_category, phase_status};
use ferro_core::model::PhaseStatus;
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

/// Phase view state of the app (not saved).
#[derive(Default)]
pub struct PhaseState {
    /// Phase picked for the view; `None` = the last phase.
    pick: Option<ElementId>,
    /// Display computed for (document, view phase).
    cache: Option<(Document, Option<ElementId>, PhaseDisplay)>,
}

/// Phase display of `doc` in view phase `view`: pure, used by the app and
/// its tests.
pub fn phase_display(doc: &Document, view: Option<ElementId>) -> PhaseDisplay {
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
        match st {
            PhaseStatus::Future | PhaseStatus::Past => {
                out.hidden.insert(e.id);
            }
            PhaseStatus::New => {}
            _ => {
                out.looks.insert(e.id, st);
            }
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
        let doc = self.session.doc();
        if let Some((d, v, shown)) = &self.phasing.cache {
            if *v == view && crate::scene::same_doc(d, doc) {
                return shown.clone();
            }
        }
        let shown = phase_display(doc, view);
        self.phasing.cache = Some((doc.clone(), view, shown.clone()));
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
                self.phasing.pick = pick;
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
        assert!(phase_display(&Document::new("x"), None) == PhaseDisplay::default());
        for (b, st) in boxes.iter().zip(["existing", "demolish", "new"]) {
            run(
                "set_renovation_status",
                json!({"elements": [b], "status": st}),
            );
        }
        let doc = s.doc();
        let ph: Vec<ElementId> = doc.phases().iter().map(|p| p.id).collect();
        let last = phase_display(doc, Some(ph[1]));
        assert_eq!(last.looks.get(&boxes[0]), Some(&PhaseStatus::Existing));
        assert_eq!(last.looks.get(&boxes[1]), Some(&PhaseStatus::Demolished));
        assert_eq!(last.looks.get(&boxes[2]), None, "new is drawn normally");
        assert_eq!(last.looks.get(&boxes[3]), Some(&PhaseStatus::Existing));
        assert!(last.hidden.is_empty());
        let first = phase_display(doc, Some(ph[0]));
        assert!(first.hidden.contains(&boxes[2]), "new box is future");
        assert_eq!(first.hidden.len(), 1);
    }
}
