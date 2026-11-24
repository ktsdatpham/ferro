use super::{
    AnyCommand, CommandOutput, DocumentChanged, Failure, IdSource, SequentialIds, Transaction,
};
use crate::algo::Kernel;
use crate::model::cut::CutCache;
use crate::model::Document;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Who performed a change (D-29). Agents are first-class actors (D-44).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Agent { model: String },
    Plugin { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    #[serde(flatten)]
    pub kind: ActorKind,
    pub name: String,
}

impl Actor {
    pub fn user(name: impl Into<String>) -> Actor {
        Actor {
            kind: ActorKind::User,
            name: name.into(),
        }
    }
    pub fn agent(name: impl Into<String>, model: impl Into<String>) -> Actor {
        Actor {
            kind: ActorKind::Agent {
                model: model.into(),
            },
            name: name.into(),
        }
    }
}

/// Context recorded with each transaction. The caller supplies the time so
/// the kernel stays deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecContext {
    pub actor: Actor,
    pub prompt: Option<String>,
    pub timestamp_ms: u64,
}

impl ExecContext {
    pub fn user() -> ExecContext {
        ExecContext {
            actor: Actor::user("user"),
            prompt: None,
            timestamp_ms: 0,
        }
    }
}

/// One journal entry per committed transaction or group (D-29).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub seq: u64,
    pub transaction: String,
    pub actor: Actor,
    pub prompt: Option<String>,
    pub timestamp_ms: u64,
    pub commands: Vec<AnyCommand>,
    pub changed: DocumentChanged,
    /// Source text of the scripts (`run_python`, ADR 0005) whose commands
    /// make up this entry, in run order. Empty for ordinary transactions;
    /// absent in older files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<String>,
}

/// A savepoint inside an open transaction group (`SubTransaction`, chapter 4
/// §4.3): rolling back to it discards what the group did since, and keeps
/// the group open. Made by [`Session::begin_sub_transaction`]; committing is
/// simply dropping it. It only applies to the group it was made in.
#[derive(Debug, Clone)]
pub struct SubTransaction {
    group: u64,
    before: Document,
    commands: usize,
    scripts: usize,
}

/// Result of a successful command.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub output: CommandOutput,
    pub changed: DocumentChanged,
    pub warnings: Vec<Failure>,
}

#[derive(Debug, Clone)]
struct Step {
    name: String,
    before: Document,
    after: Document,
}

#[derive(Debug, Clone)]
struct Group {
    name: String,
    before: Document,
    commands: Vec<AnyCommand>,
    scripts: Vec<String>,
    ctx: ExecContext,
    /// Serial number, so a `SubTransaction` cannot roll back another group.
    serial: u64,
}

/// Owns the current document, undo / redo, transaction groups and the journal.
pub struct Session {
    doc: Document,
    undo: Vec<Step>,
    redo: Vec<Step>,
    journal: Vec<JournalEntry>,
    group: Option<Group>,
    ids: Box<dyn IdSource + Send>,
    kernel: Option<Arc<dyn Kernel>>,
    cut_cache: Arc<CutCache>,
    seq: u64,
    groups_opened: u64,
}

impl Session {
    pub fn new(doc: Document, ids: Box<dyn IdSource + Send>) -> Session {
        Session {
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            group: None,
            ids,
            kernel: None,
            cut_cache: Arc::new(CutCache::new()),
            seq: 0,
            groups_opened: 0,
        }
    }

    /// The same session with a geometry kernel (ADR 0007, D-78), injected
    /// like the id source. Without one, commands that need it (booleans)
    /// fail with `ferro.kernel.unavailable`.
    pub fn with_kernel(mut self, kernel: Arc<dyn Kernel>) -> Session {
        self.kernel = Some(kernel);
        self
    }

    /// Attach or remove the geometry kernel.
    pub fn set_kernel(&mut self, kernel: Option<Arc<dyn Kernel>>) {
        self.kernel = kernel;
    }

    /// The attached geometry kernel, if any.
    pub fn kernel(&self) -> Option<&Arc<dyn Kernel>> {
        self.kernel.as_ref()
    }

    /// The session's cache of cut results (M5-5.7), shared by its
    /// transactions, previews and resolved combinations. Not part of the
    /// document: results are the same without it.
    pub fn cut_cache(&self) -> &Arc<CutCache> {
        &self.cut_cache
    }

    /// A transaction on `doc` with this session's kernel and cut cache.
    fn transaction<'a>(
        kernel: &Option<Arc<dyn Kernel>>,
        cache: &Arc<CutCache>,
        name: &str,
        doc: &Document,
        ids: &'a mut dyn IdSource,
    ) -> Transaction<'a> {
        Transaction::new(name, doc, ids)
            .with_kernel(kernel.clone())
            .with_cut_cache(Some(cache.clone()))
    }

    /// The document as seen in `combination` ([`resolve`]), regenerated
    /// with this session's kernel and cut cache, so cuts on elements an
    /// option layer changes stay applied (M5-5.6).
    ///
    /// # Errors
    /// The codes of [`resolve`].
    ///
    /// [`resolve`]: super::design_option_commands::resolve
    pub fn resolve(
        &self,
        combination: &crate::model::Combination,
    ) -> Result<Document, Vec<Failure>> {
        let chosen = combination
            .chosen(&self.doc)
            .map_err(|e| vec![super::CommandError::from(e).into_failure()])?;
        self.resolve_options(&chosen)
    }

    /// The document with exactly the layers of `options`
    /// ([`resolve_options`]), with this session's kernel and cut cache.
    ///
    /// # Errors
    /// The codes of [`resolve_options`].
    ///
    /// [`resolve_options`]: super::design_option_commands::resolve_options
    pub fn resolve_options(
        &self,
        options: &[crate::model::ElementId],
    ) -> Result<Document, Vec<Failure>> {
        super::design_option_commands::resolve_options_cached(
            &self.doc,
            options,
            self.kernel.as_ref(),
            Some(&self.cut_cache),
        )
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }
    pub fn can_undo(&self) -> bool {
        self.group.is_none() && !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        self.group.is_none() && !self.redo.is_empty()
    }
    pub fn undo_name(&self) -> Option<&str> {
        self.undo.last().map(|s| s.name.as_str())
    }
    pub fn redo_name(&self) -> Option<&str> {
        self.redo.last().map(|s| s.name.as_str())
    }

    /// Replace the document (open file); clears history and the cut cache.
    pub fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.cut_cache.clear();
        self.undo.clear();
        self.redo.clear();
        self.group = None;
    }

    /// Run one command as one transaction (or inside the open group).
    pub fn execute(&mut self, cmd: AnyCommand, ctx: &ExecContext) -> Result<Outcome, Vec<Failure>> {
        let name = cmd.name().to_string();
        let mut tx = Self::transaction(
            &self.kernel,
            &self.cut_cache,
            &name,
            &self.doc,
            self.ids.as_mut(),
        );
        let output = cmd.execute(&mut tx).map_err(|e| vec![e.into_failure()])?;
        let (after, changed, warnings) = tx.finish()?;
        let before = std::mem::replace(&mut self.doc, after);
        match &mut self.group {
            Some(g) => g.commands.push(cmd),
            None => {
                self.redo.clear();
                self.undo.push(Step {
                    name: name.clone(),
                    before,
                    after: self.doc.clone(),
                });
                self.record(&name, ctx, vec![cmd], changed.clone());
            }
        }
        Ok(Outcome {
            output,
            changed,
            warnings,
        })
    }

    /// Run a command inside a design option (ADR 0006, "edit in option"):
    /// the command runs on the document as seen in `combination` with
    /// `option` chosen for its set; the result, minus what the other sets
    /// show, becomes the option's layer through one `set_design_option`
    /// (one undo step, or part of the open group). The shared model is not
    /// changed. Element ids in the outcome are those of the resolved
    /// document (elements added by options exist only there).
    ///
    /// Fails with the command's own failures, `ferro.model.not_a_design_option`,
    /// `ferro.model.option_not_open`, `ferro.model.option_conflict` and
    /// `ferro.model.invalid_option_layer` (the edit touches something the
    /// option cannot hold, e.g. an element added by another set's option, an
    /// option set, or the project).
    pub fn execute_in_option(
        &mut self,
        option: crate::model::ElementId,
        combination: &crate::model::Combination,
        cmd: AnyCommand,
        ctx: &ExecContext,
    ) -> Result<Outcome, Vec<Failure>> {
        use super::design_option_commands::SetDesignOption;
        use crate::model::design_option::layer_from_edit;
        let fail = |e: super::CommandError| vec![e.into_failure()];
        let chosen = combination
            .with(&self.doc, option)
            .chosen(&self.doc)
            .map_err(|e| fail(e.into()))?;
        let others: Vec<_> = chosen.iter().copied().filter(|o| *o != option).collect();
        let base_others = self.resolve_options(&others)?;
        let base = self.resolve_options(&chosen)?;
        let name = cmd.name().to_string();
        let mut tx = Self::transaction(
            &self.kernel,
            &self.cut_cache,
            &name,
            &base,
            self.ids.as_mut(),
        );
        let output = cmd.execute(&mut tx).map_err(|e| vec![e.into_failure()])?;
        let (edited, changed, mut warnings) = tx.finish()?;
        let changes = layer_from_edit(&base_others, &edited).map_err(|e| fail(e.into()))?;
        let set = SetDesignOption {
            option,
            name: None,
            is_primary: None,
            changes: Some(changes),
        };
        let outer = self.execute(set.into(), ctx)?;
        warnings.extend(outer.warnings);
        Ok(Outcome {
            output,
            changed,
            warnings,
        })
    }

    /// Evaluate a command on a copy without changing the session (tool
    /// previews, AI proposals). Returns the would-be document.
    pub fn preview(&self, cmd: &AnyCommand) -> Result<(Document, CommandOutput), Vec<Failure>> {
        self.preview_on(&self.doc, cmd)
    }

    /// [`Session::preview`] on any document, e.g. one resolved for a
    /// design-option combination (the app's view while editing an option),
    /// with this session's geometry kernel.
    pub fn preview_on(
        &self,
        doc: &Document,
        cmd: &AnyCommand,
    ) -> Result<(Document, CommandOutput), Vec<Failure>> {
        let mut ids = SequentialIds::new(0);
        let mut tx = Self::transaction(&self.kernel, &self.cut_cache, cmd.name(), doc, &mut ids);
        let output = cmd.execute(&mut tx).map_err(|e| vec![e.into_failure()])?;
        let (doc, _, _) = tx.finish()?;
        Ok((doc, output))
    }

    /// Start a group: following commands become one undo step (a gesture,
    /// an AI proposal). Nested groups are not allowed.
    pub fn begin_group(&mut self, name: impl Into<String>, ctx: ExecContext) -> bool {
        if self.group.is_some() {
            return false;
        }
        self.groups_opened += 1;
        self.group = Some(Group {
            name: name.into(),
            before: self.doc.clone(),
            commands: Vec::new(),
            scripts: Vec::new(),
            ctx,
            serial: self.groups_opened,
        });
        true
    }

    /// Record a script's source text on the open group, so its journal entry
    /// names the script that made the changes (D-29, `run_python`). Returns
    /// false when no group is open.
    pub fn add_group_script(&mut self, script: impl Into<String>) -> bool {
        match &mut self.group {
            Some(g) => {
                g.scripts.push(script.into());
                true
            }
            None => false,
        }
    }

    /// A savepoint in the open group (`SubTransaction`), or `None` when no
    /// group is open. Savepoints may nest: roll back the inner one first.
    pub fn begin_sub_transaction(&self) -> Option<SubTransaction> {
        let g = self.group.as_ref()?;
        Some(SubTransaction {
            group: g.serial,
            before: self.doc.clone(),
            commands: g.commands.len(),
            scripts: g.scripts.len(),
        })
    }

    /// Discard everything the open group did since `sub` was made (document,
    /// commands and scripts); the group stays open. `None`, changing
    /// nothing, when `sub` belongs to a group that is no longer open.
    pub fn rollback_sub_transaction(&mut self, sub: SubTransaction) -> Option<DocumentChanged> {
        let g = self.group.as_mut()?;
        if g.serial != sub.group || g.commands.len() < sub.commands {
            return None;
        }
        g.commands.truncate(sub.commands);
        g.scripts.truncate(sub.scripts);
        let changed = DocumentChanged::between(&g.name, &self.doc, &sub.before);
        self.doc = sub.before;
        Some(changed)
    }

    /// Commit the group as one undo step.
    pub fn end_group(&mut self) -> Option<DocumentChanged> {
        let g = self.group.take()?;
        let changed = DocumentChanged::between(&g.name, &g.before, &self.doc);
        if changed.is_empty() {
            return Some(changed);
        }
        self.redo.clear();
        self.undo.push(Step {
            name: g.name.clone(),
            before: g.before,
            after: self.doc.clone(),
        });
        self.record(&g.name, &g.ctx, g.commands, changed.clone());
        if let Some(last) = self.journal.last_mut() {
            last.scripts = g.scripts;
        }
        Some(changed)
    }

    /// Discard everything done since `begin_group`.
    pub fn cancel_group(&mut self) -> Option<DocumentChanged> {
        let g = self.group.take()?;
        let changed = DocumentChanged::between(&g.name, &self.doc, &g.before);
        self.doc = g.before;
        Some(changed)
    }

    pub fn in_group(&self) -> bool {
        self.group.is_some()
    }

    pub fn undo(&mut self) -> Option<DocumentChanged> {
        if self.group.is_some() {
            return None;
        }
        let step = self.undo.pop()?;
        let changed =
            DocumentChanged::between(&format!("Undo {}", step.name), &self.doc, &step.before);
        self.doc = step.before.clone();
        self.redo.push(step);
        Some(changed)
    }

    pub fn redo(&mut self) -> Option<DocumentChanged> {
        if self.group.is_some() {
            return None;
        }
        let step = self.redo.pop()?;
        let changed =
            DocumentChanged::between(&format!("Redo {}", step.name), &self.doc, &step.after);
        self.doc = step.after.clone();
        self.undo.push(step);
        Some(changed)
    }

    fn record(
        &mut self,
        name: &str,
        ctx: &ExecContext,
        commands: Vec<AnyCommand>,
        changed: DocumentChanged,
    ) {
        self.seq += 1;
        self.journal.push(JournalEntry {
            seq: self.seq,
            transaction: name.to_string(),
            actor: ctx.actor.clone(),
            prompt: ctx.prompt.clone(),
            timestamp_ms: ctx.timestamp_ms,
            commands,
            changed,
            scripts: Vec::new(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::*;
    use super::*;
    use crate::math::{Length, Point3, Vec3};
    use crate::model::{keys, Category, ElementId, ParamValue};
    use crate::topo::FaceId;

    fn session() -> Session {
        Session::new(
            Document::new("Test"),
            Box::new(SequentialIds::new(1_764_000_000_000)),
        )
    }

    fn run(s: &mut Session, c: impl Into<AnyCommand>) -> Outcome {
        s.execute(c.into(), &ExecContext::user()).unwrap()
    }

    fn rect(s: &mut Session) -> (ElementId, FaceId) {
        let o = run(
            s,
            CreateRectangle {
                origin: Point3::ORIGIN,
                width: Length::from_mm(4000.0),
                depth: Length::from_mm(3000.0),
                level: None,
                name: Some("Room".into()),
            },
        );
        (
            serde_json::from_value(o.output["element"].clone()).unwrap(),
            serde_json::from_value(o.output["face"].clone()).unwrap(),
        )
    }

    #[test]
    fn draw_pull_undo_redo() {
        let mut s = session();
        let (id, face) = rect(&mut s);
        let o = run(
            &mut s,
            PushPullFace {
                element: id,
                face,
                distance: Length::from_mm(2800.0),
            },
        );
        assert_eq!(o.changed.modified, vec![id]);
        let vol = |s: &Session| match s.doc().get(id).unwrap().param(keys::FORM_VOLUME) {
            Some(ParamValue::Volume(v)) => *v,
            _ => 0.0,
        };
        assert!((vol(&s) - 4000.0 * 3000.0 * 2800.0).abs() < 1.0);
        s.undo().unwrap();
        assert_eq!(vol(&s), 0.0);
        s.redo().unwrap();
        assert!(vol(&s) > 0.0);
        assert_eq!(s.journal().len(), 2);
        assert_eq!(s.journal()[1].commands[0].name(), "push_pull_face");
    }

    #[test]
    fn errors_roll_back_completely() {
        let mut s = session();
        let (id, face) = rect(&mut s);
        let before = s.doc().clone();
        let err = s
            .execute(
                PushPullFace {
                    element: id,
                    face,
                    distance: Length::from_mm(0.1),
                }
                .into(),
                &ExecContext::user(),
            )
            .unwrap_err();
        assert_eq!(err[0].code, "ferro.algo.zero_distance");
        assert_eq!(s.doc(), &before);
    }

    #[test]
    fn group_is_one_undo_step_and_cancel_restores() {
        let mut s = session();
        s.begin_group("Two boxes", ExecContext::user());
        for x in [0.0, 5000.0] {
            run(
                &mut s,
                CreateBox {
                    origin: Point3::new(x, 0.0, 0.0),
                    width: Length::from_mm(1000.0),
                    depth: Length::from_mm(1000.0),
                    height: Length::from_mm(1000.0),
                    level: None,
                    name: None,
                },
            );
        }
        assert!(!s.can_undo());
        s.end_group();
        assert_eq!(s.doc().len(), 2);
        s.undo();
        assert_eq!(s.doc().len(), 0);

        s.begin_group("Scratch", ExecContext::user());
        rect(&mut s);
        s.cancel_group();
        assert_eq!(s.doc().len(), 0);
    }

    #[test]
    fn level_move_carries_forms_and_delete_clears_refs() {
        let mut s = session();
        let lvl: ElementId = serde_json::from_value(
            run(
                &mut s,
                CreateLevel {
                    name: "L2".into(),
                    elevation: Length::from_mm(3300.0),
                    is_building_storey: true,
                },
            )
            .output["element"]
                .clone(),
        )
        .unwrap();
        let bx: ElementId = serde_json::from_value(
            run(
                &mut s,
                CreateBox {
                    origin: Point3::ORIGIN,
                    width: Length::from_mm(1000.0),
                    depth: Length::from_mm(1000.0),
                    height: Length::from_mm(1000.0),
                    level: Some(lvl),
                    name: None,
                },
            )
            .output["element"]
                .clone(),
        )
        .unwrap();
        let zmin = |s: &Session| s.doc().get(bx).unwrap().bounding_box().unwrap().min.z;
        assert!((zmin(&s) - 3300.0).abs() < 1e-6);
        run(
            &mut s,
            SetLevelElevation {
                level: lvl,
                elevation: Length::from_mm(3600.0),
            },
        );
        assert!((zmin(&s) - 3600.0).abs() < 1e-6);
        let o = run(
            &mut s,
            DeleteElements {
                elements: vec![lvl],
            },
        );
        assert_eq!(o.warnings[0].code, "ferro.model.reference_cleared");
        assert_eq!(s.doc().get(bx).unwrap().level_id, None);
    }

    #[test]
    fn copy_array_creates_new_uids() {
        let mut s = session();
        let (id, _) = rect(&mut s);
        let o = run(
            &mut s,
            CopyElements {
                elements: vec![id],
                vector: Vec3::new(5000.0, 0.0, 0.0),
                count: 3,
            },
        );
        let ids: Vec<ElementId> = serde_json::from_value(o.output["elements"].clone()).unwrap();
        assert_eq!(ids.len(), 3);
        let uids: std::collections::HashSet<_> = s.doc().elements().map(|e| e.uid).collect();
        assert_eq!(uids.len(), 4);
    }

    #[test]
    fn parameters_are_validated() {
        let mut s = session();
        let (id, _) = rect(&mut s);
        let bad = |key: &str| {
            AnyCommand::from(SetParameter {
                element: id,
                key: key.into(),
                value: ParamValue::Number(1.0),
            })
        };
        assert_eq!(
            s.execute(bad("Bad Key"), &ExecContext::user()).unwrap_err()[0].code,
            "ferro.model.invalid_parameter_key"
        );
        assert_eq!(
            s.execute(bad(keys::FORM_AREA), &ExecContext::user())
                .unwrap_err()[0]
                .code,
            "ferro.model.read_only_parameter"
        );
        run(
            &mut s,
            SetCategory {
                element: id,
                category: Category::Floors,
            },
        );
        assert_eq!(s.doc().get(id).unwrap().category, Category::Floors);
    }

    #[test]
    fn out_of_extent_is_rejected() {
        let mut s = session();
        let r = s.execute(
            CreateRectangle {
                origin: Point3::new(20_000_000.0, 0.0, 0.0),
                width: Length::from_mm(1000.0),
                depth: Length::from_mm(1000.0),
                level: None,
                name: None,
            }
            .into(),
            &ExecContext::user(),
        );
        assert_eq!(r.unwrap_err()[0].code, "ferro.model.out_of_extent");
    }

    #[test]
    fn commands_roundtrip_through_tool_json() {
        let c = AnyCommand::from_tool(
            "create_box",
            &serde_json::json!({"origin": {"x":0,"y":0,"z":0}, "width": 1000, "depth": 2000, "height": 3000}),
        )
        .unwrap();
        assert_eq!(c.name(), "create_box");
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<AnyCommand>(&s).unwrap(), c);
        assert!(AnyCommand::from_tool("explode_everything", &serde_json::json!({})).is_err());
    }

    #[test]
    fn document_serde_roundtrip() {
        let mut s = session();
        rect(&mut s);
        let json = serde_json::to_string(s.doc()).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, s.doc());
    }

    #[test]
    fn sub_transaction_rolls_back_inside_the_group_and_keeps_earlier_work() {
        let mut s = session();
        assert!(
            s.begin_sub_transaction().is_none(),
            "no group, no savepoint"
        );
        assert!(!s.add_group_script("print(1)"));
        assert!(s.begin_group("AI proposal", ExecContext::user()));
        let (first, _) = rect(&mut s);
        assert!(s.add_group_script("kept"));
        let outer = s.begin_sub_transaction().unwrap();
        assert!(s.add_group_script("dropped"));
        let (second, _) = rect(&mut s);
        let inner = s.begin_sub_transaction().unwrap();
        let (third, _) = rect(&mut s);
        // Inner first, then outer (nested savepoints).
        let c = s.rollback_sub_transaction(inner).unwrap();
        assert_eq!(c.deleted, vec![third]);
        assert!(s.doc().contains(second));
        let c = s.rollback_sub_transaction(outer).unwrap();
        assert_eq!(c.deleted, vec![second]);
        assert!(s.in_group(), "the group stays open");
        assert!(s.doc().contains(first));
        s.end_group().unwrap();
        // One undo step, one journal entry with only the kept command / script.
        assert_eq!(s.journal().len(), 1);
        assert_eq!(s.journal()[0].commands.len(), 1);
        assert_eq!(s.journal()[0].scripts, vec!["kept".to_string()]);
        s.undo().unwrap();
        assert_eq!(s.doc().len(), 0);
        assert!(!s.can_undo());
    }

    #[test]
    fn sub_transaction_of_a_closed_group_changes_nothing() {
        let mut s = session();
        s.begin_group("g1", ExecContext::user());
        let stale = s.begin_sub_transaction().unwrap();
        rect(&mut s);
        s.end_group();
        assert!(s.rollback_sub_transaction(stale.clone()).is_none());
        s.begin_group("g2", ExecContext::user());
        rect(&mut s);
        assert!(
            s.rollback_sub_transaction(stale).is_none(),
            "a savepoint of g1 cannot roll back g2"
        );
        assert_eq!(s.doc().len(), 2);
        s.cancel_group();
        assert_eq!(s.doc().len(), 1);
    }

    #[test]
    fn scripts_reach_the_journal_and_old_entries_load_without_them() {
        let mut s = session();
        s.begin_group("run_python", ExecContext::user());
        s.add_group_script("doc.create_box(...)");
        rect(&mut s);
        s.end_group();
        let e = &s.journal()[0];
        assert_eq!(e.scripts, vec!["doc.create_box(...)".to_string()]);
        let json = serde_json::to_value(e).unwrap();
        assert_eq!(json["scripts"][0], "doc.create_box(...)");
        // Ordinary entries do not write the key; entries without it load.
        rect(&mut s);
        let plain = serde_json::to_value(&s.journal()[1]).unwrap();
        assert!(plain.get("scripts").is_none());
        let back: JournalEntry = serde_json::from_value(plain).unwrap();
        assert!(back.scripts.is_empty());
        // A group that changed nothing records nothing, script included.
        s.begin_group("run_python", ExecContext::user());
        s.add_group_script("pass");
        s.end_group();
        assert_eq!(s.journal().len(), 2);
    }
}
