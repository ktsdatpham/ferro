//! Math layer (Foundation Spec ch. 1–2): points, vectors, transforms, bounding
//! boxes, quantities and tolerances. Depends on nothing else in `ferro-core`.

mod bbox;
mod point;
mod quantity;
mod transform;
mod vector;

pub mod tol;

pub use bbox::BoundingBox3;
pub use point::{Point2, Point3};
pub use quantity::{Angle, Length};
pub use transform::Transform;
pub use vector::{UnitVec3, Vec2, Vec3};
