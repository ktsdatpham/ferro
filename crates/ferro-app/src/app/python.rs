//! `run_python` in the app (ADR 0005 §5): the script runs out of process and
//! calls back over Ferro Link with its one-time token; its calls are answered
//! on the UI thread, inside the run's group, between frames. Nothing here
//! waits for the script: [`FerroApp::poll_python`] checks it once per frame.
//!
//! While a script runs, the user's own edits and other clients' changes are
//! refused (`ferro.python.busy` for clients) so nothing can slip into, or
//! commit, a half-finished group; read-only tools keep working.

use super::{Autonomy, FerroApp, Proposal};
use eframe::egui;
use ferro_core::AnyCommand;
use ferro_mcp::python::{self, PythonRun, RunArgs, RunMode};
use ferro_tools::link::{LinkRequest, LinkResponse};
use serde_json::{json, Value};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// A script started by an agent or the chat, waiting to finish.
pub(super) struct ScriptRun {
    run: PythonRun,
    /// Where the `run_python` answer goes once the script has ended.
    reply: Sender<LinkResponse>,
    agent: String,
}

fn answer(r: Result<Value, Value>) -> LinkResponse {
    match r {
        Ok(body) => LinkResponse { ok: true, body },
        Err(body) => LinkResponse { ok: false, body },
    }
}

/// Tools another client may not call while a script runs: anything that
/// changes the document or its history.
fn changes_the_document(tool: &str) -> bool {
    AnyCommand::NAMES.contains(&tool)
        || matches!(
            tool,
            "revit_import" | "undo" | "redo" | "save_document" | python::TOOL
        )
}

impl FerroApp {
    /// True (and the user is told) while a script runs: user edits wait.
    pub(super) fn script_running(&mut self) -> bool {
        if self.python.is_some() {
            self.say(
                "A Python script is running; try again when it has finished",
                true,
            );
            return true;
        }
        false
    }

    /// Whether `token` is the running script's one-time token.
    pub(super) fn is_script_token(&self, token: &str) -> bool {
        self.python
            .as_ref()
            .is_some_and(|s| !token.is_empty() && s.run.token() == token)
    }

    /// The answer for another client's call while a script runs, when the
    /// call must wait (`ferro.python.busy`); `None` when it may go ahead.
    pub(super) fn refuse_while_script(&self, tool: &str) -> Option<LinkResponse> {
        if self.python.is_none() || !changes_the_document(tool) {
            return None;
        }
        Some(answer(Err(python::failure(
            python::BUSY,
            format!("a Python script is running; call {tool} again when it has finished"),
        ))))
    }

    /// Start `run_python` for an agent (or the chat). The answer is sent
    /// when the script ends; errors before that are sent at once.
    pub(super) fn start_python(&mut self, req: LinkRequest, reply: Sender<LinkResponse>) {
        let (info, ctx) = Self::agent_ctx(&req);
        let refuse = |body: Value| {
            let _ = reply.send(answer(Err(body)));
        };
        if self.python.is_some() {
            return refuse(python::failure(
                python::BUSY,
                "a Python script is already running",
            ));
        }
        let mut args = match RunArgs::from_tool(&req.args) {
            Ok(a) => a,
            Err(body) => return refuse(body),
        };
        // While the user edits a design option, a script lands in it (as the
        // user's own commands do), unless the caller named an option.
        if args.in_design_option.is_none() {
            args.in_design_option = self.editing_option();
        }
        let mode = match python::decide(&ctx.actor, self.autonomy == Autonomy::Propose) {
            Ok(m) => m,
            Err(body) => return refuse(body),
        };
        let Some(port) = self.link.port() else {
            return refuse(python::failure(
                python::UNAVAILABLE,
                "Ferro Link is not listening, so a script cannot call back",
            ));
        };
        // Agents propose (D-26): the script becomes part of their Proposal.
        let opened = mode == RunMode::Propose && self.proposal.is_none();
        if opened {
            self.session
                .begin_group(format!("AI proposal ({})", info.name), ctx.clone());
            self.proposal = Some(Proposal {
                agent: info.name.clone(),
                steps: 0,
            });
        }
        match PythonRun::start(
            &mut self.session,
            &self.python_config,
            &args,
            ctx,
            port,
            python::TOOL,
        ) {
            Ok(run) => {
                self.link.set_run_token(Some(run.token().to_string()));
                self.say(format!("{} is running a Python script…", info.name), false);
                self.python = Some(ScriptRun {
                    run,
                    reply,
                    agent: info.name,
                });
            }
            Err(body) => {
                if opened {
                    self.session.cancel_group();
                    self.proposal = None;
                }
                refuse(body);
            }
        }
    }

    /// Answer one call from the running script (its token matched).
    pub(super) fn script_call(&mut self, req: LinkRequest) -> LinkResponse {
        let handled = match self.python.as_mut() {
            Some(s) => s.run.handle(&mut self.session, &req.tool, &req.args),
            None => Some(Err(json!({"error": "the script's run is over"}))),
        };
        match handled {
            Some(r) => answer(r),
            // App-only tools: the selection is UI state, safe to share.
            None if matches!(req.tool.as_str(), "get_selection" | "select_elements") => {
                self.agent_call(req)
            }
            None => answer(Err(json!({"error": format!(
                "{} is not available inside run_python",
                req.tool
            )}))),
        }
    }

    /// Once per frame: finish the script when it has ended (or timed out),
    /// commit or roll back, answer the caller and retire the token.
    pub(super) fn poll_python(&mut self, ctx: &egui::Context) {
        let ended = self.python.as_mut().and_then(|s| s.run.poll());
        let Some(end) = ended else {
            if self.python.is_some() {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            return;
        };
        let Some(script) = self.python.take() else {
            return;
        };
        self.link.set_run_token(None); // the token dies with the run
        let r = script.run.finish(&mut self.session, end);
        let code = r
            .as_ref()
            .err()
            .and_then(|b| b["failures"][0]["code"].as_str())
            .unwrap_or("")
            .to_string();
        crate::report::record(if r.is_ok() {
            "agent run_python ok".to_string()
        } else {
            format!("agent run_python failed {code}")
        });
        match (&r, &mut self.proposal) {
            (Ok(_), Some(p)) => {
                p.steps += 1;
                self.say(format!("{} proposes: run_python", script.agent), false);
            }
            (Ok(_), None) => {
                self.say(format!("{}: run_python", script.agent), false);
                self.autosave();
            }
            (Err(_), proposal) => {
                if proposal.as_ref().is_some_and(|p| p.steps == 0) {
                    self.session.cancel_group();
                    self.proposal = None;
                }
                self.selection.retain(|id| self.session.doc().contains(*id));
                self.say(
                    format!(
                        "{}'s Python script failed ({code}); nothing was changed",
                        script.agent
                    ),
                    true,
                );
            }
        }
        let _ = script.reply.send(answer(r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_clients_wait_only_for_changes() {
        assert!(changes_the_document("create_wall"));
        assert!(changes_the_document("undo"));
        assert!(changes_the_document("run_python"));
        assert!(changes_the_document("revit_import"));
        assert!(!changes_the_document("list_elements"));
        assert!(!changes_the_document("get_element"));
        assert!(!changes_the_document("link_status"));
    }
}
