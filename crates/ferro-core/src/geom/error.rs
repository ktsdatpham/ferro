use std::fmt;

/// Failures of geometric construction. Codes are stable (`ferro.geom.*`).
#[derive(Debug, Clone, PartialEq)]
pub enum GeomError {
    /// A curve or edge would be shorter than `SHORT_CURVE_TOL`.
    ShortCurve,
    /// Points are coincident or collinear where a plane / arc is required.
    Degenerate,
    /// Curves do not connect end-to-start within `VERTEX_TOL`.
    LoopNotConnected,
    /// The loop does not close.
    LoopNotClosed,
    /// Curves of a loop or profile are not coplanar.
    NotPlanar,
    /// A hole lies outside the outer loop, or loops intersect.
    InvalidProfile,
    /// Radius must be positive.
    InvalidRadius,
}

impl GeomError {
    /// Stable failure code for agents and logs.
    pub fn code(&self) -> &'static str {
        match self {
            GeomError::ShortCurve => "ferro.geom.short_curve",
            GeomError::Degenerate => "ferro.geom.degenerate",
            GeomError::LoopNotConnected => "ferro.geom.loop_not_connected",
            GeomError::LoopNotClosed => "ferro.geom.loop_not_closed",
            GeomError::NotPlanar => "ferro.geom.not_planar",
            GeomError::InvalidProfile => "ferro.geom.invalid_profile",
            GeomError::InvalidRadius => "ferro.geom.invalid_radius",
        }
    }
}

impl fmt::Display for GeomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            GeomError::ShortCurve => "curve is shorter than the minimum length (0.5 mm)",
            GeomError::Degenerate => "points are coincident or collinear",
            GeomError::LoopNotConnected => "curves do not connect end to start",
            GeomError::LoopNotClosed => "loop is not closed",
            GeomError::NotPlanar => "curves are not coplanar",
            GeomError::InvalidProfile => {
                "profile loops overlap or a hole is outside the outer loop"
            }
            GeomError::InvalidRadius => "radius must be positive",
        };
        write!(f, "{msg}")
    }
}

impl std::error::Error for GeomError {}
