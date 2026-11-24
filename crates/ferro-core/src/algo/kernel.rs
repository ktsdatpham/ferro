//! The kernel seam (ADR 0007, D-78): operations Ferro's own planar
//! algorithms do not cover (booleans first; blends, offsets and curved
//! primitives later) go through the [`Kernel`] trait. `ferro-core` only
//! defines the trait and its data; an implementation (the `ferro-kernel`
//! crate, on solidcore) is injected into a `Session` like its `IdSource`.
//! `ferro-core` never depends on a kernel library.

use crate::topo::{Body, FaceId, Lineage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Which boolean a [`Kernel::boolean`] computes: a value passed to the
/// kernel, not a `Command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    /// Everything in `a` or in `b` (Revit: `BooleanOperationsType.Union`).
    Union,
    /// `a` less `b` (Revit: `BooleanOperationsType.Difference`).
    Difference,
    /// What is in both `a` and `b` (Revit: `BooleanOperationsType.Intersect`).
    Intersection,
}

/// One input face an output face came from (D-25 lineage across two
/// operands). An output face with no source is new.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceSource {
    /// It lies on face `0` of operand `a` (kept, split from it or merged
    /// from it).
    FromA(FaceId),
    /// It lies on face `0` of operand `b`.
    FromB(FaceId),
}

/// The result of a kernel operation on operand `a` (and `b`).
///
/// `body` lives in `a`'s id space: a face of `a` that survives (whole, or as
/// the largest of its pieces) keeps its id, as do vertices and edges of `a`
/// that survive; everything else gets new ids from `a`'s never-reused
/// counter (D-25). So a command that replaces `a`'s body with `body` keeps
/// the references to `a`'s surviving faces.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelResult {
    /// The result, a closed solid (possibly of several shells) that passes
    /// [`Body::validate_solid`].
    pub body: Body,
    /// Every face of `body`, with the input faces it came from (sorted;
    /// empty for a new face).
    pub faces: BTreeMap<FaceId, Vec<FaceSource>>,
    /// What happened to `a`'s faces: `kept` (same id in `body`), `created`
    /// (new id, with the face of `a` it came from when there is one) and
    /// `deleted`.
    pub lineage: Lineage,
}

/// A kernel failure with a stable code (`ferro.kernel.*`, §5.6) and a human
/// message. Never a panic: implementations map every library error here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelError {
    /// Stable code, one of the `KernelError::*` constants.
    pub code: &'static str,
    /// What went wrong, for people and agents.
    pub message: String,
}

impl KernelError {
    /// No kernel is attached to the session (tests, minimal builds).
    pub const UNAVAILABLE: &'static str = "ferro.kernel.unavailable";
    /// An operand is not a closed, valid solid.
    pub const NOT_SOLID: &'static str = "ferro.kernel.not_solid";
    /// The operand or result has geometry the kernel seam does not carry
    /// yet (for example curved faces).
    pub const UNSUPPORTED: &'static str = "ferro.kernel.unsupported";
    /// The result is empty (an intersection of disjoint solids, or a
    /// difference that removes everything).
    pub const EMPTY_RESULT: &'static str = "ferro.kernel.empty_result";
    /// The result is not manifold (solids touching only along an edge or at
    /// a vertex), which a Ferro `Body` cannot hold.
    pub const NON_MANIFOLD_RESULT: &'static str = "ferro.kernel.non_manifold_result";
    /// The result failed Ferro's validation (`Body::validate_solid`).
    pub const INVALID_RESULT: &'static str = "ferro.kernel.invalid_result";
    /// The kernel refused or failed the operation (its message says why).
    pub const FAILED: &'static str = "ferro.kernel.failed";

    /// A failure with `code` (one of the constants above) and `message`.
    pub fn new(code: &'static str, message: impl Into<String>) -> KernelError {
        KernelError {
            code,
            message: message.into(),
        }
    }

    /// `ferro.kernel.unavailable`: what a kernel command reports when its
    /// session has no kernel.
    pub fn unavailable() -> KernelError {
        KernelError::new(
            Self::UNAVAILABLE,
            "no geometry kernel is attached to this session (booleans need ferro-kernel)",
        )
    }
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for KernelError {}

/// A B-rep kernel behind Ferro's seam (ADR 0007). Implementations are pure
/// functions of their inputs: the same operands give the same result, bit
/// for bit, on every call and thread (deterministic, D-19).
///
/// All coordinates are millimetres (D-01). Implementations snap and merge at
/// Ferro's tolerances (D-06, `math::tol`): faces of `b` within `VERTEX_TOL`
/// of a plane of `a` count as flush with it, result vertices closer than
/// `VERTEX_TOL` are one vertex, and neighbouring coplanar faces are one face.
pub trait Kernel: Send + Sync {
    /// A short name for diagnostics (`"solidcore"`).
    fn name(&self) -> &'static str;

    /// `a` `op` `b`, both closed solids. The result keeps `a`'s ids where
    /// `a`'s faces survive (see [`KernelResult`]). A union of disjoint
    /// solids is one body of several shells.
    ///
    /// # Errors
    /// `ferro.kernel.not_solid` for an operand that is not a closed valid
    /// solid; `ferro.kernel.unsupported` for curved faces;
    /// `ferro.kernel.empty_result` for an empty intersection or difference;
    /// `ferro.kernel.non_manifold_result` for solids that touch only along an
    /// edge or at a vertex; `ferro.kernel.invalid_result` when the result
    /// fails `Body::validate_solid`; `ferro.kernel.failed` when the kernel
    /// itself refuses.
    fn boolean(&self, op: BooleanOp, a: &Body, b: &Body) -> Result<KernelResult, KernelError>;
}
