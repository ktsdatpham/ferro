//! Commands on the relationship table (ADR 0004 §5, M5-5.1).

use super::{Command, CommandError, CommandOutput, Transaction};
use crate::model::{ElementId, RelationshipKind, Uid};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Add a relationship row linking `elements` (ordered as the kind says:
/// `join` `[first, second]`, `cut` `[cutter, cut]`, `host` `[host,
/// hosted]`, `attach` `[attached, target]`, `bounds` `[space, boundary]`,
/// `connect` `[a, b]`, `aggregate` `[whole, part, …]`), optionally made in
/// `phase`. Returns the new row's `uid`. Fails with
/// `ferro.model.invalid_relationship` (wrong count, repeated element,
/// element without geometry, phase not a phase),
/// `ferro.model.missing_element` or `ferro.model.duplicate_relationship`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateRelationship {
    pub kind: RelationshipKind,
    pub elements: Vec<ElementId>,
    #[serde(default)]
    pub phase: Option<ElementId>,
}

impl Command for CreateRelationship {
    const NAME: &'static str = "create_relationship";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let uid = tx.add_relationship(self.kind, self.elements.clone(), self.phase)?;
        Ok(json!({ "relationship": uid.to_string() }))
    }
}

/// Change a relationship row in place, keeping its `uid` (so IFC ids and
/// change lists stay stable). `elements` replaces the linked elements;
/// `reverse: true` swaps the order of a two-element row, which for a `join`
/// is Revit's *Switch Join Order* (the other element now wins). Omitted
/// fields keep their value. Fails with `ferro.model.invalid_uid`,
/// `ferro.model.missing_relationship`, `ferro.model.invalid_relationship`
/// (also when `reverse` is given for a row of more than two elements or
/// together with `elements`), `ferro.model.missing_element` or
/// `ferro.model.duplicate_relationship`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetRelationship {
    pub relationship: String,
    #[serde(default)]
    pub elements: Option<Vec<ElementId>>,
    #[serde(default)]
    pub reverse: bool,
}

impl Command for SetRelationship {
    const NAME: &'static str = "set_relationship";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let uid = parse_uid(&self.relationship)?;
        let mut r = tx.doc().relationship(uid).cloned().ok_or_else(|| {
            CommandError::new(
                "ferro.model.missing_relationship",
                format!("no relationship {uid}"),
            )
        })?;
        let invalid = |m: &str| CommandError::new("ferro.model.invalid_relationship", m);
        match (&self.elements, self.reverse) {
            (Some(_), true) => return Err(invalid("give `elements` or `reverse`, not both")),
            (Some(e), false) => r.elements = e.clone(),
            (None, true) if r.elements.len() == 2 => r.elements.reverse(),
            (None, true) => return Err(invalid("only a two-element row can be reversed")),
            (None, false) => {}
        }
        tx.put_relationship(r.clone())?;
        Ok(json!({ "relationship": uid.to_string(), "elements": r.elements }))
    }
}

fn parse_uid(s: &str) -> Result<Uid, CommandError> {
    s.trim()
        .parse()
        .map_err(|_| CommandError::new("ferro.model.invalid_uid", format!("`{s}` is not a uid")))
}

/// Remove a relationship row by `uid` (the elements stay). Fails with
/// `ferro.model.missing_relationship` or `ferro.model.invalid_uid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteRelationship {
    pub relationship: String,
}

impl Command for DeleteRelationship {
    const NAME: &'static str = "delete_relationship";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let uid = parse_uid(&self.relationship)?;
        let r = tx.remove_relationship(uid)?;
        Ok(json!({ "deleted": uid.to_string(), "elements": r.elements }))
    }
}
