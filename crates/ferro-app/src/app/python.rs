//! `run_python` in the app (ADR 0005 §5): the script runs out of process and
//! calls back over Ferro Link with its one-time token; its calls are answered
//! on the UI thread, inside the run's group, between frames. Nothing here
//! waits for the script: [`FerroApp::poll_python`] checks it once per frame.
//!
//! While a script runs, other clients' changes are refused
//! (`ferro.python.busy`) so nothing can slip into, or commit, a half-finished
//! group; read-only tools keep working. The **user** always has priority
//! (M5-14.7): the status bar shows the running script with a Stop button
//! (Esc twice within a second also stops it), and an edit the user tries
//! meanwhile is held ([`UserAction`]) while a dialog asks whether to stop the
//! script and do it. A held action is never dropped: it runs once the script
//! has ended, whether it was stopped or finished on its own.

use super::{Autonomy, FerroApp, Proposal};
use eframe::egui::{self, Color32, RichText};
use ferro_core::{AnyCommand, ElementId};
use ferro_mcp::python::{self, PythonRun, RunArgs, RunEnd, RunMode};
use ferro_tools::link::{LinkRequest, LinkResponse};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// A script started by an agent, the chat or a plugin command, waiting to
/// finish.
pub(super) struct ScriptRun {
    pub(super) run: PythonRun,
    /// Where the `run_python` answer goes once the script has ended (`None`
    /// for a plugin command the user clicked: the status line reports it).
    pub(super) reply: Option<Sender<LinkResponse>>,
    /// Who started it: the agent's name, or the plugin's name.
    pub(super) agent: String,
    /// The plugin command's label when a plugin command runs (ADR 0009).
    pub(super) plugin: Option<String>,
}

/// An edit the user tried while a script ran, done once the script has
/// ended (M5-14.7).
#[derive(Debug, Clone)]
pub(super) enum UserAction {
    /// A command from a tool, the palette or a panel (`FerroApp::run`).
    Run(AnyCommand),
    /// A view command (`FerroApp::exec_view`).
    ExecView(AnyCommand),
    /// A value typed in the status-bar box.
    Value(String),
    Undo,
    Redo,
    Save,
    SaveAs,
    NewDocument,
    Open(PathBuf),
    AcceptProposal,
    RejectProposal,
}

/// The user's answer in the dialog "A script is running…".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    /// Stop the script (roll it back), then do the held actions.
    StopAndContinue,
    /// Let the script run; the held actions run when it ends.
    KeepWaiting,
}

/// What the app does after the user chose.
#[derive(Debug)]
pub(super) struct Plan {
    /// Cancel the running script (its end then runs the held actions).
    pub stop: bool,
    /// Actions to do now (the script had already ended).
    pub perform: Vec<UserAction>,
}

/// Actions held while a script runs, the priority dialog and the double-Esc
/// stop. Pure state, so its rules are tested without a window.
#[derive(Default)]
pub(super) struct ScriptWait {
    queued: Vec<UserAction>,
    asking: bool,
    last_esc: Option<Instant>,
}

/// Two presses of Esc closer than this stop the script.
pub(super) const DOUBLE_ESC: Duration = Duration::from_secs(1);

impl ScriptWait {
    /// Hold `action` and ask the user.
    pub fn hold(&mut self, action: UserAction) {
        self.queued.push(action);
        self.asking = true;
    }

    /// Whether the dialog is shown.
    pub fn asking(&self) -> bool {
        self.asking
    }

    /// How many actions wait.
    pub fn waiting(&self) -> usize {
        self.queued.len()
    }

    /// The user chose in the dialog; `running` says whether the script still
    /// runs. Nothing is dropped: stopping hands the held actions to the end
    /// of the run ([`Self::script_ended`]); a script that ended meanwhile
    /// hands them over at once; waiting keeps them.
    pub fn choose(&mut self, choice: Choice, running: bool) -> Plan {
        self.asking = false;
        if !running {
            return Plan {
                stop: false,
                perform: std::mem::take(&mut self.queued),
            };
        }
        Plan {
            stop: choice == Choice::StopAndContinue,
            perform: Vec::new(),
        }
    }

    /// The script ended (finished, failed, timed out or stopped): close the
    /// dialog and hand over the held actions, oldest first.
    pub fn script_ended(&mut self) -> Vec<UserAction> {
        self.asking = false;
        self.last_esc = None;
        std::mem::take(&mut self.queued)
    }

    /// Esc pressed at `now` while a script runs: true on the second press
    /// within [`DOUBLE_ESC`] (stop the script).
    pub fn escape(&mut self, now: Instant) -> bool {
        match self.last_esc {
            Some(t) if now.saturating_duration_since(t) <= DOUBLE_ESC => {
                self.last_esc = None;
                true
            }
            _ => {
                self.last_esc = Some(now);
                false
            }
        }
    }
}

/// Where a proposal's changes land, for its bar (ADR 0006): "into option:
/// <name>" per design option and "shared model", in first-use order.
pub(super) fn target_label(
    targets: &[Option<ElementId>],
    name: impl Fn(ElementId) -> String,
) -> String {
    if targets.is_empty() {
        return "shared model".into();
    }
    targets
        .iter()
        .map(|t| match t {
            Some(o) => format!("into option: {}", name(*o)),
            None => "shared model".into(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Add `target` to a proposal's targets unless it is there.
pub(super) fn note_target(targets: &mut Vec<Option<ElementId>>, target: Option<ElementId>) {
    if !targets.contains(&target) {
        targets.push(target);
    }
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
    /// While a script runs, hold `action` for later and ask the user whether
    /// to stop the script; returns true when held (the caller stops there).
    pub(super) fn hold_for_script(&mut self, action: UserAction) -> bool {
        if self.python.is_none() {
            return false;
        }
        self.script_wait.hold(action);
        true
    }

    /// Do an action that was held for a script.
    fn perform(&mut self, action: UserAction) {
        match action {
            UserAction::Run(c) => {
                self.run(c);
            }
            UserAction::ExecView(c) => {
                self.exec_view(c);
            }
            UserAction::Value(text) => {
                self.vcb = text;
                self.apply_vcb();
            }
            UserAction::Undo => self.undo(),
            UserAction::Redo => self.redo(),
            UserAction::Save => self.save(),
            UserAction::SaveAs => self.save_as(),
            UserAction::NewDocument => self.new_document(),
            UserAction::Open(p) => self.open_path(p),
            UserAction::AcceptProposal => self.accept_proposal(),
            UserAction::RejectProposal => self.reject_proposal(),
        }
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
        // One rule with single commands (ADR 0006): while the user edits a
        // design option, a script lands in it unless the caller named a
        // target (an option, or "shared").
        if args.in_design_option.is_none() {
            args.in_design_option = Some(self.editing_option());
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
                targets: Vec::new(),
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
                    reply: Some(reply),
                    agent: info.name,
                    plugin: None,
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

    /// `cancel_python` from a Ferro Link client: allowed for the client that
    /// started the script (the user stops it with the Stop button or Esc).
    pub(super) fn cancel_from_link(&mut self, req: &LinkRequest) -> LinkResponse {
        let (info, ctx) = Self::agent_ctx(req);
        let Some(script) = self.python.as_mut() else {
            return answer(Ok(
                json!({"cancelled": false, "reason": "no script is running"}),
            ));
        };
        if let Err(body) = python::may_cancel(&script.run.ctx().actor, &ctx.actor) {
            return answer(Err(body));
        }
        let end = script.run.cancel(&info.name);
        self.end_script(end);
        answer(Ok(json!({"cancelled": true})))
    }

    /// The user stops the running script (Stop button, Esc twice, or "Stop
    /// script and continue"): kill it, roll it back, then do held actions.
    pub(super) fn stop_script(&mut self) {
        let Some(script) = self.python.as_mut() else {
            return;
        };
        let end = script.run.cancel("the user");
        self.end_script(end);
    }

    /// Esc pressed: a second press within a second stops a running script.
    pub(super) fn escape_while_script(&mut self) {
        if self.python.is_none() {
            return;
        }
        if self.script_wait.escape(Instant::now()) {
            self.stop_script();
        } else {
            self.say("Press Esc again to stop the Python script", false);
        }
    }

    /// Once per frame: finish the script when it has ended (or timed out),
    /// commit or roll back, answer the caller and retire the token.
    pub(super) fn poll_python(&mut self, ctx: &egui::Context) {
        let ended = self.python.as_mut().and_then(|s| s.run.poll());
        match ended {
            Some(end) => self.end_script(end),
            None if self.python.is_some() => {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            None => {}
        }
    }

    /// Finish the script with `end`: commit or roll back, report, answer
    /// its caller, retire the token, then do the user's held actions.
    fn end_script(&mut self, end: RunEnd) {
        let Some(script) = self.python.take() else {
            return;
        };
        self.link.set_run_token(None); // the token dies with the run
        self.plugins.script_ended();
        let targets = script.run.targets().to_vec();
        let r = script.run.finish(&mut self.session, end);
        let code = r
            .as_ref()
            .err()
            .and_then(|b| b["failures"][0]["code"].as_str())
            .unwrap_or("")
            .to_string();
        if let Some(label) = &script.plugin {
            self.plugin_command_ended(label, &script.agent, end, &r);
            for action in self.script_wait.script_ended() {
                self.perform(action);
            }
            return;
        }
        crate::report::record(if r.is_ok() {
            "agent run_python ok".to_string()
        } else {
            format!("agent run_python failed {code}")
        });
        match (&r, &mut self.proposal) {
            (Ok(_), Some(p)) => {
                p.steps += 1;
                for t in targets {
                    note_target(&mut p.targets, t);
                }
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
                self.refresh_view();
                self.selection.retain(|id| self.session.doc().contains(*id));
                let text = if end == RunEnd::Cancelled {
                    format!(
                        "Stopped {}'s Python script; nothing was changed",
                        script.agent
                    )
                } else {
                    format!(
                        "{}'s Python script failed ({code}); nothing was changed",
                        script.agent
                    )
                };
                self.say(text, end != RunEnd::Cancelled);
            }
        }
        if let Some(reply) = script.reply {
            let _ = reply.send(answer(r));
        }
        for action in self.script_wait.script_ended() {
            self.perform(action);
        }
    }

    /// Status-bar item while a script runs: "Script running · N s" and Stop.
    pub(super) fn script_status(&mut self, ui: &mut egui::Ui) {
        let Some(script) = &self.python else {
            return;
        };
        let mut text = format!("Script running · {} s", script.run.elapsed().as_secs());
        let waiting = self.script_wait.waiting();
        if waiting > 0 {
            text.push_str(&format!(" · {waiting} action(s) wait"));
        }
        ui.label(
            RichText::new(text)
                .strong()
                .color(Color32::from_rgb(120, 60, 200)),
        );
        if ui
            .button("Stop")
            .on_hover_text("Stop the script and undo what it did (Esc twice)")
            .clicked()
        {
            self.stop_script();
        }
        ui.separator();
    }

    /// The dialog shown when the user tries to edit while a script runs.
    pub(super) fn script_wait_ui(&mut self, ctx: &egui::Context) {
        if !self.script_wait.asking() {
            return;
        }
        let secs = self
            .python
            .as_ref()
            .map(|s| s.run.elapsed().as_secs())
            .unwrap_or(0);
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("script_wait")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.label(format!(
                "A script is running ({secs} s). Stop it and do your action?"
            ));
            ui.label(
                RichText::new("If you keep waiting, your action runs when the script ends.").weak(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Stop script and continue").clicked() {
                    choice = Some(Choice::StopAndContinue);
                }
                if ui.button("Keep waiting").clicked() {
                    choice = Some(Choice::KeepWaiting);
                }
            });
        });
        if choice.is_none() && modal.should_close() {
            choice = Some(Choice::KeepWaiting);
        }
        let Some(choice) = choice else { return };
        let plan = self.script_wait.choose(choice, self.python.is_some());
        if plan.stop {
            self.stop_script();
        }
        for action in plan.perform {
            self.perform(action);
        }
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
        assert!(!changes_the_document(python::CANCEL_TOOL));
    }

    #[test]
    fn stopping_hands_the_held_actions_to_the_end_of_the_run() {
        let mut w = ScriptWait::default();
        w.hold(UserAction::Undo);
        w.hold(UserAction::Save);
        assert!(w.asking());
        assert_eq!(w.waiting(), 2);
        let plan = w.choose(Choice::StopAndContinue, true);
        assert!(plan.stop);
        assert!(plan.perform.is_empty(), "done once the rollback is over");
        assert!(!w.asking());
        let done = w.script_ended();
        assert!(matches!(done[..], [UserAction::Undo, UserAction::Save]));
        assert_eq!(w.waiting(), 0);
    }

    #[test]
    fn keep_waiting_never_drops_the_action() {
        let mut w = ScriptWait::default();
        w.hold(UserAction::Redo);
        let plan = w.choose(Choice::KeepWaiting, true);
        assert!(!plan.stop);
        assert!(plan.perform.is_empty());
        assert!(!w.asking());
        assert_eq!(w.waiting(), 1, "still held");
        // The script finishes on its own: the action runs then.
        assert!(matches!(w.script_ended()[..], [UserAction::Redo]));
    }

    #[test]
    fn a_script_that_ended_before_the_answer_hands_over_at_once() {
        for choice in [Choice::StopAndContinue, Choice::KeepWaiting] {
            let mut w = ScriptWait::default();
            w.hold(UserAction::NewDocument);
            let plan = w.choose(choice, false);
            assert!(!plan.stop, "nothing to stop");
            assert!(matches!(plan.perform[..], [UserAction::NewDocument]));
            assert!(w.script_ended().is_empty());
        }
    }

    #[test]
    fn esc_twice_within_a_second_stops() {
        let mut w = ScriptWait::default();
        let t0 = Instant::now();
        assert!(!w.escape(t0));
        assert!(w.escape(t0 + Duration::from_millis(400)));
        // A third press starts over.
        assert!(!w.escape(t0 + Duration::from_millis(600)));
        // Too slow.
        assert!(!w.escape(t0 + Duration::from_millis(1700)));
        assert!(w.escape(t0 + Duration::from_millis(2600)));
        // Exactly the limit still counts; the end of a run forgets presses.
        assert!(!w.escape(t0 + Duration::from_secs(5)));
        assert!(w.escape(t0 + Duration::from_secs(6)));
        assert!(!w.escape(t0 + Duration::from_secs(10)));
        w.script_ended();
        assert!(!w.escape(t0 + Duration::from_millis(10_100)));
    }

    #[test]
    fn the_proposal_bar_names_where_changes_land() {
        let name = |id: ElementId| format!("Option {}", id.0);
        assert_eq!(target_label(&[], name), "shared model");
        assert_eq!(target_label(&[None], name), "shared model");
        assert_eq!(
            target_label(&[Some(ElementId(4))], name),
            "into option: Option 4"
        );
        assert_eq!(
            target_label(&[Some(ElementId(4)), None], name),
            "into option: Option 4, shared model"
        );
        let mut t = Vec::new();
        note_target(&mut t, Some(ElementId(4)));
        note_target(&mut t, None);
        note_target(&mut t, Some(ElementId(4)));
        assert_eq!(t, vec![Some(ElementId(4)), None]);
    }
}
