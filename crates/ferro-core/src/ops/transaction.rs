use super::{CommandError, DocumentChanged, Failure};
use crate::math::Vec3;
use crate::model::{
    keys, Category, Document, Element, ElementBody, ElementId, ParamValue, Uid, EXTENT_LIMIT_MM,
};
use std::collections::BTreeMap;

/// Supplies new `Uid`s. The kernel never reads clocks or randomness itself
/// (D-19): the app passes a time/random-based source, tests a sequential one.
pub trait IdSource {
    fn next_uid(&mut self) -> Uid;
}

/// Deterministic ids: UUID v7 with a fixed timestamp and a counter.
#[derive(Debug, Clone)]
pub struct SequentialIds {
    pub unix_ms: u64,
    pub counter: u128,
}

impl SequentialIds {
    pub fn new(unix_ms: u64) -> Self {
        SequentialIds {
            unix_ms,
            counter: 0,
        }
    }
}

impl IdSource for SequentialIds {
    fn next_uid(&mut self) -> Uid {
        self.counter += 1;
        Uid::v7(self.unix_ms, self.counter)
    }
}

/// An atomic unit of change (§4.3). Commands mutate a working copy; `finish`
/// regenerates, validates and either yields the new version or the errors.
pub struct Transaction<'a> {
    name: String,
    base: Document,
    work: Document,
    ids: &'a mut dyn IdSource,
    warnings: Vec<Failure>,
}

impl<'a> Transaction<'a> {
    pub fn new(name: impl Into<String>, base: &Document, ids: &'a mut dyn IdSource) -> Self {
        Transaction {
            name: name.into(),
            base: base.clone(),
            work: base.clone(),
            ids,
            warnings: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The working copy (reads see this transaction's own changes).
    pub fn doc(&self) -> &Document {
        &self.work
    }

    pub fn element(&self, id: ElementId) -> Result<&Element, CommandError> {
        self.work.get(id).ok_or_else(|| CommandError::not_found(id))
    }

    /// Create an element and return its id.
    pub fn create(
        &mut self,
        category: Category,
        name: Option<String>,
        level_id: Option<ElementId>,
        body: ElementBody,
    ) -> Result<ElementId, CommandError> {
        if let Some(l) = level_id {
            let lvl = self.element(l)?;
            if lvl.category != Category::Levels {
                return Err(CommandError::new(
                    "ferro.model.not_a_level",
                    format!("{l} is not a level"),
                )
                .on(l));
            }
        }
        let id = self.work.alloc_id();
        let uid = self.ids.next_uid();
        self.work.put(Element {
            id,
            uid,
            category,
            name,
            type_id: None,
            level_id,
            host_id: None,
            design_option: None,
            phase_created: None,
            phase_demolished: None,
            params: BTreeMap::new(),
            body,
        });
        Ok(id)
    }

    /// Duplicate an element with a new id and uid (Q-B7: copies get both).
    pub fn duplicate(&mut self, id: ElementId) -> Result<ElementId, CommandError> {
        let mut e = self.element(id)?.clone();
        e.id = self.work.alloc_id();
        e.uid = self.ids.next_uid();
        let new = e.id;
        self.work.put(e);
        Ok(new)
    }

    /// Modify an element in place.
    pub fn modify<R>(
        &mut self,
        id: ElementId,
        f: impl FnOnce(&mut Element) -> Result<R, CommandError>,
    ) -> Result<R, CommandError> {
        let mut e = self.element(id)?.clone();
        let r = f(&mut e)?;
        e.id = id; // identity is not editable
        self.work.put(e);
        Ok(r)
    }

    /// Delete an element. References from other elements are cleared and
    /// reported as warnings.
    pub fn delete(&mut self, id: ElementId) -> Result<(), CommandError> {
        self.work
            .remove(id)
            .ok_or_else(|| CommandError::not_found(id))?;
        for other in self.work.element_ids() {
            let e = self.work.get(other).expect("listed");
            if e.level_id == Some(id) || e.host_id == Some(id) {
                let mut e = e.clone();
                if e.level_id == Some(id) {
                    e.level_id = None;
                }
                if e.host_id == Some(id) {
                    e.host_id = None;
                }
                self.warnings.push(Failure::warning(
                    "ferro.model.reference_cleared",
                    format!("{} lost its reference to deleted {id}", e.label()),
                    vec![other],
                ));
                self.work.put(e);
            }
        }
        Ok(())
    }

    pub fn warn(&mut self, f: Failure) {
        self.warnings.push(f);
    }

    pub fn project_name(&mut self, name: String) {
        self.work.project_mut().name = name;
    }

    /// Regenerate (§4.4), validate and produce the committed version.
    pub(crate) fn finish(
        mut self,
    ) -> Result<(Document, DocumentChanged, Vec<Failure>), Vec<Failure>> {
        let mut errors = Vec::new();

        // 1. Levels whose elevation changed move their dependent Forms.
        let mut level_moves: Vec<(ElementId, f64)> = Vec::new();
        for lvl in self.work.by_category(Category::Levels) {
            if let (Some(new), Some(old)) = (
                lvl.as_level(),
                self.base.get(lvl.id).and_then(|e| e.as_level()),
            ) {
                let d = new.elevation.mm() - old.elevation.mm();
                if d != 0.0 {
                    level_moves.push((lvl.id, d));
                }
            }
        }
        for (lvl, dz) in level_moves {
            let deps: Vec<ElementId> = self
                .work
                .elements()
                .filter(|e| e.level_id == Some(lvl))
                .map(|e| e.id)
                .collect();
            for d in deps {
                let mut e = self.work.get(d).expect("listed").clone();
                if let ElementBody::Form(f) = &mut e.body {
                    f.body
                        .transform(&crate::math::Transform::translation(Vec3::new(
                            0.0, 0.0, dz,
                        )));
                }
                self.work.put(e);
            }
        }

        // 2. Computed parameters and validation of changed elements.
        let changed = DocumentChanged::between(&self.name, &self.base, &self.work);
        for id in changed.added.iter().chain(&changed.modified) {
            let Some(e) = self.work.get(*id) else {
                continue;
            };
            let mut e = e.clone();
            if let ElementBody::Form(f) = &e.body {
                if let Err(err) = f.body.validate() {
                    errors.push(Failure::error(err.code(), err.to_string(), vec![*id]));
                    continue;
                }
                if let Some(bb) = f.body.bounding_box() {
                    let far = [bb.min, bb.max].iter().any(|p| {
                        p.x.abs() > EXTENT_LIMIT_MM
                            || p.y.abs() > EXTENT_LIMIT_MM
                            || p.z.abs() > EXTENT_LIMIT_MM
                    });
                    if far {
                        errors.push(Failure::error(
                            "ferro.model.out_of_extent",
                            "geometry must stay within 10 km of the Internal Origin (D-05)",
                            vec![*id],
                        ));
                        continue;
                    }
                }
                let area: f64 = f
                    .body
                    .face_ids()
                    .filter_map(|fid| f.body.face_area(fid).ok())
                    .sum();
                e.params
                    .insert(keys::FORM_AREA.into(), ParamValue::Area(area));
                if f.body.is_closed() {
                    e.params.insert(
                        keys::FORM_VOLUME.into(),
                        ParamValue::Volume(f.body.volume()),
                    );
                } else {
                    e.params.remove(keys::FORM_VOLUME);
                }
            }
            if self.work.get(*id) != Some(&e) {
                self.work.put(e);
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }
        let changed = DocumentChanged::between(&self.name, &self.base, &self.work);
        if !changed.is_empty() || self.base.project() != self.work.project() {
            self.work.bump_version();
        }
        Ok((self.work, changed, self.warnings))
    }
}
