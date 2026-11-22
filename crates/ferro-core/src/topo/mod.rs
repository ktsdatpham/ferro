//! Topology layer (Foundation Spec §2.6): half-edge boundary representation.
//!
//! * Topology (connectivity) and geometry (points, surfaces) are separate.
//! * Every vertex / edge / face id is **persistent** within a body and never
//!   reused (D-25); algorithms report what came from what in a [`Lineage`].
//! * A closed body is a solid: closed, manifold, outward-facing (D-11).
//!   Open bodies (sheets, SketchUp-style loose faces) are valid Form geometry.

mod body;
mod error;
mod ids;
mod lineage;

#[cfg(test)]
pub(crate) use body::tests_support;
pub use body::{Body, Face, FaceSpec, HalfEdge, Loop, Vertex};
pub use error::TopoError;
pub(crate) use ids::de_u64;
pub use ids::{EdgeId, FaceId, HalfEdgeId, LoopId, VertexId};
pub use lineage::Lineage;
