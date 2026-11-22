//! Geometry layer (Foundation Spec ch. 2): planes, bounded curves, curve loops,
//! profiles and surfaces. Values only: no identity, no document.

mod arc;
mod curve;
mod curve_loop;
mod error;
mod line;
mod plane;
mod profile;
mod surface;

pub mod polygon;
pub mod predicates;

pub use arc::Arc;
pub use curve::Curve;
pub use curve_loop::{CurveLoop, Orientation};
pub use error::GeomError;
pub use line::Line;
pub use plane::Plane;
pub use profile::Profile;
pub use surface::{Cylinder, Surface};
