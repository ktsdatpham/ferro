//! Design options (ADR 0006 §Design options, M5-23.9): option sets, options
//! as layers of changes over the shared model, and combinations.
//!
//! * An [`OptionSet`] and its [`DesignOption`]s are elements of category
//!   `DesignOptions` (Revit: `OST_DesignOptions`). An option's set is its
//!   `host_id` (the set contains its options; deleting a set deletes them).
//! * An option holds a **layer**: a list of [`Change`]s (the M5-26 change
//!   format) over the shared model. It can add elements, remove shared
//!   elements (in that option only) and override properties of shared
//!   elements, without copying them.
//! * A [`Combination`] picks one open option per set (the primary one by
//!   default). [`apply_layers`] gives the elements as seen in a combination;
//!   `ops::design_option_commands::resolve` adds regeneration on top.
//!
//! Layers never touch the project or design-option elements, and never set
//! the `design_option` field: in the shared model that field is always
//! `None`; in a document resolved for a combination it names the option
//! whose layer added the element (a derived index, never stored in files).

use super::change::{self, apply, conflicts, Change, ChangeError};
use super::{Category, Document, Element, ElementBody, ElementId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Name given to the first option of a new option set.
pub const FIRST_OPTION_NAME: &str = "Option 1";

/// A set of mutually exclusive alternatives (Revit: `Option Set`). Sets are
/// ordered by `order` (then id); layers of a combination apply in that
/// order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionSet {
    pub order: u32,
}

/// Lifecycle of a design option.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionStatus {
    /// Can be shown in a combination and edited.
    #[default]
    Open,
    /// Its layer was merged into the shared model (kept for the record).
    Accepted,
    /// Another option of its set was accepted; kept, never deleted silently.
    Archived,
}

impl OptionStatus {
    /// Stable snake_case name.
    pub fn as_str(self) -> &'static str {
        match self {
            OptionStatus::Open => "open",
            OptionStatus::Accepted => "accepted",
            OptionStatus::Archived => "archived",
        }
    }
}

/// One alternative of an option set (Revit: `DesignOption`): a layer of
/// changes over the shared model. Exactly one open option per set is
/// primary; only open options are primary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignOption {
    pub order: u32,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub status: OptionStatus,
    /// The layer, in the change format (paths `/elements/<uid>...`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
}

/// A design-option rule broken, with a stable code (`ferro.model.*`) and
/// the option concerned, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignOptionError {
    pub code: &'static str,
    pub message: String,
    pub option: Option<ElementId>,
}

impl DesignOptionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        DesignOptionError {
            code,
            message: message.into(),
            option: None,
        }
    }
    fn on(mut self, option: ElementId) -> Self {
        self.option = Some(option);
        self
    }
}

impl std::fmt::Display for DesignOptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// One option per set. Sets not named use their primary option; sets
/// without open options (all accepted / archived) are left out. The empty
/// combination is the primary combination.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combination {
    /// Chosen options (at most one per set).
    #[serde(default)]
    pub options: Vec<ElementId>,
}

impl Combination {
    /// The primary combination (every set shows its primary option).
    pub fn primary() -> Combination {
        Combination::default()
    }

    /// This combination with `option` chosen for its set.
    pub fn with(&self, doc: &Document, option: ElementId) -> Combination {
        let set = doc.get(option).and_then(|o| o.host_id);
        let mut options: Vec<ElementId> = self
            .options
            .iter()
            .copied()
            .filter(|o| doc.get(*o).and_then(|e| e.host_id) != set || set.is_none())
            .collect();
        options.push(option);
        Combination { options }
    }

    /// The option shown for every set with open options, in set order.
    ///
    /// Fails with `ferro.model.not_a_design_option` (an id that is not an
    /// option), `ferro.model.option_not_open` (an accepted / archived
    /// option) and `ferro.model.option_set_twice` (two options of one set).
    pub fn chosen(&self, doc: &Document) -> Result<Vec<ElementId>, DesignOptionError> {
        let mut by_set: BTreeMap<ElementId, ElementId> = BTreeMap::new();
        for o in &self.options {
            let e = doc.get(*o).filter(|e| e.as_design_option().is_some());
            let (Some(e), Some(opt)) = (e, e.and_then(|e| e.as_design_option())) else {
                return Err(DesignOptionError::new(
                    "ferro.model.not_a_design_option",
                    format!("{o} is not a design option"),
                ));
            };
            if opt.status != OptionStatus::Open {
                return Err(DesignOptionError::new(
                    "ferro.model.option_not_open",
                    format!("{} is {}", e.label(), opt.status.as_str()),
                )
                .on(*o));
            }
            let set = e.host_id.unwrap_or(*o);
            if by_set.insert(set, *o).is_some_and(|prev| prev != *o) {
                return Err(DesignOptionError::new(
                    "ferro.model.option_set_twice",
                    "a combination shows one option per option set",
                )
                .on(*o));
            }
        }
        let mut out = Vec::new();
        for set in doc.option_sets() {
            if let Some(o) = by_set.get(&set.id) {
                out.push(*o);
            } else if let Some(p) = doc.primary_option(set.id) {
                out.push(p);
            }
        }
        Ok(out)
    }
}

impl Element {
    /// The option-set data, if this element is an option set.
    pub fn as_option_set(&self) -> Option<&OptionSet> {
        match &self.body {
            ElementBody::OptionSet(s) => Some(s),
            _ => None,
        }
    }
    /// The design-option data, if this element is a design option.
    pub fn as_design_option(&self) -> Option<&DesignOption> {
        match &self.body {
            ElementBody::DesignOption(o) => Some(o),
            _ => None,
        }
    }
}

impl Document {
    /// Option sets in order (by `order`, then id).
    pub fn option_sets(&self) -> Vec<&Element> {
        let mut v: Vec<&Element> = self
            .by_category(Category::DesignOptions)
            .filter(|e| e.as_option_set().is_some())
            .collect();
        v.sort_by_key(|e| (e.as_option_set().map(|s| s.order).unwrap_or(u32::MAX), e.id));
        v
    }

    /// Options of a set in order (open, accepted and archived).
    pub fn design_options(&self, set: ElementId) -> Vec<&Element> {
        let mut v: Vec<&Element> = self
            .by_category(Category::DesignOptions)
            .filter(|e| e.host_id == Some(set) && e.as_design_option().is_some())
            .collect();
        v.sort_by_key(|e| {
            (
                e.as_design_option().map(|o| o.order).unwrap_or(u32::MAX),
                e.id,
            )
        });
        v
    }

    /// The primary option of a set; `None` when the set has no open option.
    pub fn primary_option(&self, set: ElementId) -> Option<ElementId> {
        self.design_options(set)
            .into_iter()
            .find(|e| {
                e.as_design_option()
                    .is_some_and(|o| o.is_primary && o.status == OptionStatus::Open)
            })
            .map(|e| e.id)
    }

    /// `true` if some option is open (combinations then matter).
    pub fn has_open_design_options(&self) -> bool {
        self.by_category(Category::DesignOptions).any(|e| {
            e.as_design_option()
                .is_some_and(|o| o.status == OptionStatus::Open)
        })
    }

    /// Open options that are not the primary of their set.
    pub fn secondary_options(&self) -> Vec<ElementId> {
        self.by_category(Category::DesignOptions)
            .filter(|e| {
                e.as_design_option()
                    .is_some_and(|o| o.status == OptionStatus::Open && !o.is_primary)
            })
            .map(|e| e.id)
            .collect()
    }
}

fn layer_error(option: Option<ElementId>, e: ChangeError) -> DesignOptionError {
    let err = DesignOptionError::new(
        "ferro.model.invalid_option_layer",
        format!("{} ({})", e.message, e.code),
    );
    match option {
        Some(o) => err.on(o),
        None => err,
    }
}

/// The element uid a layer path addresses (`/elements/<uid>/...`) and the
/// rest of the path, or `None` for other paths.
fn layer_target(path: &str) -> Option<(&str, Vec<&str>)> {
    let rest = path.strip_prefix("/elements/")?;
    let mut parts = rest.split('/');
    let uid = parts.next().filter(|u| !u.is_empty())?;
    Some((uid, parts.collect()))
}

/// Check a layer against the shared model `doc`: every path addresses an
/// element (`/elements/<uid>...`), no design-option element and no
/// `design_option` field is touched, and the changes apply.
///
/// Fails with `ferro.model.invalid_option_layer` (the message carries the
/// `ferro.change.*` code when applying fails).
pub fn check_layer(doc: &Document, changes: &[Change]) -> Result<(), DesignOptionError> {
    let uids: HashMap<String, &Element> = doc.elements().map(|e| (e.uid.to_string(), e)).collect();
    for c in changes {
        let bad = |m: String| DesignOptionError::new("ferro.model.invalid_option_layer", m);
        if c.path()
            .strip_prefix("/relationships/")
            .is_some_and(|r| !r.is_empty())
        {
            continue; // relationship rows (joins, cuts...) may differ per option
        }
        let Some((uid, rest)) = layer_target(c.path()) else {
            return Err(bad(format!(
                "`{}`: an option changes elements and relationships only (/elements/<uid>..., /relationships/<uid>...)",
                c.path()
            )));
        };
        if uids
            .get(uid)
            .is_some_and(|e| e.category == Category::DesignOptions)
        {
            return Err(bad(format!(
                "`{}`: an option cannot change option sets or options",
                c.path()
            )));
        }
        if rest.first() == Some(&"design_option") {
            return Err(bad(format!(
                "`{}`: the design_option field is derived, never set",
                c.path()
            )));
        }
        if let (Change::Set { value, .. }, true) = (c, rest.is_empty()) {
            if value.get("category").and_then(Value::as_str) == Some("design_options") {
                return Err(bad(format!(
                    "`{}`: an option cannot add option sets or options",
                    c.path()
                )));
            }
            if value.get("design_option").is_some_and(|v| !v.is_null()) {
                return Err(bad(format!(
                    "`{}`: the design_option field is derived, never set",
                    c.path()
                )));
            }
        }
    }
    apply(doc, changes)
        .map(|_| ())
        .map_err(|e| layer_error(None, e))
}

/// Uids of the elements a layer adds (whole-element sets of uids that are
/// not in `doc`).
pub fn added_uids(doc: &Document, changes: &[Change]) -> Vec<String> {
    let existing: std::collections::HashSet<String> =
        doc.elements().map(|e| e.uid.to_string()).collect();
    changes
        .iter()
        .filter_map(|c| match c {
            Change::Set { path, .. } => layer_target(path)
                .filter(|(u, rest)| rest.is_empty() && !existing.contains(*u))
                .map(|(u, _)| u.to_string()),
            Change::Remove { .. } => None,
        })
        .collect()
}

/// What a layer does to the shared model: elements added, shared elements
/// removed, shared elements with overridden properties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerCounts {
    pub added: usize,
    pub removed: usize,
    pub overridden: usize,
}

/// Count what a layer adds, removes and overrides in `doc`.
pub fn layer_counts(doc: &Document, changes: &[Change]) -> LayerCounts {
    let existing: std::collections::HashSet<String> =
        doc.elements().map(|e| e.uid.to_string()).collect();
    let mut out = LayerCounts::default();
    let mut overridden = std::collections::BTreeSet::new();
    for c in changes {
        let Some((uid, rest)) = layer_target(c.path()) else {
            continue;
        };
        match (c, rest.is_empty(), existing.contains(uid)) {
            (Change::Set { .. }, true, false) => out.added += 1,
            (Change::Remove { .. }, true, _) => out.removed += 1,
            (_, false, true) => {
                overridden.insert(uid.to_string());
            }
            _ => {}
        }
    }
    out.overridden = overridden.len();
    out
}

/// Elements of `doc` as seen with the layers of `options` (each an open
/// option; at most one per set) applied in set order, without
/// regeneration. In the result, every design option carries no changes
/// (so resolving it again changes nothing) and elements added by a layer
/// have `design_option` set to that option.
///
/// Fails with `ferro.model.not_a_design_option`,
/// `ferro.model.option_conflict` (two layers touch the same property, or
/// one removes what another changes; the message lists the paths) and
/// `ferro.model.invalid_option_layer` (a layer no longer applies, e.g. it
/// overrides an element deleted from the shared model since).
pub fn apply_layers(doc: &Document, options: &[ElementId]) -> Result<Document, DesignOptionError> {
    let set_pos: HashMap<ElementId, usize> = doc
        .option_sets()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let mut chosen: Vec<(usize, ElementId, &Element, &DesignOption)> = Vec::new();
    for o in options {
        let e = doc.get(*o);
        let Some((e, opt)) = e.and_then(|e| e.as_design_option().map(|d| (e, d))) else {
            return Err(DesignOptionError::new(
                "ferro.model.not_a_design_option",
                format!("{o} is not a design option"),
            ));
        };
        let pos = e
            .host_id
            .and_then(|s| set_pos.get(&s).copied())
            .unwrap_or(usize::MAX);
        chosen.push((pos, *o, e, opt));
    }
    chosen.sort_by_key(|(pos, id, _, _)| (*pos, *id));
    for (i, (_, a, ea, la)) in chosen.iter().enumerate() {
        for (_, _, eb, lb) in &chosen[i + 1..] {
            let paths = conflicts(&la.changes, &lb.changes);
            if !paths.is_empty() {
                return Err(DesignOptionError::new(
                    "ferro.model.option_conflict",
                    format!(
                        "{} and {} change the same properties: {}",
                        ea.label(),
                        eb.label(),
                        paths.join(", ")
                    ),
                )
                .on(*a));
            }
        }
    }
    for (_, id, _, opt) in &chosen {
        check_layer(doc, &opt.changes).map_err(|mut e| {
            e.option = Some(*id);
            e
        })?;
    }
    let all: Vec<Change> = chosen
        .iter()
        .flat_map(|(_, _, _, o)| o.changes.iter().cloned())
        .collect();
    let mut out = apply(doc, &all).map_err(|e| layer_error(chosen.first().map(|c| c.1), e))?;
    // Derived index: which option added which element.
    let mut added_by: HashMap<String, ElementId> = HashMap::new();
    for (_, id, _, opt) in &chosen {
        for u in added_uids(doc, &opt.changes) {
            added_by.insert(u, *id);
        }
    }
    let ids: Vec<ElementId> = out.element_ids();
    for id in ids {
        let Some(e) = out.get(id) else { continue };
        let stamp = added_by.get(&e.uid.to_string()).copied();
        let clear = e.as_design_option().is_some_and(|o| !o.changes.is_empty());
        if stamp.is_none() && !clear {
            continue;
        }
        let mut e = e.clone();
        if stamp.is_some() {
            e.design_option = stamp;
        }
        if let ElementBody::DesignOption(o) = &mut e.body {
            o.changes.clear();
        }
        out.put(e);
    }
    Ok(out)
}

/// `true` for paths of derived data that a layer must not store: the
/// regenerated geometry of walls, floors and openings, and computed
/// parameters. Resolving regenerates them.
pub fn is_derived_path(path: &str) -> bool {
    let Some((_, rest)) = layer_target(path) else {
        return false;
    };
    match rest.as_slice() {
        ["body", "geometry", ..] => true,
        ["params", key, ..] => super::keys::is_read_only(&change::unescape_segment(key)),
        _ => false,
    }
}

/// The layer that turns `base` into `edited` (both seen in the same
/// combination of the other sets): `diff` without derived data, with the
/// `design_option` index cleared on added elements.
///
/// Fails with `ferro.change.*` when an element of either document refers to
/// a missing element.
pub fn layer_from_edit(base: &Document, edited: &Document) -> Result<Vec<Change>, ChangeError> {
    let mut out = change::diff(base, edited)?;
    out.retain(|c| !is_derived_path(c.path()) && !c.path().ends_with("/design_option"));
    for c in &mut out {
        if let Change::Set { path, value } = c {
            if layer_target(path).is_some_and(|(_, rest)| rest.is_empty()) {
                if let Some(o) = value.as_object_mut() {
                    o.insert("design_option".into(), Value::Null);
                }
            }
        }
    }
    Ok(out)
}

/// Open options whose layer no longer applies to the shared model, each
/// with the reason (`ferro.model.invalid_option_layer`).
pub fn stale_layers(doc: &Document) -> Vec<DesignOptionError> {
    doc.by_category(Category::DesignOptions)
        .filter_map(|e| {
            let o = e.as_design_option()?;
            if o.status != OptionStatus::Open || o.changes.is_empty() {
                return None;
            }
            check_layer(doc, &o.changes).err().map(|mut err| {
                err.option = Some(e.id);
                err.message = format!("{}: {}", e.label(), err.message);
                err
            })
        })
        .collect()
}

/// Structural rules of option sets: every option is hosted by an option
/// set, only open options are primary, and every set with open options has
/// exactly one primary.
///
/// Fails with `ferro.model.invalid_design_options`.
pub fn check_design_options(doc: &Document) -> Result<(), DesignOptionError> {
    let bad = |m: String| DesignOptionError::new("ferro.model.invalid_design_options", m);
    let mut primaries: BTreeMap<ElementId, (usize, usize)> = BTreeMap::new();
    for e in doc.by_category(Category::DesignOptions) {
        match &e.body {
            ElementBody::OptionSet(_) => {
                primaries.entry(e.id).or_default();
            }
            ElementBody::DesignOption(o) => {
                let set = e
                    .host_id
                    .filter(|s| doc.get(*s).and_then(|x| x.as_option_set()).is_some())
                    .ok_or_else(|| bad(format!("{} is not in an option set", e.label())))?;
                if o.is_primary && o.status != OptionStatus::Open {
                    return Err(bad(format!(
                        "{} is {} and cannot be primary",
                        e.label(),
                        o.status.as_str()
                    )));
                }
                let n = primaries.entry(set).or_default();
                if o.status == OptionStatus::Open {
                    n.0 += 1;
                    if o.is_primary {
                        n.1 += 1;
                    }
                }
            }
            _ => return Err(bad(format!("{} is not an option set or option", e.label()))),
        }
    }
    for (set, (open, primary)) in primaries {
        if open > 0 && primary != 1 {
            let label = doc.get(set).map(|s| s.label()).unwrap_or_default();
            return Err(bad(format!(
                "option set {label} has {primary} primary options (needs exactly one)"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_targets_and_derived_paths() {
        assert_eq!(
            layer_target("/elements/abc/params/x"),
            Some(("abc", vec!["params", "x"]))
        );
        assert_eq!(layer_target("/elements/abc"), Some(("abc", vec![])));
        assert_eq!(layer_target("/project/name"), None);
        assert_eq!(layer_target("/elements/"), None);
        assert!(is_derived_path("/elements/a/body/geometry/faces"));
        assert!(is_derived_path(
            "/elements/a/params/ferro.dimensions.length"
        ));
        assert!(!is_derived_path("/elements/a/params/acme.fire.rating"));
        assert!(!is_derived_path("/elements/a/body/top/height"));
        assert!(!is_derived_path("/project/name"));
    }

    #[test]
    fn layer_counts_classify_edits() {
        let doc = Document::new("T");
        let changes = vec![
            Change::Set {
                path: "/elements/new-1".into(),
                value: serde_json::json!({}),
            },
            Change::Remove {
                path: "/elements/gone".into(),
            },
        ];
        let c = layer_counts(&doc, &changes);
        assert_eq!((c.added, c.removed, c.overridden), (1, 1, 0));
        assert_eq!(added_uids(&doc, &changes), vec!["new-1".to_string()]);
    }

    #[test]
    fn empty_combination_is_primary_and_empty_document_has_no_sets() {
        let doc = Document::new("T");
        assert_eq!(Combination::primary().chosen(&doc).unwrap(), vec![]);
        assert!(!doc.has_open_design_options());
        assert!(check_design_options(&doc).is_ok());
        assert!(stale_layers(&doc).is_empty());
        let same = apply_layers(&doc, &[]).unwrap();
        assert_eq!(same.len(), 0);
    }

    #[test]
    fn layers_cannot_touch_the_project_or_set_the_index() {
        let doc = Document::new("T");
        let project = vec![Change::Set {
            path: "/project/name".into(),
            value: serde_json::json!("x"),
        }];
        assert_eq!(
            check_layer(&doc, &project).unwrap_err().code,
            "ferro.model.invalid_option_layer"
        );
        let index = vec![Change::Set {
            path: "/elements/u/design_option".into(),
            value: serde_json::json!(null),
        }];
        assert!(check_layer(&doc, &index)
            .unwrap_err()
            .message
            .contains("derived"));
    }
}
