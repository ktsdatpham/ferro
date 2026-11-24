use crate::algo::{AlgoError, KernelError};
use crate::geom::GeomError;
use crate::model::ElementId;
use crate::topo::TopoError;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Commit allowed; shown to the user / agent.
    Warning,
    /// Transaction rolls back.
    Error,
}

/// A reported problem with a stable code (`ferro.<area>.<problem>`), a human
/// message and the affected elements (§4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub elements: Vec<ElementId>,
}

impl Failure {
    pub fn error(code: &str, message: impl Into<String>, elements: Vec<ElementId>) -> Failure {
        Failure {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            elements,
        }
    }
    pub fn warning(code: &str, message: impl Into<String>, elements: Vec<ElementId>) -> Failure {
        Failure {
            severity: Severity::Warning,
            ..Failure::error(code, message, elements)
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// Error returned by a command; becomes an `Error` failure.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub elements: Vec<ElementId>,
}

impl CommandError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        CommandError {
            code: code.into(),
            message: message.into(),
            elements: Vec::new(),
        }
    }
    pub fn on(mut self, id: ElementId) -> Self {
        self.elements.push(id);
        self
    }
    pub fn not_found(id: ElementId) -> Self {
        CommandError::new(
            "ferro.model.not_found",
            format!("element {id} does not exist"),
        )
        .on(id)
    }
    pub fn into_failure(self) -> Failure {
        Failure::error(&self.code, self.message, self.elements)
    }
}

impl From<AlgoError> for CommandError {
    fn from(e: AlgoError) -> Self {
        CommandError::new(e.code(), e.to_string())
    }
}
impl From<GeomError> for CommandError {
    fn from(e: GeomError) -> Self {
        CommandError::new(e.code(), e.to_string())
    }
}
impl From<TopoError> for CommandError {
    fn from(e: TopoError) -> Self {
        CommandError::new(e.code(), e.to_string())
    }
}
impl From<KernelError> for CommandError {
    fn from(e: KernelError) -> Self {
        CommandError::new(e.code, e.message)
    }
}
