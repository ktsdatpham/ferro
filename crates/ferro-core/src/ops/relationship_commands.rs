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

/// Remove a relationship row by `uid` (the elements stay). Fails with
/// `ferro.model.missing_relationship` or `ferro.model.invalid_uid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteRelationship {
    pub relationship: String,
}

impl Command for DeleteRelationship {
    const NAME: &'static str = "delete_relationship";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let uid: Uid = self.relationship.trim().parse().map_err(|_| {
            CommandError::new(
                "ferro.model.invalid_uid",
                format!("`{}` is not a uid", self.relationship),
            )
        })?;
        let r = tx.remove_relationship(uid)?;
        Ok(json!({ "deleted": uid.to_string(), "elements": r.elements }))
    }
}
