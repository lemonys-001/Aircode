#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use ai_coding_agent::platform::run_native_window;
use ai_coding_agent::ui::studio::StudioApp;

/// Command-line surface. Every flag here is wired to a real app action — the
/// Linux .desktop entry's "New Session" / "Agentic Browser" actions depend on
/// `--new-session` and `--browser` actually doing something.
const USAGE: &str = "AirCode — Pure-Rust AI Coding Studio & Agentic Browser

Usage:
  AI-Coding-Agent [path]            open a workspace (file or folder) in the GUI
  AI-Coding-Agent --tui             run in the terminal instead of a window
  AI-Coding-Agent --web             run in the browser (starts a local server)
  AI-Coding-Agent --new-session     start a fresh session instead of resuming
  AI-Coding-Agent --browser         open the agentic browser
  AI-Coding-Agent --connect         go straight to provider setup
  AI-Coding-Agent --port N          port for --web (default 7777)
  AI-Coding-Agent --host ADDR       bind address for --web (default 127.0.0.1)
  AI-Coding-Agent --version         print the version and exit
  AI-Coding-Agent --help            show this help

Front-ends (all drive the same engine):
  gui   the native desktop window (default)
  tui   the terminal UI: AI-Coding-Agent --tui
  web   a browser UI on http://127.0.0.1:7777

With no arguments AirCode resumes the most recent project.
";

/// The executable name, kept in one place because `--version`, the Linux
/// installer and the Windows resource all have to agree on it.
const BIN_NAME: &str = "AI-Coding-Agent";

fn main() {
    let mut workspace: Option<PathBuf> = None;
    let mut new_session = false;
    let mut browser = false;
    let mut connect = false;
    let mut tui = false;
    let mut web = false;
    let mut port: u16 = 7777;
    let mut host = "127.0.0.1".to_string();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // --port/--host consume the next argument, so walk with an index.
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "--new-session" | "--new" => new_session = true,
            "--browser" => browser = true,
            "--connect" | "--auth" => connect = true,
            "--tui" | "--terminal" => tui = true,
            "--web" | "--server" => web = true,
            "--port" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u16>().ok()) {
                    Some(p) => port = p,
                    None => {
                        eprintln!("--port needs a number between 1 and 65535");
                        std::process::exit(2);
                    }
                }
            }
            "--host" => {
                i += 1;
                match args.get(i) {
                    Some(h) => host = h.clone(),
                    None => {
                        eprintln!("--host needs an address, e.g. 127.0.0.1 or 0.0.0.0");
                        std::process::exit(2);
                    }
                }
            }
            "--help" | "-h" => {
                print!("{}", USAGE);
                return;
            }
            // The Linux installer asks the installed binary for its version to
            // decide whether an update is needed, so this must stay cheap and
            // must not touch the terminal or open a window.
            "--version" | "-V" => {
                println!("{} {}", BIN_NAME, env!("CARGO_PKG_VERSION"));
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
        i += 1;
    }

    let workspace = workspace.map(|p| {
        if p.is_file() {
            p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
        } else {
            p
        }
    });

    // A TUI or web front-end owns the terminal / the socket, so it must not
    // also spawn the desktop shell it would nest inside.
    let headless = tui || web;
    let mut app = StudioApp::new_with_options(workspace, !headless);
    if new_session {
        app.start_new_session();
    }
    if browser {
        app.open_browser_view();
    }
    if connect {
        app.open_connect_screen();
    }

    if tui && web {
        eprintln!("Pick one front-end: --tui or --web (try --help)");
        std::process::exit(2);
    }

    if tui {
        ai_coding_agent::tui::print_banner(&app.snapshot());
        ai_coding_agent::tui::run(app);
        return;
    }

    if web {
        ai_coding_agent::web::run(app, &host, port);
        return;
    }

    run_native_window(
        "AirCode — Pure-Rust AI Coding Studio & Agentic Browser",
        1520,
        940,
        Box::new(app),
    );
}
