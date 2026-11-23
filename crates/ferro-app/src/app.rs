//! The Ferro window: menus, workspaces, toolbar, viewport, levels and
//! properties panels, status bar with the value box (Q-R7, Q-R11, Q-R13).

use crate::camera::{FrameCamera, OrbitCamera};
use crate::render::{FrameData, Renderer, Vertex};
use crate::scene::{self, line_vertex, SceneCache};
use crate::tools::{SelectOp, ToolCtx, ToolEvent, ToolKind, ToolState};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, RichText, Sense, Vec2};
use eframe::wgpu;
use ferro_core::model::{keys, Category, ElementBody};
use ferro_core::ops::commands::*;
use ferro_core::ops::{ExecContext, Failure};
use ferro_core::{AnyCommand, Document, ElementId, Length, ParamValue, Point3, Session};
use ferro_io::FroStore;
use ferro_snap::{snap, Projector, SnapKind, SnapResult, SnapScene, SnapSettings};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workspace {
    Sketch,
    Bim,
    Render,
    Presentation,
    Review,
}

impl Workspace {
    const ALL: [Workspace; 5] = [
        Workspace::Sketch,
        Workspace::Bim,
        Workspace::Render,
        Workspace::Presentation,
        Workspace::Review,
    ];
    fn label(self) -> &'static str {
        match self {
            Workspace::Sketch => "Sketch",
            Workspace::Bim => "BIM Modelling",
            Workspace::Render => "Render",
            Workspace::Presentation => "Presentation",
            Workspace::Review => "Review",
        }
    }
    fn roadmap(self) -> Option<&'static str> {
        match self {
            Workspace::Sketch => None,
            Workspace::Bim => Some("Walls, floors, roofs, doors, windows and rooms arrive in milestone 2–3. Today: classify Forms as BIM categories in Properties."),
            Workspace::Render => Some("Materials, sun & sky, PBR, AI render and video timeline arrive in milestone 5."),
            Workspace::Presentation => Some("Boards, diagrams and styles arrive in milestone 4."),
            Workspace::Review => Some("Comments, option comparison and branches arrive in milestone 4."),
        }
    }
}

struct Status {
    text: String,
    error: bool,
    at: Instant,
}

pub struct FerroApp {
    session: Session,
    path: Option<PathBuf>,
    store: Option<FroStore>,
    saved: Document,
    tool: ToolState,
    camera: OrbitCamera,
    renderer: Option<Renderer>,
    texture: Option<egui::TextureId>,
    scene: SceneCache,
    snap_scene: Option<(Document, Vec<ElementId>, SnapScene)>,
    selection: HashSet<ElementId>,
    vcb: String,
    status: Option<Status>,
    active_level: Option<ElementId>,
    workspace: Workspace,
    new_level_name: String,
    new_level_elev: f64,
    level_edits: HashMap<ElementId, f64>,
    name_edit: Option<(ElementId, String)>,
    comment_edit: Option<(ElementId, String)>,
    show_about: bool,
    show_agents: bool,
    last_snap: Option<SnapResult>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl FerroApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        cc.egui_ctx.set_global_style(style);
        let doc = Document::new("Untitled");
        let mut app = FerroApp {
            session: Session::new(doc.clone(), Box::new(ferro_io::SystemIds)),
            path: None,
            store: None,
            saved: doc,
            tool: ToolState::new(ToolKind::Select),
            camera: OrbitCamera::default(),
            renderer: None,
            texture: None,
            scene: SceneCache::default(),
            snap_scene: None,
            selection: HashSet::new(),
            vcb: String::new(),
            status: None,
            active_level: None,
            workspace: Workspace::Sketch,
            new_level_name: "L2".into(),
            new_level_elev: 3.3,
            level_edits: HashMap::new(),
            name_edit: None,
            comment_edit: None,
            show_about: false,
            show_agents: false,
            last_snap: None,
        };
        if let Some(p) = open {
            app.open_path(p);
        }
        app
    }

    // ------------------------------------------------------------ helpers

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn dirty(&self) -> bool {
        !scene::same_doc(&self.saved, self.session.doc())
    }

    fn ctx(&self) -> ExecContext {
        ExecContext {
            timestamp_ms: now_ms(),
            ..ExecContext::user()
        }
    }

    fn run(&mut self, cmd: AnyCommand) -> bool {
        let name = cmd.name();
        match self.session.execute(cmd, &self.ctx()) {
            Ok(o) => {
                if let Some(w) = o.warnings.first() {
                    self.say(format!("{name}: {}", w.message), false);
                } else {
                    self.say(format!("Done: {name}"), false);
                }
                // New elements become the selection; deleted ones leave it.
                self.selection.retain(|id| !o.changed.deleted.contains(id));
                if !o.changed.added.is_empty() {
                    self.selection = o.changed.added.iter().copied().collect();
                }
                self.autosave();
                true
            }
            Err(f) => {
                self.say(failure_text(&f), true);
                false
            }
        }
    }

    fn autosave(&mut self) {
        if let Some(store) = &mut self.store {
            match store.sync(&self.saved, self.session.doc(), self.session.journal()) {
                Ok(_) => self.saved = self.session.doc().clone(),
                Err(e) => {
                    let msg = format!("Autosave failed: {e}");
                    self.say(msg, true);
                }
            }
        }
    }

    fn undo(&mut self) {
        self.tool.cancel();
        match self.session.undo() {
            Some(c) => {
                self.say(c.transaction_name, false);
                self.autosave();
            }
            None => self.say("Nothing to undo", false),
        }
    }

    fn redo(&mut self) {
        self.tool.cancel();
        match self.session.redo() {
            Some(c) => {
                self.say(c.transaction_name, false);
                self.autosave();
            }
            None => self.say("Nothing to redo", false),
        }
    }

    fn set_tool(&mut self, k: ToolKind) {
        self.tool = ToolState::new(k);
        self.vcb.clear();
    }

    fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let mut ids: Vec<ElementId> = self.selection.iter().copied().collect();
        ids.sort();
        self.run(DeleteElements { elements: ids }.into());
    }

    fn confirm_discard(&self) -> bool {
        !self.dirty()
            || rfd::MessageDialog::new()
                .set_title("Ferro")
                .set_description("Discard unsaved changes?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    fn new_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let doc = Document::new("Untitled");
        self.session.replace_document(doc.clone());
        self.saved = doc;
        self.path = None;
        self.store = None;
        self.selection.clear();
        self.active_level = None;
        self.camera = OrbitCamera::default();
        self.tool.cancel();
    }

    fn open_dialog(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Ferro project", &["fro"])
            .add_filter("Ferro JSON", &["json"])
            .pick_file()
        {
            self.open_path(p);
        }
    }

    fn open_path(&mut self, p: PathBuf) {
        let is_json = p.to_string_lossy().ends_with(".json");
        let loaded = if is_json {
            std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|t| ferro_io::document_from_json(&t).map_err(|e| e.to_string()))
        } else {
            ferro_io::load_fro(&p)
                .map(|(d, _)| d)
                .map_err(|e| e.to_string())
        };
        match loaded {
            Ok(doc) => {
                self.session.replace_document(doc.clone());
                self.saved = doc;
                self.store = if is_json {
                    None
                } else {
                    FroStore::open(&p).ok()
                };
                self.path = if is_json { None } else { Some(p.clone()) };
                self.selection.clear();
                self.active_level = None;
                self.camera
                    .zoom_extents(scene::doc_bounds(self.session.doc()));
                self.suggest_level_name();
                self.say(format!("Opened {}", p.display()), false);
            }
            Err(e) => self.say(format!("Could not open {}: {e}", p.display()), true),
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => match ferro_io::save_fro(&p, self.session.doc(), self.session.journal()) {
                Ok(()) => {
                    self.store = FroStore::open(&p).ok();
                    self.saved = self.session.doc().clone();
                    self.say(format!("Saved {}", p.display()), false);
                }
                Err(e) => self.say(format!("Save failed: {e}"), true),
            },
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let name = format!("{}.fro", self.session.doc().project().name);
        if let Some(mut p) = rfd::FileDialog::new()
            .add_filter("Ferro project", &["fro"])
            .set_file_name(name)
            .save_file()
        {
            if p.extension().is_none() {
                p.set_extension("fro");
            }
            self.path = Some(p);
            self.save();
        }
    }

    fn export(&mut self, glb: bool) {
        let (ext, label) = if glb {
            ("glb", "glTF binary")
        } else {
            ("obj", "Wavefront OBJ")
        };
        let name = format!("{}.{ext}", self.session.doc().project().name);
        let Some(p) = rfd::FileDialog::new()
            .add_filter(label, &[ext])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let r = if glb {
            ferro_io::export_glb(self.session.doc())
                .map_err(|e| e.to_string())
                .and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string()))
        } else {
            ferro_io::export_obj(self.session.doc())
                .map_err(|e| e.to_string())
                .and_then(|s| std::fs::write(&p, s).map_err(|e| e.to_string()))
        };
        match r {
            Ok(()) => self.say(format!("Exported {}", p.display()), false),
            Err(e) => self.say(format!("Export failed: {e}"), true),
        }
    }

    fn suggest_level_name(&mut self) {
        let levels = self.session.doc().levels();
        self.new_level_name = format!("L{}", levels.len() + 2);
        self.new_level_elev = levels
            .last()
            .and_then(|e| e.as_level())
            .map(|l| l.elevation.m() + 3.3)
            .unwrap_or(3.3);
    }

    fn level_info(&self) -> Option<(ElementId, f64)> {
        let id = self.active_level?;
        let e = self.session.doc().get(id)?;
        Some((id, e.as_level()?.elevation.mm()))
    }

    // ------------------------------------------------------------ input

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if cmd && pressed(Key::Z) {
            if shift {
                self.redo()
            } else {
                self.undo()
            }
        }
        if cmd && pressed(Key::Y) {
            self.redo();
        }
        if cmd && pressed(Key::S) {
            if shift {
                self.save_as()
            } else {
                self.save()
            }
        }
        if cmd && pressed(Key::O) {
            self.open_dialog();
        }
        if cmd && pressed(Key::N) {
            self.new_document();
        }
        if cmd && pressed(Key::A) && !typing {
            self.selection = self
                .session
                .doc()
                .by_category(Category::Forms)
                .map(|e| e.id)
                .collect();
        }
        if typing || cmd {
            return;
        }
        // "Just type" a value while a tool is active (SketchUp VCB).
        if self.tool.is_active() {
            let text: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect()
            });
            if !text.is_empty() {
                self.vcb.push_str(&text);
                return;
            }
        }
        if pressed(Key::Escape) {
            if self.tool.is_active() || !self.vcb.is_empty() {
                self.tool.cancel();
                self.vcb.clear();
            } else {
                self.selection.clear();
            }
        }
        if pressed(Key::Backspace) {
            if !self.vcb.is_empty() {
                self.vcb.pop();
            } else if self.tool.kind == ToolKind::Polygon && self.tool.is_active() {
                self.tool.backspace();
            } else {
                self.delete_selection();
            }
        }
        if pressed(Key::Delete) {
            self.delete_selection();
        }
        if !self.tool.is_active() {
            for (k, t) in [
                (Key::Space, ToolKind::Select),
                (Key::R, ToolKind::Rectangle),
                (Key::G, ToolKind::Polygon),
                (Key::P, ToolKind::PushPull),
                (Key::M, ToolKind::Move),
                (Key::E, ToolKind::Eraser),
            ] {
                if pressed(k) {
                    self.set_tool(t);
                }
            }
            if pressed(Key::Z) {
                self.camera
                    .zoom_extents(scene::doc_bounds(self.session.doc()));
            }
        }
    }

    // ------------------------------------------------------------ panels

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New            Ctrl+N").clicked() {
                    self.new_document();
                }
                if ui.button("Open…          Ctrl+O").clicked() {
                    self.open_dialog();
                }
                ui.separator();
                if ui.button("Save           Ctrl+S").clicked() {
                    self.save();
                }
                if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Export OBJ…").clicked() {
                    self.export(false);
                }
                if ui.button("Export glTF (.glb)…").clicked() {
                    self.export(true);
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let u = self
                    .session
                    .undo_name()
                    .map(|n| format!("Undo {n}"))
                    .unwrap_or("Undo".into());
                if ui
                    .add_enabled(self.session.can_undo(), egui::Button::new(u))
                    .clicked()
                {
                    self.undo();
                }
                let r = self
                    .session
                    .redo_name()
                    .map(|n| format!("Redo {n}"))
                    .unwrap_or("Redo".into());
                if ui
                    .add_enabled(self.session.can_redo(), egui::Button::new(r))
                    .clicked()
                {
                    self.redo();
                }
                ui.separator();
                if ui
                    .add_enabled(!self.selection.is_empty(), egui::Button::new("Delete"))
                    .clicked()
                {
                    self.delete_selection();
                }
                if ui.button("Select all").clicked() {
                    self.selection = self
                        .session
                        .doc()
                        .by_category(Category::Forms)
                        .map(|e| e.id)
                        .collect();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Zoom extents   Z").clicked() {
                    self.camera
                        .zoom_extents(scene::doc_bounds(self.session.doc()));
                }
                if ui.button("Iso").clicked() {
                    self.camera.set_view(-60.0, 30.0);
                }
                if ui.button("Top").clicked() {
                    self.camera.set_view(-90.0, 89.9);
                }
                if ui.button("Front").clicked() {
                    self.camera.set_view(-90.0, 0.0);
                }
                if ui.button("Right").clicked() {
                    self.camera.set_view(0.0, 0.0);
                }
            });
            ui.menu_button("AI", |ui| {
                if ui.button("Connect an AI agent (MCP)…").clicked() {
                    self.show_agents = true;
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Ferro").clicked() {
                    self.show_about = true;
                }
            });
            ui.separator();
            for w in Workspace::ALL {
                if ui
                    .selectable_label(self.workspace == w, w.label())
                    .clicked()
                {
                    self.workspace = w;
                }
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        for k in ToolKind::ALL {
            let selected = self.tool.kind == k;
            let b = egui::Button::new(RichText::new(k.label()).size(15.0))
                .right_text(RichText::new(k.icon()).weak().size(11.0))
                .selected(selected)
                .min_size(Vec2::new(124.0, 30.0));
            if ui
                .add(b)
                .on_hover_text(format!("{} ({})", k.label(), k.shortcut()))
                .clicked()
            {
                self.set_tool(k);
            }
        }
        if self.workspace == Workspace::Bim {
            ui.separator();
            ui.label(RichText::new("BIM tools").strong());
            for t in ["Wall", "Floor", "Roof", "Door", "Window", "Room"] {
                ui.add_enabled(false, egui::Button::new(t).min_size(Vec2::new(120.0, 26.0)))
                    .on_disabled_hover_text("Arrives in milestone 2–3");
            }
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Project");
            let mut name = self.session.doc().project().name.clone();
            if ui.text_edit_singleline(&mut name).lost_focus()
                && name != self.session.doc().project().name
            {
                self.run(SetProjectName { name }.into());
            }
            ui.separator();
            self.levels_ui(ui);
            ui.separator();
            self.properties_ui(ui);
            if let Some(note) = self.workspace.roadmap() {
                ui.separator();
                ui.label(RichText::new(self.workspace.label()).strong());
                ui.label(RichText::new(note).italics());
            }
        });
    }

    fn levels_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Levels");
        let levels: Vec<(ElementId, String, f64)> = self
            .session
            .doc()
            .levels()
            .iter()
            .map(|e| {
                (
                    e.id,
                    e.label(),
                    e.as_level().map(|l| l.elevation.m()).unwrap_or(0.0),
                )
            })
            .collect();
        if ui
            .radio(self.active_level.is_none(), "Ground (0.000 m)")
            .clicked()
        {
            self.active_level = None;
        }
        let mut commit = None;
        for (id, name, elev) in levels.iter().rev() {
            ui.horizontal(|ui| {
                if ui
                    .radio(self.active_level == Some(*id), name.as_str())
                    .clicked()
                {
                    self.active_level = Some(*id);
                }
                let v = self.level_edits.entry(*id).or_insert(*elev);
                let r = ui.add(
                    egui::DragValue::new(v)
                        .speed(0.05)
                        .suffix(" m")
                        .fixed_decimals(3),
                );
                if (r.drag_stopped() || r.lost_focus()) && (*v - elev).abs() > 1e-9 {
                    commit = Some((*id, *v));
                }
                if !r.dragged() && !r.has_focus() && (*v - elev).abs() > 1e-9 && commit.is_none() {
                    *v = *elev;
                }
            });
        }
        if let Some((level, m)) = commit {
            self.run(
                SetLevelElevation {
                    level,
                    elevation: Length::from_m(m),
                }
                .into(),
            );
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_level_name).desired_width(60.0));
            ui.add(
                egui::DragValue::new(&mut self.new_level_elev)
                    .speed(0.05)
                    .suffix(" m"),
            );
            if ui.button("+ Level").clicked() {
                let cmd = CreateLevel {
                    name: self.new_level_name.clone(),
                    elevation: Length::from_m(self.new_level_elev),
                    is_building_storey: true,
                };
                if self.run(cmd.into()) {
                    self.suggest_level_name();
                    self.selection.clear();
                }
            }
        });
    }

    fn properties_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        if self.selection.is_empty() {
            ui.label("Nothing selected.");
            return;
        }
        if self.selection.len() > 1 {
            ui.label(format!("{} elements selected", self.selection.len()));
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
            return;
        }
        let id = *self.selection.iter().next().expect("one");
        let Some(e) = self.session.doc().get(id).cloned() else {
            self.selection.clear();
            return;
        };
        egui::Grid::new("props")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Id");
                ui.label(format!("{}  ·  {}", e.id, &e.uid.to_string()[..8]));
                ui.end_row();

                ui.label("Name");
                let buf = match &mut self.name_edit {
                    Some((i, s)) if *i == id => s,
                    _ => {
                        self.name_edit = Some((id, e.name.clone().unwrap_or_default()));
                        &mut self.name_edit.as_mut().expect("set").1
                    }
                };
                let r = ui.text_edit_singleline(buf);
                if r.lost_focus() {
                    let new = buf.trim().to_string();
                    let new = (!new.is_empty()).then_some(new);
                    if new != e.name {
                        self.run(
                            SetName {
                                element: id,
                                name: new,
                            }
                            .into(),
                        );
                    }
                }
                ui.end_row();

                ui.label("Category");
                if e.as_form().is_some() {
                    let mut cat = e.category;
                    egui::ComboBox::from_id_salt("cat")
                        .selected_text(cat.to_string())
                        .show_ui(ui, |ui| {
                            for c in Category::ALL.iter().filter(|c| c.classifiable_form()) {
                                ui.selectable_value(&mut cat, *c, c.to_string());
                            }
                        });
                    if cat != e.category {
                        self.run(
                            SetCategory {
                                element: id,
                                category: cat,
                            }
                            .into(),
                        );
                    }
                } else {
                    ui.label(e.category.to_string());
                }
                ui.end_row();

                if let Some(l) = e.level_id.and_then(|l| self.session.doc().get(l)) {
                    ui.label("Level");
                    ui.label(l.label());
                    ui.end_row();
                }
                match &e.body {
                    ElementBody::Form(f) => {
                        ui.label("Faces");
                        ui.label(format!(
                            "{}{}",
                            f.body.face_count(),
                            if f.body.is_closed() { "  (solid)" } else { "" }
                        ));
                        ui.end_row();
                        if let Some(bb) = f.body.bounding_box() {
                            let s = bb.size();
                            ui.label("Size");
                            ui.label(format!("{:.0} × {:.0} × {:.0} mm", s.x, s.y, s.z));
                            ui.end_row();
                        }
                    }
                    ElementBody::Level(l) => {
                        ui.label("Elevation");
                        ui.label(format!("{:.3} m", l.elevation.m()));
                        ui.end_row();
                    }
                    ElementBody::Wall(_) | ElementBody::Floor(_) => {
                        let is_wall = e.as_wall().is_some();
                        let types: Vec<(ElementId, String)> = self
                            .session
                            .doc()
                            .elements()
                            .filter(|t| {
                                if is_wall {
                                    matches!(t.body, ElementBody::WallType(_))
                                } else {
                                    matches!(t.body, ElementBody::FloorType(_))
                                }
                            })
                            .map(|t| (t.id, t.label()))
                            .collect();
                        let mut chosen = e.type_id;
                        ui.label("Type");
                        let current = chosen
                            .and_then(|t| self.session.doc().get(t))
                            .map(|t| t.label())
                            .unwrap_or_default();
                        egui::ComboBox::from_id_salt("type")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                for (tid, name) in &types {
                                    ui.selectable_value(&mut chosen, Some(*tid), name);
                                }
                            });
                        ui.end_row();
                        if let (Some(t), true) = (chosen, chosen != e.type_id) {
                            self.run(
                                ferro_core::ops::bim_commands::SetElementType {
                                    element: id,
                                    element_type: t,
                                }
                                .into(),
                            );
                        }
                        for (k, label) in [
                            (keys::LENGTH, "Length"),
                            (keys::HEIGHT, "Height"),
                            (keys::THICKNESS, "Thickness"),
                        ] {
                            if let Some(ParamValue::Length(l)) = e.param(k) {
                                ui.label(label);
                                ui.label(format!("{:.0} mm", l.mm()));
                                ui.end_row();
                            }
                        }
                        if let Some(ParamValue::Area(a)) = e.param(keys::AREA) {
                            ui.label("Area");
                            ui.label(format!("{:.2} m²", a / 1e6));
                            ui.end_row();
                        }
                        if let Some(ParamValue::Volume(v)) = e.param(keys::VOLUME) {
                            ui.label("Volume");
                            ui.label(format!("{:.3} m³", v / 1e9));
                            ui.end_row();
                        }
                    }
                    ElementBody::Opening(o) => {
                        if let Some(h) = e.host_id.and_then(|h| self.session.doc().get(h)) {
                            ui.label("Host");
                            ui.label(h.label());
                            ui.end_row();
                        }
                        ui.label("Size");
                        ui.label(format!("{:.0} × {:.0} mm", o.width.mm(), o.height.mm()));
                        ui.end_row();
                        ui.label("Sill");
                        ui.label(format!("{:.0} mm", o.sill.mm()));
                        ui.end_row();
                    }
                    ElementBody::WallType(ferro_core::model::bim::WallType { structure })
                    | ElementBody::FloorType(ferro_core::model::bim::FloorType { structure }) => {
                        ui.label("Thickness");
                        ui.label(format!("{:.0} mm", structure.thickness()));
                        ui.end_row();
                        for l in &structure.layers {
                            ui.label(format!("{:?}", l.function));
                            let mat = l
                                .material
                                .and_then(|m| self.session.doc().get(m))
                                .map(|m| m.label())
                                .unwrap_or_else(|| "—".into());
                            ui.label(format!("{:.0} mm  {mat}", l.thickness.mm()));
                            ui.end_row();
                        }
                    }
                    ElementBody::Material(m) => {
                        ui.label("Colour");
                        let [r, g, b] = m.color.map(|c| (c * 255.0).round() as u8);
                        ui.colored_label(Color32::from_rgb(r, g, b), "■■■");
                        ui.end_row();
                    }
                }
                if let Some(ParamValue::Area(a)) = e.param(keys::FORM_AREA) {
                    ui.label("Surface area");
                    ui.label(format!("{:.2} m²", a / 1e6));
                    ui.end_row();
                }
                if let Some(ParamValue::Volume(v)) = e.param(keys::FORM_VOLUME) {
                    ui.label("Volume");
                    ui.label(format!("{:.3} m³", v / 1e9));
                    ui.end_row();
                }

                ui.label("Comments");
                let current = match e.param(keys::COMMENTS) {
                    Some(ParamValue::Text(t)) => t.clone(),
                    _ => String::new(),
                };
                let buf = match &mut self.comment_edit {
                    Some((i, s)) if *i == id => s,
                    _ => {
                        self.comment_edit = Some((id, current.clone()));
                        &mut self.comment_edit.as_mut().expect("set").1
                    }
                };
                if ui.text_edit_singleline(buf).lost_focus() && *buf != current {
                    let value = ParamValue::Text(buf.clone());
                    self.run(
                        SetParameter {
                            element: id,
                            key: keys::COMMENTS.into(),
                            value,
                        }
                        .into(),
                    );
                }
                ui.end_row();
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let hint = match self
                .status
                .as_ref()
                .filter(|s| s.at.elapsed() < Duration::from_secs(6))
            {
                Some(s) if s.error => RichText::new(&s.text).color(Color32::from_rgb(200, 40, 40)),
                Some(s) => RichText::new(&s.text),
                None => RichText::new(self.tool.hint()).weak(),
            };
            ui.label(hint);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.vcb)
                        .desired_width(160.0)
                        .hint_text("type a value"),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.apply_vcb();
                }
                ui.label(self.tool.value_label());
                if let Some(s) = self.last_snap {
                    ui.label(RichText::new(s.kind.label()).weak());
                }
            });
        });
    }

    fn apply_vcb(&mut self) {
        let text = std::mem::take(&mut self.vcb);
        let Some(ctx_parts) = self.tool_ctx_parts() else {
            return;
        };
        let (ray, snap) = ctx_parts;
        let level = self.level_info();
        let ev = {
            let ctx = ToolCtx {
                doc: self.session.doc(),
                snap,
                ray,
                selection: &self.selection,
                level,
                copy: false,
                near_first_px: None,
            };
            self.tool.typed(&text, &ctx)
        };
        match ev {
            Ok(ToolEvent::Commit(c)) => {
                self.run(c);
            }
            Ok(_) => {}
            Err(e) => self.say(e, true),
        }
    }

    fn tool_ctx_parts(&self) -> Option<(ferro_snap::Ray, Option<SnapResult>)> {
        let s = self.last_snap?;
        // Without a fresh cursor ray, aim from the camera at the last snap.
        let eye = self.camera.eye();
        let dir = (s.point - eye).normalize()?;
        Some((ferro_snap::Ray { origin: eye, dir }, Some(s)))
    }

    // ------------------------------------------------------------ viewport

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let origin = Point3::new(
            (self.camera.target.x / 10_000.0).round() * 10_000.0,
            (self.camera.target.y / 10_000.0).round() * 10_000.0,
            0.0,
        );

        // Navigation: middle / right drag orbit (Shift = pan), scroll / pinch zoom.
        let fc0 = FrameCamera::new(
            &self.camera,
            origin,
            [rect.min.x, rect.min.y, rect.width(), rect.height()],
        );
        for b in [PointerButton::Middle, PointerButton::Secondary] {
            if resp.dragged_by(b) {
                let d = resp.drag_delta();
                if ctx.input(|i| i.modifiers.shift) {
                    self.camera
                        .pan(d.x as f64, d.y as f64, rect.height() as f64);
                } else {
                    self.camera.orbit(d.x as f64, d.y as f64);
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary) && ctx.input(|i| i.modifiers.alt) {
            let d = resp.drag_delta();
            self.camera.orbit(d.x as f64, d.y as f64);
        }
        if resp.hovered() {
            let (scroll, zoom) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let focus = self.last_snap.map(|s| s.point);
            if scroll.abs() > 0.0 {
                self.camera.zoom((scroll as f64 * 0.004).exp(), focus);
            }
            if (zoom - 1.0).abs() > 1e-4 {
                self.camera.zoom(zoom as f64, focus);
            }
        }
        let _ = fc0;
        let fc = FrameCamera::new(
            &self.camera,
            origin,
            [rect.min.x, rect.min.y, rect.width(), rect.height()],
        );

        // Snapping against the committed document (excluding moved elements).
        let moving: Vec<ElementId> = self.tool.moving().to_vec();
        let rebuild = match &self.snap_scene {
            Some((d, m, _)) => !scene::same_doc(d, self.session.doc()) || *m != moving,
            None => true,
        };
        if rebuild {
            self.snap_scene = Some((
                self.session.doc().clone(),
                moving.clone(),
                SnapScene::from_document(self.session.doc(), &moving),
            ));
        }
        let hover = resp.hover_pos().or_else(|| {
            ctx.input(|i| i.pointer.latest_pos())
                .filter(|p| rect.contains(*p))
        });
        let level = self.level_info();
        let mut snap_res = None;
        let mut ray = fc.ray([rect.center().x, rect.center().y]);
        let mut near_first = None;
        if let Some(p) = hover {
            ray = fc.ray([p.x, p.y]);
            let settings = SnapSettings {
                ground_z: level.map(|l| l.1).unwrap_or(0.0),
                ..Default::default()
            };
            let sc = &self.snap_scene.as_ref().expect("built").2;
            snap_res = snap(sc, &ray, [p.x, p.y], &fc, self.tool.anchor(), &settings);
            near_first = self
                .tool
                .polygon_first()
                .and_then(|f| fc.project(f))
                .map(|s| ((s[0] - p.x).powi(2) + (s[1] - p.y).powi(2)).sqrt());
        }
        self.last_snap = snap_res;

        // Tool interaction.
        let shift = ctx.input(|i| i.modifiers.shift);
        let copy = ctx.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);
        let mut event = ToolEvent::None;
        let (preview_cmd, hover_face, rubber) = {
            let tctx = ToolCtx {
                doc: self.session.doc(),
                snap: snap_res,
                ray,
                selection: &self.selection,
                level,
                copy,
                near_first_px: near_first,
            };
            if resp.clicked_by(PointerButton::Primary) && !ctx.input(|i| i.modifiers.alt) {
                event = self.tool.click(&tctx, shift);
            }
            if !self.tool.is_active() || self.vcb.is_empty() {
                // live value
            }
            (
                self.tool.preview(&tctx),
                self.tool.hover_face(&tctx),
                self.tool.rubber_band(&tctx),
            )
        };
        if self.tool.is_active() && self.vcb.is_empty() {
            // Nothing typed: show live measurement as a hint (handled in status bar).
        }
        if ctx.input(|i| i.key_pressed(Key::Enter))
            && !ctx.egui_wants_keyboard_input()
            && (self.tool.is_active() || self.tool.kind == ToolKind::Polygon)
        {
            self.apply_vcb();
        }
        match event {
            ToolEvent::Commit(c) => {
                self.run(c);
            }
            ToolEvent::Select(op) => match op {
                SelectOp::Set(v) => self.selection = v.into_iter().collect(),
                SelectOp::Toggle(e) => {
                    if !self.selection.remove(&e) {
                        self.selection.insert(e);
                    }
                }
                SelectOp::Clear => self.selection.clear(),
            },
            ToolEvent::None => {}
        }

        // Document to display: the committed one, or the tool preview.
        let mut display = self.session.doc().clone();
        let mut preview_err = None;
        if let Some(cmd) = preview_cmd {
            match self.session.preview(&cmd) {
                Ok((d, _)) => display = d,
                Err(f) => preview_err = Some(failure_text(&f)),
            }
        }
        self.scene.update(&display, origin, &self.selection);

        // Overlays: hovered face, rubber band, snap marker, grid.
        let rel = |p: Point3| {
            let v = p - origin;
            [v.x as f32, v.y as f32, v.z as f32]
        };
        let mut otris: Vec<Vertex> = Vec::new();
        let eye = self.camera.eye();
        let forward = (self.camera.target - eye)
            .normalize()
            .map(|u| u.as_vec())
            .unwrap_or(ferro_core::Vec3::Y);
        let mut olines: Vec<Vertex> = scene::grid_lines(
            origin,
            self.camera.target,
            (self.camera.distance * 1.2).clamp(10_000.0, 500_000.0),
            eye,
            forward,
            (self.camera.distance * 0.01).max(50.0),
        );
        if let Some((e, f)) = hover_face {
            if let Some(m) = self.scene.mesh(e) {
                for t in scene::face_triangles(m, f) {
                    let n = (t[1] - t[0])
                        .cross(t[2] - t[0])
                        .normalize()
                        .map(|u| u.as_vec())
                        .unwrap_or_default();
                    for p in t {
                        let q = p + n * 2.0;
                        otris.push(Vertex {
                            pos: rel(q),
                            normal: [n.x as f32, n.y as f32, n.z as f32],
                            color: [0.20, 0.45, 0.95, 0.35],
                        });
                    }
                }
            }
        }
        for (a, b) in rubber {
            olines.push(line_vertex(rel(a), [0.05, 0.05, 0.05, 1.0]));
            olines.push(line_vertex(rel(b), [0.05, 0.05, 0.05, 1.0]));
        }

        // Render offscreen and show as an image.
        let size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        if let Some(rs) = frame.wgpu_render_state() {
            let r = self
                .renderer
                .get_or_insert_with(|| Renderer::new(&rs.device));
            let recreated = r.render(
                &rs.device,
                &rs.queue,
                size,
                &FrameData {
                    view_proj: fc.view_proj,
                    eye: rel(eye),
                    scene_version: self.scene.version,
                    scene_tris: &self.scene.tris,
                    scene_lines: &self.scene.lines,
                    overlay_tris: &otris,
                    overlay_lines: &olines,
                    clear: [0.93, 0.94, 0.96],
                },
            );
            if let Some(view) = r.output_view() {
                let mut egui_r = rs.renderer.write();
                match self.texture {
                    Some(id) if recreated => egui_r.update_egui_texture_from_wgpu_texture(
                        &rs.device,
                        view,
                        wgpu::FilterMode::Linear,
                        id,
                    ),
                    Some(_) => {}
                    None => {
                        self.texture = Some(egui_r.register_native_texture(
                            &rs.device,
                            view,
                            wgpu::FilterMode::Linear,
                        ))
                    }
                }
            }
        }
        let painter = ui.painter_at(rect);
        if let Some(id) = self.texture {
            painter.image(
                id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(rect, 0.0, Color32::from_gray(235));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "GPU renderer unavailable",
                egui::FontId::proportional(16.0),
                Color32::DARK_RED,
            );
        }

        // Snap marker and label (screen space).
        if let Some(s) = snap_res {
            if let Some(p) = fc.project(s.point) {
                let p = Pos2::new(p[0], p[1]);
                let (color, label) = match s.kind {
                    SnapKind::Endpoint | SnapKind::Origin => (Color32::from_rgb(20, 160, 60), true),
                    SnapKind::Midpoint => (Color32::from_rgb(0, 170, 200), true),
                    SnapKind::OnEdge => (Color32::from_rgb(200, 30, 30), true),
                    SnapKind::OnAxis(ferro_snap::Axis::X) => (Color32::from_rgb(220, 40, 40), true),
                    SnapKind::OnAxis(ferro_snap::Axis::Y) => (Color32::from_rgb(40, 160, 60), true),
                    SnapKind::OnAxis(ferro_snap::Axis::Z) => (Color32::from_rgb(40, 80, 220), true),
                    SnapKind::OnFace => (Color32::from_rgb(40, 60, 200), false),
                    SnapKind::Ground => (Color32::GRAY, false),
                };
                painter.circle_stroke(p, 5.0, egui::Stroke::new(2.0, color));
                if label && self.tool.kind != ToolKind::Select {
                    painter.text(
                        p + Vec2::new(10.0, -14.0),
                        egui::Align2::LEFT_BOTTOM,
                        s.kind.label(),
                        egui::FontId::proportional(13.0),
                        color,
                    );
                }
            }
        }
        // Live measurement near the cursor while a tool is active.
        if let Some(p) = hover {
            let m = {
                let tctx = ToolCtx {
                    doc: self.session.doc(),
                    snap: snap_res,
                    ray,
                    selection: &self.selection,
                    level,
                    copy,
                    near_first_px: near_first,
                };
                self.tool.measurement(&tctx)
            };
            let shown = if self.vcb.is_empty() {
                m
            } else {
                format!("{} ⏎", self.vcb)
            };
            if self.tool.is_active() && !shown.is_empty() {
                painter.text(
                    p + Vec2::new(14.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    shown,
                    egui::FontId::monospace(13.0),
                    Color32::from_gray(30),
                );
            }
        }
        if let Some(e) = preview_err {
            painter.text(
                rect.left_bottom() + Vec2::new(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                e,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(200, 40, 40),
            );
        }
        // Axis gizmo label.
        painter.text(
            rect.right_top() + Vec2::new(-10.0, 10.0),
            egui::Align2::RIGHT_TOP,
            format!(
                "{}  ·  {:.1} m view",
                self.workspace.label(),
                self.camera.distance / 1000.0
            ),
            egui::FontId::proportional(12.0),
            Color32::from_gray(90),
        );

        if self.tool.is_active() || resp.hovered() {
            ctx.request_repaint();
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        egui::Window::new("About Ferro").open(&mut self.show_about).resizable(false).show(ctx, |ui| {
            ui.heading(format!("Ferro {}", env!("CARGO_PKG_VERSION")));
            ui.label("Open-source, AI-native 3D design tool for architects.");
            ui.label("Licence: MPL-2.0 · github.com/ktsdatpham/ferro");
            ui.label("Milestone 1: sketch modelling, levels, push/pull, undo, .fro files, OBJ/glTF export, MCP for AI agents.");
        });
        let mcp = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.parent().map(|d| {
                    d.join(if cfg!(windows) {
                        "ferro-mcp.exe"
                    } else {
                        "ferro-mcp"
                    })
                })
            })
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "ferro-mcp".into());
        egui::Window::new("Connect an AI agent").open(&mut self.show_agents).resizable(true).show(ctx, |ui| {
            ui.label("Ferro speaks MCP. Add this server to Claude Desktop, Claude Code, Cursor or any MCP client:");
            let cfg = serde_json::json!({"mcpServers": {"ferro": {"command": mcp}}});
            let mut text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(6));
            ui.label("The agent can create levels, boxes, faces, push/pull, move, classify and save .fro files, which you then open here.");
            ui.label(RichText::new("Live editing of the open window through Ferro Link arrives in milestone 2.").italics());
        });
    }
}

fn failure_text(f: &[Failure]) -> String {
    f.first()
        .map(|x| x.message.clone())
        .unwrap_or_else(|| "failed".into())
}

impl eframe::App for FerroApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        let title = format!(
            "Ferro — {}{}",
            self.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.session.doc().project().name.clone()),
            if self.dirty() && self.store.is_none() {
                " •"
            } else {
                ""
            }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tools")
            .resizable(false)
            .default_size(136.0)
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::right("side")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, frame));
        self.windows(&ctx);
    }
}
