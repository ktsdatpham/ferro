//! Ferro desktop app (Windows, macOS, Linux).
//!
//!   cargo run -p ferro-app --release [-- path/to/project.fro]

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod camera;
mod link;
mod material_editor;
mod render;
mod scene;
mod tools;

use eframe::egui;

fn main() -> eframe::Result {
    let open = std::env::args().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ferro")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Ferro",
        options,
        Box::new(move |cc| Ok(Box::new(app::FerroApp::new(cc, open)))),
    )
}
