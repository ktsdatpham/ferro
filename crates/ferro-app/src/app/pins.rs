//! Pins in the app (ADR 0008, M5-27.5): a "Pinned" row with a toggle in
//! Properties, Pin / Unpin in the viewport's context menu and on the `PN` /
//! `UP` codes, and a "Pinned" marker on selected pinned elements.
//!
//! The app enforces nothing itself: a drag or edit of a pinned element is
//! previewed and run like any other, the `Transaction` refuses it with
//! `ferro.model.pinned`, the preview shows nothing moving and the refusal
//! reaches the status line. Only plain-text labels are drawn (the app font
//! has no pin glyph).

use super::FerroApp;
use eframe::egui::{self, Color32, Vec2};
use ferro_core::ops::pin_commands::SetPinned;
use ferro_core::{Element, ElementId};
use ferro_snap::Projector;

impl FerroApp {
    /// Pin or unpin the selection: one `set_pinned`, one undo step.
    pub(super) fn pin_selection(&mut self, pinned: bool) {
        let mut ids: Vec<ElementId> = self.selection.iter().copied().collect();
        if ids.is_empty() {
            self.say(
                if pinned {
                    "Select the elements to pin"
                } else {
                    "Select the elements to unpin"
                },
                false,
            );
            return;
        }
        ids.sort();
        self.run(
            SetPinned {
                elements: ids,
                pinned,
            }
            .into(),
        );
    }

    /// The "Pinned" row of the Properties grid, with its toggle.
    pub(super) fn pinned_row(&mut self, ui: &mut egui::Ui, e: &Element) {
        ui.label("Pinned");
        let mut pinned = e.pinned;
        let text = if e.pinned { "yes (locked)" } else { "no" };
        let tip =
            "A pinned element cannot be moved or deleted until it is unpinned (PN / UP); other edits stay allowed";
        if ui.checkbox(&mut pinned, text).on_hover_text(tip).changed() {
            self.run(
                SetPinned {
                    elements: vec![e.id],
                    pinned,
                }
                .into(),
            );
        }
        ui.end_row();
    }

    /// Pin / Unpin entries of the viewport's context menu (right click).
    pub(super) fn pin_menu(&mut self, ui: &mut egui::Ui) {
        let doc = self.doc();
        let any = |p: bool| {
            self.selection
                .iter()
                .any(|id| doc.get(*id).is_some_and(|e| e.pinned == p))
        };
        let (can_pin, can_unpin) = (any(false), any(true));
        if ui
            .add_enabled(can_pin, egui::Button::new("Pin").shortcut_text("PN"))
            .clicked()
        {
            self.pin_selection(true);
            ui.close();
        }
        if ui
            .add_enabled(can_unpin, egui::Button::new("Unpin").shortcut_text("UP"))
            .clicked()
        {
            self.pin_selection(false);
            ui.close();
        }
    }

    /// "Pinned" next to every selected pinned element that has geometry,
    /// and a corner note naming them all (levels have no 3D marker).
    pub(super) fn pinned_markers(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        fc: &dyn Projector,
    ) {
        let doc = self.doc();
        let mut names: Vec<String> = Vec::new();
        let mut ids: Vec<ElementId> = self.selection.iter().copied().collect();
        ids.sort();
        for e in ids
            .iter()
            .filter_map(|id| doc.get(*id))
            .filter(|e| e.pinned)
        {
            names.push(e.label());
            let Some(bb) = e.bounding_box() else { continue };
            let Some(p) = fc.project(bb.max) else {
                continue;
            };
            painter.text(
                egui::pos2(p[0], p[1]) + Vec2::new(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                "Pinned",
                egui::FontId::proportional(12.0),
                Color32::from_rgb(170, 90, 0),
            );
        }
        if names.is_empty() {
            return;
        }
        let shown = if names.len() > 3 {
            format!("{}, ... ({} pinned)", names[..3].join(", "), names.len())
        } else {
            names.join(", ")
        };
        // Second line: the first holds the workspace / view label.
        painter.text(
            rect.right_top() + Vec2::new(-10.0, 28.0),
            egui::Align2::RIGHT_TOP,
            format!("Pinned (locked): {shown}"),
            egui::FontId::proportional(13.0),
            Color32::from_rgb(170, 90, 0),
        );
    }
}
