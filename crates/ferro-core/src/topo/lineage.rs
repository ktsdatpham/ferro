use super::FaceId;
use serde::{Deserialize, Serialize};

/// What a topological operation did to faces (D-25). References held by
/// elements, materials, dimensions or AI use this to follow faces across edits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lineage {
    /// Faces that still exist with the same id (possibly moved or reshaped).
    pub kept: Vec<FaceId>,
    /// New faces, with the face they were derived from when there is one.
    pub created: Vec<(FaceId, Option<FaceId>)>,
    /// Faces that no longer exist.
    pub deleted: Vec<FaceId>,
}

impl Lineage {
    pub fn created_ids(&self) -> impl Iterator<Item = FaceId> + '_ {
        self.created.iter().map(|(f, _)| *f)
    }
}
