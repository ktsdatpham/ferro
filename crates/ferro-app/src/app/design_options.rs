//! Design options in the app (ADR 0006, M5-23.9): the combination the view
//! shows (app state until views become elements, M5-22), "edit in option",
//! and the Design options panel.
//!
//! Rules:
//! * a document without option sets shows the shared model as before;
//! * with option sets, the view shows a combination (primary by default):
//!   the shared model with the chosen option layers, resolved
//!   (`ops::design_option_commands::resolve`). Elements added by the other
//!   options are not shown;
//! * while an option is edited, commands run inside it
//!   (`Session::execute_in_option`): the shared model does not change, the
//!   option layer does, one undo step per command;
//! * a combination that cannot be shown (conflicting or stale option
//!   layers) shows the shared model with the reason in the panel.

use super::FerroApp;
use eframe::egui::{self, Color32, RichText};
use ferro_core::model::change::partition_applicable;
use ferro_core::model::design_option::{stale_layers, Combination, OptionStatus};
use ferro_core::ops::design_option_commands::{
    resolve, resolve_options, AcceptDesignOption, CreateDesignOption, CreateOptionSet,
    DeleteDesignOption, DeleteOptionSet, SetDesignOption,
};
use ferro_core::ops::{AnyCommand, CommandOutput, Failure, Outcome};
use ferro_core::{Document, ElementId};

/// Design-option view state of the app (not saved).
#[derive(Default)]
pub struct DesignOptionsState {
    /// Options picked for the view (sets not listed show their primary).
    picks: Vec<ElementId>,
    /// Option being edited: commands run inside it.
    editing: Option<ElementId>,
    /// The document as seen in the combination; `None` = the shared model.
    view: Option<Document>,
    /// Session document and picks `view` was resolved from.
    key: Option<(Document, Vec<ElementId>, Option<ElementId>)>,
    /// Why the combination cannot be shown.
    error: Option<String>,
}

/// `true` for commands on option sets / options themselves, which always
/// run on the shared model.
pub fn is_option_command(name: &str) -> bool {
    name.ends_with("_option_set") || name.ends_with("_design_option") || name == "apply_changes"
}

/// Combination shown for `picks` with `editing` chosen for its set.
fn combination_of(doc: &Document, picks: &[ElementId], editing: Option<ElementId>) -> Combination {
    let c = Combination {
        options: picks.to_vec(),
    };
    match editing {
        Some(o) => c.with(doc, o),
        None => c,
    }
}

fn open(doc: &Document, id: ElementId) -> bool {
    doc.get(id)
        .and_then(|e| e.as_design_option())
        .is_some_and(|o| o.status == OptionStatus::Open)
}

fn failure_line(f: &[Failure]) -> String {
    f.iter()
        .map(|x| format!("{} ({})", x.message, x.code))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The document the view shows, borrowing only the two fields (for call
/// sites that also mutate other fields of the app).
pub fn view_doc<'a>(session: &'a ferro_core::Session, st: &'a DesignOptionsState) -> &'a Document {
    st.view.as_ref().unwrap_or_else(|| session.doc())
}

impl FerroApp {
    /// The document the view shows: the combination, or the shared model.
    pub(super) fn doc(&self) -> &Document {
        self.options
            .view
            .as_ref()
            .unwrap_or_else(|| self.session.doc())
    }

    /// The option being edited, if any.
    pub(super) fn editing_option(&self) -> Option<ElementId> {
        self.options.editing
    }

    /// Recompute the view document when the session document or the picks
    /// changed (cheap otherwise).
    pub(super) fn refresh_view(&mut self) {
        let doc = self.session.doc();
        if doc.option_sets().is_empty() {
            self.options = DesignOptionsState::default();
            return;
        }
        let st = &mut self.options;
        st.picks.retain(|o| open(doc, *o));
        st.editing = st.editing.filter(|o| open(doc, *o));
        if let Some((d, p, e)) = &st.key {
            if crate::scene::same_doc(d, doc) && *p == st.picks && *e == st.editing {
                return;
            }
        }
        let combo = combination_of(doc, &st.picks, st.editing);
        match resolve(doc, &combo) {
            Ok(v) => {
                st.view = Some(v);
                st.error = None;
            }
            Err(f) => {
                st.error = Some(failure_line(&f));
                st.view = resolve_options(doc, &[]).ok();
            }
        }
        st.key = Some((doc.clone(), st.picks.clone(), st.editing));
    }

    /// Run a command: inside the edited option, or on the shared model.
    pub(super) fn execute_here(&mut self, cmd: AnyCommand) -> Result<Outcome, Vec<Failure>> {
        let ctx = self.ctx();
        let r = match self
            .options
            .editing
            .filter(|_| !is_option_command(cmd.name()))
        {
            Some(o) => {
                let combo = combination_of(self.session.doc(), &self.options.picks, None);
                self.session.execute_in_option(o, &combo, cmd, &ctx)
            }
            None => self.session.execute(cmd, &ctx),
        };
        self.refresh_view();
        r
    }

    /// Preview a command on the document the view shows.
    pub(super) fn preview_here(
        &self,
        cmd: &AnyCommand,
    ) -> Result<(Document, CommandOutput), Vec<Failure>> {
        ferro_core::Session::preview_on(self.doc(), cmd)
    }

    /// Status-line note while an option is edited or shown.
    pub(super) fn option_status(&self, ui: &mut egui::Ui) {
        let doc = self.session.doc();
        if let Some(o) = self.options.editing.and_then(|o| doc.get(o)) {
            ui.label(
                RichText::new(format!("Editing option: {}", o.label()))
                    .color(Color32::from_rgb(200, 120, 0))
                    .strong(),
            );
        } else if !self.options.picks.is_empty() {
            let names: Vec<String> = self
                .options
                .picks
                .iter()
                .filter_map(|o| doc.get(*o).map(|e| e.label()))
                .collect();
            ui.label(RichText::new(format!("Showing: {}", names.join(", "))).weak());
        }
    }

    /// Side panel section: option sets, options, the combination shown,
    /// edit / primary / accept.
    pub(super) fn design_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Design options");
        let doc = self.session.doc().clone();
        let sets = doc.option_sets();
        let mut cmds: Vec<AnyCommand> = Vec::new();
        if sets.is_empty() {
            ui.label("No option sets. Add one to compare alternatives.");
        }
        let combo = combination_of(&doc, &self.options.picks, self.options.editing);
        let shown = combo.chosen(&doc).unwrap_or_default();
        let mut pick: Option<ElementId> = None;
        let mut edit: Option<Option<ElementId>> = None;
        for set in &sets {
            ui.label(RichText::new(set.label()).strong());
            for o in doc.design_options(set.id) {
                let Some(d) = o.as_design_option() else {
                    continue;
                };
                ui.horizontal(|ui| {
                    if d.status != OptionStatus::Open {
                        ui.label(RichText::new(format!("{} ({})", o.label(), d.status.as_str())).weak());
                        if ui.small_button("×").on_hover_text("Delete this option and its layer").clicked() {
                            cmds.push(DeleteDesignOption { option: o.id }.into());
                        }
                        return;
                    }
                    let is_shown = shown.contains(&o.id);
                    if ui.radio(is_shown, o.label()).on_hover_text("Show this option in the view").clicked() && !is_shown {
                        pick = Some(o.id);
                    }
                    if d.is_primary {
                        ui.label(RichText::new("(primary)").small());
                    } else if ui.small_button("Make primary").on_hover_text("Make this the primary option of its set").clicked() {
                        cmds.push(SetDesignOption { option: o.id, name: None, is_primary: Some(true), changes: None }.into());
                    }
                    let editing = self.options.editing == Some(o.id);
                    let label = if editing { "Done" } else { "Edit" };
                    if ui.small_button(label).on_hover_text("Edit in this option: changes go into its layer, not the shared model").clicked() {
                        edit = Some(if editing { None } else { Some(o.id) });
                    }
                    if ui.small_button("Accept").on_hover_text("Merge this option into the shared model; the other options are archived").clicked() {
                        cmds.push(AcceptDesignOption { option: o.id }.into());
                    }
                    if !d.is_primary && ui.small_button("×").on_hover_text("Delete this option").clicked() {
                        cmds.push(DeleteDesignOption { option: o.id }.into());
                    }
                    ui.label(RichText::new(format!("{} changes", d.changes.len())).weak().small());
                });
            }
            ui.horizontal(|ui| {
                if ui.small_button("Add option").clicked() {
                    cmds.push(
                        CreateDesignOption {
                            set: set.id,
                            name: None,
                            copy_from: None,
                        }
                        .into(),
                    );
                }
                if ui.small_button("Delete set").clicked() {
                    cmds.push(DeleteOptionSet { set: set.id }.into());
                }
            });
        }
        if ui.button("Add option set").clicked() {
            let mut k = sets.len() + 1;
            let mut name = format!("Option set {k}");
            while sets
                .iter()
                .any(|s| s.name.as_deref() == Some(name.as_str()))
            {
                k += 1;
                name = format!("Option set {k}");
            }
            cmds.push(CreateOptionSet { name }.into());
        }
        if let Some(e) = &self.options.error {
            ui.label(
                RichText::new(format!("Cannot show this combination: {e}"))
                    .color(Color32::from_rgb(200, 40, 40))
                    .small(),
            );
        }
        for stale in stale_layers(&doc) {
            let Some(id) = stale.option else { continue };
            ui.label(
                RichText::new(stale.message.clone())
                    .color(Color32::from_rgb(200, 120, 0))
                    .small(),
            );
            let changes = doc
                .get(id)
                .and_then(|e| e.as_design_option())
                .map(|o| o.changes.clone())
                .unwrap_or_default();
            if ui.small_button("Keep what still applies").clicked() {
                match partition_applicable(&doc, &changes) {
                    Ok((kept, _)) => cmds.push(
                        SetDesignOption {
                            option: id,
                            name: None,
                            is_primary: None,
                            changes: Some(kept),
                        }
                        .into(),
                    ),
                    Err(e) => self.say(e.to_string(), true),
                }
            }
        }
        if let Some(o) = pick {
            let c = Combination {
                options: self.options.picks.clone(),
            }
            .with(&doc, o);
            self.options.picks = c.options;
            if self.options.editing.is_some_and(|e| {
                doc.get(e).and_then(|x| x.host_id) == doc.get(o).and_then(|x| x.host_id)
            }) {
                self.options.editing = None;
            }
        }
        if let Some(e) = edit {
            self.options.editing = e;
        }
        for c in cmds {
            self.run(c);
        }
        self.refresh_view();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{ExecContext, SequentialIds};
    use ferro_core::Session;
    use serde_json::json;

    #[test]
    fn option_commands_always_run_on_the_shared_model() {
        for n in [
            "create_option_set",
            "set_option_set",
            "delete_option_set",
            "create_design_option",
            "set_design_option",
            "delete_design_option",
            "accept_design_option",
            "apply_changes",
        ] {
            assert!(is_option_command(n), "{n}");
            assert!(AnyCommand::NAMES.contains(&n), "{n}");
        }
        assert!(!is_option_command("create_box"));
    }

    #[test]
    fn the_view_shows_only_the_chosen_options_elements() {
        let mut s = Session::new(Document::new("T"), Box::new(SequentialIds::new(1)));
        let ctx = ExecContext::user();
        let o = s
            .execute(
                AnyCommand::from_tool("create_option_set", &json!({"name": "R"})).unwrap(),
                &ctx,
            )
            .unwrap()
            .output;
        let a = ElementId(o["option"].as_u64().unwrap());
        let b = ElementId(
            s.execute(
                AnyCommand::from_tool("create_design_option", &json!({"set": o["element"]}))
                    .unwrap(),
                &ctx,
            )
            .unwrap()
            .output["element"]
                .as_u64()
                .unwrap(),
        );
        let boxed = |x: f64| {
            AnyCommand::from_tool("create_box", &json!({"origin": {"x": x, "y": 0, "z": 0}, "width": 1000, "depth": 1000, "height": 1000})).unwrap()
        };
        s.execute_in_option(a, &Combination::primary(), boxed(0.0), &ctx)
            .unwrap();
        s.execute_in_option(b, &Combination::primary(), boxed(5000.0), &ctx)
            .unwrap();
        let forms = |d: &Document| d.elements().filter(|e| e.as_form().is_some()).count();
        assert_eq!(forms(s.doc()), 0, "shared model");
        let primary = combination_of(s.doc(), &[], None);
        let seen = resolve(s.doc(), &primary).unwrap();
        assert_eq!(forms(&seen), 1);
        assert!(seen
            .elements()
            .filter(|e| e.as_form().is_some())
            .all(|e| e.design_option == Some(a)));
        let with_b = combination_of(s.doc(), &[], Some(b));
        let seen = resolve(s.doc(), &with_b).unwrap();
        assert!(seen
            .elements()
            .filter(|e| e.as_form().is_some())
            .all(|e| e.design_option == Some(b)));
    }
}
