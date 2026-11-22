//! ferro-core: the model kernel.
//!
//! Layers (Foundation Spec ch. 7), each depending only on those below:
//! `math` → `geom` → `topo` → `algo` → `model` → `ops`.
//!
//! Milestone 1 is rebuilding the v0.1 prototype layer by layer
//! (see `docs/milestones/M1.md`). `model` below is still the prototype.

pub mod geom;
pub mod math;
pub mod model;

pub use math::{Angle, BoundingBox3, Length, Point2, Point3, Transform, UnitVec3, Vec2, Vec3};
pub use model::{Entity, EntityId, EntityKind, Face, Model, ModelError, Solid};
