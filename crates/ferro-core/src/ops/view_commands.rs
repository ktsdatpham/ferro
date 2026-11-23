//! View commands (D-33, M5-22): create, change and delete views. Each
//! `NAME` is also the MCP tool name.
//!
//! Hiding elements in a view is a field of [`SetView`] (`hide`, `unhide`,
//! `unhide_all`) instead of separate verbs (spec §5.5 standard verbs).
//! Deleting an element removes it from every view in the same transaction
//! (`Transaction::delete`).

use super::commands::{Command, CommandOutput};
use super::{CommandError, Transaction};
use crate::math::{BoundingBox3, Length};
use crate::model::view::{validate_section_box, validate_section_plane, DEFAULT_SECTION_DEPTH_MM};
use crate::model::{
    Category, DisplayStyle, ElementBody, ElementId, PhaseFilter, SectionPlane, View, ViewCamera,
    ViewRange, ViewType,
};
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

fn invalid(msg: impl Into<String>) -> CommandError {
    CommandError::new("ferro.model.invalid_view", msg)
}

fn not_a_view(id: ElementId) -> CommandError {
    CommandError::new("ferro.model.not_a_view", format!("{id} is not a view")).on(id)
}

fn check_name_free(
    tx: &Transaction,
    name: &str,
    except: Option<ElementId>,
) -> Result<(), CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::new(
            "ferro.model.invalid_name",
            "a view needs a name",
        ));
    }
    if tx
        .doc()
        .by_category(Category::Views)
        .any(|v| Some(v.id) != except && v.name.as_deref() == Some(name))
    {
        return Err(CommandError::new(
            "ferro.model.duplicate_name",
            format!("a view named {name:?} already exists"),
        ));
    }
    Ok(())
}

fn check_phase(tx: &Transaction, phase: Option<ElementId>) -> Result<(), CommandError> {
    if let Some(p) = phase {
        if tx.element(p)?.as_phase().is_none() {
            return Err(CommandError::new(
                "ferro.model.not_a_phase",
                format!("{p} is not a phase"),
            )
            .on(p));
        }
    }
    Ok(())
}

fn check_camera(c: &ViewCamera) -> Result<(), CommandError> {
    c.validate().map_err(invalid)
}

fn check_depth(d: Length) -> Result<(), CommandError> {
    let mm = d.mm();
    if !mm.is_finite() || mm <= crate::math::tol::SHORT_CURVE_TOL {
        return Err(invalid(format!(
            "the section depth must be more than {} mm",
            crate::math::tol::SHORT_CURVE_TOL
        )));
    }
    Ok(())
}

/// Only elements drawn in the model (with geometry) and levels can be hidden.
fn check_hideable(tx: &Transaction, id: ElementId) -> Result<(), CommandError> {
    let e = tx.element(id)?;
    if e.geometry().is_none() && e.as_level().is_none() {
        return Err(CommandError::new(
            "ferro.model.not_hideable",
            format!("{} is not drawn in views", e.label()),
        )
        .on(id));
    }
    Ok(())
}

/// Sorted, unique, trimmed tag names; blank names are refused.
fn clean_tags(tags: &[String]) -> Result<Vec<String>, CommandError> {
    let mut out = Vec::with_capacity(tags.len());
    for t in tags {
        let t = t.trim();
        if t.is_empty() {
            return Err(invalid("a hidden tag name is blank"));
        }
        out.push(t.to_string());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Kind of view to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewTypeName {
    #[serde(rename = "view_3d")]
    View3D,
    PlanView,
    SectionView,
}

/// Create a view named `name` (unique among views).
///
/// * `view_3d`: optional `camera` (default iso), `section_plane`,
///   `section_box`.
/// * `plan_view`: `level` required; optional `view_range` (default cut
///   1200 mm) and `camera` (default: top-down over the origin).
/// * `section_view`: `section_plane` required; optional `depth` (mm,
///   default 10 m) and `camera` (default: facing the plane).
///
/// Optional `display_style`, `phase`, `phase_filter` for every kind.
///
/// Fails with `ferro.model.invalid_name`, `ferro.model.duplicate_name`,
/// `ferro.model.invalid_view` (missing or degenerate camera / cut / range,
/// a field the view type does not have), `ferro.model.not_a_level`,
/// `ferro.model.not_a_phase`, `ferro.model.not_found`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateView {
    pub name: String,
    pub view_type: ViewTypeName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewCamera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_plane: Option<SectionPlane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_box: Option<BoundingBox3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_range: Option<ViewRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_style: Option<DisplayStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ElementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_filter: Option<PhaseFilter>,
}

impl CreateView {
    /// A command creating a 3D view with `camera` and nothing else set.
    pub fn view_3d(name: impl Into<String>, camera: ViewCamera) -> CreateView {
        CreateView {
            name: name.into(),
            view_type: ViewTypeName::View3D,
            level: None,
            camera: Some(camera),
            section_plane: None,
            section_box: None,
            view_range: None,
            depth: None,
            display_style: None,
            phase: None,
            phase_filter: None,
        }
    }
}

impl Command for CreateView {
    const NAME: &'static str = "create_view";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        check_name_free(tx, &self.name, None)?;
        check_phase(tx, self.phase)?;
        if let Some(c) = &self.camera {
            check_camera(c)?;
        }
        let only = |ok: bool, what: &str| -> Result<(), CommandError> {
            if ok {
                Ok(())
            } else {
                Err(invalid(format!(
                    "a {} has no {what}",
                    match self.view_type {
                        ViewTypeName::View3D => "3D view",
                        ViewTypeName::PlanView => "plan view",
                        ViewTypeName::SectionView => "section view",
                    }
                )))
            }
        };
        let is = |t: ViewTypeName| self.view_type == t;
        only(self.level.is_none() || is(ViewTypeName::PlanView), "level")?;
        only(
            self.view_range.is_none() || is(ViewTypeName::PlanView),
            "view range",
        )?;
        only(
            self.section_box.is_none() || is(ViewTypeName::View3D),
            "section box",
        )?;
        only(
            self.depth.is_none() || is(ViewTypeName::SectionView),
            "depth",
        )?;
        only(
            self.section_plane.is_none() || !is(ViewTypeName::PlanView),
            "section plane (its cut comes from the view range)",
        )?;
        if let Some(p) = &self.section_plane {
            validate_section_plane(p).map_err(invalid)?;
        }
        if let Some(b) = &self.section_box {
            validate_section_box(b).map_err(invalid)?;
        }
        let (view_type, camera, level) = match self.view_type {
            ViewTypeName::View3D => (
                ViewType::View3D {
                    section_plane: self.section_plane,
                    section_box: self.section_box,
                },
                self.camera.unwrap_or_else(ViewCamera::default_3d),
                None,
            ),
            ViewTypeName::PlanView => {
                let Some(level) = self.level else {
                    return Err(invalid("a plan view needs a level"));
                };
                let lvl = tx.element(level)?;
                let Some(l) = lvl.as_level() else {
                    return Err(CommandError::new(
                        "ferro.model.not_a_level",
                        format!("{level} is not a level"),
                    )
                    .on(level));
                };
                let z = l.elevation.mm();
                let range = self.view_range.unwrap_or_default();
                range.validate().map_err(invalid)?;
                (
                    ViewType::PlanView { view_range: range },
                    self.camera.unwrap_or_else(|| ViewCamera::plan(0.0, 0.0, z)),
                    Some(level),
                )
            }
            ViewTypeName::SectionView => {
                let Some(plane) = self.section_plane else {
                    return Err(invalid("a section view needs a section plane"));
                };
                let depth = self
                    .depth
                    .unwrap_or(Length::from_mm(DEFAULT_SECTION_DEPTH_MM));
                check_depth(depth)?;
                (
                    ViewType::SectionView {
                        section_plane: plane,
                        depth,
                    },
                    self.camera.unwrap_or_else(|| ViewCamera::section(&plane)),
                    None,
                )
            }
        };
        let mut view = View::new(view_type, camera);
        if let Some(s) = self.display_style {
            view.display_style = s;
        }
        view.phase = self.phase;
        if let Some(f) = self.phase_filter {
            view.phase_filter = f;
        }
        let id = tx.create(
            Category::Views,
            Some(self.name.clone()),
            level,
            ElementBody::View(view),
        )?;
        Ok(json!({ "element": id }))
    }
}

/// Change a view. Every field is optional; a missing field is left as it
/// is. `section_plane`, `section_box` and `phase` accept `null` to clear.
///
/// * `camera`, `display_style`, `phase`, `phase_filter`, `name`: any view.
/// * `section_plane`: 3D views (`null` removes the cut) and section views
///   (required, cannot be cleared); `section_box`: 3D views; `view_range`:
///   plan views; `depth`: section views.
/// * `hide` adds elements to the view's hidden list, `unhide` removes them,
///   `unhide_all` empties it first; `hidden_tags` replaces the hidden tags.
///
/// Fails with `ferro.model.not_a_view`, `ferro.model.invalid_name`,
/// `ferro.model.duplicate_name`, `ferro.model.invalid_view`,
/// `ferro.model.not_a_phase`, `ferro.model.not_hideable`,
/// `ferro.model.not_found`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetView {
    pub view: ElementId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewCamera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_style: Option<DisplayStyle>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub section_plane: Option<Option<SectionPlane>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub section_box: Option<Option<BoundingBox3>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_range: Option<ViewRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Length>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unhide: Vec<ElementId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unhide_all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_tags: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub phase: Option<Option<ElementId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_filter: Option<PhaseFilter>,
}

impl SetView {
    /// A command changing nothing on `view` (set fields afterwards).
    pub fn new(view: ElementId) -> SetView {
        SetView {
            view,
            name: None,
            camera: None,
            display_style: None,
            section_plane: None,
            section_box: None,
            view_range: None,
            depth: None,
            hide: Vec::new(),
            unhide: Vec::new(),
            unhide_all: false,
            hidden_tags: None,
            phase: None,
            phase_filter: None,
        }
    }
}

impl Command for SetView {
    const NAME: &'static str = "set_view";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        let Some(mut view) = tx.element(self.view)?.as_view().cloned() else {
            return Err(not_a_view(self.view));
        };
        if let Some(n) = &self.name {
            check_name_free(tx, n, Some(self.view))?;
        }
        if let Some(c) = &self.camera {
            check_camera(c)?;
            view.camera = *c;
        }
        if let Some(s) = self.display_style {
            view.display_style = s;
        }
        if let Some(p) = self.phase {
            check_phase(tx, p)?;
            view.phase = p;
        }
        if let Some(f) = self.phase_filter {
            view.phase_filter = f;
        }
        let kind = view.view_type.as_str();
        let no = |what: &str| invalid(format!("a {kind} has no {what}"));
        if let Some(plane) = self.section_plane {
            if let Some(p) = &plane {
                validate_section_plane(p).map_err(invalid)?;
            }
            match (&mut view.view_type, plane) {
                (ViewType::View3D { section_plane, .. }, p) => *section_plane = p,
                (ViewType::SectionView { section_plane, .. }, Some(p)) => *section_plane = p,
                (ViewType::SectionView { .. }, None) => {
                    return Err(invalid("a section view's plane cannot be removed"))
                }
                (ViewType::PlanView { .. }, _) => return Err(no("section plane")),
            }
        }
        if let Some(b) = self.section_box {
            if let Some(b) = &b {
                validate_section_box(b).map_err(invalid)?;
            }
            match &mut view.view_type {
                ViewType::View3D { section_box, .. } => *section_box = b,
                _ => return Err(no("section box")),
            }
        }
        if let Some(r) = self.view_range {
            r.validate().map_err(invalid)?;
            match &mut view.view_type {
                ViewType::PlanView { view_range } => *view_range = r,
                _ => return Err(no("view range")),
            }
        }
        if let Some(d) = self.depth {
            check_depth(d)?;
            match &mut view.view_type {
                ViewType::SectionView { depth, .. } => *depth = d,
                _ => return Err(no("depth")),
            }
        }
        if self.unhide_all {
            view.hidden_elements.clear();
        }
        for id in &self.hide {
            check_hideable(tx, *id)?;
            if *id == self.view {
                return Err(invalid("a view cannot hide itself"));
            }
            view.hidden_elements.push(*id);
        }
        view.hidden_elements.retain(|h| !self.unhide.contains(h));
        view.hidden_elements.sort();
        view.hidden_elements.dedup();
        if let Some(tags) = &self.hidden_tags {
            view.hidden_tags = clean_tags(tags)?;
        }
        let hidden = view.hidden_elements.clone();
        let name = self.name.clone();
        tx.modify(self.view, |e| {
            if let Some(n) = name {
                e.name = Some(n);
            }
            e.body = ElementBody::View(view);
            Ok(())
        })?;
        Ok(json!({ "element": self.view, "hidden_elements": hidden }))
    }
}

/// Delete a view. Same as `delete_elements` on it, but refuses other
/// elements.
///
/// Fails with `ferro.model.not_a_view`, `ferro.model.not_found`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteView {
    pub view: ElementId,
}

impl Command for DeleteView {
    const NAME: &'static str = "delete_view";
    fn execute(&self, tx: &mut Transaction) -> Result<CommandOutput, CommandError> {
        if tx.element(self.view)?.as_view().is_none() {
            return Err(not_a_view(self.view));
        }
        tx.delete(self.view)?;
        Ok(json!({ "deleted": self.view }))
    }
}
