//! ferro-core: the model kernel.
//!
//! Layers (Foundation Spec ch. 7), each depending only on those below:
//! `math` → `geom` → `topo` → `algo` → `model` → `ops`.
//!
//! Pure (D-19): no IO, no UI, no clocks, no randomness, deterministic.

pub mod algo;
pub mod geom;
pub mod math;
pub mod model;
pub mod ops;
pub mod topo;

pub use math::{
    Angle, BoundingBox3, Length, Point2, Point3, QuantitySpec, Transform, UnitVec3, Vec2, Vec3,
};
pub use model::{Category, Document, Element, ElementBody, ElementId, ParamValue, Uid};
pub use ops::{AnyCommand, ExecContext, Session};
