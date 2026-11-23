//! Ferro Link server (D-65): agents (ferro-mcp, the in-app chat) send tool
//! calls; they are executed on the UI thread against the open model so the
//! user sees every change and can accept or reject it.

use eframe::egui;
use ferro_tools::link::{self as proto, LinkInfo, LinkRequest, LinkResponse};
use serde_json::json;
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver, Sender};
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
}

/// How long a client waits for the UI to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

impl LinkServer {
    /// Start listening on `127.0.0.1` (random port) and publish `link.json`.
    /// When that fails the in-app chat still works through [`Self::sender`].
    pub fn start(project: &str, repaint: egui::Context) -> LinkServer {
        let (tx, rx) = channel::<Pending>();
        let mut server = LinkServer { rx, tx, info: None };
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
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (token, queue, repaint) = (token.clone(), queue.clone(), repaint.clone());
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                    let resp = match proto::read_request(&stream) {
                        Err(e) => LinkResponse {
                            ok: false,
                            body: json!({"error": format!("bad request: {e}")}),
                        },
                        Ok(req) if req.token != token => LinkResponse {
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

/// Hand a request to the UI thread and wait for its answer.
pub fn ask_ui(queue: &Sender<Pending>, repaint: &egui::Context, req: LinkRequest) -> LinkResponse {
    let (reply, answer) = channel();
    if queue.send(Pending { req, reply }).is_err() {
        return LinkResponse {
            ok: false,
            body: json!({"error": "Ferro is closing"}),
        };
    }
    repaint.request_repaint();
    answer.recv_timeout(ANSWER_TIMEOUT).unwrap_or(LinkResponse {
        ok: false,
        body: json!({"error": "Ferro did not answer in time"}),
    })
}
