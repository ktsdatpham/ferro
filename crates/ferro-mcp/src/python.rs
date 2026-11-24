//! `run_python` (ADR 0005 §5): run a Python script against the open document
//! as one transaction group — all or nothing, one undo step, and (for agents
//! without raised autonomy) one Proposal.
//!
//! The script runs **out of process** in the user's own `python3`, started in
//! isolated mode (`-I`: no current folder, no user site, `PYTHON*` variables
//! ignored) with the SDK bundled in this binary on its path. It talks to the
//! host over Ferro Link with a **one-time token** passed in environment
//! variables (`FERRO_LINK_PORT`, `FERRO_LINK_TOKEN`), never in argv. Its tool
//! calls are routed by [`PythonRun::handle`] into the group the run opened.
//!
//! The same code serves the headless host ([`crate::tools::Server`], used by
//! `ferro-mcp` and `ferro-cli`) and the app (`ferro-app`, which polls
//! [`PythonRun::poll`] once per frame so the UI thread never blocks).
//!
//! Lifecycle: [`RunArgs::from_tool`] → [`decide`] → [`PythonRun::start`]
//! (spawns, opens the group) → script requests through
//! [`PythonRun::handle`] → [`PythonRun::poll`] until it returns a
//! [`RunEnd`] → [`PythonRun::finish`] (commits or rolls back, reports).
//!
//! Cancelling (`cancel_python`, M5-14.7): [`PythonRun::cancel`] kills the
//! process tree like a timeout and the run finishes with [`CANCELLED`]. Only
//! the user and the caller that started the run may cancel it
//! ([`may_cancel`]); headless hosts expose this to other threads through a
//! [`RunControl`].

use ferro_core::ops::{Actor, ActorKind, DocumentChanged, ExecContext, SubTransaction};
use ferro_core::{Document, ElementId, Session};
use ferro_tools::link::{self as proto, LinkRequest, LinkResponse};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Tool name (MCP and Ferro Link).
pub const TOOL: &str = "run_python";

/// No Python interpreter could be started (not installed, wrong path, or
/// older than 3.9).
pub const UNAVAILABLE: &str = "ferro.python.unavailable";
/// The script ran past its time limit; it was killed and rolled back.
pub const TIMEOUT: &str = "ferro.python.timeout";
/// The script raised an exception or exited with a non-zero status; rolled
/// back. The traceback is in `stderr`.
pub const FAILED: &str = "ferro.python.failed";
/// A script tried to start `run_python` itself.
pub const NESTED: &str = "ferro.python.nested";
/// Another script is running; the call was refused (retry when it is done).
pub const BUSY: &str = "ferro.python.busy";
/// The caller may not cancel this run (only the user and the caller that
/// started it). Plugins may start scripts (ADR 0009; D-51 refused them).
pub const NOT_ALLOWED: &str = "ferro.python.not_allowed";
/// The script was stopped (`cancel_python`, the app's Stop button or Esc
/// twice); its process tree was killed and nothing was changed.
pub const CANCELLED: &str = "ferro.python.cancelled";

/// Tool name of the cancel request (MCP and Ferro Link).
pub const CANCEL_TOOL: &str = "cancel_python";

/// Time limit when the caller gives none (seconds).
pub const DEFAULT_TIMEOUT_S: f64 = 30.0;
/// Longest time limit a caller may ask for (seconds).
pub const MAX_TIMEOUT_S: f64 = 300.0;
/// Bytes of `stdout` / `stderr` kept each; the rest is counted and dropped.
pub const OUTPUT_CAP: usize = 64 * 1024;
/// Largest `result` (JSON bytes) returned to the caller.
pub const RESULT_CAP: u64 = 1024 * 1024;
/// Exit status the bootstrap uses when the interpreter is too old. A script
/// exiting with it is reported as status 1.
pub const EXIT_UNSUPPORTED: i32 = 86;

/// Ferro Link requests a running script may send besides tools: savepoints
/// of the run's group, used by the SDK's `doc.transaction(name)`.
pub const BEGIN_TRANSACTION: &str = "begin_transaction";
/// Keep the changes since the innermost `begin_transaction`.
pub const COMMIT_TRANSACTION: &str = "commit_transaction";
/// Discard the changes since the innermost `begin_transaction`.
pub const ROLLBACK_TRANSACTION: &str = "rollback_transaction";

/// Ferro Link request only a running script may send (SDK `ferro.ask`, ADR
/// 0009): `{prompt, default?}` asks the user for one value in a small dialog
/// and answers `{value}`, a string, or `null` when the user cancelled. A
/// headless host answers the default ([`headless_answer`]). A stable name.
pub const GET_USER_INPUT: &str = "get_user_input";

/// Requests about the user's session that only the app can answer: the
/// selection (`get_selection`, `select_elements`) and [`GET_USER_INPUT`].
/// [`PythonRun::handle`] leaves them to the host.
pub const APP_ONLY: [&str; 3] = ["get_selection", "select_elements", GET_USER_INPUT];

/// How long a script waits for the user to answer [`GET_USER_INPUT`]. The
/// script's own time limit is paused meanwhile ([`PythonRun::extend_deadline`]).
pub const USER_INPUT_TIMEOUT: Duration = Duration::from_secs(600);

/// The answer of a headless host (no app, so no user and no selection) to an
/// [`APP_ONLY`] request: the selection is empty, nothing gets selected, and
/// [`GET_USER_INPUT`] returns its `default` (`null` when none). `None` for
/// any other request.
pub fn headless_answer(tool: &str, args: &Value) -> Option<Value> {
    match tool {
        "get_selection" => Some(json!({"elements": []})),
        "select_elements" => Some(json!({"selected": 0})),
        GET_USER_INPUT => Some(json!({"value": user_input_default(args)})),
        _ => None,
    }
}

/// The `default` of a [`GET_USER_INPUT`] request as a string (numbers are
/// written out), or `null`.
pub fn user_input_default(args: &Value) -> Value {
    match args.get("default") {
        Some(Value::String(s)) => json!(s),
        Some(Value::Number(n)) => json!(n.to_string()),
        Some(Value::Bool(b)) => json!(b.to_string()),
        _ => Value::Null,
    }
}

/// The SDK shipped inside this binary (`python/ferro/*.py`), written to a
/// private temporary folder for each run so it always matches this tool set.
const SDK_FILES: &[(&str, &str)] = &[
    (
        "__init__.py",
        include_str!("../../../python/ferro/__init__.py"),
    ),
    ("_run.py", include_str!("../../../python/ferro/_run.py")),
    (
        "_runtime.py",
        include_str!("../../../python/ferro/_runtime.py"),
    ),
    ("_tools.py", include_str!("../../../python/ferro/_tools.py")),
    (
        "document.py",
        include_str!("../../../python/ferro/document.py"),
    ),
    ("link.py", include_str!("../../../python/ferro/link.py")),
];

/// Started with `-c`: refuse interpreters older than 3.9 (written so Python 2
/// can parse it), put the SDK first on the path, hand over to `ferro._run`.
const BOOTSTRAP: &str = "import sys\n\
if sys.version_info < (3, 9):\n    \
sys.stderr.write('ferro: run_python needs Python 3.9 or newer\\n')\n    \
sys.exit(86)\n\
import os\n\
sys.path.insert(0, os.environ['FERRO_PYTHON_SDK'])\n\
from ferro import _run\n\
sys.exit(_run.main())\n";

/// The MCP / Ferro Link schema of `run_python`.
pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Run a Python script against the open document in ONE step: all its changes are one transaction group (all or nothing, one undo step; for agents one proposal the user accepts or rejects). Use it instead of many single tool calls. The script runs in the user's python3 with the `ferro` SDK: `doc` (ferro.Document) and `tools` (one function per Ferro tool, same names and arguments) are predefined, e.g. `walls = doc.create_walls([(0, 0), (\"8m\", 0), (\"8m\", \"6m\"), (0, \"6m\")], closed=True, height=\"3m\")`. Set a variable `result` to return JSON data. An uncaught exception or a non-zero exit rolls everything back (ferro.python.failed, traceback in stderr); running past timeout_s kills it and rolls back (ferro.python.timeout). `with doc.transaction(\"name\"):` makes a block all or nothing inside the script. With in_design_option the changes go into that design option, not the shared model. The user, or you, can stop a running script with cancel_python (ferro.python.cancelled, rolled back). Returns {ok, stdout, stderr (64 KB each at most), result, changed: [element ids], failures}.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "code": {"type": "string", "description": "Python 3 source. `doc`, `tools` and `ferro` are predefined; lengths in mm or strings with units (\"3m\")."},
                "timeout_s": {"type": "number", "description": "Time limit in seconds (default 30, at most 300)."},
                "in_design_option": {"type": ["integer", "string"], "description": "Where the script's changes land: a design option id (every change goes into that option's layer; a call may still name its own in_design_option), or \"shared\" for the shared model. Omitted (or null): the option the user is editing in the Ferro app, else the shared model."}
            },
            "required": ["code"]
        }
    })
}

/// The MCP / Ferro Link schema of `cancel_python`.
pub fn cancel_tool() -> Value {
    json!({
        "name": CANCEL_TOOL,
        "description": "Stop the Python script running now (started with run_python): its process is killed and everything it did is rolled back; its run_python call answers ferro.python.cancelled with the output so far. Only the client that started the script (or the user, in the Ferro app) may stop it; others get ferro.python.not_allowed. Returns {cancelled: true}, or {cancelled: false} when no script is running.",
        "inputSchema": {"type": "object", "properties": {}, "required": []}
    })
}

/// A failure body like the tools return: `{error, failures: [{severity, code, message, elements}]}`.
pub fn failure(code: &str, message: impl Into<String>) -> Value {
    let message = message.into();
    json!({
        "error": message,
        "failures": [{"severity": "error", "code": code, "message": message, "elements": []}],
    })
}

/// Arguments of one `run_python` call.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArgs {
    /// Python source text.
    pub code: String,
    /// Time limit.
    pub timeout: Duration,
    /// Where the script's commands land unless a call names its own target
    /// (ADR 0006): `None` = not given, so the host's default applies (the
    /// option the user is editing in the app, else the shared model);
    /// `Some(None)` = `"shared"`; `Some(Some(id))` = that design option.
    pub in_design_option: Option<Option<ElementId>>,
}

impl RunArgs {
    /// Read `{code, timeout_s?, in_design_option?}`. Fails (a tool error
    /// body) when `code` is missing or not a string, `timeout_s` is not a
    /// number in (0, [`MAX_TIMEOUT_S`]], or `in_design_option` is neither an
    /// id nor `"shared"`.
    pub fn from_tool(args: &Value) -> Result<RunArgs, Value> {
        let Some(code) = args.get("code").and_then(Value::as_str) else {
            return Err(json!({"error": "missing code (a string of Python source)"}));
        };
        let secs = match args.get("timeout_s") {
            None | Some(Value::Null) => DEFAULT_TIMEOUT_S,
            Some(v) => match v.as_f64() {
                Some(s) if s > 0.0 && s <= MAX_TIMEOUT_S => s,
                _ => {
                    return Err(json!({"error": format!(
                        "timeout_s must be a number of seconds above 0 and at most {MAX_TIMEOUT_S}"
                    )}))
                }
            },
        };
        let in_design_option = ferro_tools::design_option_argument(args.get("in_design_option"))
            .map_err(|m| json!({ "error": m }))?;
        Ok(RunArgs {
            code: code.to_string(),
            timeout: Duration::from_secs_f64(secs),
            in_design_option,
        })
    }
}

/// How long a Ferro Link client should wait for the answer to `tool`: the
/// script's time limit plus a margin for `run_python`,
/// [`USER_INPUT_TIMEOUT`] for [`GET_USER_INPUT`], else 120 s.
pub fn answer_timeout(tool: &str, args: &Value) -> Duration {
    const DEFAULT: Duration = Duration::from_secs(120);
    if tool == GET_USER_INPUT {
        return USER_INPUT_TIMEOUT;
    }
    if tool != TOOL {
        return DEFAULT;
    }
    let limit = RunArgs::from_tool(args)
        .map(|a| a.timeout)
        .unwrap_or(Duration::from_secs_f64(DEFAULT_TIMEOUT_S));
    (limit + Duration::from_secs(30)).max(DEFAULT)
}

/// What becomes of a script's changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Committed as one undo step when the script succeeds.
    Commit,
    /// Collected into the caller's Proposal (D-26), which the user accepts or
    /// rejects.
    Propose,
}

/// Who may start a script, and how its changes land (ADR 0005 implementation
/// note, ADR 0009): the user and plugins commit (a plugin has the user's
/// rights, like a Revit or SketchUp add-in); an agent proposes unless the
/// user raised its autonomy (`propose_agents` false). Never fails today; the
/// `Result` keeps room for a future refusal code.
pub fn decide(actor: &Actor, propose_agents: bool) -> Result<RunMode, Value> {
    match actor.kind {
        ActorKind::User | ActorKind::Plugin { .. } => Ok(RunMode::Commit),
        ActorKind::Agent { .. } if propose_agents => Ok(RunMode::Propose),
        ActorKind::Agent { .. } => Ok(RunMode::Commit),
    }
}

/// Whether `caller` may cancel a run started by `started_by`: the user
/// always (the user's command has priority), an agent or plugin only when it
/// is the one that started the run (an agent of the same name, whatever its
/// version; the same plugin id). Others are refused
/// with [`NOT_ALLOWED`]. Clients name themselves in their requests, so this
/// guards against mistakes between cooperating local clients, not against a
/// client lying about its name (all of them hold the Ferro Link token).
pub fn may_cancel(started_by: &Actor, caller: &Actor) -> Result<(), Value> {
    let same = match (&started_by.kind, &caller.kind) {
        (_, ActorKind::User) => true,
        (ActorKind::Agent { .. }, ActorKind::Agent { .. }) => started_by.name == caller.name,
        (ActorKind::Plugin { id: a }, ActorKind::Plugin { id: b }) => a == b,
        _ => false,
    };
    if same {
        return Ok(());
    }
    Err(failure(
        NOT_ALLOWED,
        format!(
            "only the user or {} (which started the script) may cancel it",
            started_by.name
        ),
    ))
}

/// Which interpreter to start and where the SDK comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct PythonConfig {
    /// Interpreter command or path: `$FERRO_PYTHON`, else `python3`
    /// (`python` on Windows).
    pub interpreter: PathBuf,
    /// Folder holding the `ferro` package (`$FERRO_PYTHON_SDK`); `None` uses
    /// the SDK bundled in this binary.
    pub sdk: Option<PathBuf>,
}

impl PythonConfig {
    /// From the environment (`FERRO_PYTHON`, `FERRO_PYTHON_SDK`).
    pub fn from_env() -> PythonConfig {
        let default = if cfg!(windows) { "python" } else { "python3" };
        PythonConfig {
            interpreter: std::env::var_os("FERRO_PYTHON")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(default)),
            sdk: std::env::var_os("FERRO_PYTHON_SDK")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

impl Default for PythonConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// How a script ended, as seen by [`PythonRun::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// The process exited with this status (`None`: killed by a signal).
    Exited(Option<i32>),
    /// The time limit passed; the process tree was killed.
    TimedOut,
    /// Cancelled ([`PythonRun::cancel`]); the process tree was killed.
    Cancelled,
}

/// What the run holds open in the session.
enum Hold {
    /// The run opened the group itself and commits or cancels it.
    Group,
    /// A group was already open (an agent's Proposal, `ferro-cli --atomic`):
    /// the run is a savepoint in it.
    Sub(Box<SubTransaction>),
}

/// Captured output of one pipe: the first [`OUTPUT_CAP`] bytes and the total.
#[derive(Default)]
struct Capture {
    data: Vec<u8>,
    total: usize,
    done: bool,
}

impl Capture {
    fn text(&self) -> (String, bool) {
        let mut s = String::from_utf8_lossy(&self.data).into_owned();
        let truncated = self.total > self.data.len();
        if truncated {
            s.push_str(&format!(
                "\n[ferro: output truncated, {} of {} bytes kept]",
                self.data.len(),
                self.total
            ));
        }
        (s, truncated)
    }
}

fn capture(mut pipe: impl Read + Send + 'static) -> Arc<Mutex<Capture>> {
    let cap = Arc::new(Mutex::new(Capture::default()));
    let out = Arc::clone(&cap);
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if let Ok(mut c) = out.lock() {
                let room = OUTPUT_CAP.saturating_sub(c.data.len());
                let keep = n.min(room);
                c.data.extend_from_slice(&buf[..keep]);
                c.total += n;
            }
        }
        if let Ok(mut c) = out.lock() {
            c.done = true;
        }
    });
    cap
}

/// One running script: its process, its token and the group it holds open.
pub struct PythonRun {
    token: String,
    ctx: ExecContext,
    in_design_option: Option<ElementId>,
    hold: Option<Hold>,
    blocks: Vec<SubTransaction>,
    before: Document,
    child: Child,
    stdout: Arc<Mutex<Capture>>,
    stderr: Arc<Mutex<Capture>>,
    deadline: Instant,
    timeout: Duration,
    exited: Option<(Option<i32>, Instant)>,
    dir: PathBuf,
    tool_failures: Vec<Value>,
    started: Instant,
    cancelled_by: Option<String>,
    targets: Vec<Option<ElementId>>,
}

/// A fresh random token (UUID v4, 122 random bits).
fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Make a private, fresh folder for one run and write the bundled SDK in it.
fn run_dir(write_sdk: bool) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("ferro-run-{}", new_token()));
    #[cfg(unix)]
    let b = {
        use std::os::unix::fs::DirBuilderExt;
        let mut b = std::fs::DirBuilder::new();
        b.mode(0o700);
        b
    };
    // Windows: the per-user temp folder is already private to the user.
    #[cfg(not(unix))]
    let b = std::fs::DirBuilder::new();
    // `create`, not `create_all`: fails if someone planted the folder.
    b.create(&dir)?;
    if write_sdk {
        let pkg = dir.join("ferro");
        b.create(&pkg)?;
        for (name, text) in SDK_FILES {
            std::fs::write(pkg.join(name), text)?;
        }
    }
    Ok(dir)
}

impl PythonRun {
    /// Spawn the interpreter for `args.code` and open the run's group.
    ///
    /// `port` is the Ferro Link port the script will call back on; the host
    /// must route requests carrying [`Self::token`] to [`Self::handle`]. When
    /// no group is open, one named `group_name` is opened with `ctx` (the
    /// caller); otherwise the run is a savepoint in the open group (so an
    /// agent's script joins its Proposal). The script text is recorded on the
    /// group's journal entry (D-29).
    ///
    /// Fails with [`UNAVAILABLE`] when the interpreter cannot be started;
    /// nothing is changed then.
    pub fn start(
        session: &mut Session,
        config: &PythonConfig,
        args: &RunArgs,
        ctx: ExecContext,
        port: u16,
        group_name: &str,
    ) -> Result<PythonRun, Value> {
        let dir = run_dir(config.sdk.is_none()).map_err(|e| {
            failure(
                UNAVAILABLE,
                format!("cannot prepare a folder for the script: {e}"),
            )
        })?;
        let sdk = config.sdk.clone().unwrap_or_else(|| dir.clone());
        let token = new_token();
        let mut cmd = Command::new(&config.interpreter);
        cmd.arg("-I")
            .arg("-u")
            .arg("-c")
            .arg(BOOTSTRAP)
            .env("FERRO_LINK_PORT", port.to_string())
            .env("FERRO_LINK_TOKEN", &token)
            .env("FERRO_PYTHON_SDK", &sdk)
            .env("FERRO_RUN_RESULT", dir.join("result.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // so a timeout kills the whole tree
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(failure(
                    UNAVAILABLE,
                    format!(
                        "cannot start Python `{}`: {e}. Install Python 3.9+ or set FERRO_PYTHON to its path.",
                        config.interpreter.display()
                    ),
                ));
            }
        };
        // The source goes in on stdin, from a thread: a large script never
        // blocks the caller (the UI thread in the app).
        if let Some(mut stdin) = child.stdin.take() {
            let code = args.code.clone();
            std::thread::spawn(move || {
                let _ = stdin.write_all(code.as_bytes());
            });
        }
        let stdout = match child.stdout.take() {
            Some(p) => capture(p),
            None => Arc::new(Mutex::new(Capture {
                done: true,
                ..Capture::default()
            })),
        };
        let stderr = match child.stderr.take() {
            Some(p) => capture(p),
            None => Arc::new(Mutex::new(Capture {
                done: true,
                ..Capture::default()
            })),
        };
        let before = session.doc().clone();
        let hold = match session.begin_sub_transaction() {
            Some(sub) => Hold::Sub(Box::new(sub)),
            None => {
                session.begin_group(group_name, ctx.clone());
                Hold::Group
            }
        };
        session.add_group_script(args.code.clone());
        Ok(PythonRun {
            token,
            ctx,
            in_design_option: args.in_design_option.flatten(),
            hold: Some(hold),
            blocks: Vec::new(),
            before,
            child,
            stdout,
            stderr,
            deadline: Instant::now() + args.timeout,
            timeout: args.timeout,
            exited: None,
            dir,
            tool_failures: Vec::new(),
            started: Instant::now(),
            cancelled_by: None,
            targets: Vec::new(),
        })
    }

    /// How long the script has been running.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Push the time limit back by `by`: the time the script spent waiting
    /// for the user ([`GET_USER_INPUT`]) does not count against it.
    pub fn extend_deadline(&mut self, by: Duration) {
        self.deadline += by;
    }

    /// Where the script's commands landed so far, in first-use order without
    /// repeats: `None` = the shared model, `Some(id)` = that design option
    /// (shown on the Proposal bar, ADR 0006).
    pub fn targets(&self) -> &[Option<ElementId>] {
        &self.targets
    }

    /// Stop the script now: kill its process tree (as on a timeout, so a
    /// script ignoring SIGTERM or running child processes is stopped too).
    /// `by` names who cancelled (the user, or the agent), for the message.
    /// Pass the returned end to [`Self::finish`], which rolls everything back
    /// and reports [`CANCELLED`] with the output so far.
    pub fn cancel(&mut self, by: &str) -> RunEnd {
        self.kill();
        self.cancelled_by = Some(by.to_string());
        RunEnd::Cancelled
    }

    /// The one-time Ferro Link token of this run. It is valid only while the
    /// run lasts; hosts must stop accepting it once [`Self::finish`] ran.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The caller that started the run (its actor goes on the journal).
    pub fn ctx(&self) -> &ExecContext {
        &self.ctx
    }

    /// Answer one request from the script. `None` for requests the host must
    /// answer itself ([`APP_ONLY`]: the selection, [`GET_USER_INPUT`]); a
    /// host refuses anything else it does not know.
    ///
    /// Commands run inside the run's group with the caller's context, and
    /// inside the run's design option (`in_design_option`) when it has one
    /// and the call names none (`Session::execute_in_option`, which commits
    /// through the group like any command).
    /// Refused here: `run_python` ([`NESTED`]), `undo` / `redo` (raise an
    /// exception to roll back instead), `cancel_python` (a script stops by
    /// raising or exiting). Savepoint requests
    /// ([`BEGIN_TRANSACTION`] and friends) map onto
    /// [`Session::begin_sub_transaction`].
    pub fn handle(
        &mut self,
        session: &mut Session,
        tool: &str,
        args: &Value,
    ) -> Option<Result<Value, Value>> {
        if APP_ONLY.contains(&tool) {
            return None;
        }
        let r = match tool {
            TOOL => Err(failure(
                NESTED,
                "a script cannot start run_python; call the tools directly",
            )),
            "undo" | "redo" | CANCEL_TOOL => Err(json!({"error": format!(
                "{tool} is not available inside run_python; raise an exception to roll the script back"
            )})),
            "link_status" => Ok(json!({
                "linked": true,
                "run_python": true,
                "project": session.doc().project().name,
            })),
            BEGIN_TRANSACTION => match session.begin_sub_transaction() {
                Some(sub) => {
                    self.blocks.push(sub);
                    Ok(json!({"depth": self.blocks.len()}))
                }
                None => Err(json!({"error": "the run's group is not open"})),
            },
            COMMIT_TRANSACTION => match self.blocks.pop() {
                Some(_) => Ok(json!({"depth": self.blocks.len()})),
                None => Err(json!({"error": "no open transaction to commit"})),
            },
            ROLLBACK_TRANSACTION => match self.blocks.pop() {
                Some(sub) => match session.rollback_sub_transaction(sub) {
                    Some(c) => Ok(json!({"depth": self.blocks.len(), "changed": c})),
                    None => Err(json!({"error": "the run's group is not open"})),
                },
                None => Err(json!({"error": "no open transaction to roll back"})),
            },
            // The run's target, unless the call names its own.
            _ => match ferro_tools::with_default_design_option(tool, args, self.in_design_option) {
                Ok((args, target)) => {
                    let r = ferro_tools::call(session, &self.ctx, tool, &args)?;
                    if ferro_tools::runs_in_design_option(tool)
                        && r.is_ok()
                        && !self.targets.contains(&target)
                    {
                        self.targets.push(target);
                    }
                    r
                }
                Err(m) => Err(json!({ "error": m })),
            },
        };
        if let Err(body) = &r {
            if let Some(f) = body.get("failures").and_then(Value::as_array) {
                self.tool_failures = f.clone();
            }
        }
        Some(r)
    }

    /// Wait (at most half a second) until the output threads have read
    /// everything the killed process tree wrote, so a cancelled or timed-out
    /// run still reports its output so far.
    fn settle_output(&self) {
        let until = Instant::now() + Duration::from_millis(500);
        let done = |c: &Arc<Mutex<Capture>>| c.lock().map(|c| c.done).unwrap_or(true);
        while !(done(&self.stdout) && done(&self.stderr)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Check the process without blocking. Returns how it ended once it has
    /// exited and its output is read (or a second has passed since, when a
    /// child process still holds the pipes); kills the process tree and
    /// returns [`RunEnd::TimedOut`] once the time limit has passed.
    pub fn poll(&mut self) -> Option<RunEnd> {
        let now = Instant::now();
        if self.exited.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) => self.exited = Some((status.code(), now)),
                Ok(None) if now >= self.deadline => {
                    self.kill();
                    return Some(RunEnd::TimedOut);
                }
                Ok(None) => return None,
                Err(_) => self.exited = Some((None, now)),
            }
        }
        let (code, at) = self.exited?;
        let read = |c: &Arc<Mutex<Capture>>| c.lock().map(|c| c.done).unwrap_or(true);
        if (read(&self.stdout) && read(&self.stderr)) || now >= at + Duration::from_secs(1) {
            Some(RunEnd::Exited(code))
        } else {
            None
        }
    }

    /// Kill the interpreter and every process it started.
    fn kill(&mut self) {
        let pid = self.child.id();
        #[cfg(unix)]
        {
            if let Ok(pgid) = i32::try_from(pid) {
                // SAFETY: plain syscall on a process group we created
                // (`process_group(0)` makes the child its leader).
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(any(unix, windows)))]
        let _ = pid;
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.exited = Some((None, Instant::now()));
    }

    /// Commit or roll back, then report. `Ok` when the script exited with
    /// status 0: `{ok: true, stdout, stderr, result, changed, failures: []}`.
    /// Otherwise everything the script did is rolled back and `Err` carries
    /// the same fields plus `error`, with failures starting with
    /// [`TIMEOUT`], [`CANCELLED`], [`UNAVAILABLE`] or [`FAILED`] and followed by those of
    /// the last tool call that failed. `changed` lists the ids of elements
    /// added, modified or deleted (sorted; empty after a rollback).
    pub fn finish(mut self, session: &mut Session, end: RunEnd) -> Result<Value, Value> {
        let ok = end == RunEnd::Exited(Some(0));
        if matches!(end, RunEnd::TimedOut | RunEnd::Cancelled) {
            self.settle_output();
        }
        let result = if ok { self.read_result() } else { Value::Null };
        let hold = self.hold.take();
        self.blocks.clear();
        let changed = if ok {
            if matches!(hold, Some(Hold::Group)) {
                session.end_group();
            }
            ids(&DocumentChanged::between(TOOL, &self.before, session.doc()))
        } else {
            match hold {
                Some(Hold::Group) => {
                    session.cancel_group();
                }
                Some(Hold::Sub(sub)) => {
                    session.rollback_sub_transaction(*sub);
                }
                None => {}
            }
            Vec::new()
        };
        let (stdout, stdout_truncated) = self.stdout.lock().map(|c| c.text()).unwrap_or_default();
        let (stderr, stderr_truncated) = self.stderr.lock().map(|c| c.text()).unwrap_or_default();
        let mut body = json!({
            "ok": ok,
            "stdout": stdout,
            "stderr": stderr,
            "result": result,
            "changed": changed,
            "failures": [],
        });
        if stdout_truncated {
            body["stdout_truncated"] = json!(true);
        }
        if stderr_truncated {
            body["stderr_truncated"] = json!(true);
        }
        if ok {
            return Ok(body);
        }
        let (code, message) = match end {
            RunEnd::TimedOut => (
                TIMEOUT,
                format!(
                    "the script ran longer than {} s; it was stopped and nothing was changed",
                    self.timeout.as_secs_f64()
                ),
            ),
            RunEnd::Cancelled => (
                CANCELLED,
                format!(
                    "the script was stopped by {} after {:.1} s; nothing was changed",
                    self.cancelled_by.as_deref().unwrap_or("the user"),
                    self.started.elapsed().as_secs_f64()
                ),
            ),
            RunEnd::Exited(Some(EXIT_UNSUPPORTED)) => (
                UNAVAILABLE,
                "the Python interpreter is older than 3.9; nothing was changed".to_string(),
            ),
            RunEnd::Exited(Some(c)) => (
                FAILED,
                format!("the script failed (exit status {c}); nothing was changed, see stderr"),
            ),
            RunEnd::Exited(None) => (
                FAILED,
                "the script was killed; nothing was changed".to_string(),
            ),
        };
        let mut failures =
            vec![json!({"severity": "error", "code": code, "message": message, "elements": []})];
        failures.append(&mut self.tool_failures);
        body["error"] = json!(message);
        body["failures"] = json!(failures);
        Err(body)
    }

    fn read_result(&self) -> Value {
        let path = self.dir.join("result.json");
        let Ok(file) = std::fs::File::open(&path) else {
            return Value::Null;
        };
        let mut text = String::new();
        if file.take(RESULT_CAP + 1).read_to_string(&mut text).is_err() {
            return json!({"error": "the result is not UTF-8 JSON"});
        }
        if text.len() as u64 > RESULT_CAP {
            return json!({"error": format!("the result is larger than {RESULT_CAP} bytes")});
        }
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
}

impl Drop for PythonRun {
    fn drop(&mut self) {
        if self.exited.is_none() {
            self.kill();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ids(c: &DocumentChanged) -> Vec<u64> {
    let mut v: Vec<u64> = c
        .added
        .iter()
        .chain(&c.modified)
        .chain(&c.deleted)
        .map(|id| id.0)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// A tool call from a script, waiting for the host thread.
pub struct ScriptCall {
    /// The request as received.
    pub req: LinkRequest,
    /// Where the answer goes.
    pub reply: Sender<LinkResponse>,
}

/// A Ferro Link listener for one headless run (the app uses its own link
/// server instead). Accepts only requests carrying the run's token and hands
/// them to the host thread; stops listening when dropped.
pub struct ScriptLink {
    port: u16,
    rx: Receiver<ScriptCall>,
    stop: Arc<AtomicBool>,
    token: Arc<Mutex<String>>,
}

impl ScriptLink {
    /// Listen on `127.0.0.1` (random port). Each call waits at most
    /// `answer_within` for the host.
    pub fn bind(answer_within: Duration) -> std::io::Result<ScriptLink> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let (tx, rx) = channel::<ScriptCall>();
        let stop = Arc::new(AtomicBool::new(false));
        let token = Arc::new(Mutex::new(String::new()));
        let (stop2, token2) = (Arc::clone(&stop), Arc::clone(&token));
        std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let (tx, token) = (tx.clone(), Arc::clone(&token2));
                std::thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                    let expected = token.lock().map(|t| t.clone()).unwrap_or_default();
                    let resp = match proto::read_request(&stream) {
                        Err(e) => LinkResponse {
                            ok: false,
                            body: json!({"error": format!("bad request: {e}")}),
                        },
                        Ok(req) if expected.is_empty() || req.token != expected => LinkResponse {
                            ok: false,
                            body: json!({"error": "invalid token"}),
                        },
                        Ok(req) => {
                            let (reply, answer) = channel();
                            if tx.send(ScriptCall { req, reply }).is_err() {
                                LinkResponse {
                                    ok: false,
                                    body: json!({"error": "the run is over"}),
                                }
                            } else {
                                answer.recv_timeout(answer_within).unwrap_or(LinkResponse {
                                    ok: false,
                                    body: json!({"error": "the run is over"}),
                                })
                            }
                        }
                    };
                    let _ = proto::write_response(&stream, &resp);
                });
            }
        });
        Ok(ScriptLink {
            port,
            rx,
            stop,
            token,
        })
    }

    /// Port to pass to [`PythonRun::start`].
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Accept requests carrying `token` (empty: refuse all).
    pub fn set_token(&self, token: &str) {
        if let Ok(mut t) = self.token.lock() {
            *t = token.to_string();
        }
    }

    /// The next script call, waiting at most `wait`.
    pub fn next(&self, wait: Duration) -> Option<ScriptCall> {
        self.rx.recv_timeout(wait).ok()
    }
}

impl Drop for ScriptLink {
    fn drop(&mut self) {
        self.set_token("");
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The script a headless host is running, seen from other threads: who
/// started it and whether someone asked to cancel it.
struct ActiveRun {
    started_by: Actor,
    cancel: Option<String>,
}

/// Lets other threads cancel the script a headless host runs with
/// [`run_blocking`] (`cancel_python` while `ferro-mcp` waits for a script).
/// Clones share the same state.
#[derive(Clone, Default)]
pub struct RunControl {
    active: Arc<Mutex<Option<ActiveRun>>>,
}

impl RunControl {
    /// No run yet.
    pub fn new() -> RunControl {
        RunControl::default()
    }

    /// Whether a script is running.
    pub fn is_running(&self) -> bool {
        self.active.lock().map(|a| a.is_some()).unwrap_or(false)
    }

    /// `cancel_python` from `caller`. Refused with [`NOT_ALLOWED`] unless
    /// [`may_cancel`] allows it; otherwise the run is stopped and rolled back
    /// before this returns (waiting at most 5 s for the running thread):
    /// `{cancelled: true}`. `{cancelled: false}` when no script is running.
    pub fn cancel(&self, caller: &Actor) -> Result<Value, Value> {
        {
            let Ok(mut active) = self.active.lock() else {
                return Err(json!({"error": "the run state is unavailable"}));
            };
            let Some(run) = active.as_mut() else {
                return Ok(json!({"cancelled": false, "reason": "no script is running"}));
            };
            may_cancel(&run.started_by, caller)?;
            run.cancel = Some(match caller.kind {
                ActorKind::User => "the user".to_string(),
                _ => caller.name.clone(),
            });
        }
        let until = Instant::now() + Duration::from_secs(5);
        while self.is_running() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(json!({"cancelled": true}))
    }

    fn begin(&self, started_by: Actor) {
        if let Ok(mut a) = self.active.lock() {
            *a = Some(ActiveRun {
                started_by,
                cancel: None,
            });
        }
    }

    fn requested(&self) -> Option<String> {
        self.active
            .lock()
            .ok()
            .and_then(|a| a.as_ref().and_then(|r| r.cancel.clone()))
    }

    fn end(&self) {
        if let Ok(mut a) = self.active.lock() {
            *a = None;
        }
    }
}

/// Run a script to the end on this thread (headless hosts: `ferro-mcp`
/// without the app, `ferro-cli`). [`APP_ONLY`] requests get the
/// [`headless_answer`] (empty selection, the default value); other requests
/// the run does not answer itself are refused, since there is no app (file
/// tools). Another thread may stop it through `control`
/// ([`RunControl::cancel`]).
pub fn run_blocking(
    session: &mut Session,
    config: &PythonConfig,
    args: &Value,
    ctx: ExecContext,
    propose_agents: bool,
    control: &RunControl,
) -> Result<Value, Value> {
    let run_args = RunArgs::from_tool(args)?;
    run_blocking_as(
        session,
        config,
        &run_args,
        ctx,
        propose_agents,
        control,
        TOOL,
    )
}

/// [`run_blocking`] with parsed arguments and the name of the group the run
/// opens (its undo step and journal entry): a plugin command passes its
/// label (`ferro_mcp::plugins`).
pub fn run_blocking_as(
    session: &mut Session,
    config: &PythonConfig,
    run_args: &RunArgs,
    ctx: ExecContext,
    propose_agents: bool,
    control: &RunControl,
    group_name: &str,
) -> Result<Value, Value> {
    decide(&ctx.actor, propose_agents)?;
    let link = ScriptLink::bind(run_args.timeout + Duration::from_secs(30)).map_err(|e| {
        failure(
            UNAVAILABLE,
            format!("cannot open Ferro Link for the script: {e}"),
        )
    })?;
    let started_by = ctx.actor.clone();
    let mut run = PythonRun::start(session, config, run_args, ctx, link.port(), group_name)?;
    link.set_token(run.token());
    control.begin(started_by);
    let end = loop {
        while let Some(call) = link.next(Duration::from_millis(5)) {
            let tool = call.req.tool.clone();
            let r = run
                .handle(session, &tool, &call.req.args)
                .unwrap_or_else(|| match headless_answer(&tool, &call.req.args) {
                    Some(body) => Ok(body),
                    None => Err(json!({"error": format!("{tool} is not available inside run_python here (no Ferro app)")})),
                });
            let _ = call.reply.send(match r {
                Ok(body) => LinkResponse { ok: true, body },
                Err(body) => LinkResponse { ok: false, body },
            });
        }
        if let Some(by) = control.requested() {
            break run.cancel(&by);
        }
        if let Some(end) = run.poll() {
            break end;
        }
    };
    link.set_token(""); // the token dies with the run
    let r = run.finish(session, end);
    control.end();
    r
}

/// Whether `path` names an interpreter that starts (used by tests to skip
/// gracefully where Python is missing).
pub fn interpreter_works(path: &Path) -> bool {
    Command::new(path)
        .args([
            "-I",
            "-c",
            "import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Server;
    use ferro_core::ops::Actor;

    /// A headless server with the system python3, or `None` (test skipped)
    /// where no Python 3.9+ is installed.
    fn server() -> Option<Server> {
        let config = PythonConfig {
            interpreter: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
            sdk: None,
        };
        if !interpreter_works(&config.interpreter) {
            eprintln!("skipped: no python3 (3.9+) on PATH");
            return None;
        }
        let mut s = Server::headless(Actor::agent("test-agent", "1"));
        s.python = config;
        Some(s)
    }

    fn run(s: &mut Server, code: &str) -> Result<Value, Value> {
        s.call(TOOL, &json!({ "code": code }))
    }

    const BOX: &str =
        "doc.call('create_box', origin=[0, 0, 0], width='4m', depth='3m', height='2.8m')\n";

    #[test]
    fn a_script_commits_as_one_undo_step_and_returns_its_result() {
        let Some(mut s) = server() else { return };
        let code = format!(
            "{BOX}{BOX}w = doc.create_wall([0, 0], ['8m', 0], height='3m')\nprint('made', w.id)\nresult = {{'wall': w.id, 'n': len(doc.elements())}}\n"
        );
        let r = run(&mut s, &code).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["ok"], true);
        assert!(r["stdout"].as_str().unwrap().starts_with("made "), "{r}");
        let n = s.session.doc().len();
        assert!(n >= 3, "{n}");
        assert_eq!(r["result"]["n"], n as u64);
        assert_eq!(r["changed"].as_array().unwrap().len(), n, "{r}");
        assert_eq!(r["failures"], json!([]));
        // One journal entry, with the script and the caller as actor.
        let j = s.session.journal();
        assert_eq!(j.len(), 1);
        assert_eq!(j[0].transaction, TOOL);
        assert_eq!(j[0].scripts, vec![code.clone()]);
        assert_eq!(j[0].commands.len(), 3);
        assert_eq!(j[0].actor.name, "test-agent");
        // One undo undoes all of it.
        s.call("undo", &json!({})).unwrap();
        assert_eq!(s.session.doc().len(), 0);
    }

    #[test]
    fn an_exception_rolls_everything_back() {
        let Some(mut s) = server() else { return };
        let before = s.session.doc().clone();
        let r = run(&mut s, &format!("{BOX}{BOX}raise ValueError('boom')\n")).unwrap_err();
        assert_eq!(r["ok"], false);
        assert_eq!(r["failures"][0]["code"], FAILED);
        let stderr = r["stderr"].as_str().unwrap();
        assert!(stderr.contains("ValueError: boom"), "{stderr}");
        assert!(stderr.contains("<run_python>"), "{stderr}");
        assert!(
            !stderr.contains("_run.py"),
            "the bootstrap frame is hidden: {stderr}"
        );
        assert_eq!(r["changed"], json!([]));
        assert_eq!(s.session.doc(), &before);
        assert!(s.session.journal().is_empty());
        assert!(!s.session.in_group());
        assert!(!s.session.can_undo());
    }

    #[test]
    fn a_failed_tool_call_is_reported_after_the_python_failure() {
        let Some(mut s) = server() else { return };
        let r = run(
            &mut s,
            &format!("{BOX}doc.call('push_pull_face', element=999, face=1, distance=100)\n"),
        )
        .unwrap_err();
        assert_eq!(r["failures"][0]["code"], FAILED);
        assert_eq!(r["failures"][1]["code"], "ferro.model.not_found", "{r}");
        assert!(r["stderr"].as_str().unwrap().contains("FerroError"));
        assert_eq!(s.session.doc().len(), 0);
        // Caught, the same failure does not stop the script.
        let r = run(
            &mut s,
            &format!("{BOX}try:\n    doc.call('push_pull_face', element=999, face=1, distance=100)\nexcept ferro.FerroError as e:\n    print(e.code)\n"),
        )
        .unwrap();
        assert_eq!(r["stdout"], "ferro.model.not_found\n");
        assert_eq!(s.session.doc().len(), 1);
    }

    #[test]
    fn sys_exit_statuses() {
        let Some(mut s) = server() else { return };
        assert!(run(&mut s, &format!("{BOX}import sys\nsys.exit(0)\n")).is_ok());
        assert_eq!(s.session.doc().len(), 1);
        let r = run(&mut s, &format!("{BOX}import sys\nsys.exit(3)\n")).unwrap_err();
        assert_eq!(r["failures"][0]["code"], FAILED);
        assert!(r["error"].as_str().unwrap().contains("exit status 3"));
        // The status reserved for "too old" is not mistaken for it.
        let r = run(&mut s, "import sys\nsys.exit(86)\n").unwrap_err();
        assert_eq!(r["failures"][0]["code"], FAILED);
        assert_eq!(s.session.doc().len(), 1);
    }

    #[test]
    fn a_timeout_kills_the_process_tree_and_rolls_back() {
        let Some(mut s) = server() else { return };
        let started = Instant::now();
        let code = format!(
            "{BOX}import subprocess, sys\np = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\nprint(p.pid, flush=True)\nwhile True:\n    pass\n"
        );
        let r = s
            .call(TOOL, &json!({"code": code, "timeout_s": 1.5}))
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(r["failures"][0]["code"], TIMEOUT, "{r}");
        assert_eq!(s.session.doc().len(), 0);
        assert!(s.session.journal().is_empty());
        #[cfg(target_os = "linux")]
        {
            let pid: u32 = r["stdout"].as_str().unwrap().trim().parse().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            // Gone, or a zombie waiting for init: not running either way.
            let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let running = state
                .rsplit(')')
                .next()
                .and_then(|rest| rest.split_whitespace().next())
                .is_some_and(|st| st != "Z" && st != "X");
            assert!(!running, "grandchild {pid} still running: {state}");
        }
    }

    #[test]
    fn a_missing_interpreter_has_a_code() {
        let mut s = Server::headless(Actor::user("tester"));
        s.python = PythonConfig {
            interpreter: PathBuf::from("/nonexistent/ferro/python3"),
            sdk: None,
        };
        let r = run(&mut s, "print(1)").unwrap_err();
        assert_eq!(r["failures"][0]["code"], UNAVAILABLE);
        assert!(!s.session.in_group());
        assert!(s.session.journal().is_empty());
    }

    #[test]
    fn large_output_is_capped() {
        let Some(mut s) = server() else { return };
        let r = run(
            &mut s,
            "import sys\nsys.stdout.write('x' * 1000000)\nsys.stderr.write('y' * 200000)\n",
        )
        .unwrap();
        let out = r["stdout"].as_str().unwrap();
        assert!(out.len() < OUTPUT_CAP + 200, "{}", out.len());
        assert!(out.starts_with(&"x".repeat(1000)));
        assert!(out.contains("1000000 bytes"), "{}", &out[out.len() - 80..]);
        assert_eq!(r["stdout_truncated"], true);
        assert_eq!(r["stderr_truncated"], true);
    }

    #[test]
    fn a_nested_run_is_refused() {
        let Some(mut s) = server() else { return };
        let r = run(
            &mut s,
            &format!("{BOX}try:\n    tools.run_python(code='pass')\nexcept ferro.FerroError as e:\n    print(e.code)\n"),
        )
        .unwrap();
        assert_eq!(r["stdout"], format!("{NESTED}\n"));
        assert_eq!(s.session.doc().len(), 1);
    }

    #[test]
    fn transaction_blocks_are_savepoints_of_the_run() {
        let Some(mut s) = server() else { return };
        let code = format!(
            "{BOX}try:\n    with doc.transaction('two more'):\n        {BOX}        {BOX}        raise RuntimeError('undo the block')\nexcept RuntimeError:\n    pass\nwith doc.transaction('kept'):\n    {BOX}result = len(doc.elements())\n"
        );
        let r = run(&mut s, &code).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["result"], 2, "{r}");
        assert_eq!(s.session.doc().len(), 2);
        assert_eq!(s.session.journal()[0].commands.len(), 2);
        // ferro.connect() inside a run returns the run's document.
        let r = run(
            &mut s,
            "d = ferro.connect()\nresult = d.link.token == doc.link.token\n",
        )
        .unwrap();
        assert_eq!(r["result"], true);
    }

    #[test]
    fn a_run_inside_an_open_group_is_a_savepoint() {
        // As in the app's Proposal or `ferro-cli run --atomic`.
        let Some(mut s) = server() else { return };
        let ctx = s.ctx();
        s.session.begin_group("AI proposal", ctx);
        s.call(
            "create_box",
            &json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000}),
        )
        .unwrap();
        run(&mut s, &format!("{BOX}raise SystemExit(2)\n")).unwrap_err();
        assert_eq!(s.session.doc().len(), 1, "only the script's work is undone");
        assert!(s.session.in_group(), "the Proposal stays open");
        run(&mut s, BOX).unwrap();
        s.session.end_group();
        assert_eq!(s.session.doc().len(), 2);
        let j = s.session.journal();
        assert_eq!(j.len(), 1);
        assert_eq!(
            j[0].scripts,
            vec![BOX.to_string()],
            "the failed script is not recorded"
        );
    }

    #[test]
    fn a_run_in_a_design_option_changes_only_its_layer() {
        let Some(mut s) = server() else { return };
        let set = s
            .call("create_option_set", &json!({"name": "Roof"}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        s.call("create_design_option", &json!({"set": set}))
            .unwrap();
        let b = s
            .call("create_design_option", &json!({"set": set}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        let shared = s.session.doc().len();
        let undo_steps = s.session.journal().len();
        let code = format!("{BOX}{BOX}");
        let r = s
            .call(TOOL, &json!({"code": code, "in_design_option": b}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.session.doc().len(), shared, "shared model untouched");
        assert_eq!(r["changed"], json!([b]), "only the option's layer changed");
        let l = s.call("list_design_options", &json!({})).unwrap();
        assert_eq!(l["sets"][0]["options"][2]["added"], 2, "{l}");
        assert_eq!(s.session.journal().len(), undo_steps + 1, "one step");
        // A failing script leaves the layer as it was.
        let r = s
            .call(
                TOOL,
                &json!({"code": format!("{BOX}raise ValueError()"), "in_design_option": b}),
            )
            .unwrap_err();
        assert_eq!(r["failures"][0]["code"], FAILED);
        let l = s.call("list_design_options", &json!({})).unwrap();
        assert_eq!(l["sets"][0]["options"][2]["added"], 2, "{l}");
        // One undo removes both boxes from the option.
        s.call("undo", &json!({})).unwrap();
        let l = s.call("list_design_options", &json!({})).unwrap();
        assert_eq!(l["sets"][0]["options"][2]["added"], 0, "{l}");
        // An id that is no option: the first command fails, nothing changes.
        let r = s
            .call(TOOL, &json!({"code": BOX, "in_design_option": 9999}))
            .unwrap_err();
        assert_eq!(
            r["failures"][1]["code"], "ferro.model.not_a_design_option",
            "{r}"
        );
        assert!(RunArgs::from_tool(&json!({"code": "", "in_design_option": "x"})).is_err());
    }

    #[test]
    fn the_token_dies_with_the_run() {
        let Some(mut s) = server() else { return };
        let r = run(
            &mut s,
            "import os\nresult = [os.environ['FERRO_LINK_PORT'], os.environ['FERRO_LINK_TOKEN']]\n",
        )
        .unwrap();
        let port: u16 = r["result"][0].as_str().unwrap().parse().unwrap();
        let token = r["result"][1].as_str().unwrap().to_string();
        let info = proto::LinkInfo {
            port,
            token: token.clone(),
            pid: 0,
            project: String::new(),
            version: String::new(),
        };
        let req = LinkRequest {
            token,
            tool: "create_box".into(),
            args: json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000}),
            agent: None,
            prompt: None,
        };
        // The listener is gone (or, if the port was reused, refuses the token).
        let after = proto::send(&info, &req, Duration::from_secs(2));
        assert!(after.map(|r| !r.ok).unwrap_or(true));
        assert_eq!(s.session.doc().len(), 0);
    }

    #[test]
    fn arguments_decisions_and_timeouts() {
        assert!(RunArgs::from_tool(&json!({})).is_err());
        assert!(RunArgs::from_tool(&json!({"code": 5})).is_err());
        assert!(RunArgs::from_tool(&json!({"code": "", "timeout_s": 0})).is_err());
        assert!(RunArgs::from_tool(&json!({"code": "", "timeout_s": 301})).is_err());
        assert!(RunArgs::from_tool(&json!({"code": "", "timeout_s": "5"})).is_err());
        let a = RunArgs::from_tool(&json!({"code": "pass"})).unwrap();
        assert_eq!(a.timeout, Duration::from_secs(30));
        let a = RunArgs::from_tool(&json!({"code": "pass", "timeout_s": 300})).unwrap();
        assert_eq!(a.timeout, Duration::from_secs(300));
        assert_eq!(
            answer_timeout(TOOL, &json!({"code": "", "timeout_s": 300})),
            Duration::from_secs(330)
        );
        assert_eq!(
            answer_timeout("create_box", &json!({})),
            Duration::from_secs(120)
        );
        assert_eq!(
            answer_timeout(TOOL, &json!({"code": ""})),
            Duration::from_secs(120)
        );

        assert_eq!(decide(&Actor::user("u"), true), Ok(RunMode::Commit));
        assert_eq!(decide(&Actor::agent("a", "m"), true), Ok(RunMode::Propose));
        assert_eq!(decide(&Actor::agent("a", "m"), false), Ok(RunMode::Commit));
        let plugin = Actor {
            kind: ActorKind::Plugin { id: "p".into() },
            name: "p".into(),
        };
        // ADR 0009: a plugin has the user's rights, whatever the agents'
        // autonomy.
        assert_eq!(decide(&plugin, true), Ok(RunMode::Commit));
        assert_eq!(decide(&plugin, false), Ok(RunMode::Commit));
    }

    /// Wait until `control` runs a script, then cancel it as `caller` after
    /// `delay`, on another thread (as a second MCP client would).
    fn cancel_later(
        control: &RunControl,
        caller: Actor,
        delay: Duration,
    ) -> std::thread::JoinHandle<Result<Value, Value>> {
        let control = control.clone();
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(20);
            while !control.is_running() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(delay);
            control.cancel(&caller)
        })
    }

    /// Whether process `pid` still runs (Linux; gone or zombie = not running).
    #[cfg(target_os = "linux")]
    fn process_runs(pid: u32) -> bool {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        state
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|st| st != "Z" && st != "X")
    }

    #[test]
    fn cancel_stops_a_busy_script_rolls_back_and_frees_the_session() {
        let Some(mut s) = server() else { return };
        let canceller = cancel_later(
            &s.runs,
            Actor::agent("test-agent", "2"),
            Duration::from_millis(400),
        );
        let started = Instant::now();
        let r = s
            .call(
                TOOL,
                &json!({"code": format!("{BOX}print('working', flush=True)\nwhile True:\n    pass\n"), "timeout_s": 60}),
            )
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "stopped early");
        assert_eq!(r["failures"][0]["code"], CANCELLED, "{r}");
        assert!(r["error"].as_str().unwrap().contains("test-agent"), "{r}");
        assert_eq!(r["stdout"], "working\n", "the output so far is kept");
        assert_eq!(r["changed"], json!([]));
        let c = canceller.join().unwrap().unwrap();
        assert_eq!(c["cancelled"], true);
        // Nothing changed, no history, and the next command runs.
        assert_eq!(s.session.doc().len(), 0);
        assert!(s.session.journal().is_empty());
        assert!(!s.session.in_group());
        assert!(!s.runs.is_running());
        s.call(
            "create_box",
            &json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000}),
        )
        .unwrap();
        assert_eq!(s.session.doc().len(), 1);
        assert_eq!(s.session.journal().len(), 1);
        // With no script running, cancel says so.
        let none = s.call(CANCEL_TOOL, &json!({})).unwrap();
        assert_eq!(none["cancelled"], false);
    }

    #[test]
    fn only_the_user_or_the_starter_may_cancel() {
        let Some(mut s) = server() else { return };
        let other = cancel_later(
            &s.runs,
            Actor::agent("other-agent", "1"),
            Duration::from_millis(200),
        );
        let r = s
            .call(
                TOOL,
                &json!({"code": format!("{BOX}import time\ntime.sleep(1.0)\n"), "timeout_s": 30}),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r["ok"], true, "the script ran to the end");
        let refused = other.join().unwrap().unwrap_err();
        assert_eq!(refused["failures"][0]["code"], NOT_ALLOWED, "{refused}");
        assert_eq!(s.session.doc().len(), 1);
        // The user may always.
        let user = cancel_later(&s.runs, Actor::user("me"), Duration::from_millis(200));
        let r = s
            .call(TOOL, &json!({"code": "while True:\n    pass\n"}))
            .unwrap_err();
        assert_eq!(r["failures"][0]["code"], CANCELLED);
        assert!(r["error"].as_str().unwrap().contains("the user"), "{r}");
        assert_eq!(user.join().unwrap().unwrap()["cancelled"], true);

        let agent = Actor::agent("a", "1");
        assert!(
            may_cancel(&agent, &Actor::agent("a", "2")).is_ok(),
            "any version"
        );
        assert!(may_cancel(&agent, &Actor::user("u")).is_ok());
        assert!(may_cancel(&Actor::user("u"), &agent).is_err());
        let plugin = Actor {
            kind: ActorKind::Plugin { id: "a".into() },
            name: "a".into(),
        };
        assert!(
            may_cancel(&agent, &plugin).is_err(),
            "same name, other kind"
        );
    }

    #[test]
    fn cancel_kills_a_script_that_ignores_sigterm_and_its_children() {
        let Some(mut s) = server() else { return };
        let canceller = cancel_later(&s.runs, Actor::user("me"), Duration::from_millis(600));
        let code = "import signal, subprocess, sys\n\
try:\n    signal.signal(signal.SIGTERM, signal.SIG_IGN)\nexcept (AttributeError, ValueError):\n    pass\n\
p = subprocess.Popen([sys.executable, '-c', 'import signal, time\\ntry:\\n    signal.signal(signal.SIGTERM, signal.SIG_IGN)\\nexcept Exception:\\n    pass\\ntime.sleep(60)'])\n\
print(p.pid, flush=True)\nimport time\nwhile True:\n    time.sleep(0.05)\n";
        let r = s
            .call(TOOL, &json!({"code": code, "timeout_s": 60}))
            .unwrap_err();
        assert_eq!(r["failures"][0]["code"], CANCELLED, "{r}");
        canceller.join().unwrap().unwrap();
        #[cfg(target_os = "linux")]
        {
            let pid: u32 = r["stdout"].as_str().unwrap().trim().parse().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert!(!process_runs(pid), "child {pid} still running");
        }
    }

    #[test]
    fn a_script_targets_shared_explicitly_or_follows_its_run() {
        let Some(mut s) = server() else { return };
        let set = s
            .call("create_option_set", &json!({"name": "Roof"}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        let b = s
            .call("create_design_option", &json!({"set": set}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        let shared = s.session.doc().len();
        // The run's option, but one call says "shared": that box lands in
        // the shared model.
        let code = format!(
            "{BOX}doc.call('create_box', origin=[0, 0, 0], width='1m', depth='1m', height='1m', in_design_option='shared')\n"
        );
        let r = s
            .call(TOOL, &json!({"code": code, "in_design_option": b}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.session.doc().len(), shared + 1, "{r}");
        let l = s.call("list_design_options", &json!({})).unwrap();
        assert_eq!(l["sets"][0]["options"][1]["added"], 1, "{l}");
        // The whole run "shared".
        s.call(TOOL, &json!({"code": BOX, "in_design_option": "shared"}))
            .unwrap();
        assert_eq!(s.session.doc().len(), shared + 2);
        let a = RunArgs::from_tool(&json!({"code": "", "in_design_option": "shared"})).unwrap();
        assert_eq!(a.in_design_option, Some(None));
        let a = RunArgs::from_tool(&json!({"code": "", "in_design_option": 4})).unwrap();
        assert_eq!(a.in_design_option, Some(Some(ElementId(4))));
        let a = RunArgs::from_tool(&json!({"code": ""})).unwrap();
        assert_eq!(a.in_design_option, None);
    }

    #[test]
    fn a_run_reports_where_its_commands_landed() {
        let Some(mut s) = server() else { return };
        let set = s
            .call("create_option_set", &json!({"name": "Roof"}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        let b = s
            .call("create_design_option", &json!({"set": set}))
            .unwrap()["result"]["element"]
            .as_u64()
            .unwrap();
        let link = ScriptLink::bind(Duration::from_secs(30)).unwrap();
        let args = RunArgs::from_tool(&json!({"code": "", "in_design_option": b})).unwrap();
        let mut run = PythonRun::start(
            &mut s.session,
            &s.python,
            &args,
            ExecContext::user(),
            link.port(),
            TOOL,
        )
        .unwrap();
        let bx = json!({"origin":[0,0,0],"width":1000,"depth":1000,"height":1000});
        run.handle(&mut s.session, "create_box", &bx)
            .unwrap()
            .unwrap();
        run.handle(&mut s.session, "create_box", &bx)
            .unwrap()
            .unwrap();
        let mut sh = bx.clone();
        sh["in_design_option"] = json!("shared");
        run.handle(&mut s.session, "create_box", &sh)
            .unwrap()
            .unwrap();
        run.handle(&mut s.session, "list_elements", &json!({}))
            .unwrap()
            .unwrap();
        assert_eq!(run.targets(), &[Some(ElementId(b)), None]);
        let refused = run.handle(&mut s.session, CANCEL_TOOL, &json!({})).unwrap();
        assert!(refused.is_err(), "a script does not cancel itself");
        let end = run.cancel("the user");
        assert_eq!(end, RunEnd::Cancelled);
        let r = run.finish(&mut s.session, end).unwrap_err();
        assert_eq!(r["failures"][0]["code"], CANCELLED);
        assert_eq!(s.session.doc().len(), 3, "set and its two options only");
    }

    #[test]
    fn the_bundled_sdk_has_every_module() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/ferro");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".py"))
            .collect();
        on_disk.sort();
        let bundled: Vec<String> = SDK_FILES.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(bundled, on_disk, "add new SDK modules to SDK_FILES");
    }
}
