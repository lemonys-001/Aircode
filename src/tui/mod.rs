//! The terminal front-end.
//!
//! `aircode --tui` runs the same `StudioApp` the desktop GUI runs, but
//! drives it through the shared [`crate::ui::snapshot::AppSnapshot`] instead of
//! a canvas. Every feature the GUI has — the composer with slash autocomplete
//! and history, `/connect`, the command palette, streaming turns, permission
//! prompts — is reached by forwarding the decoded keys to the same
//! `handle_key_down` / `handle_char_input` entry points, so there is no second
//! implementation of the app to keep in sync.

mod screen;
pub mod term;
mod view;

use std::io::Write;
use std::sync::mpsc::RecvTimeoutError;

use crate::platform::KeyCode;
use crate::tui::screen::{render_diff, Screen};
use crate::tui::term::{Key, KeyDecoder};
use crate::tui::view::{draw, ViewState};
use crate::ui::snapshot::AppSnapshot;
use crate::ui::studio::StudioApp;

/// Runs the TUI until the user quits. Consumes `app`.
pub fn run(mut app: StudioApp) {
    let mut terminal = term::Terminal::new();
    let mut decoder = KeyDecoder::new();
    let rx = term::spawn_input_reader();
    let mut state = ViewState {
        scroll: 0,
        follow: true,
        user_scrolled: false,
    };

    let mut screen = Screen::new(terminal.size().0, terminal.size().1);
    let mut prev = Screen::new(screen.cols, screen.rows);

    if !terminal.raw() {
        // Cooked mode: we can still draw and read, just one line at a time.
        let _ = writeln!(
            std::io::stderr(),
            "[AirCode] no raw-mode terminal available; input arrives one line at a time."
        );
    }

    loop {
        terminal.refresh_size();
        screen.resize(terminal.size().0, terminal.size().1);

        // 1. Pump the agent stream, the file watcher and the terminal PTYs.
        app.tick();

        // 2. Draw.
        let snap = app.snapshot();
        draw(&mut screen, &snap, &state);
        let mut frame = String::new();
        render_diff(&mut frame, &prev, &screen);
        // Park the caret at the end of the prompt line.
        frame.push_str(&format!("\x1b[{};{}H\x1b[?25h", screen.rows, screen.cols.min(200)));
        let _ = terminal.present(&frame);
        std::mem::swap(&mut prev, &mut screen);

        // 3. Fast poll while streaming so the transcript keeps moving; near-zero
        //    CPU when idle.
        let timeout = if snap.agent_running {
            term::BUSY_TICK
        } else {
            term::IDLE_TICK
        };

        // 4. Wait for input, or wake up to redraw a streaming turn.
        match rx.recv_timeout(timeout) {
            Ok(bytes) => {
                for key in decoder.feed(&bytes) {
                    if quit(&key) {
                        terminal.teardown();
                        return;
                    }
                    handle_key(&mut app, &mut state, key, &snap);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // Input went quiet: a lone ESC is a real Escape press now.
                if let Some(key) = decoder.flush_pending_escape() {
                    if quit(&key) {
                        terminal.teardown();
                        return;
                    }
                    handle_key(&mut app, &mut state, key, &snap);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // stdin closed (e.g. piped input finished).
                terminal.teardown();
                return;
            }
        }

        // 5. Drain anything else that arrived while we were drawing.
        while let Ok(bytes) = rx.try_recv() {
            for key in decoder.feed(&bytes) {
                if quit(&key) {
                    terminal.teardown();
                    return;
                }
                handle_key(&mut app, &mut state, key, &snap);
            }
        }
    }
}

fn quit(key: &Key) -> bool {
    matches!(key, Key::Ctrl('c') | Key::Ctrl('d'))
}

/// Routes one key. Anything the app's own composer handles is forwarded; the
/// TUI only claims the handful of keys that are terminal-specific.
fn handle_key(
    app: &mut StudioApp,
    state: &mut ViewState,
    key: Key,
    snap: &AppSnapshot,
) {
    match key {
        // Scrolling is a view concern, not an app one.
        Key::Named(KeyCode::PageUp) => {
            state.page_up(10);
            return;
        }
        Key::Named(KeyCode::PageDown) => {
            state.page_down(10, usize::MAX);
            return;
        }
        // F1 / ctrl+p open the palette without depending on the terminal
        // forwarding a chord we did not ask for.
        Key::Named(KeyCode::Other(0x70)) | Key::Ctrl('p') => {
            app.open_command_palette();
            return;
        }
        Key::Ctrl('o') => {
            app.show_connect();
            return;
        }
        _ => {}
    }

    // The connect screen owns the keyboard while it is open, as in the GUI.
    if snap.connect.open {
        if let Key::Named(KeyCode::Escape) = key {
            app.close_connect_wizard();
            return;
        }
    }

    if let Some(k) = key.as_key_code() {
        app.handle_key_down(k, key.modifiers());
    } else if let Key::Char(c) = key {
        app.handle_char_input(c);
    }
}

/// A short banner printed before the alternate screen takes over, so `--tui`
/// never looks like it hung.
pub fn print_banner(snap: &AppSnapshot) {
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "AirCode {} — terminal UI · {} · model {} · ctrl+c to quit",
        snap.version,
        snap.workspace,
        if snap.model.is_empty() {
            "unset"
        } else {
            &snap.model
        }
    );
    let _ = writeln!(
        err,
        "Type /connect to set up a provider, or just ask a question."
    );
}

// ---------------------------------------------------------------------------
// Test hooks
// ---------------------------------------------------------------------------

/// Decodes a byte slice the way the reader thread would.
pub fn decode_for_test(bytes: &[u8]) -> Vec<Key> {
    let mut d = KeyDecoder::new();
    d.feed(bytes)
}

/// The scroll helpers the main loop uses, exercised directly by tests.
pub fn scroll_by(state: &mut ViewState, lines: i32, max: usize) {
    if lines < 0 {
        state.scroll_up((-lines) as usize);
    } else {
        state.scroll_down(lines as usize, max);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteStore;
    use crate::tui::screen::{render_diff, Screen};

    #[test]
    fn ctrl_chords_are_forwarded_to_the_app() {
        // The app's key table matches uppercase letters (that is how the GUI
        // receives them from the OS). A terminal reports ctrl chords lowercase,
        // so `as_key_code` has to uppercase them or ctrl+L / ctrl+V / ctrl+H
        // are dropped here and never reach the composer at all.
        assert_eq!(Key::Ctrl('l').as_key_code(), Some(KeyCode::Letter('L')));
        assert_eq!(Key::Ctrl('v').as_key_code(), Some(KeyCode::Letter('V')));
        assert_eq!(Key::Ctrl('h').as_key_code(), Some(KeyCode::Letter('H')));
        assert_eq!(Key::Ctrl('a').as_key_code(), Some(KeyCode::Letter('A')));
        assert_eq!(Key::Char('a').as_key_code(), None);
        assert_eq!(Key::Named(KeyCode::F3).as_key_code(), Some(KeyCode::F3));
    }

    #[test]
    fn test_key_decoding_covers_the_composer() {
        let k = decode_for_test(b"hi\r");
        assert_eq!(k, vec![Key::Char('h'), Key::Char('i'), Key::Named(KeyCode::Enter)]);
    }

    #[test]
    fn test_key_decoding_arrows_and_editing() {
        // CSI sequences the terminal actually sends.
        let k = decode_for_test(b"\x1b[A\x1b[B\x1b[C\x1b[D\x7f\t");
        assert_eq!(
            k,
            vec![
                Key::Named(KeyCode::Up),
                Key::Named(KeyCode::Down),
                Key::Named(KeyCode::Right),
                Key::Named(KeyCode::Left),
                Key::Named(KeyCode::Backspace),
                Key::Named(KeyCode::Tab),
            ]
        );
    }

    #[test]
    fn test_key_decoding_navigation_keys() {
        let k = decode_for_test(b"\x1b[H\x1b[F\x1b[5~\x1b[6~\x1b[3~");
        assert_eq!(
            k,
            vec![
                Key::Named(KeyCode::Home),
                Key::Named(KeyCode::End),
                Key::Named(KeyCode::PageUp),
                Key::Named(KeyCode::PageDown),
                Key::Named(KeyCode::Delete),
            ]
        );
    }

    #[test]
    fn test_key_decoding_control_and_escape() {
        // A trailing ESC is deliberately held back: it may be the first byte of
        // an arrow sequence whose remainder has not arrived yet.
        let mut d = KeyDecoder::new();
        let k = d.feed(b"\x01\x0f\x1b");
        assert_eq!(k, vec![Key::Ctrl('a'), Key::Ctrl('o')]);
        assert!(d.has_pending(), "lone ESC should be pending");

        // Once input goes quiet the main loop releases it as a real Escape.
        assert_eq!(d.flush_pending_escape(), Some(Key::Named(KeyCode::Escape)));
        assert!(!d.has_pending());
        // …and a second flush must not invent another one.
        assert_eq!(d.flush_pending_escape(), None);
    }

    #[test]
    fn test_escape_split_across_reads_is_not_lost() {
        // A read boundary in the middle of a CSI sequence must not produce a
        // bogus Escape — the decoder has to wait for the rest.
        let mut d = KeyDecoder::new();
        assert!(d.feed(b"\x1b[").is_empty());
        let k = d.feed(b"A");
        assert_eq!(k, vec![Key::Named(KeyCode::Up)]);
    }

    #[test]
    fn test_utf8_is_decoded_whole() {
        let k = decode_for_test("héllo→".as_bytes());
        assert_eq!(
            k,
            vec![
                Key::Char('h'),
                Key::Char('é'),
                Key::Char('l'),
                Key::Char('l'),
                Key::Char('o'),
                Key::Char('→'),
            ]
        );
    }

    #[test]
    fn test_ss3_arrows_are_understood() {
        let k = decode_for_test(b"\x1bOA\x1bOB");
        assert_eq!(
            k,
            vec![Key::Named(KeyCode::Up), Key::Named(KeyCode::Down)]
        );
    }

    #[test]
    fn test_screen_diff_emits_nothing_when_unchanged() {
        let mut a = Screen::new(20, 4);
        let mut b = Screen::new(20, 4);
        a.text(0, 0, "hello", 0xffffff, 0x000000, false);
        b.text(0, 0, "hello", 0xffffff, 0x000000, false);
        let mut out = String::new();
        render_diff(&mut out, &a, &b);
        assert!(out.is_empty(), "unchanged frame should emit nothing");
    }

    #[test]
    fn test_screen_diff_emits_a_cursor_move_and_colour() {
        let a = Screen::new(20, 4);
        let mut b = Screen::new(20, 4);
        b.text(2, 1, "hi", 0xff0000, 0x00ff00, true);
        let mut out = String::new();
        render_diff(&mut out, &a, &b);
        // 1-based row/col, a truecolour SGR, then the text.
        assert!(out.contains("\x1b[2;3H"), "missing cursor move: {:?}", out);
        assert!(out.contains("38;2;255;0;0"), "missing fg: {:?}", out);
        assert!(out.contains("48;2;0;255;0"), "missing bg: {:?}", out);
        assert!(out.contains("hi"));
    }

    #[test]
    fn test_screen_diff_clears_on_resize() {
        let a = Screen::new(20, 4);
        let b = Screen::new(30, 8);
        let mut out = String::new();
        render_diff(&mut out, &a, &b);
        assert!(out.contains("\x1b[2J"));
    }

    #[test]
    fn test_view_state_scroll_clamps_and_follows() {
        let mut st = ViewState::default();
        st.scroll_up(5);
        assert_eq!(st.scroll, 0);
        assert!(st.user_scrolled);

        st.scroll_down(3, 10);
        assert_eq!(st.scroll, 3);
        st.scroll_down(99, 10);
        assert_eq!(st.scroll, 10);
        // Reaching the bottom re-enables follow.
        assert!(!st.user_scrolled);
    }

    #[test]
    fn test_scroll_by_helper() {
        let mut st = ViewState::default();
        scroll_by(&mut st, -4, 100);
        assert_eq!(st.scroll, 0);
        scroll_by(&mut st, 7, 100);
        assert_eq!(st.scroll, 7);
    }

    /// The whole point of the TUI: keys reach the real app.
    #[test]
    fn test_keys_drive_the_real_composer() {
        let dir = std::env::temp_dir().join(make_unique_id("tui_keys"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let mut app = crate::ui::studio::StudioApp::new_headless_for_test(store, dir.clone());
        app.tick();

        for key in decode_for_test(b"/connect") {
            match key {
                Key::Char(c) => app.handle_char_input(c),
                other => {
                    if let Some(k) = other.as_key_code() {
                        app.handle_key_down(k, other.modifiers());
                    }
                }
            }
        }
        assert_eq!(app.prompt_text(), "/connect");
        // Tab should accept the slash completion.
        app.handle_key_down(KeyCode::Tab, Default::default());
        assert!(
            app.prompt_text().starts_with("/connect"),
            "tab did not complete: {:?}",
            app.prompt_text()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tui_connect_screen_opens_and_closes() {
        let dir = std::env::temp_dir().join(make_unique_id("tui_connect"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let mut app = crate::ui::studio::StudioApp::new_headless_for_test(store, dir.clone());

        // ctrl+o is the TUI's connect shortcut.
        app.show_connect();
        assert_eq!(
            app.shell_view(),
            crate::ui::shell::ShellView::Connect,
            "connect screen did not open"
        );
        assert!(app.snapshot().connect.open);

        // Escape must return to the conversation, not the IDE...
        app.close_connect_wizard();
        assert_eq!(app.shell_view(), crate::ui::shell::ShellView::Chat);
        assert!(!app.snapshot().connect.open);
        // ...and hand the keyboard back. The old code drove this with
        // `ToggleShellView`, which changed the view but left focus stranded on
        // the connect key field: after closing /connect nothing you typed
        // reached the composer ever again.
        app.handle_char_input('h');
        app.handle_char_input('i');
        assert_eq!(app.prompt_text(), "hi");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Saving the wizard must not strand focus either.
    #[test]
    fn test_saving_the_connect_wizard_keeps_the_composer_usable() {
        let dir = std::env::temp_dir().join(make_unique_id("tui_connect_save"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let mut app = crate::ui::studio::StudioApp::new_headless_for_test(store, dir.clone());

        app.show_connect();
        app.execute_action(crate::ui::UiAction::SaveConnectWizard);
        assert_eq!(app.shell_view(), crate::ui::shell::ShellView::Chat);
        assert!(!app.snapshot().connect.open);

        app.handle_char_input('o');
        app.handle_char_input('k');
        assert_eq!(app.prompt_text(), "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ctrl+L clears the composer, in the TUI and in the web UI alike.
    #[test]
    fn test_ctrl_l_clears_the_composer() {
        let dir = std::env::temp_dir().join(make_unique_id("tui_ctrl_l"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let mut app = crate::ui::studio::StudioApp::new_headless_for_test(store, dir.clone());
        for c in ['h', 'e', 'l', 'l', 'o'] {
            app.handle_char_input(c);
        }
        assert_eq!(app.prompt_text(), "hello");
        // The TUI reports ctrl chords lowercase; the app matches uppercase.
        let k = Key::Ctrl('l').as_key_code().expect("ctrl+l must map to a key");
        app.handle_key_down(k, Key::Ctrl('l').modifiers());
        assert_eq!(app.prompt_text(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_unique_id(tag: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "aircode_{}_{}_{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }
}
