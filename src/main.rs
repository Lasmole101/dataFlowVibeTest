//! Entry point for the Data Flow Diagramming Tool.
//!
//! All modules are declared in `lib.rs`.  This file only contains `main`.

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Data Flow Diagramming Tool")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "dataflow",
        native_options,
        Box::new(|cc| Ok(Box::new(dataFlowClaude::app::AppState::new(cc)))),
    )
}
