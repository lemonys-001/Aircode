//! The OpenCode-style shell.
//!
//! Layout (mirrors opencode's TUI / desktop app):
//!
//! ```text
//! +--------------------------------------------------------------+----------+
//! | [session-1] [session-2] [+]                                  |  (tabs)  |
//! +--------------------------------------------------------------+----------+
//! |                                                              | CONTEXT  |
//! |  Model (12:04)                                              | tokens   |
//! |  > user prompt                                               | %  cost  |
//! |                                                              |          |
//! |  assistant markdown                                          | MCP      |
//! |  + Thought: 16s                                              | LSP      |
//! |  + Read src/greet.py                                         | TODOS    |
//! |  + added / - removed (green/red)                             | MODIFIED |
//! |                                                              |          |
//! |  Done.                                                       |          |
//! |  Build - Model - Provider - effort                           |          |
//! |  ----------------------------------------------------------  |          |
//! |  enter to send | shift+enter newline | ctrl+h tools          |          |
//! |  > _                                                  [Send] |          |
//! +--------------------------------------------------------------+----------+
//! | [ctrl+? help] Tokens: 16.4K (8%) Cost: $0.12  [Claude 3.7 v] |          |
//! +--------------------------------------------------------------+----------+
//! ```
//!
//! `Workspace` mode reuses the existing IDE panels (explorer, editor, terminal,
//! git, browser) behind the same tab strip, so nothing that used to work was
//! thrown away — the chat view is simply the new front door.

pub mod chat;
pub mod connect;
pub mod markdown;
pub mod palette;
pub mod prompt;
pub mod sidebar;
pub mod theme;

pub use theme::{
    alpha_blend, card, clamp_scroll, darken, disclosure, dot, fmt_cost, fmt_tokens, grid_text,
    lighten, mix, panel, pill, scrollbar, section_label, shorten_path, truncate_left,
    truncate_right, wrap_text, CARD_RADIUS, ROW_H, SHELL_PAD, SIDEBAR_MIN_W, SIDEBAR_W,
    STATUSBAR_H, TABSTRIP_H,
};

use crate::platform::SurfaceCanvas;
use crate::ui::UiAction;

/// The two top-level surfaces of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellView {
    /// OpenCode-style conversation (the default front door).
    Chat,
    /// The full IDE: explorer, editor, terminal, git, browser.
    Workspace,
    /// Provider setup screen.
    Connect,
}

impl Default for ShellView {
    fn default() -> Self {
        ShellView::Chat
    }
}

/// Sidebar sections that can be collapsed, in display order.
pub const SIDEBAR_SECTIONS: &[&str] = &["context", "mcp", "lsp", "todos", "files"];

/// Transient, one-line status message shown in the status bar.
#[derive(Clone, Debug, Default)]
pub struct ShellFlash {
    pub text: String,
    pub ticks: u32,
}

impl ShellFlash {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ticks: 240,
        }
    }
    pub fn tick(&mut self) {
        self.ticks = self.ticks.saturating_sub(1);
    }
    pub fn visible(&self) -> bool {
        self.ticks > 0 && !self.text.is_empty()
    }
}

/// A session tab in the top strip.
#[derive(Clone, Debug)]
pub struct SessionTab {
    pub id: String,
    pub title: String,
    pub active: bool,
}

/// Everything the shell draws, resolved once per frame by `StudioApp`.
pub struct ShellFrame<'a> {
    pub width: i32,
    pub height: i32,
    pub palette: &'a crate::themes::ThemePalette,
    pub view: ShellView,
    pub tabs: &'a [SessionTab],
    pub active_tab: usize,

    // chat
    pub chat: chat::ChatModel<'a>,
    pub chat_scroll: usize,

    // sidebar
    pub sidebar: Option<sidebar::SidebarModel<'a>>,

    // prompt
    pub prompt: prompt::PromptModel<'a>,

    // status bar
    pub model_label: &'a str,
    pub provider_label: &'a str,
    pub effort_label: &'a str,
    pub tokens: u64,
    pub token_pct: f32,
    pub cost_usd: f64,
    pub running: bool,
    pub flash: Option<&'a ShellFlash>,
    pub workspace_path: &'a str,
    pub version: &'a str,

    // overlays
    pub command_palette: Option<palette::PaletteModel<'a>>,
    pub connect: Option<connect::ConnectModel<'a>>,
}

/// Draw the whole shell and return the clickable regions.
pub fn render(canvas: &mut dyn SurfaceCanvas, frame: &ShellFrame) -> Vec<(crate::ui::UiRect, UiAction)> {
    let mut hits: Vec<(crate::ui::UiRect, UiAction)> = Vec::new();
    let pal = frame.palette;
    let w = frame.width;
    let h = frame.height;
    let (cell_w, cell_h) = canvas.mono_cell_size(crate::platform::FontRole::CodeMono);
    let row_h = cell_h.max(18);

    canvas.clear(pal.bg_app);

    // ---- tab strip ----------------------------------------------------------
    let tab_h = TABSTRIP_H;
    canvas.fill_rect(0, 0, w, tab_h, pal.bg_header);
    canvas.draw_hline(0, tab_h, w, pal.border_subtle);
    let mut tx = 8;
    for (i, tab) in frame.tabs.iter().enumerate() {
        let label = theme::truncate_right(&tab.title, 18);
        let tw = canvas.measure_text_width(&label, FontRole::UiSmall) + 22;
        let selected = i == frame.active_tab;
        canvas.fill_rounded_rect(
            tx,
            5,
            tw,
            tab_h - 12,
            5,
            if selected { pal.bg_card } else { pal.bg_header },
        );
        canvas.draw_text(
            tx + 11,
            11,
            &label,
            FontRole::UiSmall,
            if selected {
                pal.text_primary
            } else {
                pal.text_muted
            },
        );
        hits.push((
            crate::ui::UiRect::new(tx, 5, tw, tab_h - 12),
            UiAction::SelectSessionTab(i),
        ));
        tx += tw + 6;
        if tx > w - 90 {
            break;
        }
    }
    // `+` new session
    let plus_w = 26;
    canvas.fill_rounded_rect(tx, 5, plus_w, tab_h - 12, 5, pal.bg_header);
    canvas.draw_text(tx + 8, 11, "+", FontRole::UiSemibold, pal.text_secondary);
    hits.push((
        crate::ui::UiRect::new(tx, 5, plus_w, tab_h - 12),
        UiAction::NewSession,
    ));
    // view switch
    let switch_label = match frame.view {
        ShellView::Workspace => "Chat",
        _ => "Workspace",
    };
    let sw_w = canvas.measure_text_width(switch_label, FontRole::UiSmall) + 20;
    canvas.fill_rounded_rect(w - sw_w - 10, 5, sw_w, tab_h - 12, 5, pal.bg_button);
    canvas.draw_text(
        w - sw_w - 2,
        11,
        switch_label,
        FontRole::UiSmall,
        pal.text_secondary,
    );
    hits.push((
        crate::ui::UiRect::new(w - sw_w - 10, 5, sw_w, tab_h - 12),
        UiAction::ToggleShellView,
    ));

    let body_y = tab_h + 1;
    let body_h = h - body_y - STATUSBAR_H;

    match frame.view {
        ShellView::Connect => {
            if let Some(cm) = &frame.connect {
                let lay = connect::ConnectLayout {
                    x: 0,
                    y: body_y,
                    w,
                    h: body_h,
                };
                hits.extend(connect::draw(canvas, &lay, cm, row_h, cell_w, &pal));
            }
        }
        ShellView::Chat => {
            // ---- conversation column + sidebar -------------------------------
            let sidebar_w = if frame.sidebar.is_some() && w > 900 {
                SIDEBAR_W.min(w / 3).max(SIDEBAR_MIN_W)
            } else {
                0
            };
            let chat_w = w - sidebar_w;
            let chat_lay = chat::layout(0, body_y, chat_w, body_h, cell_w, row_h);
            let rows = chat::build_rows(&frame.chat, chat_lay.cols);
            hits.extend(chat::draw(
                canvas,
                &chat_lay,
                &rows,
                frame.chat_scroll,
                &pal,
                cell_w,
                row_h,
            ));

            if sidebar_w > 0 {
                if let Some(sm) = &frame.sidebar {
                    let slay = sidebar::layout(chat_w, body_y, sidebar_w, body_h);
                    hits.extend(sidebar::draw(canvas, &slay, sm, row_h, &pal));
                }
            }

            // ---- prompt ------------------------------------------------------
            let prompt_h = 58;
            let prompt_y = body_y + body_h - prompt_h;
            let play = prompt::layout(0, prompt_y, w, prompt_h);
            hits.extend(prompt::draw(canvas, &play, &frame.prompt, cell_w, row_h, &pal));
        }
        ShellView::Workspace => {
            // StudioApp::render dispatches to the IDE panels itself; the shell
            // only owns the tab strip and status bar in this mode.
        }
    }

    // ---- status bar ---------------------------------------------------------
    draw_status_bar(canvas, frame, &pal, w, h);

    // ---- palette overlay ----------------------------------------------------
    if let Some(pm) = &frame.command_palette {
        let lay = palette::PaletteLayout {
            x: 0,
            y: body_y,
            w,
            h: body_h,
        };
        hits.extend(palette::draw(canvas, &lay, pm, row_h, &pal));
    }

    hits
}

fn draw_status_bar(
    canvas: &mut dyn SurfaceCanvas,
    frame: &ShellFrame,
    pal: &crate::themes::ThemePalette,
    w: i32,
    h: i32,
) {
    let y = h - STATUSBAR_H;
    canvas.fill_rect(0, y, w, STATUSBAR_H, pal.bg_header);
    canvas.draw_hline(0, y, w, pal.border_subtle);

    let pct = (frame.token_pct.clamp(0.0, 1.0) * 100.0) as i32;
    let left = format!(
        "[ctrl+? help]  Tokens: {} ({}%)  Cost: {}",
        theme::fmt_tokens(frame.tokens),
        pct,
        theme::fmt_cost(frame.cost_usd)
    );
    canvas.draw_text(10, y + 6, &left, FontRole::UiSmall, pal.text_secondary);

    if let Some(f) = frame.flash {
        if f.visible() {
            let msg = theme::truncate_right(&f.text, 60);
            let mw = canvas.measure_text_width(&msg, FontRole::UiSmall);
            canvas.draw_text(w / 2 - mw / 2, y + 6, &msg, FontRole::UiSmall, pal.accent);
        }
    }

    // right: workspace + model box
    let ws = theme::shorten_path(frame.workspace_path, 40);
    let ws_w = canvas.measure_text_width(&ws, FontRole::UiSmall);
    canvas.draw_text(w - ws_w - 10, y + 6, &ws, FontRole::UiSmall, pal.text_muted);

    let model_box = format!("{} | {}", frame.model_label, frame.effort_label);
    let mb_w = canvas.measure_text_width(&model_box, FontRole::UiSmall) + 18;
    let mb_x = w - ws_w - mb_w - 26;
    canvas.fill_rounded_rect(mb_x, y + 3, mb_w, STATUSBAR_H - 6, 4, pal.accent);
    canvas.draw_text(
        mb_x + 9,
        y + 6,
        &model_box,
        FontRole::UiSmall,
        pal.on_accent_text(pal.accent),
    );
    canvas.draw_text(
        mb_x - 74,
        y + 6,
        &format!("{} v{}", frame.provider_label, frame.version),
        FontRole::UiSmall,
        pal.text_muted,
    );
}

use crate::platform::FontRole;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{FontRole, SurfaceCanvas};
    use crate::sqlite::{ChatMessage, SessionDetail, ToolExecutionRecord};
    use crate::themes::ThemePalette;
    use crate::ui::UiRect;

    /// Minimal in-memory canvas so the shell can be rendered headlessly.
    struct MockCanvas {
        w: i32,
        h: i32,
        ops: usize,
        texts: Vec<String>,
    }

    impl MockCanvas {
        fn new(w: i32, h: i32) -> Self {
            Self {
                w,
                h,
                ops: 0,
                texts: Vec::new(),
            }
        }
    }

    impl SurfaceCanvas for MockCanvas {
        fn width(&self) -> i32 {
            self.w
        }
        fn height(&self) -> i32 {
            self.h
        }
        fn clear(&mut self, _rgb: u32) {
            self.ops += 1;
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn fill_rounded_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _r: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn stroke_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn stroke_rounded_rect(&mut self, _x: i32, _y: i32, _w: i32, _h: i32, _r: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn draw_hline(&mut self, _x: i32, _y: i32, _w: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn draw_vline(&mut self, _x: i32, _y: i32, _h: i32, _rgb: u32) {
            self.ops += 1;
        }
        fn draw_text(&mut self, _x: i32, _y: i32, text: &str, _f: FontRole, _rgb: u32) {
            self.ops += 1;
            self.texts.push(text.to_string());
        }
        fn draw_text_clipped(&mut self, _x: i32, _y: i32, _m: i32, text: &str, _f: FontRole, _rgb: u32) {
            self.ops += 1;
            self.texts.push(text.to_string());
        }
        fn measure_text_width(&mut self, text: &str, _f: FontRole) -> i32 {
            text.chars().count() as i32 * 8
        }
        fn mono_cell_size(&mut self, _f: FontRole) -> (i32, i32) {
            (8, 18)
        }
        fn push_clip(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}
        fn pop_clip(&mut self) {}
    }

    fn sample_session() -> SessionDetail {
        SessionDetail {
            id: "s1".into(),
            title: "Fix the login bug".into(),
            project_path: "/home/user/project".into(),
            mode: "build".into(),
            parent_id: String::new(),
            compacted_count: 0,
            created_at: "2026-01-01T10:00:00".into(),
            updated_at: "2026-01-01T10:05:00".into(),
            messages: vec![
                ChatMessage {
                    id: "m1".into(),
                    session_id: "s1".into(),
                    role: "user".into(),
                    content: "fix the failing login test".into(),
                    thinking: String::new(),
                    mode: "build".into(),
                    provider: "opencode".into(),
                    model: "big-pickle".into(),
                    tools: Vec::new(),
                    created_at: "2026-01-01T10:00:00".into(),
                },
                ChatMessage {
                    id: "m2".into(),
                    session_id: "s1".into(),
                    role: "assistant".into(),
                    content: "I found the bug.\n\n- wrong password hash\n- missing session cookie".into(),
                    thinking: "The test compares plaintext.".into(),
                    mode: "build".into(),
                    provider: "opencode".into(),
                    model: "big-pickle".into(),
                    tools: vec![ToolExecutionRecord {
                        id: "t1".into(),
                        session_id: "s1".into(),
                        tool_name: "edit".into(),
                        args_json: "{\"path\":\"src/auth.rs\"}".into(),
                        result_summary: "patched hash comparison".into(),
                        status: "ok".into(),
                        duration_ms: 42,
                        diff_text: "--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1,2 +1,2 @@\n-old line\n+new line".into(),
                        file_path: "src/auth.rs".into(),
                        created_at: "2026-01-01T10:01:00".into(),
                    }],
                    created_at: "2026-01-01T10:01:00".into(),
                },
            ],
        }
    }

    #[test]
    fn test_shell_renders_without_panicking() {
        let pal = ThemePalette::by_id("low-cortisol", None);
        let session = sample_session();
        let tabs = vec![SessionTab {
            id: "s1".into(),
            title: "Fix the login bug".into(),
            active: true,
        }];
        let chat = chat::ChatModel {
            session: &session,
            streaming_text: "",
            streaming_thinking: "",
            live_tools: &[],
            running: false,
            status_line: "",
            show_thinking: true,
            expanded: &[],
            active_model: "big-pickle",
            active_mode: "build",
            effort: "medium",
        };
        let sidebar = sidebar::SidebarModel {
            session: &session,
            tokens: 16_400,
            token_pct: 0.08,
            cost_usd: 0.12,
            mcp_servers: &[],
            mcp_status: &[],
            todos: &[],
            modified: &[("src/auth.rs".to_string(), 3, 1)],
            collapsed: &[],
            version: "2.2.0",
        };
        let prompt = prompt::PromptModel {
            value: "",
            caret: 0,
            focused: true,
            caret_visible: true,
            history: &[],
            slash_hits: &[],
            file_hits: &[],
            ac_index: 0,
            running: false,
        };
        let frame = ShellFrame {
            width: 1400,
            height: 900,
            palette: &pal,
            view: ShellView::Chat,
            tabs: &tabs,
            active_tab: 0,
            chat,
            chat_scroll: 0,
            sidebar: Some(sidebar),
            prompt,
            model_label: "big-pickle",
            provider_label: "opencode",
            effort_label: "medium",
            tokens: 16_400,
            token_pct: 0.08,
            cost_usd: 0.12,
            running: false,
            flash: None,
            workspace_path: "/home/user/project",
            version: "2.2.0",
            command_palette: None,
            connect: None,
        };
        let mut canvas = MockCanvas::new(1400, 900);
        let hits = render(&mut canvas, &frame);
        assert!(canvas.ops > 50, "shell drew almost nothing");
        assert!(!hits.is_empty(), "shell produced no hit zones");
        // The transcript, sidebar, prompt and tab strip all register clicks.
        assert!(hits.iter().any(|(r, _)| r.y < TABSTRIP_H));
        assert!(hits
            .iter()
            .any(|(r, _)| r.x > 1000 && r.y > TABSTRIP_H));
    }

    #[test]
    fn test_shell_renders_connect_view() {
        let pal = ThemePalette::by_id("low-cortisol", None);
        let session = sample_session();
        let tabs = vec![SessionTab {
            id: "s1".into(),
            title: "s".into(),
            active: true,
        }];
        let chat = chat::ChatModel {
            session: &session,
            streaming_text: "",
            streaming_thinking: "",
            live_tools: &[],
            running: false,
            status_line: "",
            show_thinking: true,
            expanded: &[],
            active_model: "big-pickle",
            active_mode: "build",
            effort: "medium",
        };
        let prompt = prompt::PromptModel {
            value: "",
            caret: 0,
            focused: false,
            caret_visible: false,
            history: &[],
            slash_hits: &[],
            file_hits: &[],
            ac_index: 0,
            running: false,
        };
        let models: Vec<String> = vec!["big-pickle".into(), "grok-code".into()];
        let connect = connect::ConnectModel {
            provider: "opencode",
            model: "big-pickle",
            key: "",
            url: "http://127.0.0.1:4096",
            auth_mode: "zen",
            zen_models: &models,
            zen_status: "",
            detected: &["~/.local/share/opencode/auth.json".into()],
            detected_file: &Some("~/.local/share/opencode/auth.json".into()),
            local_server_alive: false,
            ollama_alive: false,
            focus_key: false,
            caret_visible: false,
            scroll: 0,
        };
        let frame = ShellFrame {
            width: 1200,
            height: 800,
            palette: &pal,
            view: ShellView::Connect,
            tabs: &tabs,
            active_tab: 0,
            chat,
            chat_scroll: 0,
            sidebar: None,
            prompt,
            model_label: "big-pickle",
            provider_label: "opencode",
            effort_label: "medium",
            tokens: 0,
            token_pct: 0.0,
            cost_usd: 0.0,
            running: false,
            flash: None,
            workspace_path: "/home/user/project",
            version: "2.2.0",
            command_palette: None,
            connect: Some(connect),
        };
        let mut canvas = MockCanvas::new(1200, 800);
        let hits = render(&mut canvas, &frame);
        // The free-lane and save buttons must be clickable.
        assert!(hits
            .iter()
            .any(|(_, a)| matches!(a, UiAction::UseZenFreeLane)));
        assert!(hits
            .iter()
            .any(|(_, a)| matches!(a, UiAction::SaveConnectWizard)));
        assert!(hits
            .iter()
            .any(|(_, a)| matches!(a, UiAction::ImportDetectedAuth)));
        assert!(hits
            .iter()
            .any(|(_, a)| matches!(a, UiAction::FetchZenFreeModels)));
        assert!(canvas.texts.iter().any(|t| t.contains("Connect a provider")));
    }

    #[test]
    fn test_chat_rows_include_disclosures_and_diffs() {
        let session = sample_session();
        let model = chat::ChatModel {
            session: &session,
            streaming_text: "",
            streaming_thinking: "",
            live_tools: &[],
            running: false,
            status_line: "",
            show_thinking: true,
            expanded: &["tool:m2:0".into()],
            active_model: "big-pickle",
            active_mode: "build",
            effort: "medium",
        };
        let rows = chat::build_rows(&model, 80);
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::TurnHeader));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::UserText));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::ThoughtHeader));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::ToolHeader));
        // The expanded tool block exposes its diff rows.
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::DiffAdd));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::DiffDel));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::Closer));
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::Footer));
        assert!(rows.iter().any(|r| r.text.contains("src/auth.rs")));
    }

    #[test]
    fn test_chat_rows_collapsed_hides_payloads() {
        let session = sample_session();
        let model = chat::ChatModel {
            session: &session,
            streaming_text: "",
            streaming_thinking: "",
            live_tools: &[],
            running: false,
            status_line: "",
            show_thinking: true,
            expanded: &[],
            active_model: "big-pickle",
            active_mode: "build",
            effort: "medium",
        };
        let rows = chat::build_rows(&model, 80);
        // Nothing is expanded, so no body / diff rows should exist.
        assert!(!rows.iter().any(|r| r.kind == chat::RowKind::ToolBody));
        assert!(!rows.iter().any(|r| r.kind == chat::RowKind::DiffAdd));
        assert!(!rows.iter().any(|r| r.kind == chat::RowKind::ThoughtBody));
        // ...but the disclosure headers are still there to click.
        assert!(rows.iter().any(|r| r.kind == chat::RowKind::ToolHeader));
    }

    #[test]
    fn test_theme_helpers() {
        assert_eq!(truncate_right("hello", 10), "hello");
        assert_eq!(truncate_right("hello world", 8), "hello w~");
        assert_eq!(truncate_right("abc", 0), "");
        assert!(truncate_left("abcdefghij", 4).ends_with("hij"));
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(16_400), "16.4K");
        assert_eq!(fmt_tokens(2_500_000), "2.5M");
        assert_eq!(fmt_cost(0.1234), "$0.123");
        assert_eq!(fmt_cost(0.0012), "$0.0012");
        assert_eq!(fmt_cost(1.5), "$1.50");
        let short = shorten_path("/a/b/c/d.rs", 6);
        assert!(short.ends_with("d.rs"));
        assert!(short.chars().count() <= 6);
        let wrapped = wrap_text("aaa bbb ccc ddd", 7);
        assert!(wrapped.iter().all(|l| l.chars().count() <= 7));
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(lighten(0x101010, 5), 0x151515);
        let _ = UiRect::new(0, 0, 1, 1);
    }
}
