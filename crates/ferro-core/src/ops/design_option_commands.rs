//! Design-option commands (ADR 0006 §Design options, M5-23.9) and the
//! change-list command `apply_changes` (M5-26.2). Each `NAME` is also the
//! MCP tool name.
//!
//! Editing inside an option: [`Session::execute_in_option`] runs any
//! command on the document as seen in a combination and stores the
//! difference as the option's layer (one `set_design_option`, so undo /
//! redo and the journal work as for any command). [`resolve`] gives the
//! document as seen in a combination.
//!
//! [`Session::execute_in_option`]: super::Session::execute_in_option

use super::commands::{Command, CommandOutput};
use super::{CommandError, Failure, SequentialIds, Transaction};
use crate::algo::Kernel;
use crate::model::change::{apply, Change, ChangeError};
use crate::model::cut::CutCache;
use crate::model::design_option::{
    apply_layers, check_design_options, check_layer, Combination, DesignOption, DesignOptionError,
    OptionSet, OptionStatus, FIRST_OPTION_NAME,
};
use crate::model::{Category, Document, ElementBody, ElementId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

impl From<DesignOptionError> for CommandError {
    fn from(e: DesignOptionError) -> Self {
        let c = CommandError::new(e.code, e.message);
        match e.option {
            Some(o) => c.on(o),
            None => c,
        }
    }
}

impl From<ChangeError> for CommandError {
    fn from(e: ChangeError) -> Self {
        CommandError::new(e.code, e.message)
    }
}

/// The document as seen with exactly the layers of `options` (at most one
/// per set), regenerated with `kernel`: derived geometry and computed
/// parameters follow the overrides, and `cut` rows stay applied to the
/// elements a layer changes (M5-5.6; without a kernel they fall back to
/// the uncut body with a `ferro.kernel.unavailable` warning, as in any
/// transaction). Pass the session's kernel (`Session::kernel`), or use
/// `Session::resolve_options`, which also shares the session's cut cache.
/// A document without option sets is returned unchanged.
///
/// Fails with the codes of [`apply_layers`] (`ferro.model.option_conflict`,
/// `ferro.model.invalid_option_layer`, ...) and with the regeneration
/// errors of an element added by a layer that does not fit (e.g. a door
/// wider than its wall).
pub fn resolve_options(
    doc: &Document,
    options: &[ElementId],
    kernel: Option<&Arc<dyn Kernel>>,
) -> Result<Document, Vec<Failure>> {
    resolve_options_cached(doc, options, kernel, None)
}

/// [`resolve_options`] with a cut cache (M5-5.7), e.g. the session's.
pub fn resolve_options_cached(
    doc: &Document,
    options: &[ElementId],
    kernel: Option<&Arc<dyn Kernel>>,
    cut_cache: Option<&Arc<CutCache>>,
) -> Result<Document, Vec<Failure>> {
    if options.is_empty() && doc.option_sets().is_empty() {
        return Ok(doc.clone());
    }
    let layered =
        apply_layers(doc, options).map_err(|e| vec![CommandError::from(e).into_failure()])?;
    let mut ids = SequentialIds::new(0);
    let mut tx = Transaction::new("resolve", doc, &mut ids)
        .with_kernel(kernel.cloned())
        .with_cut_cache(cut_cache.cloned());
    tx.replace_with(&layered);
    let (out, _, _) = tx.finish()?;
    Ok(out)
}

/// The document as seen in `combination` (ADR 0006 §3): the shared model
/// with the chosen layers applied in set order, regenerated with `kernel`
/// (see [`resolve_options`]; `Session::resolve` uses the session's kernel
/// and cut cache). Pure and deterministic. With no option sets it is the
/// shared model.
///
/// Fails with the codes of [`Combination::chosen`] and [`resolve_options`];
/// conflicting layers of different sets are reported
/// (`ferro.model.option_conflict`), never merged.
pub fn resolve(
    doc: &Document,
    combination: &Combination,
    kernel: Option<&Arc<dyn Kernel>>,
) -> Result<Document, Vec<Failure>> {
    let chosen = combination
        .chosen(doc)
        .map_err(|e| vec![CommandError::from(e).into_failure()])?;
    resolve_options(doc, &chosen, kernel)
}

fn not_a_set(id: ElementId) -> CommandError {
    CommandError::new(
        "ferro.model.not_an_option_set",
        format!("{id} is not an option set"),
    )
    .on(id)
}

fn not_an_option(id: ElementId) -> CommandError {
    CommandError::new(
        "ferro.model.not_a_design_option",
        format!("{id} is not a design option"),
    )
    .on(id)
}

fn require_set(tx: &Transaction, id: ElementId) -> Result<(), CommandError> {
    match tx.element(id)?.as_option_set() {
        Some(_) => Ok(()),
        None => Err(not_a_set(id)),
    }
}

/// The option's data and its set.
fn require_option(
    tx: &Transaction,
    id: ElementId,
) -> Result<(DesignOption, ElementId), CommandError> {
    let e = tx.element(id)?;
    match (e.as_design_option(), e.host_id) {
        (Some(o), Some(set)) => Ok((o.clone(), set)),
        _ => Err(not_an_option(id)),
    }
}

fn require_open(o: &DesignOption, id: ElementId, label: &str) -> Result<(), CommandError> {
    if o.status == OptionStatus::Open {
        return Ok(());
    }
    Err(CommandError::new(
        "ferro.model.option_not_open",
        format!(
            "{label} is {}; only open options can change",
            o.status.as_str()
        ),
    )
    .on(id))
}

fn check_name(
    name: &str,
    taken: impl Iterator<Item = (ElementId, Option<String>)>,
    except: Option<ElementId>,
    what: &str,
) -> Result<(), CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::new(
            "ferro.model.invalid_name",
            format!("{what} needs a name"),
        ));
    }
    for (id, n) in taken {
        if Some(id) != except && n.as_deref() == Some(name) {
            return Err(CommandError::new(
                "ferro.model.duplicate_name",
                format!("{what} named {name:?} already exists"),
            ));
        }
    }
    Ok(())
}

fn set_names(doc: &Document) -> Vec<(ElementId, Option<String>)> {
    doc.option_sets()
        .iter()
        .map(|e| (e.id, e.name.clone()))
        .collect()
}

fn option_names(doc: &Document, set: ElementId) -> Vec<(ElementId, Option<String>)> {
    doc.design_options(set)
        .iter()
        .map(|e| (e.id, e.name.clone()))
        .collect()
}

fn modify_option(
    tx: &mut Transaction,
    id: ElementId,
    f: impl FnOnce(&mut DesignOption),
) -> Result<(), CommandError> {
    tx.modify(id, |e| match &mut e.body {
        ElementBody::DesignOption(o) => {
            f(o);
            Ok(())
        }
        _ => Err(not_an_option(e.id)),
    })
}

/// Write `order = position` for every element in `ids` (sets or options).
fn renumber(tx: &mut Transaction, ids: &[ElementId]) -> Result<(), CommandError> {
    for (i, id) in ids.iter().enumerate() {
        let order = i as u32;
        let current = match &tx.element(*id)?.body {
            ElementBody::OptionSet(s) => s.order,
            ElementBody::DesignOption(o) => o.order,
            _ => continue,
        };
        if current != order {
            tx.modify(*id, |e| {
                match &mut e.body {
                    ElementBody::OptionSet(s) => s.order = order,
                    ElementBody::DesignOption(o) => o.order = order,
                    _ => {}
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

fn ids_of(v: Vec<&crate::model::Element>) -> Vec<ElementId> {
    v.iter().map(|e| e.id).collect()
}

/// Create an option set (Revit: *Option Set*) with its first option
/// ("Option 1", primary, empty layer), after the other sets.
///
/// Fails with `ferro.model.invalid_name`, `ferro.model.duplicate_name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateOptionSet {
    pub name: String,
}

impl Command for CreateOptionSet {
    const NAME: &'static str = "create_option_set";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_name(
            &self.name,
            set_names(tx.doc()).into_iter(),
            None,
            "an option set",
        )?;
        let order = tx.doc().option_sets().len() as u32;
        let set = tx.create(
            Category::DesignOptions,
            Some(self.name.clone()),
            None,
            ElementBody::OptionSet(OptionSet { order }),
        )?;
        let option = tx.create(
            Category::DesignOptions,
            Some(FIRST_OPTION_NAME.into()),
            None,
            ElementBody::DesignOption(DesignOption {
                order: 0,
                is_primary: true,
                status: OptionStatus::Open,
                changes: Vec::new(),
            }),
        )?;
        tx.modify(option, |e| {
            e.host_id = Some(set);
            Ok(())
        })?;
        Ok(json!({ "element": set, "option": option }))
    }
}

/// Rename an option set or move it to `position` (0 = first; past the end
/// = last). The order decides in which order layers apply.
///
/// Fails with `ferro.model.not_an_option_set`, `ferro.model.invalid_name`,
/// `ferro.model.duplicate_name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetOptionSet {
    pub set: ElementId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<u32>,
}

impl Command for SetOptionSet {
    const NAME: &'static str = "set_option_set";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        require_set(tx, self.set)?;
        if let Some(n) = &self.name {
            check_name(
                n,
                set_names(tx.doc()).into_iter(),
                Some(self.set),
                "an option set",
            )?;
            let n = n.clone();
            tx.modify(self.set, |e| {
                e.name = Some(n);
                Ok(())
            })?;
        }
        let mut ids = ids_of(tx.doc().option_sets());
        if let Some(pos) = self.position {
            ids.retain(|s| *s != self.set);
            let at = (pos as usize).min(ids.len());
            ids.insert(at, self.set);
            renumber(tx, &ids)?;
        }
        Ok(json!({ "element": self.set, "sets": ids }))
    }
}

/// Delete an option set with all its options (open, accepted and archived;
/// Revit deletes a set with its options too). The shared model is not
/// changed: the options' layers simply stop being shown.
///
/// Fails with `ferro.model.not_an_option_set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteOptionSet {
    pub set: ElementId,
}

impl Command for DeleteOptionSet {
    const NAME: &'static str = "delete_option_set";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        require_set(tx, self.set)?;
        let options = ids_of(tx.doc().design_options(self.set));
        // Options are hosted by the set: deleting it deletes them.
        tx.delete(self.set)?;
        let ids = ids_of(tx.doc().option_sets());
        renumber(tx, &ids)?;
        Ok(json!({ "deleted": self.set, "options": options, "sets": ids }))
    }
}

/// Add an option to a set, after its other options; empty, or with a copy
/// of `copy_from`'s layer (any open option). Default name "Option N". It is
/// primary when the set has no other open option.
///
/// Fails with `ferro.model.not_an_option_set`,
/// `ferro.model.not_a_design_option` (`copy_from`),
/// `ferro.model.invalid_option_layer` (the copied layer does not apply),
/// `ferro.model.invalid_name`, `ferro.model.duplicate_name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateDesignOption {
    pub set: ElementId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub copy_from: Option<ElementId>,
}

impl Command for CreateDesignOption {
    const NAME: &'static str = "create_design_option";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        require_set(tx, self.set)?;
        let existing = option_names(tx.doc(), self.set);
        let name = match &self.name {
            Some(n) => n.clone(),
            None => {
                let mut k = existing.len() + 1;
                let mut n = format!("Option {k}");
                while existing
                    .iter()
                    .any(|(_, x)| x.as_deref() == Some(n.as_str()))
                {
                    k += 1;
                    n = format!("Option {k}");
                }
                n
            }
        };
        check_name(&name, existing.into_iter(), None, "an option of this set")?;
        let changes = match self.copy_from {
            Some(c) => {
                let (o, _) = require_option(tx, c)?;
                check_layer(tx.doc(), &o.changes).map_err(|e| CommandError::from(e).on(c))?;
                o.changes
            }
            None => Vec::new(),
        };
        let is_primary = tx.doc().primary_option(self.set).is_none();
        let order = tx.doc().design_options(self.set).len() as u32;
        let id = tx.create(
            Category::DesignOptions,
            Some(name),
            None,
            ElementBody::DesignOption(DesignOption {
                order,
                is_primary,
                status: OptionStatus::Open,
                changes,
            }),
        )?;
        let set = self.set;
        tx.modify(id, |e| {
            e.host_id = Some(set);
            Ok(())
        })?;
        Ok(json!({ "element": id, "is_primary": is_primary }))
    }
}

/// Edit an open design option: rename it, make it the primary option of
/// its set (`is_primary: true`; the previous primary stops being primary),
/// and / or replace its layer (`changes`, the change format: paths
/// `/elements/<uid>...`; whole elements to add, `remove` to hide a shared
/// element in this option, property paths to override). The layer is
/// checked against the shared model.
///
/// Fails with `ferro.model.not_a_design_option`,
/// `ferro.model.option_not_open`, `ferro.model.primary_required`
/// (`is_primary: false`: make another option primary instead),
/// `ferro.model.invalid_option_layer`, `ferro.model.invalid_name`,
/// `ferro.model.duplicate_name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetDesignOption {
    pub option: ElementId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub changes: Option<Vec<Change>>,
}

impl Command for SetDesignOption {
    const NAME: &'static str = "set_design_option";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let (opt, set) = require_option(tx, self.option)?;
        let label = tx.element(self.option)?.label();
        if let Some(n) = &self.name {
            check_name(
                n,
                option_names(tx.doc(), set).into_iter(),
                Some(self.option),
                "an option of this set",
            )?;
            let n = n.clone();
            tx.modify(self.option, |e| {
                e.name = Some(n);
                Ok(())
            })?;
        }
        if self.is_primary.is_some() || self.changes.is_some() {
            require_open(&opt, self.option, &label)?;
        }
        match self.is_primary {
            Some(false) if opt.is_primary => {
                return Err(CommandError::new(
                    "ferro.model.primary_required",
                    "each option set has one primary option; make another option primary instead",
                )
                .on(self.option));
            }
            Some(true) if !opt.is_primary => {
                for other in ids_of(tx.doc().design_options(set)) {
                    let me = other == self.option;
                    modify_option(tx, other, |o| o.is_primary = me)?;
                }
            }
            _ => {}
        }
        if let Some(changes) = &self.changes {
            check_layer(tx.doc(), changes).map_err(|e| CommandError::from(e).on(self.option))?;
            let changes = changes.clone();
            modify_option(tx, self.option, |o| o.changes = changes)?;
        }
        Ok(json!({ "element": self.option }))
    }
}

/// Delete a design option (open, accepted or archived) with its layer. The
/// primary option cannot be deleted: make another option primary first, or
/// delete the whole set.
///
/// Fails with `ferro.model.not_a_design_option`,
/// `ferro.model.option_is_primary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteDesignOption {
    pub option: ElementId,
}

impl Command for DeleteDesignOption {
    const NAME: &'static str = "delete_design_option";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let (opt, set) = require_option(tx, self.option)?;
        if opt.is_primary {
            return Err(CommandError::new(
                "ferro.model.option_is_primary",
                "the primary option cannot be deleted; make another option primary or delete the option set",
            )
            .on(self.option));
        }
        tx.delete(self.option)?;
        let ids = ids_of(tx.doc().design_options(set));
        renumber(tx, &ids)?;
        Ok(json!({ "deleted": self.option, "options": ids }))
    }
}

/// Accept a design option (ADR 0006 §4): its layer is merged into the
/// shared model like merging a branch. The option becomes *accepted* and
/// the other options of its set *archived*: kept with their layers, never
/// deleted. Layers of other sets that no longer apply are reported as
/// `ferro.model.option_layer_stale` warnings.
///
/// Fails with `ferro.model.not_a_design_option`,
/// `ferro.model.option_not_open`, `ferro.model.invalid_option_layer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptDesignOption {
    pub option: ElementId,
}

impl Command for AcceptDesignOption {
    const NAME: &'static str = "accept_design_option";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let (opt, set) = require_option(tx, self.option)?;
        let label = tx.element(self.option)?.label();
        require_open(&opt, self.option, &label)?;
        check_layer(tx.doc(), &opt.changes).map_err(|e| CommandError::from(e).on(self.option))?;
        let merged = apply(tx.doc(), &opt.changes)?;
        tx.replace_with(&merged);
        let mut archived = Vec::new();
        for other in ids_of(tx.doc().design_options(set)) {
            let me = other == self.option;
            let was_open = tx
                .element(other)?
                .as_design_option()
                .is_some_and(|o| o.status == OptionStatus::Open);
            modify_option(tx, other, |o| {
                o.is_primary = false;
                if me {
                    o.status = OptionStatus::Accepted;
                } else if o.status == OptionStatus::Open {
                    o.status = OptionStatus::Archived;
                }
            })?;
            if !me && was_open {
                archived.push(other);
            }
        }
        Ok(json!({ "accepted": self.option, "archived": archived, "merged": opt.changes.len() }))
    }
}

/// Apply a change list (the change format, M5-26: `[{op: set, path,
/// value} | {op: remove, path}]`, elements addressed by uid) to the
/// document, e.g. the output of `diff_documents` or a change from another
/// copy of the project. All or nothing.
///
/// Fails with `ferro.change.*` (bad path, missing target, reference to a
/// missing element, invalid element data) and
/// `ferro.model.invalid_design_options` (the result breaks the option-set
/// rules, e.g. two primary options in a set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplyChanges {
    pub changes: Vec<Change>,
}

impl Command for ApplyChanges {
    const NAME: &'static str = "apply_changes";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let out = apply(tx.doc(), &self.changes)?;
        check_design_options(&out)?;
        tx.replace_with(&out);
        Ok(json!({ "applied": self.changes.len() }))
    }
}
