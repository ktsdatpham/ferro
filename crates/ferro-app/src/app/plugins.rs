//! Plugins in the app (ADR 0009, M5-28): the loaded plugins' commands in
//! the command palette, a Plugins menu, a Plugins group in the left panel
//! and the keymap; hot reload; "New plugin..." from the template; the
//! install screen and the Plugins window (enable, disable, remove).
//!
//! Loading, keys, the template and installing live in `ferro_mcp::plugins`
//! (no window, tested there). A plugin command runs through the same host
//! as `run_python` ([`super::python`]): one undo step named after its
//! label, all or nothing, stoppable like any script, committed as the
//! plugin (the user's rights; never a Proposal). `ferro.ask` inside it
//! opens [`FerroApp::ask_ui`]; the script's time limit waits meanwhile.

use super::python::ScriptRun;
use super::FerroApp;
use crate::keymap::Target;
use eframe::egui::{self, Key, RichText, Vec2};
use ferro_mcp::plugins::{self, Plugin, PluginGroup, PluginWatch, StagedPlugin};
use ferro_mcp::python::{self, PythonRun, RunEnd};
use ferro_tools::link::{LinkRequest, LinkResponse};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Instant;

/// One loaded plugin command, by slot (`Target::Plugin(slot)`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Slot {
    pub plugin: String,
    pub command: String,
    pub label: String,
    pub plugin_name: String,
    pub group: PluginGroup,
}

/// A `ferro.ask` dialog waiting for the user.
struct Ask {
    prompt: String,
    value: String,
    reply: Sender<LinkResponse>,
    focus: bool,
}

/// Plugins state of the app.
#[derive(Default)]
pub(super) struct PluginState {
    /// The plugins folder (`$FERRO_HOME/plugins`), when a home is known.
    root: Option<PathBuf>,
    /// Every plugin folder, loaded or broken.
    list: Vec<Plugin>,
    /// Commands of the active plugins, in menu order.
    slots: Vec<Slot>,
    /// Key conflicts of the last load.
    warnings: Vec<String>,
    watch: Option<PluginWatch>,
    /// Name typed in the "New plugin" dialog, while it is open.
    new_name: Option<String>,
    /// A plugin waiting on the install screen.
    install: Option<StagedPlugin>,
    /// The Plugins window is open.
    pub show_window: bool,
    ask: Option<Ask>,
}

impl PluginState {
    /// The running script ended: a pending `ferro.ask` has no one to answer.
    pub fn script_ended(&mut self) {
        self.ask = None;
    }

    /// The command in `slot`.
    pub fn slot(&self, slot: u16) -> Option<&Slot> {
        self.slots.get(slot as usize)
    }

    /// The manifest of the installed (valid) plugin with this id, enabled
    /// or not.
    pub fn installed(&self, id: &str) -> Option<&plugins::PluginManifest> {
        self.list
            .iter()
            .find_map(|p| p.manifest.as_ref().ok().filter(|m| m.id == id))
    }

    /// Commands of the active plugins (palette entries), with their slots.
    pub fn targets(&self) -> impl Iterator<Item = (Target, &Slot)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((Target::Plugin(u16::try_from(i).ok()?), s)))
    }
}

/// The slots of the active plugins' commands, in folder then manifest order.
pub(super) fn slots_of(list: &[Plugin]) -> Vec<Slot> {
    list.iter()
        .filter_map(Plugin::active)
        .flat_map(|m| {
            m.commands.iter().map(move |c| Slot {
                plugin: m.id.clone(),
                command: c.id.clone(),
                label: c.label.clone(),
                plugin_name: m.name.clone(),
                group: c.group,
            })
        })
        .take(u16::MAX as usize)
        .collect()
}

/// Open `dir` in the operating system's file manager.
fn open_folder(dir: &Path) -> std::io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The last non-empty line of a run's output stream, shortened.
fn last_line(body: &Value, stream: &str) -> Option<String> {
    let line = body[stream]
        .as_str()?
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())?;
    let mut s: String = line.trim().chars().take(160).collect();
    if line.trim().chars().count() > 160 {
        s.push_str("...");
    }
    Some(s)
}

impl FerroApp {
    /// Scan the plugins folder, rebuild the slots and the plugin keys.
    /// Called at start, by the hot-reload watch, Plugins > Reload plugins
    /// and after install / enable / disable / remove.
    pub(super) fn reload_plugins(&mut self) {
        let root = plugins::plugins_dir();
        let now = Instant::now();
        if let Some(r) = &root {
            if self.plugins.watch.is_none() {
                plugins::clean_staging(r);
            }
            match &mut self.plugins.watch {
                Some(w) if w.root() == r.as_path() => w.loaded(now),
                w => *w = Some(PluginWatch::new(r, now)),
            }
        }
        self.plugins.list = root.as_deref().map(plugins::scan).unwrap_or_default();
        self.plugins.root = root;
        self.plugins.slots = slots_of(&self.plugins.list);
        self.apply_plugin_keys();
        let broken = self
            .plugins
            .list
            .iter()
            .filter(|p| p.enabled && p.manifest.is_err())
            .count();
        crate::report::record(format!(
            "plugins loaded {} broken {broken}",
            self.plugins.slots.len()
        ));
        if broken > 0 {
            self.say(
                format!("{broken} plugin(s) could not load: see Plugins > Manage plugins"),
                true,
            );
        } else if let Some(w) = self.plugins.warnings.first().cloned() {
            self.say(w, true);
        }
    }

    /// Bind the plugin commands' keys in the current keymap (after a reload,
    /// or when the keymap was replaced). Built-in keys always win.
    pub(super) fn apply_plugin_keys(&mut self) {
        self.keymap.clear_plugins();
        let keymap = &self.keymap;
        let (keys, warnings) = plugins::assign_keys(&self.plugins.list, |k| {
            keymap.get(k).map(|t| t.label().to_string())
        });
        for b in keys {
            let slot = self
                .plugins
                .slots
                .iter()
                .position(|s| s.plugin == b.plugin && s.command == b.command)
                .and_then(|i| u16::try_from(i).ok());
            if let Some(slot) = slot {
                self.keymap.bind_plugin(&b.key, slot);
            }
        }
        self.plugins.warnings = warnings;
    }

    /// Once per frame: reload when the plugins folder changed (polled once
    /// a second, so edits show without restarting).
    pub(super) fn poll_plugins(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.plugins.watch else {
            return;
        };
        if w.poll(Instant::now()) {
            self.reload_plugins();
            self.say("Plugins reloaded", false);
        }
        ctx.request_repaint_after(plugins::POLL_INTERVAL);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            let is_plugin = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
                || p.join(plugins::MANIFEST_FILE).is_file();
            if is_plugin {
                self.stage_plugin(&p);
            }
        }
    }

    /// Run the plugin command in `slot` (button, menu, palette or key).
    pub(super) fn run_plugin_slot(&mut self, slot: u16) {
        let Some(s) = self.plugins.slot(slot).cloned() else {
            return;
        };
        self.run_plugin_command(&s.plugin, &s.command);
    }

    /// Start a plugin command as a script run. Refused while another script
    /// runs or an AI proposal is open (its changes would join it).
    pub(super) fn run_plugin_command(&mut self, plugin_id: &str, command_id: &str) {
        if self.python.is_some() {
            self.say(
                "A script is running: run the plugin when it has finished",
                true,
            );
            return;
        }
        if self.proposal.is_some() {
            self.say(
                "Accept or reject the AI proposal before running a plugin",
                true,
            );
            return;
        }
        let Some((plugin, manifest, command)) =
            plugins::find(&self.plugins.list, plugin_id, command_id)
        else {
            self.say(
                format!("Plugin command {plugin_id}.{command_id} is not loaded"),
                true,
            );
            return;
        };
        // A paid plugin without its entitlement stays loaded, but its
        // commands do not run (ADR 0010): say so and open its listing.
        let entitlements = &self.account.entitlements;
        if let Err(body) = plugins::check_entitlement(manifest, |e| entitlements.has(e)) {
            let failure = &body["failures"][0];
            let message = failure["message"]
                .as_str()
                .unwrap_or("not licensed")
                .to_string();
            let id = failure["plugin"].as_str().unwrap_or(plugin_id).to_string();
            crate::report::record(format!(
                "plugin command refused {}",
                plugins::LICENSE_MISSING
            ));
            self.say(format!("{}: {message}", plugins::LICENSE_MISSING), true);
            self.open_listing(&id);
            return;
        }
        let Some(port) = self.link.port() else {
            self.say("Ferro Link is not listening, so a plugin cannot run", true);
            return;
        };
        let mut args = plugins::run_args(&plugin.dir, command);
        // One rule with scripts (ADR 0006): into the option being edited.
        args.in_design_option = Some(self.editing_option());
        let ctx = plugins::plugin_ctx(manifest, super::now_ms());
        let (label, name) = (command.label.clone(), manifest.name.clone());
        match PythonRun::start(
            &mut self.session,
            &self.python_config,
            &args,
            ctx,
            port,
            &label,
        ) {
            Ok(run) => {
                self.link.set_run_token(Some(run.token().to_string()));
                crate::report::record("plugin command started");
                self.say(format!("{label}..."), false);
                self.python = Some(ScriptRun {
                    run,
                    reply: None,
                    agent: name,
                    plugin: Some(label),
                });
            }
            Err(body) => {
                let m = body["error"]
                    .as_str()
                    .unwrap_or("cannot start Python")
                    .to_string();
                self.say(format!("{label}: {m}"), true);
            }
        }
    }

    /// A plugin command's run ended (already committed or rolled back).
    pub(super) fn plugin_command_ended(
        &mut self,
        label: &str,
        plugin: &str,
        end: RunEnd,
        r: &Result<Value, Value>,
    ) {
        match r {
            Ok(body) => {
                crate::report::record("plugin command ok");
                let text = match last_line(body, "stdout") {
                    Some(l) => format!("{label}: {l}"),
                    None => format!("{label}: done"),
                };
                self.say(text, false);
                self.autosave();
            }
            Err(body) => {
                let code = body["failures"][0]["code"].as_str().unwrap_or("");
                crate::report::record(format!("plugin command failed {code}"));
                self.refresh_view();
                self.selection.retain(|id| self.session.doc().contains(*id));
                let text = if end == RunEnd::Cancelled {
                    format!("Stopped {label}; nothing was changed")
                } else {
                    let why = last_line(body, "stderr")
                        .or_else(|| body["error"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    format!("{label} ({plugin}) failed, nothing was changed: {why}")
                };
                self.say(text, end != RunEnd::Cancelled);
            }
        }
    }

    /// `get_user_input` from the running script (`ferro.ask`): show the
    /// dialog; the answer is sent when the user closes it.
    pub(super) fn ask_from_script(&mut self, req: LinkRequest, reply: Sender<LinkResponse>) {
        if self.plugins.ask.is_some() {
            let _ = reply.send(LinkResponse {
                ok: false,
                body: json!({"error": "the script is already asking the user"}),
            });
            return;
        }
        let prompt = req.args["prompt"].as_str().unwrap_or("Value").to_string();
        let value = python::user_input_default(&req.args)
            .as_str()
            .unwrap_or("")
            .to_string();
        self.plugins.ask = Some(Ask {
            prompt,
            value,
            reply,
            focus: true,
        });
        if let Some(s) = &mut self.python {
            s.run.pause_time_limit();
        }
    }

    /// The `ferro.ask` dialog: OK (Enter) sends the text, Cancel (Esc) sends
    /// `null`. The time spent here is given back to the script's limit.
    pub(super) fn ask_ui(&mut self, ctx: &egui::Context) {
        let Some(ask) = &mut self.plugins.ask else {
            return;
        };
        let title = self
            .python
            .as_ref()
            .map(|s| s.plugin.clone().unwrap_or_else(|| s.agent.clone()))
            .unwrap_or_else(|| "Script".into());
        let mut answer: Option<Option<String>> = None;
        egui::Modal::new(egui::Id::unique("plugin_ask")).show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.label(RichText::new(&title).weak());
            ui.label(RichText::new(&ask.prompt).strong());
            let r = ui.add(egui::TextEdit::singleline(&mut ask.value).desired_width(280.0));
            if ask.focus {
                r.request_focus();
                ask.focus = false;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    answer = Some(Some(ask.value.clone()));
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(None);
                }
            });
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                answer = Some(Some(ask.value.clone()));
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                answer = Some(None);
            }
        });
        let Some(value) = answer else { return };
        let Some(ask) = self.plugins.ask.take() else {
            return;
        };
        if let Some(s) = &mut self.python {
            s.run.resume_time_limit();
        }
        let _ = ask.reply.send(LinkResponse {
            ok: true,
            body: json!({ "value": value }),
        });
    }

    /// The Plugins menu: each plugin's commands, broken plugins greyed out,
    /// then New / Install / Reload / Manage.
    pub(super) fn plugins_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Plugins", |ui| {
            let mut run = None;
            for p in &self.plugins.list {
                match (&p.manifest, p.enabled) {
                    (Ok(m), true) => {
                        let locked = self.plugin_locked(&m.id);
                        ui.menu_button(&m.name, |ui| {
                            for c in &m.commands {
                                let slot = self
                                    .plugins
                                    .slots
                                    .iter()
                                    .position(|s| s.plugin == m.id && s.command == c.id);
                                let key = slot
                                    .and_then(|i| u16::try_from(i).ok())
                                    .map(|i| self.keymap.label_for(Target::Plugin(i)))
                                    .unwrap_or_default();
                                let label = if locked {
                                    format!("{} (Buy)", c.label)
                                } else {
                                    c.label.clone()
                                };
                                let b =
                                    egui::Button::new(label).right_text(RichText::new(key).weak());
                                if ui.add(b).clicked() {
                                    run = Some((m.id.clone(), c.id.clone()));
                                    ui.close();
                                }
                            }
                        });
                    }
                    (Err(reason), true) => {
                        ui.add_enabled(false, egui::Button::new(format!("{} (broken)", p.name())))
                            .on_disabled_hover_text(reason);
                    }
                    (_, false) => {}
                }
            }
            if !self.plugins.list.is_empty() {
                ui.separator();
            }
            if ui.button("New plugin…").clicked() {
                self.plugins.new_name = Some(String::new());
                ui.close();
            }
            if ui.button("Marketplace…").clicked() {
                self.market.show_window = true;
                ui.close();
            }
            if ui.button("Install from file…").clicked() {
                ui.close();
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Ferro plugin", &["zip"])
                    .pick_file()
                {
                    self.stage_plugin(&p);
                }
            }
            if ui.button("Install from folder…").clicked() {
                ui.close();
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.stage_plugin(&p);
                }
            }
            if ui.button("Reload plugins").clicked() {
                self.reload_plugins();
                self.say("Plugins reloaded", false);
                ui.close();
            }
            if ui.button("Manage plugins…").clicked() {
                self.plugins.show_window = true;
                ui.close();
            }
            if let Some((p, c)) = run {
                self.run_plugin_command(&p, &c);
            }
        });
    }

    /// The Plugins group in the left panel: a button per command with group
    /// `panel`.
    pub(super) fn plugins_toolbar(&mut self, ui: &mut egui::Ui) {
        let buttons: Vec<(u16, String)> = self
            .plugins
            .targets()
            .filter(|(_, s)| s.group == PluginGroup::Panel)
            .filter_map(|(t, s)| match t {
                Target::Plugin(i) if self.plugin_locked(&s.plugin) => {
                    Some((i, format!("{} (Buy)", s.label)))
                }
                Target::Plugin(i) => Some((i, s.label.clone())),
                _ => None,
            })
            .collect();
        if buttons.is_empty() {
            return;
        }
        ui.separator();
        ui.label(RichText::new("Plugins").strong());
        let mut run = None;
        for (slot, label) in buttons {
            let key = self.keymap.label_for(Target::Plugin(slot));
            let tip = self
                .plugins
                .slot(slot)
                .map(|s| format!("{} ({})", s.label, s.plugin_name))
                .unwrap_or_default();
            let b = egui::Button::new(RichText::new(&label).size(14.0))
                .right_text(RichText::new(&key).weak().size(11.0))
                .min_size(Vec2::new(124.0, 26.0));
            if ui.add(b).on_hover_text(tip).clicked() {
                run = Some(slot);
            }
        }
        if let Some(slot) = run {
            self.run_plugin_slot(slot);
        }
    }

    /// Unpack or copy a plugin for the install screen.
    pub(super) fn stage_plugin(&mut self, source: &Path) {
        let Some(root) = self.plugins.root.clone().or_else(plugins::plugins_dir) else {
            self.say("No home folder: cannot install plugins", true);
            return;
        };
        if let Some(old) = self.plugins.install.take() {
            plugins::cancel_install(old);
        }
        match plugins::stage_install(source, &root) {
            Ok(s) => self.plugins.install = Some(s),
            Err(m) => self.say(format!("Cannot install: {m}"), true),
        }
    }

    /// Windows of this module: the ask dialog, New plugin, the install
    /// screen and the Plugins window.
    pub(super) fn plugins_windows(&mut self, ctx: &egui::Context) {
        self.ask_ui(ctx);
        self.new_plugin_ui(ctx);
        self.install_ui(ctx);
        self.plugins_window(ctx);
    }

    fn new_plugin_ui(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.plugins.new_name.take() else {
            return;
        };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New plugin")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Name of the plugin:");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("e.g. My tools")
                        .desired_width(260.0),
                );
                r.request_focus();
                ui.label(
                    RichText::new("Creates a folder with plugin.json, main.py (a working \"Box at origin\" command) and a README, then opens it.")
                        .weak(),
                );
                ui.horizontal(|ui| {
                    create = ui.button("Create").clicked() || ui.input(|i| i.key_pressed(Key::Enter));
                });
            });
        if !create {
            if open {
                self.plugins.new_name = Some(name);
            }
            return;
        }
        let Some(root) = plugins::plugins_dir() else {
            self.say("No home folder: cannot create plugins", true);
            return;
        };
        match plugins::create_plugin(&root, &name, "") {
            Ok(dir) => {
                self.reload_plugins();
                let opened = open_folder(&dir).is_ok();
                self.say(
                    if opened {
                        format!(
                            "Created {}: edit main.py, save, then click \"Box at origin\"",
                            dir.display()
                        )
                    } else {
                        format!("Created {} (open it in your file manager)", dir.display())
                    },
                    false,
                );
            }
            Err(m) => self.say(format!("Cannot create the plugin: {m}"), true),
        }
    }

    fn install_ui(&mut self, ctx: &egui::Context) {
        let Some(staged) = &self.plugins.install else {
            return;
        };
        let m = &staged.manifest;
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("plugin_install")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(format!("Install {}?", m.name));
            ui.label(format!("Version {}, by {}", m.version, m.author));
            if !m.description.is_empty() {
                ui.label(&m.description);
            }
            if let Some(v) = &staged.replaces {
                ui.label(RichText::new(format!("Replaces the installed version {v}.")).italics());
            }
            ui.add_space(4.0);
            ui.label(RichText::new("It adds:").strong());
            for c in &m.commands {
                let mut line = format!("  {}", c.label);
                if let Some(k) = &c.key {
                    line.push_str(&format!("  (key {k})"));
                }
                ui.label(line);
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("This plugin runs with your rights, like any program you install. Install it only if you trust its author.")
                    .strong(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Install").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        let Some(install) = choice else { return };
        let Some(staged) = self.plugins.install.take() else {
            return;
        };
        let name = staged.manifest.name.clone();
        if !install {
            plugins::cancel_install(staged);
            return;
        }
        let Some(root) = self.plugins.root.clone().or_else(plugins::plugins_dir) else {
            plugins::cancel_install(staged);
            return;
        };
        match plugins::finish_install(staged, &root) {
            Ok(_) => {
                self.reload_plugins();
                crate::report::record("plugin installed");
                self.say(format!("Installed {name}: see the Plugins menu"), false);
            }
            Err(m) => self.say(format!("Cannot install {name}: {m}"), true),
        }
    }

    fn plugins_window(&mut self, ctx: &egui::Context) {
        if !self.plugins.show_window {
            return;
        }
        let mut open = true;
        let mut toggle: Option<(PathBuf, bool)> = None;
        let mut remove: Option<(PathBuf, String)> = None;
        let mut reveal: Option<PathBuf> = None;
        egui::Window::new("Plugins")
            .open(&mut open)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                match &self.plugins.root {
                    Some(r) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(r.display().to_string()).weak());
                            if ui.small_button("Open folder").clicked() {
                                reveal = Some(r.clone());
                            }
                        });
                    }
                    None => {
                        ui.label("No home folder: plugins are off.");
                    }
                }
                if self.plugins.list.is_empty() {
                    ui.label("No plugins yet. Use Plugins > New plugin… or Install from file….");
                }
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for p in &self.plugins.list {
                            ui.separator();
                            ui.horizontal(|ui| {
                                let mut on = p.enabled;
                                if ui
                                    .checkbox(&mut on, RichText::new(p.name()).strong())
                                    .changed()
                                {
                                    toggle = Some((p.dir.clone(), on));
                                }
                                if let Ok(m) = &p.manifest {
                                    ui.label(format!("{} by {}", m.version, m.author));
                                }
                            });
                            match &p.manifest {
                                Ok(m) => {
                                    if !m.description.is_empty() {
                                        ui.label(RichText::new(&m.description).weak());
                                    }
                                    let labels: Vec<&str> =
                                        m.commands.iter().map(|c| c.label.as_str()).collect();
                                    ui.label(format!("Adds: {}", labels.join(", ")));
                                }
                                Err(reason) => {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(200, 60, 40),
                                        format!("Broken: {reason}"),
                                    );
                                }
                            }
                            if !p.enabled {
                                ui.label(RichText::new("Disabled").italics());
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("Open folder").clicked() {
                                    reveal = Some(p.dir.clone());
                                }
                                if ui.small_button("Remove").clicked() {
                                    remove = Some((p.dir.clone(), p.name().to_string()));
                                }
                            });
                        }
                    });
                if !self.plugins.warnings.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("Keys").strong());
                    for w in &self.plugins.warnings {
                        ui.label(RichText::new(w).weak());
                    }
                }
            });
        self.plugins.show_window = open;
        if let Some(dir) = reveal {
            if open_folder(&dir).is_err() {
                self.say(format!("Cannot open {}", dir.display()), true);
            }
        }
        if let Some((dir, on)) = toggle {
            match plugins::set_enabled(&dir, on) {
                Ok(()) => self.reload_plugins(),
                Err(m) => self.say(m, true),
            }
        }
        if let Some((dir, name)) = remove {
            let yes = rfd::MessageDialog::new()
                .set_title("Ferro")
                .set_description(format!("Remove the plugin {name}? Its folder is deleted."))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes;
            if yes {
                let r = match &self.plugins.root {
                    Some(root) => plugins::remove_plugin(root, &dir),
                    None => Err("no plugins folder".into()),
                };
                match r {
                    Ok(()) => {
                        self.reload_plugins();
                        self.say(format!("Removed {name}"), false);
                    }
                    Err(m) => self.say(m, true),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KeyScheme, Keymap};

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ferro-app-plugins-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn plugin_slots_follow_folders_and_skip_disabled_and_broken() {
        let root = temp();
        let a = plugins::create_plugin(&root, "A tools", "me").unwrap();
        let b = plugins::create_plugin(&root, "B tools", "me").unwrap();
        std::fs::create_dir_all(root.join("c-broken")).unwrap();
        let list = plugins::scan(&root);
        let slots = slots_of(&list);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].plugin, "a-tools");
        assert_eq!(slots[1].plugin_name, "B tools");
        assert_eq!(slots[0].label, "Box at origin");
        plugins::set_enabled(&a, false).unwrap();
        let slots = slots_of(&plugins::scan(&root));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].plugin, "b-tools");
        let _ = (b, std::fs::remove_dir_all(root));
    }

    #[test]
    fn plugin_keys_bind_after_built_ins() {
        // Both template plugins ask for BX: the first gets it; WA stays Wall.
        let root = temp();
        plugins::create_plugin(&root, "A", "me").unwrap();
        plugins::create_plugin(&root, "B", "me").unwrap();
        let list = plugins::scan(&root);
        let mut keymap = Keymap::scheme(KeyScheme::Ferro);
        let (keys, warnings) =
            plugins::assign_keys(&list, |k| keymap.get(k).map(|t| t.label().to_string()));
        assert_eq!(keys.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(keymap.bind_plugin(&keys[0].key, 0));
        assert_eq!(keymap.get("BX"), Some(Target::Plugin(0)));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_output_shows_its_last_line() {
        let body = json!({"stdout": "a\nlast one\n\n", "stderr": ""});
        assert_eq!(last_line(&body, "stdout").as_deref(), Some("last one"));
        assert_eq!(last_line(&body, "stderr"), None);
        let long = json!({"stdout": "x".repeat(300)});
        assert_eq!(last_line(&long, "stdout").unwrap().len(), 163);
    }
}
