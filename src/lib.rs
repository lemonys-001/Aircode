//! AirCode — 100% Pure-Rust Native Desktop IDE, Autonomous Coding Agent & Agentic Browser.
//! Zero WebView application UI, zero HTML/CSS/JS UI bundle, zero C++ host.
//! Native Win32 DIB/ClearType + ConPTY + WinHTTP on Windows; X11 + POSIX on Linux DE.

pub mod agent;
pub mod browser;
pub mod editor;
pub mod fs_ops;
pub mod git_ops;
pub mod json;
pub mod mcp;
pub mod opencode_auth;
pub mod platform;
pub mod skills;
pub mod sqlite;
pub mod subagents;
pub mod syntax;
pub mod themes;
pub mod ui;
pub mod vt_screen;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::SharedBrowser;
    use crate::editor::{build_side_by_side_diff, DiffLineKind, EditorTab};
    use crate::json::JsonValue;
    use crate::platform::{DesktopAppHandler, FontRole, SurfaceCanvas};
    use crate::skills::load_all_skills;
    use crate::sqlite::{
        make_unique_id, now_timestamp_label, ChatMessage, SqliteStore, ToolExecutionRecord,
    };
    use crate::subagents::{load_all_subagents, save_subagent, SubAgentSpec};
    use crate::syntax::{tokenize_line, LangFamily, TokenKind};
    use crate::vt_screen::VtScreen;
    use std::collections::HashSet;

    #[test]
    fn test_json_roundtrip_and_escapes() {
        let raw = r#"{"provider":"opencode","iters":16,"flags":[true,false,null],"nested":{"msg":"hello\nworld \"rust\""}}"#;
        let parsed = JsonValue::parse(raw).expect("valid json");
        assert_eq!(parsed.string_or("provider", ""), "opencode");
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
    fn test_sqlite_settings_sessions_fork_and_compaction() {
        let dir = std::env::temp_dir().join(make_unique_id("sqlite_test"));
        let store = SqliteStore::open(&dir).expect("open sqlite db");
        let mut settings = store.load_settings();
        settings.active_provider = "opencode".to_string();
        settings.active_model = "big-pickle".to_string();
        settings.agent_effort = "max".to_string();
        settings.theme = "catppuccin-mocha".to_string();
        settings.show_thinking = false;
        store.save_settings(&settings);

        let loaded = store.load_settings();
        assert_eq!(loaded.active_provider, "opencode");
        assert_eq!(loaded.agent_effort, "max");
        assert_eq!(loaded.theme, "catppuccin-mocha");
        assert!(!loaded.show_thinking);

        let sess = store.create_session("Pure Rust Refactor", "/tmp/project", "build");
        for idx in 0..4 {
            store.append_message(&ChatMessage {
                id: make_unique_id("msg"),
                session_id: sess.id.clone(),
                role: if idx % 2 == 0 { "user".into() } else { "assistant".into() },
                content: format!("Message #{}", idx + 1),
                thinking: "Step-by-step analysis".to_string(),
                mode: "build".to_string(),
                provider: "opencode".to_string(),
                model: "big-pickle".to_string(),
                tools: vec![ToolExecutionRecord {
                    id: make_unique_id("tool"),
                    session_id: sess.id.clone(),
                    tool_name: "edit_file".to_string(),
                    args_json: r#"{"path":"src/main.rs"}"#.to_string(),
                    result_summary: "Edited src/main.rs".to_string(),
                    status: "ok".to_string(),
                    duration_ms: 14,
                    diff_text: "@@ -1 +1 @@\n-old\n+new".to_string(),
                    file_path: "src/main.rs".to_string(),
                    created_at: now_timestamp_label(),
                }],
                created_at: now_timestamp_label(),
            });
        }

        let forked = store.fork_session(&sess.id, Some(1)).expect("fork session");
        assert_eq!(forked.messages.len(), 2);
        assert_eq!(forked.parent_id, sess.id);

        let compacted = store
            .compact_session(&sess.id, "- Completed refactor")
            .expect("compact session");
        assert_eq!(compacted.compacted_count, 1);
        assert_eq!(compacted.messages.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_themes_skills_subagents_and_agentic_browser() {
        let dir = std::env::temp_dir().join(make_unique_id("feat_test"));
        std::fs::create_dir_all(dir.join(".aircode")).unwrap();
        std::fs::write(
            dir.join(".aircode/theme.json"),
            r##"{"name":"Neon Mint","base":"tokyo-night","accent":"#00F5D4"}"##,
        )
        .unwrap();
        let pal = crate::themes::ThemePalette::by_id("custom", Some(&dir));
        assert_eq!(pal.accent, 0x00F5D4);

        let skills = load_all_skills(&dir, &dir, &HashSet::new());
        assert!(skills.iter().any(|s| s.slug == "aircode-customizer"));

        save_subagent(
            &dir,
            &SubAgentSpec {
                slug: "perf-auditor".into(),
                title: "Performance Auditor".into(),
                effort: "ultra".into(),
                readonly: true,
                system_prompt: "Audit allocations.".into(),
                builtin: false,
            },
        )
        .unwrap();
        let subs = load_all_subagents(&dir);
        assert!(subs.iter().any(|s| s.slug == "perf-auditor"));

        let browser = SharedBrowser::new();
        let snap = browser.snapshot(20);
        assert!(snap.contains("Agentic Browser for Builders"));
        assert!(snap.contains("[e1]"));

        browser.interrupt_for_user();
        assert!(browser.is_user_takeover());
        assert!(browser.navigate("aircode://opencode-auth", true).is_err());
        browser.hand_back_to_ai();
        assert!(browser.navigate("aircode://opencode-auth", true).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_editor_smart_pairs_comments_and_brackets() {
        let mut tab = EditorTab::new_code(
            "demo.rs".into(),
            "demo.rs".into(),
            None,
            "",
            false,
        );
        tab.insert_smart_char('f');
        tab.insert_smart_char('n');
        tab.insert_smart_char(' ');
        tab.insert_smart_char('m');
        tab.insert_smart_char('(');
        // '(' auto-inserted ')'
        assert_eq!(tab.full_text(), "fn m()");
        tab.insert_smart_char(')'); // skips over ')'
        tab.insert_smart_char('{');
        assert_eq!(tab.full_text(), "fn m(){}");
        assert!(tab.matching_bracket().is_some());
        tab.toggle_comment();
        assert!(tab.full_text().starts_with("// "));
        tab.toggle_comment();
        assert_eq!(tab.full_text(), "fn m(){}");

        let diff = build_side_by_side_diff("a\nb\n", "a\nc\n");
        assert!(diff.iter().any(|r| r.kind != DiffLineKind::Unchanged));
    }

    #[test]
    fn test_syntax_and_vt_screen() {
        let toks = tokenize_line(LangFamily::Rust, "pub fn main() -> i32 { 42 }");
        assert!(toks.iter().any(|t| t.kind == TokenKind::Keyword));
        assert!(toks.iter().any(|t| t.kind == TokenKind::NumberLit));

        let mut vt = VtScreen::new(10, 40);
        vt.feed_bytes(b"\x1b[32mPASS\x1b[0m\r\ndone");
        assert_eq!(vt.grid[0][0].ch, 'P');
        assert_eq!(vt.grid[1][0].ch, 'd');
    }

    #[test]
    fn test_studio_headless_command_palette_and_slash_commands() {
        let dir = std::env::temp_dir().join(make_unique_id("studio_test"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let mut app = ui::studio::StudioApp::new_headless_for_test(store, dir.clone());
        app.execute_slash_command("/thinking off");
        assert!(!app.settings.show_thinking);
        app.execute_slash_command("/thinking on");
        assert!(app.settings.show_thinking);
        app.execute_slash_command("/effort ultra");
        assert_eq!(app.settings.agent_effort, "ultra");
        app.execute_slash_command("/plan");
        assert_eq!(app.active_mode, "plan");
        app.execute_slash_command("/build");
        assert_eq!(app.active_mode, "build");
        app.execute_slash_command("/theme rose-pine");
        assert_eq!(app.settings.theme, "rose-pine");
        // The quiet daylight theme is the shipped default.
        app.execute_slash_command("/theme low-cortisol");
        assert_eq!(app.settings.theme, "low-cortisol");
        assert!(app.palette.is_light);

        // OpenCode Zen free lane: one slash command, no key anywhere.
        app.execute_slash_command("/zen");
        assert_eq!(app.settings.active_provider, "opencode");
        assert_eq!(app.settings.opencode_auth_mode, "zen");
        assert!(app.settings.opencode_api_key.is_empty());
        assert!(app
            .settings
            .active_model
            .ends_with("-free")
            .then_some(true)
            .unwrap_or_else(|| app.settings.active_model == "big-pickle"));

        let mut surf = TestSurface::new(1440, 900);
        app.render(&mut surf);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_opencode_zen_client_emulation() {
        use crate::opencode_auth::{
            is_zen_free_model, new_zen_session_id, zen_client_headers, zen_tool_name_in,
            zen_tool_name_out, ZEN_CLIENT_USER_AGENT, ZEN_PUBLIC_KEY,
        };

        // Session ids must match the shape the gateway fingerprints:
        //   ^ses_[0-9a-f]{12}[0-9A-Za-z]{14}$  with the 12 hex being ^[0-9a-f]{9}f[c-f][0-9a-f]$
        for _ in 0..64 {
            let sid = new_zen_session_id();
            assert!(sid.starts_with("ses_"), "bad prefix: {}", sid);
            let rest = &sid[4..];
            assert_eq!(rest.len(), 26, "bad length: {}", sid);
            let (hex12, tail) = rest.split_at(12);
            assert!(
                hex12.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "non-lowercase-hex: {}",
                sid
            );
            assert_eq!(&hex12[9..10], "f", "10th hex char must be 'f': {}", sid);
            assert!(
                "cdef".contains(&hex12[10..11]),
                "11th hex char must be c/d/e/f: {}",
                sid
            );
            assert!(
                tail.chars().all(|c| c.is_ascii_alphanumeric()),
                "tail must be alphanumeric: {}",
                sid
            );
        }
        // Two calls must not collide.
        assert_ne!(new_zen_session_id(), new_zen_session_id());

        // The header quartet the gateway requires - and nothing it rejects.
        let sid = new_zen_session_id();
        let hdrs = zen_client_headers(&sid, ZEN_PUBLIC_KEY);
        let keys: Vec<&str> = hdrs.iter().map(|(k, _)| *k).collect();
        for required in [
            "Authorization",
            "x-api-key",
            "User-Agent",
            "x-opencode-client",
            "x-opencode-project",
            "x-opencode-session",
            "x-opencode-session-id",
        ] {
            assert!(keys.contains(&required), "missing header {}", required);
        }
        assert!(
            !keys.contains(&"x-opencode-request"),
            "x-opencode-request makes the gateway answer 403"
        );
        assert_eq!(
            hdrs.iter().find(|(k, _)| *k == "User-Agent").unwrap().1,
            ZEN_CLIENT_USER_AGENT
        );
        assert_eq!(
            hdrs.iter().find(|(k, _)| *k == "x-opencode-session").unwrap().1,
            sid
        );
        assert_eq!(
            hdrs.iter().find(|(k, _)| *k == "x-opencode-session-id").unwrap().1,
            sid
        );

        // Free-lane tool remap round-trips.
        assert_eq!(zen_tool_name_out("execute_command"), "bash");
        assert_eq!(zen_tool_name_out("read_file"), "read");
        assert_eq!(zen_tool_name_in("bash"), "execute_command");
        assert_eq!(zen_tool_name_in("read"), "read_file");
        for name in ["execute_command", "read_file", "write_file", "search_project"] {
            assert_eq!(zen_tool_name_in(zen_tool_name_out(name)), name);
        }

        assert!(is_zen_free_model("big-pickle"));
        assert!(is_zen_free_model("deepseek-v4.1-flash-free"));
        assert!(!is_zen_free_model("claude-sonnet-4-5"));
    }

    #[test]
    fn test_opencode_zen_sse_and_tool_remap() {
        // A Zen-shaped SSE body: reasoning delta, two content deltas, and one tool
        // call split across two chunks (name first, arguments second).
        let body = r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"think "}}]}

data: {"choices":[{"index":0,"delta":{"content":"Hello "}}]}

data: {"choices":[{"index":0,"delta":{"content":"world"}}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"bash"}}]}}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\": \"ls\"}"}}]}}]}

data: [DONE]

"#;
        let (text, reasoning, calls) = crate::agent::parse_openai_sse(body);
        assert_eq!(text, "Hello world");
        assert_eq!(reasoning, "think ");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call_1");
        assert_eq!(calls[0].1, "bash");
        assert_eq!(calls[0].2.string_or("cmd", ""), "ls");

        // A plain (non-streamed) JSON body must still parse.
        let json_body = r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi","tool_calls":[{"id":"c2","type":"function","function":{"name":"read","arguments":"{\"path\":\"a.rs\"}"}}]}}]}"#;
        let (t2, r2, c2) = crate::agent::parse_openai_sse(json_body);
        assert_eq!(t2, "hi");
        assert_eq!(r2, "");
        assert_eq!(c2.len(), 1);
        assert_eq!(c2[0].1, "read");
        assert_eq!(c2[0].2.string_or("path", ""), "a.rs");

        // Tool schemas get remapped onto the free-lane names; unknown ones are dropped.
        let tools = crate::agent::tool_schemas_openai();
        let remapped = crate::agent::remap_tools_for_zen(&tools);
        let names: Vec<String> = remapped
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("function").map(|f| f.string_or("name", "")))
            .collect();
        assert!(names.contains(&"bash".to_string()), "{:?}", names);
        assert!(names.contains(&"read".to_string()), "{:?}", names);
        assert!(!names.contains(&"execute_command".to_string()));
    }

    /// Minimal in-memory software canvas used by headless UI smoke tests (all platforms).
    struct TestSurface {
        width: i32,
        height: i32,
        pixels: Vec<u32>,
        text_calls: usize,
    }

    impl TestSurface {
        fn new(w: i32, h: i32) -> Self {
            Self {
                width: w,
                height: h,
                pixels: vec![0; (w * h) as usize],
                text_calls: 0,
            }
        }
    }

    impl platform::SurfaceCanvas for TestSurface {
        fn width(&self) -> i32 {
            self.width
        }
        fn height(&self) -> i32 {
            self.height
        }
        fn clear(&mut self, rgb: u32) {
            self.pixels.fill(rgb);
        }
        fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
            for py in y.max(0)..(y + h).min(self.height) {
                for px in x.max(0)..(x + w).min(self.width) {
                    self.pixels[(py * self.width + px) as usize] = rgb;
                }
            }
        }
        fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, _r: i32, rgb: u32) {
            self.fill_rect(x, y, w, h, rgb);
        }
        fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
            self.fill_rect(x, y, w, 1, rgb);
            self.fill_rect(x, y + h - 1, w, 1, rgb);
            self.fill_rect(x, y, 1, h, rgb);
            self.fill_rect(x + w - 1, y, 1, h, rgb);
        }
        fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, _r: i32, rgb: u32) {
            self.stroke_rect(x, y, w, h, rgb);
        }
        fn draw_hline(&mut self, x: i32, y: i32, w: i32, rgb: u32) {
            self.fill_rect(x, y, w, 1, rgb);
        }
        fn draw_vline(&mut self, x: i32, y: i32, h: i32, rgb: u32) {
            self.fill_rect(x, y, 1, h, rgb);
        }
        fn draw_text(&mut self, x: i32, y: i32, text: &str, font: FontRole, rgb: u32) {
            self.text_calls += 1;
            let _ = (x, y, text, font, rgb);
        }
        fn draw_text_clipped(&mut self, x: i32, y: i32, max_w: i32, text: &str, font: FontRole, rgb: u32) {
            self.text_calls += 1;
            let _ = (x, y, max_w, text, font, rgb);
        }
        fn measure_text_width(&mut self, text: &str, font: FontRole) -> i32 {
            let per = match font {
                FontRole::CodeMono | FontRole::TerminalMono => 8,
                _ => 7,
            };
            text.chars().count() as i32 * per
        }
        fn mono_cell_size(&mut self, font: FontRole) -> (i32, i32) {
            match font {
                FontRole::TerminalMono => (7, 15),
                _ => (8, 16),
            }
        }
        fn push_clip(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}
        fn pop_clip(&mut self) {}
    }
}
