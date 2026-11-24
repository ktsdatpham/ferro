//! Ferro Link server (D-65): agents (ferro-mcp, the in-app chat) send tool
//! calls; they are executed on the UI thread against the open model so the
//! user sees every change and can accept or reject it.

use eframe::egui;
use ferro_tools::link::{self as proto, LinkInfo, LinkRequest, LinkResponse};
use serde_json::json;
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A tool call waiting for the UI thread.
pub struct Pending {
    pub req: LinkRequest,
    pub reply: Sender<LinkResponse>,
}

/// Listening server plus the queue of calls for the UI thread.
pub struct LinkServer {
    rx: Receiver<Pending>,
    tx: Sender<Pending>,
    info: Option<LinkInfo>,
    /// One-time token of the running `run_python` script, accepted besides
    /// the token in `link.json` while the script runs.
    run_token: Arc<Mutex<Option<String>>>,
}

impl LinkServer {
    /// Start listening on `127.0.0.1` (random port) and publish `link.json`.
    /// When that fails the in-app chat still works through [`Self::sender`].
    pub fn start(project: &str, repaint: egui::Context) -> LinkServer {
        let (tx, rx) = channel::<Pending>();
        let mut server = LinkServer {
            rx,
            tx,
            info: None,
            run_token: Arc::new(Mutex::new(None)),
        };
        let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
            return server;
        };
        let Ok(addr) = listener.local_addr() else {
            return server;
        };
        let info = LinkInfo {
            port: addr.port(),
            token: uuid::Uuid::new_v4().simple().to_string(),
            pid: std::process::id(),
            project: project.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        if proto::write_info(&info).is_err() {
            return server;
        }
        let token = info.token.clone();
        let queue = server.tx.clone();
        let run_token = Arc::clone(&server.run_token);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (token, queue, repaint) = (token.clone(), queue.clone(), repaint.clone());
                let run_token = Arc::clone(&run_token);
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                    let resp = match proto::read_request(&stream) {
                        Err(e) => LinkResponse {
                            ok: false,
                            body: json!({"error": format!("bad request: {e}")}),
                        },
                        Ok(req) if !accepted(&req.token, &token, &run_token) => LinkResponse {
                            ok: false,
                            body: json!({"error": "invalid token"}),
                        },
                        Ok(req) => ask_ui(&queue, &repaint, req),
                    };
                    let _ = proto::write_response(&stream, &resp);
                });
            }
        });
        server.info = Some(info);
        server
    }

    /// Queue for in-process clients (the chat panel).
    pub fn sender(&self) -> Sender<Pending> {
        self.tx.clone()
    }

    /// Port, when listening.
    pub fn port(&self) -> Option<u16> {
        self.info.as_ref().map(|i| i.port)
    }

    /// Accept `token` (a running script's one-time token) until called with
    /// `None`.
    pub fn set_run_token(&self, token: Option<String>) {
        if let Ok(mut t) = self.run_token.lock() {
            *t = token;
        }
    }

    /// Calls waiting for the UI thread.
    pub fn drain(&self) -> Vec<Pending> {
        self.rx.try_iter().collect()
    }

    /// Keep `link.json` in sync with the open project's name.
    pub fn set_project(&mut self, name: &str) {
        if let Some(info) = &mut self.info {
            if info.project != name {
                info.project = name.to_string();
                let _ = proto::write_info(info);
            }
        }
    }
}

impl Drop for LinkServer {
    fn drop(&mut self) {
        if let Some(info) = &self.info {
            proto::remove_info(&info.token);
        }
    }
}

/// The token in `link.json`, or the running script's one-time token.
fn accepted(given: &str, token: &str, run_token: &Mutex<Option<String>>) -> bool {
    if given == token {
        return true;
    }
    run_token
        .lock()
        .map(|t| t.as_deref().is_some_and(|t| !t.is_empty() && t == given))
        .unwrap_or(false)
}

/// Hand a request to the UI thread and wait for its answer (120 s, or the
/// script's time limit plus a margin for `run_python`).
pub fn ask_ui(queue: &Sender<Pending>, repaint: &egui::Context, req: LinkRequest) -> LinkResponse {
    let timeout = ferro_mcp::python::answer_timeout(&req.tool, &req.args);
    let (reply, answer) = channel();
    if queue.send(Pending { req, reply }).is_err() {
        return LinkResponse {
            ok: false,
            body: json!({"error": "Ferro is closing"}),
        };
    }
    repaint.request_repaint();
    answer.recv_timeout(timeout).unwrap_or(LinkResponse {
        ok: false,
        body: json!({"error": "Ferro did not answer in time"}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_token_is_accepted_only_while_set() {
        let run = Mutex::new(None);
        assert!(accepted("main", "main", &run));
        assert!(!accepted("once", "main", &run));
        assert!(!accepted("", "main", &run));
        *run.lock().unwrap() = Some("once".to_string());
        assert!(accepted("once", "main", &run));
        assert!(!accepted("", "main", &run));
        *run.lock().unwrap() = None;
        assert!(!accepted("once", "main", &run));
    }
}
