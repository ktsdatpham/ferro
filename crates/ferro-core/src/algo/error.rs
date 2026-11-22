use crate::geom::GeomError;
use crate::topo::TopoError;
use std::fmt;

/// Failures of algorithms. Codes are stable (`ferro.algo.*` or inner codes).
#[derive(Debug, Clone, PartialEq)]
pub enum AlgoError {
    Geom(GeomError),
    Topo(TopoError),
    /// Distance must be at least `SHORT_CURVE_TOL`.
    ZeroDistance,
    /// The push / pull would collapse or invert the solid.
    Collapse,
    /// Configuration not supported yet (message says which).
    Unsupported(&'static str),
    /// Triangulation failed (self-intersecting face).
    Triangulation,
}

impl AlgoError {
    pub fn code(&self) -> &'static str {
        match self {
            AlgoError::Geom(e) => e.code(),
            AlgoError::Topo(e) => e.code(),
            AlgoError::ZeroDistance => "ferro.algo.zero_distance",
            AlgoError::Collapse => "ferro.algo.collapse",
            AlgoError::Unsupported(_) => "ferro.algo.unsupported",
            AlgoError::Triangulation => "ferro.algo.triangulation",
        }
    }
}

impl fmt::Display for AlgoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlgoError::Geom(e) => write!(f, "{e}"),
            AlgoError::Topo(e) => write!(f, "{e}"),
            AlgoError::ZeroDistance => write!(f, "distance is too small (minimum 0.5 mm)"),
            AlgoError::Collapse => write!(f, "this would collapse or turn the solid inside out"),
            AlgoError::Unsupported(s) => write!(f, "not supported yet: {s}"),
            AlgoError::Triangulation => write!(f, "face could not be triangulated"),
        }
    }
}

impl std::error::Error for AlgoError {}

impl From<GeomError> for AlgoError {
    fn from(e: GeomError) -> Self {
        AlgoError::Geom(e)
    }
}
impl From<TopoError> for AlgoError {
    fn from(e: TopoError) -> Self {
        AlgoError::Topo(e)
    }
}
