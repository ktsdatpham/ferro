//! File format versions and migrations (D-21, M5-21).
//!
//! `format_version` goes up whenever saved data gains a shape an older Ferro
//! cannot read. Opening a file runs every migration step from its version to
//! [`FORMAT_VERSION`]; saving always writes the current version. A file newer
//! than this app is refused with `ferro.io.version`.
//!
//! | Version | Milestone | What changed |
//! |---|---|---|
//! | 1 | M1–M4 | levels, Forms, walls, floors, openings, types, materials (colour, roughness) |
//! | 2 | M5 | quantity-typed parameters (`{"type":"quantity"}`), parameter definitions in the project, material assets (appearance, physical, thermal, environmental, cost), direct geometry of any category |
//! | 3 | M5 | phases (ADR 0006): elements of category `phases` with a `{"kind":"phase"}` body; `phase_created` / `phase_demolished` filled in |
//! | 4 | M5 | the relationship table (M5-5) |
//! | 5 | M5 | views (D-33, M5-22): elements of category `views` with a `{"kind":"view"}` body (camera, cut, display style, hidden elements / tags as uids, phase, phase filter) |
//! | 6 | M5 | design options (ADR 0006): elements of category `design_options` with `{"kind":"option_set"}` / `{"kind":"design_option"}` bodies (layers in the change format); `design_option` stays `null` in files |
//!
//! Versions 2 to 6 only add data, so older files need no rewriting; the
//! steps are still listed so every version has one and the chain is tested.
//! Version 3 is needed because a format 2 reader refuses the new `phases`
//! category and `phase` body (unknown enum variants), even though the
//! element fields existed (always `null`) before. Versions 5 and 6 likewise: a
//! format 4 reader refuses the `views` category and `view` body, a format 5 reader the `design_options` category.

use crate::IoError;
use serde_json::Value;

/// Current file format version.
pub const FORMAT_VERSION: u32 = 6;

/// One migration: from version `n` to `n + 1`, on the raw JSON of the project
/// and the elements (before they are parsed).
type Step = fn(&mut Value, &mut [Value]) -> Result<(), IoError>;

/// `STEPS[i]` migrates version `i + 1` to `i + 2`.
const STEPS: [Step; (FORMAT_VERSION - 1) as usize] =
    [v1_to_v2, v2_to_v3, v3_to_v4, v4_to_v5, v5_to_v6];

/// 1 → 2: additive (new optional fields and parameter types); nothing to rewrite.
fn v1_to_v2(_project: &mut Value, _elements: &mut [Value]) -> Result<(), IoError> {
    Ok(())
}

/// 2 → 3: additive (phases); nothing to rewrite.
fn v2_to_v3(_project: &mut Value, _elements: &mut [Value]) -> Result<(), IoError> {
    Ok(())
}

/// 3 → 4: additive (the relationship table, M5-5); older files have none.
/// The bump makes older readers refuse files they would lose rows from.
fn v3_to_v4(_project: &mut Value, _elements: &mut [Value]) -> Result<(), IoError> {
    Ok(())
}

/// 4 → 5: additive (views); nothing to rewrite.
fn v4_to_v5(_project: &mut Value, _elements: &mut [Value]) -> Result<(), IoError> {
    Ok(())
}

/// 5 → 6: additive (design options, ADR 0006); nothing to rewrite.
fn v5_to_v6(_project: &mut Value, _elements: &mut [Value]) -> Result<(), IoError> {
    Ok(())
}

/// Check a file's version and bring its data up to [`FORMAT_VERSION`].
///
/// Fails with `ferro.io.version` when the version is missing, 0, or newer
/// than this app.
pub fn migrate(version: u32, project: &mut Value, elements: &mut [Value]) -> Result<(), IoError> {
    if version == 0 {
        return Err(IoError::new(
            "ferro.io.version",
            "the file has no format version; it was not written by Ferro",
        ));
    }
    if version > FORMAT_VERSION {
        return Err(IoError::new(
            "ferro.io.version",
            format!(
                "this file was saved by a newer Ferro (file format {version}, this app reads up to {FORMAT_VERSION}); please update Ferro"
            ),
        ));
    }
    for step in &STEPS[(version - 1) as usize..] {
        step(project, elements)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_version_has_a_step_and_current_is_a_no_op() {
        assert_eq!(STEPS.len() as u32, FORMAT_VERSION - 1);
        let (mut p, mut e) = (json!({"name": "x"}), vec![json!({"kind": "level"})]);
        migrate(FORMAT_VERSION, &mut p, &mut e).unwrap();
        migrate(1, &mut p, &mut e).unwrap();
        migrate(2, &mut p, &mut e).unwrap();
        migrate(3, &mut p, &mut e).unwrap();
        migrate(4, &mut p, &mut e).unwrap();
        assert_eq!(p, json!({"name": "x"}));
    }

    #[test]
    fn missing_and_future_versions_are_refused_clearly() {
        let (mut p, mut e) = (json!({}), Vec::new());
        let err = migrate(0, &mut p, &mut e).unwrap_err();
        assert_eq!(err.code, "ferro.io.version");
        let err = migrate(FORMAT_VERSION + 1, &mut p, &mut e).unwrap_err();
        assert!(err.message.contains("newer Ferro"), "{}", err.message);
    }
}

#[cfg(test)]
mod file_tests {
    use crate::{document_from_json, document_to_json, load_fro, save_fro};

    /// A house saved in format 1 (M4): levels, walls with joins, a floor,
    /// a door, a window, wall / floor types, a material, a Form.
    const M4_HOUSE: &str = include_str!("../tests/fixtures/m4-house.v1.fro.json");

    #[test]
    fn m4_json_file_opens_and_saves_as_current_version() {
        let doc = document_from_json(M4_HOUSE).expect("format 1 opens");
        assert!(doc.len() >= 8, "elements kept: {}", doc.len());
        let again = document_to_json(&doc).unwrap();
        assert!(again.contains(&format!("\"format_version\": {}", super::FORMAT_VERSION)));
        assert_eq!(document_from_json(&again).unwrap(), doc, "round trip");
    }

    #[test]
    fn m4_fro_file_opens_and_is_upgraded_on_save() {
        let doc = document_from_json(M4_HOUSE).unwrap();
        let dir = std::env::temp_dir().join(format!("ferro-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("house.fro");
        save_fro(&path, &doc, &[]).unwrap();
        // Make it a format 1 file, as M4 wrote it.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE meta SET value = '1' WHERE key = 'format_version'",
            [],
        )
        .unwrap();
        drop(conn);
        let (loaded, _) = load_fro(&path).expect("format 1 .fro opens");
        assert_eq!(loaded, doc);
        save_fro(&path, &loaded, &[]).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        let v: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'format_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, super::FORMAT_VERSION.to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
