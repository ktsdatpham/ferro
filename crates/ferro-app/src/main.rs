//! Placeholder for the Ferro app.
//!
//! Milestone 1 replaces this with a window (winit) and a wgpu viewport:
//! orbit / pan / zoom, rectangle tool, push-pull tool, inference snapping.
//! See docs/ROADMAP.md. Until then this runs a tiny headless demo.

use ferro_core::{EntityKind, Model, Vec3};

fn main() {
    let mut model = Model::new();
    let room = model
        .add_rectangle(Vec3::ZERO, 4000.0, 3000.0)
        .expect("valid rectangle");
    model.push_pull(room, 2800.0).expect("valid push-pull");

    for e in model.entities() {
        if let EntityKind::Solid(s) = &e.kind {
            println!("entity {} is a solid with {} faces", e.id, s.faces.len());
        }
    }
    println!("\nNative .fro JSON:\n{}", model.to_json());
}
