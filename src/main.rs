//! Binary entry point for the Data Flow Diagramming Tool.
//!
//! All modules are declared in `lib.rs`; this file contains only `main`.
//! The application is a native desktop window managed by [`eframe`].

fn main() -> eframe::Result<()> {
    // Initialise a basic logger so that egui's internal warnings reach stderr
    // in debug builds.  No-op if a logger is already installed.
    #[cfg(debug_assertions)]
    eprintln!("DataFlow starting (debug build)");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DataFlow")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "dataflow",
        native_options,
        Box::new(|cc| Ok(Box::new(dataFlowClaude::app::AppState::new(cc)))),
    )
}
