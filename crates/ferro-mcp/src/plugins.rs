//! Plugins (ADR 0009, D-80): a folder with a small manifest and Python code
//! on the Ferro SDK, run with the user's rights like a Revit add-in or a
//! SketchUp extension.
//!
//! ```text
//! ~/.ferro/plugins/            ($FERRO_HOME/plugins when set)
//!   box-tools/
//!     plugin.json              the PluginManifest
//!     main.py                  def box_at_origin(doc): ...
//! ```
//!
//! `plugin.json`:
//!
//! ```json
//! {
//!   "id": "box-tools",
//!   "name": "Box tools",
//!   "version": "0.1.0",
//!   "author": "Your name",
//!   "description": "What the plugin does.",
//!   "commands": [
//!     {"id": "box_at_origin", "label": "Box at origin", "key": "BX",
//!      "group": "panel", "run": "main:box_at_origin"}
//!   ]
//! }
//! ```
//!
//! A command's `run` is `module:function` (the function is called with
//! `doc` when it takes a parameter) or a `.py` file in the plugin folder
//! (run like a `run_python` script, with `doc`, `tools` and `ferro`
//! predefined). Each command runs exactly like `run_python`, through the
//! same host ([`crate::python`]): one undo step named after the command's
//! label, all or nothing, cancellable, committed as the plugin
//! ([`ActorKind::Plugin`], the user's rights, never a Proposal).
//!
//! This module is the part without a window: manifests, the loader
//! ([`scan`]), key assignment ([`assign_keys`]), the hot-reload watch
//! ([`PluginWatch`]), the template ([`create_plugin`]) and installing from
//! a folder or a `.zip` ([`stage_install`]). The app draws the menus, the
//! palette entries, the left panel group and the install screen.

use crate::python::{self, PythonConfig, RunArgs, RunControl};
use ferro_core::ops::{Actor, ActorKind, ExecContext};
use ferro_core::Session;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

mod unzip;

pub use unzip::{extract as extract_zip, ZipLimits};

/// File name of a plugin's manifest.
pub const MANIFEST_FILE: &str = "plugin.json";
/// Marker file: a plugin folder holding it is disabled (listed, not loaded).
pub const DISABLED_FILE: &str = ".disabled";
/// Largest manifest read (bytes).
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// Longest plugin or command id.
pub const MAX_ID_LEN: usize = 64;
/// How often [`PluginWatch`] looks at the plugins folder.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Files looked at by one [`fingerprint`] at most (the rest is ignored).
pub const MAX_WATCHED_FILES: usize = 5000;
/// Largest plugin copied or unpacked at install (bytes, all files).
pub const MAX_INSTALL_BYTES: u64 = 200 * 1024 * 1024;
/// Most files in a plugin copied or unpacked at install.
pub const MAX_INSTALL_FILES: usize = 10_000;

/// The plugins folder: `$FERRO_HOME/plugins`, else `~/.ferro/plugins`.
/// `None` when no home folder is known.
pub fn plugins_dir() -> Option<PathBuf> {
    ferro_tools::link::ferro_dir().map(|d| d.join("plugins"))
}

/// Where a plugin command shows besides the command palette and the
/// Plugins menu (which list every command).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginGroup {
    /// Also a button in the Plugins group of the left panel (the default).
    Panel,
    /// Only the Plugins menu and the command palette.
    Menu,
}

impl PluginGroup {
    /// The manifest spelling (`"panel"`, `"menu"`).
    pub fn as_str(self) -> &'static str {
        match self {
            PluginGroup::Panel => "panel",
            PluginGroup::Menu => "menu",
        }
    }
}

/// What a plugin command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEntry {
    /// `module:function`: import `module` from the plugin folder and call
    /// `function(doc)` (or `function()` when it takes no parameter); what it
    /// returns becomes the run's `result`.
    Function {
        /// Dotted module path, e.g. `main` or `tools.walls`.
        module: String,
        /// Function name in that module.
        function: String,
    },
    /// A `.py` file in the plugin folder (relative path), run as a script
    /// with `doc`, `tools` and `ferro` predefined; its `result` variable is
    /// returned.
    Script(PathBuf),
}

impl PluginEntry {
    /// The manifest spelling (`main:box_at_origin`, `scripts/box.py`).
    pub fn spelling(&self) -> String {
        match self {
            PluginEntry::Function { module, function } => format!("{module}:{function}"),
            PluginEntry::Script(p) => p.to_string_lossy().replace('\\', "/"),
        }
    }
}

/// One entry a plugin adds to the app: the command palette, the Plugins
/// menu, the left panel (group `panel`) and, with a key, the keymap. Not a
/// `Command`: running it runs Python, whose tool calls are Commands.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommand {
    /// Id, unique in the plugin (`[a-z0-9_-]`, starts with a letter or digit).
    pub id: String,
    /// Text on the button, the menu item and in the palette; also the name
    /// of the undo step.
    pub label: String,
    /// Two-letter code typed in sequence (Revit style), upper case.
    pub key: Option<String>,
    /// Where it shows besides the palette and the menu.
    pub group: PluginGroup,
    /// The Python to run.
    pub entry: PluginEntry,
    /// Time limit in seconds (default 30, at most 300); time spent waiting
    /// for the user in `ferro.ask` does not count.
    pub timeout_s: Option<f64>,
}

/// A plugin's `plugin.json`, validated.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginManifest {
    /// Id (`[a-z0-9_-]`, starts with a letter or digit), unique among the
    /// installed plugins; the install folder's name.
    pub id: String,
    /// Name shown to the user.
    pub name: String,
    /// Version text (free form, e.g. `0.1.0`).
    pub version: String,
    /// Who made it (shown on the install screen).
    pub author: String,
    /// What it does (optional).
    pub description: String,
    /// What it adds; at least one.
    pub commands: Vec<PluginCommand>,
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// A relative path inside the plugin folder: no root, no drive, no `..`.
fn inside(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || rel.contains('\\') || rel.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Parse a command's `run` field. `dir` (the plugin folder) is used to check
/// that the module or script exists; `None` skips that check.
pub fn parse_entry(run: &str, dir: Option<&Path>) -> Result<PluginEntry, String> {
    let run = run.trim();
    if run.ends_with(".py") && !run.contains(':') {
        let Some(rel) = inside(run) else {
            return Err(format!(
                "run `{run}`: the script must be a relative path inside the plugin folder"
            ));
        };
        if let Some(d) = dir {
            if !d.join(&rel).is_file() {
                return Err(format!("run `{run}`: no such file in the plugin folder"));
            }
        }
        return Ok(PluginEntry::Script(rel));
    }
    let Some((module, function)) = run.split_once(':') else {
        return Err(format!(
            "run `{run}`: write `module:function` (e.g. `main:box_at_origin`) or a .py file"
        ));
    };
    let (module, function) = (module.trim(), function.trim());
    if module.is_empty() || !module.split('.').all(valid_ident) {
        return Err(format!(
            "run `{run}`: `{module}` is not a Python module name"
        ));
    }
    if !valid_ident(function) {
        return Err(format!(
            "run `{run}`: `{function}` is not a Python function name"
        ));
    }
    if let Some(d) = dir {
        let base: PathBuf = module.split('.').collect();
        let file = d.join(&base).with_extension("py");
        let package = d.join(&base).join("__init__.py");
        if !file.is_file() && !package.is_file() {
            return Err(format!(
                "run `{run}`: no {}.py in the plugin folder",
                base.to_string_lossy().replace('\\', "/")
            ));
        }
    }
    Ok(PluginEntry::Function {
        module: module.to_string(),
        function: function.to_string(),
    })
}

fn text_field(
    obj: &serde_json::Map<String, Value>,
    key: &str,
    required: bool,
    errors: &mut Vec<String>,
    at: &str,
) -> String {
    match obj.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None | Some(Value::Null) => {
            if required {
                errors.push(format!("{at}missing `{key}`"));
            }
            String::new()
        }
        Some(_) => {
            errors.push(format!("{at}`{key}` must be text"));
            String::new()
        }
    }
}

impl PluginManifest {
    /// Parse and validate `plugin.json` text. `dir` (the plugin folder)
    /// checks that entry points exist; `None` skips that. Fails with every
    /// problem found, joined by `"; "`: not JSON, missing or empty `id`,
    /// `name`, `version`, `author` or `commands`, a bad id, a duplicate
    /// command id, a bad `key` (two letters A-Z), a bad `group` (`panel` /
    /// `menu`), a bad `timeout_s` (above 0, at most 300) or a bad `run`.
    pub fn parse(text: &str, dir: Option<&Path>) -> Result<PluginManifest, String> {
        let v: Value =
            serde_json::from_str(text).map_err(|e| format!("{MANIFEST_FILE} is not JSON: {e}"))?;
        let Some(obj) = v.as_object() else {
            return Err(format!("{MANIFEST_FILE} must be a JSON object {{...}}"));
        };
        let mut errors = Vec::new();
        let id = text_field(obj, "id", true, &mut errors, "");
        if !id.is_empty() && !valid_id(&id) {
            errors.push(format!(
                "id `{id}`: use lower-case letters, digits, `-` and `_` (at most {MAX_ID_LEN})"
            ));
        }
        let name = text_field(obj, "name", true, &mut errors, "");
        let version = text_field(obj, "version", true, &mut errors, "");
        let author = text_field(obj, "author", true, &mut errors, "");
        let description = text_field(obj, "description", false, &mut errors, "");
        let mut commands = Vec::new();
        match obj.get("commands") {
            Some(Value::Array(list)) if !list.is_empty() => {
                let mut seen = HashSet::new();
                for (i, c) in list.iter().enumerate() {
                    let at = format!("commands[{i}]: ");
                    let Some(c) = c.as_object() else {
                        errors.push(format!("{at}must be an object"));
                        continue;
                    };
                    let before = errors.len();
                    let cid = text_field(c, "id", true, &mut errors, &at);
                    if !cid.is_empty() && !valid_id(&cid) {
                        errors.push(format!(
                            "{at}id `{cid}`: use lower-case letters, digits, `-` and `_`"
                        ));
                    }
                    if !cid.is_empty() && !seen.insert(cid.clone()) {
                        errors.push(format!("{at}duplicate command id `{cid}`"));
                    }
                    let label = text_field(c, "label", true, &mut errors, &at);
                    let key = match c.get("key") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(k)) => {
                            let k = k.trim().to_ascii_uppercase();
                            if k.len() == 2 && k.chars().all(|c| c.is_ascii_uppercase()) {
                                Some(k)
                            } else {
                                errors.push(format!(
                                    "{at}key `{k}`: use a two-letter code such as \"BX\""
                                ));
                                None
                            }
                        }
                        Some(_) => {
                            errors.push(format!("{at}`key` must be text"));
                            None
                        }
                    };
                    let group = match c.get("group").and_then(Value::as_str) {
                        None if c.get("group").is_none_or(Value::is_null) => PluginGroup::Panel,
                        Some("panel") => PluginGroup::Panel,
                        Some("menu") => PluginGroup::Menu,
                        _ => {
                            errors.push(format!("{at}group must be \"panel\" or \"menu\""));
                            PluginGroup::Panel
                        }
                    };
                    let timeout_s = match c.get("timeout_s") {
                        None | Some(Value::Null) => None,
                        Some(t) => match t.as_f64() {
                            Some(s) if s > 0.0 && s <= python::MAX_TIMEOUT_S => Some(s),
                            _ => {
                                errors.push(format!(
                                    "{at}timeout_s must be seconds above 0 and at most {}",
                                    python::MAX_TIMEOUT_S
                                ));
                                None
                            }
                        },
                    };
                    let run = text_field(c, "run", true, &mut errors, &at);
                    let entry = if run.is_empty() {
                        None
                    } else {
                        match parse_entry(&run, dir) {
                            Ok(e) => Some(e),
                            Err(m) => {
                                errors.push(format!("{at}{m}"));
                                None
                            }
                        }
                    };
                    if errors.len() == before {
                        if let Some(entry) = entry {
                            commands.push(PluginCommand {
                                id: cid,
                                label,
                                key,
                                group,
                                entry,
                                timeout_s,
                            });
                        }
                    }
                }
            }
            Some(Value::Array(_)) => errors.push("`commands` is empty: add one command".into()),
            None | Some(Value::Null) => errors.push("missing `commands`".into()),
            Some(_) => errors.push("`commands` must be a list [...]".into()),
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(PluginManifest {
            id,
            name,
            version,
            author,
            description,
            commands,
        })
    }

    /// Read and validate `dir/plugin.json` ([`Self::parse`] with entry
    /// checks). Fails when the file is missing, unreadable or too large.
    pub fn read(dir: &Path) -> Result<PluginManifest, String> {
        use std::io::Read;
        let path = dir.join(MANIFEST_FILE);
        let file = std::fs::File::open(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("no {MANIFEST_FILE} in the folder"),
            _ => format!("cannot read {MANIFEST_FILE}: {e}"),
        })?;
        let mut text = String::new();
        file.take(MAX_MANIFEST_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("cannot read {MANIFEST_FILE}: {e}"))?;
        if text.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(format!(
                "{MANIFEST_FILE} is larger than {MAX_MANIFEST_BYTES} bytes"
            ));
        }
        PluginManifest::parse(&text, Some(dir))
    }

    /// The command with this id.
    pub fn command(&self, id: &str) -> Option<&PluginCommand> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// The manifest as `plugin.json` text (pretty JSON).
    pub fn to_json(&self) -> String {
        let commands: Vec<Value> = self
            .commands
            .iter()
            .map(|c| {
                let mut o = json!({
                    "id": c.id,
                    "label": c.label,
                    "group": c.group.as_str(),
                    "run": c.entry.spelling(),
                });
                if let Some(k) = &c.key {
                    o["key"] = json!(k);
                }
                if let Some(t) = c.timeout_s {
                    o["timeout_s"] = json!(t);
                }
                o
            })
            .collect();
        let v = json!({
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "author": self.author,
            "description": self.description,
            "commands": commands,
        });
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
    }
}

/// One folder in the plugins folder, loaded or broken.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    /// The plugin's folder.
    pub dir: PathBuf,
    /// The folder's name.
    pub folder: String,
    /// False when the folder holds [`DISABLED_FILE`]: listed, not loaded.
    pub enabled: bool,
    /// The manifest, or why the plugin is broken (shown to the user).
    pub manifest: Result<PluginManifest, String>,
}

impl Plugin {
    /// The manifest's id, else the folder name (broken plugins).
    pub fn id(&self) -> &str {
        match &self.manifest {
            Ok(m) => &m.id,
            Err(_) => &self.folder,
        }
    }

    /// The manifest's name, else the folder name.
    pub fn name(&self) -> &str {
        match &self.manifest {
            Ok(m) => &m.name,
            Err(_) => &self.folder,
        }
    }

    /// The manifest when the plugin is enabled and not broken.
    pub fn active(&self) -> Option<&PluginManifest> {
        if self.enabled {
            self.manifest.as_ref().ok()
        } else {
            None
        }
    }
}

/// Load every plugin folder in `root` (sorted by folder name). Folders whose
/// name starts with `.` (install staging) and plain files are skipped. A
/// missing `root` gives an empty list. Never fails: a plugin whose manifest
/// is missing or invalid, or whose id another folder (earlier by name)
/// already uses, is returned with its reason in `manifest`.
pub fn scan(root: &Path) -> Vec<Plugin> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| !name.starts_with('.') && name != "__pycache__")
        .collect();
    dirs.sort();
    let mut owners: HashMap<String, String> = HashMap::new();
    dirs.into_iter()
        .map(|(folder, dir)| {
            let enabled = !dir.join(DISABLED_FILE).exists();
            let manifest = PluginManifest::read(&dir).and_then(|m| match owners.get(&m.id) {
                Some(first) => Err(format!(
                    "the plugin in folder `{first}` already has the id `{}`",
                    m.id
                )),
                None => {
                    owners.insert(m.id.clone(), folder.clone());
                    Ok(m)
                }
            });
            Plugin {
                dir,
                folder,
                enabled,
                manifest,
            }
        })
        .collect()
}

/// Find an active (enabled, not broken) plugin's command by ids.
pub fn find<'a>(
    plugins: &'a [Plugin],
    plugin_id: &str,
    command_id: &str,
) -> Option<(&'a Plugin, &'a PluginManifest, &'a PluginCommand)> {
    plugins.iter().find_map(|p| {
        let m = p.active().filter(|m| m.id == plugin_id)?;
        Some((p, m, m.command(command_id)?))
    })
}

/// A key given to a plugin command by [`assign_keys`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    /// The two-letter code.
    pub key: String,
    /// The plugin's id.
    pub plugin: String,
    /// The command's id.
    pub command: String,
}

/// Give the active plugins' commands their keys. A key already used by a
/// built-in (`builtin(key)` returns what it does) stays the built-in's; a key
/// two plugins ask for goes to the first (by folder name). Each refusal is
/// returned as a warning for the user.
pub fn assign_keys(
    plugins: &[Plugin],
    builtin: impl Fn(&str) -> Option<String>,
) -> (Vec<KeyBinding>, Vec<String>) {
    let mut out: Vec<KeyBinding> = Vec::new();
    let mut warnings = Vec::new();
    for p in plugins {
        let Some(m) = p.active() else { continue };
        for c in &m.commands {
            let Some(key) = &c.key else { continue };
            if let Some(what) = builtin(key) {
                warnings.push(format!(
                    "Plugin {}: key {key} of \"{}\" is Ferro's {what}; the built-in keeps it",
                    m.name, c.label
                ));
            } else if let Some(other) = out.iter().find(|b| &b.key == key) {
                warnings.push(format!(
                    "Plugin {}: key {key} of \"{}\" is already used by plugin {}",
                    m.name, c.label, other.plugin
                ));
            } else {
                out.push(KeyBinding {
                    key: key.clone(),
                    plugin: m.id.clone(),
                    command: c.id.clone(),
                });
            }
        }
    }
    (out, warnings)
}

fn py_str(s: &str) -> String {
    // A JSON string is a valid Python string literal.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The Python source that runs `entry` of the plugin in `dir` inside a run
/// (`doc`, `tools` and `ferro` are predefined by the run's bootstrap). It
/// makes the plugin folder the current folder and puts it on `sys.path`
/// right after the SDK, then calls the function or runs the script.
pub fn command_source(dir: &Path, entry: &PluginEntry) -> String {
    let d = py_str(&dir.to_string_lossy());
    let head = format!("import os as _os, sys as _sys\n_os.chdir({d})\n_sys.path.insert(1, {d})\n");
    match entry {
        PluginEntry::Function { module, function } => format!(
            "{head}import importlib as _importlib, inspect as _inspect\n\
             _fn = getattr(_importlib.import_module({m}), {f})\n\
             try:\n    _takes_doc = len(_inspect.signature(_fn).parameters) > 0\n\
             except (TypeError, ValueError):\n    _takes_doc = False\n\
             result = _fn(doc) if _takes_doc else _fn()\n",
            m = py_str(module),
            f = py_str(function),
        ),
        PluginEntry::Script(rel) => format!(
            "{head}import runpy as _runpy\n\
             _ns = _runpy.run_path({p}, init_globals={{'doc': doc, 'tools': tools, 'ferro': ferro}}, run_name='__main__')\n\
             result = _ns.get('result')\n",
            p = py_str(&dir.join(rel).to_string_lossy()),
        ),
    }
}

/// The run arguments of a plugin command (its source from
/// [`command_source`], its time limit; the target design option is left to
/// the host, as for `run_python`).
pub fn run_args(dir: &Path, command: &PluginCommand) -> RunArgs {
    RunArgs {
        code: command_source(dir, &command.entry),
        timeout: Duration::from_secs_f64(command.timeout_s.unwrap_or(python::DEFAULT_TIMEOUT_S)),
        in_design_option: None,
    }
}

/// Who a plugin command's changes are recorded as: the plugin (journal
/// actor kind `plugin`, its name), which commits like the user (ADR 0009).
pub fn plugin_ctx(manifest: &PluginManifest, timestamp_ms: u64) -> ExecContext {
    ExecContext {
        actor: Actor {
            kind: ActorKind::Plugin {
                id: manifest.id.clone(),
            },
            name: manifest.name.clone(),
        },
        prompt: None,
        timestamp_ms,
    }
}

/// Run a plugin command to the end on this thread (headless hosts and
/// tests): the same host as `run_python` ([`python::run_blocking_as`]), one
/// group named after the command's label, committed as the plugin. Another
/// thread may cancel it through `control`. Returns the run's report.
pub fn run_command_blocking(
    session: &mut Session,
    config: &PythonConfig,
    plugin: &Plugin,
    command_id: &str,
    control: &RunControl,
) -> Result<Value, Value> {
    let manifest = match &plugin.manifest {
        Ok(m) => m,
        Err(reason) => {
            return Err(json!({"error": format!("plugin {} is broken: {reason}", plugin.folder)}))
        }
    };
    let Some(command) = manifest.command(command_id) else {
        return Err(
            json!({"error": format!("plugin {} has no command {command_id}", manifest.id)}),
        );
    };
    python::run_blocking_as(
        session,
        config,
        &run_args(&plugin.dir, command),
        plugin_ctx(manifest, 0),
        false,
        control,
        &command.label,
    )
}

/// The state of the plugins folder for hot reload: every file's path, size
/// and modification time (`__pycache__` and folders starting with `.` are
/// skipped; [`DISABLED_FILE`] counts, so enabling reloads too).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprint(Vec<(PathBuf, u64, Option<SystemTime>)>);

/// Take the [`Fingerprint`] of `root` (at most [`MAX_WATCHED_FILES`] files,
/// four folders deep). A missing folder gives an empty fingerprint.
pub fn fingerprint(root: &Path) -> Fingerprint {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, u64, Option<SystemTime>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            if out.len() >= MAX_WATCHED_FILES {
                return;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < 4 && !name.starts_with('.') && name != "__pycache__" {
                    walk(&e.path(), depth + 1, out);
                }
            } else {
                out.push((e.path(), meta.len(), meta.modified().ok()));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, 0, &mut v);
    v.sort();
    Fingerprint(v)
}

/// Polls the plugins folder for hot reload: at most once per
/// [`POLL_INTERVAL`], [`Self::poll`] says whether anything changed since the
/// last load. No file-watching dependency; a folder of a few plugins costs
/// microseconds to look at.
#[derive(Debug, Clone)]
pub struct PluginWatch {
    root: PathBuf,
    last: Fingerprint,
    next: Instant,
}

impl PluginWatch {
    /// Watch `root`, taking its state now as loaded.
    pub fn new(root: &Path, now: Instant) -> PluginWatch {
        PluginWatch {
            root: root.to_path_buf(),
            last: fingerprint(root),
            next: now + POLL_INTERVAL,
        }
    }

    /// The watched folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True when the folder changed since the last load (then the new state
    /// counts as loaded). Looks at the disk only when [`POLL_INTERVAL`] has
    /// passed since the last look; earlier calls return false.
    pub fn poll(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + POLL_INTERVAL;
        let f = fingerprint(&self.root);
        if f == self.last {
            return false;
        }
        self.last = f;
        true
    }

    /// The plugins were just reloaded (e.g. the Reload menu item): take the
    /// folder's state as loaded.
    pub fn loaded(&mut self, now: Instant) {
        self.last = fingerprint(&self.root);
        self.next = now + POLL_INTERVAL;
    }
}

/// A plugin id made from a name: ASCII letters and digits kept (lower case;
/// Vietnamese letters lose their marks), anything else becomes `-`.
/// `"Hộp ở gốc"` gives `hop-o-goc`; an empty result gives `my-plugin`.
pub fn plugin_id_for(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        let c = fold_vietnamese(c).to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let mut id: String = id.trim_end_matches('-').chars().take(40).collect();
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        "my-plugin".into()
    } else {
        id
    }
}

fn fold_vietnamese(c: char) -> char {
    const GROUPS: &[(&str, char)] = &[
        ("àáảãạăằắẳẵặâầấẩẫậ", 'a'),
        ("ÀÁẢÃẠĂẰẮẲẴẶÂẦẤẨẪẬ", 'A'),
        ("èéẻẽẹêềếểễệ", 'e'),
        ("ÈÉẺẼẸÊỀẾỂỄỆ", 'E'),
        ("ìíỉĩị", 'i'),
        ("ÌÍỈĨỊ", 'I'),
        ("òóỏõọôồốổỗộơờớởỡợ", 'o'),
        ("ÒÓỎÕỌÔỒỐỔỖỘƠỜỚỞỠỢ", 'O'),
        ("ùúủũụưừứửữự", 'u'),
        ("ÙÚỦŨỤƯỪỨỬỮỰ", 'U'),
        ("ỳýỷỹỵ", 'y'),
        ("ỲÝỶỸỴ", 'Y'),
        ("đ", 'd'),
        ("Đ", 'D'),
    ];
    GROUPS
        .iter()
        .find(|(set, _)| set.contains(c))
        .map(|(_, a)| *a)
        .unwrap_or(c)
}

/// `main.py` of a new plugin: one working command, "Box at origin".
pub const TEMPLATE_MAIN: &str = r#""""My Ferro plugin. Edit this file, save it, then click the command in Ferro.

Each function named in plugin.json ("run": "main:<function>") is a command.
It receives `doc` (the open Ferro document) and runs as ONE undo step: if it
raises an exception, nothing it did is kept.

Lengths are millimetres, or text with units: "3m", "300cm".
"""

import ferro


def box_at_origin(doc):
    """Make a box at the origin, asking for its height."""
    height = ferro.ask("Box height", default="3m")
    if height is None:  # the user pressed Cancel
        return None
    box = doc.call("create_box", origin=[0, 0, 0], width="2m", depth="2m", height=height)
    # The user's selection: doc.selection() gives the selected element ids.
    print("selected before:", doc.selection())
    return box
"#;

/// `README.md` of a new plugin (Vietnamese, then English).
pub const TEMPLATE_README: &str = r#"# {name}

## Tiếng Việt

Plugin Ferro của bạn. Chỉ cần 3 bước:

1. **Sửa `main.py`**: mỗi hàm được khai báo trong `plugin.json`
   (`"run": "main:ten_ham"`) là một lệnh. Hàm nhận `doc` (bản vẽ đang mở).
2. **Lưu file.** Ferro tự nạp lại plugin sau khoảng một giây.
3. **Bấm lệnh** trong menu *Plugins*, ở nhóm *Plugins* bên trái, hoặc gõ tên
   lệnh trong bảng lệnh (Ctrl+K).

Mỗi lần chạy là **một bước Undo**; nếu có lỗi, không có gì bị thay đổi.
`ferro.ask("Chiều cao", default="3m")` hỏi người dùng một giá trị;
`doc.selection()` trả về id các đối tượng đang chọn. Plugin chạy với quyền
của bạn, như mọi chương trình bạn cài.

## English

Your Ferro plugin. Three steps:

1. **Edit `main.py`**: each function named in `plugin.json`
   (`"run": "main:function_name"`) is a command. It receives `doc` (the
   open document).
2. **Save.** Ferro reloads the plugin within a second.
3. **Click the command** in the *Plugins* menu, in the *Plugins* group on
   the left, or type its name in the command palette (Ctrl+K).

Each run is **one undo step**; if it fails, nothing is changed.
`ferro.ask("Height", default="3m")` asks the user for a value;
`doc.selection()` returns the ids of the selected elements. A plugin runs
with your rights, like any program you install.

Python SDK guide: <https://github.com/ktsdatpham/ferro/blob/main/python/README.md>
"#;

/// Create a new plugin from the template in `root`: a folder named after
/// the plugin id ([`plugin_id_for`] of `name`, with `-2`, `-3`... when
/// taken) holding `plugin.json` (one command, "Box at origin", key `BX`),
/// `main.py` ([`TEMPLATE_MAIN`]) and `README.md` ([`TEMPLATE_README`]).
/// Returns the folder. Fails when the folder cannot be written.
pub fn create_plugin(root: &Path, name: &str, author: &str) -> Result<PathBuf, String> {
    let name = match name.trim() {
        "" => "My plugin",
        n => n,
    };
    let author = match author.trim() {
        "" => "me",
        a => a,
    };
    let base = plugin_id_for(name);
    std::fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
    let taken: HashSet<String> = scan(root).iter().map(|p| p.id().to_string()).collect();
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) || root.join(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    let dir = root.join(&id);
    std::fs::create_dir(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let manifest = PluginManifest {
        id,
        name: name.to_string(),
        version: "0.1.0".into(),
        author: author.to_string(),
        description: "Made from the Ferro plugin template.".into(),
        commands: vec![PluginCommand {
            id: "box_at_origin".into(),
            label: "Box at origin".into(),
            key: Some("BX".into()),
            group: PluginGroup::Panel,
            entry: PluginEntry::Function {
                module: "main".into(),
                function: "box_at_origin".into(),
            },
            timeout_s: None,
        }],
    };
    let write = |file: &str, text: &str| {
        std::fs::write(dir.join(file), text).map_err(|e| format!("cannot write {file}: {e}"))
    };
    write(MANIFEST_FILE, &manifest.to_json())?;
    write("main.py", TEMPLATE_MAIN)?;
    write("README.md", &TEMPLATE_README.replace("{name}", name))?;
    Ok(dir)
}

/// Enable or disable the plugin in `dir` (removes or writes
/// [`DISABLED_FILE`]).
pub fn set_enabled(dir: &Path, enabled: bool) -> Result<(), String> {
    let marker = dir.join(DISABLED_FILE);
    let r = if enabled {
        match std::fs::remove_file(&marker) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    } else {
        std::fs::write(&marker, "Remove this file to enable the plugin.\n")
    };
    r.map_err(|e| format!("cannot change {}: {e}", marker.display()))
}

/// Remove the plugin folder `dir`, which must be directly inside `root`
/// (refused otherwise, so a wrong path never deletes anything else).
pub fn remove_plugin(root: &Path, dir: &Path) -> Result<(), String> {
    let in_root =
        dir.parent().is_some_and(|p| p == root) && dir.file_name().is_some_and(|n| !n.is_empty());
    if !in_root {
        return Err(format!("{} is not in the plugins folder", dir.display()));
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("cannot remove {}: {e}", dir.display()))
}

/// A plugin unpacked or copied into a staging folder, waiting for the user
/// to confirm the install screen ([`finish_install`]) or cancel it
/// ([`cancel_install`]).
#[derive(Debug, Clone, PartialEq)]
pub struct StagedPlugin {
    /// The staging folder (`root/.install-<random>`), removed when done.
    pub staging: PathBuf,
    /// The plugin's own folder inside it (the folder holding the manifest).
    pub plugin_dir: PathBuf,
    /// Its manifest.
    pub manifest: PluginManifest,
    /// Version of the installed plugin with the same id, which the install
    /// replaces.
    pub replaces: Option<String>,
}

fn copy_tree(src: &Path, dst: &Path, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    let entries =
        std::fs::read_dir(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name();
        if t.is_symlink() || name == "__pycache__" {
            continue; // links could point anywhere; caches are rebuilt
        }
        let to = dst.join(&name);
        if t.is_dir() {
            copy_tree(&e.path(), &to, files, bytes)?;
        } else if t.is_file() {
            *files += 1;
            *bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            if *files > MAX_INSTALL_FILES || *bytes > MAX_INSTALL_BYTES {
                return Err(format!(
                    "the plugin is too large (more than {MAX_INSTALL_FILES} files or {} MB)",
                    MAX_INSTALL_BYTES / (1024 * 1024)
                ));
            }
            std::fs::copy(e.path(), &to)
                .map_err(|err| format!("cannot copy {}: {err}", e.path().display()))?;
        }
    }
    Ok(())
}

/// The folder holding the manifest: `dir` itself, or its only subfolder
/// (zips usually hold one top folder).
fn manifest_folder(dir: &Path) -> Option<PathBuf> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n != "__MACOSX"))
        .collect();
    match subs.as_slice() {
        [one] if one.join(MANIFEST_FILE).is_file() => Some(one.clone()),
        _ => None,
    }
}

/// Unpack (`.zip`) or copy (folder) `source` into a staging folder in
/// `root` and read its manifest, for the install screen. Zips are unpacked
/// with [`extract_zip`] (no path may leave the staging folder, size and
/// count limits). Fails, leaving nothing behind, when the source cannot be
/// read, is too large, or holds no valid plugin.
pub fn stage_install(source: &Path, root: &Path) -> Result<StagedPlugin, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
    let staging = root.join(format!(".install-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&staging).map_err(|e| format!("cannot create a staging folder: {e}"))?;
    let fail = |m: String| {
        let _ = std::fs::remove_dir_all(&staging);
        Err(m)
    };
    let is_zip = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let copied = if source.is_dir() {
        copy_tree(source, &staging.join("plugin"), &mut 0, &mut 0)
    } else if is_zip {
        extract_zip(
            source,
            &staging,
            &ZipLimits {
                max_files: MAX_INSTALL_FILES,
                max_bytes: MAX_INSTALL_BYTES,
            },
        )
        .map(|_| ())
    } else {
        Err("choose a plugin folder or a .zip file".into())
    };
    if let Err(m) = copied {
        return fail(m);
    }
    let Some(plugin_dir) =
        manifest_folder(&staging).or_else(|| manifest_folder(&staging.join("plugin")))
    else {
        return fail(format!(
            "no {MANIFEST_FILE} found: this is not a Ferro plugin"
        ));
    };
    let manifest = match PluginManifest::read(&plugin_dir) {
        Ok(m) => m,
        Err(m) => return fail(format!("the plugin is broken: {m}")),
    };
    let replaces = scan(root)
        .into_iter()
        .find(|p| p.id() == manifest.id)
        .map(|p| match p.manifest {
            Ok(m) => m.version,
            Err(_) => "broken".into(),
        });
    Ok(StagedPlugin {
        staging,
        plugin_dir,
        manifest,
        replaces,
    })
}

/// The user confirmed the install screen: move the staged plugin into
/// `root`, enabled, replacing the installed plugin with the same id (in
/// whatever folder). Its folder is `root/<id>`, or `<id>-2`, `-3`... when a
/// different plugin uses that folder name. Returns its folder.
pub fn finish_install(staged: StagedPlugin, root: &Path) -> Result<PathBuf, String> {
    let id = staged.manifest.id.clone();
    for p in scan(root) {
        let same = p.manifest.as_ref().is_ok_and(|m| m.id == id)
            || (p.manifest.is_err() && p.folder == id);
        if same {
            if let Err(m) = remove_plugin(root, &p.dir) {
                cancel_install(staged);
                return Err(m);
            }
        }
    }
    let mut dest = root.join(&id);
    let mut n = 2;
    while dest.exists() {
        dest = root.join(format!("{id}-{n}"));
        n += 1;
    }
    let r = std::fs::rename(&staged.plugin_dir, &dest)
        .map_err(|e| format!("cannot install into {}: {e}", dest.display()));
    let _ = std::fs::remove_dir_all(&staged.staging);
    r?;
    let _ = std::fs::remove_file(dest.join(DISABLED_FILE));
    Ok(dest)
}

/// The user cancelled the install screen: remove the staging folder.
pub fn cancel_install(staged: StagedPlugin) {
    let _ = std::fs::remove_dir_all(&staged.staging);
}

/// Remove staging folders left by an install the app never finished (it
/// crashed or quit with the screen open).
pub fn clean_staging(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with(".install-") {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests;
