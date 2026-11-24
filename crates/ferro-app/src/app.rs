//! The Ferro window: menus, workspaces, toolbar, viewport, levels and
//! properties panels, status bar with the value box (Q-R7, Q-R11, Q-R13).

use crate::ai::{self, AiSettings, ChatEvent, Provider};
use crate::camera::{FrameCamera, OrbitCamera};
use crate::keymap::{KeyResolver, KeyScheme, Keymap, KeymapFile, Target};
use crate::link::{self, LinkServer};
use crate::material_editor::{DraftAction, MaterialDraft};
use crate::render::{FrameData, Renderer, Vertex};
use crate::scene::{self, line_vertex, SceneCache};
use crate::tools::{
    pick_sub, SectionPlane, SelectOp, SubPick, ToolCtx, ToolEvent, ToolKind, ToolState,
};
use crate::units;
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, RichText, Sense, Vec2};
use eframe::wgpu;
use ferro_core::model::{keys, Category, ElementBody};
use ferro_core::ops::commands::*;
use ferro_core::ops::Actor;
use ferro_core::ops::{ExecContext, Failure};
use ferro_core::{AnyCommand, Document, ElementId, Length, ParamValue, Point3, Session};
use ferro_io::FroStore;
use ferro_snap::{snap, Projector, SnapKind, SnapResult, SnapScene, SnapSettings};
use ferro_tools::link::{AgentInfo, LinkRequest, LinkResponse};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod design_options;
mod phases;
mod pins;
mod python;
mod views;

/// How much an AI agent may do on its own (D-26 autonomy slider).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Autonomy {
    /// Agent changes collect into one proposal the user accepts or rejects.
    Propose,
    /// Agent changes apply immediately (each is still one undo step).
    Apply,
}

/// Changes an agent made that wait for the user's decision.
struct Proposal {
    agent: String,
    steps: usize,
    /// Where its changes land (`None` = the shared model), shown on the bar.
    targets: Vec<Option<ElementId>>,
}

#[derive(Debug, Clone)]
enum ChatLine {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        ok: bool,
        summary: String,
    },
    Error(String),
}

struct Chat {
    open: bool,
    input: String,
    lines: Vec<ChatLine>,
    history: Arc<Mutex<Vec<Value>>>,
    busy: bool,
    rx: Option<Receiver<ChatEvent>>,
    settings: AiSettings,
    show_settings: bool,
    settings_note: String,
}

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
            Workspace::Bim => Some("Walls, floors, doors and windows are live. Push/Pull a wall's top to change its height, its end to change its length. Roofs, rooms and stairs arrive in milestone 3."),
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
    link: LinkServer,
    /// Material the Paint tool applies.
    material: Option<ElementId>,
    new_material_color: [f32; 3],
    new_material_name: String,
    /// Material open in the Materials panel editor.
    material_draft: Option<MaterialDraft>,
    /// The open Project settings window (M5-24.2), if any.
    project_settings: Option<crate::settings_window::ProjectSettingsDraft>,
    clipboard: Vec<ElementId>,
    /// Last Move-copy, for arrays typed afterwards (`5x`, `/5`).
    last_copy: Option<CopyElements>,
    /// Screen position where a selection window drag started.
    select_drag: Option<Pos2>,
    /// Elements hidden in the active view (copy of the view's list).
    hidden: HashSet<ElementId>,
    /// Tags whose elements are not shown (copy of the active view's).
    hidden_tags: HashSet<String>,
    /// Tag name typed in the Tags panel.
    new_tag: String,
    /// Section plane cutting the view (copy of the active view's cut).
    section: Option<SectionPlane>,
    /// Hidden line style: white faces, black edges.
    hidden_line: bool,
    /// Form being edited (double-click, SketchUp group editing).
    editing: Option<ElementId>,
    /// Faces / edges / vertices picked inside it.
    sub_sel: Vec<SubPick>,
    /// Axis locked with the arrow keys (→ red X, ← green Y, ↑ blue Z).
    axis_lock: Option<ferro_snap::Axis>,
    /// Axis locked while Shift is held over an axis inference.
    shift_lock: Option<(SnapKind, ferro_core::Vec3)>,
    /// Keyboard shortcuts (preset + `~/.ferro/keymap.json`).
    keymap: Keymap,
    keys: KeyResolver,
    /// Command palette query, when open (Ctrl+K).
    palette: Option<String>,
    /// Problem report being reviewed by the user (Help > Report a problem,
    /// or a crash report from the last session).
    report: Option<ReportDraft>,
    /// Direction of the last edge hovered: reference for parallel /
    /// perpendicular inference (SketchUp: hover an edge to teach it).
    ref_dir: Option<ferro_core::UnitVec3>,
    autonomy: Autonomy,
    proposal: Option<Proposal>,
    /// The `run_python` script running now, if any (ADR 0005 §5).
    python: Option<python::ScriptRun>,
    /// User actions held while a script runs, and the priority dialog.
    script_wait: python::ScriptWait,
    /// Interpreter and SDK for `run_python` (`FERRO_PYTHON`, `FERRO_PYTHON_SDK`).
    python_config: ferro_mcp::python::PythonConfig,
    chat: Chat,
    /// View phase and phase display (M5-23; copy of the active view's).
    phasing: phases::PhaseState,
    /// The active view (M5-22, `views`).
    views: views::ViewState,
    /// Design-option combination shown and option edited (M5-23.9; view
    /// state, not saved).
    options: design_options::DesignOptionsState,
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
            session: Session::new(doc.clone(), Box::new(ferro_io::SystemIds))
                .with_kernel(std::sync::Arc::new(ferro_kernel::SolidcoreKernel::new())),
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
            new_level_elev: 3300.0,
            level_edits: HashMap::new(),
            name_edit: None,
            comment_edit: None,
            show_about: false,
            show_agents: false,
            last_snap: None,
            link: LinkServer::start("Untitled", cc.egui_ctx.clone()),
            material: None,
            new_material_color: [0.80, 0.55, 0.40],
            new_material_name: "Material 1".into(),
            material_draft: None,
            project_settings: None,
            clipboard: Vec::new(),
            last_copy: None,
            select_drag: None,
            hidden: HashSet::new(),
            hidden_tags: HashSet::new(),
            new_tag: String::new(),
            section: None,
            hidden_line: false,
            editing: None,
            sub_sel: Vec::new(),
            axis_lock: None,
            shift_lock: None,
            keymap: Keymap::scheme(KeyScheme::Ferro),
            keys: KeyResolver::default(),
            palette: None,
            report: crate::report::pending_crashes().pop().and_then(|p| {
                let text = std::fs::read_to_string(&p).ok()?;
                Some(ReportDraft::new(text, Some(p)))
            }),
            ref_dir: None,
            autonomy: Autonomy::Propose,
            proposal: None,
            python: None,
            script_wait: Default::default(),
            python_config: ferro_mcp::python::PythonConfig::from_env(),
            chat: Chat {
                open: false,
                input: String::new(),
                lines: Vec::new(),
                history: Arc::new(Mutex::new(Vec::new())),
                busy: false,
                rx: None,
                settings: AiSettings::load(),
                show_settings: false,
                settings_note: String::new(),
            },
            phasing: Default::default(),
            views: Default::default(),
            options: Default::default(),
        };
        if let Some(p) = open {
            app.open_path(p);
        }
        app.load_keymap();
        app
    }

    // ------------------------------------------------------------ helpers

    fn say(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            crate::report::record(format!("error shown: {}", first_code(&text)));
        }
        self.status = Some(Status {
            text,
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
        if self.python.is_some() {
            self.hold_for_script(python::UserAction::Run(cmd));
            return false;
        }
        // Editing while a proposal is open accepts it first.
        if self.proposal.is_some() {
            self.accept_proposal();
        }
        self.sub_sel.clear();
        self.last_copy = match &cmd {
            AnyCommand::CopyElements(c) => Some(c.clone()),
            _ => None,
        };
        let name = cmd.name();
        // While phasing is used, created elements get the view phase in the
        // same undo step (Revit behaviour, see `phases`).
        let grouped = name.starts_with("create_")
            && self.editing_option().is_none()
            && !self.session.in_group()
            && self.phase_for_new_elements().is_some()
            && self.session.begin_group(name, self.ctx());
        let result = self.execute_here(cmd);
        if grouped {
            match &result {
                Ok(o) => {
                    let added: Vec<ElementId> = o.changed.added.clone();
                    self.assign_view_phase(&added);
                    self.session.end_group();
                    self.refresh_view();
                }
                Err(_) => {
                    self.session.cancel_group();
                }
            }
        }
        crate::report::record(match &result {
            Ok(_) => format!("{name} ok"),
            Err(f) => format!(
                "{name} failed {}",
                f.iter()
                    .map(|x| x.code.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        });
        match result {
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
        if self.session.in_group() {
            return;
        }
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
        if self.hold_for_script(python::UserAction::Undo) {
            return;
        }
        if self.proposal.is_some() {
            self.reject_proposal();
            return;
        }
        let undone = self.session.undo();
        self.refresh_view();
        match undone {
            Some(c) => {
                self.say(c.transaction_name, false);
                self.autosave();
            }
            None => self.say("Nothing to undo", false),
        }
    }

    fn redo(&mut self) {
        self.tool.cancel();
        if self.hold_for_script(python::UserAction::Redo) {
            return;
        }
        if self.proposal.is_some() {
            return;
        }
        let redone = self.session.redo();
        self.refresh_view();
        match redone {
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

    /// Tags of an element (`ferro.common.tags`).
    fn tags_of(e: &ferro_core::Element) -> &[String] {
        match e.params.get(keys::TAGS) {
            Some(ParamValue::Tags(t)) => t,
            _ => &[],
        }
    }

    /// Elements not shown: hidden ones and those on a hidden Tag.
    fn hidden_elements(&self) -> HashSet<ElementId> {
        let mut h: HashSet<ElementId> = self
            .hidden
            .iter()
            .copied()
            .filter(|id| self.doc().get(*id).is_some())
            .collect();
        if !self.hidden_tags.is_empty() {
            for e in self.doc().elements() {
                if Self::tags_of(e)
                    .iter()
                    .any(|t| self.hidden_tags.contains(t))
                {
                    h.insert(e.id);
                }
            }
        }
        h
    }

    fn set_section(&mut self, p: Option<SectionPlane>) {
        if self.set_view_section(p) && p.is_some() {
            self.say("Section plane set. View > Clear section removes it.", false);
        }
    }

    /// Horizontal plan cut 1.2 m above the active level, looking down.
    fn plan_section(&mut self) {
        let z = self.level_info().map(|l| l.1).unwrap_or(0.0) + 1200.0;
        self.set_section(Some(SectionPlane {
            point: Point3::new(self.camera.target.x, self.camera.target.y, z),
            normal: ferro_core::UnitVec3::Z,
        }));
    }

    fn hide_selection(&mut self) {
        let ids: Vec<ElementId> = self.selection.drain().collect();
        self.hide_in_view(ids);
    }

    fn tags_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Tags");
        let mut all: Vec<String> = self
            .session
            .doc()
            .elements()
            .flat_map(|e| Self::tags_of(e).iter().cloned())
            .collect();
        all.sort();
        all.dedup();
        if all.is_empty() {
            ui.label(RichText::new("No tags yet.").weak());
        }
        for t in &all {
            ui.horizontal(|ui| {
                let mut shown = !self.hidden_tags.contains(t);
                if ui.checkbox(&mut shown, t).changed() {
                    self.set_tag_hidden(t, !shown);
                }
                if ui
                    .small_button("Select")
                    .on_hover_text("Select the elements with this tag")
                    .clicked()
                {
                    self.selection = self
                        .session
                        .doc()
                        .elements()
                        .filter(|e| Self::tags_of(e).contains(t))
                        .map(|e| e.id)
                        .collect();
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_tag)
                    .hint_text("tag")
                    .desired_width(90.0),
            );
            let tag = self.new_tag.trim().to_string();
            let ok = !tag.is_empty() && !self.selection.is_empty();
            if ui
                .add_enabled(ok, egui::Button::new("Tag selection"))
                .clicked()
            {
                self.tag_selection(&tag, true);
                self.new_tag.clear();
            }
            if ui.add_enabled(ok, egui::Button::new("Untag")).clicked() {
                self.tag_selection(&tag, false);
            }
        });
        if !self.hidden.is_empty()
            && ui
                .button(format!("Unhide all ({})", self.hidden.len()))
                .clicked()
        {
            self.unhide_all_in_view();
        }
    }

    /// Add (or remove) `tag` on the selected elements, as one undo step.
    fn tag_selection(&mut self, tag: &str, add: bool) {
        let mut ids: Vec<ElementId> = self.selection.iter().copied().collect();
        ids.sort();
        let cmds: Vec<AnyCommand> = ids
            .into_iter()
            .filter_map(|id| {
                let e = self.doc().get(id)?;
                let mut tags = Self::tags_of(e).to_vec();
                let has = tags.iter().any(|t| t == tag);
                if add == has {
                    return None;
                }
                if add {
                    tags.push(tag.to_string());
                } else {
                    tags.retain(|t| t != tag);
                }
                Some(
                    SetParameter {
                        element: id,
                        key: keys::TAGS.into(),
                        value: ParamValue::Tags(tags),
                    }
                    .into(),
                )
            })
            .collect();
        if cmds.is_empty() {
            return;
        }
        let name = if add { "Tag" } else { "Untag" };
        let opened = self.session.begin_group(name, self.ctx());
        for c in cmds {
            self.run(c);
        }
        if opened {
            self.session.end_group();
        }
    }

    fn delete_selection(&mut self) {
        if let (Some(e), false) = (self.editing, self.sub_sel.is_empty()) {
            let edges: Vec<_> = self
                .sub_sel
                .iter()
                .filter_map(|p| match p {
                    SubPick::Edge(x) => Some(*x),
                    _ => None,
                })
                .collect();
            let faces: Vec<_> = self
                .sub_sel
                .iter()
                .filter_map(|p| match p {
                    SubPick::Face(x) => Some(*x),
                    _ => None,
                })
                .collect();
            if !edges.is_empty() {
                self.run(ferro_core::ops::form_commands::EraseEdges { element: e, edges }.into());
            } else if !faces.is_empty() {
                self.run(ferro_core::ops::form_commands::EraseFaces { element: e, faces }.into());
            }
            if !self.doc().contains(e) {
                self.editing = None;
            }
            return;
        }
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
        if self.hold_for_script(python::UserAction::NewDocument) || !self.confirm_discard() {
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
        self.reset_views();
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
        if self.python.is_some() {
            self.hold_for_script(python::UserAction::Open(p));
            return;
        }
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
                self.camera.zoom_extents(scene::doc_bounds(self.doc()));
                // A file with views opens in its first view.
                self.reset_views();
                self.suggest_level_name();
                self.say(format!("Opened {}", p.display()), false);
            }
            Err(e) => self.say(format!("Could not open {}: {e}", p.display()), true),
        }
    }

    fn save(&mut self) {
        // Never save a half-finished script group.
        if self.hold_for_script(python::UserAction::Save) {
            return;
        }
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
        if self.hold_for_script(python::UserAction::SaveAs) {
            return;
        }
        let name = format!("{}.fro", self.doc().project().name);
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
        let name = format!("{}.{ext}", self.doc().project().name);
        let Some(p) = rfd::FileDialog::new()
            .add_filter(label, &[ext])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let r = if glb {
            ferro_io::export_glb(self.doc())
                .map_err(|e| e.to_string())
                .and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string()))
        } else {
            ferro_io::export_obj(self.doc())
                .map_err(|e| e.to_string())
                .and_then(|s| std::fs::write(&p, s).map_err(|e| e.to_string()))
        };
        match r {
            Ok(()) => self.say(format!("Exported {}", p.display()), false),
            Err(e) => self.say(format!("Export failed: {e}"), true),
        }
    }

    fn export_ifc(&mut self) {
        let name = format!("{}.ifc", self.doc().project().name);
        let Some(p) = rfd::FileDialog::new()
            .add_filter("IFC4", &["ifc"])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let r = ferro_io::export_ifc(self.doc())
            .map_err(|e| e.to_string())
            .and_then(|t| std::fs::write(&p, t).map_err(|e| e.to_string()));
        match r {
            Ok(()) => self.say(format!("Exported {}", p.display()), false),
            Err(e) => self.say(format!("Export failed: {e}"), true),
        }
    }

    fn suggest_level_name(&mut self) {
        let levels = design_options::view_doc(&self.session, &self.options).levels();
        self.new_level_name = format!("L{}", levels.len() + 2);
        self.new_level_elev = levels
            .last()
            .and_then(|e| e.as_level())
            .map(|l| l.elevation.mm() + 3300.0)
            .unwrap_or(3300.0);
    }

    /// Paste: the copied elements follow the cursor from their corner; click
    /// to place (a Move-copy).
    fn paste(&mut self) {
        let elems: Vec<ElementId> = self
            .clipboard
            .iter()
            .copied()
            .filter(|e| self.doc().contains(*e))
            .collect();
        if elems.is_empty() {
            return;
        }
        let bb = elems
            .iter()
            .filter_map(|e| self.doc().get(*e).and_then(|x| x.bounding_box()))
            .reduce(|a, b| a.union(b));
        let Some(bb) = bb else { return };
        self.tool.start_paste(elems, bb.min);
        self.vcb.clear();
        self.say("Click to place the copy", false);
    }

    fn materials_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Materials");
        let mats: Vec<(ElementId, String, [f32; 3])> = self
            .session
            .doc()
            .elements()
            .filter_map(|e| match &e.body {
                ElementBody::Material(m) => Some((e.id, e.label(), m.color)),
                _ => None,
            })
            .collect();
        for (id, name, [r, g, b]) in &mats {
            ui.horizontal(|ui| {
                let c = Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::hover());
                ui.painter().rect_filled(rect, 3.0, c);
                if ui
                    .selectable_label(self.material == Some(*id), name)
                    .clicked()
                {
                    self.material = Some(*id);
                    self.set_tool(ToolKind::Paint);
                }
                if ui
                    .small_button("Edit")
                    .on_hover_text("Edit material")
                    .clicked()
                {
                    self.open_material_editor(*id);
                }
            });
        }
        self.material_editor_ui(ui);
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(&mut self.new_material_color);
            ui.add(egui::TextEdit::singleline(&mut self.new_material_name).desired_width(110.0));
            if ui.button("+ Add").clicked() {
                let [r, g, b] = self.new_material_color.map(|c| (c * 255.0).round() as u8);
                let name = self.new_material_name.trim().to_string();
                if !name.is_empty()
                    && self.run(
                        ferro_core::ops::bim_commands::CreateMaterial {
                            name,
                            color: format!("#{r:02x}{g:02x}{b:02x}"),
                            ..Default::default()
                        }
                        .into(),
                    )
                {
                    self.material = self.selection.iter().next().copied();
                    self.selection.clear();
                    self.set_tool(ToolKind::Paint);
                    self.new_material_name = format!("Material {}", mats.len() + 2);
                }
            }
        });
    }

    fn open_material_editor(&mut self, id: ElementId) {
        let doc = self.doc();
        self.material_draft = doc.get(id).and_then(|e| match &e.body {
            ElementBody::Material(m) => Some(MaterialDraft::from_material(id, e.label(), m)),
            _ => None,
        });
    }

    fn material_editor_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.material_draft.take() else {
            return;
        };
        if !self.doc().contains(draft.material) {
            return; // deleted (or undone) meanwhile
        }
        ui.separator();
        match draft.ui(ui) {
            DraftAction::Close => return,
            DraftAction::Apply => match draft.to_command() {
                Ok(cmd) => {
                    let id = draft.material;
                    let current = self.doc().get(id).map(|e| e.label());
                    let name = draft.name.trim().to_string();
                    if !name.is_empty() && current.as_deref() != Some(name.as_str()) {
                        self.run(
                            SetName {
                                element: id,
                                name: Some(name),
                            }
                            .into(),
                        );
                    }
                    if self.run(cmd.into()) {
                        // Show the stored (validated) values.
                        self.open_material_editor(id);
                        return;
                    }
                }
                Err(msg) => self.say(msg, true),
            },
            DraftAction::None => {}
        }
        self.material_draft = Some(draft);
    }

    /// The Project settings window: its edits run as commands (undoable).
    fn project_settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.project_settings.take() else {
            return;
        };
        let mut open = true;
        let mut cmds = Vec::new();
        egui::Window::new("Project settings")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| cmds = draft.ui(ui, self.doc()));
        for c in cmds {
            self.run(c);
        }
        if open {
            self.project_settings = Some(draft);
        }
    }

    fn level_info(&self) -> Option<(ElementId, f64)> {
        let id = self.active_level?;
        let e = self.doc().get(id)?;
        Some((id, e.as_level()?.elevation.mm()))
    }

    // ------------------------------------------------------------ input

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        // Esc twice within a second stops a running script (even in a text
        // box: the user always can).
        if pressed(Key::Escape) && !self.script_wait.asking() {
            self.escape_while_script();
        }
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
        if cmd && pressed(Key::K) {
            self.palette = Some(String::new());
        }
        // egui turns Ctrl+C / Ctrl+V into Copy / Paste events.
        let copy_ev = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        let paste_ev = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        if (copy_ev || cmd && pressed(Key::C)) && !typing {
            let mut v: Vec<ElementId> = self.selection.iter().copied().collect();
            v.sort();
            // Put a marker on the system clipboard too: egui only reports
            // Ctrl+V when the clipboard holds text.
            ctx.copy_text(format!(
                "ferro-elements:{}",
                v.iter()
                    .map(|e| e.0.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            self.clipboard = v;
            let n = self.clipboard.len();
            self.say(format!("Copied {n} element(s)"), false);
        }
        if (paste_ev || cmd && pressed(Key::V)) && !typing {
            self.paste();
        }
        if cmd && pressed(Key::A) && !typing {
            self.selection = self
                .session
                .doc()
                .elements()
                .filter(|e| e.geometry().is_some())
                .map(|e| e.id)
                .collect();
        }
        if typing || cmd {
            return;
        }
        // "Just type" a value while a tool is active (SketchUp VCB), or
        // right after a Move-copy (arrays: 5x, /5).
        if self.tool.is_active() || self.last_copy.is_some() {
            let text: String = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect()
            });
            // After a copy only array input is captured (5x, /5); other keys
            // stay tool shortcuts.
            let array_only = !self.tool.is_active();
            let text: String = if array_only {
                text.chars()
                    .filter(|c| c.is_ascii_digit() || "xX*/".contains(*c))
                    .collect()
            } else {
                text
            };
            if !text.is_empty() {
                self.vcb.push_str(&text);
                return;
            }
        }
        // Arrow keys lock drawing to an axis (press again to release).
        for (k, a) in [
            (Key::ArrowRight, ferro_snap::Axis::X),
            (Key::ArrowLeft, ferro_snap::Axis::Y),
            (Key::ArrowUp, ferro_snap::Axis::Z),
        ] {
            if pressed(k) {
                self.axis_lock = if self.axis_lock == Some(a) {
                    None
                } else {
                    Some(a)
                };
            }
        }
        if pressed(Key::ArrowDown) {
            self.axis_lock = None;
        }
        if pressed(Key::Escape) {
            self.axis_lock = None;
            if self.tool.is_active() || !self.vcb.is_empty() {
                self.tool.cancel();
                self.vcb.clear();
            } else if self.editing.is_some() {
                if self.sub_sel.is_empty() {
                    self.editing = None;
                } else {
                    self.sub_sel.clear();
                }
            } else {
                self.selection.clear();
            }
        }
        if pressed(Key::Backspace) {
            if !self.vcb.is_empty() {
                self.vcb.pop();
            } else if self.tool.kind.is_chain() && self.tool.is_active() {
                self.tool.backspace();
            } else {
                self.delete_selection();
            }
        }
        // Shortcuts from the keymap: single keys and two-letter codes.
        let (now, presses): (f64, Vec<(Key, bool)>) = ctx.input(|i| {
            (
                i.time,
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if !modifiers.command && !modifiers.alt => Some((*key, modifiers.shift)),
                        _ => None,
                    })
                    .collect(),
            )
        });
        for (key, shift) in presses {
            let name = key.name();
            let target = match key {
                Key::Space => self.keys.named(&self.keymap, "SPACE"),
                Key::Delete => self.keys.named(&self.keymap, "DELETE"),
                _ if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic()) => {
                    if shift {
                        self.keys.named(&self.keymap, &format!("SHIFT+{name}"))
                    } else {
                        let c = name.chars().next().unwrap_or(' ');
                        self.keys.letter(&self.keymap, c, now)
                    }
                }
                _ => None,
            };
            if let Some(t) = target {
                // Tools switch only when none is in the middle of a pick.
                if t.tool().is_none() || !self.tool.is_active() {
                    self.run_target(t);
                }
            }
        }
    }

    /// Do what a shortcut or the command palette asks.
    fn run_target(&mut self, t: Target) {
        match (t.tool(), t) {
            (Some(k), _) => self.set_tool(k),
            (None, Target::Hide) => self.hide_selection(),
            (None, Target::UnhideAll) => self.unhide_all_in_view(),
            (None, Target::ZoomExtents) => self.camera.zoom_extents(scene::doc_bounds(self.doc())),
            (None, Target::Pin) => self.pin_selection(true),
            (None, Target::Unpin) => self.pin_selection(false),
            (None, _) => self.delete_selection(),
        }
    }

    fn load_keymap(&mut self) {
        let Some(path) = ferro_tools::link::ferro_dir().map(|d| d.join("keymap.json")) else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match serde_json::from_str::<KeymapFile>(&text) {
            Ok(f) => {
                let (k, bad) = Keymap::from_file(&f);
                self.keymap = k;
                if !bad.is_empty() {
                    self.say(
                        format!("keymap.json: ignored keys {}", bad.join(", ")),
                        true,
                    );
                }
            }
            Err(e) => self.say(format!("keymap.json: {e}"), true),
        }
    }

    fn set_scheme(&mut self, p: KeyScheme) {
        self.keymap = Keymap::scheme(p);
        if let Some(dir) = ferro_tools::link::ferro_dir() {
            let f = KeymapFile {
                scheme: p,
                bindings: Default::default(),
            };
            if let Ok(text) = serde_json::to_string_pretty(&f) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join("keymap.json"), text);
            }
        }
    }

    /// Review and send a problem report. Nothing leaves the computer unless
    /// the user presses "Open GitHub issue" (and then submits it there).
    fn report_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.report.take() else {
            return;
        };
        let mut keep = true;
        let crash = d.crash_file.is_some();
        egui::Window::new(if crash {
            "Ferro closed unexpectedly last time"
        } else {
            "Report a problem"
        })
        .collapsible(false)
        .default_width(560.0)
        .show(ctx, |ui| {
            ui.label("Describe what you were doing. The report below holds the app version, your system and the recent commands — no model data. Read it before sending.");
            ui.add(
                egui::TextEdit::multiline(&mut d.what)
                    .hint_text("What happened? What did you expect?")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut d.body)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY),
                );
            });
            ui.horizontal(|ui| {
                let full = format!("{}

{}", d.what.trim(), d.body);
                if ui.button("Open GitHub issue").clicked() {
                    let saved = crate::report::save(&full, "report");
                    let title = if crash { "Crash report" } else { "Problem report" };
                    match crate::report::open_url(&crate::report::issue_url(title, &full)) {
                        Ok(()) => self.say(
                            match saved {
                                Some(p) => format!("Report saved to {}", p.display()),
                                None => "Opened the issue page".into(),
                            },
                            false,
                        ),
                        Err(e) => self.say(format!("Could not open the browser: {e}"), true),
                    }
                    keep = false;
                }
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(full.clone());
                }
                if ui.button("Save file").clicked() {
                    if let Some(p) = crate::report::save(&full, "report") {
                        self.say(format!("Report saved to {}", p.display()), false);
                    }
                }
                if ui.button(if crash { "Don't send" } else { "Cancel" }).clicked() {
                    keep = false;
                }
            });
        });
        if keep {
            self.report = Some(d);
        } else if let Some(p) = &d.crash_file {
            crate::report::dismiss(p);
        }
    }

    /// Ctrl+K: type a tool name or shortcut, Enter runs the first match.
    fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut q) = self.palette.take() else {
            return;
        };
        let mut open = true;
        let mut run = None;
        egui::Window::new("Command palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text("tool name or shortcut, e.g. wall or WA")
                        .desired_width(320.0),
                );
                r.request_focus();
                let needle = q.trim().to_lowercase();
                let hits: Vec<Target> = Target::ALL
                    .into_iter()
                    .filter(|t| {
                        needle.is_empty()
                            || t.label().to_lowercase().contains(&needle)
                            || self
                                .keymap
                                .keys_for(*t)
                                .iter()
                                .any(|k| k.to_lowercase() == needle)
                    })
                    .collect();
                for t in &hits {
                    let label = format!("{:<16} {}", t.label(), self.keymap.label_for(*t));
                    if ui
                        .selectable_label(false, RichText::new(label).monospace())
                        .clicked()
                    {
                        run = Some(*t);
                    }
                }
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    run = hits.first().copied();
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    open = false;
                }
            });
        match run {
            Some(t) => self.run_target(t),
            None if open => self.palette = Some(q),
            None => {}
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
                if ui.button("Export IFC4…").clicked() {
                    self.export_ifc();
                }
                ui.separator();
                if ui.button("Project settings…").clicked() {
                    self.project_settings.get_or_insert_with(Default::default);
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
                ui.separator();
                if ui
                    .add_enabled(!self.selection.is_empty(), egui::Button::new("Hide (H)"))
                    .clicked()
                {
                    self.hide_selection();
                }
                if ui
                    .add_enabled(
                        !self.hidden.is_empty(),
                        egui::Button::new("Unhide all (Shift+H)"),
                    )
                    .clicked()
                {
                    self.unhide_all_in_view();
                }
                ui.separator();
                if ui.button("Command palette…   Ctrl+K").clicked() {
                    self.palette = Some(String::new());
                    ui.close();
                }
                ui.menu_button("Keyboard shortcuts", |ui| {
                    for p in KeyScheme::ALL {
                        if ui.radio(self.keymap.scheme == p, p.label()).clicked() {
                            self.set_scheme(p);
                        }
                    }
                    ui.label(RichText::new("Custom keys: ~/.ferro/keymap.json").weak());
                });
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
                    self.camera.zoom_extents(scene::doc_bounds(self.doc()));
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
                if ui.button("Back").clicked() {
                    self.camera.set_view(90.0, 0.0);
                }
                if ui.button("Left").clicked() {
                    self.camera.set_view(180.0, 0.0);
                }
                ui.separator();
                ui.checkbox(&mut self.camera.parallel, "Parallel projection");
                // Display style of the active view (one undo step each).
                let style = self.display_style();
                for s in ferro_core::model::DisplayStyle::ALL {
                    if ui.radio(style == s, s.label()).clicked() {
                        self.set_display_style(s);
                    }
                }
                ui.separator();
                if ui.button("Plan section (1.2 m above level)").clicked() {
                    self.plan_section();
                    ui.close();
                }
                if ui
                    .add_enabled(self.section.is_some(), egui::Button::new("Flip section"))
                    .clicked()
                {
                    if let Some(mut p) = self.section {
                        p.normal = -p.normal;
                        self.set_section(Some(p));
                    }
                }
                if ui
                    .add_enabled(self.section.is_some(), egui::Button::new("Clear section"))
                    .clicked()
                {
                    self.set_section(None);
                }
            });
            ui.menu_button("AI", |ui| {
                if ui.button("AI chat…").clicked() {
                    self.chat.open = true;
                }
                if ui.button("Chat settings…").clicked() {
                    self.chat.show_settings = true;
                }
                if ui.button("Connect an AI agent (MCP)…").clicked() {
                    self.show_agents = true;
                }
                ui.separator();
                ui.label(RichText::new("Agent changes").weak());
                ui.radio_value(
                    &mut self.autonomy,
                    Autonomy::Propose,
                    "Propose (I review first)",
                );
                ui.radio_value(&mut self.autonomy, Autonomy::Apply, "Apply directly");
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Report a problem…").clicked() {
                    self.report = Some(ReportDraft::new(String::new(), None));
                    ui.close();
                }
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
            if k == ToolKind::Wall {
                ui.separator();
                ui.label(RichText::new("BIM").strong());
            }
            let selected = self.tool.kind == k;
            let key = self.keymap.label_for(Target::from_tool(k));
            let b = egui::Button::new(RichText::new(k.label()).size(15.0))
                .right_text(RichText::new(&key).weak().size(11.0))
                .selected(selected)
                .min_size(Vec2::new(124.0, 30.0));
            if ui
                .add(b)
                .on_hover_text(format!("{} ({key})", k.label()))
                .clicked()
            {
                self.set_tool(k);
            }
        }
        if self.workspace == Workspace::Bim {
            for t in ["Roof", "Room", "Stair"] {
                ui.add_enabled(false, egui::Button::new(t).min_size(Vec2::new(120.0, 26.0)))
                    .on_disabled_hover_text("Arrives in milestone 3");
            }
        }
        self.renovation_toolbar(ui);
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Project");
            let mut name = self.doc().project().name.clone();
            if ui.text_edit_singleline(&mut name).lost_focus() && name != self.doc().project().name
            {
                self.run(
                    SetProjectName {
                        name: Some(name),
                        auto_pin_datums: None,
                    }
                    .into(),
                );
            }
            ui.separator();
            self.levels_ui(ui);
            ui.separator();
            self.views_ui(ui);
            ui.separator();
            self.phases_ui(ui);
            ui.separator();
            self.design_options_ui(ui);
            ui.separator();
            self.properties_ui(ui);
            ui.separator();
            self.materials_ui(ui);
            ui.separator();
            self.tags_ui(ui);
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
                    e.as_level().map(|l| l.elevation.mm()).unwrap_or(0.0),
                )
            })
            .collect();
        let u = self.doc().project().settings.units.clone();
        let ground = format!("Ground ({})", units::length(&u, 0.0));
        if ui.radio(self.active_level.is_none(), ground).clicked() {
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
                let r = units::length_drag(ui, &u, v);
                if (r.drag_stopped() || r.lost_focus()) && (*v - elev).abs() > 1e-9 {
                    commit = Some((*id, *v));
                }
                if !r.dragged() && !r.has_focus() && (*v - elev).abs() > 1e-9 && commit.is_none() {
                    *v = *elev;
                }
            });
        }
        if let Some((level, mm)) = commit {
            self.run(
                SetLevelElevation {
                    level,
                    elevation: Length::from_mm(mm),
                }
                .into(),
            );
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_level_name).desired_width(60.0));
            units::length_drag(ui, &u, &mut self.new_level_elev);
            if ui.button("+ Level").clicked() {
                let cmd = CreateLevel {
                    name: self.new_level_name.clone(),
                    elevation: Length::from_mm(self.new_level_elev),
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
        let u = self.doc().project().settings.units.clone();
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
        let Some(e) = self.doc().get(id).cloned() else {
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

                if let Some(src) = e.geometry_source() {
                    use ferro_core::model::GeometrySource;
                    ui.label("Geometry");
                    ui.horizontal(|ui| {
                        let (label, other, button, tip) = match src {
                            GeometrySource::Parametric => (
                                "Parametric",
                                GeometrySource::Direct,
                                "Edit freely",
                                "Freeze the shape so it can be edited like a SketchUp group; category, type and data stay",
                            ),
                            GeometrySource::Direct => (
                                "Direct",
                                GeometrySource::Parametric,
                                "Make parametric",
                                "Turn a plain wall or slab shape back into a parametric wall / floor",
                            ),
                        };
                        ui.label(label);
                        if ui.small_button(button).on_hover_text(tip).clicked() {
                            self.run(
                                ferro_core::ops::geometry_source::SetGeometrySource {
                                    element: id,
                                    source: other,
                                }
                                .into(),
                            );
                        }
                    });
                    ui.end_row();
                }

                if let Some(l) = e.level_id.and_then(|l| self.doc().get(l)) {
                    ui.label("Level");
                    ui.label(l.label());
                    ui.end_row();
                }
                // Type: parametric walls / floors, and direct ones of that category.
                let type_of = match (&e.body, e.category) {
                    (ElementBody::Wall(_), _) | (ElementBody::Form(_), Category::Walls) => Some(true),
                    (ElementBody::Floor(_), _) | (ElementBody::Form(_), Category::Floors) => Some(false),
                    _ => None,
                };
                if let Some(is_wall) = type_of {
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
                            .and_then(|t| self.doc().get(t))
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
                            ui.label(units::lengths(&u, &[s.x, s.y, s.z]));
                            ui.end_row();
                        }
                    }
                    ElementBody::Level(l) => {
                        ui.label("Elevation");
                        ui.label(units::length(&u, l.elevation.mm()));
                        ui.end_row();
                    }
                    ElementBody::Wall(_) | ElementBody::Floor(_) => {
                        for (k, label) in [
                            (keys::LENGTH, "Length"),
                            (keys::HEIGHT, "Height"),
                            (keys::THICKNESS, "Thickness"),
                        ] {
                            if let Some(ParamValue::Length(l)) = e.param(k) {
                                ui.label(label);
                                ui.label(units::length(&u, l.mm()));
                                ui.end_row();
                            }
                        }
                        if let Some(ParamValue::Area(a)) = e.param(keys::AREA) {
                            ui.label("Area");
                            ui.label(units::area(&u, *a));
                            ui.end_row();
                        }
                        if let Some(ParamValue::Volume(v)) = e.param(keys::VOLUME) {
                            ui.label("Volume");
                            ui.label(units::volume(&u, *v));
                            ui.end_row();
                        }
                    }
                    ElementBody::Opening(o) => {
                        if let Some(h) = e.host_id.and_then(|h| self.doc().get(h)) {
                            ui.label("Host");
                            ui.label(h.label());
                            ui.end_row();
                        }
                        ui.label("Size");
                        ui.label(units::lengths(&u, &[o.width.mm(), o.height.mm()]));
                        ui.end_row();
                        ui.label("Sill");
                        ui.label(units::length(&u, o.sill.mm()));
                        ui.end_row();
                    }
                    ElementBody::WallType(ferro_core::model::bim::WallType { structure })
                    | ElementBody::FloorType(ferro_core::model::bim::FloorType { structure }) => {
                        ui.label("Thickness");
                        ui.label(units::length(&u, structure.thickness()));
                        ui.end_row();
                        for l in &structure.layers {
                            ui.label(format!("{:?}", l.function));
                            let mat = l
                                .material
                                .and_then(|m| self.doc().get(m))
                                .map(|m| m.label())
                                .unwrap_or_else(|| "—".into());
                            ui.label(format!("{}  {mat}", units::length(&u, l.thickness.mm())));
                            ui.end_row();
                        }
                        self.type_quantities_ui(ui, id);
                    }
                    ElementBody::Material(m) => {
                        ui.label("Colour");
                        let [r, g, b] = m.color.map(|c| (c * 255.0).round() as u8);
                        ui.colored_label(Color32::from_rgb(r, g, b), "■■■");
                        ui.end_row();
                    }
                    ElementBody::View(v) => self.view_rows(ui, v),
                    ElementBody::Phase(p) => {
                        for (label, d) in [("Start", &p.start_date), ("End", &p.end_date)] {
                            ui.label(label);
                            ui.label(d.as_deref().unwrap_or("—"));
                            ui.end_row();
                        }
                    }
                    ElementBody::OptionSet(_) => {}
                    ElementBody::DesignOption(o) => {
                        ui.label("Status");
                        ui.label(o.status.as_str());
                        ui.end_row();
                        ui.label("Changes");
                        ui.label(o.changes.len().to_string());
                        ui.end_row();
                    }
                }
                if let Some(ParamValue::Area(a)) = e.param(keys::FORM_AREA) {
                    ui.label("Surface area");
                    ui.label(units::area(&u, *a));
                    ui.end_row();
                }
                if let Some(ParamValue::Volume(v)) = e.param(keys::FORM_VOLUME) {
                    ui.label("Volume");
                    ui.label(units::volume(&u, *v));
                    ui.end_row();
                }
                // Quantity parameters in display units (ADR 0004 §2), e.g.
                // "0.8 W/(m·K)"; built-in ones are shown above.
                for (k, p) in e
                    .params
                    .iter()
                    .filter(|(k, p)| !k.starts_with("ferro.") && p.spec().is_some())
                {
                    ui.label(k.as_str());
                    ui.label(match p.as_quantity() {
                        Some((spec, v)) => units::quantity(&u, spec, v),
                        None => p.display(),
                    });
                    ui.end_row();
                }

                self.element_phase_rows(ui, &e);
                self.pinned_row(ui, &e);

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

    /// Read-only quantities of a wall / floor type from its layer materials
    /// (U-value, mass, GWP, cost per m²); "—" with the layers that lack data.
    fn type_quantities_ui(&self, ui: &mut egui::Ui, id: ElementId) {
        use ferro_core::model::structure_quantities::{StructureQuantities, StructureQuantity};
        let Some(q) = StructureQuantities::from_type(self.doc(), id) else {
            return;
        };
        let lacking = |what: StructureQuantity| -> String {
            let layers: Vec<String> = q
                .gaps
                .iter()
                .filter(|g| g.quantity == what)
                .map(|g| format!("layer {}", g.layer + 1))
                .collect();
            format!("— (no data: {})", layers.join(", "))
        };
        let rows = [
            (
                "U-value",
                StructureQuantity::UValue,
                q.u_value_w_m2k.map(|v| {
                    units::quantity(
                        &self.doc().project().settings.units,
                        ferro_core::math::QuantitySpec::UValue,
                        v,
                    )
                }),
            ),
            (
                "Mass",
                StructureQuantity::Mass,
                q.mass_kg_m2.map(|v| format!("{v:.1} kg/m²")),
            ),
            (
                "GWP A1–A3",
                StructureQuantity::Gwp,
                q.gwp_a1_a3_kgco2e_m2.map(|v| format!("{v:.1} kg CO₂e/m²")),
            ),
            (
                "Cost",
                StructureQuantity::Cost,
                q.cost_per_m2.map(|v| {
                    units::cost_per_m2(
                        v,
                        q.currency.as_deref(),
                        self.doc().project().settings.currency.as_deref(),
                    )
                }),
            ),
        ];
        let mismatch = units::currency_mismatch(
            q.currency.as_deref(),
            self.doc().project().settings.currency.as_deref(),
        );
        for (label, what, value) in rows {
            ui.label(label);
            match value {
                Some(v) if mismatch && what == StructureQuantity::Cost => ui
                    .colored_label(Color32::from_rgb(200, 120, 0), v)
                    .on_hover_text("The materials are priced in another currency than the project's; costs are not converted."),
                Some(v) => ui.label(v),
                None => ui.label(RichText::new(lacking(what)).weak()),
            };
            ui.end_row();
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.script_status(ui);
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
                self.option_status(ui);
                if let Some(s) = self.last_snap {
                    ui.label(RichText::new(s.kind.label()).weak());
                }
            });
        });
    }

    fn apply_vcb(&mut self) {
        let text = std::mem::take(&mut self.vcb);
        if self.python.is_some() {
            self.hold_for_script(python::UserAction::Value(text));
            return;
        }
        // After a Move-copy: "5x" repeats it 5 times, "/5" divides it in 5.
        let t = text.trim();
        if let Some(copy) = self.last_copy.clone().filter(|_| !self.tool.is_active()) {
            let times = t
                .strip_suffix(['x', 'X', '*'])
                .or_else(|| t.strip_prefix(['x', 'X', '*']));
            let parts = t.strip_prefix('/');
            let array = match (
                times.and_then(|n| n.trim().parse::<u32>().ok()),
                parts.and_then(|n| n.trim().parse::<u32>().ok()),
            ) {
                (Some(n), _) if n >= 1 => Some(CopyElements {
                    count: n,
                    ..copy.clone()
                }),
                (_, Some(n)) if n >= 1 => Some(CopyElements {
                    count: n,
                    vector: copy.vector * (1.0 / n as f64),
                    ..copy.clone()
                }),
                _ => None,
            };
            if let Some(a) = array {
                self.session.undo();
                self.run(a.into());
                return;
            }
        }
        let Some(ctx_parts) = self.tool_ctx_parts() else {
            return;
        };
        let (ray, snap) = ctx_parts;
        let level = self.level_info();
        let ev = {
            let ctx = ToolCtx {
                doc: design_options::view_doc(&self.session, &self.options),
                snap,
                ray,
                selection: &self.selection,
                level,
                copy: false,
                near_first_px: None,
                material: self.material,
                editing: self.editing,
                sub: &self.sub_sel,
            };
            self.tool.typed(&text, &ctx)
        };
        match ev {
            Ok(ToolEvent::Commit(c)) => {
                self.run(c);
            }
            Ok(ToolEvent::Section(p)) => self.set_section(Some(p)),
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
        let mut moving: Vec<ElementId> = self.tool.moving().to_vec();
        let mut hidden = self.hidden_elements();
        let phased = self.phase_display();
        hidden.extend(phased.hidden.iter().copied());
        self.scene.phase = phased.looks;
        moving.extend(hidden.iter().copied());
        moving.sort();
        moving.dedup();
        self.selection.retain(|id| !hidden.contains(id));
        self.scene.hidden = hidden;
        let rebuild = match &self.snap_scene {
            Some((d, m, _)) => !scene::same_doc(d, self.doc()) || *m != moving,
            None => true,
        };
        if rebuild {
            self.snap_scene = Some((
                self.doc().clone(),
                moving.clone(),
                SnapScene::from_document(self.doc(), &moving),
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
                reference: self.ref_dir,
                ..Default::default()
            };
            if let Some((_, _, sc)) = &self.snap_scene {
                snap_res = snap(sc, &ray, [p.x, p.y], &fc, self.tool.anchor(), &settings);
            }
            if let Some(d) = snap_res.and_then(|s| s.edge_dir) {
                self.ref_dir = Some(d);
            }
            // Inference locking: Shift keeps the axis currently inferred;
            // arrow keys force one.
            if ctx.input(|i| i.modifiers.shift) && self.tool.anchor().is_some() {
                if self.shift_lock.is_none() {
                    if let (Some(s), Some(a)) = (snap_res, self.tool.anchor()) {
                        let lockable = matches!(
                            s.kind,
                            SnapKind::OnAxis(_) | SnapKind::Parallel | SnapKind::Perpendicular
                        );
                        if let (true, Some(d)) = (lockable, (s.point - a).normalize()) {
                            self.shift_lock = Some((s.kind, d.as_vec()));
                        }
                    }
                }
            } else {
                self.shift_lock = None;
            }
            let arrow = self.axis_lock.map(|axis| {
                let d = match axis {
                    ferro_snap::Axis::X => ferro_core::Vec3::X,
                    ferro_snap::Axis::Y => ferro_core::Vec3::Y,
                    ferro_snap::Axis::Z => ferro_core::Vec3::Z,
                };
                (SnapKind::OnAxis(axis), d)
            });
            if let (Some((kind, d)), Some(anchor)) = (arrow.or(self.shift_lock), self.tool.anchor())
            {
                // Closest point on the locked line to the cursor ray; a
                // snapped point is projected onto the line instead.
                let point = match snap_res.filter(|s| {
                    !matches!(
                        s.kind,
                        SnapKind::Ground
                            | SnapKind::OnFace
                            | SnapKind::OnAxis(_)
                            | SnapKind::Parallel
                            | SnapKind::Perpendicular
                    )
                }) {
                    Some(s) => anchor + d * (s.point - anchor).dot(d),
                    None => {
                        let r = ray.dir.as_vec();
                        let w = ray.origin - anchor;
                        let (dr, rr, dw, rw) = (d.dot(r), r.dot(r), d.dot(w), r.dot(w));
                        let den = rr - dr * dr;
                        let t = if den.abs() < 1e-9 {
                            0.0
                        } else {
                            (rr * dw - dr * rw) / den
                        };
                        anchor + d * t
                    }
                };
                snap_res = Some(SnapResult {
                    point,
                    kind,
                    element: None,
                    face: None,
                    normal: None,
                    edge_dir: None,
                });
            }
            near_first = self
                .tool
                .polygon_first()
                .and_then(|f| fc.project(f))
                .map(|s| ((s[0] - p.x).powi(2) + (s[1] - p.y).powi(2)).sqrt());
        }
        self.last_snap = snap_res;

        // Window (left→right: fully inside) / crossing (right→left: touching)
        // selection with the Select tool.
        let mut sel_rect: Option<(Rect, bool)> = None;
        if self.tool.kind == ToolKind::Select && !ctx.input(|i| i.modifiers.alt) {
            if resp.drag_started_by(PointerButton::Primary) {
                self.select_drag = ctx.input(|i| i.pointer.press_origin());
            }
            if let (Some(a), Some(b)) = (self.select_drag, hover) {
                let r = Rect::from_two_pos(a, b);
                let window = b.x >= a.x;
                sel_rect = Some((r, window));
                if resp.drag_stopped_by(PointerButton::Primary) {
                    let mut picked = Vec::new();
                    for e in self.doc().elements() {
                        if self.scene.hidden.contains(&e.id) {
                            continue;
                        }
                        let Some(m) = self.scene.mesh(e.id) else {
                            continue;
                        };
                        let pts: Vec<Pos2> = m
                            .positions
                            .iter()
                            .filter_map(|p| fc.project(*p))
                            .map(|q| Pos2::new(q[0], q[1]))
                            .collect();
                        if pts.is_empty() {
                            continue;
                        }
                        let hit = if window {
                            pts.len() == m.positions.len() && pts.iter().all(|q| r.contains(*q))
                        } else {
                            let bb = Rect::from_points(&pts);
                            pts.iter().any(|q| r.contains(*q)) || bb.intersects(r)
                        };
                        if hit {
                            picked.push(e.id);
                        }
                    }
                    if !ctx.input(|i| i.modifiers.shift) {
                        self.selection.clear();
                    }
                    for id in picked {
                        if !self.selection.insert(id) && ctx.input(|i| i.modifiers.shift) {
                            self.selection.remove(&id);
                        }
                    }
                    self.select_drag = None;
                    sel_rect = None;
                }
            }
            if !resp.dragged_by(PointerButton::Primary)
                && !resp.drag_stopped_by(PointerButton::Primary)
            {
                self.select_drag = None;
            }
        }

        // Tool interaction.
        let shift = ctx.input(|i| i.modifiers.shift);
        let copy = ctx.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);
        let mut event = ToolEvent::None;
        let (mut enter_edit, mut exit_edit) = (None, false);
        let (mut sub_set, mut sub_toggle) = (None, None);
        let (preview_cmd, hover_face, rubber, section_live) = {
            let tctx = ToolCtx {
                doc: design_options::view_doc(&self.session, &self.options),
                snap: snap_res,
                ray,
                selection: &self.selection,
                level,
                copy,
                near_first_px: near_first,
                material: self.material,
                editing: self.editing,
                sub: &self.sub_sel,
            };
            let form_under = snap_res.and_then(|s| s.element).filter(|e| {
                self.session
                    .doc()
                    .get(*e)
                    .is_some_and(|x| x.as_form().is_some())
            });
            if self.tool.kind == ToolKind::Select && resp.double_clicked_by(PointerButton::Primary)
            {
                if let Some(e) = form_under {
                    enter_edit = Some(e);
                }
            } else if resp.double_clicked_by(PointerButton::Primary) {
                event = self.tool.double_click(&tctx);
            } else if let (ToolKind::Select, Some(e), true) = (
                self.tool.kind,
                self.editing,
                resp.clicked_by(PointerButton::Primary),
            ) {
                match snap_res.and_then(|s| pick_sub(self.doc(), e, &s)) {
                    Some(p) if shift => sub_toggle = Some(p),
                    Some(p) => sub_set = Some(p),
                    None => exit_edit = true,
                }
            } else if resp.clicked_by(PointerButton::Primary) && !ctx.input(|i| i.modifiers.alt) {
                event = self.tool.click(&tctx, shift);
            }
            if !self.tool.is_active() || self.vcb.is_empty() {
                // live value
            }
            (
                self.tool.preview(&tctx),
                self.tool.hover_face(&tctx),
                self.tool.rubber_band(&tctx),
                self.tool.section_preview(&tctx),
            )
        };
        if self.tool.is_active() && self.vcb.is_empty() {
            // Nothing typed: show live measurement as a hint (handled in status bar).
        }
        if ctx.input(|i| i.key_pressed(Key::Enter))
            && !ctx.egui_wants_keyboard_input()
            && (self.tool.is_active() || self.tool.kind.is_chain() || !self.vcb.is_empty())
        {
            self.apply_vcb();
        }
        if !matches!(event, ToolEvent::None) || resp.clicked_by(PointerButton::Primary) {
            self.axis_lock = None;
        }
        if let Some(e) = enter_edit {
            self.editing = Some(e);
            self.sub_sel.clear();
            self.selection.clear();
            self.say("Editing the Form: click faces, edges or corners; Move stretches them; Esc to finish", false);
        }
        if exit_edit {
            self.editing = None;
            self.sub_sel.clear();
        }
        if let Some(p) = sub_set {
            self.sub_sel = vec![p];
        }
        if let Some(p) = sub_toggle {
            if let Some(i) = self.sub_sel.iter().position(|x| *x == p) {
                self.sub_sel.remove(i);
            } else {
                self.sub_sel.push(p);
            }
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
            ToolEvent::Message(m) => self.say(m, false),
            ToolEvent::Section(p) => self.set_section(Some(p)),
            ToolEvent::None => {}
        }

        // Document to display: the committed one, or the tool preview.
        let mut display = self.doc().clone();
        let mut preview_err = None;
        if let Some(cmd) = preview_cmd {
            match self.preview_here(&cmd) {
                Ok((d, _)) => display = d,
                Err(f) => preview_err = Some(failure_text(&f)),
            }
        }
        self.scene.section = section_live.or(self.section).map(|p| (p.point, p.normal));
        self.scene.hidden_line = self.hidden_line;
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
        let mut olines: Vec<Vertex> = self.scene.cut.clone();
        olines.extend(scene::grid_lines(
            origin,
            self.camera.target,
            (self.camera.distance * 1.2).clamp(10_000.0, 500_000.0),
            eye,
            forward,
            (self.camera.distance * 0.01).max(50.0),
        ));
        if let Some((e, f)) = hover_face {
            if let Some(m) = self.scene.mesh(e) {
                for t in scene::face_triangles(&m, f) {
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
        if let Some(e) = self.editing {
            let sel = [0.18f32, 0.45, 0.95];
            if let Some(g) = display.get(e).and_then(|x| x.geometry()) {
                for p in &self.sub_sel {
                    match p {
                        SubPick::Face(f) => {
                            if let Some(m) = self.scene.mesh(e) {
                                for t in scene::face_triangles(&m, *f) {
                                    let n = (t[1] - t[0])
                                        .cross(t[2] - t[0])
                                        .normalize()
                                        .map(|u| u.as_vec())
                                        .unwrap_or_default();
                                    for q in t {
                                        otris.push(Vertex {
                                            pos: rel(q + n * 2.0),
                                            normal: [n.x as f32, n.y as f32, n.z as f32],
                                            color: [sel[0], sel[1], sel[2], 0.45],
                                        });
                                    }
                                }
                            }
                        }
                        SubPick::Edge(x) => {
                            if let Some((_, a, b)) = g.edges().find(|(id, _, _)| id == x) {
                                olines.push(line_vertex(rel(a), [sel[0], sel[1], sel[2], 1.0]));
                                olines.push(line_vertex(rel(b), [sel[0], sel[1], sel[2], 1.0]));
                            }
                        }
                        SubPick::Vertex(_) => {}
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
                    xray: self.scene.xray,
                    clip: self.scene.section.map(|(p, n)| {
                        let n = n.as_vec();
                        [n.x as f32, n.y as f32, n.z as f32, n.dot(p - origin) as f32]
                    }),
                    hidden_line: self.hidden_line,
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

        if let Some((r, window)) = sel_rect {
            let (fill, stroke) = if window {
                (
                    Color32::from_rgba_unmultiplied(40, 90, 220, 30),
                    Color32::from_rgb(40, 90, 220),
                )
            } else {
                (
                    Color32::from_rgba_unmultiplied(40, 170, 80, 30),
                    Color32::from_rgb(40, 150, 70),
                )
            };
            painter.rect_filled(r, 0.0, fill);
            if window {
                painter.rect_stroke(
                    r,
                    0.0,
                    egui::Stroke::new(1.0, stroke),
                    egui::StrokeKind::Inside,
                );
            } else {
                for (a, b) in [
                    (r.left_top(), r.right_top()),
                    (r.right_top(), r.right_bottom()),
                    (r.right_bottom(), r.left_bottom()),
                    (r.left_bottom(), r.left_top()),
                ] {
                    painter.extend(egui::Shape::dashed_line(
                        &[a, b],
                        egui::Stroke::new(1.0, stroke),
                        6.0,
                        4.0,
                    ));
                }
            }
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
                    SnapKind::Parallel | SnapKind::Perpendicular => {
                        (Color32::from_rgb(200, 40, 200), true)
                    }
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
                    doc: self.doc(),
                    snap: snap_res,
                    ray,
                    selection: &self.selection,
                    level,
                    copy,
                    near_first_px: near_first,
                    material: self.material,
                    editing: self.editing,
                    sub: &self.sub_sel,
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
        self.pinned_markers(&painter, rect, &fc);
        resp.context_menu(|ui| self.pin_menu(ui));
        if let Some(e) = preview_err {
            painter.text(
                rect.left_bottom() + Vec2::new(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                e,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(200, 40, 40),
            );
        }
        if let Some(e) = self.editing {
            painter.text(
                rect.left_top() + Vec2::new(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "Editing {}  ·  Esc to finish",
                    self.session
                        .doc()
                        .get(e)
                        .map(|x| x.label())
                        .unwrap_or_default()
                ),
                egui::FontId::proportional(13.0),
                Color32::from_rgb(40, 90, 220),
            );
            if let Some(g) = self.doc().get(e).and_then(|x| x.geometry()) {
                for p in &self.sub_sel {
                    if let SubPick::Vertex(v) = p {
                        if let Some(q) = g.point(*v).and_then(|q| fc.project(q)) {
                            painter.circle_filled(
                                Pos2::new(q[0], q[1]),
                                4.0,
                                Color32::from_rgb(40, 90, 220),
                            );
                        }
                    }
                }
            }
        }
        // Axis gizmo label.
        painter.text(
            rect.right_top() + Vec2::new(-10.0, 10.0),
            egui::Align2::RIGHT_TOP,
            format!(
                "{}  ·  {} view",
                self.workspace.label(),
                units::length(&self.doc().project().settings.units, self.camera.distance)
            ),
            egui::FontId::proportional(12.0),
            Color32::from_gray(90),
        );

        if self.tool.is_active() || resp.hovered() {
            ctx.request_repaint();
        }
    }

    // ------------------------------------------------------------ agents

    fn accept_proposal(&mut self) {
        if self.hold_for_script(python::UserAction::AcceptProposal) {
            return;
        }
        if let Some(p) = self.proposal.take() {
            self.session.end_group();
            self.say(
                format!("Accepted {} change(s) from {}", p.steps, p.agent),
                false,
            );
            self.autosave();
        }
    }

    fn reject_proposal(&mut self) {
        if self.hold_for_script(python::UserAction::RejectProposal) {
            return;
        }
        if let Some(p) = self.proposal.take() {
            self.session.cancel_group();
            self.selection
                .retain(|id| design_options::view_doc(&self.session, &self.options).contains(*id));
            self.say(format!("Rejected the proposal from {}", p.agent), false);
        }
    }

    /// Answer queued Ferro Link calls (MCP agents and the chat panel).
    fn process_agent_calls(&mut self) {
        let name = self.doc().project().name.clone();
        self.link.set_project(&name);
        for p in self.link.drain() {
            let tool = p.req.tool.clone();
            let resp = if self.is_script_token(&p.req.token) {
                self.script_call(p.req)
            } else if tool == ferro_mcp::python::CANCEL_TOOL {
                self.cancel_from_link(&p.req)
            } else if let Some(busy) = self.refuse_while_script(&tool) {
                busy
            } else if tool == ferro_mcp::python::TOOL {
                // Answered when the script ends (`poll_python`).
                self.start_python(p.req, p.reply);
                continue;
            } else {
                self.agent_call(p.req)
            };
            // Tool name and outcome code only (no arguments: model data).
            let code = resp.body["failures"][0]["code"].as_str().unwrap_or("");
            crate::report::record(if resp.ok {
                format!("agent {tool} ok")
            } else {
                format!("agent {tool} failed {code}")
            });
            let _ = p.reply.send(resp);
        }
    }

    /// Who sent a Ferro Link request, and the context its changes carry.
    fn agent_ctx(req: &LinkRequest) -> (AgentInfo, ExecContext) {
        let info = req.agent.clone().unwrap_or(AgentInfo {
            name: "agent".into(),
            version: String::new(),
        });
        let ctx = ExecContext {
            actor: Actor::agent(info.name.clone(), info.version.clone()),
            prompt: req.prompt.clone(),
            timestamp_ms: now_ms(),
        };
        (info, ctx)
    }

    fn agent_call(&mut self, req: LinkRequest) -> LinkResponse {
        let (info, ctx) = Self::agent_ctx(&req);
        let answer = |r: Result<Value, Value>| match r {
            Ok(body) => LinkResponse { ok: true, body },
            Err(body) => LinkResponse { ok: false, body },
        };
        let tool = req.tool.as_str();
        match tool {
            "link_status" => {
                return answer(Ok(
                    json!({"linked": true, "project": self.doc().project().name, "autonomy": if self.autonomy == Autonomy::Propose { "propose" } else { "apply" }}),
                ))
            }
            "save_document" => {
                if self.proposal.is_some() {
                    return answer(Err(
                        json!({"error": "the user is still reviewing your proposal; it is saved once accepted"}),
                    ));
                }
                if self.path.is_none() {
                    return answer(Err(
                        json!({"error": "this project has no file yet; ask the user to use File > Save As"}),
                    ));
                }
                self.save();
                return answer(Ok(json!({"saved": self.path})));
            }
            "new_document" | "open_document" | "export_obj" | "export_glb" | "export_ifc" => {
                return answer(Err(
                    json!({"error": format!("{tool} is not available while linked to the open app; ask the user to do it in Ferro")}),
                ));
            }
            "get_selection" => {
                let mut sel: Vec<ElementId> = self.selection.iter().copied().collect();
                sel.sort();
                return answer(Ok(ferro_tools::summary_of(self.doc(), None, Some(&sel))));
            }
            "select_elements" => {
                let ids: Vec<ElementId> = req
                    .args
                    .get("elements")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                self.selection = ids
                    .into_iter()
                    .filter(|id| self.doc().contains(*id))
                    .collect();
                return answer(Ok(json!({"selected": self.selection.len()})));
            }
            "undo" | "redo" if self.proposal.is_some() => {
                return answer(Err(
                    json!({"error": "your changes are an open proposal; the user accepts or rejects them"}),
                ));
            }
            _ => {}
        }
        // Imports from Revit change the model too: they are proposals.
        let is_command = AnyCommand::NAMES.contains(&tool) || tool == "revit_import";
        if is_command && self.autonomy == Autonomy::Propose && self.proposal.is_none() {
            self.session
                .begin_group(format!("AI proposal ({})", info.name), ctx.clone());
            self.proposal = Some(Proposal {
                agent: info.name.clone(),
                steps: 0,
                targets: Vec::new(),
            });
        }
        // One rule with scripts (ADR 0006): while the user edits a design
        // option, an agent command lands in it unless it names a target.
        let (args, target) =
            match ferro_tools::with_default_design_option(tool, &req.args, self.editing_option()) {
                Ok(a) => a,
                Err(m) => {
                    if self.proposal.as_ref().is_some_and(|p| p.steps == 0) {
                        self.session.cancel_group();
                        self.proposal = None;
                    }
                    return answer(Err(json!({ "error": m })));
                }
            };
        let r = ferro_tools::call(&mut self.session, &ctx, tool, &args)
            .unwrap_or_else(|| Err(json!({"error": format!("unknown tool: {tool}")})));
        // An import that changed nothing is not a step.
        let no_op = tool == "revit_import"
            && r.as_ref().is_ok_and(|b| {
                let rep = &b["report"];
                rep["created"].as_array().is_none_or(|a| a.is_empty())
                    && rep["updated"].as_u64() == Some(0)
                    && rep["deleted"].as_u64() == Some(0)
            });
        if no_op {
            if self.proposal.as_ref().is_some_and(|p| p.steps == 0) {
                self.session.cancel_group();
                self.proposal = None;
            }
            return answer(r);
        }
        if is_command {
            match (&r, &mut self.proposal) {
                (Ok(_), Some(p)) => {
                    p.steps += 1;
                    python::note_target(&mut p.targets, target);
                    self.say(format!("{} proposes: {tool}", info.name), false);
                }
                (Ok(_), None) => {
                    self.say(format!("{}: {tool}", info.name), false);
                    self.autosave();
                }
                (Err(_), Some(p)) if p.steps == 0 => {
                    self.session.cancel_group();
                    self.proposal = None;
                }
                _ => {}
            }
        }
        answer(r)
    }

    fn proposal_bar(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.proposal else { return };
        let text = format!(
            "{} proposes {} change(s). Review them in the viewport.",
            p.agent, p.steps
        );
        let doc = self.session.doc();
        let target = python::target_label(&p.targets, |o| {
            doc.get(o)
                .map(|e| e.label())
                .unwrap_or_else(|| format!("#{}", o.0))
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("AI")
                    .strong()
                    .color(Color32::from_rgb(120, 60, 200)),
            );
            ui.label(text);
            ui.label(
                RichText::new(target)
                    .strong()
                    .color(Color32::from_rgb(200, 120, 0)),
            );
            if ui.button("Accept").clicked() {
                self.accept_proposal();
            }
            if ui.button("Reject").clicked() {
                self.reject_proposal();
            }
        });
    }

    fn send_chat(&mut self, ctx: &egui::Context) {
        let text = self.chat.input.trim().to_string();
        if text.is_empty() || self.chat.busy {
            return;
        }
        self.chat.input.clear();
        self.chat.lines.push(ChatLine::User(text.clone()));
        self.chat.busy = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.chat.rx = Some(rx);
        let settings = self.chat.settings.clone();
        let history = self.chat.history.clone();
        let queue = self.link.sender();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let exec = |name: &str, args: &Value| {
                let req = LinkRequest {
                    token: String::new(),
                    tool: name.to_string(),
                    args: args.clone(),
                    agent: Some(AgentInfo {
                        name: "Ferro chat".into(),
                        version: settings.model.clone(),
                    }),
                    prompt: Some(text.clone()),
                };
                let r = link::ask_ui(&queue, &repaint, req);
                if r.ok {
                    Ok(r.body)
                } else {
                    Err(r.body)
                }
            };
            let mut emit = |e: ChatEvent| {
                let _ = tx.send(e);
                repaint.request_repaint();
            };
            let result = match history.lock() {
                Ok(mut h) => {
                    ai::run_turn(&settings, &mut h, &text, &ai::http_post, &exec, &mut emit)
                }
                Err(_) => Err("chat state is unavailable".into()),
            };
            if let Err(m) = result {
                emit(ChatEvent::Error(m));
                emit(ChatEvent::Done);
            }
        });
    }

    fn poll_chat(&mut self) {
        let Some(rx) = &self.chat.rx else { return };
        let events: Vec<ChatEvent> = rx.try_iter().collect();
        for e in events {
            match e {
                ChatEvent::Assistant(t) => self.chat.lines.push(ChatLine::Assistant(t)),
                ChatEvent::Tool { name, ok, summary } => {
                    self.chat.lines.push(ChatLine::Tool { name, ok, summary })
                }
                ChatEvent::Error(m) => self.chat.lines.push(ChatLine::Error(m)),
                ChatEvent::Done => {
                    self.chat.busy = false;
                    self.chat.rx = None;
                }
            }
        }
    }

    fn chat_window(&mut self, ctx: &egui::Context) {
        let mut open = self.chat.open;
        let mut send = false;
        let mut settings = false;
        egui::Window::new("AI chat")
            .open(&mut open)
            .default_pos([160.0, 80.0])
            .default_size([400.0, 540.0])
            .min_height(320.0)
            .show(ctx, |ui| {
                if !self.chat.settings.is_ready() {
                    ui.label("Connect a model to chat: your own Anthropic key, or any OpenAI-compatible endpoint such as a local Ollama.");
                    if ui.button("Chat settings…").clicked() {
                        settings = true;
                    }
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.chat.settings.model).weak());
                    if ui.small_button("Settings").clicked() {
                        settings = true;
                    }
                    if ui
                        .add_enabled(!self.chat.busy, egui::Button::new("Clear").small())
                        .clicked()
                    {
                        self.chat.lines.clear();
                        if let Ok(mut h) = self.chat.history.lock() {
                            h.clear();
                        }
                    }
                });
                ui.separator();
                let input_h = 64.0;
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - input_h).max(80.0))
                    .show(ui, |ui| {
                        for l in &self.chat.lines {
                            match l {
                                ChatLine::User(t) => {
                                    ui.label(RichText::new(t).strong());
                                }
                                ChatLine::Assistant(t) => {
                                    ui.label(t);
                                }
                                ChatLine::Tool { name, ok, summary } => {
                                    let c = if *ok {
                                        Color32::from_rgb(40, 140, 70)
                                    } else {
                                        Color32::from_rgb(200, 40, 40)
                                    };
                                    ui.label(RichText::new(format!("tool {name} {summary}")).small().color(c));
                                }
                                ChatLine::Error(m) => {
                                    ui.label(RichText::new(m).color(Color32::from_rgb(200, 40, 40)));
                                }
                            }
                            ui.add_space(4.0);
                        }
                        if self.chat.busy {
                            ui.spinner();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut self.chat.input)
                            .desired_rows(2)
                            .desired_width(ui.available_width() - 60.0)
                            .hint_text("e.g. Draw a 10 × 8 m house with 3 m walls, a door and two windows"),
                    );
                    let enter = r.has_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                    if ui
                        .add_enabled(!self.chat.busy, egui::Button::new("Send"))
                        .clicked()
                        || enter
                    {
                        send = true;
                    }
                });
            });
        self.chat.open = open;
        if settings {
            self.chat.show_settings = true;
        }
        if send {
            // Enter leaves a newline in the multiline box; drop it.
            self.chat.input = self.chat.input.trim_end_matches('\n').to_string();
            self.send_chat(ctx);
        }
        let mut show = self.chat.show_settings;
        egui::Window::new("Chat settings")
            .open(&mut show)
            .resizable(false)
            .show(ctx, |ui| {
                let s = &mut self.chat.settings;
                egui::Grid::new("ai_settings").num_columns(2).show(ui, |ui| {
                    ui.label("Provider");
                    let before = s.provider;
                    egui::ComboBox::from_id_salt("provider")
                        .selected_text(match s.provider {
                            Provider::Anthropic => "Anthropic",
                            Provider::OpenaiCompatible => "OpenAI-compatible",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut s.provider, Provider::Anthropic, "Anthropic");
                            ui.selectable_value(&mut s.provider, Provider::OpenaiCompatible, "OpenAI-compatible");
                        });
                    if s.provider != before {
                        *s = match s.provider {
                            Provider::Anthropic => AiSettings::default(),
                            Provider::OpenaiCompatible => AiSettings {
                                provider: Provider::OpenaiCompatible,
                                model: "llama3.1".into(),
                                api_key: String::new(),
                                base_url: "http://localhost:11434/v1".into(),
                            },
                        };
                    }
                    ui.end_row();
                    ui.label("Model");
                    ui.text_edit_singleline(&mut s.model);
                    ui.end_row();
                    ui.label("Endpoint");
                    ui.text_edit_singleline(&mut s.base_url);
                    ui.end_row();
                    ui.label("API key");
                    ui.add(egui::TextEdit::singleline(&mut s.api_key).password(true));
                    ui.end_row();
                });
                ui.label(
                    RichText::new("The key is stored in plain text in ~/.ferro/settings.json on this computer.")
                        .small()
                        .italics(),
                );
                if ui.button("Save").clicked() {
                    self.chat.settings_note = match self.chat.settings.save() {
                        Ok(()) => "Saved.".into(),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
                if !self.chat.settings_note.is_empty() {
                    ui.label(&self.chat.settings_note);
                }
            });
        self.chat.show_settings = show;
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.project_settings_window(ctx);
        egui::Window::new("About Ferro").open(&mut self.show_about).resizable(false).show(ctx, |ui| {
            ui.heading(format!("Ferro {}", env!("CARGO_PKG_VERSION")));
            ui.label("Open-source, AI-native 3D design tool for architects.");
            ui.label("Licence: MPL-2.0 · github.com/ktsdatpham/ferro");
            ui.label("Sketch modelling, levels, push/pull, walls, floors, doors and windows with types and materials, .fro files, OBJ/glTF export, MCP agents, Ferro Link and an in-app AI chat.");
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
        let link_note = match self.link.port() {
            Some(p) => format!("Ferro Link is on (127.0.0.1:{p})."),
            None => "Ferro Link could not start; agents will work on their own copy.".into(),
        };
        egui::Window::new("Connect an AI agent").open(&mut self.show_agents).resizable(true).show(ctx, |ui| {
            ui.label(RichText::new(&link_note).strong());
            ui.label("Ferro speaks MCP. Add this server to Claude Desktop, Claude Code, Cursor or any MCP client:");
            let cfg = serde_json::json!({"mcpServers": {"ferro": {"command": mcp}}});
            let mut text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(6));
            ui.label("While this window is open, the agent edits the model you are looking at (Ferro Link). Its changes appear as a proposal you accept or reject; switch to \"Apply directly\" in the AI menu to skip the review.");
            ui.label(RichText::new("Without the app open, ferro-mcp works on its own copy and saves .fro files.").italics());
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
        self.refresh_view();
        self.process_agent_calls();
        self.poll_python(&ctx);
        self.sync_view();
        self.poll_chat();
        self.shortcuts(&ctx);
        self.palette_ui(&ctx);
        self.report_ui(&ctx);
        let title = format!(
            "Ferro — {}{}",
            self.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.doc().project().name.clone()),
            if self.dirty() && self.store.is_none() {
                " •"
            } else {
                ""
            }
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        if self.proposal.is_some() {
            egui::Panel::top("proposal").show(ui, |ui| self.proposal_bar(ui));
        }
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
        self.chat_window(&ctx);
        self.script_wait_ui(&ctx);
    }
}

/// A problem report the user is reviewing.
struct ReportDraft {
    /// The user's description.
    what: String,
    /// Version, platform, session log (or the saved crash report).
    body: String,
    /// Crash report file this draft came from (dismissed when closed).
    crash_file: Option<PathBuf>,
}

impl ReportDraft {
    fn new(crash_text: String, crash_file: Option<PathBuf>) -> ReportDraft {
        ReportDraft {
            what: String::new(),
            body: if crash_text.is_empty() {
                crate::report::report_text("(see above)")
            } else {
                crash_text
            },
            crash_file,
        }
    }
}

/// The failure code at the start of a status message, if any
/// (`"ferro.model.x: …"`), else a short tag: messages may contain names.
fn first_code(text: &str) -> &str {
    let head = text.split([':', ' ']).next().unwrap_or("");
    if head.starts_with("ferro.") {
        head
    } else {
        "message"
    }
}
