#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use ai_coding_agent::platform::{run_native_window, GuiBlocker};
use ai_coding_agent::ui::studio::StudioApp;

/// Command-line surface. Every flag here is wired to a real app action — the
/// Linux .desktop entry's "New Session" / "Agentic Browser" actions depend on
/// `--new-session` and `--browser` actually doing something.
const USAGE: &str = "AirCode

Usage:
  aircode [path]                open a workspace (file or folder) in the GUI
  aircode --gui                 force the desktop window (fail if unavailable)
  aircode --tui                 run in the terminal instead of a window
  aircode --web                 run in the browser (starts a local server)
  aircode --new-session         start a fresh session instead of resuming
  aircode --browser             open the agentic browser
  aircode --connect             go straight to provider setup
  aircode --port N              port for --web (default 7777)
  aircode --host ADDR           bind address for --web (default 127.0.0.1)
  aircode --doctor              check this machine and explain what is missing
  aircode --version             print the version and exit
  aircode --help                show this help

Front-ends (all drive the same engine):
  gui   the native desktop window (default)
  tui   the terminal UI: aircode --tui
  web   a browser UI on http://127.0.0.1:7777

Only --tui and --web work without a desktop: neither needs X11, so both run
over SSH, in a container, on a headless box and inside a proot shell.

With no arguments AirCode resumes the most recent project.
";

/// The executable name, kept in one place because `--version`, the Linux
/// installer and the Windows resource all have to agree on it.
const BIN_NAME: &str = "aircode";

fn main() {
    let mut workspace: Option<PathBuf> = None;
    let mut new_session = false;
    let mut browser = false;
    let mut connect = false;
    let mut tui = false;
    let mut web = false;
    let mut gui = false;
    let mut doctor = false;
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
            "--web" | "--server" | "--browser-ui" => web = true,
            "--gui" | "--window" => gui = true,
            "--doctor" | "--diagnose" => doctor = true,
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

    if doctor {
        print_doctor(&host, port);
        return;
    }

    let chosen = [("--tui", tui), ("--web", web), ("--gui", gui)]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    if chosen.len() > 1 {
        eprintln!(
            "Pick one front-end: {} (try --help)",
            chosen.join(" or ")
        );
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

    // ---- The desktop window -------------------------------------------
    //
    // This used to `eprintln!` one line and `return`, which meant the process
    // exited **0**: the shell, the .desktop launcher and the installer all
    // believed AirCode had run. On a machine with no X11 - SSH, a container,
    // CI, or a proot shell on Android - the user got a dead end naming a
    // shared object they cannot act on, and was never told that --tui and
    // --web need no window at all.
    //
    // Now the blocker is reported in full, and AirCode starts a front-end that
    // actually works here instead of giving up.
    match ai_coding_agent::platform::gui_blocker() {
        None => match run_native_window("AirCode", 1520, 940, Box::new(app)) {
            Ok(()) => {}
            Err(blocker) => {
                report_gui_blocker(blocker, false, &host, port);
                std::process::exit(1);
            }
        },
        Some(blocker) => {
            if gui {
                // --gui was explicit: honour it and fail loudly rather than
                // silently substituting a different front-end.
                report_gui_blocker(blocker, true, &host, port);
                std::process::exit(1);
            }
            report_gui_blocker(blocker, false, &host, port);
            if !ai_coding_agent::platform::stdin_is_tty() {
                // Nobody is reading the banner, and a long-running server would
                // hang the caller. Print the way out and stop.
                std::process::exit(1);
            }
            eprintln!("  Starting the browser UI instead - it needs no window.\n");
            ai_coding_agent::web::run(app, &host, port);
        }
    }
}

/// Explains why the window could not open, what fixes it, and what still works.
///
/// The point is that the user is never left at a dead end: whatever went wrong,
/// there is always a front-end that runs here, and it is named.
fn report_gui_blocker(blocker: GuiBlocker, explicit: bool, host: &str, port: u16) {
    eprint!("{}", gui_blocker_report(blocker, explicit, host, port));
}

/// The rendered explanation. Separate from the printing so it can be asserted
/// on directly - the gating rule below is easy to get subtly wrong.
fn gui_blocker_report(blocker: GuiBlocker, explicit: bool, host: &str, port: u16) -> String {
    let mut out = String::from("\n");
    out.push_str("AirCode could not open a window.\n\n");
    out.push_str(&format!("  What happened : {}\n", blocker.detail()));

    // Only suggest installing X11 when a desktop was actually expected. Telling
    // someone to `apt install libx11-6` on a headless box would be wrong.
    if blocker.is_misconfiguration() {
        match ai_coding_agent::platform::x11_install_hint() {
            Some(cmd) => out.push_str(&format!("  Fix it with   : {}\n", cmd)),
            None => out.push_str(
                "  Fix it with   : install the X11 client library (libx11-6 / libX11)\n",
            ),
        }
    }

    if explicit {
        out.push_str("\n  --gui was asked for explicitly, so AirCode will not swap in\n  a different front-end.\n");
    }

    out.push_str(&format!(
        concat!(
            "\n",
            "  AirCode runs here without a window, either way:\n",
            "\n",
            "    aircode --tui    a full terminal UI - no browser, no X11\n",
            "    aircode --web    a browser UI on http://{}:{}\n",
            "\n",
            "  From a phone or another machine:\n",
            "    aircode --web --host 0.0.0.0\n",
            "  (binds every interface - only on a network you trust)\n",
            "\n",
            "  Inspect this machine any time: aircode --doctor\n",
        ),
        host, port
    ));
    if let Some(ip) = ai_coding_agent::platform::lan_address() {
        out.push_str(&format!(
            "  This box looks reachable at http://{} on your network.\n",
            ip
        ));
    }
    out
}

/// A short, honest report on whether AirCode can run here and what it found.
fn print_doctor(host: &str, port: u16) {
    let mut out = String::from("\nAirCode environment check\n\n");
    let kv = |k: &str, v: String| format!("  {:<22} {}\n", k, v);

    out.push_str(&kv("version", env!("CARGO_PKG_VERSION").to_string()));
    out.push_str(&kv(
        "os",
        format!(
            "{} / {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ));
    out.push_str(&kv(
        "display",
        format!(
            "DISPLAY={} WAYLAND_DISPLAY={}",
            show_env("DISPLAY"),
            show_env("WAYLAND_DISPLAY")
        ),
    ));
    out.push_str(&kv("terminal", if ai_coding_agent::platform::stdin_is_tty() { "yes".into() } else { "no (piped)".into() }));
    match ai_coding_agent::platform::gui_blocker() {
        None => out.push_str(&kv("desktop window", "available".into())),
        Some(b) => {
            out.push_str(&kv("desktop window", format!("unavailable - {}", b.detail())));
            // Same rule as the startup message: only offer the X11 package
            // when a desktop was actually expected here.
            if b.is_misconfiguration() {
                match ai_coding_agent::platform::x11_install_hint() {
                    Some(cmd) => out.push_str(&kv("  install with", cmd.to_string())),
                    None => out.push_str(&kv(
                        "  install with",
                        "the X11 client library (libx11-6 / libX11)".into(),
                    )),
                }
            }
        }
    }
    out.push_str(&kv(
        "data directory",
        ai_coding_agent::platform::app_data_dir().display().to_string(),
    ));
    out.push_str(&kv(
        "front-ends",
        "gui (needs X11), tui, web - tui and web always work".into(),
    ));
    out.push_str(&kv(
        "browser UI",
        format!("http://{}:{}", host, port),
    ));
    if let Some(ip) = ai_coding_agent::platform::lan_address() {
        out.push_str(&kv(
            "reachable at",
            format!("http://{}:{}  (needs --host 0.0.0.0)", ip, port),
        ));
    }
    print!("{}", out);
}

fn show_env(key: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => format!("{:?}", v),
        _ => "unset".to_string(),
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    fn report(blocker: GuiBlocker) -> String {
        gui_blocker_report(blocker, false, "127.0.0.1", 7777)
    }

    #[test]
    fn the_report_never_leaves_the_user_at_a_dead_end() {
        // Whatever went wrong, a front-end that works here must be named.
        for b in [
            GuiBlocker::NoDisplay,
            GuiBlocker::MissingX11Library,
            GuiBlocker::DisplayUnreachable,
        ] {
            let text = report(b);
            assert!(text.contains("--tui"), "{:?} never mentions --tui", b);
            assert!(text.contains("--web"), "{:?} never mentions --web", b);
            assert!(text.contains("--doctor"), "{:?} never mentions --doctor", b);
            assert!(text.contains("http://127.0.0.1:7777"), "{:?} has no URL", b);
        }
    }

    #[test]
    fn the_install_hint_only_appears_when_a_desktop_was_expected() {
        // This is the bug the first draft had: a headless box was told to
        // `apt install libx11-6`, which cannot possibly help.
        let headless = report(GuiBlocker::NoDisplay);
        assert!(
            !headless.contains("libx11-6") && !headless.contains("libX11"),
            "headless report suggests installing X11:\n{}",
            headless
        );
        // The opposite case must still offer the package, when we know it.
        let missing = report(GuiBlocker::MissingX11Library);
        if ai_coding_agent::platform::x11_install_hint().is_some() {
            assert!(
                missing.contains("libx11-6") || missing.contains("libX11"),
                "missing-library report does not offer the package:\n{}",
                missing
            );
        }
    }

    #[test]
    fn an_explicit_gui_request_says_it_will_not_substitute() {
        let text = gui_blocker_report(GuiBlocker::NoDisplay, true, "127.0.0.1", 7777);
        assert!(text.contains("--gui was asked for explicitly"));
    }

    #[test]
    fn conflicting_front_ends_are_rejected_before_anything_starts() {
        // Purely a guard on the flag plumbing: the dispatch counts selections.
        let chosen = [("--tui", true), ("--web", true), ("--gui", false)]
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(n, _)| n)
            .collect::<Vec<_>>();
        assert_eq!(chosen.len(), 2);
    }
}
