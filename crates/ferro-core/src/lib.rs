//! ferro-core: the model kernel.
//!
//! Everything the app, the AI agents (via MCP) and the Revit bridge do goes
//! through [`Model`]. Keep this crate free of UI and IO so it stays easy to
//! test and easy for contributors to understand.

pub mod geom;
pub mod model;
pub mod units;

pub use geom::Vec3;
pub use model::{Entity, EntityId, EntityKind, Face, Model, ModelError, Solid};
pub use units::Length;
