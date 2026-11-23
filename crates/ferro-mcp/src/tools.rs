//! MCP tool host. Tool schemas, unit conversion and command dispatch live in
//! `ferro-tools` (shared with the app's Ferro Link server). When the Ferro app
//! is open, calls are forwarded to it over Ferro Link so the agent edits the
//! model the user is looking at (D-65); otherwise ferro-mcp keeps its own
//! headless session with file tools.

use ferro_core::ops::{Actor, ExecContext};
use ferro_core::{Document, Session};
use ferro_kernel::SolidcoreKernel;
use ferro_tools::link::{self, AgentInfo, LinkRequest};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// All tools offered over MCP.
pub fn list() -> Vec<Value> {
    let mut v = ferro_tools::list();
    v.extend(ferro_tools::file_tools());
    v.push(json!({"name": "link_status", "description": "Whether the Ferro app is open and linked. When linked, every tool acts on the model open in the app; changes appear there as a proposal the user accepts or rejects.", "inputSchema": {"type": "object", "properties": {}, "required": []}}));
    v
}

/// The tool list as pretty JSON, `{"tools": [...]}` exactly as MCP
/// `tools/list` returns it. Keys are sorted, so the output is deterministic.
pub fn tools_json() -> String {
    let v = json!({ "tools": list() });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
}

/// Server state: one session (the open project) plus its file path.
pub struct Server {
    /// The open project, its undo history and journal.
    pub session: Session,
    /// File the project was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// Actor recorded in the journal for every call.
    pub agent: Actor,
    /// Forward to a running app when one is linked (off in tests or with
    /// `FERRO_LINK=off`).
    pub use_link: bool,
}

impl Server {
    /// A server with an empty "Untitled" project. Forwards to a linked app
    /// unless `FERRO_LINK=off`.
    pub fn new() -> Server {
        Server {
            session: Session::new(Document::new("Untitled"), Box::new(ferro_io::SystemIds))
                .with_kernel(Arc::new(SolidcoreKernel::new())),
            path: None,
            agent: Actor::agent("mcp-client", "unknown"),
            use_link: std::env::var("FERRO_LINK")
                .map(|v| v != "off")
                .unwrap_or(true),
        }
    }

    /// A headless server that never forwards to the app (scripts, CI).
    pub fn headless(agent: Actor) -> Server {
        Server {
            agent,
            use_link: false,
            ..Server::new()
        }
    }

    /// Execution context for the next call: this server's actor, now.
    pub fn ctx(&self) -> ExecContext {
        ExecContext {
            actor: self.agent.clone(),
            prompt: None,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    fn agent_info(&self) -> AgentInfo {
        let version = match &self.agent.kind {
            ferro_core::ops::ActorKind::Agent { model } => model.clone(),
            _ => String::new(),
        };
        AgentInfo {
            name: self.agent.name.clone(),
            version,
        }
    }

    /// Forward to the linked app. `None` when no app is reachable.
    fn forward(&self, name: &str, args: &Value) -> Option<Result<Value, Value>> {
        if !self.use_link {
            return None;
        }
        let info = link::read_info()?;
        let req = LinkRequest {
            token: info.token.clone(),
            tool: name.into(),
            args: args.clone(),
            agent: Some(self.agent_info()),
            prompt: None,
        };
        match link::send(&info, &req, Duration::from_secs(120)) {
            Ok(r) if r.ok => Some(Ok(r.body)),
            Ok(r) => Some(Err(r.body)),
            Err(_) => None, // stale link file: fall back to the local session
        }
    }

    /// Run a tool. Ok = JSON result, Err = (message, failures JSON).
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Value, Value> {
        let err = |m: String| Err(json!({"error": m}));
        if name == "link_status" {
            let linked = self.use_link
                && link::read_info().is_some_and(|i| {
                    std::net::TcpStream::connect_timeout(
                        &std::net::SocketAddr::from(([127, 0, 0, 1], i.port)),
                        Duration::from_millis(300),
                    )
                    .is_ok()
                });
            let project = link::read_info().map(|i| i.project);
            return Ok(json!({"linked": linked, "project": if linked { project } else { None }}));
        }
        if let Some(r) = self.forward(name, args) {
            return r;
        }
        let ctx = self.ctx();
        if let Some(r) = ferro_tools::call(&mut self.session, &ctx, name, args) {
            return r;
        }
        let path_arg = || args.get("path").and_then(Value::as_str).map(PathBuf::from);
        let io_err = |e: ferro_io::IoError| Err(io_failure(&e));
        let write =
            |p: &PathBuf, bytes: &[u8]| std::fs::write(p, bytes).map_err(ferro_io::IoError::from);
        match name {
            "new_document" => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled");
                self.session.replace_document(Document::new(name));
                self.path = None;
                Ok(json!({"ok": true}))
            }
            "open_document" => {
                let Some(p) = path_arg() else {
                    return err("missing path".into());
                };
                let doc = if p.to_string_lossy().ends_with(".json") {
                    std::fs::read_to_string(&p)
                        .map_err(ferro_io::IoError::from)
                        .and_then(|t| ferro_io::document_from_json(&t))
                } else {
                    ferro_io::load_fro(&p).map(|(d, _)| d)
                };
                match doc {
                    Ok(d) => {
                        self.session.replace_document(d);
                        self.path = Some(p);
                        Ok(ferro_tools::summary(self.session.doc(), None))
                    }
                    Err(e) => io_err(e),
                }
            }
            "save_document" => {
                let Some(p) = path_arg().or_else(|| self.path.clone()) else {
                    return err("no path: give one, or open a file first".into());
                };
                let r = if p.to_string_lossy().ends_with(".json") {
                    ferro_io::document_to_json(self.session.doc())
                        .and_then(|t| write(&p, t.as_bytes()))
                } else {
                    ferro_io::save_fro(&p, self.session.doc(), self.session.journal())
                };
                match r {
                    Ok(()) => {
                        self.path = Some(p.clone());
                        Ok(json!({"saved": p}))
                    }
                    Err(e) => io_err(e),
                }
            }
            "export_ifc" | "export_obj" | "export_glb" => {
                let Some(p) = path_arg() else {
                    return err("missing path".into());
                };
                let doc = self.session.doc();
                let bytes = match name {
                    "export_ifc" => ferro_io::export_ifc(doc).map(String::into_bytes),
                    "export_obj" => ferro_io::export_obj(doc).map(String::into_bytes),
                    _ => ferro_io::export_glb(doc),
                };
                match bytes.and_then(|b| write(&p, &b)) {
                    Ok(()) => Ok(json!({"exported": p})),
                    Err(e) => io_err(e),
                }
            }
            other => err(format!("unknown tool: {other}")),
        }
    }
}

/// A file error as a tool error: `{"error", "failures": [{code: "ferro.io.*"}]}`.
pub fn io_failure(e: &ferro_io::IoError) -> Value {
    json!({
        "error": e.message,
        "failures": [{"severity": "error", "code": e.code, "message": e.message, "elements": []}],
    })
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
