//! Model layer (Foundation Spec ch. 3): the `Document` and its `Element`s.
//!
//! The document is an immutable, structurally shared tree (D-24): cloning is
//! O(1) and every committed transaction produces a new version. Mutation is
//! only possible through `ops::Transaction` (D-17); the mutating methods here
//! are crate-private.

pub mod bim;
mod category;
pub mod change;
mod document;
mod element;
pub mod generate;
mod ids;
pub mod material;
mod parameter;
mod parameter_definition;
pub mod phase;
pub mod phase_quantities;
pub mod project_settings;
pub mod structure_quantities;

pub use category::Category;
pub use document::{Document, ProjectInfo, ProjectOrigins, EXTENT_LIMIT_MM};
pub use element::{Element, ElementBody, Form, GeometrySource, Level};
pub use ids::{ElementId, Uid};
pub use parameter::{keys, ParamValue};
pub use parameter_definition::{
    builtin_definition, builtin_definitions, IfcPropertyMapping, ParameterBinding,
    ParameterDataType, ParameterDefinition, ParameterGroup,
};
pub use phase::{Phase, PhaseStatus};
pub use phase_quantities::PhaseQuantities;
