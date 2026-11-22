//! Algorithms (Foundation Spec §2.7): pure functions over geometry and
//! topology. Each documents its tolerance use and returns explicit results.

mod error;
mod extrude;
mod mesh;
mod push_pull;

pub use error::AlgoError;
pub use extrude::{extrude_profile, ExtrudeFaces};
pub use mesh::{body_mesh, triangulate_face, Mesh};
pub use push_pull::{push_pull_face, PushPullMode};
