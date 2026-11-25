//! AI Coding Agent — 100% Pure-Rust Native Desktop IDE & Autonomous Coding Studio.
//! Zero WebView, zero HTML/CSS/JS, zero C++ host.
//! Native Win32 DIB/ClearType + ConPTY + WinHTTP on Windows; X11 + POSIX on Linux.

pub mod agent;
pub mod editor;
pub mod fs_ops;
pub mod git_ops;
pub mod json;
pub mod platform;
pub mod sqlite;
pub mod syntax;
pub mod ui;
pub mod vt_screen;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{build_side_by_side_diff, DiffLineKind, EditorTab};
    use crate::json::JsonValue;
    use crate::platform::DesktopAppHandler;
    use crate::sqlite::{
        make_unique_id, now_timestamp_label, ChatMessage, SqliteStore, ToolExecutionRecord,
    };
    use crate::syntax::{tokenize_line, LangFamily, TokenKind};
    use crate::vt_screen::VtScreen;

    #[test]
    fn test_json_roundtrip_and_escapes() {
        let raw = r#"{"provider":"ollama","iters":16,"flags":[true,false,null],"nested":{"msg":"hello\nworld \"rust\""}}"#;
        let parsed = JsonValue::parse(raw).expect("valid json");
        assert_eq!(parsed.string_or("provider", ""), "ollama");
        assert_eq!(parsed.i64_or("iters", 0), 16);
        let msg = parsed
            .get("nested")
            .map(|n| n.string_or("msg", ""))
            .unwrap_or_default();
        assert_eq!(msg, "hello\nworld \"rust\"");

        let serialized = parsed.to_json_string();
        let reparsed = JsonValue::parse(&serialized).expect("reparse");
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn test_sqlite_settings_sessions_and_audit() {
        let dir = std::env::temp_dir().join(make_unique_id("sqlite_test"));
        let store = SqliteStore::open(&dir).expect("open sqlite db");
        let mut settings = store.load_settings();
        settings.active_provider = "anthropic".to_string();
        settings.active_model = "claude-sonnet-4-20250514".to_string();
        settings.max_agent_iterations = 24;
        store.save_settings(&settings);

        let loaded = store.load_settings();
        assert_eq!(loaded.active_provider, "anthropic");
        assert_eq!(loaded.max_agent_iterations, 24);

        let sess = store.create_session("Pure Rust Refactor", "/tmp/project", "autonomous");
        store.append_message(&ChatMessage {
            id: make_unique_id("msg"),
            session_id: sess.id.clone(),
            role: "user".to_string(),
            content: "Add unit tests".to_string(),
            thinking: String::new(),
            mode: "autonomous".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            tools: Vec::new(),
            created_at: now_timestamp_label(),
        });

        store.record_tool_execution(&ToolExecutionRecord {
            id: make_unique_id("tool"),
            session_id: sess.id.clone(),
            tool_name: "run_tests".to_string(),
            args_json: "{}".to_string(),
            result_summary: "test result: ok".to_string(),
            status: "ok".to_string(),
            duration_ms: 42,
            diff_text: String::new(),
            file_path: String::new(),
            created_at: now_timestamp_label(),
        });

        let fetched = store.load_session(&sess.id).expect("session exists");
        assert_eq!(fetched.messages.len(), 1);
        assert_eq!(fetched.messages[0].content, "Add unit tests");

        let logs = store.list_tool_history(10);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].tool_name, "run_tests");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_editor_undo_redo_and_diff() {
        let mut tab = EditorTab::new_code(
            "main.rs".to_string(),
            "src/main.rs".to_string(),
            None,
            "fn main() {\n}",
            false,
        );
        tab.move_cursor(0, 11, false, 20);
        tab.insert_newline_auto_indent();
        tab.insert_str("    let x = 42;");
        assert!(tab.full_text().contains("let x = 42;"));
        tab.undo();
        tab.redo();
        assert!(tab.full_text().contains("let x = 42;"));

        let diff =
            build_side_by_side_diff("alpha\nbeta\ngamma\n", "alpha\nbeta_v2\ngamma\ndelta\n");
        assert!(diff.iter().any(|r| r.kind == DiffLineKind::Changed));
        assert!(diff.iter().any(|r| r.kind == DiffLineKind::Added));
    }

    #[test]
    fn test_syntax_rust_tokens() {
        let spans = tokenize_line(
            LangFamily::Rust,
            "pub fn compute(val: u64) -> String { // fast",
        );
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Function));
        assert!(spans.iter().any(|s| s.kind == TokenKind::TypeName));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn test_vt_screen_ansi_colors_and_cursor() {
        let mut vt = VtScreen::new(40, 10);
        vt.feed_bytes(b"\x1b[32mPASS\x1b[0m \x1b[38;2;255;128;0mWARN\x1b[0m\r\nNext");
        let row0 = vt.visible_row(0).expect("row0");
        assert_eq!(row0[0].ch, 'P');
        assert_eq!(row0[5].ch, 'W');
        assert_eq!(row0[5].fg, 0xFF8000);
        let row1 = vt.visible_row(1).expect("row1");
        assert_eq!(row1[0].ch, 'N');
    }

    #[test]
    fn test_workspace_sandbox_blocks_escape() {
        let tmp = std::env::temp_dir().join("aircode_sandbox_test");
        let _ = std::fs::create_dir_all(&tmp);
        let ok_res = fs_ops::resolve_workspace_path(&tmp, "src/new_mod.rs", true);
        assert!(ok_res.is_ok());

        let escape_res = fs_ops::resolve_workspace_path(&tmp, "../../etc/passwd", true);
        assert!(escape_res.is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_headless_studio_render_frame() {
        let cwd = std::env::current_dir().unwrap();
        let mut app = ui::studio::StudioApp::new_with_options(Some(cwd), false);
        let mut surface = platform::linux::SoftwareSurface::new(1480, 920);
        app.render(&mut surface);
        assert!(surface.pixels.iter().any(|&px| px != 0));
    }
}
