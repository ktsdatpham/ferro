use super::{CommandError, DocumentChanged, Failure};
use crate::math::{tol, Vec3};
use crate::model::bim::WallTop;
use crate::model::generate;
use crate::model::{
    keys, Category, Document, Element, ElementBody, ElementId, ParamValue, Relationship,
    RelationshipKind, Uid, ViewType, EXTENT_LIMIT_MM,
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

    /// Add a relationship row (ADR 0004 §5) with a new uid; returns it.
    /// Fails with the codes of [`Relationship::validate`].
    pub fn add_relationship(
        &mut self,
        kind: RelationshipKind,
        elements: Vec<ElementId>,
        phase: Option<ElementId>,
    ) -> Result<Uid, CommandError> {
        let r = Relationship {
            uid: self.ids.next_uid(),
            kind,
            elements,
            phase,
        };
        self.put_relationship(r)
    }

    /// Insert or replace a relationship row (same uid = replace) after
    /// validating it against the working document.
    pub fn put_relationship(&mut self, r: Relationship) -> Result<Uid, CommandError> {
        if let Err(e) = r.validate(&self.work) {
            let first = r.elements.first().copied();
            let mut err = CommandError::new(e.code, e.message);
            if let Some(id) = first {
                err = err.on(id);
            }
            return Err(err);
        }
        let uid = r.uid;
        self.work.put_relationship(r);
        Ok(uid)
    }

    /// Remove a relationship row. Fails with `ferro.model.missing_relationship`.
    pub fn remove_relationship(&mut self, uid: Uid) -> Result<Relationship, CommandError> {
        self.work.remove_relationship(uid).ok_or_else(|| {
            CommandError::new(
                "ferro.model.missing_relationship",
                format!("no relationship {uid}"),
            )
        })
    }

    /// Delete an element. Hosted elements (doors, windows) are deleted with
    /// their host, plan views with their level; the element leaves every
    /// view's hidden list (and a deleted phase every view's phase) silently;
    /// other references are cleared and reported as warnings.
    /// Deleting a type that is still used fails with `ferro.model.type_in_use`,
    /// a phase that is still used with `ferro.model.phase_in_use`.
    pub fn delete(&mut self, id: ElementId) -> Result<(), CommandError> {
        let target = self.element(id)?.clone();
        if target.is_definition() {
            if let Some(user) = self
                .work
                .elements()
                .find(|e| e.type_id == Some(id) && e.id != id)
            {
                return Err(CommandError::new(
                    "ferro.model.type_in_use",
                    format!("{} is used by {}", target.label(), user.label()),
                )
                .on(id));
            }
        }
        if target.as_phase().is_some() {
            if let Some(r) = self.work.relationships().find(|r| r.phase == Some(id)) {
                return Err(CommandError::new(
                    "ferro.model.phase_in_use",
                    format!(
                        "{} is used by the {} relationship {}",
                        target.label(),
                        r.kind.name(),
                        r.uid
                    ),
                )
                .on(id));
            }
            if let Some(user) = self.work.elements().find(|e| {
                e.id != id && (e.phase_created == Some(id) || e.phase_demolished == Some(id))
            }) {
                return Err(CommandError::new(
                    "ferro.model.phase_in_use",
                    format!(
                        "{} is used by {}; delete_phase with merge_into moves its elements to a neighbouring phase",
                        target.label(),
                        user.label()
                    ),
                )
                .on(id));
            }
        }
        // Hosted elements go with their host; plan views with their level
        // (Revit behaviour).
        let hosted: Vec<ElementId> = self
            .work
            .elements()
            .filter(|e| {
                e.host_id == Some(id)
                    || (e.level_id == Some(id)
                        && matches!(&e.body, ElementBody::View(v)
                            if matches!(v.view_type, ViewType::PlanView { .. })))
            })
            .map(|e| e.id)
            .collect();
        for h in hosted {
            if self.work.contains(h) {
                self.delete(h)?;
            }
        }
        self.work.remove(id);
        self.forget_in_views(id);
        let elevation = target.as_level().map(|l| l.elevation.mm());
        for other in self.work.element_ids() {
            let e = self.work.get(other).expect("listed");
            let top_ref = matches!(&e.body, ElementBody::Wall(w)
                if matches!(w.top, WallTop::Level { level, .. } if level == id));
            if e.level_id == Some(id) || top_ref {
                let mut e = e.clone();
                // Keep walls where they are: resolve heights before the
                // level reference goes away.
                if let (ElementBody::Wall(w), Some(top)) = (&e.body, elevation) {
                    let base =
                        generate::level_elevation(&self.work, e.level_id) + w.base_offset.mm();
                    let base = if e.level_id == Some(id) {
                        top + w.base_offset.mm()
                    } else {
                        base
                    };
                    let top_z = match &w.top {
                        WallTop::Level { level, offset } if *level == id => top + offset.mm(),
                        WallTop::Level { level, offset } => {
                            generate::level_elevation(&self.work, Some(*level)) + offset.mm()
                        }
                        WallTop::Unconnected { height } => base + height.mm(),
                    };
                    let mut w = w.clone();
                    if e.level_id == Some(id) {
                        w.base_offset = crate::math::Length::from_mm(base);
                    }
                    w.top = WallTop::Unconnected {
                        height: crate::math::Length::from_mm(top_z - base),
                    };
                    e.body = ElementBody::Wall(w);
                }
                if let (ElementBody::Floor(f), Some(z), true) =
                    (&mut e.body, elevation, e.level_id == Some(id))
                {
                    f.offset = crate::math::Length::from_mm(z + f.offset.mm());
                }
                if e.level_id == Some(id) {
                    e.level_id = None;
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

    /// Remove a deleted element from every view: hidden lists, the
    /// reserved design-option list and the view phase (a view whose phase
    /// is deleted shows the last phase). No dangling references remain.
    fn forget_in_views(&mut self, id: ElementId) {
        let users: Vec<ElementId> = self
            .work
            .elements()
            .filter(|e| {
                e.as_view().is_some_and(|v| {
                    v.phase == Some(id) || v.hides(id) || v.design_options.contains(&id)
                })
            })
            .map(|e| e.id)
            .collect();
        for u in users {
            if let Some(mut e) = self.work.get(u).cloned() {
                if let ElementBody::View(v) = &mut e.body {
                    v.hidden_elements.retain(|h| *h != id);
                    v.design_options.retain(|h| *h != id);
                    if v.phase == Some(id) {
                        v.phase = None;
                    }
                }
                self.work.put(e);
            }
        }
    }

    /// Make the working copy equal to `doc` (same ids): elements and
    /// relationship rows missing from `doc` are removed, differing ones
    /// replaced, the project info copied. Used by commands that apply a change list
    /// (`accept_design_option`, `apply_changes`); `doc` is authoritative, so
    /// no reference is cleared and nothing cascades.
    pub(crate) fn replace_with(&mut self, doc: &Document) {
        for id in self.work.element_ids() {
            if !doc.contains(id) {
                self.work.remove(id);
            }
        }
        for e in doc.elements() {
            if self.work.get(e.id) != Some(e) {
                self.work.put(e.clone());
            }
        }
        if self.work.project() != doc.project() {
            *self.work.project_mut() = doc.project().clone();
        }
        let gone: Vec<crate::model::Uid> = self
            .work
            .relationships()
            .filter(|r| doc.relationship(r.uid).is_none())
            .map(|r| r.uid)
            .collect();
        for uid in gone {
            self.work.remove_relationship(uid);
        }
        for r in doc.relationships() {
            if self.work.relationship(r.uid) != Some(r) {
                self.work.put_relationship(r.clone());
            }
        }
    }

    pub fn warn(&mut self, f: Failure) {
        self.warnings.push(f);
    }

    pub fn project_name(&mut self, name: String) {
        self.work.project_mut().name = name;
    }

    /// Change the project settings (units, location, currency). The caller
    /// validates (`ops::project_commands`).
    pub fn project_settings(
        &mut self,
        f: impl FnOnce(&mut crate::model::project_settings::ProjectSettings),
    ) {
        f(&mut self.work.project_mut().settings);
    }

    /// Add or replace (by key) a project parameter definition, keeping the
    /// list sorted by key. The caller validates it (`SetParameterDefinition`).
    pub fn put_parameter_definition(&mut self, def: crate::model::ParameterDefinition) {
        let defs = &mut self.work.project_mut().parameter_definitions;
        match defs.binary_search_by(|d| d.key.as_str().cmp(&def.key)) {
            Ok(i) => defs[i] = def,
            Err(i) => defs.insert(i, def),
        }
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

        // 2. Derived BIM geometry (walls, floors, openings).
        let pre = DocumentChanged::between(&self.name, &self.base, &self.work);
        let bim_touched = pre
            .added
            .iter()
            .chain(&pre.modified)
            .filter_map(|id| self.work.get(*id))
            .chain(pre.deleted.iter().filter_map(|id| self.base.get(*id)))
            .any(generate::affects_bim);
        if bim_touched {
            let regen = generate::regenerate(&self.work);
            for (id, body) in regen.updates {
                if let Some(e) = self.work.get(id) {
                    let mut e = e.clone();
                    generate::set_geometry(&mut e, body);
                    self.work.put(e);
                }
            }
            for issue in regen.issues {
                // A wall or opening placed in this very transaction that
                // does not fit is an error, not a warning.
                let fresh = pre.added.contains(&issue.element);
                let f = if issue.is_error || fresh {
                    Failure::error(issue.code, issue.message, vec![issue.element])
                } else {
                    Failure::warning(issue.code, issue.message, vec![issue.element])
                };
                if f.severity == super::Severity::Error {
                    errors.push(f);
                } else {
                    self.warnings.push(f);
                }
            }
        }

        // 3. Computed parameters and validation of changed elements.
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
                // D-06 rule 3: no command may leave an edge shorter than
                // SHORT_CURVE_TOL (tiny circles, slivers from imprint or
                // offset, scaling down).
                let shortest = f
                    .body
                    .edges()
                    .map(|(_, a, b)| a.distance_to(b))
                    .reduce(f64::min);
                if shortest.is_some_and(|m| m + tol::PARAM_TOL < tol::SHORT_CURVE_TOL) {
                    errors.push(Failure::error(
                        "ferro.geom.short_curve",
                        "this would create an edge shorter than 0.5 mm",
                        vec![*id],
                    ));
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
            self.bim_params(&mut e);
            if let Some(bb) = e.bounding_box() {
                let far = [bb.min, bb.max].iter().any(|p| {
                    p.x.abs() > EXTENT_LIMIT_MM
                        || p.y.abs() > EXTENT_LIMIT_MM
                        || p.z.abs() > EXTENT_LIMIT_MM
                });
                if far && e.as_form().is_none() {
                    errors.push(Failure::error(
                        "ferro.model.out_of_extent",
                        "geometry must stay within 10 km of the Internal Origin (D-05)",
                        vec![*id],
                    ));
                    continue;
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
        self.warn_stale_layers(&changed);
        let rows_changed = !self.base.relationships().eq(self.work.relationships());
        if !changed.is_empty() || rows_changed || self.base.project() != self.work.project() {
            self.work.bump_version();
        }
        Ok((self.work, changed, self.warnings))
    }

    /// Step 4 of `finish`: design-option layers (ADR 0006) that this
    /// transaction made inapplicable, e.g. by deleting a shared element an option overrides:
    /// reported as `ferro.model.option_layer_stale` warnings on the option,
    /// never dropped silently. Only layers that mention a changed element
    /// are checked, and only newly broken ones are reported.
    fn warn_stale_layers(&mut self, changed: &DocumentChanged) {
        use crate::model::design_option::{check_layer, OptionStatus};
        if !self.work.has_open_design_options() {
            return;
        }
        let touched: Vec<String> = changed
            .modified
            .iter()
            .filter_map(|id| self.work.get(*id))
            .chain(changed.deleted.iter().filter_map(|id| self.base.get(*id)))
            .filter(|e| e.category != Category::DesignOptions)
            .map(|e| e.uid.to_string())
            .collect();
        if touched.is_empty() {
            return;
        }
        let mut found = Vec::new();
        for e in self.work.by_category(Category::DesignOptions) {
            let Some(o) = e.as_design_option() else {
                continue;
            };
            if o.status != OptionStatus::Open || o.changes.is_empty() {
                continue;
            }
            let text = serde_json::to_string(&o.changes).unwrap_or_default();
            if !touched.iter().any(|u| text.contains(u.as_str())) {
                continue;
            }
            if let Err(err) = check_layer(&self.work, &o.changes) {
                if check_layer(&self.base, &o.changes).is_ok() {
                    found.push(Failure::warning(
                        "ferro.model.option_layer_stale",
                        format!(
                            "design option {} no longer applies: {}",
                            e.label(),
                            err.message
                        ),
                        vec![e.id],
                    ));
                }
            }
        }
        self.warnings.extend(found);
    }

    /// Computed dimensions of walls and floors, parametric or direct
    /// (ADR 0004 §1: quantities come from the solid for every category).
    fn bim_params(&self, e: &mut Element) {
        let snapshot = e.clone();
        let mut out: Vec<(&str, ParamValue)> = Vec::new();
        let mut set = |k: &'static str, v: ParamValue| out.push((k, v));
        let body_volume = |b: &crate::topo::Body| if b.is_closed() { b.volume() } else { 0.0 };
        let len = crate::math::Length::from_mm;
        match &snapshot.body {
            ElementBody::Wall(w) => {
                if let Ok(lay) = generate::wall_layout(&self.work, &snapshot, w) {
                    set(keys::LENGTH, ParamValue::Length(len(lay.length())));
                    set(keys::HEIGHT, ParamValue::Length(len(lay.height())));
                    set(keys::THICKNESS, ParamValue::Length(len(lay.thickness())));
                }
                set(keys::VOLUME, ParamValue::Volume(body_volume(&w.geometry)));
            }
            ElementBody::Floor(f) => {
                let t = generate::floor_structure(&self.work, &snapshot).thickness();
                set(keys::THICKNESS, ParamValue::Length(len(t)));
                let v = body_volume(&f.geometry);
                set(keys::VOLUME, ParamValue::Volume(v));
                if t > 0.0 {
                    set(keys::AREA, ParamValue::Area(v / t));
                }
            }
            ElementBody::Form(f) if snapshot.category != Category::Forms => {
                // Direct geometry classified as a building element.
                let b = &f.body;
                if b.is_closed() {
                    set(keys::VOLUME, ParamValue::Volume(b.volume()));
                }
                if let Some(bb) = b.bounding_box() {
                    set(keys::HEIGHT, ParamValue::Length(len(bb.max.z - bb.min.z)));
                }
                let typed = match snapshot
                    .type_id
                    .and_then(|t| self.work.get(t))
                    .map(|t| &t.body)
                {
                    Some(ElementBody::WallType(t)) => Some(t.structure.thickness()),
                    Some(ElementBody::FloorType(t)) => Some(t.structure.thickness()),
                    _ => None,
                };
                if let Some(t) = typed {
                    set(keys::THICKNESS, ParamValue::Length(len(t)));
                }
                if snapshot.category == Category::Floors {
                    // Plan area: the faces looking up.
                    let up: f64 = b
                        .face_ids()
                        .filter(|fid| {
                            b.face_normal(*fid)
                                .is_ok_and(|n| n.as_vec().z > 1.0 - crate::math::tol::ANGLE_TOL)
                        })
                        .filter_map(|fid| b.face_area(fid).ok())
                        .sum();
                    set(keys::AREA, ParamValue::Area(up));
                }
            }
            _ => {}
        }
        // Computed keys this element no longer has (e.g. a frozen wall's
        // length) are removed so nothing stale is reported.
        if snapshot.geometry_source() == Some(crate::model::GeometrySource::Direct) {
            for k in [
                keys::LENGTH,
                keys::HEIGHT,
                keys::THICKNESS,
                keys::AREA,
                keys::VOLUME,
            ] {
                if !out.iter().any(|(o, _)| *o == k) {
                    e.params.remove(k);
                }
            }
        }
        for (k, v) in out {
            e.params.insert(k.into(), v);
        }
    }
}
