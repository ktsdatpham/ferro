//! Wall end joins in the app (M5-5.2): the ends of selected walls get
//! small square handles, labelled "no join" when disallowed, and the
//! viewport's context menu offers "Disallow join" / "Allow join" per end
//! (Revit's wall end context menu). Join Geometry, Unjoin Geometry and
//! Switch Join Order are solid tools (`tools::join_command`). Plain-text
//! labels only (the app font has no symbol glyphs).

use super::FerroApp;
use eframe::egui::{self, Color32, Stroke, Vec2};
use ferro_core::model::bim::WallJoin;
use ferro_core::model::generate;
use ferro_core::ops::bim_commands::SetWall;
use ferro_core::{Element, ElementId, Point3};
use ferro_snap::Projector;

/// `set_wall` changing only one end's join state.
fn set_join(wall: ElementId, at_start: bool, state: WallJoin) -> SetWall {
    SetWall {
        wall,
        start: None,
        end: None,
        mid: None,
        base_offset: None,
        height: None,
        top_level: None,
        top_offset: None,
        location_line: None,
        join_start: at_start.then_some(state),
        join_end: (!at_start).then_some(state),
    }
}

/// The base points of a wall's curve ends, with their join states.
fn wall_ends(doc: &ferro_core::Document, e: &Element) -> Option<[(Point3, WallJoin); 2]> {
    let w = e.as_wall()?;
    let z = generate::wall_layout(doc, e, w).ok()?.z0;
    let at = |p: Point3| Point3::new(p.x, p.y, z);
    Some([
        (at(w.curve.start()), w.join_start),
        (at(w.curve.end()), w.join_end),
    ])
}

impl FerroApp {
    /// The single selected wall, if the selection is exactly one wall.
    fn selected_wall(&self) -> Option<ElementId> {
        let mut it = self.selection.iter();
        let (Some(id), None) = (it.next(), it.next()) else {
            return None;
        };
        self.doc().get(*id).and_then(|e| e.as_wall()).map(|_| *id)
    }

    /// "Disallow join" / "Allow join" for each end of the selected wall, in
    /// the viewport's context menu.
    pub(super) fn join_menu(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_wall() else {
            return;
        };
        let Some(ends) = self.doc().get(id).and_then(|e| wall_ends(self.doc(), e)) else {
            return;
        };
        ui.separator();
        for (at_start, (_, state)) in [true, false].into_iter().zip(ends) {
            let end = if at_start { "start" } else { "end" };
            let (label, next) = match state {
                WallJoin::Auto => (format!("Disallow join at {end}"), WallJoin::Disallow),
                WallJoin::Disallow => (format!("Allow join at {end}"), WallJoin::Auto),
            };
            if ui
                .button(label)
                .on_hover_text("A disallowed end stays square and never joins other walls (Revit: Disallow Join)")
                .clicked()
            {
                self.run(set_join(id, at_start, next).into());
                ui.close();
            }
        }
    }

    /// Square handles on the ends of selected walls; "no join" next to a
    /// disallowed end.
    pub(super) fn wall_end_handles(&self, painter: &egui::Painter, fc: &dyn Projector) {
        let doc = self.doc();
        let mut ids: Vec<ElementId> = self.selection.iter().copied().collect();
        ids.sort();
        for e in ids.iter().filter_map(|id| doc.get(*id)) {
            let Some(ends) = wall_ends(doc, e) else {
                continue;
            };
            for (p, state) in ends {
                let Some(q) = fc.project(p) else { continue };
                let c = egui::pos2(q[0], q[1]);
                let color = match state {
                    WallJoin::Auto => Color32::from_rgb(30, 110, 200),
                    WallJoin::Disallow => Color32::from_rgb(190, 60, 40),
                };
                painter.rect_stroke(
                    egui::Rect::from_center_size(c, Vec2::splat(9.0)),
                    0.0,
                    Stroke::new(1.5, color),
                    egui::StrokeKind::Middle,
                );
                if state == WallJoin::Disallow {
                    painter.text(
                        c + Vec2::new(8.0, 8.0),
                        egui::Align2::LEFT_TOP,
                        "no join",
                        egui::FontId::proportional(12.0),
                        color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_join_changes_one_end_only() {
        let c = set_join(ElementId(4), false, WallJoin::Disallow);
        assert_eq!((c.join_start, c.join_end), (None, Some(WallJoin::Disallow)));
        assert!(c.start.is_none() && c.height.is_none());
    }
}
