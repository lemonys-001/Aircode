#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use ai_coding_agent::platform::run_native_window;
use ai_coding_agent::ui::studio::StudioApp;

fn main() {
    let initial_workspace = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .map(|p| {
            if p.is_file() {
                p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
            } else {
                p
            }
        });

    let app = StudioApp::new(initial_workspace);
    run_native_window(
        "AirCode — Pure-Rust AI Coding Studio & Agentic Browser",
        1520,
        940,
        Box::new(app),
    );
}
