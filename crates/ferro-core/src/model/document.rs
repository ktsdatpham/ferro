use super::{Category, Element, ElementId, Uid};
use crate::math::{Angle, Point3};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Geometry must stay within this distance of the Internal Origin (D-05).
pub const EXTENT_LIMIT_MM: f64 = 10_000_000.0;

/// Revit's three-point system (D-04). Positions are relative to the
/// Internal Origin, which never moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectOrigins {
    pub project_base_point: Point3,
    pub survey_point: Point3,
    /// Rotation from project north to true north, counter-clockwise.
    pub true_north: Angle,
}

impl Default for ProjectOrigins {
    fn default() -> Self {
        ProjectOrigins {
            project_base_point: Point3::ORIGIN,
            survey_point: Point3::ORIGIN,
            true_north: Angle::ZERO,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub origins: ProjectOrigins,
}

/// The project container (Revit: `Document`). Cheap to clone: elements live in
/// a persistent map shared between versions (D-24).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    project: ProjectInfo,
    elements: OrdMap<ElementId, Arc<Element>>,
    next_element_id: u64,
    /// Incremented on every commit.
    version: u64,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            project: ProjectInfo {
                name: "Untitled".into(),
                ..Default::default()
            },
            elements: OrdMap::new(),
            next_element_id: 1,
            version: 0,
        }
    }
}

impl Document {
    pub fn new(name: impl Into<String>) -> Document {
        Document {
            project: ProjectInfo {
                name: name.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Assemble a document from loaded parts (opening a file, importing).
    /// This creates a new document; it does not mutate an existing one, so it
    /// does not bypass D-17. Ids and uids must be unique.
    pub fn from_parts(project: ProjectInfo, elements: Vec<Element>) -> Result<Document, String> {
        let mut doc = Document {
            project,
            ..Default::default()
        };
        let mut uids = std::collections::HashSet::new();
        for e in elements {
            if doc.contains(e.id) {
                return Err(format!("duplicate element id {}", e.id));
            }
            if !uids.insert(e.uid) {
                return Err(format!("duplicate uid {}", e.uid));
            }
            doc.put(e);
        }
        for e in doc.elements() {
            for r in [e.level_id, e.type_id, e.host_id, e.design_option]
                .into_iter()
                .flatten()
            {
                if !doc.contains(r) {
                    return Err(format!("{} references missing element {r}", e.id));
                }
            }
        }
        Ok(doc)
    }

    // ---------------------------------------------------------------- reads

    pub fn project(&self) -> &ProjectInfo {
        &self.project
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn get(&self, id: ElementId) -> Option<&Element> {
        self.elements.get(&id).map(|e| e.as_ref())
    }
    pub fn contains(&self, id: ElementId) -> bool {
        self.elements.contains_key(&id)
    }
    pub fn len(&self) -> usize {
        self.elements.len()
    }
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
    /// All elements in id order (deterministic).
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.elements.values().map(|e| e.as_ref())
    }
    pub fn by_category(&self, c: Category) -> impl Iterator<Item = &Element> {
        self.elements().filter(move |e| e.category == c)
    }
    pub fn find_uid(&self, uid: Uid) -> Option<&Element> {
        self.elements().find(|e| e.uid == uid)
    }
    /// Levels sorted by elevation.
    pub fn levels(&self) -> Vec<&Element> {
        let mut v: Vec<&Element> = self.by_category(Category::Levels).collect();
        v.sort_by(|a, b| {
            let ea = a.as_level().map(|l| l.elevation.mm()).unwrap_or(0.0);
            let eb = b.as_level().map(|l| l.elevation.mm()).unwrap_or(0.0);
            ea.total_cmp(&eb)
        });
        v
    }

    /// `true` if `other` shares this element's storage (unchanged since).
    pub fn same_element(&self, other: &Document, id: ElementId) -> bool {
        match (self.elements.get(&id), other.elements.get(&id)) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    // ------------------------------------------------- crate-private writes

    pub(crate) fn alloc_id(&mut self) -> ElementId {
        let id = ElementId(self.next_element_id);
        self.next_element_id += 1;
        id
    }
    pub(crate) fn put(&mut self, e: Element) {
        self.next_element_id = self.next_element_id.max(e.id.0 + 1);
        self.elements.insert(e.id, Arc::new(e));
    }
    pub(crate) fn remove(&mut self, id: ElementId) -> Option<Arc<Element>> {
        self.elements.remove(&id)
    }
    pub(crate) fn project_mut(&mut self) -> &mut ProjectInfo {
        &mut self.project
    }
    pub(crate) fn bump_version(&mut self) {
        self.version += 1;
    }
    pub(crate) fn element_ids(&self) -> Vec<ElementId> {
        self.elements.keys().copied().collect()
    }
}
