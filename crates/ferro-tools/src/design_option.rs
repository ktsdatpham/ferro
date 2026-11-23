//! Design-option tools (ADR 0006, M5-23.9) and the change-format tools
//! (M5-26.2): tool definitions, `list_design_options`, `diff_documents` and
//! the combination argument shared by read tools.

use ferro_core::model::change::diff;
use ferro_core::model::design_option::{layer_counts, stale_layers, Combination};
use ferro_core::ops::design_option_commands::{resolve, resolve_options};
use ferro_core::{Document, ElementId};
use serde_json::{json, Value};

use crate::{failures_json, tool};

fn id(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}

/// JSON schema of a combination argument.
pub fn combination_schema(desc: &str) -> Value {
    json!({"type": ["array", "string"], "items": {"type": "integer"}, "description": format!("{desc}: \"shared\" (the shared model, no option layers), [] (primary combination), or option ids (one per option set; other sets show their primary)")})
}

const CHANGES: &str = "Change list: [{\"op\": \"set\", \"path\": \"/elements/<uid>/name\", \"value\": ...} | {\"op\": \"remove\", \"path\": \"/elements/<uid>\"}]; paths are JSON pointers into {project, elements: {uid: element}, relationships: {uid: row}}, references between elements are uids.";

/// Tool definitions for design options and change lists.
pub fn tools() -> Vec<Value> {
    let changes = json!({"type": "array", "items": {"type": "object"}, "description": CHANGES});
    vec![
        tool("create_option_set", "Create a design option set (Revit Option Set) with its first option \"Option 1\" (primary, empty). Options are alternatives shown one per set; each is a layer of changes over the shared model. Returns {element: set, option}.",
            json!({"name": {"type": "string"}}), &["name"]),
        tool("set_option_set", "Rename an option set or move it to `position` (0 = first). Layers of a combination apply in set order.",
            json!({"set": id("Option set id"), "name": {"type": "string"}, "position": {"type": "integer", "minimum": 0}}), &["set"]),
        tool("delete_option_set", "Delete an option set with all its options (their layers stop being shown; the shared model is unchanged).",
            json!({"set": id("Option set id")}), &["set"]),
        tool("create_design_option", "Add an option to a set: empty, or a copy of `copy_from`'s layer. Primary only if the set has no other open option. To fill it, run any command with `in_design_option` set to its id. Returns {element, is_primary}.",
            json!({"set": id("Option set id"), "name": {"type": "string"}, "copy_from": id("Option whose layer to copy")}), &["set"]),
        tool("set_design_option", &format!("Edit an open design option: rename, make it primary (is_primary: true), or replace its layer (`changes`, checked against the shared model; usually easier: run commands with `in_design_option`). {CHANGES}"),
            json!({"option": id("Design option id"), "name": {"type": "string"}, "is_primary": {"type": "boolean"}, "changes": changes.clone()}), &["option"]),
        tool("delete_design_option", "Delete a design option and its layer. The primary option cannot be deleted (make another primary first).",
            json!({"option": id("Design option id")}), &["option"]),
        tool("accept_design_option", "Accept a design option: merge its layer into the shared model (like merging a branch). The other options of its set are archived (kept, not deleted). Returns {accepted, archived}.",
            json!({"option": id("Design option id")}), &["option"]),
        tool("apply_changes", &format!("Apply a change list to the document, all or nothing (e.g. from diff_documents). {CHANGES}"),
            json!({"changes": changes}), &["changes"]),
        tool("list_design_options", "Option sets in order with their options: id, name, primary, status (open / accepted / archived), what each layer adds / removes / overrides, and failures (layers that no longer apply, conflicts in the primary combination).",
            json!({}), &[]),
        tool("diff_documents", "Change list from one view of the open document to another, e.g. what option B changes compared with the primary combination. Feed it to apply_changes to reproduce it.",
            json!({"from": combination_schema("From (default \"shared\")"), "to": combination_schema("To (default [] = primary combination)")}), &[]),
    ]
}

/// Read a combination argument: `None` for `"shared"` (or a missing value
/// when `default_shared`), else the combination.
pub fn combination_arg(
    v: Option<&Value>,
    default_shared: bool,
) -> Result<Option<Combination>, String> {
    match v {
        None | Some(Value::Null) if default_shared => Ok(None),
        None | Some(Value::Null) => Ok(Some(Combination::primary())),
        Some(Value::String(s)) if s == "shared" => Ok(None),
        Some(Value::String(s)) if s == "primary" => Ok(Some(Combination::primary())),
        Some(Value::Array(a)) => {
            let mut options = Vec::new();
            for x in a {
                let n = x
                    .as_u64()
                    .ok_or_else(|| format!("option ids must be integers, not {x}"))?;
                options.push(ElementId(n));
            }
            Ok(Some(Combination { options }))
        }
        Some(other) => Err(format!(
            "a combination is \"shared\", \"primary\" or a list of option ids, not {other}"
        )),
    }
}

/// The document as seen in a combination (`None` = shared model). Both
/// carry empty option layers, so two views diff to model changes only.
pub fn seen(doc: &Document, c: &Option<Combination>) -> Result<Document, Value> {
    let r = match c {
        None => resolve_options(doc, &[]),
        Some(c) => resolve(doc, c),
    };
    r.map_err(
        |f| json!({"error": "the combination cannot be shown", "failures": failures_json(&f)}),
    )
}

/// `list_design_options`.
pub fn list_design_options(doc: &Document) -> Value {
    let sets: Vec<Value> = doc
        .option_sets()
        .iter()
        .map(|s| {
            let options: Vec<Value> = doc
                .design_options(s.id)
                .iter()
                .filter_map(|o| {
                    let d = o.as_design_option()?;
                    let c = layer_counts(doc, &d.changes);
                    Some(json!({"id": o.id, "name": o.name, "is_primary": d.is_primary, "status": d.status.as_str(), "changes": d.changes.len(), "added": c.added, "removed": c.removed, "overridden": c.overridden}))
                })
                .collect();
            json!({"id": s.id, "name": s.name, "primary": doc.primary_option(s.id), "options": options})
        })
        .collect();
    let mut failures: Vec<Value> = stale_layers(doc)
        .iter()
        .map(|e| json!({"severity": "warning", "code": e.code, "message": e.message, "elements": e.option.into_iter().collect::<Vec<_>>()}))
        .collect();
    if let Err(f) = resolve(doc, &Combination::primary()) {
        if let Value::Array(a) = failures_json(&f) {
            failures.extend(a);
        }
    }
    json!({"sets": sets, "failures": failures})
}

/// `diff_documents {from, to}`.
pub fn diff_documents(doc: &Document, args: &Value) -> Result<Value, Value> {
    let bad = |m: String| json!({"error": m});
    let from = combination_arg(args.get("from"), true).map_err(bad)?;
    let to = combination_arg(args.get("to"), false).map_err(bad)?;
    let (a, b) = (seen(doc, &from)?, seen(doc, &to)?);
    let changes = diff(&a, &b).map_err(|e| bad(e.to_string()))?;
    Ok(json!({"count": changes.len(), "changes": changes}))
}
