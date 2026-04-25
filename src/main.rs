//! Entry point for the Data Flow Diagramming Tool.
//!
//! Initialises [`eframe`] and hands control to [`app::AppState`].

#![warn(clippy::all, missing_docs)]

mod app;
mod model;
mod persistence;
mod ui;
mod undo;
mod validation;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Data Flow Diagramming Tool")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "dataFlowClaude",
        native_options,
        Box::new(|cc| Ok(Box::new(app::AppState::new(cc)))),
    )
}
