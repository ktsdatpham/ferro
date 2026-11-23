//! Operations layer (Foundation Spec ch. 4): the only way to change a
//! `Document` (D-17).
//!
//! * A [`Command`] is a serialisable intent; its name equals its MCP tool name.
//! * Commands run inside a [`Transaction`] on a working copy; regeneration then
//!   updates dependents and computed parameters (D-18).
//! * Errors roll everything back; warnings are reported and commit.
//! * A [`Session`] owns the current document, undo / redo, transaction groups
//!   (one undo step per gesture) and the journal (D-29).

pub mod bim_commands;
pub mod commands;
mod events;
mod failure;
pub mod form_commands;
pub mod geometry_source;
pub mod material_commands;
pub mod parameter_commands;
pub mod project_commands;
mod session;
mod transaction;

pub use commands::{AnyCommand, Command, CommandOutput};
pub use events::DocumentChanged;
pub use failure::{CommandError, Failure, Severity};
pub use session::{Actor, ActorKind, ExecContext, JournalEntry, Outcome, Session};
pub use transaction::{IdSource, SequentialIds, Transaction};
#[cfg(test)]
mod bim_tests;
#[cfg(test)]
mod change_tests;
#[cfg(test)]
mod form_tests;
#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod geometry_source_tests;
#[cfg(test)]
mod material_tests;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod scenario_common_tests;
#[cfg(test)]
mod scenario_tests;
#[cfg(test)]
mod stress_tests;
