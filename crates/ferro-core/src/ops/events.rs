use crate::model::{Document, ElementId};
use serde::{Deserialize, Serialize};

/// Emitted after every commit, undo and redo (§4.6). Renderer, UI, MCP and the
/// Revit link subscribe to this; nobody polls the document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentChanged {
    pub transaction_name: String,
    pub added: Vec<ElementId>,
    pub modified: Vec<ElementId>,
    pub deleted: Vec<ElementId>,
}

impl DocumentChanged {
    /// Diff two versions. Unchanged elements share storage, so this is fast.
    pub fn between(name: &str, before: &Document, after: &Document) -> DocumentChanged {
        let mut c = DocumentChanged {
            transaction_name: name.to_string(),
            ..Default::default()
        };
        for e in after.elements() {
            if !before.contains(e.id) {
                c.added.push(e.id);
            } else if !before.same_element(after, e.id) && before.get(e.id) != Some(e) {
                c.modified.push(e.id);
            }
        }
        for e in before.elements() {
            if !after.contains(e.id) {
                c.deleted.push(e.id);
            }
        }
        // A changed relationship changes the generated geometry of the
        // elements it links (a join, a cut): report them as modified.
        let mut touched: Vec<ElementId> = Vec::new();
        for r in after.relationships() {
            if before.relationship(r.uid) != Some(r) {
                touched.extend(&r.elements);
            }
        }
        for r in before.relationships() {
            if after.relationship(r.uid).is_none() {
                touched.extend(&r.elements);
            }
        }
        touched.sort();
        touched.dedup();
        for id in touched {
            if after.contains(id) && before.contains(id) && !c.modified.contains(&id) {
                c.modified.push(id);
            }
        }
        c.modified.sort();
        c
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }
}
