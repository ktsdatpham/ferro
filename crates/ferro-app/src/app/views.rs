//! Views in the app (D-33, M5-22): the screen shows the *active view*, an
//! element of category `Views`; the Views panel creates, renames, switches
//! and deletes views.
//!
//! Rules:
//! * The app keeps working copies of the view state (camera, display
//!   style, hidden elements and tags, cut, phase, phase filter) and reloads
//!   them whenever the active view element changes (undo / redo, agents).
//! * Every user change to the view state except the camera is one
//!   `set_view` command = one undo step.
//! * Orbit / pan / zoom never create undo steps: the camera is written to
//!   the view only by *Save camera*, or when the user switches to another
//!   view (then the old view keeps where the user left it, as one
//!   `set_view` step, only if the camera moved).
//! * A document without views shows a default 3D view that lives in
//!   memory only, so opening and looking at a file never changes it. The
//!   first view change (style, hide, section, phase...) creates it as a
//!   real view "3D View" in the same undo step. Undoing that step goes
//!   back to the in-memory default.
//! * Plan views cut at their view range (level + cut height) and section
//!   views at their plane; the free section tools work on 3D views.

use super::FerroApp;
use crate::camera::OrbitCamera;
use crate::tools::SectionPlane;
use eframe::egui::{self, RichText};
use ferro_core::math::tol;
use ferro_core::model::{DisplayStyle, PhaseFilter, Projection, View, ViewCamera, ViewType};
use ferro_core::ops::view_commands::{CreateView, DeleteView, SetView, ViewTypeName};
use ferro_core::ops::Outcome;
use ferro_core::{Angle, AnyCommand, Document, ElementId, UnitVec3, Vec3};

/// View state of the app (not saved itself: views are elements).
#[derive(Default)]
pub struct ViewState {
    /// The active view element; `None` = the in-memory default 3D view
    /// (only while the document has no views).
    active: Option<ElementId>,
    /// The view data last applied to the screen, to notice changes made by
    /// undo, redo or agents.
    loaded: Option<View>,
    /// Name typed for the active view.
    rename: Option<(ElementId, String)>,
}

/// The app's orbit camera as a view camera (eye / target / up, mm).
pub fn camera_to_view(c: &OrbitCamera) -> ViewCamera {
    // Straight down / up: the up direction follows the yaw, as in
    // `OrbitCamera::view_proj`.
    let up = if c.pitch.abs() > 1.54 {
        Vec3::new(-c.yaw.cos(), -c.yaw.sin(), 0.0)
            .normalize()
            .unwrap_or(UnitVec3::Y)
    } else {
        UnitVec3::Z
    };
    ViewCamera {
        eye: c.eye(),
        target: c.target,
        up,
        projection: if c.parallel {
            Projection::Parallel
        } else {
            Projection::Perspective
        },
        fov_y: Angle::from_rad(c.fov_y),
    }
}

/// A view camera as the app's orbit camera. Pitch is limited to ±1.55 rad
/// like orbiting; for a camera looking straight down the yaw comes from
/// the up direction.
pub fn camera_from_view(v: &ViewCamera) -> OrbitCamera {
    let d = v.eye - v.target;
    let distance = d.length().max(50.0);
    let pitch = (d.z / distance).clamp(-1.0, 1.0).asin();
    let horizontal = (d.x * d.x + d.y * d.y).sqrt();
    let yaw = if horizontal > tol::VERTEX_TOL {
        d.y.atan2(d.x)
    } else {
        let u = v.up.as_vec();
        (-u.y).atan2(-u.x)
    };
    OrbitCamera {
        target: v.target,
        yaw,
        pitch: pitch.clamp(-1.55, 1.55),
        distance,
        fov_y: v.fov_y.rad(),
        parallel: v.projection == Projection::Parallel,
    }
}

/// `true` if two cameras are the same within the tolerances (VERTEX_TOL on
/// points, ANGLE_TOL on directions and field of view).
pub fn same_camera(a: &ViewCamera, b: &ViewCamera) -> bool {
    a.eye.is_almost_equal(b.eye)
        && a.target.is_almost_equal(b.target)
        && (a.up.as_vec() - b.up.as_vec()).length() <= tol::ANGLE_TOL
        && a.projection == b.projection
        && tol::eq_angle(a.fov_y.rad(), b.fov_y.rad())
}

/// Display style from the app's three style switches (hidden line wins,
/// then wireframe, then X-ray).
pub fn style_of(xray: bool, wireframe: bool, hidden_line: bool) -> DisplayStyle {
    if hidden_line {
        DisplayStyle::HiddenLine
    } else if wireframe {
        DisplayStyle::Wireframe
    } else if xray {
        DisplayStyle::XRay
    } else {
        DisplayStyle::Shaded
    }
}

/// `base`, or `base 2`, `base 3`… : the first name no view has.
pub fn unique_view_name(doc: &Document, base: &str) -> String {
    let taken = |n: &str| doc.views().iter().any(|v| v.name.as_deref() == Some(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// Short label of a view type for the list.
fn type_label(t: &ViewType) -> &'static str {
    match t {
        ViewType::View3D { .. } => "3D",
        ViewType::PlanView { .. } => "Plan",
        ViewType::SectionView { .. } => "Section",
    }
}

impl FerroApp {
    /// Forget the active view (new / opened document): the next frame
    /// picks the document's first view, or the in-memory default.
    pub(super) fn reset_views(&mut self) {
        self.views = ViewState::default();
        self.apply_default_view_state();
    }

    /// Default view state (in-memory view): everything shown, shaded, no
    /// cut, last phase. The camera is kept.
    fn apply_default_view_state(&mut self) {
        self.hidden.clear();
        self.hidden_tags.clear();
        self.section = None;
        self.scene.xray = false;
        self.scene.wireframe = false;
        self.hidden_line = false;
        self.phasing.pick = None;
        self.phasing.filter = PhaseFilter::ShowAll;
    }

    /// Display style currently on screen.
    pub(super) fn display_style(&self) -> DisplayStyle {
        style_of(self.scene.xray, self.scene.wireframe, self.hidden_line)
    }

    /// Once per frame: follow the active view element. Reloads the screen
    /// state when the view changed (camera only if the view's camera did).
    pub(super) fn sync_view(&mut self) {
        let doc = self.session.doc();
        let is_view = |id: ElementId| doc.get(id).and_then(|e| e.as_view()).is_some();
        let active = self.views.active.filter(|v| is_view(*v)).or_else(|| {
            let views = doc.views();
            views
                .iter()
                .find(|e| matches!(e.as_view(), Some(v) if matches!(v.view_type, ViewType::View3D { .. })))
                .or(views.first())
                .map(|e| e.id)
        });
        if active != self.views.active {
            self.views.active = active;
            self.views.rename = None;
        }
        let Some(id) = active else {
            if self.views.loaded.take().is_some() {
                // The last view went away (deleted, or its creation undone).
                self.apply_default_view_state();
            }
            return;
        };
        let Some(e) = doc.get(id) else { return };
        let Some(v) = e.as_view().cloned() else {
            return;
        };
        if self.views.loaded.as_ref() == Some(&v) {
            return;
        }
        let cut = doc.view_cut_plane(e);
        let camera_changed = self
            .views
            .loaded
            .as_ref()
            .is_none_or(|l| !same_camera(&l.camera, &v.camera));
        self.hidden = v.hidden_elements.iter().copied().collect();
        self.hidden_tags = v.hidden_tags.iter().cloned().collect();
        self.section = cut;
        let s = v.display_style;
        self.scene.xray = s == DisplayStyle::XRay;
        self.scene.wireframe = s == DisplayStyle::Wireframe;
        self.hidden_line = s == DisplayStyle::HiddenLine;
        self.phasing.pick = v.phase;
        self.phasing.filter = v.phase_filter;
        if camera_changed {
            self.camera = camera_from_view(&v.camera);
        }
        self.views.loaded = Some(v);
    }

    /// Execute a view command without touching the selection; errors are
    /// shown in the status bar. Autosaves outside groups.
    fn exec_view(&mut self, cmd: AnyCommand) -> Option<Outcome> {
        if self.proposal.is_some() {
            self.accept_proposal();
        }
        let name = cmd.name();
        let ctx = self.ctx();
        match self.session.execute(cmd, &ctx) {
            Ok(o) => {
                crate::report::record(format!("{name} ok"));
                self.autosave();
                Some(o)
            }
            Err(f) => {
                crate::report::record(format!(
                    "{name} failed {}",
                    f.iter()
                        .map(|x| x.code.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ));
                self.say(super::failure_text(&f), true);
                None
            }
        }
    }

    /// Create a view of `kind` from what is on screen (camera, style, cut,
    /// hidden elements and tags, phase). Returns its id.
    fn create_view_from_screen(&mut self, kind: ViewTypeName, name: String) -> Option<ElementId> {
        let mut cmd = CreateView::view_3d(name, camera_to_view(&self.camera));
        cmd.view_type = kind;
        cmd.display_style = Some(self.display_style());
        cmd.phase = self.phasing.pick;
        cmd.phase_filter = Some(self.phasing.filter);
        match kind {
            ViewTypeName::View3D => cmd.section_plane = self.section.map(Box::new),
            ViewTypeName::SectionView => {
                cmd.section_plane = self.section.map(Box::new);
                cmd.camera = self.section.map(|p| Box::new(ViewCamera::section(&p)));
            }
            ViewTypeName::PlanView => {
                let (level, z) = self.level_info()?;
                cmd.level = Some(level);
                cmd.view_range = Some(Box::default());
                cmd.camera = Some(Box::new(ViewCamera::plan(
                    self.camera.target.x,
                    self.camera.target.y,
                    z,
                )));
                cmd.display_style = Some(DisplayStyle::HiddenLine);
            }
        }
        let opened = self.session.begin_group("create_view", self.ctx());
        let made = self
            .exec_view(cmd.into())
            .and_then(|o| o.output["element"].as_u64())
            .map(ElementId);
        let ok = match made {
            Some(v) if !self.hidden.is_empty() || !self.hidden_tags.is_empty() => {
                let mut set = SetView::new(v);
                set.hide = self.hidden_elements_by_id();
                let mut tags: Vec<String> = self.hidden_tags.iter().cloned().collect();
                tags.sort();
                set.hidden_tags = Some(tags);
                self.exec_view(set.into()).is_some()
            }
            Some(_) => true,
            None => false,
        };
        if opened {
            if ok {
                self.session.end_group();
                self.autosave();
            } else {
                self.session.cancel_group();
            }
        }
        made.filter(|_| ok)
    }

    /// Hidden elements that still exist and can be hidden, sorted.
    fn hidden_elements_by_id(&self) -> Vec<ElementId> {
        let doc = self.session.doc();
        let mut ids: Vec<ElementId> = self
            .hidden
            .iter()
            .copied()
            .filter(|id| {
                doc.get(*id)
                    .is_some_and(|e| e.geometry().is_some() || e.as_level().is_some())
            })
            .collect();
        ids.sort();
        ids
    }

    /// Change the active view as one undo step. Without views, the
    /// in-memory default is first created as "3D View" in the same step.
    pub(super) fn edit_view(&mut self, f: impl FnOnce(&mut SetView)) -> bool {
        if let Some(v) = self.views.active {
            let mut cmd = SetView::new(v);
            f(&mut cmd);
            return self.exec_view(cmd.into()).is_some();
        }
        let opened = self.session.begin_group("set_view", self.ctx());
        let name = unique_view_name(self.session.doc(), "3D View");
        let ok = match self.create_view_from_screen(ViewTypeName::View3D, name) {
            Some(v) => {
                let mut cmd = SetView::new(v);
                f(&mut cmd);
                let ok = self.exec_view(cmd.into()).is_some();
                if ok {
                    self.views.active = Some(v);
                }
                ok
            }
            None => false,
        };
        if opened {
            if ok {
                self.session.end_group();
                self.autosave();
            } else {
                self.session.cancel_group();
            }
        }
        ok
    }

    /// Set the display style of the active view.
    pub(super) fn set_display_style(&mut self, s: DisplayStyle) {
        if s != self.display_style() {
            self.edit_view(|c| c.display_style = Some(s));
        }
    }

    /// Hide elements in the active view (only elements drawn in the model).
    pub(super) fn hide_in_view(&mut self, ids: Vec<ElementId>) {
        let doc = self.session.doc();
        let mut ids: Vec<ElementId> = ids
            .into_iter()
            .filter(|id| {
                doc.get(*id)
                    .is_some_and(|e| e.geometry().is_some() || e.as_level().is_some())
            })
            .collect();
        ids.sort();
        if !ids.is_empty() {
            self.edit_view(|c| c.hide = ids);
        }
    }

    /// Show every element hidden in the active view.
    pub(super) fn unhide_all_in_view(&mut self) {
        if !self.hidden.is_empty() {
            self.edit_view(|c| c.unhide_all = true);
        }
    }

    /// Hide or show the elements with `tag` in the active view.
    pub(super) fn set_tag_hidden(&mut self, tag: &str, hidden: bool) {
        let mut tags: Vec<String> = self.hidden_tags.iter().cloned().collect();
        tags.retain(|t| t != tag);
        if hidden {
            tags.push(tag.to_string());
        }
        tags.sort();
        self.edit_view(|c| c.hidden_tags = Some(tags));
    }

    /// Cut the active view with `p` (or remove its cut). 3D views take any
    /// plane; a section view's plane can move but not go; plan views cut
    /// at their view range.
    pub(super) fn set_view_section(&mut self, p: Option<SectionPlane>) -> bool {
        let kind = self.views.loaded.as_ref().map(|v| v.view_type.clone());
        match (kind, p) {
            (Some(ViewType::PlanView { .. }), _) => {
                self.say(
                    "Plan views cut at their view range; use a 3D view for a free section",
                    true,
                );
                false
            }
            (Some(ViewType::SectionView { .. }), None) => {
                self.say("A section view always cuts; switch to a 3D view", true);
                false
            }
            _ => self.edit_view(|c| c.section_plane = Some(p.map(Box::new))),
        }
    }

    /// Show `phase` in the active view (`None` = the last phase).
    pub(super) fn set_view_phase(&mut self, phase: Option<ElementId>) {
        if phase != self.phasing.pick {
            self.edit_view(|c| c.phase = Some(phase));
        }
    }

    /// Set the active view's phase filter (M5-23.8).
    pub(super) fn set_view_phase_filter(&mut self, f: PhaseFilter) {
        if f != self.phasing.filter {
            self.edit_view(|c| c.phase_filter = Some(f));
        }
    }

    /// Write the camera into the active view (Save camera). One undo step.
    pub(super) fn save_view_camera(&mut self) {
        let cam = camera_to_view(&self.camera);
        if let Err(m) = cam.validate() {
            self.say(m, true);
            return;
        }
        if self.edit_view(|c| c.camera = Some(Box::new(cam))) {
            self.say("Camera saved in the view", false);
        }
    }

    /// The camera moved since the active view was loaded.
    fn camera_moved(&self) -> bool {
        match (&self.views.loaded, self.views.active) {
            (Some(v), Some(_)) => !same_camera(&v.camera, &camera_to_view(&self.camera)),
            _ => false,
        }
    }

    /// Make `id` the active view. The view left keeps the camera the user
    /// left it with (one `set_view` step, only if the camera moved).
    fn switch_view(&mut self, id: ElementId) {
        if Some(id) == self.views.active {
            return;
        }
        if self.camera_moved() {
            let cam = camera_to_view(&self.camera);
            if cam.validate().is_ok() {
                self.edit_view(|c| c.camera = Some(Box::new(cam)));
            }
        }
        self.views.active = Some(id);
        self.views.loaded = None;
        self.views.rename = None;
        self.sync_view();
    }

    /// Side panel section: the views of the project.
    pub(super) fn views_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Views");
        let doc = self.session.doc();
        let rows: Vec<(ElementId, String, &'static str)> = doc
            .views()
            .iter()
            .filter_map(|e| Some((e.id, e.label(), type_label(&e.as_view()?.view_type))))
            .collect();
        if rows.is_empty() {
            ui.label(
                RichText::new("Default 3D view (not saved). Changing the style, hiding or cutting saves it as a view.")
                    .weak()
                    .small(),
            );
        }
        let active = self.views.active;
        let mut switch = None;
        let mut delete = None;
        for (id, name, kind) in &rows {
            ui.horizontal(|ui| {
                let text = format!("{kind} · {name}");
                if ui.selectable_label(active == Some(*id), text).clicked() {
                    switch = Some(*id);
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Delete this view")
                    .clicked()
                {
                    delete = Some(*id);
                }
            });
        }
        if let Some(id) = active {
            let current = rows
                .iter()
                .find(|r| r.0 == id)
                .map(|r| r.1.clone())
                .unwrap_or_default();
            let edit = match &mut self.views.rename {
                Some((v, s)) if *v == id => s,
                _ => {
                    self.views.rename = Some((id, current.clone()));
                    match &mut self.views.rename {
                        Some((_, s)) => s,
                        None => return,
                    }
                }
            };
            let mut commit = None;
            ui.horizontal(|ui| {
                ui.label("Name");
                let r = ui.text_edit_singleline(edit);
                if r.lost_focus() && edit.trim() != current && !edit.trim().is_empty() {
                    commit = Some(edit.trim().to_string());
                }
            });
            if let Some(name) = commit {
                self.views.rename = None;
                self.edit_view(|c| c.name = Some(name));
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("New 3D")
                .on_hover_text("New 3D view from what is on screen")
                .clicked()
            {
                let name = unique_view_name(self.session.doc(), "3D View");
                if let Some(v) = self.create_view_from_screen(ViewTypeName::View3D, name) {
                    self.switch_view(v);
                }
            }
            let level = self.level_info();
            if ui
                .add_enabled(level.is_some(), egui::Button::new("New plan"))
                .on_hover_text("Plan of the active level, cut 1200 mm above it")
                .on_disabled_hover_text("Pick an active level first")
                .clicked()
            {
                let lname = level
                    .and_then(|(l, _)| self.session.doc().get(l))
                    .map(|e| e.label())
                    .unwrap_or_default();
                let name = unique_view_name(self.session.doc(), &format!("Plan {lname}"));
                if let Some(v) = self.create_view_from_screen(ViewTypeName::PlanView, name) {
                    self.switch_view(v);
                }
            }
            let free_cut = self.section.is_some()
                && !matches!(
                    self.views.loaded.as_ref().map(|v| &v.view_type),
                    Some(ViewType::PlanView { .. })
                );
            if ui
                .add_enabled(free_cut, egui::Button::new("New section"))
                .on_hover_text("Section view from the current section plane")
                .on_disabled_hover_text("Cut the view with the Section tool first")
                .clicked()
            {
                let name = unique_view_name(self.session.doc(), "Section");
                if let Some(v) = self.create_view_from_screen(ViewTypeName::SectionView, name) {
                    self.switch_view(v);
                }
            }
            if ui
                .button("Save camera")
                .on_hover_text(
                    "Orbiting does not change the view; this stores the camera in it (undoable)",
                )
                .clicked()
            {
                self.save_view_camera();
            }
        });
        if let Some(id) = switch {
            self.switch_view(id);
        }
        if let Some(id) = delete {
            self.exec_view(DeleteView { view: id }.into());
        }
    }

    /// Properties rows of a view element.
    pub(super) fn view_rows(&self, ui: &mut egui::Ui, v: &View) {
        ui.label("View type");
        ui.label(type_label(&v.view_type));
        ui.end_row();
        ui.label("Style");
        ui.label(v.display_style.label());
        ui.end_row();
        ui.label("Hidden");
        ui.label(format!(
            "{} element(s), {} tag(s)",
            v.hidden_elements.len(),
            v.hidden_tags.len()
        ));
        ui.end_row();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{ExecContext, SequentialIds};
    use ferro_core::{Point3, Session};
    use serde_json::json;

    #[test]
    fn camera_round_trips_through_the_view() {
        let c = OrbitCamera {
            target: Point3::new(1000.0, -2000.0, 500.0),
            yaw: 0.7,
            pitch: 0.4,
            distance: 12_000.0,
            fov_y: 0.9,
            parallel: true,
        };
        let v = camera_to_view(&c);
        v.validate().unwrap();
        let back = camera_from_view(&v);
        assert!(same_camera(&camera_to_view(&back), &v));
        assert!((back.yaw - 0.7).abs() < 1e-9 && (back.pitch - 0.4).abs() < 1e-9);
        assert!(back.parallel);
    }

    #[test]
    fn top_down_cameras_keep_their_up_direction() {
        // A plan camera (straight down, north up) becomes a top orbit view
        // whose screen-up is +Y.
        let plan = ViewCamera::plan(0.0, 0.0, 3000.0);
        let o = camera_from_view(&plan);
        assert!(
            (o.pitch - 1.55).abs() < 1e-12,
            "pitch limited like orbiting"
        );
        let v = camera_to_view(&o);
        assert!((v.up.as_vec() - Vec3::Y).length() < 1e-9, "{:?}", v.up);
        // Degenerate: a zero-distance view camera still gives a usable orbit.
        let mut z = plan;
        z.eye = z.target;
        assert!(camera_from_view(&z).distance >= 50.0);
    }

    #[test]
    fn styles_and_names() {
        assert_eq!(style_of(false, false, false), DisplayStyle::Shaded);
        assert_eq!(style_of(true, false, false), DisplayStyle::XRay);
        assert_eq!(style_of(true, true, false), DisplayStyle::Wireframe);
        assert_eq!(style_of(true, true, true), DisplayStyle::HiddenLine);
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        assert_eq!(unique_view_name(s.doc(), "3D View"), "3D View");
        for n in ["3D View", "3D View 2"] {
            s.execute(
                AnyCommand::from_tool("create_view", &json!({"name": n, "view_type": "view_3d"}))
                    .unwrap(),
                &ExecContext::user(),
            )
            .unwrap();
        }
        assert_eq!(unique_view_name(s.doc(), "3D View"), "3D View 3");
    }

    #[test]
    fn camera_tolerance_edge() {
        let a = ViewCamera::default_3d();
        let mut b = a;
        b.eye.x += tol::VERTEX_TOL * 0.5;
        assert!(same_camera(&a, &b));
        b.eye.x += tol::VERTEX_TOL * 2.0;
        assert!(!same_camera(&a, &b));
    }
}
