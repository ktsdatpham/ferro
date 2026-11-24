use std::fmt;

/// Topological failures. Codes are stable (`ferro.topo.*`).
#[derive(Debug, Clone, PartialEq)]
pub enum TopoError {
    /// A loop has fewer than three distinct vertices, or repeats a vertex.
    DegenerateLoop,
    /// An edge is used more than twice, or twice in the same direction.
    NonManifold,
    /// A face, vertex or loop id does not exist in the body.
    NotFound(String),
    /// Internal links are inconsistent (bug guard).
    Corrupt(String),
    /// A solid must be closed.
    NotClosed,
    /// A closed body has non-positive volume (faces point inward).
    InwardFacing,
    /// A planar face has vertices off its plane.
    NonPlanarFace(String),
    /// Two faces cross or touch away from their shared edges and vertices
    /// (the message names them); decided with exact predicates (D-53).
    SelfIntersecting(String),
}

impl TopoError {
    pub fn code(&self) -> &'static str {
        match self {
            TopoError::DegenerateLoop => "ferro.topo.degenerate_loop",
            TopoError::NonManifold => "ferro.topo.non_manifold",
            TopoError::NotFound(_) => "ferro.topo.not_found",
            TopoError::Corrupt(_) => "ferro.topo.corrupt",
            TopoError::NotClosed => "ferro.topo.not_closed",
            TopoError::InwardFacing => "ferro.topo.inward_facing",
            TopoError::NonPlanarFace(_) => "ferro.topo.non_planar_face",
            TopoError::SelfIntersecting(_) => "ferro.topo.self_intersecting",
        }
    }
}

impl fmt::Display for TopoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopoError::DegenerateLoop => write!(f, "loop is degenerate"),
            TopoError::NonManifold => write!(f, "geometry is non-manifold"),
            TopoError::NotFound(s) => write!(f, "{s} not found"),
            TopoError::Corrupt(s) => write!(f, "corrupt topology: {s}"),
            TopoError::NotClosed => write!(f, "body is not closed"),
            TopoError::InwardFacing => write!(f, "solid faces point inward"),
            TopoError::NonPlanarFace(s) => write!(f, "face {s} is not planar"),
            TopoError::SelfIntersecting(s) => write!(f, "the body intersects itself ({s})"),
        }
    }
}

impl std::error::Error for TopoError {}
