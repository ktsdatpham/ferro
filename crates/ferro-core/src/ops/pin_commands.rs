//! Pin (ADR 0008, D-79, "like Revit"): the `set_pinned` command. What a pin
//! blocks (moving and deleting) is enforced by the `Transaction` itself
//! (`Element::pin_moved`), so every path obeys it. Anyone who may edit may
//! unpin: the user, agents under their normal autonomy, scripts, plugins.

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::model::ElementId;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Pin or unpin elements (Revit *Pin* / *Unpin*, `Element.Pinned`). A
/// pinned element cannot be moved or deleted (`ferro.model.pinned`) until
/// unpinned; every other edit stays allowed. Elements already in the asked
/// state are left as they are. Returns `{elements, changed}` (`changed` =
/// ids whose pin actually changed).
///
/// Fails with `ferro.model.not_found` for a missing id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPinned {
    pub elements: Vec<ElementId>,
    pub pinned: bool,
}

impl Command for SetPinned {
    const NAME: &'static str = "set_pinned";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let mut changed = Vec::new();
        for id in &self.elements {
            if tx.element(*id)?.pinned == self.pinned {
                continue;
            }
            let pinned = self.pinned;
            tx.modify(*id, |e| {
                e.pinned = pinned;
                Ok(())
            })?;
            changed.push(*id);
        }
        Ok(json!({ "elements": self.elements, "changed": changed }))
    }
}
