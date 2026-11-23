//! Phase commands (ADR 0006 §Phases, M5-23): the phase list and the phase
//! fields of elements. Each `NAME` is also the MCP tool name.
//!
//! New documents have no phases. The first time phasing is used through
//! the simple mode ([`SetRenovationStatus`]) the two default phases
//! "Existing" and "New Construction" are created; [`CreatePhase`] on an
//! empty list creates just the phase asked for.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::model::phase::{
    is_phased_category, is_valid_date, Phase, EXISTING_PHASE_NAME, NEW_CONSTRUCTION_PHASE_NAME,
};
use crate::model::{Category, Document, ElementBody, ElementId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

/// Deserialise a field where a missing key means "leave unchanged" and
/// `null` means "clear".
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

fn not_a_phase(id: ElementId) -> CommandError {
    CommandError::new("ferro.model.not_a_phase", format!("{id} is not a phase")).on(id)
}

/// Phase ids in order.
fn phase_ids(doc: &Document) -> Vec<ElementId> {
    doc.phases().iter().map(|e| e.id).collect()
}

fn require_phase(tx: &Transaction, id: ElementId) -> Result<(), CommandError> {
    match tx.element(id)?.as_phase() {
        Some(_) => Ok(()),
        None => Err(not_a_phase(id)),
    }
}

/// Write `order = position` for every phase in `ids`.
fn renumber(tx: &mut Transaction, ids: &[ElementId]) -> Result<(), CommandError> {
    for (i, id) in ids.iter().enumerate() {
        let order = i as u32;
        if tx.element(*id)?.as_phase().map(|p| p.order) != Some(order) {
            tx.modify(*id, |e| match &mut e.body {
                ElementBody::Phase(p) => {
                    p.order = order;
                    Ok(())
                }
                _ => Err(not_a_phase(e.id)),
            })?;
        }
    }
    Ok(())
}

fn check_name_free(
    tx: &Transaction,
    name: &str,
    except: Option<ElementId>,
) -> Result<(), CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::new(
            "ferro.model.invalid_name",
            "a phase needs a name",
        ));
    }
    if tx
        .doc()
        .by_category(Category::Phases)
        .any(|p| Some(p.id) != except && p.name.as_deref() == Some(name))
    {
        return Err(CommandError::new(
            "ferro.model.duplicate_name",
            format!("a phase named {name:?} already exists"),
        ));
    }
    Ok(())
}

fn check_dates(start: Option<&str>, end: Option<&str>) -> Result<(), CommandError> {
    for d in [start, end].into_iter().flatten() {
        if !is_valid_date(d) {
            return Err(CommandError::new(
                "ferro.model.invalid_date",
                format!("{d:?} is not a date (YYYY-MM-DD)"),
            ));
        }
    }
    if let (Some(s), Some(e)) = (start, end) {
        // ISO dates compare as text.
        if e < s {
            return Err(CommandError::new(
                "ferro.model.invalid_date",
                format!("the phase ends ({e}) before it starts ({s})"),
            ));
        }
    }
    Ok(())
}

/// Every element's demolished phase must not be earlier than its created
/// phase. Fails with `ferro.model.invalid_phase_order` naming the first
/// offender.
fn check_element_phase_order(doc: &Document) -> Result<(), CommandError> {
    let ids = phase_ids(doc);
    let pos = |id: Option<ElementId>| id.and_then(|i| ids.iter().position(|p| *p == i));
    for e in doc.elements() {
        if let (Some(c), Some(d)) = (pos(e.phase_created), pos(e.phase_demolished)) {
            if d < c {
                return Err(CommandError::new(
                    "ferro.model.invalid_phase_order",
                    format!(
                        "{} would be demolished in an earlier phase than it is created in",
                        e.label()
                    ),
                )
                .on(e.id));
            }
        }
    }
    Ok(())
}

/// Create a phase named `name` at the end of the list (or before phase
/// `before`). Optional dates `YYYY-MM-DD`.
///
/// Fails with `ferro.model.duplicate_name`, `ferro.model.invalid_name`,
/// `ferro.model.invalid_date`, `ferro.model.not_a_phase` (`before`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatePhase {
    pub name: String,
    #[serde(default)]
    pub before: Option<ElementId>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

impl Command for CreatePhase {
    const NAME: &'static str = "create_phase";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_name_free(tx, &self.name, None)?;
        check_dates(self.start_date.as_deref(), self.end_date.as_deref())?;
        let mut ids = phase_ids(tx.doc());
        let at = match self.before {
            Some(b) => {
                require_phase(tx, b)?;
                ids.iter().position(|p| *p == b).unwrap_or(ids.len())
            }
            None => ids.len(),
        };
        let id = tx.create(
            Category::Phases,
            Some(self.name.clone()),
            None,
            ElementBody::Phase(Phase {
                order: at as u32,
                start_date: self.start_date.clone(),
                end_date: self.end_date.clone(),
            }),
        )?;
        ids.insert(at, id);
        renumber(tx, &ids)?;
        Ok(json!({ "element": id, "phases": ids }))
    }
}

/// Create the phases "Existing" and "New Construction" if the project has
/// fewer than two phases (the first time the simple mode is used). Returns
/// the first and last phase.
pub(crate) fn ensure_default_phases(
    tx: &mut Transaction,
) -> Result<(ElementId, ElementId, Vec<ElementId>), CommandError> {
    let mut created = Vec::new();
    for wanted in [EXISTING_PHASE_NAME, NEW_CONSTRUCTION_PHASE_NAME] {
        if phase_ids(tx.doc()).len() >= 2 {
            break;
        }
        // With one user phase already there, it plays "Existing" and only
        // "New Construction" is added (with a free name).
        if wanted == EXISTING_PHASE_NAME && !phase_ids(tx.doc()).is_empty() {
            continue;
        }
        let mut name = wanted.to_string();
        let mut n = 2;
        while check_name_free(tx, &name, None).is_err() {
            name = format!("{wanted} {n}");
            n += 1;
        }
        let out = CreatePhase {
            name,
            before: None,
            start_date: None,
            end_date: None,
        }
        .execute(tx)?;
        if let Some(id) = out["element"].as_u64() {
            created.push(ElementId(id));
        }
    }
    let ids = phase_ids(tx.doc());
    match (ids.first(), ids.last()) {
        (Some(f), Some(l)) if f != l => Ok((*f, *l, created)),
        _ => Err(CommandError::new(
            "ferro.model.not_a_phase",
            "the default phases could not be created",
        )),
    }
}

/// Rename a phase, move it to `position` (0 = first; past the end = last)
/// or change its dates (`""` clears a date).
///
/// Fails with `ferro.model.not_a_phase`, `ferro.model.duplicate_name`,
/// `ferro.model.invalid_name`, `ferro.model.invalid_date`, and
/// `ferro.model.invalid_phase_order` when moving the phase would put some
/// element's demolition before its creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPhase {
    pub phase: ElementId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<u32>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

impl Command for SetPhase {
    const NAME: &'static str = "set_phase";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        require_phase(tx, self.phase)?;
        if let Some(n) = &self.name {
            check_name_free(tx, n, Some(self.phase))?;
        }
        let old = tx.element(self.phase)?.as_phase().cloned();
        let pick = |new: &Option<String>, old: Option<String>| match new.as_deref() {
            Some("") => None,
            Some(d) => Some(d.to_string()),
            None => old,
        };
        let start = pick(
            &self.start_date,
            old.as_ref().and_then(|p| p.start_date.clone()),
        );
        let end = pick(
            &self.end_date,
            old.as_ref().and_then(|p| p.end_date.clone()),
        );
        check_dates(start.as_deref(), end.as_deref())?;
        let name = self.name.clone();
        tx.modify(self.phase, |e| {
            if let Some(n) = name {
                e.name = Some(n);
            }
            match &mut e.body {
                ElementBody::Phase(p) => {
                    p.start_date = start;
                    p.end_date = end;
                    Ok(())
                }
                _ => Err(not_a_phase(e.id)),
            }
        })?;
        let mut ids = phase_ids(tx.doc());
        if let Some(pos) = self.position {
            ids.retain(|p| *p != self.phase);
            let at = (pos as usize).min(ids.len());
            ids.insert(at, self.phase);
            renumber(tx, &ids)?;
            check_element_phase_order(tx.doc())?;
        }
        Ok(json!({ "element": self.phase, "phases": ids }))
    }
}

/// Delete a phase. A phase used by elements can only be deleted by merging
/// it into the phase just before or after it (`merge_into`; Revit: combine
/// with previous / next): its elements and views then use that phase.
/// Views alone do not keep a phase in use: deleting it without `merge_into`
/// clears their phase (they show the last phase).
///
/// Fails with `ferro.model.not_a_phase`, `ferro.model.phase_in_use` (used
/// and no `merge_into`) and `ferro.model.phase_not_adjacent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeletePhase {
    pub phase: ElementId,
    #[serde(default)]
    pub merge_into: Option<ElementId>,
}

impl Command for DeletePhase {
    const NAME: &'static str = "delete_phase";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        require_phase(tx, self.phase)?;
        let mut ids = phase_ids(tx.doc());
        let mut moved = Vec::new();
        if let Some(into) = self.merge_into {
            require_phase(tx, into)?;
            let (Some(a), Some(b)) = (
                ids.iter().position(|p| *p == self.phase),
                ids.iter().position(|p| *p == into),
            ) else {
                return Err(not_a_phase(into));
            };
            if a.abs_diff(b) != 1 {
                return Err(CommandError::new(
                    "ferro.model.phase_not_adjacent",
                    "a phase can only be merged into the phase just before or after it",
                )
                .on(into));
            }
            let users: Vec<ElementId> = tx
                .doc()
                .elements()
                .filter(|e| {
                    e.phase_created == Some(self.phase) || e.phase_demolished == Some(self.phase)
                })
                .map(|e| e.id)
                .collect();
            let gone = self.phase;
            for u in users {
                tx.modify(u, |e| {
                    if e.phase_created == Some(gone) {
                        e.phase_created = Some(into);
                    }
                    if e.phase_demolished == Some(gone) {
                        e.phase_demolished = Some(into);
                    }
                    Ok(())
                })?;
                moved.push(u);
            }
            let rows: Vec<_> = tx
                .doc()
                .relationships()
                .filter(|r| r.phase == Some(gone))
                .cloned()
                .collect();
            for mut r in rows {
                r.phase = Some(into);
                tx.put_relationship(r)?;
            }
            // Views of the merged phase show the phase it merged into.
            let views: Vec<ElementId> = tx
                .doc()
                .by_category(Category::Views)
                .filter(|e| e.as_view().is_some_and(|v| v.phase == Some(gone)))
                .map(|e| e.id)
                .collect();
            for v in views {
                tx.modify(v, |e| {
                    if let ElementBody::View(view) = &mut e.body {
                        view.phase = Some(into);
                    }
                    Ok(())
                })?;
            }
        }
        tx.delete(self.phase)?;
        ids.retain(|p| *p != self.phase);
        renumber(tx, &ids)?;
        Ok(json!({ "deleted": self.phase, "moved": moved, "phases": ids }))
    }
}

/// Set the phase an element is created in and / or demolished in (Revit:
/// *Phase Created*, *Phase Demolished*). A missing field is left unchanged;
/// `null` clears it.
///
/// Fails with `ferro.model.not_a_phase`, `ferro.model.not_phased` (levels,
/// types, materials, phases) and `ferro.model.invalid_phase_order`
/// (demolished earlier than created).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetElementPhase {
    pub elements: Vec<ElementId>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<Option<ElementId>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub demolished: Option<Option<ElementId>>,
}

impl Command for SetElementPhase {
    const NAME: &'static str = "set_element_phase";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        for p in [self.created, self.demolished]
            .into_iter()
            .flatten()
            .flatten()
        {
            require_phase(tx, p)?;
        }
        set_fields(tx, &self.elements, self.created, self.demolished)?;
        Ok(json!({ "elements": self.elements }))
    }
}

fn set_fields(
    tx: &mut Transaction,
    elements: &[ElementId],
    created: Option<Option<ElementId>>,
    demolished: Option<Option<ElementId>>,
) -> Result<(), CommandError> {
    for id in elements {
        let e = tx.element(*id)?;
        if !is_phased_category(e) {
            return Err(CommandError::new(
                "ferro.model.not_phased",
                format!(
                    "{} has no phases (only elements placed in the model do)",
                    e.label()
                ),
            )
            .on(*id));
        }
        tx.modify(*id, |e| {
            if let Some(c) = created {
                e.phase_created = c;
            }
            if let Some(d) = demolished {
                e.phase_demolished = d;
            }
            Ok(())
        })?;
    }
    check_element_phase_order(tx.doc())
}

/// Renovation status of the simple mode (ADR 0006 §4, ArchiCAD-like).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenovationStatus {
    /// Created in the first phase, not demolished.
    Existing,
    /// Created in the first phase, demolished in the last.
    Demolish,
    /// Created in the last phase.
    New,
}

/// Simple mode: mark elements as existing, to be demolished or new. Maps to
/// `phase_created` / `phase_demolished` on the first and last phase,
/// creating "Existing" and "New Construction" if the project has fewer than
/// two phases.
///
/// Fails with `ferro.model.not_phased`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetRenovationStatus {
    pub elements: Vec<ElementId>,
    pub status: RenovationStatus,
}

impl Command for SetRenovationStatus {
    const NAME: &'static str = "set_renovation_status";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let (first, last, created) = ensure_default_phases(tx)?;
        let (c, d) = match self.status {
            RenovationStatus::Existing => (first, None),
            RenovationStatus::Demolish => (first, Some(last)),
            RenovationStatus::New => (last, None),
        };
        set_fields(tx, &self.elements, Some(Some(c)), Some(d))?;
        Ok(json!({ "elements": self.elements, "phases_created": created }))
    }
}
