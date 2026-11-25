#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use ai_coding_agent::platform::run_native_window;
use ai_coding_agent::ui::studio::StudioApp;

/// Command-line surface. Every flag here is wired to a real app action — the
/// Linux .desktop entry's "New Session" / "Agentic Browser" actions depend on
/// `--new-session` and `--browser` actually doing something.
const USAGE: &str = "AirCode — Pure-Rust AI Coding Studio & Agentic Browser

Usage:
  AI-Coding-Agent [path]            open a workspace (file or folder)
  AI-Coding-Agent --new-session     start a fresh session instead of resuming
  AI-Coding-Agent --browser         open the agentic browser
  AI-Coding-Agent --connect         go straight to provider setup
  AI-Coding-Agent --help            show this help

With no arguments AirCode resumes the most recent project.
";

fn main() {
    let mut workspace: Option<PathBuf> = None;
    let mut new_session = false;
    let mut browser = false;
    let mut connect = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--new-session" | "--new" => new_session = true,
            "--browser" => browser = true,
            "--connect" | "--auth" => connect = true,
            "--help" | "-h" => {
                print!("{}", USAGE);
                return;
            }
            other => {
                if other.starts_with('-') {
                    eprintln!("Unknown option: {} (try --help)", other);
                    std::process::exit(2);
                }
                let p = PathBuf::from(other);
                if p.exists() {
                    workspace = Some(p);
                } else {
                    eprintln!("No such file or directory: {}", other);
                    std::process::exit(2);
                }
            }
        }
    }

    let workspace = workspace.map(|p| {
        if p.is_file() {
            p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
        } else {
            p
        }
    });

    let mut app = StudioApp::new(workspace);
    if new_session {
        app.start_new_session();
    }
    if browser {
        app.open_browser_view();
    }
    if connect {
        app.open_connect_screen();
    }

    run_native_window(
        "AirCode — Pure-Rust AI Coding Studio & Agentic Browser",
        1520,
        940,
        Box::new(app),
    );
}
