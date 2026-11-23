//! File IO for Ferro (outside the pure kernel, D-19).
//!
//! * `.fro`: SQLite container (D-21): elements, project, journal; full and
//!   incremental saves; crash-safe.
//! * `.fro.json`: JSON interchange with the same logical schema; element
//!   references are written as `Uid`s.
//! * OBJ and glTF (GLB) export (D-75, M1); IFC4 export (M3).
//! * [`SystemIds`]: time + random UUID v7 source for apps.

mod export;
mod fro;
mod ids;
mod ifc;
mod json;
mod migrate;

pub use export::{export_glb, export_obj};
pub use fro::{load_fro, save_fro, FroStore};
pub use ids::SystemIds;
pub use ifc::{export_ifc, export_ifc_combination, export_ifc_phase, ifc_guid};
pub use json::{document_from_json, document_to_json};
pub use migrate::{migrate, FORMAT_VERSION};

use std::fmt;

/// IO failure with a stable code (`ferro.io.*`).
#[derive(Debug)]
pub struct IoError {
    pub code: &'static str,
    pub message: String,
}

impl IoError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        IoError {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for IoError {}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::new("ferro.io.file", e.to_string())
    }
}
impl From<rusqlite::Error> for IoError {
    fn from(e: rusqlite::Error) -> Self {
        IoError::new("ferro.io.sqlite", e.to_string())
    }
}
impl From<serde_json::Error> for IoError {
    fn from(e: serde_json::Error) -> Self {
        IoError::new("ferro.io.json", e.to_string())
    }
}
