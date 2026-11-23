use super::{
    builtin_definition, Category, Element, ElementBody, ElementId, ParameterDefinition,
    Relationship, Uid,
};
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

/// Project-wide information (Revit: `ProjectInfo` plus parameter bindings).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub origins: ProjectOrigins,
    /// Project (user-defined) parameter definitions, sorted by key, keys
    /// unique, never in the built-in `ferro` namespace (ADR 0004 §2.3).
    /// Omitted from files when empty, so files without it still load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_definitions: Vec<ParameterDefinition>,
    /// Display units, site location, currency (M5-24). Omitted when default.
    #[serde(default, skip_serializing_if = "is_default_settings")]
    pub settings: super::project_settings::ProjectSettings,
}

fn is_default_settings(s: &super::project_settings::ProjectSettings) -> bool {
    *s == super::project_settings::ProjectSettings::default()
}

/// The project container (Revit: `Document`). Cheap to clone: elements live in
/// a persistent map shared between versions (D-24).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    project: ProjectInfo,
    elements: OrdMap<ElementId, Arc<Element>>,
    /// The relationship table (ADR 0004 §5), keyed by uid.
    #[serde(default)]
    relationships: OrdMap<Uid, Relationship>,
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
            relationships: OrdMap::new(),
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
    pub fn from_parts(
        mut project: ProjectInfo,
        elements: Vec<Element>,
    ) -> Result<Document, String> {
        project
            .parameter_definitions
            .sort_by(|a, b| a.key.cmp(&b.key));
        for (i, d) in project.parameter_definitions.iter().enumerate() {
            if ParameterDefinition::is_builtin_key(&d.key) {
                return Err(format!("{} is a built-in parameter key", d.key));
            }
            if project.parameter_definitions[..i]
                .iter()
                .any(|o| o.key == d.key || o.uid == d.uid)
            {
                return Err(format!("duplicate parameter definition {}", d.key));
            }
        }
        let mut doc = Document {
            project,
            ..Default::default()
        };
        let mut uids = std::collections::HashSet::new();
        for mut e in elements {
            // Ids are renumbered on load: keep view lists sorted (M5-22).
            if let ElementBody::View(v) = &mut e.body {
                v.hidden_elements.sort();
                v.hidden_elements.dedup();
                v.hidden_tags.sort();
                v.hidden_tags.dedup();
            }
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
            for r in [e.phase_created, e.phase_demolished].into_iter().flatten() {
                if doc.get(r).and_then(|p| p.as_phase()).is_none() {
                    return Err(format!("{} has phase {r}, which is not a phase", e.id));
                }
            }
            if let Some(v) = e.as_view() {
                if let Some(r) = v.phase {
                    if doc.get(r).and_then(|p| p.as_phase()).is_none() {
                        return Err(format!("view {} has phase {r}, which is not a phase", e.id));
                    }
                }
                for r in v.hidden_elements.iter().chain(&v.design_options) {
                    if !doc.contains(*r) {
                        return Err(format!("view {} references missing element {r}", e.id));
                    }
                }
            }
        }
        Ok(doc)
    }

    /// Add a loaded relationship table to a document assembled with
    /// [`Document::from_parts`]. Every row is validated
    /// ([`Relationship::validate`]); uids must be unique and differ from
    /// element uids.
    pub fn with_relationships(mut self, rows: Vec<Relationship>) -> Result<Document, String> {
        for r in rows {
            if self.relationships.contains_key(&r.uid) || self.find_uid(r.uid).is_some() {
                return Err(format!("duplicate uid {}", r.uid));
            }
            r.validate(&self).map_err(|e| e.to_string())?;
            self.relationships.insert(r.uid, r);
        }
        Ok(self)
    }

    // ---------------------------------------------------------------- reads

    /// The relationship table, in uid order.
    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> {
        self.relationships.values()
    }

    /// The relationship with this uid.
    pub fn relationship(&self, uid: Uid) -> Option<&Relationship> {
        self.relationships.get(&uid)
    }

    /// Relationships that link `id`, in uid order.
    pub fn relationships_of(&self, id: ElementId) -> impl Iterator<Item = &Relationship> {
        self.relationships.values().filter(move |r| r.involves(id))
    }

    pub fn project(&self) -> &ProjectInfo {
        &self.project
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    /// The definition of a parameter key: the built-in one for `ferro.*`
    /// keys, else the project's. `None` if the key has no definition (any
    /// typed value is then allowed, D-15).
    pub fn parameter_definition(&self, key: &str) -> Option<ParameterDefinition> {
        builtin_definition(key).or_else(|| {
            self.project
                .parameter_definitions
                .iter()
                .find(|d| d.key == key)
                .cloned()
        })
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
    /// Remove an element and every relationship that links it (a row
    /// never points at a missing element).
    pub(crate) fn remove(&mut self, id: ElementId) -> Option<Arc<Element>> {
        let gone: Vec<Uid> = self.relationships_of(id).map(|r| r.uid).collect();
        for u in gone {
            self.relationships.remove(&u);
        }
        self.elements.remove(&id)
    }
    /// Insert or replace a relationship row. The caller validates it.
    pub(crate) fn put_relationship(&mut self, r: Relationship) {
        self.relationships.insert(r.uid, r);
    }
    pub(crate) fn remove_relationship(&mut self, uid: Uid) -> Option<Relationship> {
        self.relationships.remove(&uid)
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
