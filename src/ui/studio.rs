//! AirCode — Pure-Rust Native Desktop Studio Workbench (`StudioApp`).
//!
//! Renders directly onto the OS 32-bit pixel surface (Win32 DIBSection + ClearType GDI on Windows,
//! X11 SoftwareSurface on Linux) — zero WebView, zero HTML/CSS/JS application UI.
//!
//! v2.1 layout (uncluttered & spacious):
//!   ┌───────────────────────────────────────────────────────────────────────────────┐
//!   │ Header: brand · workspace · provider · mode · effort · theme · Ctrl+K search    │
//!   ├──────┬───────────────────────────────────────────────┬─────────────────────────┤
//!   │ Rail │ Explorer / Search / Git / Sessions / Skills   │  Agent · Timeline       │
//!   │ 58px │ (native editor, minimap, diff view)           │  (chat, prompts, tools) │
//!   ├──────┴───────────────────────────────────────────────┴─────────────────────────┤
//!   │ Integrated Terminal Drawer (toggle Ctrl+J)                                       │
//!   ├─────────────────────────────────────────────────────────────────────────────────┤
//!   │ Status footer                                                                    │
//!   └─────────────────────────────────────────────────────────────────────────────────┘
//! Overlays: Fuzzy Command Panel (Ctrl+K / Ctrl+P / /), /connect Wizard, Theme Picker,
//! Safety Settings, Permission Gate & Delete Confirmation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crate::agent::{
    probe_ollama_models, spawn_autonomous_agent_run, spawn_session_compaction, AgentUiEvent,
    PermissionGate, PermissionPrompt,
};
use crate::browser::{open_in_external_browser, BrowserJsEngine, BrowserOwner, SharedBrowser};
use crate::editor::{char_len, slice_chars, DiffLineKind, EditorTab, TabContent};
use crate::fs_ops::{
    resolve_workspace_path, scan_project_tree, search_project_files, DriveInfo, FileEntry,
    SearchMatch,
};
use crate::git_ops::{inspect_git_status, load_head_and_working_diff, run_git, GitStatusSnapshot};
use crate::mcp::{load_mcp_configs, save_workspace_mcp_configs, McpServerConfig};
use crate::opencode_auth::{
    discover_local_credentials, try_spawn_local_opencode_server, ProviderGuide, PROVIDER_GUIDES,
};
use crate::platform::{
    self, CursorShape, DesktopAppHandler, DirectoryWatcher, FontRole, KeyCode, Modifiers, MouseBtn,
    NativePtySession, PtyEvent, SurfaceCanvas, WindowEvent,
};
use crate::skills::load_all_skills;
use crate::sqlite::{AppSettings, SessionDetail, SessionSummary, SqliteStore, ToolExecutionRecord};
use crate::subagents::{load_all_subagents, save_subagent, SubAgentSpec};
use crate::syntax::tokenize_line;
use crate::themes::ThemePalette;
use crate::ui::commands::{builtin_palette_commands, filter_palette_items, PaletteItem};
use crate::ui::shell::{self, ShellFlash, ShellView, SessionTab};
use crate::ui::{
    CenterView, FocusTarget, HitZone, LeftTab, ModalDialog, UiAction, UiRect,
};
use crate::vt_screen::VtScreen;

// ---------------------------------------------------------------------------
// Layout constants — generous spacing for a clean, modern, uncluttered feel
// ---------------------------------------------------------------------------
/// Workspace-mode chrome must sit below the shell's tab strip and above its
/// status bar, otherwise the two layers overlap.
const SHELL_CHROME_TOP: i32 = crate::ui::shell::TABSTRIP_H + 1;
const SHELL_CHROME_BOTTOM: i32 = crate::ui::shell::STATUSBAR_H;
const RAIL_W: i32 = 60;
const FOOTER_H: i32 = 26;
const PAD: i32 = 14;
const CARD_RADIUS: i32 = 8;

struct TerminalSession {
    title: String,
    shell_kind: String,
    pty: NativePtySession,
    rx: Receiver<PtyEvent>,
    screen: VtScreen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragSplitter {
    LeftPanel,
    RightAgent,
    BottomTerminal,
}

#[derive(Clone, Debug)]
struct DeleteConfirmState {
    abs_path: PathBuf,
    rel_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentPanelTab {
    Chat,
    Timeline,
}

/// Guided OpenCode-style `/connect` wizard state.
#[derive(Clone, Debug)]
pub struct ConnectWizardState {
    pub provider_id: String,
    pub model: String,
    pub key_input: String,
    pub url_input: String,
    pub opencode_auth_mode: String, // "zen" | "local-serve"
    pub detected_summary: Vec<String>,
    pub detected_opencode_file: Option<String>,
    pub local_server_alive: bool,
    pub ollama_alive: bool,
    /// Live free-tier catalog pulled anonymously from `GET /zen/v1/models`.
    pub zen_free_models: Vec<String>,
    /// Short status line for the Zen free-lane card.
    pub zen_status: String,
}

#[derive(Clone, Debug)]
struct ChatBlock {
    bg: u32,
    fg: u32,
    text: String,
    diff_rel: Option<String>,
    /// Some((prompt_index)) when this block starts a persisted message
    msg_idx: Option<usize>,
}

pub struct StudioApp {
    pub width: i32,
    pub height: i32,
    mouse_x: i32,
    mouse_y: i32,

    // Resizable dock dimensions
    left_w: i32,
    right_w: i32,
    term_h: i32,
    active_splitter: Option<DragSplitter>,
    editor_dragging_selection: bool,

    // SQLite store + live settings + dynamic theme palette
    db: Arc<SqliteStore>,
    pub settings: AppSettings,
    pub palette: ThemePalette,
    ollama_models: Vec<String>,

    // Workspace & Explorer
    workspace: PathBuf,
    drives: Vec<DriveInfo>,
    entries: Vec<FileEntry>,
    collapsed_dirs: HashSet<String>,
    explorer_scroll: usize,
    left_tab: LeftTab,
    left_panel_open: bool,
    watcher: Option<DirectoryWatcher>,
    fs_changed_flag: Arc<AtomicBool>,

    creating_entry: Option<bool>,
    new_entry_name: String,

    // Search panel
    search_query: String,
    search_results: Vec<SearchMatch>,
    search_scroll: usize,

    // Git panel
    git_snapshot: GitStatusSnapshot,
    commit_message: String,
    git_scroll: usize,

    // Sessions & prompt timeline
    sessions: Vec<SessionSummary>,
    active_session: SessionDetail,
    sessions_scroll: usize,
    agent_panel_tab: AgentPanelTab,
    timeline_scroll: usize,
    msg_line_offsets: Vec<usize>,

    // Code Editor & Diff viewer tabs
    tabs: Vec<EditorTab>,
    active_tab: usize,
    show_find_bar: bool,
    find_query: String,
    show_goto_line: bool,
    goto_line_buf: String,
    editor_rect: UiRect,
    show_minimap: bool,

    // Integrated terminal drawer
    terminals: Vec<TerminalSession>,
    active_term: usize,
    terminal_open: bool,
    term_grid_rect: UiRect,

    // Autonomous AI agent
    pub active_mode: String,
    live_tools: Vec<ToolExecutionRecord>,
    streaming_thinking: String,
    streaming_text: String,
    pub agent_prompt: String,
    agent_running: bool,
    compacting: bool,
    agent_status_line: String,
    agent_rx: Option<Receiver<AgentUiEvent>>,
    compact_rx: Option<Receiver<AgentUiEvent>>,
    permission_gate: Arc<PermissionGate>,
    agent_scroll_lines: usize,

    // Inbuilt Agentic Browser
    browser: SharedBrowser,
    center_view: CenterView,
    browser_url_buf: String,
    browser_scroll: i32,
    browser_rect: UiRect,

    // Command palette & modals
    modal: ModalDialog,
    palette_open: bool,
    palette_query: String,
    palette_index: usize,
    pub connect: ConnectWizardState,

    // Skills / Sub-Agents / MCP hub
    skills: Vec<crate::skills::SkillEntry>,
    subagents: Vec<SubAgentSpec>,
    mcp_servers: Vec<McpServerConfig>,
    extensions_scroll: usize,
    new_mcp_name: String,
    new_mcp_target: String,
    show_mcp_add: bool,
    hidden_thinking_ids: Vec<String>,

    // Safety / preferences modal

    // ---- OpenCode-style shell state --------------------------------------
    shell_view: ShellView,
    chat_scroll: usize,
    /// When true the transcript stays pinned to the newest row.
    chat_pinned: bool,
    /// Block ids whose `+ Thought` / `+ Tool` disclosure is expanded.
    expanded_blocks: Vec<String>,
    show_tool_blocks: bool,
    sidebar_open: bool,
    sidebar_collapsed: Vec<String>,
    /// Prompts already sent this process, newest last.
    prompt_history: Vec<String>,
    /// Index into `prompt_history` while recalling with Up/Down.
    history_idx: Option<usize>,
    shell_flash: ShellFlash,
    /// Scratch buffers so the shell models can borrow instead of clone.
    shell_mcp_status_buf: Vec<(String, String)>,
    shell_todos_buf: Vec<String>,
    shell_modified_buf: Vec<(String, usize, usize)>,
    shell_slash_buf: Vec<(&'static str, &'static str)>,
    shell_file_buf: Vec<String>,
    shell_palette_buf: Vec<PaletteItem>,
    shell_zen_buf: Vec<String>,

    pending_approval: Option<PermissionPrompt>,
    pending_delete: Option<DeleteConfirmState>,
    toast_text: String,
    toast_ticks: u32,

    pub focus: FocusTarget,
    blink_phase: bool,
    tick_counter: u64,
    hit_zones: Vec<HitZone>,
}

impl StudioApp {
    pub fn new(initial_workspace: Option<PathBuf>) -> Self {
        Self::new_with_options(initial_workspace, true)
    }

    /// Test / introspection helpers for the shell state machine.
    pub fn shell_view(&self) -> crate::ui::shell::ShellView {
        self.shell_view
    }
    pub fn palette_open(&self) -> bool {
        self.palette_open
    }
    pub fn prompt_text(&self) -> &str {
        &self.agent_prompt
    }
    pub fn prompt_text_set(&mut self, v: &str) {
        self.agent_prompt = v.to_string();
    }
    pub fn chat_block_expanded(&self, id: &str) -> bool {
        self.expanded_blocks.iter().any(|b| b == id)
    }
    /// Test helper: pin the transcript to the newest row.
    pub fn chat_pin_to_bottom(&mut self) {
        self.chat_pinned = true;
    }

    /// Test helper: append a message to the active session and reload it.
    pub fn seed_message_for_test(&mut self, role: &str, content: &str, thinking: &str) {
        let msg = crate::sqlite::ChatMessage {
            id: crate::sqlite::make_unique_id("msg"),
            session_id: self.active_session.id.clone(),
            role: role.to_string(),
            content: content.to_string(),
            thinking: thinking.to_string(),
            mode: self.active_mode.clone(),
            provider: self.settings.active_provider.clone(),
            model: self.settings.active_model.clone(),
            tools: Vec::new(),
            created_at: crate::sqlite::now_timestamp_label(),
        };
        self.db.append_message(&msg);
        if let Some(updated) = self.db.load_session(&self.active_session.id) {
            self.active_session = updated;
        }
    }

    /// Snapshot of the interactive regions from the last frame (used by tests).
    pub fn hit_zones_snapshot(&self) -> Vec<crate::ui::UiRect> {
        self.hit_zones.iter().map(|z| z.rect).collect()
    }
    pub fn sidebar_section_collapsed(&self, name: &str) -> bool {
        self.sidebar_collapsed.iter().any(|c| c == name)
    }

    /// Public API used by the CLI (`--new-session`, `--browser`, ...) so the
    /// desktop-entry actions are real features rather than dead shortcuts.
    pub fn start_new_session(&mut self) {
        self.execute_action(UiAction::NewSession);
        self.shell_view = ShellView::Chat;
        self.focus = FocusTarget::AgentPrompt;
    }

    /// Open the agentic browser as the primary surface.
    pub fn open_browser_view(&mut self) {
        self.shell_view = ShellView::Workspace;
        self.center_view = CenterView::Browser;
        self.left_panel_open = false;
        self.terminal_open = false;
    }

    /// Open the provider setup screen (what `/connect` and the palette do).
    pub fn open_connect_screen(&mut self) {
        self.show_connect();
    }

    /// Headless constructor used by unit tests / CI smoke runs (no PTY spawning).
    pub fn new_headless_for_test(store: Arc<SqliteStore>, workspace: PathBuf) -> Self {
        Self::build(Some(store), Some(workspace), false)
    }

    pub fn new_with_options(initial_workspace: Option<PathBuf>, spawn_default_shell: bool) -> Self {
        Self::build(None, initial_workspace, spawn_default_shell)
    }

    fn build(
        store: Option<Arc<SqliteStore>>,
        initial_workspace: Option<PathBuf>,
        spawn_default_shell: bool,
    ) -> Self {
        let data_dir = platform::app_data_dir();
        let db = store.unwrap_or_else(|| {
            Arc::new(SqliteStore::open(&data_dir).unwrap_or_else(|_| {
                let fallback = std::env::temp_dir().join("aircode_fallback_db");
                SqliteStore::open(&fallback).expect("fallback sqlite open")
            }))
        });
        let settings = db.load_settings();
        let recent = db.list_recent_projects();

        let ws = initial_workspace
            .or_else(|| recent.first().map(PathBuf::from).filter(|p| p.is_dir()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let ws_clean = clean_win_verbatim(&std::fs::canonicalize(&ws).unwrap_or(ws));
        let ws_str = ws_clean.to_string_lossy().to_string();
        db.record_recent_project(&ws_str);

        let active_session = db.ensure_default_session(&ws_str);
        let sessions = db.list_sessions();
        let drives = platform::list_drives();
        let fs_changed_flag = Arc::new(AtomicBool::new(false));
        let palette = ThemePalette::by_id(&settings.theme, Some(&ws_clean));
        let browser = SharedBrowser::new();

        let mut app = Self {
            width: 1520,
            height: 940,
            mouse_x: 0,
            mouse_y: 0,
            left_w: 288,
            right_w: 430,
            term_h: 232,
            active_splitter: None,
            editor_dragging_selection: false,
            db,
            settings: settings.clone(),
            palette,
            ollama_models: Vec::new(),
            workspace: ws_clean,
            drives,
            entries: Vec::new(),
            collapsed_dirs: HashSet::new(),
            explorer_scroll: 0,
            left_tab: LeftTab::Explorer,
            left_panel_open: true,
            watcher: None,
            fs_changed_flag,
            creating_entry: None,
            new_entry_name: String::new(),
            search_query: String::new(),
            search_results: Vec::new(),
            search_scroll: 0,
            git_snapshot: GitStatusSnapshot::default(),
            commit_message: String::new(),
            git_scroll: 0,
            sessions,
            active_session,
            sessions_scroll: 0,
            agent_panel_tab: AgentPanelTab::Chat,
            timeline_scroll: 0,
            msg_line_offsets: Vec::new(),
            tabs: Vec::new(),
            active_tab: 0,
            show_find_bar: false,
            find_query: String::new(),
            show_goto_line: false,
            goto_line_buf: String::new(),
            editor_rect: UiRect::new(RAIL_W, 100, 700, 400),
            show_minimap: true,
            terminals: Vec::new(),
            active_term: 0,
            terminal_open: true,
            term_grid_rect: UiRect::new(RAIL_W, 600, 700, 200),
            active_mode: if settings.default_mode.is_empty() {
                "build".to_string()
            } else {
                settings.default_mode.clone()
            },
            live_tools: Vec::new(),
            streaming_thinking: String::new(),
            streaming_text: String::new(),
            agent_prompt: String::new(),
            agent_running: false,
            compacting: false,
            agent_status_line: "Ready — AirCode Pure-Rust Studio".to_string(),
            agent_rx: None,
            compact_rx: None,
            permission_gate: Arc::new(PermissionGate::new()),
            agent_scroll_lines: 0,
            browser,
            center_view: CenterView::Editor,
            browser_url_buf: String::new(),
            browser_scroll: 0,
            browser_rect: UiRect::new(RAIL_W, 100, 700, 400),
            modal: ModalDialog::None,
            palette_open: false,
            palette_query: String::new(),
            palette_index: 0,
            connect: ConnectWizardState {
                provider_id: settings.active_provider.clone(),
                model: settings.active_model.clone(),
                key_input: String::new(),
                url_input: "http://127.0.0.1:4096".to_string(),
                opencode_auth_mode: settings.opencode_auth_mode.clone(),
                detected_summary: Vec::new(),
                detected_opencode_file: None,
                local_server_alive: false,
                ollama_alive: false,
                zen_free_models: crate::opencode_auth::ZEN_FREE_MODELS
                    .iter()
                    .map(|(id, _)| (*id).to_string())
                    .collect(),
                zen_status: String::new(),
            },
            skills: Vec::new(),
            subagents: Vec::new(),
            mcp_servers: Vec::new(),
            extensions_scroll: 0,
            new_mcp_name: String::new(),
            new_mcp_target: String::new(),
            show_mcp_add: false,
            hidden_thinking_ids: Vec::new(),
            pending_approval: None,
            pending_delete: None,
            toast_text: "AirCode v2.2 — Pure-Rust AI Coding Studio & Agentic Browser".to_string(),
            toast_ticks: 110,
            focus: FocusTarget::AgentPrompt,
            blink_phase: true,
            tick_counter: 0,
            hit_zones: Vec::with_capacity(320),
            shell_view: ShellView::Chat,
            chat_scroll: 0,
            chat_pinned: true,
            expanded_blocks: Vec::new(),
            show_tool_blocks: true,
            sidebar_open: true,
            sidebar_collapsed: Vec::new(),
            prompt_history: Vec::new(),
            history_idx: None,
            shell_flash: ShellFlash::default(),
            shell_mcp_status_buf: Vec::new(),
            shell_todos_buf: Vec::new(),
            shell_modified_buf: Vec::new(),
            shell_slash_buf: Vec::new(),
            shell_file_buf: Vec::new(),
            shell_palette_buf: Vec::new(),
            shell_zen_buf: Vec::new(),
        };

        app.reload_customizations();
        app.reload_workspace(true);
        if spawn_default_shell {
            let shell = if app.settings.default_shell == "cmd" {
                "cmd"
            } else if cfg!(target_os = "windows") {
                "powershell"
            } else {
                "sh"
            };
            app.spawn_terminal(shell);
        }
        app
    }

    // -----------------------------------------------------------------------
    // Small utilities
    // -----------------------------------------------------------------------
    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast_text = msg.into();
        self.toast_ticks = 100;
    }

    fn refresh_palette(&mut self) {
        self.palette = ThemePalette::by_id(&self.settings.theme, Some(&self.workspace));
    }

    /// Hot-reloads skills, sub-agents and MCP servers from `.aircode/`.
    fn reload_customizations(&mut self) {
        let disabled: HashSet<String> = self.settings.disabled_skills.iter().cloned().collect();
        self.skills = load_all_skills(&self.workspace, &platform::app_data_dir(), &disabled);
        self.subagents = load_all_subagents(&self.workspace);
        self.mcp_servers = load_mcp_configs(&self.workspace, &platform::app_data_dir());
    }

    pub fn reload_workspace(&mut self, reset_watcher: bool) {
        self.drives = platform::list_drives();
        self.entries = scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
        self.git_snapshot = inspect_git_status(&self.workspace);
        if reset_watcher {
            if let Some(mut old) = self.watcher.take() {
                old.stop();
            }
            self.fs_changed_flag.store(false, Ordering::SeqCst);
            self.watcher = Some(DirectoryWatcher::start(
                &self.workspace,
                self.fs_changed_flag.clone(),
            ));
        }

        if self.tabs.is_empty() {
            for candidate in ["src/main.rs", "README.md", "Cargo.toml"] {
                let p = self.workspace.join(candidate);
                if p.is_file() {
                    let _ = self.open_file_in_tab(&p, candidate);
                    break;
                }
            }
        }
    }

    fn switch_workspace(&mut self, new_ws: PathBuf) {
        let clean = clean_win_verbatim(&std::fs::canonicalize(&new_ws).unwrap_or(new_ws));
        if !clean.is_dir() {
            self.show_toast(format!("Directory unavailable: {}", clean.display()));
            return;
        }
        self.workspace = clean;
        self.db
            .record_recent_project(&self.workspace.to_string_lossy());
        self.explorer_scroll = 0;
        self.collapsed_dirs.clear();
        self.reload_workspace(true);
        self.reload_customizations();
        self.refresh_palette();
        self.sync_active_pty_cwd();
        self.show_toast(format!("Workspace: {}", self.workspace.display()));
    }

    fn open_file_in_tab(&mut self, abs_path: &Path, rel_path: &str) -> Result<(), String> {
        for (idx, tab) in self.tabs.iter().enumerate() {
            if tab.full_path.as_deref() == Some(abs_path) {
                self.active_tab = idx;
                self.focus = FocusTarget::Editor;
                self.center_view = CenterView::Editor;
                return Ok(());
            }
        }
        let bytes = std::fs::read(abs_path)
            .map_err(|e| format!("Failed reading {}: {}", abs_path.display(), e))?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("File exceeds 4 MB editor tab limit.".to_string());
        }
        if bytes.iter().take(2048).any(|&b| b == 0) {
            return Err("Binary file skipped.".to_string());
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let title = abs_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| rel_path.to_string());
        let tab = EditorTab::new_code(
            title,
            rel_path.to_string(),
            Some(abs_path.to_path_buf()),
            &text,
            false,
        );
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.focus = FocusTarget::Editor;
        self.center_view = CenterView::Editor;
        Ok(())
    }

    fn open_git_diff_tab(&mut self, rel_path: &str) {
        let (orig, modified) = load_head_and_working_diff(&self.workspace, rel_path);
        for (idx, t) in self.tabs.iter_mut().enumerate() {
            if matches!(t.content, TabContent::Diff { .. }) && t.rel_path == rel_path {
                *t = EditorTab::new_diff(rel_path.to_string(), &orig, &modified);
                self.active_tab = idx;
                self.center_view = CenterView::Editor;
                return;
            }
        }
        self.tabs
            .push(EditorTab::new_diff(rel_path.to_string(), &orig, &modified));
        self.active_tab = self.tabs.len() - 1;
        self.center_view = CenterView::Editor;
    }

    fn refresh_open_code_tabs_from_disk(&mut self) {
        for tab in &mut self.tabs {
            if !tab.is_dirty() {
                if let Some(full) = tab.full_path.clone() {
                    if full.is_file() {
                        if let Ok(fresh) = std::fs::read_to_string(&full) {
                            let norm = fresh.replace("\r\n", "\n");
                            if norm != tab.full_text() {
                                if let TabContent::Code {
                                    lines,
                                    saved_text,
                                    cursor_line,
                                    cursor_col,
                                    scroll_y,
                                    ..
                                } = &mut tab.content
                                {
                                    *lines = norm.split('\n').map(|s| s.to_string()).collect();
                                    if lines.is_empty() {
                                        lines.push(String::new());
                                    }
                                    *saved_text = norm;
                                    *cursor_line = (*cursor_line).min(lines.len() - 1);
                                    *cursor_col = (*cursor_col).min(char_len(&lines[*cursor_line]));
                                    *scroll_y = (*scroll_y).min(lines.len() - 1);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn save_active_tab(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            if matches!(tab.content, TabContent::Code { .. }) {
                let target = tab
                    .full_path
                    .clone()
                    .unwrap_or_else(|| self.workspace.join(&tab.rel_path));
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let content = tab.full_text();
                match std::fs::write(&target, content.as_bytes()) {
                    Ok(()) => {
                        tab.full_path = Some(target);
                        tab.mark_saved();
                        let saved_title = tab.title.clone();
                        self.git_snapshot = inspect_git_status(&self.workspace);
                        self.entries =
                            scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
                        self.show_toast(format!("Saved {}", saved_title));
                    }
                    Err(e) => self.show_toast(format!("Save failed: {}", e)),
                }
            }
        }
    }

    fn spawn_terminal(&mut self, shell_kind: &str) {
        let cols = ((self.width - self.left_w - self.right_w - 24).max(320) / 8) as u16;
        let rows = ((self.term_h - 38).max(80) / 17) as u16;
        match NativePtySession::spawn(
            shell_kind,
            &self.workspace,
            cols.max(60),
            rows.max(12),
            &[],
        ) {
            Ok((pty, rx)) => {
                let idx = self.terminals.len() + 1;
                let label = if shell_kind.eq_ignore_ascii_case("cmd") {
                    format!("CMD #{}", idx)
                } else if cfg!(target_os = "windows") {
                    format!("PowerShell #{}", idx)
                } else {
                    format!("Shell #{}", idx)
                };
                let mut screen = VtScreen::new(cols.max(60) as usize, rows.max(12) as usize);
                screen.feed_str(&format!(
                    "\x1b[36m[Connected: {} in {}]\x1b[0m\r\n",
                    label,
                    self.workspace.display()
                ));
                self.terminals.push(TerminalSession {
                    title: label,
                    shell_kind: shell_kind.to_string(),
                    pty,
                    rx,
                    screen,
                });
                self.active_term = self.terminals.len() - 1;
                self.terminal_open = true;
            }
            Err(e) => self.show_toast(format!("Terminal error: {}", e)),
        }
    }

    fn sync_active_pty_cwd(&mut self) {
        if let Some(term) = self.terminals.get(self.active_term) {
            let ws = self.workspace.to_string_lossy().to_string();
            let cmd = if term.shell_kind == "cmd" {
                format!("cd /d \"{}\"\r", ws)
            } else if cfg!(target_os = "windows") {
                format!("Set-Location -LiteralPath '{}'\r", ws.replace('\'', "''"))
            } else {
                format!("cd '{}'\n", ws.replace('\'', "'\\''"))
            };
            term.pty.write(cmd.as_bytes());
        }
    }

    // -----------------------------------------------------------------------
    // Agent run + background stream draining
    // -----------------------------------------------------------------------
    /// Starts an autonomous run for `prompt_override`, or for whatever is in
    /// the composer when `None`. Shared by every front-end.
    pub fn launch_agent_prompt(&mut self, prompt_override: Option<String>) {
        if self.agent_running {
            self.show_toast("Agent is running — click STOP to interrupt first.");
            return;
        }
        let prompt = prompt_override.unwrap_or_else(|| {
            let p = self.agent_prompt.trim().to_string();
            self.agent_prompt.clear();
            p
        });
        if prompt.is_empty() {
            return;
        }

        // Remember what was sent so Up/Down can recall it in the composer.
        if self.prompt_history.last().map(|p| p != &prompt).unwrap_or(true) {
            self.prompt_history.push(prompt.clone());
            if self.prompt_history.len() > 64 {
                self.prompt_history.remove(0);
            }
        }
        self.history_idx = None;
        self.chat_pinned = true; // follow the new turn

        self.db.save_settings(&self.settings);
        self.live_tools.clear();
        self.streaming_thinking.clear();
        self.streaming_text.clear();
        self.agent_scroll_lines = 0;
        self.agent_panel_tab = AgentPanelTab::Chat;

        let (tx, rx) = mpsc::channel();
        let sid = self.active_session.id.clone();
        let mode = self.active_mode.clone();

        spawn_autonomous_agent_run(
            self.db.clone(),
            self.permission_gate.clone(),
            self.browser.clone(),
            self.workspace.clone(),
            sid.clone(),
            prompt,
            mode,
            tx,
        );

        if let Some(updated) = self.db.load_session(&sid) {
            self.active_session = updated;
        }
        self.agent_rx = Some(rx);
        self.agent_running = true;
        self.agent_status_line = format!(
            "Autonomous run started — {} / {} mode",
            self.settings.active_provider, self.active_mode
        );
    }

    fn spawn_compaction(&mut self) {
        if self.compacting {
            self.show_toast("Compaction already in progress.");
            return;
        }
        let (tx, rx) = mpsc::channel();
        spawn_session_compaction(self.db.clone(), self.active_session.id.clone(), tx);
        self.compact_rx = Some(rx);
        self.compacting = true;
        self.agent_status_line = "Compacting session context into a checkpoint...".to_string();
        self.show_toast("Compacting session context...");
    }

    /// A plain-data view of the app for front-ends that have no canvas.
    ///
    /// This is the seam the TUI and the web front-end render through. It is
    /// deliberately cheap: everything is a `String`, `Vec` or scalar, so it can
    /// be serialised straight to JSON or walked as text.
    pub fn snapshot(&self) -> crate::ui::snapshot::AppSnapshot {
        use crate::ui::snapshot::*;

        let messages = self
            .active_session
            .messages
            .iter()
            .map(|m| SnapshotMessage {
                id: m.id.clone(),
                role: m.role.clone(),
                content: m.content.clone(),
                thinking: m.thinking.clone(),
                model: m.model.clone(),
                provider: m.provider.clone(),
                mode: m.mode.clone(),
                created_at: m.created_at.clone(),
                tools: m.tools.iter().map(SnapshotTool::from_record).collect(),
            })
            .collect();

        // Same numbers the shell's status bar shows, so a terminal and the
        // desktop app never disagree about cost or context pressure.
        let tokens = self.shell_token_estimate();
        let context_pct = self.shell_token_pct();
        let cost = self.shell_cost();

        let (mcp, todos, modified) = self.shell_sidecar_lists();

        AppSnapshot {
            version: crate::VERSION.to_string(),
            workspace: self.workspace.display().to_string(),
            shell_view: match self.shell_view {
                crate::ui::shell::ShellView::Chat => "chat".to_string(),
                crate::ui::shell::ShellView::Workspace => "workspace".to_string(),
                crate::ui::shell::ShellView::Connect => "connect".to_string(),
            },
            palette_open: self.palette_open,

            sessions: self.sessions.clone(),
            active_session_id: self.active_session.id.clone(),
            active_title: self.active_session.title.clone(),
            messages,

            agent_running: self.agent_running,
            compacting: self.compacting,
            status_line: self.agent_status_line.clone(),
            streaming_thinking: self.streaming_thinking.clone(),
            streaming_text: self.streaming_text.clone(),
            live_tools: self.live_tools.iter().map(SnapshotTool::from_record).collect(),

            prompt_text: self.agent_prompt.clone(),
            prompt_history: self.prompt_history.clone(),
            slash_hits: self
                .shell_slash_buf
                .iter()
                .map(|(c, d)| ((*c).to_string(), (*d).to_string()))
                .collect(),
            file_hits: self.shell_file_buf.clone(),
            palette_items: self
                .shell_palette_buf
                .iter()
                .map(|i| crate::ui::snapshot::SnapshotPaletteItem {
                    id: i.id.clone(),
                    slash: i.slash.clone(),
                    title: i.title.clone(),
                    subtitle: i.subtitle.clone(),
                    shortcut: i.shortcut.clone(),
                    is_file: i.is_file,
                })
                .collect(),
            palette_query: self.palette_query.clone(),
            toast: self.toast_text.clone(),

            model: self.settings.active_model.clone(),
            provider: self.settings.active_provider.clone(),
            mode: self.active_mode.clone(),
            tokens,
            context_pct,
            cost,

            mcp_servers: mcp,
            todos,
            modified_files: modified,

            pending_approval: self.pending_approval.clone(),

            palette: self.palette.clone(),
            connect: SnapshotConnect {
                open: self.connect_screen_open(),
                provider_id: self.connect.provider_id.clone(),
                model: self.connect.model.clone(),
                key_input: self.connect.key_input.clone(),
                url_input: self.connect.url_input.clone(),
                auth_mode: self.connect.opencode_auth_mode.clone(),
                detected: self.connect.detected_summary.clone(),
                detected_file: self.connect.detected_opencode_file.clone(),
                local_server_alive: self.connect.local_server_alive,
                ollama_alive: self.connect.ollama_alive,
                zen_models: self.connect.zen_free_models.clone(),
                zen_status: self.connect.zen_status.clone(),
            },
            sidebar_open: self.sidebar_open,
            expanded_blocks: self.expanded_blocks.clone(),
            show_tool_blocks: self.show_tool_blocks,
        }
    }

    /// Pumps the terminal PTYs, the filesystem watcher and the agent event
    /// stream. The native GUI calls this from `render()`; headless front-ends
    /// (TUI, web) have no render loop of their own, so they call `tick()`.
    pub fn tick(&mut self) {
        self.drain_background_streams();
        // Keeps Tab-completion and the palette live without a render frame.
        self.refresh_shell_scratch();
    }

    fn drain_background_streams(&mut self) {
        // 1. Terminal PTY streams
        for term in &mut self.terminals {
            while let Ok(ev) = term.rx.try_recv() {
                match ev {
                    PtyEvent::Output(bytes) => term.screen.feed_bytes(&bytes),
                    PtyEvent::Exited(code) => {
                        term.screen.feed_str(&format!(
                            "\r\n\x1b[33m[Process exited with code {}]\x1b[0m\r\n",
                            code
                        ));
                    }
                }
            }
        }

        // 2. Filesystem watcher
        if self.fs_changed_flag.swap(false, Ordering::SeqCst) {
            self.entries = scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
            self.git_snapshot = inspect_git_status(&self.workspace);
            self.refresh_open_code_tabs_from_disk();
        }

        // 3. Autonomous agent events
        let mut finished_session: Option<Option<SessionDetail>> = None;
        let mut workspace_mutated = false;

        let agent_rx = self.agent_rx.take();
        if let Some(rx) = agent_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    AgentUiEvent::Status(s) => self.agent_status_line = s,
                    AgentUiEvent::ThinkingDelta(d) => self.streaming_thinking.push_str(&d),
                    AgentUiEvent::TextDelta(d) => {
                        if !self.streaming_text.is_empty() {
                            self.streaming_text.push('\n');
                        }
                        self.streaming_text.push_str(&d);
                    }
                    AgentUiEvent::ToolStarted(rec) => self.live_tools.push(rec),
                    AgentUiEvent::ToolFinished(rec) => {
                        if !rec.file_path.is_empty()
                            || matches!(
                                rec.tool_name.as_str(),
                                "write_file" | "edit_file" | "execute_command" | "git_operation"
                            )
                        {
                            workspace_mutated = true;
                        }
                        if let Some(existing) = self.live_tools.iter_mut().find(|r| r.id == rec.id) {
                            *existing = rec;
                        } else {
                            self.live_tools.push(rec);
                        }
                    }
                    AgentUiEvent::PermissionRequested(prompt) => {
                        self.pending_approval = Some(prompt);
                    }
                    AgentUiEvent::WorkspaceMutated => workspace_mutated = true,
                    AgentUiEvent::OpenBrowserViewport => {
                        self.center_view = CenterView::Browser;
                        self.show_toast("Agent opened the Agentic Browser viewport.");
                    }
                    AgentUiEvent::ReloadCustomizations => {
                        self.reload_customizations();
                        self.show_toast("Customizations reloaded (skills / MCP / sub-agents).");
                    }
                    AgentUiEvent::Error(err) => {
                        self.agent_status_line = format!("Error: {}", err);
                    }
                    AgentUiEvent::Done(opt_detail) => finished_session = Some(opt_detail),
                }
            }
            if self.agent_running {
                self.agent_rx = Some(rx);
            }
        }

        // 4. Compaction stream
        let compact_rx = self.compact_rx.take();
        if let Some(rx) = compact_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    AgentUiEvent::Status(s) => self.agent_status_line = s,
                    AgentUiEvent::Done(detail) => {
                        if let Some(d) = detail {
                            self.active_session = d;
                        }
                        self.compacting = false;
                        self.compact_rx = None;
                        self.sessions = self.db.list_sessions();
                        self.show_toast("Session compacted — timeline & forks preserved.");
                    }
                    AgentUiEvent::Error(err) => {
                        self.compacting = false;
                        self.compact_rx = None;
                        self.agent_status_line = format!("Compaction error: {}", err);
                    }
                    _ => {}
                }
            }
            if self.compacting {
                self.compact_rx = Some(rx);
            }
        }

        if workspace_mutated {
            self.entries = scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
            self.git_snapshot = inspect_git_status(&self.workspace);
            self.refresh_open_code_tabs_from_disk();
        }

        if let Some(opt_detail) = finished_session {
            self.agent_running = false;
            self.agent_rx = None;
            self.pending_approval = None;
            self.streaming_thinking.clear();
            self.streaming_text.clear();
            self.live_tools.clear();
            if let Some(detail) = opt_detail {
                self.active_session = detail;
            } else if let Some(detail) = self.db.load_session(&self.active_session.id) {
                self.active_session = detail;
            }
            self.sessions = self.db.list_sessions();
            if !self.agent_status_line.starts_with("Error:") {
                self.agent_status_line = "Turn complete.".to_string();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Slash commands, fuzzy command palette, /connect wizard, themes & extensions
// ---------------------------------------------------------------------------
impl StudioApp {
    /// Executes a `/command` typed into the agent prompt. Returns true when handled.
    pub fn execute_slash_command(&mut self, raw: &str) -> bool {
        let cmd = raw.trim();
        if cmd.is_empty() {
            return false;
        }
        let lower = cmd.to_ascii_lowercase();

        match lower.as_str() {
            "/thinking" | "/thinking on" | "/think" => {
                self.settings.show_thinking = true;
                self.persist_settings();
                self.show_toast("AI thinking blocks are now VISIBLE.");
            }
            "/thinking off" | "/hide-thinking" => {
                self.settings.show_thinking = false;
                self.persist_settings();
                self.show_toast("AI thinking blocks are now HIDDEN.");
            }
            "/plan" | "/mode plan" => {
                self.active_mode = "plan".to_string();
                self.settings.default_mode = "plan".to_string();
                self.persist_settings();
                self.show_toast("PLAN mode — read-only research & architecture planning.");
            }
            "/build" | "/mode build" => {
                self.active_mode = "build".to_string();
                self.settings.default_mode = "build".to_string();
                self.persist_settings();
                self.show_toast("BUILD mode — full autonomous execution enabled.");
            }
            "/compact" | "/summarize" => self.spawn_compaction(),
            "/fork" => {
                self.execute_action(UiAction::ForkActiveSession);
            }
            "/new" | "/new-session" => {
                self.execute_action(UiAction::NewSession);
            }
            "/timeline" => {
                self.agent_panel_tab = AgentPanelTab::Timeline;
                self.show_toast("Prompt Timeline — click a prompt to jump or fork from it.");
            }
            "/chat" => {
                self.agent_panel_tab = AgentPanelTab::Chat;
            }
            "/browser" => {
                self.center_view = CenterView::Browser;
                self.show_toast("Agentic Browser viewport opened.");
            }
            "/workspace" => {
                self.execute_action(UiAction::ToggleShellView);
            }
            "/sidebar" => {
                self.execute_action(UiAction::ToggleSidebar);
            }
            "/tools" => {
                self.execute_action(UiAction::ToggleToolBlocks);
            }
            "/model" => {
                self.cycle_model();
            }
            "/models" => {
                // Documented behaviour: refresh the anonymous Zen catalog.
                self.fetch_zen_free_models();
                if !self.connect.zen_free_models.is_empty() {
                    self.show_toast(format!(
                        "{} free Zen models live right now.",
                        self.connect.zen_free_models.len()
                    ));
                }
            }
            "/editor" => {
                self.center_view = CenterView::Editor;
            }
            "/terminal" => {
                self.terminal_open = !self.terminal_open;
            }
            "/takeover" => {
                self.browser.interrupt_for_user();
                self.show_toast("You now control the browser — AI actions paused.");
            }
            "/resume" => {
                self.browser.hand_back_to_ai();
                self.show_toast("AI browser control resumed.");
            }
            "/skills" | "/mcp" | "/subagents" | "/extensions" => {
                self.left_tab = LeftTab::Extensions;
                self.left_panel_open = true;
            }
            "/sessions" => {
                self.left_tab = LeftTab::Sessions;
                self.left_panel_open = true;
            }
            "/theme" | "/themes" => {
                self.modal = ModalDialog::ThemePicker;
            }
            "/connect" | "/setup" | "/auth" | "/keys" | "/login" | "/signin" => {
                self.show_connect();
            }
            // One-click OpenCode Zen free lane (OpenCode-client auth emulation).
            "/zen" | "/free" => {
                self.use_zen_free_lane();
            }
            // Refresh the anonymous Zen catalog.
            "/zen models" => {
                self.fetch_zen_free_models();
                if !self.connect.zen_free_models.is_empty() {
                    self.show_toast(format!(
                        "{} free Zen models: {}",
                        self.connect.zen_free_models.len(),
                        self.connect
                            .zen_free_models
                            .iter()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            "/settings" | "/safety" => {
                self.modal = ModalDialog::SafetySettings;
            }
            "/goto" => {
                self.show_goto_line = true;
                self.focus = FocusTarget::GotoLineInput;
            }
            "/find" => {
                self.show_find_bar = true;
                self.focus = FocusTarget::FindInput;
            }
            "/customize" | "/customizer" => {
                self.agent_prompt = "Use the aircode-customizer skill to customize AirCode. First read .aircode/skills/aircode-customizer/SKILL.md, then ask me which theme colors, font size, skills, sub-agents or MCP servers I want, and write the correct .aircode/ files.".to_string();
                self.show_toast("AirCode Customizer skill prompt loaded.");
            }
            _ => {
                // Parameterized commands
                if let Some(rest) = lower.strip_prefix("/effort") {
                    let effort = rest.trim();
                    if matches!(effort, "low" | "medium" | "max" | "ultra") {
                        self.settings.agent_effort = effort.to_string();
                        self.persist_settings();
                        self.show_toast(format!("Agent effort set to {}", effort.to_uppercase()));
                        return true;
                    }
                    self.cycle_effort();
                    return true;
                }
                if let Some(rest) = lower.strip_prefix("/theme ") {
                    let id = rest.trim();
                    if ThemePalette::builtin_list().iter().any(|(bid, _)| *bid == id) {
                        self.settings.theme = id.to_string();
                        self.refresh_palette();
                        self.persist_settings();
                        self.show_toast(format!("Theme applied: {}", self.palette.name));
                        return true;
                    }
                    self.show_toast(format!("Unknown theme '{}' — opening theme picker.", id));
                    self.modal = ModalDialog::ThemePicker;
                    return true;
                }
                if let Some(rest) = lower.strip_prefix("/model ") {
                    let m = rest.trim().to_string();
                    if !m.is_empty() {
                        self.settings.active_model = m;
                        self.persist_settings();
                        self.show_toast(format!("Model set to {}", self.settings.active_model));
                        return true;
                    }
                }
                if let Some(rest) = lower.strip_prefix("/provider ") {
                    let p = rest.trim().to_string();
                    if PROVIDER_GUIDES.iter().any(|g| g.id == p) {
                        self.settings.active_provider = p.clone();
                        if let Some(first) = AppSettings::preset_models_for_provider(&p).first() {
                            self.settings.active_model = (*first).to_string();
                        }
                        self.persist_settings();
                        self.show_toast(format!("Provider set to {}", p));
                        return true;
                    }
                }
                if let Some(rest) = lower.strip_prefix("/fork ") {
                    // "/fork 3" forks from prompt #3
                    if let Ok(n) = rest.trim().parse::<usize>() {
                        self.execute_action(UiAction::ForkFromMessage(n.saturating_sub(1)));
                        return true;
                    }
                }
                if let Some(rest) = lower.strip_prefix("/goto ") {
                    if let Ok(n) = rest.trim().parse::<usize>() {
                        let vis = self.visible_editor_rows();
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            tab.goto_line(n, vis);
                        }
                        return true;
                    }
                }
                return false;
            }
        }
        true
    }

    fn persist_settings(&mut self) {
        self.db.save_settings(&self.settings);
    }

    fn visible_editor_rows(&self) -> usize {
        ((self.editor_rect.h - 12).max(40) / 20).max(3) as usize
    }

    pub fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_index = 0;
        self.modal = ModalDialog::CommandPalette;
        self.focus = FocusTarget::CommandPaletteInput;
    }

    fn palette_entries(&self) -> Vec<PaletteItem> {
        let files: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .take(900)
            .map(|e| (e.rel_path.clone(), e.name.clone()))
            .collect();
        let commands = builtin_palette_commands(
            self.settings.show_thinking,
            &self.active_mode,
            &self.settings.agent_effort,
            &self.settings.theme,
        );
        filter_palette_items(&self.palette_query, &commands, &files)
    }

    fn apply_palette_item(&mut self, item: &PaletteItem) {
        self.modal = ModalDialog::None;
        self.palette_open = false;
        if item.is_file {
            if let Some(rel) = item.id.strip_prefix("file:") {
                let abs = self.workspace.join(rel);
                if let Err(e) = self.open_file_in_tab(&abs, rel) {
                    self.show_toast(e);
                }
            }
            return;
        }
        let id = item.id.clone();
        self.execute_action(UiAction::ExecuteCommandId(id));
    }

    fn execute_command_id(&mut self, id: String) {
        match id.as_str() {
            "cmd:zen_free" => self.use_zen_free_lane(),
            "cmd:zen_models" => self.fetch_zen_free_models(),
            "cmd:connect" => self.show_connect(),
            "cmd:shell_workspace" => {
                self.execute_action(UiAction::ToggleShellView);
            }
            "cmd:shell_sidebar" => {
                self.execute_action(UiAction::ToggleSidebar);
            }
            "cmd:shell_tools" => {
                self.execute_action(UiAction::ToggleToolBlocks);
            }
            "cmd:cycle_model" => {
                self.cycle_model();
            }
            "cmd:toggle_thinking" => {
                self.settings.show_thinking = !self.settings.show_thinking;
                self.persist_settings();
                self.show_toast(if self.settings.show_thinking {
                    "AI thinking blocks VISIBLE."
                } else {
                    "AI thinking blocks HIDDEN."
                });
            }
            "cmd:compact" => self.spawn_compaction(),
            "cmd:fork" => self.execute_action(UiAction::ForkActiveSession),
            "cmd:new_session" => self.execute_action(UiAction::NewSession),
            "cmd:timeline" => {
                self.agent_panel_tab = AgentPanelTab::Timeline;
            }
            "cmd:mode_plan" => self.execute_action(UiAction::SetMode("plan")),
            "cmd:mode_build" => self.execute_action(UiAction::SetMode("build")),
            "cmd:effort_low" => self.execute_action(UiAction::SetEffort("low")),
            "cmd:effort_medium" => self.execute_action(UiAction::SetEffort("medium")),
            "cmd:effort_max" => self.execute_action(UiAction::SetEffort("max")),
            "cmd:effort_ultra" => self.execute_action(UiAction::SetEffort("ultra")),
            "cmd:theme_picker" => self.modal = ModalDialog::ThemePicker,
            "cmd:browser" => {
                self.center_view = CenterView::Browser;
                self.show_toast("Agentic Browser — co-browse with your AI.");
            }
            "cmd:browser_takeover" => {
                self.center_view = CenterView::Browser;
                self.browser.interrupt_for_user();
                self.show_toast("Browser takeover active.");
            }
            "cmd:skills" => {
                self.left_tab = LeftTab::Extensions;
                self.left_panel_open = true;
            }
            "cmd:customize_aircode" => {
                let _ = self.execute_slash_command("/customize");
            }
            "cmd:toggle_terminal" => self.terminal_open = !self.terminal_open,
            "cmd:goto_line" => {
                self.show_goto_line = true;
                self.focus = FocusTarget::GotoLineInput;
            }
            "cmd:find" => {
                self.show_find_bar = true;
                self.focus = FocusTarget::FindInput;
            }
            "cmd:open_folder" => self.execute_action(UiAction::PickFolder),
            "cmd:safety" => self.modal = ModalDialog::SafetySettings,
            "panel:chat" => {
                self.agent_panel_tab = AgentPanelTab::Chat;
            }
            "panel:timeline" => {
                self.agent_panel_tab = AgentPanelTab::Timeline;
            }
            "mcp:add" => {
                self.show_mcp_add = true;
                self.focus = FocusTarget::McpNameInput;
            }
            "skills:guide" => self.execute_action(UiAction::OpenCustomizerGuide),
            "subagent:new" => {
                self.save_subagent_from_dialog("helper", "General Helper", "medium", false);
            }
            _ => {
                if let Some(rest) = id.strip_prefix("theme:") {
                    self.execute_action(UiAction::ApplyTheme(rest.to_string()));
                }
            }
        }
    }

    fn cycle_effort(&mut self) {
        let order = ["low", "medium", "max", "ultra"];
        let cur = order
            .iter()
            .position(|e| *e == self.settings.agent_effort)
            .unwrap_or(1);
        let next = order[(cur + 1) % order.len()];
        self.settings.agent_effort = next.to_string();
        self.persist_settings();
        self.show_toast(format!("Agent effort -> {}", next.to_uppercase()));
    }

    // -----------------------------------------------------------------------
    // /connect guided provider setup (OpenCode-style)
    // -----------------------------------------------------------------------
    pub fn open_connect_wizard(&mut self) {
        let disc = discover_local_credentials(&self.settings.opencode_server_url, &self.settings.ollama_base_url);
        self.connect = ConnectWizardState {
            provider_id: self.settings.active_provider.clone(),
            model: self.settings.active_model.clone(),
            key_input: String::new(),
            url_input: self.settings.opencode_server_url.clone(),
            opencode_auth_mode: self.settings.opencode_auth_mode.clone(),
            detected_summary: disc.sources_summary.clone(),
            detected_opencode_file: disc
                .opencode_auth_file
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            local_server_alive: disc.local_opencode_server_alive,
            ollama_alive: disc.local_ollama_alive,
            zen_free_models: crate::opencode_auth::ZEN_FREE_MODELS
                .iter()
                .map(|(id, _)| (*id).to_string())
                .collect(),
            zen_status: String::new(),
        };
        self.modal = ModalDialog::ConnectWizard;
        self.focus = FocusTarget::ConnectKeyInput;
        self.show_toast("Guided AI setup — pick a provider, paste a key, or import detected auth.");
    }

    fn connect_provider_guide(&self) -> &'static ProviderGuide {
        PROVIDER_GUIDES
            .iter()
            .find(|g| g.id == self.connect.provider_id)
            .unwrap_or(&PROVIDER_GUIDES[0])
    }

    /// One-click OpenCode Zen free lane: no key, no account, no card.
    ///
    /// This is the workaround for "free models only work inside OpenCode": AirCode
    /// speaks the official client dialect (UA fingerprint + `x-opencode-*` session
    /// headers + `stream: true` + `bash`/`read` tool names), so the gateway treats
    /// it as the CLI and serves the anonymous `public` lane.
    fn use_zen_free_lane(&mut self) {
        self.connect.provider_id = "opencode".to_string();
        self.connect.opencode_auth_mode = "zen".to_string();
        self.connect.key_input = String::new();
        self.settings.active_provider = "opencode".to_string();
        self.settings.opencode_auth_mode = "zen".to_string();
        self.settings.opencode_api_key = String::new();
        let first_free = crate::opencode_auth::ZEN_FREE_MODELS
            .first()
            .map(|(id, _)| (*id).to_string())
            .unwrap_or_else(|| "big-pickle".to_string());
        self.settings.active_model = first_free.clone();
        self.connect.model = first_free.clone();
        self.persist_settings();
        self.refresh_palette();
        self.modal = ModalDialog::None;
        self.show_toast(format!(
            "OpenCode Zen free lane active — {} (no API key needed)",
            first_free
        ));
    }

    /// Refreshes the free-tier catalog straight from `GET https://opencode.ai/zen/v1/models`
    /// (that endpoint answers anonymously, which is how the catalog stays current).
    fn fetch_zen_free_models(&mut self) {
        self.connect.zen_status = "Fetching live Zen catalog...".to_string();
        let live = crate::opencode_auth::fetch_zen_free_models();
        if live.is_empty() {
            self.connect.zen_status =
                "Catalog unreachable from here — showing the built-in free list.".to_string();
        } else {
            self.connect.zen_free_models = live;
            self.connect.zen_status = format!(
                "{} free models live right now — pick one in the model box.",
                self.connect.zen_free_models.len()
            );
        }
    }

    fn save_connect_wizard(&mut self) {
        let provider = self.connect.provider_id.clone();
        let model = self.connect.model.clone();
        let key = self.connect.key_input.trim().to_string();
        let mode = self.connect.opencode_auth_mode.clone();
        let url = self.connect.url_input.trim().to_string();

        self.settings.active_provider = provider.clone();
        if !model.is_empty() {
            self.settings.active_model = model;
        }
        if !url.is_empty() {
            self.settings.opencode_server_url = url;
        }
        self.settings.opencode_auth_mode = mode.clone();
        match provider.as_str() {
            "opencode" => self.settings.opencode_api_key = key,
            "anthropic" => self.settings.anthropic_api_key = key,
            "openai" => self.settings.openai_api_key = key,
            "gemini" => self.settings.gemini_api_key = key,
            "openrouter" => self.settings.openrouter_api_key = key,
            "custom" => {
                self.settings.custom_api_key = key;
                if !self.settings.opencode_server_url.starts_with("http") {
                    self.settings.custom_base_url = self.settings.opencode_server_url.clone();
                }
            }
            "ollama" => {
                if !key.is_empty() {
                    self.settings.ollama_base_url = key;
                }
            }
            _ => {}
        }
        if self.settings.active_model.is_empty() {
            if let Some(first) = AppSettings::preset_models_for_provider(&provider).first() {
                self.settings.active_model = (*first).to_string();
            }
        }
        self.persist_settings();
        self.modal = ModalDialog::None;
        self.show_toast(format!(
            "Connected to {} ({}) — auth mode {}",
            provider, self.settings.active_model, self.settings.opencode_auth_mode
        ));
    }

    fn import_detected_auth(&mut self) {
        let disc = discover_local_credentials(
            &self.settings.opencode_server_url,
            &self.settings.ollama_base_url,
        );
        let mut imported = Vec::new();
        if let Some(k) = disc.opencode_zen_key.clone() {
            self.settings.opencode_api_key = k;
            imported.push("OpenCode Zen");
        }
        if let Some(k) = disc.anthropic_key.clone() {
            self.settings.anthropic_api_key = k;
            imported.push("Anthropic");
        }
        if let Some(k) = disc.openai_key.clone() {
            self.settings.openai_api_key = k;
            imported.push("OpenAI");
        }
        if let Some(k) = disc.gemini_key.clone() {
            self.settings.gemini_api_key = k;
            imported.push("Gemini");
        }
        if let Some(k) = disc.openrouter_key.clone() {
            self.settings.openrouter_api_key = k;
            imported.push("OpenRouter");
        }
        if disc.local_opencode_server_alive {
            self.settings.opencode_auth_mode = "local-serve".to_string();
            imported.push("opencode serve (bridge)");
        }
        self.connect.detected_summary = disc.sources_summary.clone();
        self.connect.detected_opencode_file = disc
            .opencode_auth_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        if imported.is_empty() {
            self.show_toast("No local credentials found — paste a key or use OpenCode Zen public mode.");
        } else {
            self.persist_settings();
            self.show_toast(format!("Imported credentials: {}", imported.join(", ")));
        }
    }

    fn spawn_local_opencode_serve(&mut self) {
        let port = 4096u16;
        match try_spawn_local_opencode_server(port) {
            Ok(url) => {
                self.settings.opencode_server_url = url.clone();
                self.settings.opencode_auth_mode = "local-serve".to_string();
                self.connect.url_input = url.clone();
                self.connect.opencode_auth_mode = "local-serve".to_string();
                self.connect.local_server_alive = true;
                self.persist_settings();
                self.show_toast(format!("opencode serve bridge running at {}", url));
            }
            Err(e) => {
                self.show_toast(e);
                self.show_toast("Fallback: OpenCode Zen public/key mode still works offline.");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Sessions, forks, compaction & timeline
    // -----------------------------------------------------------------------
    fn fork_active_session(&mut self, from_msg: Option<usize>) {
        let src = self.active_session.id.clone();
        let title_base = self.active_session.title.clone();
        let forked = self.db.fork_session(&src, from_msg);
        match forked {
            Some(detail) => {
                self.active_session = detail;
                self.sessions = self.db.list_sessions();
                self.agent_scroll_lines = 0;
                self.show_toast(format!(
                    "Forked {} ({} messages retained)",
                    title_base,
                    self.active_session.messages.len()
                ));
            }
            None => self.show_toast("Fork failed — source session unavailable."),
        }
    }

    fn scroll_to_message(&mut self, idx: usize) {
        if let Some(off) = self.msg_line_offsets.get(idx).copied() {
            self.agent_scroll_lines = off;
        }
    }

    // -----------------------------------------------------------------------
    // Skills, sub-agents, MCP hub
    // -----------------------------------------------------------------------
    fn toggle_skill(&mut self, slug: &str) {
        let mut disabled = self.settings.disabled_skills.clone();
        if disabled.iter().any(|s| s == slug) {
            disabled.retain(|s| s != slug);
        } else {
            disabled.push(slug.to_string());
        }
        self.settings.disabled_skills = disabled;
        self.persist_settings();
        self.reload_customizations();
    }

    fn add_quick_mcp_server(&mut self) {
        let name = self.new_mcp_name.trim().to_string();
        let target = self.new_mcp_target.trim().to_string();
        if name.is_empty() || target.is_empty() {
            self.show_toast("MCP server needs a name and a command / URL.");
            return;
        }
        let is_http = target.starts_with("http://") || target.starts_with("https://");
        let (transport, command, url, args) = if is_http {
            ("http".to_string(), String::new(), target.clone(), Vec::new())
        } else {
            let mut parts = target.split_whitespace();
            let cmd = parts.next().unwrap_or("").to_string();
            let rest: Vec<String> = parts.map(|s| s.to_string()).collect();
            ("stdio".to_string(), cmd, String::new(), rest)
        };
        let mut servers = self.mcp_servers.clone();
        servers.push(McpServerConfig {
            name,
            transport,
            command,
            args,
            url,
            enabled: true,
            status: "unprobed".to_string(),
            discovered_tools: Vec::new(),
        });
        match save_workspace_mcp_configs(&self.workspace, &servers) {
            Ok(()) => {
                self.mcp_servers = servers;
                self.new_mcp_name.clear();
                self.new_mcp_target.clear();
                self.show_mcp_add = false;
                self.show_toast("MCP server saved to .aircode/mcp.json");
            }
            Err(e) => self.show_toast(e),
        }
    }

    fn probe_all_mcp_servers(&mut self) {
        let mut probed = Vec::new();
        let mut total_tools = 0usize;
        for mut srv in self.mcp_servers.clone() {
            match crate::mcp::probe_mcp_server_tools(&mut srv, &self.workspace) {
                Ok(tools) => {
                    total_tools += tools.len();
                    srv.discovered_tools = tools;
                    srv.status = "online".to_string();
                }
                Err(e) => {
                    srv.status = format!("error: {}", crate::ui::studio::truncate_for_status(&e));
                }
            }
            probed.push(srv);
        }
        self.mcp_servers = probed;
        self.show_toast(format!("MCP probe complete — {} tools discovered.", total_tools));
    }

    fn save_subagent_from_dialog(&mut self, slug: &str, title: &str, effort: &str, readonly: bool) {
        if slug.trim().is_empty() || title.trim().is_empty() {
            self.show_toast("Sub-agent needs a slug and a title.");
            return;
        }
        let spec = SubAgentSpec {
            slug: slug.trim().to_string(),
            title: title.trim().to_string(),
            effort: effort.to_string(),
            readonly,
            system_prompt: format!(
                "You are {}, a specialized AirCode sub-agent focused on: {}. Work autonomously and report a concise result.",
                title.trim(),
                title.trim()
            ),
            builtin: false,
        };
        match save_subagent(&self.workspace, &spec) {
            Ok(_) => {
                self.reload_customizations();
                self.show_toast(format!("Sub-agent '{}' created in .aircode/subagents/", spec.slug));
            }
            Err(e) => self.show_toast(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Action dispatcher
// ---------------------------------------------------------------------------
impl StudioApp {
    // -----------------------------------------------------------------------
    // OpenCode-style shell actions
    // -----------------------------------------------------------------------
    fn shell_show_flash(&mut self, msg: impl Into<String>) {
        self.shell_flash = ShellFlash::new(msg);
    }

    fn toggle_chat_block(&mut self, id: String) {
        if let Some(pos) = self.expanded_blocks.iter().position(|b| *b == id) {
            self.expanded_blocks.remove(pos);
        } else {
            self.expanded_blocks.push(id);
        }
    }

    fn toggle_sidebar_section(&mut self, name: String) {
        if let Some(pos) = self.sidebar_collapsed.iter().position(|c| *c == name) {
            self.sidebar_collapsed.remove(pos);
        } else {
            self.sidebar_collapsed.push(name);
        }
    }

    /// Walk the sent-prompt history. `dir` is -1 (older) or +1 (newer).
    fn recall_prompt_history(&mut self, dir: i32) {
        if self.prompt_history.is_empty() {
            return;
        }
        let len = self.prompt_history.len();
        let next = match self.history_idx {
            None => {
                if dir < 0 {
                    Some(len - 1)
                } else {
                    None
                }
            }
            Some(i) => {
                if dir < 0 {
                    Some(i.saturating_sub(1))
                } else if i + 1 < len {
                    Some(i + 1)
                } else {
                    None
                }
            }
        };
        match next {
            Some(i) => {
                self.history_idx = Some(i);
                self.agent_prompt = self.prompt_history[i].clone();
            }
            None => {
                self.history_idx = None;
                self.agent_prompt.clear();
            }
        }
    }

    fn cycle_model(&mut self) {
        let models: Vec<String> = if self.settings.active_provider == "opencode" {
            if self.connect.zen_free_models.is_empty() {
                crate::opencode_auth::ZEN_FREE_MODELS
                    .iter()
                    .map(|(id, _)| (*id).to_string())
                    .collect()
            } else {
                self.connect.zen_free_models.clone()
            }
        } else {
            AppSettings::preset_models_for_provider(&self.settings.active_provider)
                .iter()
                .map(|m| (*m).to_string())
                .collect()
        };
        if models.is_empty() {
            self.shell_show_flash("No models for this provider yet.");
            return;
        }
        let cur = models
            .iter()
            .position(|m| *m == self.settings.active_model)
            .unwrap_or(0);
        let next = models[(cur + 1) % models.len()].clone();
        self.settings.active_model = next.clone();
        self.persist_settings();
        self.shell_show_flash(format!("Model -> {}", next));
    }

    /// Enter the provider-setup screen from anywhere.
    pub fn show_connect(&mut self) {
        self.shell_view = ShellView::Connect;
        self.open_connect_wizard();
    }

    pub fn execute_action(&mut self, action: UiAction) {
        // ---- OpenCode-style shell actions -----------------------------------
        match action {
            UiAction::ToggleShellView => {
                self.shell_view = if self.shell_view == ShellView::Workspace {
                    ShellView::Chat
                } else {
                    ShellView::Workspace
                };
                self.shell_show_flash(if self.shell_view == ShellView::Workspace {
                    "Workspace view — files, editor, terminal, git."
                } else {
                    "Chat view — the conversation is the app."
                });
                return;
            }
            UiAction::ShowConnect => {
                self.show_connect();
                return;
            }
            UiAction::SelectSessionTab(i) => {
                let tabs = self.shell_tabs();
                if let Some(t) = tabs.get(i) {
                    if t.active {
                        self.shell_view = ShellView::Chat;
                    } else {
                        let id = t.id.clone();
                        self.execute_action(UiAction::SelectSession(id));
                    }
                }
                return;
            }
            UiAction::ToggleChatBlock(id) => {
                self.toggle_chat_block(id);
                return;
            }
            UiAction::ToggleSidebarSection(name) => {
                self.toggle_sidebar_section(name);
                return;
            }
            UiAction::OpenWorkspaceFile(rel) => {
                let abs = self.workspace.join(&rel);
                if let Err(e) = self.open_file_in_tab(&abs, &rel) {
                    self.shell_show_flash(e);
                } else {
                    self.shell_view = ShellView::Workspace;
                }
                return;
            }
            UiAction::PaletteSelect(i) => {
                let items = self.shell_palette_items();
                if let Some(item) = items.get(i).cloned() {
                    self.apply_palette_item(&item);
                }
                return;
            }
            UiAction::FocusPrompt => {
                self.focus = FocusTarget::AgentPrompt;
                return;
            }
            UiAction::ScrollChat(delta) => {
                let rows = shell::chat::build_rows(&self.shell_chat_model(), ((self.width / 8).max(20)) as usize).len();
                let visible = ((self.height - shell::TABSTRIP_H - shell::STATUSBAR_H - 58) / 20).max(3) as usize;
                let max = rows.saturating_sub(visible);
                let next = (self.chat_scroll as isize + delta as isize).clamp(0, max as isize) as usize;
                self.chat_scroll = next;
                self.chat_pinned = next >= max;
                return;
            }
            UiAction::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                self.persist_settings();
                return;
            }
            UiAction::ToggleToolBlocks => {
                self.show_tool_blocks = !self.show_tool_blocks;
                self.expanded_blocks.clear();
                self.shell_show_flash(if self.show_tool_blocks {
                    "Tool blocks shown — click + to expand."
                } else {
                    "Tool blocks hidden."
                });
                return;
            }
            UiAction::CycleModel => {
                self.cycle_model();
                return;
            }
            _ => {}
        }
        match action {
            // Handled by the shell pass above; repeated so this match is exhaustive.
            UiAction::ToggleShellView
            | UiAction::ShowConnect
            | UiAction::SelectSessionTab(_)
            | UiAction::ToggleChatBlock(_)
            | UiAction::ToggleSidebarSection(_)
            | UiAction::OpenWorkspaceFile(_)
            | UiAction::PaletteSelect(_)
            | UiAction::FocusPrompt
            | UiAction::ScrollChat(_)
            | UiAction::ToggleSidebar
            | UiAction::ToggleToolBlocks
            | UiAction::CycleModel => {}
            UiAction::Focus(target) => self.focus = target,
            UiAction::PickFolder => {
                if let Some(chosen) = platform::pick_folder_dialog(&self.workspace) {
                    self.switch_workspace(chosen);
                }
            }
            UiAction::SwitchDrive(mount) => {
                let target = PathBuf::from(&mount);
                if let Some(chosen) = platform::pick_folder_dialog(&target) {
                    self.switch_workspace(chosen);
                } else if target.is_dir() {
                    self.switch_workspace(target);
                }
            }
            UiAction::ToggleRailTab(t) => {
                if self.left_tab == t && self.left_panel_open {
                    self.left_panel_open = false;
                } else {
                    self.left_tab = t;
                    self.left_panel_open = true;
                    match t {
                        LeftTab::Search => self.focus = FocusTarget::SearchInput,
                        LeftTab::Git => {
                            self.git_snapshot = inspect_git_status(&self.workspace);
                        }
                        LeftTab::Sessions => self.sessions = self.db.list_sessions(),
                        _ => {}
                    }
                }
            }
            UiAction::ToggleTerminalDrawer => {
                self.terminal_open = !self.terminal_open;
                if self.terminal_open && self.terminals.is_empty() {
                    self.spawn_terminal(if cfg!(target_os = "windows") {
                        "powershell"
                    } else {
                        "sh"
                    });
                }
            }
            UiAction::ToggleAgentPane => {
                self.right_w = if self.right_w > 60 { 0 } else { 430 };
            }
            UiAction::SwitchCenterView(v) => {
                self.center_view = v;
                if v == CenterView::Browser {
                    self.focus = FocusTarget::BrowserUrlInput;
                }
            }
            UiAction::RefreshWorkspace => {
                self.reload_workspace(false);
                self.refresh_open_code_tabs_from_disk();
                self.reload_customizations();
                self.show_toast("Workspace refreshed.");
            }
            UiAction::StartCreateEntry { is_dir } => {
                self.creating_entry = Some(is_dir);
                self.new_entry_name.clear();
                self.focus = FocusTarget::NewEntryInput;
            }
            UiAction::CancelCreateEntry => {
                self.creating_entry = None;
                self.new_entry_name.clear();
            }
            UiAction::ConfirmCreateEntry => {
                if let Some(is_dir) = self.creating_entry.take() {
                    let rel = self
                        .new_entry_name
                        .trim()
                        .trim_start_matches(['/', '\\'])
                        .to_string();
                    self.new_entry_name.clear();
                    if !rel.is_empty() {
                        match resolve_workspace_path(&self.workspace, &rel, true) {
                            Ok(abs) => {
                                let res = if is_dir {
                                    std::fs::create_dir_all(&abs)
                                } else {
                                    if let Some(p) = abs.parent() {
                                        let _ = std::fs::create_dir_all(p);
                                    }
                                    std::fs::write(&abs, b"")
                                };
                                match res {
                                    Ok(()) => {
                                        self.reload_workspace(false);
                                        if !is_dir {
                                            let _ = self.open_file_in_tab(&abs, &rel);
                                        }
                                        self.show_toast(format!("Created {}", rel));
                                    }
                                    Err(e) => self.show_toast(format!("Create failed: {}", e)),
                                }
                            }
                            Err(e) => self.show_toast(e),
                        }
                    }
                }
            }
            UiAction::ToggleDir(rel) => {
                if !self.collapsed_dirs.remove(&rel) {
                    self.collapsed_dirs.insert(rel);
                }
                self.entries = scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
            }
            UiAction::OpenFile(abs, rel, _name, _is_binary) => {
                if let Err(e) = self.open_file_in_tab(&abs, &rel) {
                    self.show_toast(e);
                }
            }
            UiAction::RequestDeleteFile(abs, rel) => {
                if self.settings.require_delete_confirmation {
                    self.pending_delete = Some(DeleteConfirmState {
                        abs_path: abs,
                        rel_path: rel,
                    });
                } else {
                    self.perform_delete_file(&abs, &rel);
                }
            }
            UiAction::RevealFile(abs) => platform::reveal_in_file_manager(&abs),
            UiAction::RunSearch => {
                let q = self.search_query.trim().to_string();
                if !q.is_empty() {
                    self.search_results =
                        search_project_files(&self.workspace, &q, false, 250);
                    self.search_scroll = 0;
                    self.show_toast(format!(
                        "Found {} matches for '{}'",
                        self.search_results.len(),
                        q
                    ));
                }
            }
            UiAction::OpenSearchHit(abs, rel, line_1) => {
                if self.open_file_in_tab(&abs, &rel).is_ok() {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        if let TabContent::Code {
                            lines,
                            cursor_line,
                            cursor_col,
                            sel_anchor,
                            scroll_y,
                            ..
                        } = &mut tab.content
                        {
                            let row = line_1.saturating_sub(1).min(lines.len().saturating_sub(1));
                            *cursor_line = row;
                            *cursor_col = 0;
                            *sel_anchor = None;
                            *scroll_y = row.saturating_sub(5);
                        }
                    }
                }
            }
            UiAction::GitStageFile {
                path,
                currently_staged,
            } => {
                let res = if currently_staged {
                    run_git(&self.workspace, &["reset", "HEAD", "--", &path])
                } else {
                    run_git(&self.workspace, &["add", "--", &path])
                };
                if let Err(e) = res {
                    self.show_toast(e);
                }
                self.git_snapshot = inspect_git_status(&self.workspace);
            }
            UiAction::GitStageAll => {
                if let Err(e) = run_git(&self.workspace, &["add", "-A"]) {
                    self.show_toast(e);
                }
                self.git_snapshot = inspect_git_status(&self.workspace);
            }
            UiAction::GitCommit => {
                let msg = self.commit_message.trim().to_string();
                if msg.is_empty() {
                    self.show_toast("Enter a commit message first.");
                    self.focus = FocusTarget::CommitInput;
                    return;
                }
                match run_git(&self.workspace, &["commit", "-m", &msg]) {
                    Ok((0, out, _)) => {
                        self.commit_message.clear();
                        self.git_snapshot = inspect_git_status(&self.workspace);
                        self.show_toast(out.lines().next().unwrap_or("Committed.").to_string());
                    }
                    Ok((_, o, e)) => {
                        self.show_toast(format!("Commit failed: {} {}", e.trim(), o.trim()));
                    }
                    Err(e) => self.show_toast(e),
                }
            }
            UiAction::GitCheckout(branch) => {
                if let Ok((0, _, _)) = run_git(&self.workspace, &["checkout", &branch]) {
                    self.reload_workspace(false);
                    self.refresh_open_code_tabs_from_disk();
                    self.show_toast(format!("Checked out {}", branch));
                }
            }
            UiAction::GitOpenDiff(rel) => self.open_git_diff_tab(&rel),
            UiAction::SelectEditorTab(idx) => {
                if idx < self.tabs.len() {
                    self.active_tab = idx;
                    self.focus = FocusTarget::Editor;
                    self.center_view = CenterView::Editor;
                }
            }
            UiAction::CloseEditorTab(idx) => {
                if idx < self.tabs.len() {
                    self.tabs.remove(idx);
                    if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                        self.active_tab = self.tabs.len() - 1;
                    }
                }
            }
            UiAction::SaveActiveTab => self.save_active_tab(),
            UiAction::NewScratchTab => {
                let name = format!("scratch_{}.rs", self.tabs.len() + 1);
                let abs = self.workspace.join(&name);
                let tab = EditorTab::new_code(
                    name.clone(),
                    name,
                    Some(abs),
                    "// AirCode Pure-Rust Scratch Buffer — Ctrl+S saves into the workspace\nfn main() {\n    println!(\"Hello from AirCode!\");\n}\n",
                    false,
                );
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
                self.focus = FocusTarget::Editor;
                self.center_view = CenterView::Editor;
            }
            UiAction::ToggleFindBar => {
                self.show_find_bar = !self.show_find_bar;
                self.focus = if self.show_find_bar {
                    FocusTarget::FindInput
                } else {
                    FocusTarget::Editor
                };
            }
            UiAction::FindNext => {
                let vis = self.visible_editor_rows();
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    if !tab.find_next(&self.find_query, vis.max(5)) {
                        self.show_toast(format!("No match for '{}'", self.find_query));
                    }
                }
            }
            UiAction::ToggleGotoLine => {
                self.show_goto_line = !self.show_goto_line;
                self.focus = if self.show_goto_line {
                    FocusTarget::GotoLineInput
                } else {
                    FocusTarget::Editor
                };
            }
            UiAction::ConfirmGotoLine => {
                if let Ok(n) = self.goto_line_buf.trim().parse::<usize>() {
                    let vis = self.visible_editor_rows();
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.goto_line(n, vis);
                    }
                    self.show_goto_line = false;
                    self.goto_line_buf.clear();
                    self.focus = FocusTarget::Editor;
                } else {
                    self.show_toast("Enter a valid line number.");
                }
            }
            UiAction::SelectPtyTab(idx) => {
                if idx < self.terminals.len() {
                    self.active_term = idx;
                    self.focus = FocusTarget::Terminal;
                }
            }
            UiAction::ClosePtyTab(idx) => {
                if idx < self.terminals.len() {
                    let mut removed = self.terminals.remove(idx);
                    removed.pty.kill();
                    if self.active_term >= self.terminals.len() && !self.terminals.is_empty() {
                        self.active_term = self.terminals.len() - 1;
                    }
                }
            }
            UiAction::SpawnPtyTab(kind) => {
                self.spawn_terminal(kind);
                self.focus = FocusTarget::Terminal;
            }
            UiAction::PtySendCtrlC => {
                if let Some(term) = self.terminals.get(self.active_term) {
                    term.pty.write(&[0x03]);
                }
            }
            UiAction::PtySyncCwd => self.sync_active_pty_cwd(),
            UiAction::PtyClearScreen => {
                if let Some(term) = self.terminals.get_mut(self.active_term) {
                    term.screen.feed_str("\x1b[2J\x1b[H");
                    term.pty.write(b"\r");
                }
            }
            UiAction::SetMode(mode) => {
                self.active_mode = mode.to_string();
                self.settings.default_mode = mode.to_string();
                self.persist_settings();
                self.show_toast(format!("{} mode active.", mode.to_uppercase()));
            }
            UiAction::CycleEffort => self.cycle_effort(),
            UiAction::SetEffort(effort) => {
                self.settings.agent_effort = effort.to_string();
                self.persist_settings();
                self.show_toast(format!("Agent effort -> {}", effort.to_uppercase()));
            }
            UiAction::ToggleThinkingVisibility => {
                self.settings.show_thinking = !self.settings.show_thinking;
                self.persist_settings();
                self.show_toast(if self.settings.show_thinking {
                    "AI thinking blocks VISIBLE."
                } else {
                    "AI thinking blocks HIDDEN."
                });
            }
            UiAction::ToggleMessageThinking(msg_id) => {
                if !self.hidden_thinking_ids.iter().any(|m| *m == msg_id) {
                    self.hidden_thinking_ids.push(msg_id);
                } else {
                    self.hidden_thinking_ids.retain(|m| *m != msg_id);
                }
            }
            UiAction::SendAgentPrompt => {
                let text = self.agent_prompt.trim().to_string();
                if text.starts_with('/') && self.execute_slash_command(&text) {
                    self.agent_prompt.clear();
                    return;
                }
                self.launch_agent_prompt(None);
            }
            UiAction::QuickPrompt(text) => self.launch_agent_prompt(Some(text.to_string())),
            UiAction::EmergencyStop => {
                if self.agent_running {
                    self.permission_gate.trigger_emergency_stop();
                    self.pending_approval = None;
                    self.agent_status_line = "Emergency Stop triggered by user!".to_string();
                    self.show_toast("Emergency Stop — halting agent loop immediately.");
                } else {
                    self.show_toast("No active agent run to stop.");
                }
            }
            UiAction::NewSession => {
                self.active_session = self.db.create_session(
                    "New Session",
                    &self.workspace.to_string_lossy(),
                    &self.active_mode,
                );
                self.sessions = self.db.list_sessions();
                self.live_tools.clear();
                self.agent_scroll_lines = 0;
                self.focus = FocusTarget::AgentPrompt;
                self.show_toast("New session created.");
            }
            UiAction::SelectSession(id) => {
                if let Some(detail) = self.db.load_session(&id) {
                    self.active_session = detail;
                    self.live_tools.clear();
                    self.agent_scroll_lines = 0;
                    self.sessions = self.db.list_sessions();
                }
            }
            UiAction::DeleteSession(id) => {
                self.db.delete_session(&id);
                self.sessions = self.db.list_sessions();
                if self.active_session.id == id {
                    self.active_session = self
                        .db
                        .ensure_default_session(&self.workspace.to_string_lossy());
                    self.sessions = self.db.list_sessions();
                }
            }
            UiAction::ForkActiveSession => self.fork_active_session(None),
            UiAction::ForkFromMessage(idx) => self.fork_active_session(Some(idx)),
            UiAction::CompactActiveSession => self.spawn_compaction(),
            UiAction::ScrollToMessage(idx) => {
                self.agent_panel_tab = AgentPanelTab::Chat;
                self.scroll_to_message(idx);
            }
            UiAction::OpenDiffPreview { title, diff } => {
                let tab = EditorTab::new_diff(title.clone(), &diff, &String::new());
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
                self.center_view = CenterView::Editor;
                self.show_toast(format!("Opened diff preview: {}", title));
            }
            UiAction::OpenModal(m) => {
                self.modal = m;
                match m {
                    ModalDialog::CommandPalette => self.open_command_palette(),
                    ModalDialog::ConnectWizard => self.open_connect_wizard(),
                    _ => {}
                }
            }
            UiAction::CloseModal => {
                self.modal = ModalDialog::None;
                self.palette_open = false;
            }
            UiAction::ExecuteCommandId(id) => self.execute_command_id(id),
            UiAction::SelectConnectProvider(p) => {
                self.connect.provider_id = p.to_string();
                if p == "ollama" {
                    let (alive, models) = probe_ollama_models(&self.settings.ollama_base_url);
                    self.ollama_models = models;
                    self.connect.ollama_alive = alive;
                    if let Some(first) = self.ollama_models.first() {
                        self.connect.model = first.clone();
                    }
                }
                if let Some(first) = AppSettings::preset_models_for_provider(p).first() {
                    if self.connect.model.is_empty() || p != "ollama" {
                        self.connect.model = (*first).to_string();
                    }
                }
                self.focus = FocusTarget::ConnectKeyInput;
            }
            UiAction::SelectConnectModel(m) => self.connect.model = m,
            UiAction::SetOpenCodeAuthMode(mode) => {
                self.connect.opencode_auth_mode = mode.to_string();
            }
            UiAction::PasteKeyFromClipboard => {
                if let Some(clip) = platform::clipboard_get_text() {
                    let clean: String = clip
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .take(512)
                        .collect();
                    self.connect.key_input = clean;
                    self.show_toast("Pasted key from clipboard.");
                } else {
                    self.show_toast("Clipboard is empty or unavailable.");
                }
            }
            UiAction::OpenProviderKeyUrl(url, open_external) => {
                let url = url.as_str();
                if open_external {
                    open_in_external_browser(url);
                    self.show_toast(format!("Opened {} in your system browser.", url));
                } else {
                    self.center_view = CenterView::Browser;
                    self.browser_url_buf = url.to_string();
                    let _ = self.browser.navigate(url, false);
                }
            }
            UiAction::ImportDetectedAuth => self.import_detected_auth(),
            UiAction::SpawnLocalOpenCodeServe => self.spawn_local_opencode_serve(),
            UiAction::UseZenFreeLane => self.use_zen_free_lane(),
            UiAction::FetchZenFreeModels => self.fetch_zen_free_models(),
            UiAction::SaveConnectWizard => self.save_connect_wizard(),
            UiAction::ApplyTheme(id) => {
                self.settings.theme = id.clone();
                self.refresh_palette();
                self.persist_settings();
                self.show_toast(format!("Theme: {}", self.palette.name));
            }
            UiAction::ToggleSkill(slug) => self.toggle_skill(&slug),
            UiAction::OpenCustomizerGuide => {
                self.center_view = CenterView::Editor;
                let rel = ".aircode/skills/aircode-customizer/SKILL.md";
                let abs = self.workspace.join(rel);
                if abs.is_file() {
                    let _ = self.open_file_in_tab(&abs, rel);
                } else {
                    let _ = self.execute_slash_command("/customize");
                }
            }
            UiAction::ProbeAllMcpServers => self.probe_all_mcp_servers(),
            UiAction::AddQuickMcpServer => self.add_quick_mcp_server(),
            UiAction::ToggleMcpServer(idx) => {
                if let Some(srv) = self.mcp_servers.get_mut(idx) {
                    srv.enabled = !srv.enabled;
                    srv.status = "unprobed".to_string();
                    let servers = self.mcp_servers.clone();
                    if let Err(e) = save_workspace_mcp_configs(&self.workspace, &servers) {
                        self.show_toast(e);
                    }
                }
            }
            UiAction::ToggleSettingBool(field) => match field {
                "require_shell_approval" => {
                    self.settings.require_shell_approval = !self.settings.require_shell_approval;
                }
                "require_delete_confirmation" => {
                    self.settings.require_delete_confirmation =
                        !self.settings.require_delete_confirmation;
                }
                "restrict_to_workspace" => {
                    self.settings.restrict_to_workspace = !self.settings.restrict_to_workspace;
                }
                "show_thinking" => {
                    self.settings.show_thinking = !self.settings.show_thinking;
                }
                "minimap" => {
                    self.show_minimap = !self.show_minimap;
                }
                _ => {}
            },
            UiAction::AdjustFontSize(delta) => {
                self.settings.editor_font_size =
                    (self.settings.editor_font_size + delta).clamp(12, 24);
                self.persist_settings();
            }
            UiAction::BrowserBack => self.browser.go_back(),
            UiAction::BrowserForward => self.browser.go_forward(),
            UiAction::BrowserReload => {
                let url = self.current_browser_url();
                if !url.is_empty() {
                    let _ = self.browser.navigate(&url, false);
                }
            }
            UiAction::BrowserGoUrl => {
                let url = self.browser_url_buf.trim().to_string();
                if !url.is_empty() {
                    match self.browser.navigate(&url, false) {
                        Ok(_) => self.show_toast(format!("Loaded {}", url)),
                        Err(e) => self.show_toast(e),
                    }
                }
            }
            UiAction::BrowserClickRef(r) => match self.browser.click_ref(r, false) {
                Ok(_) => {}
                Err(e) => self.show_toast(e),
            },
            UiAction::BrowserInterruptTakeover => {
                self.browser.interrupt_for_user();
                self.show_toast("You took control of the browser — AI paused.");
            }
            UiAction::BrowserHandBackToAi => {
                self.browser.hand_back_to_ai();
                self.show_toast("AI browser control resumed.");
            }
            UiAction::BrowserToggleEngine => {
                let next = if self.browser_engine() == BrowserJsEngine::NativeFast {
                    BrowserJsEngine::HeadlessDomJs
                } else {
                    BrowserJsEngine::NativeFast
                };
                if let Ok(mut g) = self.browser.inner.lock() {
                    g.engine = next;
                }
                self.show_toast(format!("Browser engine: {:?}", next));
            }
            UiAction::BrowserToggleDomInspector => {
                if let Ok(mut g) = self.browser.inner.lock() {
                    g.show_dom_inspector = !g.show_dom_inspector;
                }
            }
            UiAction::BrowserPopOutExternal => {
                let url = self.current_browser_url();
                open_in_external_browser(&url);
                self.show_toast("Opened current page in your system browser.");
            }
            UiAction::ApprovePermission { approved, remember } => {
                if let Some(req) = self.pending_approval.take() {
                    self.permission_gate
                        .resolve(&req.request_id, &req.kind, approved, remember);
                }
            }
            UiAction::ConfirmFileDelete { confirmed } => {
                if let Some(del) = self.pending_delete.take() {
                    if confirmed {
                        self.perform_delete_file(&del.abs_path, &del.rel_path);
                    }
                }
            }
        }
    }

    fn perform_delete_file(&mut self, abs: &Path, rel: &str) {
        let res = if abs.is_dir() {
            std::fs::remove_dir_all(abs)
        } else {
            std::fs::remove_file(abs)
        };
        match res {
            Ok(()) => {
                self.tabs.retain(|t| t.full_path.as_deref() != Some(abs));
                if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                    self.active_tab = self.tabs.len() - 1;
                }
                self.reload_workspace(false);
                self.show_toast(format!("Deleted {}", rel));
            }
            Err(e) => self.show_toast(format!("Delete failed: {}", e)),
        }
    }

    // -----------------------------------------------------------------------
    // Inbuilt Agentic Browser helpers
    // -----------------------------------------------------------------------
    fn current_browser_url(&self) -> String {
        if let Ok(g) = self.browser.inner.lock() {
            g.current_url.clone()
        } else {
            String::new()
        }
    }

    fn browser_engine(&self) -> BrowserJsEngine {
        if let Ok(g) = self.browser.inner.lock() {
            g.engine
        } else {
            BrowserJsEngine::NativeFast
        }
    }
}

// ---------------------------------------------------------------------------
// Keyboard / mouse input pipeline
// ---------------------------------------------------------------------------
impl StudioApp {
    fn active_single_line_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            FocusTarget::NewEntryInput => Some(&mut self.new_entry_name),
            FocusTarget::SearchInput => Some(&mut self.search_query),
            FocusTarget::CommitInput => Some(&mut self.commit_message),
            FocusTarget::FindInput => Some(&mut self.find_query),
            FocusTarget::GotoLineInput => Some(&mut self.goto_line_buf),
            FocusTarget::CommandPaletteInput => Some(&mut self.palette_query),
            FocusTarget::BrowserUrlInput => Some(&mut self.browser_url_buf),
            FocusTarget::ConnectKeyInput => Some(&mut self.connect.key_input),
            FocusTarget::ConnectModelInput => Some(&mut self.connect.model),
            FocusTarget::ConnectUrlInput => Some(&mut self.connect.url_input),
            FocusTarget::McpNameInput => Some(&mut self.new_mcp_name),
            FocusTarget::McpTargetInput => Some(&mut self.new_mcp_target),
            _ => None,
        }
    }

    pub fn handle_key_down(&mut self, key: KeyCode, mods: Modifiers) {
        let modal_blocking = self.modal != ModalDialog::None
            || self.pending_approval.is_some()
            || self.pending_delete.is_some();

        // ---- Command palette input takes priority when open -----------------
        if self.palette_open {
            match key {
                KeyCode::Escape => {
                    self.palette_open = false;
                    self.modal = ModalDialog::None;
                    self.focus = FocusTarget::AgentPrompt;
                    return;
                }
                KeyCode::Up => {
                    self.palette_index = self.palette_index.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    let max = self.palette_entries().len().saturating_sub(1);
                    self.palette_index = (self.palette_index + 1).min(max);
                    return;
                }
                KeyCode::Enter => {
                    let items = self.palette_entries();
                    if let Some(item) = items
                        .get(self.palette_index.min(items.len().saturating_sub(1)))
                        .cloned()
                    {
                        self.apply_palette_item(&item);
                    } else {
                        self.palette_open = false;
                        self.modal = ModalDialog::None;
                    }
                    return;
                }
                KeyCode::Backspace => {
                    self.palette_query.pop();
                    self.palette_index = 0;
                    if self.palette_query.is_empty() {
                        self.palette_open = false;
                        self.modal = ModalDialog::None;
                        self.focus = FocusTarget::AgentPrompt;
                    }
                    return;
                }
                _ => {}
            }
        }

        // ---- Global shortcuts -------------------------------------------------
        if mods.ctrl && mods.shift {
            match key {
                KeyCode::Letter('P') => {
                    self.open_connect_wizard();
                    return;
                }
                KeyCode::Letter('E') => {
                    self.left_tab = LeftTab::Explorer;
                    self.left_panel_open = true;
                    return;
                }
                KeyCode::Letter('F') => {
                    self.left_tab = LeftTab::Search;
                    self.left_panel_open = true;
                    return;
                }
                KeyCode::Letter('G') => {
                    self.left_tab = LeftTab::Git;
                    self.left_panel_open = true;
                    return;
                }
                KeyCode::Letter('S') => {
                    self.left_tab = LeftTab::Sessions;
                    self.left_panel_open = true;
                    return;
                }
                KeyCode::Letter('X') => {
                    self.left_tab = LeftTab::Extensions;
                    self.left_panel_open = true;
                    return;
                }
                KeyCode::Letter('T') => {
                    self.agent_panel_tab = if self.agent_panel_tab == AgentPanelTab::Timeline {
                        AgentPanelTab::Chat
                    } else {
                        AgentPanelTab::Timeline
                    };
                    return;
                }
                KeyCode::Letter('K') => {
                    self.open_command_palette();
                    return;
                }
                _ => {}
            }
        }

        if mods.ctrl {
            match key {
                KeyCode::Letter('K') | KeyCode::Letter('P') => {
                    self.open_command_palette();
                    return;
                }
                KeyCode::Letter('B') => {
                    self.center_view = if self.center_view == CenterView::Browser {
                        CenterView::Editor
                    } else {
                        CenterView::Browser
                    };
                    return;
                }
                KeyCode::Letter('J') => {
                    self.terminal_open = !self.terminal_open;
                    return;
                }
                KeyCode::Letter('G') if !modal_blocking => {
                    self.show_goto_line = !self.show_goto_line;
                    self.focus = if self.show_goto_line {
                        FocusTarget::GotoLineInput
                    } else {
                        FocusTarget::Editor
                    };
                    return;
                }
                _ => {}
            }
            if !modal_blocking {
                match key {
                    KeyCode::Letter('S') => {
                        self.save_active_tab();
                        return;
                    }
                    KeyCode::Letter('O') => {
                        self.execute_action(UiAction::PickFolder);
                        return;
                    }
                    KeyCode::Letter('F') => {
                        if mods.shift {
                            self.left_tab = LeftTab::Search;
                            self.left_panel_open = true;
                        } else {
                            self.show_find_bar = true;
                            self.focus = FocusTarget::FindInput;
                        }
                        return;
                    }
                    KeyCode::Letter('N') => {
                        if mods.shift {
                            self.execute_action(UiAction::NewSession);
                        } else {
                            self.execute_action(UiAction::NewScratchTab);
                        }
                        return;
                    }
                    KeyCode::Letter('W') => {
                        self.execute_action(UiAction::CloseEditorTab(self.active_tab));
                        return;
                    }
                    KeyCode::Tab => {
                        if !self.tabs.is_empty() {
                            let next = (self.active_tab + 1) % self.tabs.len();
                            self.execute_action(UiAction::SelectEditorTab(next));
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        if key == KeyCode::F5 && !modal_blocking {
            self.execute_action(UiAction::RefreshWorkspace);
            return;
        }
        if key == KeyCode::F3 {
            self.execute_action(UiAction::FindNext);
            return;
        }

        if key == KeyCode::Escape {
            if self.shell_view == ShellView::Connect {
                self.shell_view = ShellView::Chat;
                self.modal = ModalDialog::None;
                self.focus = FocusTarget::AgentPrompt;
                return;
            }
            if self.pending_approval.is_some() {
                self.execute_action(UiAction::ApprovePermission {
                    approved: false,
                    remember: false,
                });
                return;
            }
            if self.pending_delete.is_some() {
                self.pending_delete = None;
                return;
            }
            if self.modal != ModalDialog::None {
                self.modal = ModalDialog::None;
                self.show_mcp_add = false;
                return;
            }
            if self.show_find_bar {
                self.show_find_bar = false;
                self.focus = FocusTarget::Editor;
                return;
            }
            if self.show_goto_line {
                self.show_goto_line = false;
                self.focus = FocusTarget::Editor;
                return;
            }
        }

        if modal_blocking {
            // Route simple navigation for modal focus targets
            match key {
                KeyCode::Up | KeyCode::Down => return,
                _ => {}
            }
        }

        // ---- Terminal focus ---------------------------------------------------
        if self.focus == FocusTarget::Terminal && !modal_blocking {
            if let Some(term) = self.terminals.get_mut(self.active_term) {
                if mods.ctrl {
                    match key {
                        KeyCode::Letter('C') => {
                            term.pty.write(&[0x03]);
                            return;
                        }
                        KeyCode::Letter('V') => {
                            if let Some(clip) = platform::clipboard_get_text() {
                                term.pty.write(clip.as_bytes());
                            }
                            return;
                        }
                        KeyCode::Letter('L') => {
                            term.pty.write(&[0x0c]);
                            return;
                        }
                        _ => {}
                    }
                }
                let seq: Option<&[u8]> = match key {
                    KeyCode::Enter => Some(b"\r"),
                    KeyCode::Backspace => Some(b"\x7f"),
                    KeyCode::Tab => Some(b"\t"),
                    KeyCode::Up => Some(b"\x1b[A"),
                    KeyCode::Down => Some(b"\x1b[B"),
                    KeyCode::Right => Some(b"\x1b[C"),
                    KeyCode::Left => Some(b"\x1b[D"),
                    KeyCode::Home => Some(b"\x1b[H"),
                    KeyCode::End => Some(b"\x1b[F"),
                    KeyCode::Delete => Some(b"\x1b[3~"),
                    _ => None,
                };
                if let Some(bytes) = seq {
                    term.screen.scroll_offset = 0;
                    term.pty.write(bytes);
                }
            }
            return;
        }

        // ---- Code editor focus -------------------------------------------------
        if self.focus == FocusTarget::Editor && !modal_blocking && !self.tabs.is_empty() {
            let vis = self.visible_editor_rows();
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                if mods.ctrl {
                    match key {
                        KeyCode::Letter('Z') => {
                            tab.undo();
                            return;
                        }
                        KeyCode::Letter('Y') => {
                            tab.redo();
                            return;
                        }
                        KeyCode::Letter('A') => {
                            tab.select_all();
                            return;
                        }
                        KeyCode::Letter('C') => {
                            if let Some(sel) = tab.selected_text() {
                                platform::clipboard_set_text(&sel);
                            }
                            return;
                        }
                        KeyCode::Letter('X') => {
                            if let Some(sel) = tab.selected_text() {
                                platform::clipboard_set_text(&sel);
                                tab.delete_selection();
                            }
                            return;
                        }
                        KeyCode::Letter('V') => {
                            if let Some(clip) = platform::clipboard_get_text() {
                                tab.insert_str(&clip);
                            }
                            return;
                        }
                        KeyCode::Letter('D') => {
                            tab.select_word_or_next_match(vis);
                            return;
                        }
                        KeyCode::Other(191) => {
                            tab.toggle_comment();
                            return;
                        }
                        _ => {}
                    }
                }
                if mods.alt {
                    match key {
                        KeyCode::Up => {
                            tab.move_line_vertical(-1, vis);
                            return;
                        }
                        KeyCode::Down => {
                            tab.move_line_vertical(1, vis);
                            return;
                        }
                        _ => {}
                    }
                }

                // Smart indentation / unindentation of the selected block
                if key == KeyCode::Tab && tab.ordered_selection().is_some() {
                    tab.indent_block(!mods.shift);
                    return;
                }

                match key {
                    KeyCode::Left => tab.move_cursor(0, -1, mods.shift, vis),
                    KeyCode::Right => tab.move_cursor(0, 1, mods.shift, vis),
                    KeyCode::Up => tab.move_cursor(-1, 0, mods.shift, vis),
                    KeyCode::Down => tab.move_cursor(1, 0, mods.shift, vis),
                    KeyCode::Home => tab.jump_home_end(false, mods.shift),
                    KeyCode::End => tab.jump_home_end(true, mods.shift),
                    KeyCode::PageUp => tab.move_cursor(-(vis as isize), 0, mods.shift, vis),
                    KeyCode::PageDown => tab.move_cursor(vis as isize, 0, mods.shift, vis),
                    KeyCode::Backspace => tab.backspace(),
                    KeyCode::Delete => tab.delete_forward(),
                    KeyCode::Enter => tab.insert_newline_auto_indent(),
                    KeyCode::Tab => tab.insert_str("    "),
                    _ => {}
                }
            }
            return;
        }

        // ---- Prompt composer (reworked) --------------------------------------
        //
        // The old composer hijacked a leading "/" into the command palette, so
        // you could never type "/connect" — which is exactly why auth felt
        // impossible to start. Slash commands now autocomplete inline, and the
        // composer behaves like a real input: history recall, tab-completion,
        // shift+enter newlines and an explicit clear.
        if self.focus == FocusTarget::AgentPrompt && !modal_blocking {
            if mods.ctrl {
                match key {
                    KeyCode::Letter('V') => {
                        if let Some(clip) = platform::clipboard_get_text() {
                            self.agent_prompt.push_str(&clip.replace('\r', ""));
                        }
                        return;
                    }
                    KeyCode::Letter('H') => {
                        self.execute_action(UiAction::ToggleToolBlocks);
                        return;
                    }
                    KeyCode::Letter('L') => {
                        self.agent_prompt.clear();
                        self.history_idx = None;
                        return;
                    }
                    _ => {}
                }
            }
            match key {
                KeyCode::Enter => {
                    if mods.shift {
                        self.agent_prompt.push('\n');
                    } else if !self.agent_prompt.trim().is_empty() {
                        self.execute_action(UiAction::SendAgentPrompt);
                    }
                }
                KeyCode::Backspace => {
                    self.agent_prompt.pop();
                    self.history_idx = None;
                }
                KeyCode::Tab => {
                    // Accept the highlighted autocomplete entry.
                    if !self.shell_slash_buf.is_empty() {
                        let (cmd, _) = self.shell_slash_buf[0];
                        let mut base = cmd.to_string();
                        if !base.ends_with(' ') {
                            base.push(' ');
                        }
                        self.agent_prompt = base;
                    } else if !self.shell_file_buf.is_empty() {
                        let f = self.shell_file_buf[0].clone();
                        let cut = self
                            .agent_prompt
                            .rfind('@')
                            .map(|i| i + 1)
                            .unwrap_or(self.agent_prompt.len());
                        self.agent_prompt.truncate(cut);
                        self.agent_prompt.push_str(&f);
                        self.agent_prompt.push(' ');
                    }
                }
                KeyCode::Up => {
                    self.recall_prompt_history(-1);
                }
                KeyCode::Down => {
                    self.recall_prompt_history(1);
                }
                KeyCode::Escape => {
                    if !self.agent_prompt.is_empty() {
                        self.agent_prompt.clear();
                        self.history_idx = None;
                    }
                }
                _ => {}
            }
            return;
        }

        // ---- Browser URL bar ----------------------------------------------------
        if self.focus == FocusTarget::BrowserUrlInput && !modal_blocking {
            match key {
                KeyCode::Enter => self.execute_action(UiAction::BrowserGoUrl),
                KeyCode::Escape => self.focus = FocusTarget::Editor,
                _ => {}
            }
        }

        // ---- Single-line inputs -------------------------------------------------
        let current_focus = self.focus;
        if let Some(line) = self.active_single_line_mut() {
            if mods.ctrl {
                match key {
                    KeyCode::Letter('V') => {
                        if let Some(clip) = platform::clipboard_get_text() {
                            let clean: String =
                                clip.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                            line.push_str(&clean);
                        }
                        return;
                    }
                    KeyCode::Letter('U') => {
                        line.clear();
                        return;
                    }
                    _ => {}
                }
            }
            match key {
                KeyCode::Backspace => {
                    line.pop();
                }
                KeyCode::Enter => match current_focus {
                    FocusTarget::SearchInput => self.execute_action(UiAction::RunSearch),
                    FocusTarget::NewEntryInput => {
                        self.execute_action(UiAction::ConfirmCreateEntry)
                    }
                    FocusTarget::CommitInput => self.execute_action(UiAction::GitCommit),
                    FocusTarget::FindInput => self.execute_action(UiAction::FindNext),
                    FocusTarget::GotoLineInput => self.execute_action(UiAction::ConfirmGotoLine),
                    FocusTarget::BrowserUrlInput => self.execute_action(UiAction::BrowserGoUrl),
                    FocusTarget::ConnectKeyInput => self.execute_action(UiAction::SaveConnectWizard),
                    _ => {}
                },
                KeyCode::Escape => {
                    if current_focus == FocusTarget::NewEntryInput {
                        self.creating_entry = None;
                    }
                }
                _ => {}
            }
        }
    }

    pub fn handle_char_input(&mut self, ch: char) {
        if ch < ' ' {
            return;
        }
        let modal_blocking = self.modal != ModalDialog::None
            || self.pending_approval.is_some()
            || self.pending_delete.is_some();

        if self.palette_open {
            self.palette_query.push(ch);
            self.palette_index = 0;
            return;
        }

        if self.focus == FocusTarget::Terminal && !modal_blocking {
            if let Some(term) = self.terminals.get_mut(self.active_term) {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                term.screen.scroll_offset = 0;
                term.pty.write(s.as_bytes());
            }
            return;
        }

        if self.focus == FocusTarget::Editor && !modal_blocking {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.insert_smart_char(ch);
            }
            return;
        }

        if self.focus == FocusTarget::AgentPrompt && !modal_blocking {
            // Slash commands autocomplete inline (see shell::prompt); the leading
            // "/" is just text now.
            self.agent_prompt.push(ch);
            self.history_idx = None;
            return;
        }

        if let Some(line) = self.active_single_line_mut() {
            line.push(ch);
        }
    }

    fn editor_hit_to_cursor(
        &self,
        mx: i32,
        my: i32,
        lines: &[String],
        scroll_y: usize,
        scroll_x: usize,
    ) -> (usize, usize) {
        let gutter_w = 58;
        let line_h = 20;
        let char_w = 8;
        let rel_y = (my - self.editor_rect.y - 6).max(0) as usize;
        let rel_x = (mx - (self.editor_rect.x + gutter_w + 8)).max(0) as usize;
        let row = (scroll_y + rel_y / line_h).min(lines.len().saturating_sub(1));
        let line_len = lines.get(row).map(|l| char_len(l)).unwrap_or(0);
        let col = (scroll_x + (rel_x + char_w / 2) / char_w).min(line_len);
        (row, col)
    }

    fn compute_cursor(&self) -> CursorShape {
        if self.modal != ModalDialog::None
            || self.pending_approval.is_some()
            || self.pending_delete.is_some()
        {
            return CursorShape::Arrow;
        }
        let top = SHELL_CHROME_TOP;
        let bottom = self.height - SHELL_CHROME_BOTTOM;
        if self.mouse_y >= top && self.mouse_y < bottom {
            let panel_x = if self.left_panel_open {
                RAIL_W + self.left_w
            } else {
                RAIL_W
            };
            if (self.mouse_x - panel_x).abs() <= 4
                || (self.mouse_x - (self.width - self.right_w)).abs() <= 4
            {
                return CursorShape::ResizeH;
            }
            if self.terminal_open {
                let term_top = bottom - self.term_h;
                if self.mouse_x > panel_x
                    && self.mouse_x < self.width - self.right_w
                    && (self.mouse_y - term_top).abs() <= 4
                {
                    return CursorShape::ResizeV;
                }
            }
        }
        if self.center_view == CenterView::Browser && self.browser_rect.contains(self.mouse_x, self.mouse_y) {
            return CursorShape::Hand;
        }
        if self.center_view == CenterView::Editor
            && self.editor_rect.contains(self.mouse_x, self.mouse_y)
            && !self.tabs.is_empty()
        {
            return CursorShape::IBeam;
        }
        for z in &self.hit_zones {
            if z.rect.contains(self.mouse_x, self.mouse_y) {
                return match z.action {
                    UiAction::Focus(_) => CursorShape::IBeam,
                    _ => CursorShape::Hand,
                };
            }
        }
        CursorShape::Arrow
    }
}

// ---------------------------------------------------------------------------
// Rendering pipeline — 100% Pure-Rust double-buffered pixel + ClearType renderer
// ---------------------------------------------------------------------------
impl StudioApp {
    fn push_hit(&mut self, rect: UiRect, action: UiAction) {
        self.hit_zones.push(HitZone { rect, action });
    }

    fn draw_button(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        rect: UiRect,
        label: &str,
        bg: u32,
        fg: u32,
        border: u32,
        action: Option<UiAction>,
    ) {
        if rect.w <= 0 || rect.h <= 0 {
            return;
        }
        let hovered = rect.contains(self.mouse_x, self.mouse_y);
        let fill = if hovered && action.is_some() {
            lighten(bg, 16)
        } else {
            bg
        };
        canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 6, fill);
        canvas.stroke_rounded_rect(rect.x, rect.y, rect.w, rect.h, 6, border);
        let tw = canvas.measure_text_width(label, FontRole::UiSemibold);
        let tx = rect.x + ((rect.w - tw) / 2).max(5);
        let ty = rect.y + ((rect.h - 15) / 2).max(2);
        canvas.draw_text_clipped(tx, ty, rect.w - 8, label, FontRole::UiSemibold, fg);
        if let Some(act) = action {
            self.push_hit(rect, act);
        }
    }

    fn draw_pill(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        rect: UiRect,
        label: &str,
        active: bool,
        action: Option<UiAction>,
    ) {
        let p = self.palette.clone();
        let bg = if active {
            mix(p.accent, p.bg_card, 0.22)
        } else {
            p.bg_card
        };
        let fg = if active { p.accent } else { p.text_secondary };
        self.draw_button(
            canvas,
            rect,
            label,
            bg,
            fg,
            if active { p.accent } else { p.border_subtle },
            action,
        );
    }

    fn draw_text_input(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        rect: UiRect,
        value: &str,
        placeholder: &str,
        focused: bool,
        font: FontRole,
        action: UiAction,
    ) {
        let p = self.palette.clone();
        canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 6, p.bg_input);
        canvas.stroke_rounded_rect(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            6,
            if focused { p.border_focus } else { p.border_subtle },
        );
        let shown = if focused && self.blink_phase {
            format!("{}|", value)
        } else {
            value.to_string()
        };
        let (text, color) = if value.is_empty() && !focused {
            (placeholder.to_string(), p.text_muted)
        } else {
            (shown, p.text_primary)
        };
        canvas.draw_text_clipped(rect.x + 9, rect.y + 5, rect.w - 16, &text, font, color);
        self.push_hit(rect, action);
    }

    fn draw_section_title(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        y: i32,
        w: i32,
        label: &str,
    ) {
        let p = self.palette.clone();
        canvas.draw_text_clipped(x, y, w, label, FontRole::UiSmall, p.text_muted);
    }

    // -----------------------------------------------------------------------
    // Header
    // -----------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // OpenCode-style shell rendering
    // -----------------------------------------------------------------------

    /// Session tabs for the top strip (active session first).
    fn shell_tabs(&self) -> Vec<SessionTab> {
        let mut tabs: Vec<SessionTab> = Vec::with_capacity(self.sessions.len() + 1);
        let active_id = self.active_session.id.clone();
        tabs.push(SessionTab {
            id: active_id.clone(),
            title: if self.active_session.title.is_empty() {
                "New session".to_string()
            } else {
                self.active_session.title.clone()
            },
            active: true,
        });
        for s in self.sessions.iter().take(7) {
            if s.id == active_id {
                continue;
            }
            tabs.push(SessionTab {
                id: s.id.clone(),
                title: s.title.clone(),
                active: false,
            });
        }
        tabs
    }

    /// Rough token accounting for the context readout: ~4 chars per token over
    /// the whole transcript plus the live buffers.
    fn shell_token_estimate(&self) -> u64 {
        let mut chars: u64 = 0;
        for m in self.active_session.messages.iter() {
            chars += m.content.chars().count() as u64;
            chars += m.thinking.chars().count() as u64 / 2;
            for t in m.tools.iter() {
                chars += t.result_summary.chars().count() as u64 / 2;
                chars += t.args_json.chars().count() as u64 / 4;
            }
        }
        chars += self.streaming_text.chars().count() as u64;
        chars += self.streaming_thinking.chars().count() as u64 / 2;
        chars / 4
    }

    /// MCP / todo / modified-file lists, as the sidebar shows them.
    ///
    /// Headless front-ends cannot borrow the per-frame scratch buffers the GUI
    /// fills during `render()`, so they get the values computed fresh.
    fn shell_sidecar_lists(&self) -> (Vec<(String, String)>, Vec<String>, Vec<(String, usize, usize)>) {
        (
            self.shell_mcp_status(),
            self.shell_todos(),
            self.shell_modified_files(),
        )
    }

    /// True when the `/connect` screen is what the user is looking at.
    fn connect_screen_open(&self) -> bool {
        matches!(self.shell_view, crate::ui::shell::ShellView::Connect)
    }

    fn shell_token_pct(&self) -> f32 {
        // A 200K-token context window is the common denominator across providers.
        (self.shell_token_estimate() as f32 / 200_000.0).clamp(0.0, 1.0)
    }

    fn shell_cost(&self) -> f64 {
        // Rough blended estimate; providers are not queried per token.
        self.shell_token_estimate() as f64 * 0.000_003
    }

    fn shell_slash_hits(&self) -> Vec<(&'static str, &'static str)> {
        shell::prompt::slash_hits(&self.agent_prompt)
    }

    fn shell_file_hits(&self) -> Vec<String> {
        let q = shell::prompt::at_query(&self.agent_prompt, self.agent_prompt.chars().count());
        let Some(q) = q else { return Vec::new() };
        self.entries
            .iter()
            .filter(|e| !e.is_dir)
            .filter(|e| q.is_empty() || e.rel_path.to_ascii_lowercase().contains(&q))
            .take(8)
            .map(|e| e.rel_path.clone())
            .collect()
    }

    fn shell_palette_items(&self) -> Vec<PaletteItem> {
        let files: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .take(900)
            .map(|e| (e.rel_path.clone(), e.name.clone()))
            .collect();
        shell::palette::items_for(
            &self.palette_query,
            self.settings.show_thinking,
            &self.active_mode,
            &self.settings.agent_effort,
            &self.settings.theme,
            &files,
        )
    }

    /// Modified files for the sidebar: git changes plus any dirty editor tabs.
    /// The +/- counts come from `git diff --numstat` when the workspace is a repo.
    fn shell_modified_files(&self) -> Vec<(String, usize, usize)> {
        let numstat = crate::git_ops::run_git(&self.workspace, &["diff", "--numstat"])
            .ok()
            .map(|(_, out, _)| out)
            .unwrap_or_default();
        let mut counts: std::collections::HashMap<String, (usize, usize)> =
            std::collections::HashMap::new();
        for line in numstat.lines() {
            let mut it = line.split('\t');
            let add = it.next().unwrap_or("0").parse::<usize>().unwrap_or(0);
            let del = it.next().unwrap_or("0").parse::<usize>().unwrap_or(0);
            if let Some(path) = it.next() {
                counts.insert(path.to_string(), (add, del));
            }
        }
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for f in self.git_snapshot.files.iter() {
            let (a, d) = counts.get(&f.path).copied().unwrap_or((0, 0));
            out.push((f.path.clone(), a, d));
        }
        if out.is_empty() {
            for t in self.tabs.iter() {
                if t.is_dirty() {
                    out.push((t.rel_path.clone(), 0, 0));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn shell_todos(&self) -> Vec<String> {
        // Surface the plan's todo lines when the assistant writes a checklist.
        let mut out: Vec<String> = Vec::new();
        if let Some(m) = self.active_session.messages.last() {
            for line in m.content.lines() {
                let t = line.trim();
                if (t.starts_with("- [ ]") || t.starts_with("* [ ]"))
                    && out.len() < 12
                {
                    out.push(t.trim_start_matches("- [ ]").trim_start_matches("* [ ]").trim().to_string());
                }
            }
        }
        out
    }

    fn shell_mcp_status(&self) -> Vec<(String, String)> {
        self.mcp_servers
            .iter()
            .map(|m| (m.name.clone(), m.status.clone()))
            .collect()
    }

    fn draw_shell_chrome(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let w = self.width;
        let h = self.height;
        canvas.fill_rect(0, 0, w, shell::TABSTRIP_H, self.palette.bg_header);
        canvas.draw_hline(0, shell::TABSTRIP_H, w, self.palette.border_subtle);

        let tabs = self.shell_tabs();
        let mut tx = 8;
        for (i, tab) in tabs.iter().enumerate() {
            let label = shell::truncate_right(&tab.title, 18);
            let tw = canvas.measure_text_width(&label, FontRole::UiSmall) + 22;
            let selected = tab.active;
            canvas.fill_rounded_rect(
                tx,
                5,
                tw,
                shell::TABSTRIP_H - 12,
                5,
                if selected { self.palette.bg_card } else { self.palette.bg_header },
            );
            canvas.draw_text(
                tx + 11,
                11,
                &label,
                FontRole::UiSmall,
                if selected { self.palette.text_primary } else { self.palette.text_muted },
            );
            self.push_hit(
                UiRect::new(tx, 5, tw, shell::TABSTRIP_H - 12),
                UiAction::SelectSessionTab(i),
            );
            tx += tw + 6;
            if tx > w - 120 {
                break;
            }
        }
        canvas.fill_rounded_rect(tx, 5, 26, shell::TABSTRIP_H - 12, 5, self.palette.bg_header);
        canvas.draw_text(tx + 8, 11, "+", FontRole::UiSemibold, self.palette.text_secondary);
        self.push_hit(UiRect::new(tx, 5, 26, shell::TABSTRIP_H - 12), UiAction::NewSession);

        let switch_label = if self.shell_view == ShellView::Workspace {
            "Chat"
        } else {
            "Workspace"
        };
        let sw_w = canvas.measure_text_width(switch_label, FontRole::UiSmall) + 20;
        canvas.fill_rounded_rect(w - sw_w - 10, 5, sw_w, shell::TABSTRIP_H - 12, 5, self.palette.bg_button);
        canvas.draw_text(w - sw_w - 2, 11, switch_label, FontRole::UiSmall, self.palette.text_secondary);
        self.push_hit(
            UiRect::new(w - sw_w - 10, 5, sw_w, shell::TABSTRIP_H - 12),
            UiAction::ToggleShellView,
        );

        let sy = h - shell::STATUSBAR_H;
        canvas.fill_rect(0, sy, w, shell::STATUSBAR_H, self.palette.bg_header);
        canvas.draw_hline(0, sy, w, self.palette.border_subtle);
        let pct = (self.shell_token_pct() * 100.0) as i32;
        let left = format!(
            "[ctrl+? help]  Tokens: {} ({}%)  Cost: {}",
            shell::fmt_tokens(self.shell_token_estimate()),
            pct,
            shell::fmt_cost(self.shell_cost())
        );
        canvas.draw_text(10, sy + 6, &left, FontRole::UiSmall, self.palette.text_secondary);
        if self.shell_flash.visible() {
            let msg = shell::truncate_right(&self.shell_flash.text, 60);
            let mw = canvas.measure_text_width(&msg, FontRole::UiSmall);
            canvas.draw_text(w / 2 - mw / 2, sy + 6, &msg, FontRole::UiSmall, self.palette.accent);
        }
        let ws = shell::shorten_path(&self.workspace.to_string_lossy(), 40);
        let ws_w = canvas.measure_text_width(&ws, FontRole::UiSmall);
        canvas.draw_text(w - ws_w - 10, sy + 6, &ws, FontRole::UiSmall, self.palette.text_muted);
        let model_box = format!("{} | {}", self.settings.active_model, self.settings.agent_effort);
        let mb_w = canvas.measure_text_width(&model_box, FontRole::UiSmall) + 18;
        let mb_x = w - ws_w - mb_w - 26;
        canvas.fill_rounded_rect(mb_x, sy + 3, mb_w, shell::STATUSBAR_H - 6, 4, self.palette.accent);
        canvas.draw_text(
            mb_x + 9,
            sy + 6,
            &model_box,
            FontRole::UiSmall,
            self.palette.on_accent_text(self.palette.accent),
        );
        canvas.draw_text(
            mb_x - 78,
            sy + 6,
            &format!("{} v{}", self.settings.active_provider, crate::VERSION),
            FontRole::UiSmall,
            self.palette.text_muted,
        );
        self.push_hit(
            UiRect::new(mb_x, sy, mb_w, shell::STATUSBAR_H),
            UiAction::CycleModel,
        );
    }

    /// Recomputes the scratch buffers the shell models borrow from.
    ///
    /// The GUI calls this once per frame. Headless front-ends (TUI, web) have no
    /// frame, so `tick()` calls it instead — without it, Tab-completion in the
    /// composer silently does nothing because `shell_slash_buf` stays empty.
    pub fn refresh_shell_scratch(&mut self) {
        self.shell_mcp_status_buf = self.shell_mcp_status();
        self.shell_todos_buf = self.shell_todos();
        self.shell_modified_buf = self.shell_modified_files();
        self.shell_slash_buf = self.shell_slash_hits();
        self.shell_file_buf = self.shell_file_hits();
        self.shell_palette_buf = self.shell_palette_items();
        self.shell_zen_buf = self.connect.zen_free_models.clone();
    }

    fn render_shell(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let w = self.width;
        let h = self.height;

        // Resolve the effective scroll offset: pinned means "bottom".
        let row_count = shell::chat::build_rows(&self.shell_chat_model(), ((w / 8).max(20)) as usize).len();
        let visible_rows =
            ((h - shell::TABSTRIP_H - shell::STATUSBAR_H - 58) / 20).max(3) as usize;
        if self.chat_pinned {
            self.chat_scroll = row_count.saturating_sub(visible_rows);
        } else {
            self.chat_scroll = self.chat_scroll.min(row_count.saturating_sub(visible_rows));
        }
        let effective_scroll = self.chat_scroll;

        self.refresh_shell_scratch();

        let tabs = self.shell_tabs();
        let sidebar_model = if self.sidebar_open && w > 900 {
            Some(self.shell_sidebar_model())
        } else {
            None
        };
        let prompt_model = self.shell_prompt_model();
        let palette_model = if self.palette_open {
            Some(self.shell_palette_model())
        } else {
            None
        };
        let connect_model = if self.shell_view == ShellView::Connect {
            Some(self.shell_connect_model())
        } else {
            None
        };

        let frame = shell::ShellFrame {
            width: w,
            height: h,
            palette: &self.palette,
            view: self.shell_view,
            tabs: &tabs,
            active_tab: 0,
            chat: self.shell_chat_model(),
            chat_scroll: effective_scroll,
            sidebar: sidebar_model,
            prompt: prompt_model,
            model_label: &self.settings.active_model,
            provider_label: &self.settings.active_provider,
            effort_label: &self.settings.agent_effort,
            tokens: self.shell_token_estimate(),
            token_pct: self.shell_token_pct(),
            cost_usd: self.shell_cost(),
            running: self.agent_running,
            flash: if self.shell_flash.visible() {
                Some(&self.shell_flash)
            } else {
                None
            },
            workspace_path: &self.workspace.to_string_lossy(),
            version: crate::VERSION,
            command_palette: palette_model,
            connect: connect_model,
        };

        let zones = shell::render(canvas, &frame);
        for (rect, action) in zones {
            self.push_hit(rect, action);
        }

        // Overlays that only exist in Chat mode.
        if self.shell_view == ShellView::Chat {
            if self.modal == ModalDialog::ThemePicker {
                self.draw_theme_picker(canvas);
            } else if self.modal == ModalDialog::SafetySettings {
                self.draw_safety_modal(canvas);
            }
        }
        if let Some(req) = self.pending_approval.clone() {
            self.draw_approval_modal(canvas, &req);
        }
        if let Some(del) = self.pending_delete.clone() {
            self.draw_delete_modal(canvas, &del);
        }
    }

    fn shell_chat_model(&self) -> shell::chat::ChatModel<'_> {
        shell::chat::ChatModel {
            session: &self.active_session,
            streaming_text: self.streaming_text.as_str(),
            streaming_thinking: self.streaming_thinking.as_str(),
            live_tools: self.live_tools.as_slice(),
            running: self.agent_running,
            status_line: self.agent_status_line.as_str(),
            show_thinking: self.settings.show_thinking,
            expanded: self.expanded_blocks.as_slice(),
            active_model: self.settings.active_model.as_str(),
            active_mode: self.active_mode.as_str(),
            effort: self.settings.agent_effort.as_str(),
        }
    }

    fn shell_sidebar_model(&self) -> shell::sidebar::SidebarModel<'_> {
        shell::sidebar::SidebarModel {
            session: &self.active_session,
            tokens: self.shell_token_estimate(),
            token_pct: self.shell_token_pct(),
            cost_usd: self.shell_cost(),
            mcp_servers: self.mcp_servers.as_slice(),
            mcp_status: &self.shell_mcp_status_buf,
            todos: &self.shell_todos_buf,
            modified: &self.shell_modified_buf,
            collapsed: self.sidebar_collapsed.as_slice(),
            version: crate::VERSION,
        }
    }

    fn shell_prompt_model(&self) -> shell::prompt::PromptModel<'_> {
        shell::prompt::PromptModel {
            value: self.agent_prompt.as_str(),
            caret: self.agent_prompt.chars().count(),
            focused: self.focus == FocusTarget::AgentPrompt,
            caret_visible: self.blink_phase,
            history: self.prompt_history.as_slice(),
            slash_hits: self.shell_slash_buf.as_slice(),
            file_hits: self.shell_file_buf.as_slice(),
            ac_index: 0,
            running: self.agent_running,
        }
    }

    fn shell_palette_model(&self) -> shell::palette::PaletteModel<'_> {
        shell::palette::PaletteModel {
            query: self.palette_query.as_str(),
            index: self.palette_index,
            items: self.shell_palette_buf.as_slice(),
            caret_visible: self.blink_phase,
        }
    }

    fn shell_connect_model(&self) -> shell::connect::ConnectModel<'_> {
        shell::connect::ConnectModel {
            provider: self.connect.provider_id.as_str(),
            model: self.connect.model.as_str(),
            key: self.connect.key_input.as_str(),
            url: self.connect.url_input.as_str(),
            auth_mode: self.connect.opencode_auth_mode.as_str(),
            zen_models: self.shell_zen_buf.as_slice(),
            zen_status: self.connect.zen_status.as_str(),
            detected: self.connect.detected_summary.as_slice(),
            detected_file: &self.connect.detected_opencode_file,
            local_server_alive: self.connect.local_server_alive,
            ollama_alive: self.connect.ollama_alive,
            focus_key: self.focus == FocusTarget::ConnectKeyInput,
            caret_visible: self.blink_phase,
            scroll: 0,
        }
    }


    // -----------------------------------------------------------------------
    // Left activity rail + panels
    // -----------------------------------------------------------------------
    fn draw_left_rail(&mut self, canvas: &mut dyn SurfaceCanvas, top: i32, bottom: i32) {
        let p = self.palette.clone();
        canvas.fill_rect(0, top, RAIL_W, bottom - top, p.bg_rail);
        canvas.draw_vline(RAIL_W - 1, top, bottom - top, p.border_subtle);

        let git_badge = self.git_snapshot.files.len();
        let items: Vec<(LeftTab, &str, usize)> = vec![
            (LeftTab::Explorer, "files", 0),
            (LeftTab::Search, "search", 0),
            (LeftTab::Git, "branch", git_badge),
            (LeftTab::Sessions, "chat", self.sessions.len()),
            (LeftTab::Extensions, "puzzle", self.skills.len() + self.mcp_servers.len()),
        ];
        let mut y = top + PAD;
        for (tab, glyph_name, badge) in items {
            let rect = UiRect::new(6, y, RAIL_W - 12, 44);
            let active = self.left_tab == tab && self.left_panel_open;
            if active {
                // A quiet 3px tint bar reads as "selected" without a filled slab.
                canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 8, mix(p.accent, p.bg_rail, 0.07));
                canvas.fill_rect(rect.x, rect.y + 6, 3, rect.h - 12, mix(p.accent, p.bg_rail, 0.55));
            }
            let hovered = rect.contains(self.mouse_x, self.mouse_y);
            if !active && hovered {
                canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 8, mix(p.accent, p.bg_rail, 0.04));
            }
            let fg = if active { p.accent } else { p.text_muted };
            canvas.draw_text(rect.x + 16, rect.y + 6, glyph(glyph_name), FontRole::UiTitle, fg);
            let label = match tab {
                LeftTab::Explorer => "Files",
                LeftTab::Search => "Search",
                LeftTab::Git => "Git",
                LeftTab::Sessions => "Chats",
                LeftTab::Extensions => "Tools",
            };
            canvas.draw_text_clipped(rect.x + 2, rect.y + 26, rect.w - 4, label, FontRole::UiSmall, fg);
            if badge > 0 {
                let bl = if badge > 99 { "99+".to_string() } else { badge.to_string() };
                let bw = canvas.measure_text_width(&bl, FontRole::UiSmall) + 10;
                canvas.fill_rounded_rect(
                    rect.x + rect.w - bw - 2,
                    rect.y + 2,
                    bw,
                    14,
                    7,
                    mix(p.accent_secondary, p.bg_rail, 0.22),
                );
                canvas.draw_text(
                    rect.x + rect.w - bw + 3,
                    rect.y + 3,
                    &bl,
                    FontRole::UiSmall,
                    p.text_secondary,
                );
            }
            self.push_hit(rect, UiAction::ToggleRailTab(tab));
            y += 54;
        }

        // Bottom toggles: terminal + browser
        let mut by = bottom - PAD - 44;
        let term_rect = UiRect::new(6, by, RAIL_W - 12, 40);
        canvas.fill_rounded_rect(
            term_rect.x,
            term_rect.y,
            term_rect.w,
            term_rect.h,
            8,
            if self.terminal_open {
                mix(p.accent, p.bg_rail, 0.10)
            } else {
                p.bg_rail
            },
        );
        canvas.draw_text(
            term_rect.x + 16,
            term_rect.y + 4,
            glyph("term"),
            FontRole::UiTitle,
            if self.terminal_open { p.accent } else { p.text_muted },
        );
        canvas.draw_text(
            term_rect.x + 2,
            term_rect.y + 24,
            "Term",
            FontRole::UiSmall,
            p.text_muted,
        );
        self.push_hit(term_rect, UiAction::ToggleTerminalDrawer);
        by -= 48;

        let br_rect = UiRect::new(6, by, RAIL_W - 12, 40);
        canvas.fill_rounded_rect(
            br_rect.x,
            br_rect.y,
            br_rect.w,
            br_rect.h,
            8,
            if self.center_view == CenterView::Browser {
                mix(p.accent, p.bg_rail, 0.10)
            } else {
                p.bg_rail
            },
        );
        canvas.draw_text(
            br_rect.x + 16,
            br_rect.y + 4,
            glyph("globe"),
            FontRole::UiTitle,
            if self.center_view == CenterView::Browser {
                p.accent
            } else {
                p.text_muted
            },
        );
        canvas.draw_text(br_rect.x + 2, br_rect.y + 24, "Web", FontRole::UiSmall, p.text_muted);
        self.push_hit(br_rect, UiAction::SwitchCenterView(CenterView::Browser));
    }

    fn draw_left_panel(&mut self, canvas: &mut dyn SurfaceCanvas, top: i32, bottom: i32) {
        let p = self.palette.clone();
        let x = RAIL_W;
        let w = self.left_w;
        canvas.fill_rect(x, top, w, bottom - top, p.bg_sidebar);
        canvas.draw_vline(x + w - 1, top, bottom - top, p.border_subtle);

        let title = match self.left_tab {
            LeftTab::Explorer => "EXPLORER",
            LeftTab::Search => "SEARCH",
            LeftTab::Git => "SOURCE CONTROL",
            LeftTab::Sessions => "SESSIONS",
            LeftTab::Extensions => "SKILLS · SUB-AGENTS · MCP",
        };
        self.draw_section_title(canvas, x + PAD, top + PAD, w - 24, title);
        let body_top = top + PAD + 20;
        match self.left_tab {
            LeftTab::Explorer => self.draw_explorer_panel(canvas, body_top, bottom, w),
            LeftTab::Search => self.draw_search_panel(canvas, body_top, bottom, w),
            LeftTab::Git => self.draw_git_panel(canvas, body_top, bottom, w),
            LeftTab::Sessions => self.draw_sessions_panel(canvas, body_top, bottom, w),
            LeftTab::Extensions => self.draw_extensions_panel(canvas, body_top, bottom, w),
        }
    }
}

// ---------------------------------------------------------------------------
// Left panels
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_explorer_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        let p = self.palette.clone();
        let x = RAIL_W;

        // Drives / quick workspace switches
        let mut dx = x + PAD;
        for drv in self.drives.clone().into_iter().take(4) {
            let bw = 52;
            if dx + bw > x + w - PAD {
                break;
            }
            let active = self
                .workspace
                .to_string_lossy()
                .to_uppercase()
                .starts_with(&drv.letter.to_uppercase());
            self.draw_pill(
                canvas,
                UiRect::new(dx, y, bw, 24),
                &drv.letter,
                active,
                Some(UiAction::SwitchDrive(drv.mount_path.clone())),
            );
            dx += bw + 6;
        }
        y += 30;

        let bw = (w - PAD * 2 - 12) / 3;
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, y, bw, 26),
            "+ File",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::StartCreateEntry { is_dir: false }),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + bw + 6, y, bw, 26),
            "+ Folder",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::StartCreateEntry { is_dir: true }),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + (bw + 6) * 2, y, bw, 26),
            "Refresh",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::RefreshWorkspace),
        );
        y += 34;

        if let Some(is_dir) = self.creating_entry {
            let card = UiRect::new(x + PAD, y, w - PAD * 2, 62);
            canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, CARD_RADIUS, p.bg_card);
            canvas.stroke_rounded_rect(card.x, card.y, card.w, card.h, CARD_RADIUS, p.accent);
            canvas.draw_text(
                card.x + 10,
                card.y + 7,
                if is_dir { "New folder path" } else { "New file path" },
                FontRole::UiSmall,
                p.accent,
            );
            let inp = UiRect::new(card.x + 8, card.y + 26, card.w - 78, 26);
            self.draw_text_input(
                canvas,
                inp,
                self.new_entry_name.clone().as_str(),
                "src/module.rs",
                self.focus == FocusTarget::NewEntryInput,
                FontRole::TerminalMono,
                UiAction::Focus(FocusTarget::NewEntryInput),
            );
            self.draw_button(
                canvas,
                UiRect::new(card.x + card.w - 66, card.y + 26, 28, 26),
                "OK",
                p.success,
                p.on_accent_text(p.success),
                p.success,
                Some(UiAction::ConfirmCreateEntry),
            );
            self.draw_button(
                canvas,
                UiRect::new(card.x + card.w - 34, card.y + 26, 26, 26),
                "X",
                p.bg_button,
                p.text_secondary,
                p.border_subtle,
                Some(UiAction::CancelCreateEntry),
            );
            y += 70;
        }

        canvas.draw_hline(x, y - 4, w - 1, p.border_subtle);

        // Roomier rows: a calm file list beats a dense one.
        let row_h = 31;
        let avail_rows = ((bottom - y - 6).max(0) / row_h) as usize;
        let max_scroll = self.entries.len().saturating_sub(avail_rows.max(1));
        if self.explorer_scroll > max_scroll {
            self.explorer_scroll = max_scroll;
        }
        let active_abs = self.tabs.get(self.active_tab).and_then(|t| t.full_path.clone());
        let visible: Vec<FileEntry> = self
            .entries
            .iter()
            .skip(self.explorer_scroll)
            .take(avail_rows)
            .cloned()
            .collect();

        canvas.push_clip(x, y, w - 2, bottom - y);
        for (i, entry) in visible.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let row_rect = UiRect::new(x + 4, ry, w - 10, row_h - 2);
            let is_open = active_abs.as_ref() == Some(&entry.path);
            let hovered = row_rect.contains(self.mouse_x, self.mouse_y);
            if is_open {
                canvas.fill_rounded_rect(row_rect.x, row_rect.y, row_rect.w, row_rect.h, 5, mix(p.accent, p.bg_sidebar, 0.20));
            } else if hovered {
                canvas.fill_rounded_rect(row_rect.x, row_rect.y, row_rect.w, row_rect.h, 5, p.bg_card);
            }
            let indent = x + PAD + (entry.depth.min(8) as i32) * 13;
            if entry.is_dir {
                let collapsed = self.collapsed_dirs.contains(&entry.rel_path);
                canvas.draw_text(
                    indent,
                    ry + 4,
                    if collapsed { glyph("chev_r") } else { glyph("chev_d") },
                    FontRole::TerminalMono,
                    p.accent_secondary,
                );
                canvas.draw_text_clipped(
                    indent + 15,
                    ry + 4,
                    w - (indent + 15 - x) - 26,
                    &entry.name,
                    FontRole::UiSemibold,
                    p.text_primary,
                );
                self.push_hit(row_rect, UiAction::ToggleDir(entry.rel_path));
            } else {
                let fam = crate::syntax::LangFamily::from_path(&entry.name);
                canvas.draw_text(indent, ry + 5, fam.badge(), FontRole::TerminalMono, fam_color(fam, &p));
                canvas.draw_text_clipped(
                    indent + 26,
                    ry + 4,
                    w - (indent + 26 - x) - 26,
                    &entry.name,
                    FontRole::UiSmall,
                    if is_open { p.text_primary } else { p.text_secondary },
                );
                self.push_hit(
                    row_rect,
                    UiAction::OpenFile(entry.path.clone(), entry.rel_path.clone(), entry.name.clone(), false),
                );
            }
        }
        canvas.pop_clip();
    }

    fn draw_search_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        let p = self.palette.clone();
        let x = RAIL_W;
        let inp = UiRect::new(x + PAD, y, w - PAD * 2 - 64, 28);
        self.draw_text_input(
            canvas,
            inp,
            self.search_query.clone().as_str(),
            "Search in workspace...",
            self.focus == FocusTarget::SearchInput,
            FontRole::UiSmall,
            UiAction::Focus(FocusTarget::SearchInput),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - PAD - 58, y, 58, 28),
            "Find",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::RunSearch),
        );
        y += 38;

        canvas.draw_text(
            x + PAD,
            y,
            &format!("{} results", self.search_results.len()),
            FontRole::UiSmall,
            p.text_muted,
        );
        y += 20;

        let row_h = 46;
        let avail = ((bottom - y - 6).max(0) / row_h) as usize;
        let max_scroll = self.search_results.len().saturating_sub(avail.max(1));
        if self.search_scroll > max_scroll {
            self.search_scroll = max_scroll;
        }
        let visible: Vec<SearchMatch> = self
            .search_results
            .iter()
            .skip(self.search_scroll)
            .take(avail)
            .cloned()
            .collect();

        canvas.push_clip(x, y, w - 2, bottom - y);
        for (i, m) in visible.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, row_h - 4);
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, 6, if hovered { p.bg_card_hover } else { p.bg_card });
            canvas.draw_text_clipped(
                card.x + 8,
                card.y + 4,
                card.w - 16,
                &format!("{}:{}", m.rel_path, m.line),
                FontRole::UiSemibold,
                p.accent_secondary,
            );
            canvas.draw_text_clipped(
                card.x + 8,
                card.y + 21,
                card.w - 16,
                &m.preview,
                FontRole::TerminalMono,
                p.text_secondary,
            );
            self.push_hit(card, UiAction::OpenSearchHit(m.path.clone(), m.rel_path.clone(), m.line));
        }
        canvas.pop_clip();
    }

    fn draw_git_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        let p = self.palette.clone();
        let x = RAIL_W;

        if !self.git_snapshot.is_repo {
            canvas.draw_text(
                x + PAD,
                y + 8,
                "No git repository detected in this workspace.",
                FontRole::UiSmall,
                p.text_muted,
            );
            return;
        }

        // Branch selector row
        let branch_w = (w - PAD * 2 - 120).clamp(80, 220);
        self.draw_pill(
            canvas,
            UiRect::new(x + PAD, y, branch_w, 26),
            &format!("{} {}", glyph("branch"), truncate_right(&self.git_snapshot.branch, 16)),
            true,
            None,
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - PAD - 112, y, 54, 26),
            "Stage All",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::GitStageAll),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - PAD - 54, y, 54, 26),
            "Diff",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            None,
        );
        y += 34;

        // Commit box
        let commit = UiRect::new(x + PAD, y, w - PAD * 2, 34);
        self.draw_text_input(
            canvas,
            commit,
            self.commit_message.clone().as_str(),
            "Commit message...",
            self.focus == FocusTarget::CommitInput,
            FontRole::UiSmall,
            UiAction::Focus(FocusTarget::CommitInput),
        );
        self.draw_button(
            canvas,
            UiRect::new(commit.x + commit.w - 74, commit.y + 4, 68, 26),
            "Commit",
            p.success,
            p.on_accent_text(p.success),
            p.success,
            Some(UiAction::GitCommit),
        );
        y += 44;

        self.draw_section_title(
            canvas,
            x + PAD,
            y,
            w - 24,
            &format!("CHANGES ({})", self.git_snapshot.files.len()),
        );
        y += 18;

        let row_h = 30;
        let avail = ((bottom - y - 6).max(0) / row_h) as usize;
        let max_scroll = self.git_snapshot.files.len().saturating_sub(avail.max(1));
        if self.git_scroll > max_scroll {
            self.git_scroll = max_scroll;
        }
        let files: Vec<crate::git_ops::GitChangedFile> = self
            .git_snapshot
            .files
            .iter()
            .skip(self.git_scroll)
            .take(avail)
            .cloned()
            .collect();

        canvas.push_clip(x, y, w - 2, bottom - y);
        for (i, f) in files.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let row_rect = UiRect::new(x + 4, ry, w - 10, row_h - 2);
            let hovered = row_rect.contains(self.mouse_x, self.mouse_y);
            if hovered {
                canvas.fill_rounded_rect(row_rect.x, row_rect.y, row_rect.w, row_rect.h, 5, p.bg_card);
            }
            let status_color = match f.status.as_str() {
                "M" | "modified" => p.warning,
                "A" | "added" => p.success,
                "D" | "deleted" => p.danger,
                _ => p.text_muted,
            };
            canvas.draw_text(row_rect.x + 6, ry + 4, &f.status, FontRole::TerminalMono, status_color);
            canvas.draw_text_clipped(
                row_rect.x + 24,
                ry + 4,
                w - (row_rect.x + 24 - x) - 66,
                &f.path,
                FontRole::UiSmall,
                p.text_secondary,
            );
            if hovered {
                let stage_rect = UiRect::new(row_rect.x + row_rect.w - 56, ry + 3, 26, row_h - 8);
                canvas.fill_rounded_rect(stage_rect.x, stage_rect.y, stage_rect.w, stage_rect.h, 4, p.bg_button);
                canvas.draw_text(stage_rect.x + 7, ry + 4, "+", FontRole::UiSmall, p.text_primary);
                self.push_hit(
                    stage_rect,
                    UiAction::GitStageFile {
                        path: f.path.clone(),
                        currently_staged: f.staged,
                    },
                );
                let diff_rect = UiRect::new(row_rect.x + row_rect.w - 28, ry + 3, 26, row_h - 8);
                canvas.fill_rounded_rect(diff_rect.x, diff_rect.y, diff_rect.w, diff_rect.h, 4, p.bg_button);
                canvas.draw_text(diff_rect.x + 7, ry + 4, "~", FontRole::UiSmall, p.accent_secondary);
                self.push_hit(diff_rect, UiAction::GitOpenDiff(f.path.clone()));
            }
        }
        canvas.pop_clip();
    }

    fn draw_sessions_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        let p = self.palette.clone();
        let x = RAIL_W;
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, y, (w - PAD * 2 - 8) / 2, 28),
            "+ New Session",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::NewSession),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + (w - PAD * 2 - 8) / 2 + 8, y, (w - PAD * 2 - 8) / 2, 28),
            "Fork Active",
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::ForkActiveSession),
        );
        y += 38;

        let row_h = 58;
        let avail = ((bottom - y - 6).max(0) / row_h) as usize;
        let max_scroll = self.sessions.len().saturating_sub(avail.max(1));
        if self.sessions_scroll > max_scroll {
            self.sessions_scroll = max_scroll;
        }
        let sessions: Vec<SessionSummary> = self
            .sessions
            .iter()
            .skip(self.sessions_scroll)
            .take(avail)
            .cloned()
            .collect();

        canvas.push_clip(x, y, w - 2, bottom - y);
        for (i, s) in sessions.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, row_h - 6);
            let active = s.id == self.active_session.id;
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(
                card.x,
                card.y,
                card.w,
                card.h,
                8,
                if active { mix(p.accent, p.bg_sidebar, 0.16) } else if hovered { p.bg_card_hover } else { p.bg_card },
            );
            canvas.draw_text_clipped(
                card.x + 10,
                card.y + 6,
                card.w - 20,
                &truncate_right(&s.title, 30),
                FontRole::UiSemibold,
                if active { p.accent } else { p.text_primary },
            );
            canvas.draw_text_clipped(
                card.x + 10,
                card.y + 23,
                card.w - 20,
                &format!(
                    "{} msgs · {} · {}",
                    s.message_count,
                    s.mode.to_uppercase(),
                    crate::sqlite::short_time_label(&s.updated_at)
                ),
                FontRole::UiSmall,
                p.text_muted,
            );
            if !s.parent_id.is_empty() {
                canvas.draw_text(card.x + card.w - 66, card.y + 23, "fork", FontRole::UiSmall, p.purple);
            }
            if s.compacted_count > 0 {
                canvas.draw_text(
                    card.x + card.w - 34,
                    card.y + 23,
                    &format!("c{}", s.compacted_count),
                    FontRole::UiSmall,
                    p.warning,
                );
            }
            self.push_hit(card, UiAction::SelectSession(s.id.clone()));
        }
        canvas.pop_clip();
    }

    fn draw_extensions_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        let p = self.palette.clone();
        let x = RAIL_W;
        let avail_total = (bottom - y).max(120);
        let row_h = 34;
        let avail_rows = ((avail_total - 40) / row_h).max(4) as usize;

        // ---- Skills -------------------------------------------------------
        self.draw_section_title(
            canvas,
            x + PAD,
            y,
            w - 24,
            &format!("SKILLS ({}) — toggle to enable", self.skills.len()),
        );
        y += 18;
        let skill_rows = avail_rows.min(self.skills.len().max(1));
        for i in 0..skill_rows {
            let Some(skill) = self.skills.get(i).cloned() else { break };
            let ry = y + i as i32 * row_h;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, row_h - 4);
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, 6, if hovered { p.bg_card_hover } else { p.bg_card });
            let dot = if skill.enabled { p.success } else { p.text_muted };
            canvas.fill_rounded_rect(card.x + 8, card.y + 9, 10, 10, 5, dot);
            canvas.draw_text_clipped(
                card.x + 26,
                card.y + 5,
                card.w - 100,
                &truncate_right(&skill.title, 30),
                FontRole::UiSemibold,
                p.text_primary,
            );
            canvas.draw_text_clipped(
                card.x + 26,
                card.y + 19,
                card.w - 100,
                &truncate_right(&skill.description, 34),
                FontRole::UiSmall,
                p.text_muted,
            );
            if skill.builtin {
                canvas.draw_text(card.x + card.w - 68, card.y + 10, "builtin", FontRole::UiSmall, p.accent_secondary);
            }
            self.push_hit(card, UiAction::ToggleSkill(skill.slug.clone()));
        }
        y += skill_rows as i32 * row_h + 10;

        // ---- Sub-agents ----------------------------------------------------
        self.draw_section_title(
            canvas,
            x + PAD,
            y,
            w - 24,
            &format!("SUB-AGENTS ({})", self.subagents.len()),
        );
        y += 18;
        for (i, sa) in self.subagents.clone().into_iter().take(5).enumerate() {
            let ry = y + i as i32 * 26;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, 22);
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            if hovered {
                canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, 5, p.bg_card);
            }
            canvas.draw_text(card.x + 8, ry + 4, glyph("robot"), FontRole::UiSmall, p.purple);
            canvas.draw_text_clipped(card.x + 28, ry + 4, card.w - 130, &truncate_right(&sa.title, 24), FontRole::UiSmall, p.text_secondary);
            canvas.draw_text_clipped(
                card.x + card.w - 96,
                ry + 4,
                52,
                &sa.effort.to_uppercase(),
                FontRole::UiSmall,
                if sa.effort == "ultra" { p.danger } else { p.warning },
            );
            if sa.readonly {
                canvas.draw_text(card.x + card.w - 40, ry + 4, "RO", FontRole::UiSmall, p.text_muted);
            }
        }
        y += 5 * 26 + 8;

        // ---- MCP servers ----------------------------------------------------
        self.draw_section_title(
            canvas,
            x + PAD,
            y,
            w - 24,
            &format!("MCP SERVERS ({})", self.mcp_servers.len()),
        );
        y += 18;
        for (i, srv) in self.mcp_servers.clone().into_iter().take(6).enumerate() {
            let ry = y + i as i32 * 26;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, 22);
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            if hovered {
                canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, 5, p.bg_card);
            }
            let status_col = if srv.status.starts_with("online") {
                p.success
            } else if srv.status.starts_with("error") {
                p.danger
            } else {
                p.text_muted
            };
            canvas.fill_rounded_rect(card.x + 8, ry + 7, 8, 8, 4, status_col);
            canvas.draw_text_clipped(card.x + 24, ry + 4, card.w - 150, &truncate_right(&srv.name, 18), FontRole::UiSmall, p.text_secondary);
            canvas.draw_text_clipped(
                card.x + card.w - 122,
                ry + 4,
                50,
                &format!("{} tools", srv.discovered_tools.len()),
                FontRole::UiSmall,
                p.accent_secondary,
            );
            let toggle = UiRect::new(card.x + card.w - 62, ry + 2, 52, 18);
            self.draw_button(
                canvas,
                toggle,
                if srv.enabled { "ON" } else { "OFF" },
                if srv.enabled { p.success } else { p.bg_button },
                if srv.enabled { p.on_accent_text(p.success) } else { p.text_muted },
                p.border_subtle,
                Some(UiAction::ToggleMcpServer(i)),
            );
        }
        y += 6 * 26 + 12;

        if self.show_mcp_add {
            let card = UiRect::new(x + PAD, y, w - PAD * 2, 92);
            canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, CARD_RADIUS, p.bg_card);
            canvas.stroke_rounded_rect(card.x, card.y, card.w, card.h, CARD_RADIUS, p.accent);
            canvas.draw_text(card.x + 10, card.y + 8, "Add MCP server", FontRole::UiSemibold, p.accent);
            let name_rect = UiRect::new(card.x + 8, card.y + 28, (card.w - 24) / 2, 24);
            self.draw_text_input(
                canvas,
                name_rect,
                self.new_mcp_name.clone().as_str(),
                "name",
                self.focus == FocusTarget::McpNameInput,
                FontRole::UiSmall,
                UiAction::Focus(FocusTarget::McpNameInput),
            );
            let tgt_rect = UiRect::new(name_rect.x + name_rect.w + 8, card.y + 28, (card.w - 24) / 2, 24);
            self.draw_text_input(
                canvas,
                tgt_rect,
                self.new_mcp_target.clone().as_str(),
                "cmd args | https://url",
                self.focus == FocusTarget::McpTargetInput,
                FontRole::UiSmall,
                UiAction::Focus(FocusTarget::McpTargetInput),
            );
            self.draw_button(
                canvas,
                UiRect::new(card.x + 8, card.y + 58, 86, 26),
                "Add",
                p.success,
                p.on_accent_text(p.success),
                p.success,
                Some(UiAction::AddQuickMcpServer),
            );
            self.draw_button(
                canvas,
                UiRect::new(card.x + 100, card.y + 58, 86, 26),
                "Cancel",
                p.bg_button,
                p.text_secondary,
                p.border_subtle,
                Some(UiAction::CloseModal),
            );
        }

        // Action row
        let bw = (w - PAD * 2 - 8) / 2;
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, bottom - 36, bw, 28),
            "Probe MCP",
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::ProbeAllMcpServers),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + bw + 8, bottom - 36, bw, 28),
            "+ MCP Server",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::ExecuteCommandId("mcp:add".to_string())),
        );
    }
}

// ---------------------------------------------------------------------------
// Center: native code editor / diff viewer + agentic browser viewport
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_center(&mut self, canvas: &mut dyn SurfaceCanvas, x: i32, top: i32, w: i32, h: i32) {
        match self.center_view {
            CenterView::Editor => self.draw_center_editor(canvas, x, top, w, h),
            CenterView::Browser => self.draw_browser_viewport(canvas, x, top, w, h),
        }
    }

    fn draw_center_editor(&mut self, canvas: &mut dyn SurfaceCanvas, x: i32, top: i32, w: i32, h: i32) {
        let p = self.palette.clone();
        canvas.fill_rect(x, top, w, h, p.bg_editor);

        // ---- Tab strip ------------------------------------------------------
        let strip_h = 36;
        canvas.fill_rect(x, top, w, strip_h, p.bg_sidebar);
        canvas.draw_hline(x, top + strip_h - 1, w, p.border_subtle);

        let mut tx = x + PAD;
        let max_tab_x = x + w - 200;
        let tab_summaries: Vec<(String, bool, bool)> = self
            .tabs
            .iter()
            .map(|t| {
                let is_diff = matches!(t.content, TabContent::Diff { .. });
                (t.title.clone(), t.is_dirty(), is_diff)
            })
            .collect();
        for (idx, (title, dirty, is_diff)) in tab_summaries.into_iter().enumerate() {
            if tx + 96 > max_tab_x {
                break;
            }
            let tw = ((title.chars().count() as i32) * 7 + 44).clamp(104, 190);
            let active = idx == self.active_tab;
            let tab_rect = UiRect::new(tx, top + 5, tw, strip_h - 10);
            canvas.fill_rounded_rect(
                tab_rect.x,
                tab_rect.y,
                tab_rect.w,
                tab_rect.h,
                6,
                if active { mix(p.accent, p.bg_editor, 0.14) } else { p.bg_card },
            );
            if active {
                canvas.fill_rect(tab_rect.x + 8, tab_rect.y + 6, 3, tab_rect.h - 12, p.accent);
            }
            let label = if dirty { format!("* {}", title) } else { title };
            canvas.draw_text_clipped(
                tx + 18,
                top + 11,
                tw - 38,
                &label,
                if active { FontRole::UiSemibold } else { FontRole::UiSmall },
                if is_diff {
                    p.warning
                } else if active {
                    p.text_primary
                } else {
                    p.text_secondary
                },
            );
            let close_rect = UiRect::new(tx + tw - 22, top + 9, 18, 18);
            canvas.draw_text(close_rect.x + 5, close_rect.y + 1, "x", FontRole::UiSmall, p.text_muted);
            self.push_hit(UiRect::new(tx, top + 5, tw - 26, strip_h - 10), UiAction::SelectEditorTab(idx));
            self.push_hit(close_rect, UiAction::CloseEditorTab(idx));
            tx += tw + 6;
        }

        self.draw_button(
            canvas,
            UiRect::new(x + w - 158, top + 6, 46, 24),
            "+ New",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::NewScratchTab),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 108, top + 6, 50, 24),
            "Find",
            if self.show_find_bar { p.accent } else { p.bg_card },
            if self.show_find_bar { p.on_accent_text(p.accent) } else { p.text_secondary },
            p.border_subtle,
            Some(UiAction::ToggleFindBar),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 54, top + 6, 46, 24),
            "Save",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::SaveActiveTab),
        );

        let mut content_y = top + strip_h;

        // ---- Find / Go-To-Line bars -----------------------------------------
        if self.show_find_bar {
            canvas.fill_rect(x, content_y, w, 32, p.bg_card);
            canvas.draw_hline(x, content_y + 31, w, p.border_subtle);
            canvas.draw_text(x + 12, content_y + 9, "Find", FontRole::UiSmall, p.text_muted);
            let f_inp = UiRect::new(x + 52, content_y + 4, 260.min(w - 200), 24);
            self.draw_text_input(
                canvas,
                f_inp,
                self.find_query.clone().as_str(),
                "Search in file...",
                self.focus == FocusTarget::FindInput,
                FontRole::TerminalMono,
                UiAction::Focus(FocusTarget::FindInput),
            );
            self.draw_button(
                canvas,
                UiRect::new(f_inp.x + f_inp.w + 8, content_y + 4, 58, 24),
                "Next",
                p.accent,
                p.on_accent_text(p.accent),
                p.accent,
                Some(UiAction::FindNext),
            );
            self.draw_button(
                canvas,
                UiRect::new(f_inp.x + f_inp.w + 72, content_y + 4, 74, 24),
                "Go to line",
                p.bg_button,
                p.text_secondary,
                p.border_subtle,
                Some(UiAction::ToggleGotoLine),
            );
            content_y += 32;
        }
        if self.show_goto_line {
            canvas.fill_rect(x, content_y, w, 32, p.bg_card);
            canvas.draw_hline(x, content_y + 31, w, p.border_subtle);
            canvas.draw_text(x + 12, content_y + 9, "Line", FontRole::UiSmall, p.text_muted);
            let g_inp = UiRect::new(x + 52, content_y + 4, 140, 24);
            self.draw_text_input(
                canvas,
                g_inp,
                self.goto_line_buf.clone().as_str(),
                "e.g. 128",
                self.focus == FocusTarget::GotoLineInput,
                FontRole::TerminalMono,
                UiAction::Focus(FocusTarget::GotoLineInput),
            );
            self.draw_button(
                canvas,
                UiRect::new(g_inp.x + g_inp.w + 8, content_y + 4, 74, 24),
                "Jump",
                p.accent,
                p.on_accent_text(p.accent),
                p.accent,
                Some(UiAction::ConfirmGotoLine),
            );
            content_y += 32;
        }

        let body_h = (top + h - content_y).max(20);
        self.editor_rect = UiRect::new(x, content_y, w, body_h);

        if self.tabs.is_empty() {
            self.draw_editor_empty_state(canvas, x, content_y, w, body_h);
            return;
        }

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };

        let (char_w_raw, line_h_raw) = canvas.mono_cell_size(FontRole::CodeMono);
        let char_w = char_w_raw.max(7);
        let line_h = line_h_raw.max(19);
        let gutter_w = 58;
        let minimap_w = if self.show_minimap { 74 } else { 0 };

        canvas.push_clip(x, content_y, w, body_h);
        match &tab.content {
            TabContent::Code {
                lines,
                cursor_line,
                cursor_col,
                scroll_y,
                scroll_x,
                ..
            } => {
                canvas.fill_rect(x, content_y, gutter_w, body_h, p.bg_gutter);
                canvas.draw_vline(x + gutter_w - 1, content_y, body_h, p.border_subtle);

                let vis_rows = (body_h / line_h) as usize + 1;
                let sel = tab.ordered_selection();
                let lang = tab.lang;
                let editor_focused = self.focus == FocusTarget::Editor && self.modal == ModalDialog::None;
                let bracket = tab.matching_bracket();
                let text_w = w - gutter_w - 8 - minimap_w;

                for idx in 0..vis_rows {
                    let row = *scroll_y + idx;
                    if row >= lines.len() {
                        break;
                    }
                    let ry = content_y + 6 + idx as i32 * line_h;
                    if row == *cursor_line {
                        canvas.fill_rect(x + gutter_w, ry - 2, text_w, line_h, p.bg_active_line);
                    }
                    canvas.draw_text(
                        x + 10,
                        ry,
                        &format!("{:>4}", row + 1),
                        FontRole::CodeMono,
                        if row == *cursor_line { p.accent } else { p.text_muted },
                    );

                    let line = &lines[row];
                    let total_chars = char_len(line);

                    if let Some(((sr, sc), (er, ec))) = sel {
                        if row >= sr && row <= er {
                            let s_c = if row == sr { sc } else { 0 };
                            let e_c = if row == er { ec } else { total_chars + 1 };
                            if e_c > *scroll_x {
                                let vis_s = s_c.saturating_sub(*scroll_x);
                                let vis_e = e_c.saturating_sub(*scroll_x);
                                let sx = x + gutter_w + 8 + (vis_s as i32) * char_w;
                                let sw = ((vis_e - vis_s) as i32 * char_w).max(6);
                                canvas.fill_rect(sx, ry - 2, sw, line_h, p.bg_selection);
                            }
                        }
                    }

                    // Matching-bracket highlight
                    if let Some((bl, bc)) = bracket {
                        if bl == row {
                            let bx = x + gutter_w + 8 + ((bc.saturating_sub(*scroll_x)) as i32) * char_w;
                            if bx > x + gutter_w {
                                canvas.stroke_rect(bx, ry - 2, char_w, line_h, p.accent_secondary);
                            }
                        }
                    }

                    let spans = tokenize_line(lang, line);
                    let mut col_cursor = 0usize;
                    for sp in spans {
                        let sp_len = char_len(&sp.text);
                        let sp_end = col_cursor + sp_len;
                        if sp_end > *scroll_x {
                            let vis_start = col_cursor.max(*scroll_x);
                            let local_skip = vis_start - col_cursor;
                            let visible_piece = slice_chars(&sp.text, local_skip, sp_len);
                            let sx = x + gutter_w + 8 + ((vis_start - *scroll_x) as i32) * char_w;
                            if sx < x + w {
                                canvas.draw_text(
                                    sx,
                                    ry,
                                    &visible_piece,
                                    FontRole::CodeMono,
                                    sp.kind.themed_rgb(&p),
                                );
                            }
                        }
                        col_cursor = sp_end;
                    }

                    if editor_focused
                        && self.blink_phase
                        && row == *cursor_line
                        && *cursor_col >= *scroll_x
                    {
                        let cx = x + gutter_w + 8 + ((*cursor_col - *scroll_x) as i32) * char_w;
                        canvas.fill_rect(cx, ry - 1, 2, line_h - 2, p.accent);
                    }
                }

                // Vertical scrollbar
                let total_lines = lines.len();
                if total_lines > vis_rows {
                    let track_h = body_h - 12;
                    let thumb_h = (((track_h as usize * vis_rows) / total_lines) as i32).max(24);
                    let max_scroll = total_lines.saturating_sub(vis_rows);
                    let ratio = if max_scroll == 0 { 0.0 } else { *scroll_y as f32 / max_scroll as f32 };
                    let thumb_y = content_y + 6 + ((track_h - thumb_h) as f32 * ratio) as i32;
                    let sb_x = x + w - 10;
                    canvas.fill_rounded_rect(sb_x, content_y + 6, 5, track_h, 2, p.bg_input);
                    canvas.fill_rounded_rect(sb_x, thumb_y, 5, thumb_h, 2, mix(p.accent, p.bg_editor, 0.4));
                }

                // Minimap
                if self.show_minimap {
                    let mm_x = x + w - minimap_w;
                    canvas.fill_rect(mm_x, content_y, minimap_w, body_h, mix(p.bg_gutter, p.bg_editor, 0.5));
                    canvas.draw_vline(mm_x, content_y, body_h, p.border_subtle);
                    let mm_rows = ((body_h - 12) / 3).max(1) as usize;
                    let mm_total = mm_rows * 12;
                    for (i, line) in lines.iter().enumerate().take(mm_total) {
                        let ry = content_y + 6 + (i as i32) * 3;
                        let indent = line.len() - line.trim_start().len();
                        let width = ((line.trim().chars().count().min(60) as i32) * 1).max(1);
                        let col = if line.trim_start().starts_with("//") {
                            p.syn_comment
                        } else if i == *cursor_line {
                            p.accent
                        } else {
                            mix(p.text_muted, p.bg_editor, 0.35)
                        };
                        canvas.fill_rect(mm_x + 4 + (indent as i32).min(20), ry, width.min(minimap_w - 10), 2, col);
                    }
                }
            }
            TabContent::Diff { rows, scroll_y } => {
                let half_w = w / 2;
                canvas.fill_rect(x, content_y, w, 26, p.bg_card);
                canvas.draw_text_clipped(x + 12, content_y + 5, half_w - 20, "HEAD (Original)", FontRole::UiSemibold, p.danger);
                canvas.draw_text_clipped(
                    x + half_w + 12,
                    content_y + 5,
                    half_w - 20,
                    "Working Copy (Modified)",
                    FontRole::UiSemibold,
                    p.success,
                );
                canvas.draw_vline(x + half_w, content_y, body_h, p.border_subtle);

                let d_line_h = 19;
                let vis_rows = ((body_h - 30) / d_line_h) as usize + 1;
                for idx in 0..vis_rows {
                    let r_i = *scroll_y + idx;
                    let Some(row) = rows.get(r_i) else { break };
                    let ry = content_y + 30 + idx as i32 * d_line_h;
                    let (l_bg, r_bg) = match row.kind {
                        DiffLineKind::Unchanged => (p.bg_editor, p.bg_editor),
                        DiffLineKind::Removed => (p.diff_del_bg, p.bg_editor),
                        DiffLineKind::Added => (p.bg_editor, p.diff_add_bg),
                        DiffLineKind::Changed => (p.diff_del_bg, p.diff_add_bg),
                    };
                    canvas.fill_rect(x, ry, half_w, d_line_h, l_bg);
                    canvas.fill_rect(x + half_w + 1, ry, half_w - 1, d_line_h, r_bg);
                    if let Some(ln) = row.left_no {
                        canvas.draw_text(x + 8, ry + 1, &format!("{:>4}", ln), FontRole::TerminalMono, p.text_muted);
                    }
                    canvas.draw_text_clipped(x + 46, ry + 1, half_w - 52, &row.left_text, FontRole::TerminalMono, p.text_primary);
                    if let Some(rn) = row.right_no {
                        canvas.draw_text(x + half_w + 10, ry + 1, &format!("{:>4}", rn), FontRole::TerminalMono, p.text_muted);
                    }
                    canvas.draw_text_clipped(
                        x + half_w + 48,
                        ry + 1,
                        half_w - 54,
                        &row.right_text,
                        FontRole::TerminalMono,
                        p.text_primary,
                    );
                }
            }
        }
        canvas.pop_clip();
    }

    fn draw_editor_empty_state(&mut self, canvas: &mut dyn SurfaceCanvas, x: i32, y: i32, w: i32, h: i32) {
        let p = self.palette.clone();
        let cw = 560.min(w - PAD * 3);
        let cx = x + ((w - cw) / 2).max(PAD);
        let cy = y + (h / 2 - 150).max(PAD);
        let ch = 300.min((h - PAD * 2).max(160));
        canvas.fill_rounded_rect(cx, cy, cw, ch, 12, p.bg_card);
        canvas.stroke_rounded_rect(cx, cy, cw, ch, 12, p.border_subtle);
        canvas.draw_text(cx + 28, cy + 26, "AirCode", FontRole::UiTitle, p.accent);
        canvas.draw_text(
            cx + 28,
            cy + 52,
            "100% Pure-Rust Native Desktop Studio & Agentic Browser",
            FontRole::UiSemibold,
            p.text_primary,
        );
        canvas.draw_text_clipped(
            cx + 28,
            cy + 74,
            cw - 56,
            "Zero WebView. Native editor, autonomous agent, MCP, sub-agents & inbuilt browser.",
            FontRole::UiSmall,
            p.text_secondary,
        );

        let bw = (cw - 56 - 12) / 2;
        self.draw_button(
            canvas,
            UiRect::new(cx + 28, cy + 108, bw, 34),
            "Open Project Folder",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::PickFolder),
        );
        self.draw_button(
            canvas,
            UiRect::new(cx + 28 + bw + 12, cy + 108, bw, 34),
            "New Scratch Buffer",
            p.bg_button,
            p.text_primary,
            p.border_subtle,
            Some(UiAction::NewScratchTab),
        );
        self.draw_button(
            canvas,
            UiRect::new(cx + 28, cy + 150, bw, 34),
            "/ Connect AI Provider",
            p.bg_button,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::OpenModal(ModalDialog::ConnectWizard)),
        );
        self.draw_button(
            canvas,
            UiRect::new(cx + 28 + bw + 12, cy + 150, bw, 34),
            "Agentic Browser",
            p.bg_button,
            p.purple,
            p.border_subtle,
            Some(UiAction::SwitchCenterView(CenterView::Browser)),
        );
        canvas.draw_text_clipped(
            cx + 28,
            cy + 204,
            cw - 56,
            "Tip: press Ctrl+K (or /) for the fuzzy command palette — themes, fork, compact, effort, skills, MCP.",
            FontRole::UiSmall,
            p.text_muted,
        );
    }
}

// ---------------------------------------------------------------------------
// Inbuilt Agentic Browser viewport (Playwright-style refs + user takeover)
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_browser_viewport(&mut self, canvas: &mut dyn SurfaceCanvas, x: i32, top: i32, w: i32, h: i32) {
        let p = self.palette.clone();
        canvas.fill_rect(x, top, w, h, p.bg_editor);
        self.browser_rect = UiRect::new(x, top, w, h);

        // ---- Navigation toolbar ---------------------------------------------
        let bar_h = 40;
        canvas.fill_rect(x, top, w, bar_h, p.bg_sidebar);
        canvas.draw_hline(x, top + bar_h - 1, w, p.border_subtle);

        self.draw_button(
            canvas,
            UiRect::new(x + PAD, top + 6, 34, 28),
            glyph("arrow_l"),
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::BrowserBack),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + 38, top + 6, 34, 28),
            glyph("arrow_r"),
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::BrowserForward),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + 76, top + 6, 34, 28),
            glyph("reload"),
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::BrowserReload),
        );

        let url_x = x + PAD + 118;
        let url_w = (w - PAD * 2 - 118 - 190).max(120);
        let url_rect = UiRect::new(url_x, top + 6, url_w, 28);
        self.draw_text_input(
            canvas,
            url_rect,
            self.browser_url_buf.clone().as_str(),
            "https://localhost:3000  ·  press Enter to navigate",
            self.focus == FocusTarget::BrowserUrlInput,
            FontRole::TerminalMono,
            UiAction::Focus(FocusTarget::BrowserUrlInput),
        );

        let mut rx = x + w - PAD;
        rx -= 88;
        self.draw_button(
            canvas,
            UiRect::new(rx, top + 6, 88, 28),
            "Engine: JS",
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::BrowserToggleEngine),
        );
        rx -= 88;
        self.draw_button(
            canvas,
            UiRect::new(rx, top + 6, 88, 28),
            "Pop Out",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::BrowserPopOutExternal),
        );

        let content_y = top + bar_h;

        // ---- Page state + takeover controls ---------------------------------
        let (url, title, status, owner_label, owner_col, engine_label, inspector) = {
            let g = self.browser.inner.lock().unwrap();
            let owner_label = match &g.owner {
                BrowserOwner::Idle => "IDLE".to_string(),
                BrowserOwner::AiDriving(a) => format!("AI DRIVING · {}", truncate_right(a, 26)),
                BrowserOwner::UserTakeover => "YOU ARE IN CONTROL".to_string(),
            };
            let owner_col = match &g.owner {
                BrowserOwner::UserTakeover => p.success,
                BrowserOwner::AiDriving(_) => p.purple,
                BrowserOwner::Idle => p.text_muted,
            };
            (
                g.current_url.clone(),
                g.title.clone(),
                g.http_status,
                owner_label,
                owner_col,
                match g.engine {
                    BrowserJsEngine::NativeFast => "NativeFast".to_string(),
                    BrowserJsEngine::HeadlessDomJs => "Headless DOM+JS".to_string(),
                },
                g.show_dom_inspector,
            )
        };
        if self.browser_url_buf.is_empty() {
            self.browser_url_buf = url.clone();
        }

        let state_h = 32;
        canvas.fill_rect(x, content_y, w, state_h, p.bg_card);
        canvas.draw_hline(x, content_y + state_h - 1, w, p.border_subtle);
        canvas.fill_rounded_rect(x + PAD, content_y + 8, 8, 8, 4, owner_col);
        canvas.draw_text_clipped(x + PAD + 14, content_y + 8, (w / 3).max(80), &owner_label, FontRole::UiSemibold, owner_col);
        canvas.draw_text_clipped(
            x + PAD + 14 + (w / 3).max(80) + 8,
            content_y + 8,
            (w / 3).max(80),
            &format!("{} · HTTP {} · {}", truncate_right(&title, 30), status, engine_label),
            FontRole::UiSmall,
            p.text_muted,
        );
        let mut bx = x + w - PAD;
        bx -= 104;
        self.draw_button(
            canvas,
            UiRect::new(bx, content_y + 4, 104, 24),
            "Take Over",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::BrowserInterruptTakeover),
        );
        bx -= 104;
        self.draw_button(
            canvas,
            UiRect::new(bx, content_y + 4, 100, 24),
            "Resume AI",
            p.bg_button,
            p.purple,
            p.border_subtle,
            Some(UiAction::BrowserHandBackToAi),
        );

        let page_y = content_y + state_h;
        let page_h = (top + h - page_y).max(20);

        // ---- DOM inspector pane ---------------------------------------------
        let inspector_w = if inspector { 250 } else { 0 };
        if inspector {
            let ix = x + w - inspector_w;
            canvas.fill_rect(ix, page_y, inspector_w, page_h, p.bg_card);
            canvas.draw_vline(ix, page_y, page_h, p.border_subtle);
            self.draw_section_title(canvas, ix + 10, page_y + 8, inspector_w - 20, "PLAYWRIGHT REFS");
            let els: Vec<crate::browser::InteractiveElement> = {
                let g = self.browser.inner.lock().unwrap();
                g.elements.clone()
            };
            let mut ey = page_y + 28;
            for el in els.iter().take(40) {
                let row = UiRect::new(ix + 6, ey, inspector_w - 12, 24);
                let hovered = row.contains(self.mouse_x, self.mouse_y);
                if hovered {
                    canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 4, p.bg_card_hover);
                }
                canvas.draw_text(row.x + 6, ey + 3, &format!("[e{}]", el.ref_id), FontRole::TerminalMono, p.accent_secondary);
                canvas.draw_text_clipped(row.x + 46, ey + 3, row.w - 52, &truncate_right(&el.label, 26), FontRole::UiSmall, p.text_secondary);
                if el.kind == "input" {
                    let input_rect = UiRect::new(ix + 6, ey + 24, inspector_w - 12, 22);
                    self.push_hit(input_rect, UiAction::BrowserClickRef(el.ref_id));
                    ey += 26;
                }
                ey += 26;
            }
        }

        let page_w = w - inspector_w;

        // ---- Page content ----------------------------------------------------
        canvas.push_clip(x, page_y, page_w, page_h);
        let blocks: Vec<crate::browser::PageBlock> = {
            let g = self.browser.inner.lock().unwrap();
            g.blocks.clone()
        };
        let mut by = page_y + PAD - self.browser_scroll;
        let text_w = page_w - PAD * 2 - 20;
        for blk in blocks.iter().take(300) {
            match blk {
                crate::browser::PageBlock::Heading { level, text } => {
                    let font = match level {
                        1 => FontRole::UiTitle,
                        2 => FontRole::UiSemibold,
                        _ => FontRole::UiRegular,
                    };
                    let lh = if *level <= 1 { 26 } else { 22 };
                    by += 6;
                    if by > page_y - 30 && by < page_y + page_h + 30 {
                        canvas.draw_text_clipped(x + PAD, by, text_w, text, font, p.text_primary);
                    }
                    by += lh;
                }
                crate::browser::PageBlock::Paragraph { text } | crate::browser::PageBlock::Banner { text } => {
                    for line in wrap_text(text, ((text_w / 7).max(20)) as usize).into_iter().take(24) {
                        if by > page_y - 20 && by < page_y + page_h + 20 {
                            canvas.draw_text_clipped(x + PAD, by, text_w, &line, FontRole::UiSmall, p.text_secondary);
                        }
                        by += 17;
                    }
                    by += 6;
                }
                crate::browser::PageBlock::ListItem { text } => {
                    if by > page_y - 20 && by < page_y + page_h + 20 {
                        canvas.draw_text(x + PAD, by, "•", FontRole::UiSmall, p.accent);
                        canvas.draw_text_clipped(x + PAD + 12, by, text_w - 12, text, FontRole::UiSmall, p.text_secondary);
                    }
                    by += 18;
                }
                crate::browser::PageBlock::CodeBlock { text } => {
                    let bh = (text.lines().count() as i32 * 17 + 12).min(220);
                    let card = UiRect::new(x + PAD, by, page_w - PAD * 2, bh);
                    if by > page_y - bh && by < page_y + page_h + bh {
                        canvas.fill_rounded_rect(card.x, card.y, card.w, card.h, 6, p.bg_input);
                        for (i, line) in text.lines().take(12).enumerate() {
                            canvas.draw_text_clipped(
                                card.x + 8,
                                card.y + 6 + i as i32 * 17,
                                card.w - 16,
                                line,
                                FontRole::TerminalMono,
                                p.syn_string,
                            );
                        }
                    }
                    by += bh + 8;
                }
                crate::browser::PageBlock::Link { ref_id, label, .. } => {
                    let row = UiRect::new(x + PAD - 4, by - 2, page_w - PAD * 2 + 8, 22);
                    let hovered = row.contains(self.mouse_x, self.mouse_y);
                    if hovered {
                        canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 5, p.bg_card_hover);
                    }
                    if by > page_y - 20 && by < page_y + page_h + 20 {
                        canvas.draw_text(x + PAD, by, &format!("[e{}]", ref_id), FontRole::TerminalMono, p.accent_secondary);
                        canvas.draw_text_clipped(x + PAD + 42, by, text_w - 42, label, FontRole::UiSmall, p.accent);
                    }
                    self.push_hit(row, UiAction::BrowserClickRef(*ref_id));
                    by += 24;
                }
                crate::browser::PageBlock::Button { ref_id, label, .. } => {
                    let btn_w = (canvas.measure_text_width(label, FontRole::UiSemibold) + 56).min(page_w - PAD * 3);
                    let row = UiRect::new(x + PAD, by, btn_w, 28);
                    let hovered = row.contains(self.mouse_x, self.mouse_y);
                    canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 6, if hovered { lighten(p.accent, 14) } else { mix(p.accent, p.bg_editor, 0.18) });
                    canvas.stroke_rounded_rect(row.x, row.y, row.w, row.h, 6, p.accent);
                    canvas.draw_text(row.x + 14, by + 6, &format!("[e{}]", ref_id), FontRole::TerminalMono, p.accent_secondary);
                    canvas.draw_text_clipped(row.x + 46, by + 6, row.w - 54, label, FontRole::UiSemibold, p.text_primary);
                    self.push_hit(row, UiAction::BrowserClickRef(*ref_id));
                    by += 34;
                }
                crate::browser::PageBlock::Input { ref_id, placeholder, value, .. } => {
                    let row = UiRect::new(x + PAD, by, page_w - PAD * 2, 30);
                    canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 6, p.bg_input);
                    canvas.stroke_rounded_rect(row.x, row.y, row.w, row.h, 6, p.border_subtle);
                    canvas.draw_text(row.x + 10, by + 8, &format!("[e{}]", ref_id), FontRole::TerminalMono, p.accent_secondary);
                    let shown = if value.is_empty() { placeholder.clone() } else { value.clone() };
                    canvas.draw_text_clipped(row.x + 50, by + 8, row.w - 60, &shown, FontRole::TerminalMono, if value.is_empty() { p.text_muted } else { p.text_primary });
                    self.push_hit(row, UiAction::BrowserClickRef(*ref_id));
                    by += 36;
                }
                crate::browser::PageBlock::Divider => {
                    canvas.draw_hline(x + PAD, by + 8, page_w - PAD * 2, p.border_subtle);
                    by += 18;
                }
            }
        }
        canvas.pop_clip();

        // Vertical scrollbar for the page
        canvas.fill_rounded_rect(x + w - 6, page_y + 4, 4, page_h - 8, 2, p.bg_input);
        let thumb_h = 60.min(page_h - 8);
        let thumb_y = page_y + 4 + ((self.browser_scroll as i32).clamp(0, 400) / 4).min(page_h - thumb_h - 8);
        canvas.fill_rounded_rect(x + w - 6, thumb_y, 4, thumb_h, 2, mix(p.accent, p.bg_editor, 0.5));
    }
}

// ---------------------------------------------------------------------------
// Integrated terminal drawer
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_bottom_terminal(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        top: i32,
        w: i32,
        h: i32,
    ) {
        let p = self.palette.clone();
        canvas.fill_rect(x, top, w, h, p.bg_terminal);
        canvas.draw_hline(x, top, w, p.border_subtle);

        let bar_h = 32;
        canvas.fill_rect(x, top + 1, w, bar_h, p.bg_sidebar);
        canvas.draw_hline(x, top + bar_h, w, p.border_subtle);

        let mut tx = x + PAD;
        let term_titles: Vec<String> = self.terminals.iter().map(|t| t.title.clone()).collect();
        for (idx, title) in term_titles.into_iter().enumerate() {
            let tw = 132;
            if tx + tw > x + w - 260 {
                break;
            }
            let active = idx == self.active_term;
            canvas.fill_rounded_rect(
                tx,
                top + 5,
                tw,
                23,
                5,
                if active { p.bg_terminal } else { p.bg_card },
            );
            canvas.draw_text_clipped(
                tx + 10,
                top + 8,
                tw - 30,
                &title,
                FontRole::UiSemibold,
                if active { p.success } else { p.text_secondary },
            );
            canvas.draw_text(tx + tw - 18, top + 8, "x", FontRole::UiSmall, p.text_muted);
            self.push_hit(UiRect::new(tx, top + 5, tw - 22, 23), UiAction::SelectPtyTab(idx));
            self.push_hit(UiRect::new(tx + tw - 22, top + 5, 22, 23), UiAction::ClosePtyTab(idx));
            tx += tw + 6;
        }

        let ps_lbl = if cfg!(target_os = "windows") { "+ PS" } else { "+ Shell" };
        self.draw_button(
            canvas,
            UiRect::new(x + w - 312, top + 5, 70, 23),
            ps_lbl,
            p.bg_button,
            p.success,
            p.border_subtle,
            Some(UiAction::SpawnPtyTab(if cfg!(target_os = "windows") { "powershell" } else { "sh" })),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 238, top + 5, 62, 23),
            "+ CMD",
            p.bg_button,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::SpawnPtyTab("cmd")),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 172, top + 5, 52, 23),
            "CWD",
            p.bg_button,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::PtySyncCwd),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 116, top + 5, 46, 23),
            "^C",
            mix(p.danger, p.bg_button, 0.4),
            p.danger,
            p.border_subtle,
            Some(UiAction::PtySendCtrlC),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 66, top + 5, 54, 23),
            "Clear",
            p.bg_button,
            p.text_muted,
            p.border_subtle,
            Some(UiAction::PtyClearScreen),
        );

        let grid_y = top + bar_h + 4;
        let grid_h = (h - bar_h - 6).max(20);
        self.term_grid_rect = UiRect::new(x + 8, grid_y, (w - 16).max(40), grid_h);
        self.push_hit(self.term_grid_rect, UiAction::Focus(FocusTarget::Terminal));

        let (cw_raw, ch_raw) = canvas.mono_cell_size(FontRole::TerminalMono);
        let char_w = cw_raw.max(7);
        let row_h = ch_raw.max(16);
        let target_cols = (self.term_grid_rect.w / char_w).max(30) as usize;
        let target_rows = (self.term_grid_rect.h / row_h).max(5) as usize;

        if let Some(term) = self.terminals.get_mut(self.active_term) {
            if term.screen.cols != target_cols || term.screen.rows != target_rows {
                term.screen.resize(target_cols, target_rows);
                term.pty.resize(target_cols as u16, target_rows as u16);
            }
            canvas.push_clip(x + 4, grid_y, w - 8, grid_h);
            for r in 0..term.screen.rows {
                let ry = grid_y + r as i32 * row_h;
                let Some(line) = term.screen.visible_row(r) else { continue };
                let mut col = 0usize;
                while col < line.len() {
                    let style = line[col].clone();
                    let mut end = col + 1;
                    let mut run = String::new();
                    run.push(style.ch);
                    while end < line.len() && line[end].fg == style.fg && line[end].bg == style.bg {
                        run.push(line[end].ch);
                        end += 1;
                    }
                    let rx = self.term_grid_rect.x + col as i32 * char_w;
                    let rw = (end - col) as i32 * char_w;
                    if style.bg != p.bg_terminal {
                        canvas.fill_rect(rx, ry, rw, row_h, style.bg);
                    }
                    if run.chars().any(|c| c != ' ') {
                        canvas.draw_text(rx, ry, &run, FontRole::TerminalMono, style.fg);
                    }
                    col = end;
                }
            }
            if self.focus == FocusTarget::Terminal
                && self.modal == ModalDialog::None
                && term.screen.scroll_offset == 0
                && self.blink_phase
            {
                let cx = self.term_grid_rect.x + term.screen.cursor_col as i32 * char_w;
                let cy = grid_y + term.screen.cursor_row as i32 * row_h;
                canvas.fill_rect(cx, cy + 1, char_w, row_h - 2, p.success);
            }
            canvas.pop_clip();
        }
    }
}

// ---------------------------------------------------------------------------
// Right panel: agent chat + interactive prompt timeline
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_right_agent_panel(&mut self, canvas: &mut dyn SurfaceCanvas, top: i32, bottom: i32) {
        if self.right_w <= 40 {
            return;
        }
        let p = self.palette.clone();
        let x = self.width - self.right_w;
        let w = self.right_w;
        let h = bottom - top;
        canvas.fill_rect(x, top, w, h, p.bg_agent);
        canvas.draw_vline(x, top, h, p.border_subtle);

        // ---- Panel header ---------------------------------------------------
        canvas.fill_rect(x + 1, top, w - 1, 40, p.bg_sidebar);
        canvas.draw_hline(x + 1, top + 39, w - 1, p.border_subtle);
        canvas.draw_text_clipped(
            x + PAD,
            top + 8,
            w - 130,
            &truncate_right(&self.active_session.title, 26),
            FontRole::UiSemibold,
            p.text_primary,
        );
        canvas.draw_text(
            x + PAD,
            top + 25,
            &format!(
                "{} · {} · {}",
                if self.active_mode == "plan" { "PLAN" } else { "BUILD" },
                self.settings.agent_effort.to_uppercase(),
                truncate_right(&self.settings.active_model, 16)
            ),
            FontRole::UiSmall,
            p.text_muted,
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 118, top + 7, 54, 26),
            "+ New",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::NewSession),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 60, top + 7, 48, 26),
            "Fork",
            p.bg_button,
            p.purple,
            p.border_subtle,
            Some(UiAction::ForkActiveSession),
        );

        // ---- Chat / Timeline tabs -------------------------------------------
        let tabs_y = top + 40;
        let tab_w = (w - PAD * 2 - 8) / 2;
        self.draw_pill(
            canvas,
            UiRect::new(x + PAD, tabs_y + 4, tab_w, 24),
            "Chat",
            self.agent_panel_tab == AgentPanelTab::Chat,
            Some(UiAction::ExecuteCommandId("panel:chat".to_string())),
        );
        self.draw_pill(
            canvas,
            UiRect::new(x + PAD + tab_w + 8, tabs_y + 4, tab_w, 24),
            "Prompt Timeline",
            self.agent_panel_tab == AgentPanelTab::Timeline,
            Some(UiAction::ExecuteCommandId("panel:timeline".to_string())),
        );

        let comp_h = 148;
        let comp_top = bottom - comp_h;
        let feed_top = tabs_y + 34;
        let feed_bottom = comp_top - 8;

        match self.agent_panel_tab {
            AgentPanelTab::Chat => self.draw_agent_chat(canvas, x, w, feed_top, feed_bottom),
            AgentPanelTab::Timeline => self.draw_prompt_timeline(canvas, x, w, feed_top, feed_bottom),
        }

        // ---- Composer ---------------------------------------------------------
        canvas.fill_rect(x + 1, comp_top, w - 1, comp_h, p.bg_sidebar);
        canvas.draw_hline(x + 1, comp_top, w - 1, p.border_subtle);

        let chip_w = (w - PAD * 2 - 16) / 3;
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, comp_top + 8, chip_w, 22),
            "Explain",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::QuickPrompt(
                "Inspect the repository files and explain the architecture concisely.",
            )),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + chip_w + 8, comp_top + 8, chip_w, 22),
            "Diagnose",
            p.bg_card,
            p.warning,
            p.border_subtle,
            Some(UiAction::QuickPrompt(
                "Run diagnose_errors on this workspace and fix any compiler or test issues found.",
            )),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + (chip_w + 8) * 2, comp_top + 8, chip_w, 22),
            "Git Summary",
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::QuickPrompt(
                "Check git status & diff and summarize all uncommitted changes.",
            )),
        );

        let box_rect = UiRect::new(x + PAD, comp_top + 36, w - PAD * 2, 62);
        let focused = self.focus == FocusTarget::AgentPrompt && self.modal == ModalDialog::None;
        canvas.fill_rounded_rect(box_rect.x, box_rect.y, box_rect.w, box_rect.h, 8, p.bg_input);
        canvas.stroke_rounded_rect(
            box_rect.x,
            box_rect.y,
            box_rect.w,
            box_rect.h,
            8,
            if focused { p.border_focus } else { p.border_subtle },
        );
        self.push_hit(box_rect, UiAction::Focus(FocusTarget::AgentPrompt));

        let prompt_disp = if self.agent_prompt.is_empty() && !focused {
            "Ask AirCode to build, refactor, or debug — type / for commands".to_string()
        } else if focused && self.blink_phase {
            format!("{}|", self.agent_prompt)
        } else {
            self.agent_prompt.clone()
        };
        let wrap_cols = (((box_rect.w - 20) / 7).max(20)) as usize;
        let p_color = if self.agent_prompt.is_empty() && !focused {
            p.text_muted
        } else {
            p.text_primary
        };
        for (idx, pl) in wrap_text(&prompt_disp, wrap_cols).into_iter().rev().take(3).rev().enumerate() {
            canvas.draw_text_clipped(box_rect.x + 10, box_rect.y + 8 + idx as i32 * 17, box_rect.w - 20, &pl, FontRole::UiSmall, p_color);
        }

        canvas.draw_text(
            x + PAD,
            comp_top + 104,
            "Enter: run · Shift+Enter: newline · /: commands",
            FontRole::UiSmall,
            p.text_muted,
        );
        let run_label = if self.agent_running {
            "Stop"
        } else if self.compacting {
            "Compacting"
        } else {
            "Run"
        };
        self.draw_button(
            canvas,
            UiRect::new(x + w - PAD - 86, comp_top + 100, 86, 28),
            run_label,
            if self.agent_running { mix(p.danger, p.bg_button, 0.4) } else { p.accent },
            if self.agent_running { p.danger } else { p.on_accent_text(p.accent) },
            if self.agent_running { p.danger } else { p.accent },
            Some(if self.agent_running {
                UiAction::EmergencyStop
            } else {
                UiAction::SendAgentPrompt
            }),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, comp_top + 100, 86, 28),
            "Compact",
            p.bg_card,
            p.warning,
            p.border_subtle,
            Some(UiAction::CompactActiveSession),
        );
    }
}

impl StudioApp {
    fn draw_agent_chat(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        w: i32,
        feed_top: i32,
        feed_bottom: i32,
    ) {
        let p = self.palette.clone();
        let wrap_cols = ((w - 40).max(160) / 7) as usize;
        let mut blocks: Vec<ChatBlock> = Vec::new();
        self.msg_line_offsets.clear();

        if self.active_session.messages.is_empty() && self.live_tools.is_empty() {
            blocks.push(ChatBlock {
                bg: p.bg_card,
                fg: p.accent,
                text: "AirCode Agent — ready".to_string(),
                diff_rel: None,
                msg_idx: None,
            });
            for info in [
                "Plan mode researches & designs; Build mode edits files, runs shell & tests.",
                "Type / for the command palette: /zen /connect /theme /fork /compact /effort /skills /browser",
                "Ctrl+K opens fuzzy search over every command and file in the workspace.",
            ] {
                for wl in wrap_text(info, wrap_cols) {
                    blocks.push(ChatBlock {
                        bg: p.bg_card,
                        fg: p.text_secondary,
                        text: wl,
                        diff_rel: None,
                        msg_idx: None,
                    });
                }
            }
        }

        for (msg_idx, msg) in self.active_session.messages.iter().enumerate() {
            // A blank line above each turn keeps the feed calm instead of a solid wall.
            blocks.push(ChatBlock {
                bg: p.bg_agent,
                fg: p.text_muted,
                text: String::new(),
                diff_rel: None,
                msg_idx: None,
            });
            self.msg_line_offsets.push(blocks.len());
            let is_user = msg.role == "user";
            // Barely-tinted user bubble: present, never loud.
            let bg = if is_user { mix(p.accent, p.bg_agent, 0.06) } else { p.bg_card };
            let hdr_fg = if is_user { p.accent } else { p.success };
            let stamp = crate::sqlite::short_time_label(&msg.created_at);
            blocks.push(ChatBlock {
                bg,
                fg: hdr_fg,
                text: format!(
                    "{} {}  [{}]",
                    if is_user { glyph("user") } else { glyph("spark") },
                    if is_user { "YOU" } else { "AIRCODE" },
                    stamp
                ),
                diff_rel: None,
                msg_idx: Some(msg_idx),
            });

            let thinking_hidden = !self.settings.show_thinking
                || self.hidden_thinking_ids.iter().any(|m| *m == msg.id);
            if !msg.thinking.trim().is_empty() {
                if thinking_hidden {
                    blocks.push(ChatBlock {
                        bg,
                        fg: p.text_muted,
                        text: format!("{} thinking hidden — /thinking to show", glyph("brain")),
                        diff_rel: None,
                        msg_idx: Some(msg_idx),
                    });
                } else {
                    blocks.push(ChatBlock {
                        bg,
                        fg: p.purple,
                        text: format!("{} thinking", glyph("brain")),
                        diff_rel: None,
                        msg_idx: Some(msg_idx),
                    });
                    for rl in wrap_text(&format!("    {}", msg.thinking.trim()), wrap_cols)
                        .into_iter()
                        .take(6)
                    {
                        blocks.push(ChatBlock {
                            bg,
                            fg: mix(p.purple, p.text_secondary, 0.35),
                            text: rl,
                            diff_rel: None,
                            msg_idx: Some(msg_idx),
                        });
                    }
                }
            }

            for tool in &msg.tools {
                let st_col = if tool.status == "ok" {
                    p.success
                } else {
                    p.danger
                };
                let diff_rel = if tool.file_path.is_empty() {
                    None
                } else {
                    Some(tool.file_path.clone())
                };
                blocks.push(ChatBlock {
                    bg: mix(p.bg_card, p.bg_agent, 0.5),
                    fg: st_col,
                    text: format!(
                        "{} {} · {}ms",
                        glyph("wrench"),
                        tool.tool_name,
                        tool.duration_ms
                    ),
                    diff_rel,
                    msg_idx: Some(msg_idx),
                });
                if !tool.result_summary.is_empty() {
                    for tl in wrap_text(&tool.result_summary, wrap_cols).into_iter().take(2) {
                        blocks.push(ChatBlock {
                            bg: mix(p.bg_card, p.bg_agent, 0.5),
                            fg: p.text_secondary,
                            text: tl,
                            diff_rel: None,
                            msg_idx: Some(msg_idx),
                        });
                    }
                }
            }

            for wl in wrap_text(&msg.content, wrap_cols).into_iter().take(60) {
                blocks.push(ChatBlock {
                    bg,
                    fg: p.text_primary,
                    text: wl,
                    diff_rel: None,
                    msg_idx: Some(msg_idx),
                });
            }
            blocks.push(ChatBlock {
                bg: p.bg_agent,
                fg: p.text_muted,
                text: String::new(),
                diff_rel: None,
                msg_idx: Some(msg_idx),
            });
        }

        // Live streaming state
        if !self.live_tools.is_empty() || !self.streaming_text.is_empty() {
            for rec in &self.live_tools {
                let st_col = match rec.status.as_str() {
                    "ok" => p.success,
                    "running" => p.warning,
                    _ => p.danger,
                };
                let diff_rel = if rec.file_path.is_empty() {
                    None
                } else {
                    Some(rec.file_path.clone())
                };
                blocks.push(ChatBlock {
                    bg: mix(p.bg_card, p.bg_agent, 0.5),
                    fg: st_col,
                    text: format!(
                        "{} live {} · {}ms",
                        glyph("wrench"),
                        rec.tool_name,
                        rec.duration_ms
                    ),
                    diff_rel,
                    msg_idx: None,
                });
                if !rec.result_summary.is_empty() {
                    for tl in wrap_text(&rec.result_summary, wrap_cols).into_iter().take(2) {
                        blocks.push(ChatBlock {
                            bg: mix(p.bg_card, p.bg_agent, 0.5),
                            fg: p.text_secondary,
                            text: tl,
                            diff_rel: None,
                            msg_idx: None,
                        });
                    }
                }
            }
            if !self.streaming_thinking.is_empty() && self.settings.show_thinking {
                blocks.push(ChatBlock {
                    bg: p.bg_card,
                    fg: p.purple,
                    text: format!("{} thinking...", glyph("brain")),
                    diff_rel: None,
                    msg_idx: None,
                });
                for rl in wrap_text(&self.streaming_thinking, wrap_cols).into_iter().take(6) {
                    blocks.push(ChatBlock {
                        bg: p.bg_card,
                        fg: mix(p.purple, p.text_secondary, 0.35),
                        text: rl,
                        diff_rel: None,
                        msg_idx: None,
                    });
                }
            }
            if !self.streaming_text.is_empty() {
                for wl in wrap_text(&self.streaming_text, wrap_cols).into_iter().take(40) {
                    blocks.push(ChatBlock {
                        bg: p.bg_card,
                        fg: p.text_primary,
                        text: wl,
                        diff_rel: None,
                        msg_idx: None,
                    });
                }
            }
        }

        let line_h = 21;
        let vis_lines = ((feed_bottom - feed_top).max(20) / line_h) as usize;
        let total = blocks.len();
        let max_off = total.saturating_sub(vis_lines);
        let start_idx = if self.agent_scroll_lines == 0 {
            max_off
        } else {
            max_off.saturating_sub(self.agent_scroll_lines.min(max_off))
        };

        canvas.push_clip(x + 2, feed_top, w - 4, (feed_bottom - feed_top).max(10));
        let mut cur_msg: Option<usize> = None;
        for (i, blk) in blocks
            .into_iter()
            .skip(start_idx)
            .take(vis_lines)
            .enumerate()
        {
            let ly = feed_top + i as i32 * line_h;
            if blk.msg_idx != cur_msg {
                cur_msg = blk.msg_idx;
            }
            if blk.bg != p.bg_agent {
                canvas.fill_rect(x + 8, ly, w - 16, line_h, blk.bg);
            }
            let text_max_w = if blk.diff_rel.is_some() { w - 96 } else { w - 24 };
            canvas.draw_text_clipped(x + 14, ly + 1, text_max_w, &blk.text, FontRole::UiSmall, blk.fg);
            if let Some(rel) = blk.diff_rel {
                self.draw_button(
                    canvas,
                    UiRect::new(x + w - 78, ly + 1, 66, line_h - 3),
                    "Diff",
                    p.bg_button,
                    p.accent_secondary,
                    p.border_subtle,
                    Some(UiAction::GitOpenDiff(rel)),
                );
            }
            if let Some(idx) = blk.msg_idx {
                if i > 0 {
                    let fork_rect = UiRect::new(x + w - 14, ly + 1, 12, line_h - 3);
                    self.push_hit(fork_rect, UiAction::ForkFromMessage(idx));
                }
            }
        }
        canvas.pop_clip();

        // Status strip between feed and composer
        let st_col = if self.agent_running {
            p.warning
        } else if self.compacting {
            p.accent_secondary
        } else {
            p.text_muted
        };
        canvas.fill_rect(x + 1, feed_bottom, w - 1, 22, p.bg_sidebar);
        canvas.draw_text_clipped(
            x + PAD,
            feed_bottom + 3,
            w - PAD * 2,
            &truncate_right(&self.agent_status_line, (w - 24) as usize / 6),
            FontRole::UiSmall,
            st_col,
        );
    }

    fn draw_prompt_timeline(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        w: i32,
        feed_top: i32,
        feed_bottom: i32,
    ) {
        let p = self.palette.clone();
        canvas.fill_rect(x + 2, feed_top, w - 4, feed_bottom - feed_top, p.bg_agent);

        self.draw_section_title(
            canvas,
            x + PAD,
            feed_top + 6,
            w - 24,
            &format!(
                "PROMPT TIMELINE — {} turns · {} compacted",
                self.active_session.messages.len(),
                self.active_session.compacted_count
            ),
        );

        let row_h = 46;
        let avail = ((feed_bottom - feed_top - 26).max(20) / row_h) as usize;
        let total = self.active_session.messages.len();
        let max_scroll = total.saturating_sub(avail.max(1));
        if self.timeline_scroll > max_scroll {
            self.timeline_scroll = max_scroll;
        }
        let msgs: Vec<(usize, crate::sqlite::ChatMessage)> = self
            .active_session
            .messages
            .iter()
            .enumerate()
            .skip(self.timeline_scroll)
            .take(avail)
            .map(|(i, m)| (i, m.clone()))
            .collect();

        canvas.push_clip(x + 2, feed_top + 22, w - 4, feed_bottom - feed_top - 22);
        for (idx, msg) in msgs.into_iter() {
            let ry = feed_top + 22 + idx as i32 * row_h - self.timeline_scroll as i32 * row_h;
            let card = UiRect::new(x + PAD - 2, ry, w - PAD * 2 + 4, row_h - 6);
            let is_user = msg.role == "user";
            let hovered = card.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(
                card.x,
                card.y,
                card.w,
                card.h,
                8,
                if hovered { p.bg_card_hover } else { p.bg_card },
            );
            canvas.fill_rounded_rect(card.x + 8, card.y + 10, 22, 22, 6, if is_user { mix(p.accent, p.bg_card, 0.3) } else { p.bg_card_hover });
            canvas.draw_text(
                card.x + 12,
                card.y + 14,
                &format!("{}", idx + 1),
                FontRole::UiSemibold,
                if is_user { p.accent } else { p.success },
            );
            let stamp = crate::sqlite::short_time_label(&msg.created_at);
            canvas.draw_text(card.x + 38, card.y + 5, &stamp, FontRole::UiSmall, p.accent_secondary);
            let preview: String = msg.content.chars().take(64).collect();
            canvas.draw_text_clipped(
                card.x + 38,
                card.y + 20,
                card.w - 48,
                &if is_user {
                    format!("{} {}", glyph("user"), preview)
                } else {
                    format!("{} {}", glyph("spark"), preview)
                },
                FontRole::UiSmall,
                p.text_secondary,
            );
            if !msg.tools.is_empty() {
                canvas.draw_text(
                    card.x + card.w - 78,
                    card.y + 5,
                    &format!("{} tools", msg.tools.len()),
                    FontRole::UiSmall,
                    p.warning,
                );
            }
            // Jump to this turn in the chat
            self.push_hit(card, UiAction::ScrollToMessage(idx));
            // Fork from this prompt
            let fork_rect = UiRect::new(card.x + card.w - 74, card.y + card.h - 20, 66, 17);
            self.draw_button(
                canvas,
                fork_rect,
                "Fork",
                mix(p.purple, p.bg_card, 0.35),
                p.purple,
                p.border_subtle,
                Some(UiAction::ForkFromMessage(idx)),
            );
        }
        canvas.pop_clip();

        // Timeline action row
        self.draw_button(
            canvas,
            UiRect::new(x + PAD, feed_bottom - 26, (w - PAD * 2 - 8) / 2, 22),
            "Compact Session",
            p.bg_card,
            p.warning,
            p.border_subtle,
            Some(UiAction::CompactActiveSession),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + PAD + (w - PAD * 2 - 8) / 2 + 8, feed_bottom - 26, (w - PAD * 2 - 8) / 2, 22),
            "Fork Whole Session",
            p.bg_card,
            p.purple,
            p.border_subtle,
            Some(UiAction::ForkActiveSession),
        );
    }

    fn draw_status_footer(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let p = self.palette.clone();
        let h = FOOTER_H;
        let y = self.height - h;
        canvas.fill_rect(0, y, self.width, h, p.bg_header);
        canvas.draw_hline(0, y, self.width, p.border_subtle);

        let left_info = format!(
            "  {} {}   ·   {} mode   ·   {} {}   ·   {} effort   ·   {}",
            glyph("branch"),
            if self.git_snapshot.is_repo {
                &self.git_snapshot.branch
            } else {
                "no-git"
            },
            self.active_mode.to_uppercase(),
            self.settings.active_provider,
            self.settings.active_model,
            self.settings.agent_effort.to_uppercase(),
            self.palette.name,
        );
        canvas.draw_text_clipped(6, y + 5, self.width / 2, &left_info, FontRole::UiSmall, p.text_secondary);

        let right_info = if self.toast_ticks > 0 {
            truncate_right(&self.toast_text, (self.width / 2 / 6) as usize)
        } else if let Some(tab) = self.tabs.get(self.active_tab) {
            if let TabContent::Code {
                lines,
                cursor_line,
                cursor_col,
                ..
            } = &tab.content
            {
                format!(
                    "Ln {}, Col {}  ·  {} lines  ·  {}",
                    cursor_line + 1,
                    cursor_col + 1,
                    lines.len(),
                    tab.rel_path
                )
            } else {
                "Side-by-Side Diff Viewer  ·  Pure-Rust Native GUI".to_string()
            }
        } else {
            "100% Pure-Rust Native GUI — no WebView, no HTML/JS".to_string()
        };
        let tw = canvas.measure_text_width(&right_info, FontRole::UiSmall);
        canvas.draw_text_clipped(
            (self.width - tw - 14).max(self.width / 2),
            y + 5,
            self.width / 2,
            &right_info,
            FontRole::UiSmall,
            p.text_secondary,
        );
    }
}

// ---------------------------------------------------------------------------
// Overlays: command palette, /connect wizard, theme picker, safety settings
// ---------------------------------------------------------------------------
impl StudioApp {
    fn draw_command_palette(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let p = self.palette.clone();
        let mw = 720.min(self.width - 80);
        let mh = 460.min(self.height - 120);
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 3;
        canvas.fill_rect(0, 0, self.width, self.height, 0x40000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.accent);

        canvas.draw_text(mx + 20, my + 16, "Command Palette", FontRole::UiTitle, p.text_primary);
        canvas.draw_text(
            mx + 20,
            my + 38,
            "Fuzzy search commands & files — ↑↓ to navigate, Enter to run, Esc to close",
            FontRole::UiSmall,
            p.text_muted,
        );

        let inp = UiRect::new(mx + 20, my + 58, mw - 40, 36);
        self.draw_text_input(
            canvas,
            inp,
            self.palette_query.clone().as_str(),
            "> type a command, or a file name...",
            true,
            FontRole::UiRegular,
            UiAction::Focus(FocusTarget::CommandPaletteInput),
        );

        let items = self.palette_entries();
        let list_y = my + 112;
        let row_h = 44;
        let avail = ((mh - 130) / row_h).max(3) as usize;
        canvas.push_clip(mx, list_y, mw, mh - 120);
        for (i, item) in items.iter().take(avail).enumerate() {
            let ry = list_y + i as i32 * row_h;
            let row = UiRect::new(mx + 12, ry, mw - 24, row_h - 4);
            let selected = i == self.palette_index;
            let hovered = row.contains(self.mouse_x, self.mouse_y);
            if selected {
                // Selected = a tint plus a left rule, not a filled slab + outline.
                canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 8, mix(p.accent, p.bg_modal, 0.10));
                canvas.fill_rect(row.x, row.y + 6, 3, row.h - 12, p.accent);
            } else if hovered {
                canvas.fill_rounded_rect(row.x, row.y, row.w, row.h, 8, p.bg_card);
            }
            let badge = if item.is_file { "FILE" } else { "CMD" };
            canvas.fill_rounded_rect(
                row.x + 8,
                row.y + 8,
                42,
                18,
                5,
                if item.is_file {
                    p.bg_button
                } else {
                    mix(p.accent, p.bg_modal, 0.20)
                },
            );
            canvas.draw_text(
                row.x + 14,
                row.y + 10,
                badge,
                FontRole::UiSmall,
                if item.is_file {
                    p.text_muted
                } else {
                    p.accent
                },
            );
            canvas.draw_text_clipped(
                row.x + 58,
                row.y + 5,
                row.w - 150,
                &truncate_right(&item.title, 52),
                FontRole::UiSemibold,
                p.text_primary,
            );
            canvas.draw_text_clipped(
                row.x + 58,
                row.y + 21,
                row.w - 150,
                &truncate_right(&item.subtitle, 62),
                FontRole::UiSmall,
                p.text_muted,
            );
            if !item.shortcut.is_empty() {
                canvas.draw_text_clipped(
                    row.x + row.w - 84,
                    row.y + 12,
                    78,
                    &item.shortcut,
                    FontRole::UiSmall,
                    p.accent_secondary,
                );
            }
            self.push_hit(row, UiAction::ExecuteCommandId(item.id.clone()));
        }
        canvas.pop_clip();
    }

    fn draw_connect_wizard(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let p = self.palette.clone();
        // Tall enough for the Zen free-lane card plus every provider option.
        let mw = 820.min(self.width - 60);
        let mh = 640.min(self.height - 40);
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;
        canvas.fill_rect(0, 0, self.width, self.height, 0x40000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.accent);

        canvas.draw_text(mx + 24, my + 18, "Connect AI Provider", FontRole::UiTitle, p.text_primary);
        canvas.draw_text(
            mx + 24,
            my + 42,
            "Guided setup — auto-detect local keys, paste from clipboard, or use the OpenCode auth workaround.",
            FontRole::UiSmall,
            p.text_secondary,
        );

        // --- OpenCode Zen free lane: the headline option, not a footnote ---------
        let zen_card = UiRect::new(mx + 24, my + 62, mw - 48, 62);
        canvas.fill_rounded_rect(zen_card.x, zen_card.y, zen_card.w, zen_card.h, 10, mix(p.success, p.bg_modal, 0.10));
        canvas.stroke_rounded_rect(zen_card.x, zen_card.y, zen_card.w, zen_card.h, 10, mix(p.success, p.bg_modal, 0.45));
        canvas.draw_text(
            zen_card.x + 14,
            zen_card.y + 9,
            &format!("{}  OpenCode Zen — free lane, no API key", glyph("spark")),
            FontRole::UiSemibold,
            p.text_primary,
        );
        canvas.draw_text_clipped(
            zen_card.x + 14,
            zen_card.y + 28,
            zen_card.w - 210,
            "AirCode authenticates exactly like the official OpenCode client, so the anonymous public tier works with zero setup.",
            FontRole::UiSmall,
            p.text_secondary,
        );
        let models_line = if self.connect.zen_free_models.is_empty() {
            "big-pickle, glm-5.3-flash-free, deepseek-v4.1-flash-free, nemotron-3.5-lightning-free, ...".to_string()
        } else {
            format!(
                "{} free models: {}",
                self.connect.zen_free_models.len(),
                self.connect
                    .zen_free_models
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        canvas.draw_text_clipped(
            zen_card.x + 14,
            zen_card.y + 45,
            zen_card.w - 210,
            &models_line,
            FontRole::UiSmall,
            p.text_muted,
        );
        self.draw_button(
            canvas,
            UiRect::new(zen_card.x + zen_card.w - 178, zen_card.y + 12, 164, 24),
            "Use free lane",
            mix(p.success, p.bg_modal, 0.22),
            p.success,
            mix(p.success, p.bg_modal, 0.45),
            Some(UiAction::UseZenFreeLane),
        );
        self.draw_button(
            canvas,
            UiRect::new(zen_card.x + zen_card.w - 178, zen_card.y + 38, 164, 20),
            "Refresh catalog",
            p.bg_card,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::FetchZenFreeModels),
        );

        // Provider cards
        let guides = PROVIDER_GUIDES;
        let card_w = (mw - 48 - 12 * (guides.len() as i32 - 1)) / guides.len() as i32;
        let card_h = 88;
        for (i, g) in guides.iter().enumerate() {
            let cx = mx + 24 + i as i32 * (card_w + 12);
            let rect = UiRect::new(cx, my + 134, card_w, card_h);
            let selected = self.connect.provider_id == g.id;
            let hovered = rect.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                8,
                if selected { mix(p.accent, p.bg_modal, 0.2) } else if hovered { p.bg_card_hover } else { p.bg_card },
            );
            canvas.stroke_rounded_rect(rect.x, rect.y, rect.w, rect.h, 8, if selected { p.accent } else { p.border_subtle });
            canvas.draw_text_clipped(rect.x + 10, rect.y + 8, rect.w - 20, g.display_name, FontRole::UiSemibold, p.text_primary);
            canvas.fill_rounded_rect(rect.x + 10, rect.y + 26, 78, 16, 5, p.bg_button);
            canvas.draw_text(rect.x + 14, rect.y + 28, g.badge, FontRole::UiSmall, p.accent_secondary);
            for (li, line) in wrap_text(g.tagline, (((rect.w - 20) / 6).max(14)) as usize).into_iter().take(3).enumerate() {
                canvas.draw_text_clipped(rect.x + 10, rect.y + 48 + li as i32 * 14, rect.w - 20, &line, FontRole::UiSmall, p.text_muted);
            }
            self.push_hit(rect, UiAction::SelectConnectProvider(g.id));
        }

        let guide = self.connect_provider_guide();
        let mut y = my + 134 + card_h + 22;

        // Detected credentials
        canvas.draw_text(mx + 24, y, "Auto-detected credentials", FontRole::UiSemibold, p.text_primary);
        y += 20;
        if self.connect.detected_summary.is_empty() {
            canvas.draw_text(
                mx + 24,
                y,
                "Nothing detected yet — env vars, ~/.local/share/opencode/auth.json and local servers are scanned.",
                FontRole::UiSmall,
                p.text_muted,
            );
        } else {
            for s in self.connect.detected_summary.clone().into_iter().take(4) {
                canvas.draw_text(mx + 24, y, &format!("{} {}", glyph("check"), s), FontRole::UiSmall, p.success);
                y += 16;
            }
        }
        y += 6;
        let half = (mw - 48 - 12) / 2;
        self.draw_button(
            canvas,
            UiRect::new(mx + 24, y, half, 28),
            "Import Detected Auth",
            p.bg_card,
            p.success,
            p.border_subtle,
            Some(UiAction::ImportDetectedAuth),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 24 + half + 12, y, half, 28),
            "Spawn opencode serve (bridge)",
            p.bg_card,
            p.purple,
            p.border_subtle,
            Some(UiAction::SpawnLocalOpenCodeServe),
        );
        y += 40;

        // Key entry row
        canvas.draw_text(mx + 24, y, "API key / token", FontRole::UiSemibold, p.text_primary);
        y += 20;
        let key_w = mw - 48 - 120;
        let key_rect = UiRect::new(mx + 24, y, key_w, 32);
        self.draw_text_input(
            canvas,
            key_rect,
            self.connect.key_input.clone().as_str(),
            "sk-... (paste your key, or leave empty for OpenCode Zen public mode)",
            self.focus == FocusTarget::ConnectKeyInput,
            FontRole::TerminalMono,
            UiAction::Focus(FocusTarget::ConnectKeyInput),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 24 + key_w + 8, y, 112, 32),
            "Paste",
            p.bg_button,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::PasteKeyFromClipboard),
        );
        y += 44;

        // Model + URL row
        let model_rect = UiRect::new(mx + 24, y, (mw - 60) / 2, 30);
        self.draw_text_input(
            canvas,
            model_rect,
            self.connect.model.clone().as_str(),
            "model id",
            self.focus == FocusTarget::ConnectModelInput,
            FontRole::TerminalMono,
            UiAction::Focus(FocusTarget::ConnectModelInput),
        );
        let url_rect = UiRect::new(mx + 24 + (mw - 60) / 2 + 12, y, (mw - 60) / 2, 30);
        self.draw_text_input(
            canvas,
            url_rect,
            self.connect.url_input.clone().as_str(),
            "http://127.0.0.1:4096",
            self.focus == FocusTarget::ConnectUrlInput,
            FontRole::TerminalMono,
            UiAction::Focus(FocusTarget::ConnectUrlInput),
        );
        y += 42;

        // OpenCode auth mode
        canvas.draw_text(mx + 24, y, "OpenCode auth mode", FontRole::UiSemibold, p.text_primary);
        y += 22;
        let mode_w = (mw - 48 - 24) / 3;
        self.draw_pill(
            canvas,
            UiRect::new(mx + 24, y, mode_w, 28),
            "Zen (public key)",
            self.connect.opencode_auth_mode == "zen",
            Some(UiAction::SetOpenCodeAuthMode("zen")),
        );
        self.draw_pill(
            canvas,
            UiRect::new(mx + 24 + mode_w + 12, y, mode_w, 28),
            "opencode serve",
            self.connect.opencode_auth_mode == "local-serve",
            Some(UiAction::SetOpenCodeAuthMode("local-serve")),
        );
        self.draw_pill(
            canvas,
            UiRect::new(mx + 24 + (mode_w + 12) * 2, y, mode_w, 28),
            "Ollama local",
            self.connect.provider_id == "ollama",
            Some(UiAction::SelectConnectProvider("ollama")),
        );
        y += 34;

        // Footer actions
        let help = format!(
            "{} {} — {}. Keys are stored in your local SQLite settings only.",
            glyph("lock"),
            guide.display_name,
            guide.tagline
        );
        canvas.draw_text_clipped(mx + 24, y, mw - 48, &help, FontRole::UiSmall, p.text_muted);
        let fy = my + mh - 52;
        self.draw_button(
            canvas,
            UiRect::new(mx + 24, fy, 200, 34),
            &format!("Open {}", guide.display_name),
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::OpenProviderKeyUrl(guide.key_url.to_string(), true)),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 232, fy, 170, 34),
            "Preview in Browser",
            p.bg_card,
            p.purple,
            p.border_subtle,
            Some(UiAction::OpenProviderKeyUrl(guide.key_url.to_string(), false)),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 240, fy, 100, 34),
            "Cancel",
            p.bg_button,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::CloseModal),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 132, fy, 108, 34),
            "Connect",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::SaveConnectWizard),
        );
    }

    fn draw_theme_picker(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let p = self.palette.clone();
        let mw = 620.min(self.width - 60);
        let mh = 400.min(self.height - 80);
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;
        canvas.fill_rect(0, 0, self.width, self.height, 0x40000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.accent);
        canvas.draw_text(mx + 24, my + 18, "Color Theme", FontRole::UiTitle, p.text_primary);
        canvas.draw_text(
            mx + 24,
            my + 42,
            "Pick a built-in theme, or drop a custom .aircode/theme.json into your workspace.",
            FontRole::UiSmall,
            p.text_secondary,
        );

        let themes = ThemePalette::builtin_list();
        let row_h = 46;
        for (i, (id, name)) in themes.iter().enumerate() {
            let ry = my + 68 + i as i32 * row_h;
            let rect = UiRect::new(mx + 24, ry, mw - 48, row_h - 8);
            let selected = self.settings.theme == *id;
            let hovered = rect.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                8,
                if selected { mix(p.accent, p.bg_modal, 0.18) } else if hovered { p.bg_card_hover } else { p.bg_card },
            );
            canvas.stroke_rounded_rect(rect.x, rect.y, rect.w, rect.h, 8, if selected { p.accent } else { p.border_subtle });

            // Live color preview swatches
            let swatch = ThemePalette::by_id(id, Some(&self.workspace));
            let mut sx = rect.x + 12;
            for col in [swatch.bg_app, swatch.bg_editor, swatch.accent, swatch.success, swatch.warning, swatch.purple] {
                canvas.fill_rounded_rect(sx, rect.y + 12, 16, 16, 4, col);
                sx += 20;
            }
            canvas.draw_text(rect.x + 150, rect.y + 12, name, FontRole::UiSemibold, p.text_primary);
            if *id == "custom" {
                canvas.draw_text(rect.x + 330, rect.y + 13, ".aircode/theme.json", FontRole::UiSmall, p.text_muted);
            }
            if selected {
                canvas.draw_text(rect.x + rect.w - 76, rect.y + 12, "ACTIVE", FontRole::UiSmall, p.accent);
            }
            self.push_hit(rect, UiAction::ApplyTheme(id.to_string()));
        }

        let fy = my + mh - 52;
        canvas.draw_text_clipped(
            mx + 24,
            fy + 10,
            mw - 190,
            "Themes apply instantly to the editor, terminal, agent panel & browser.",
            FontRole::UiSmall,
            p.text_muted,
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 150, fy, 126, 34),
            "Done",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::CloseModal),
        );
    }

    fn draw_safety_modal(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let p = self.palette.clone();
        let mw = 560.min(self.width - 60);
        let mh = 400.min(self.height - 80);
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;
        canvas.fill_rect(0, 0, self.width, self.height, 0x40000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.accent);
        canvas.draw_text(mx + 24, my + 18, "Safety & Editor Preferences", FontRole::UiTitle, p.text_primary);
        canvas.draw_text(
            mx + 24,
            my + 42,
            "Permission gates, sandbox scope and editor typography.",
            FontRole::UiSmall,
            p.text_secondary,
        );

        let toggles: Vec<(&str, &str, bool)> = vec![
            ("require_shell_approval", "Require approval before shell commands", self.settings.require_shell_approval),
            ("require_delete_confirmation", "Confirm before deleting files", self.settings.require_delete_confirmation),
            ("restrict_to_workspace", "Restrict file tools to the workspace", self.settings.restrict_to_workspace),
            ("show_thinking", "Show AI thinking blocks", self.settings.show_thinking),
            ("minimap", "Show editor minimap", self.show_minimap),
        ];
        let mut y = my + 72;
        for (field, label, on) in toggles {
            let rect = UiRect::new(mx + 24, y, mw - 48, 34);
            let hovered = rect.contains(self.mouse_x, self.mouse_y);
            if hovered {
                canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 6, p.bg_card);
            }
            canvas.draw_text(rect.x + 12, rect.y + 9, label, FontRole::UiSmall, p.text_primary);
            let sw = UiRect::new(rect.x + rect.w - 62, rect.y + 6, 50, 22);
            canvas.fill_rounded_rect(sw.x, sw.y, sw.w, sw.h, 11, if on { p.success } else { p.bg_button });
            canvas.fill_rounded_rect(
                sw.x + if on { 28 } else { 4 },
                sw.y + 3,
                16,
                16,
                8,
                if on { p.on_accent_text(p.success) } else { p.text_muted },
            );
            canvas.draw_text(sw.x + 6, sw.y + 4, if on { "ON" } else { "OFF" }, FontRole::UiSmall, if on { p.on_accent_text(p.success) } else { p.text_muted });
            self.push_hit(rect, UiAction::ToggleSettingBool(field));
            y += 38;
        }

        // Font size
        y += 6;
        canvas.draw_text(mx + 24, y + 6, "Editor font size", FontRole::UiSemibold, p.text_primary);
        self.draw_button(
            canvas,
            UiRect::new(mx + 190, y, 40, 28),
            "-",
            p.bg_button,
            p.text_primary,
            p.border_subtle,
            Some(UiAction::AdjustFontSize(-1)),
        );
        canvas.draw_text(mx + 240, y + 6, &format!("{} px", self.settings.editor_font_size), FontRole::UiSemibold, p.accent);
        self.draw_button(
            canvas,
            UiRect::new(mx + 300, y, 40, 28),
            "+",
            p.bg_button,
            p.text_primary,
            p.border_subtle,
            Some(UiAction::AdjustFontSize(1)),
        );

        let fy = my + mh - 52;
        self.draw_button(
            canvas,
            UiRect::new(mx + 24, fy, 160, 34),
            "Open /connect",
            p.bg_card,
            p.accent_secondary,
            p.border_subtle,
            Some(UiAction::OpenModal(ModalDialog::ConnectWizard)),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 150, fy, 126, 34),
            "Done",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::CloseModal),
        );
    }

    fn draw_approval_modal(&mut self, canvas: &mut dyn SurfaceCanvas, req: &PermissionPrompt) {
        let p = self.palette.clone();
        let mw = 520.min(self.width - 60);
        let mh = 240;
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;
        canvas.fill_rect(0, 0, self.width, self.height, 0x50000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.warning);
        canvas.draw_text(mx + 24, my + 20, "Permission Required", FontRole::UiTitle, p.warning);
        canvas.draw_text_clipped(mx + 24, my + 48, mw - 48, &req.title, FontRole::UiSemibold, p.text_primary);
        for (i, line) in wrap_text(&req.detail, ((mw - 60) / 7).max(20) as usize).into_iter().take(3).enumerate() {
            canvas.draw_text_clipped(mx + 24, my + 72 + i as i32 * 17, mw - 48, &line, FontRole::UiSmall, p.text_secondary);
        }
        canvas.fill_rounded_rect(mx + 24, my + 128, mw - 48, 30, 6, p.bg_input);
        canvas.draw_text_clipped(
            mx + 32,
            my + 135,
            mw - 64,
            &truncate_right(&req.command_or_path, 66),
            FontRole::TerminalMono,
            p.accent_secondary,
        );
        let bw = 120;
        self.draw_button(
            canvas,
            UiRect::new(mx + 24, my + mh - 52, bw, 34),
            "Deny",
            p.bg_button,
            p.danger,
            p.border_subtle,
            Some(UiAction::ApprovePermission {
                approved: false,
                remember: false,
            }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 24 + bw + 12, my + mh - 52, bw, 34),
            "Allow Once",
            p.accent,
            p.on_accent_text(p.accent),
            p.accent,
            Some(UiAction::ApprovePermission {
                approved: true,
                remember: false,
            }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 24 + (bw + 12) * 2, my + mh - 52, bw, 34),
            "Allow Always",
            mix(p.success, p.bg_button, 0.5),
            p.success,
            p.border_subtle,
            Some(UiAction::ApprovePermission {
                approved: true,
                remember: true,
            }),
        );
    }

    fn draw_delete_modal(&mut self, canvas: &mut dyn SurfaceCanvas, del: &DeleteConfirmState) {
        let p = self.palette.clone();
        let mw = 460.min(self.width - 60);
        let mh = 176;
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;
        canvas.fill_rect(0, 0, self.width, self.height, 0x50000000);
        canvas.fill_rounded_rect(mx, my, mw, mh, 14, p.bg_modal);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 14, p.danger);
        canvas.draw_text(mx + 24, my + 20, "Confirm Delete", FontRole::UiTitle, p.danger);
        canvas.draw_text_clipped(
            mx + 24,
            my + 48,
            mw - 48,
            &format!("Delete '{}'? This cannot be undone.", del.rel_path),
            FontRole::UiSmall,
            p.text_secondary,
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 24, my + mh - 56, 110, 34),
            "Cancel",
            p.bg_button,
            p.text_secondary,
            p.border_subtle,
            Some(UiAction::ConfirmFileDelete { confirmed: false }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 134, my + mh - 56, 110, 34),
            "Delete",
            mix(p.danger, p.bg_button, 0.5),
            p.danger,
            p.border_subtle,
            Some(UiAction::ConfirmFileDelete { confirmed: true }),
        );
    }
}

// ---------------------------------------------------------------------------
// Window event loop + top-level render
// ---------------------------------------------------------------------------
impl StudioApp {
    fn on_event(&mut self, event: WindowEvent) -> CursorShape {
        match event {
            WindowEvent::Resize { width, height } => {
                self.width = width.max(760);
                self.height = height.max(520);
            }
            WindowEvent::Tick => {
                self.tick_counter = self.tick_counter.wrapping_add(1);
                self.drain_background_streams();
                if self.tick_counter % 15 == 0 {
                    self.blink_phase = !self.blink_phase;
                }
                if self.toast_ticks > 0 {
                    self.toast_ticks -= 1;
                }
            }
            WindowEvent::MouseMove { x, y, .. } => {
                self.mouse_x = x;
                self.mouse_y = y;

                if let Some(sp) = &self.active_splitter {
                    match sp {
                        DragSplitter::LeftPanel => {
                            let max_left = (self.width - self.right_w - 320).max(220);
                            self.left_w = (x - RAIL_W).clamp(200, max_left);
                        }
                        DragSplitter::RightAgent => {
                            self.right_w = (self.width - x)
                                .clamp(300, (self.width - RAIL_W - self.left_w - 320).max(320));
                        }
                        DragSplitter::BottomTerminal => {
                            let bottom = self.height - FOOTER_H;
                            self.term_h = (bottom - y).clamp(120, (self.height - 240).max(140));
                        }
                    }
                } else if self.editor_dragging_selection {
                    let hit = if let Some(tab) = self.tabs.get(self.active_tab) {
                        if let TabContent::Code {
                            lines,
                            scroll_y,
                            scroll_x,
                            ..
                        } = &tab.content
                        {
                            Some(self.editor_hit_to_cursor(x, y, lines, *scroll_y, *scroll_x))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let (Some((row, col)), Some(tab)) = (hit, self.tabs.get_mut(self.active_tab)) {
                        if let TabContent::Code {
                            cursor_line,
                            cursor_col,
                            ..
                        } = &mut tab.content
                        {
                            *cursor_line = row;
                            *cursor_col = col;
                        }
                    }
                }
            }
            WindowEvent::MouseDown {
                x,
                y,
                button: MouseBtn::Left,
                mods,
            } => {
                self.mouse_x = x;
                self.mouse_y = y;
                self.blink_phase = true;

                let mut clicked_action = None;
                for zone in self.hit_zones.iter().rev() {
                    if zone.rect.contains(x, y) {
                        clicked_action = Some(zone.action.clone());
                        break;
                    }
                }
                if let Some(act) = clicked_action {
                    self.execute_action(act);
                    return self.compute_cursor();
                }

                if self.modal != ModalDialog::None
                    || self.pending_approval.is_some()
                    || self.pending_delete.is_some()
                {
                    return self.compute_cursor();
                }

                let top = SHELL_CHROME_TOP;
                let bottom = self.height - SHELL_CHROME_BOTTOM;
                if y >= top && y < bottom {
                    let panel_x = if self.left_panel_open {
                        RAIL_W + self.left_w
                    } else {
                        RAIL_W
                    };
                    if self.left_panel_open && (x - panel_x).abs() <= 4 {
                        self.active_splitter = Some(DragSplitter::LeftPanel);
                        return CursorShape::ResizeH;
                    }
                    if (x - (self.width - self.right_w)).abs() <= 4 {
                        self.active_splitter = Some(DragSplitter::RightAgent);
                        return CursorShape::ResizeH;
                    }
                    if self.terminal_open {
                        let term_top = bottom - self.term_h;
                        if x > panel_x
                            && x < self.width - self.right_w
                            && (y - term_top).abs() <= 4
                        {
                            self.active_splitter = Some(DragSplitter::BottomTerminal);
                            return CursorShape::ResizeV;
                        }
                    }
                }

                // Center editor click-to-caret
                if self.center_view == CenterView::Editor && self.editor_rect.contains(x, y) {
                    self.focus = FocusTarget::Editor;
                    let hit = if let Some(tab) = self.tabs.get(self.active_tab) {
                        if let TabContent::Code {
                            lines,
                            scroll_y,
                            scroll_x,
                            ..
                        } = &tab.content
                        {
                            Some(self.editor_hit_to_cursor(x, y, lines, *scroll_y, *scroll_x))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let (Some((row, col)), Some(tab)) = (hit, self.tabs.get_mut(self.active_tab)) {
                        if let TabContent::Code {
                            cursor_line,
                            cursor_col,
                            sel_anchor,
                            ..
                        } = &mut tab.content
                        {
                            if mods.shift {
                                if sel_anchor.is_none() {
                                    *sel_anchor = Some((*cursor_line, *cursor_col));
                                }
                            } else {
                                *sel_anchor = Some((row, col));
                            }
                            *cursor_line = row;
                            *cursor_col = col;
                            self.editor_dragging_selection = true;
                        }
                    }
                }
            }
            WindowEvent::MouseDown {
                x,
                y,
                button: MouseBtn::Right,
                ..
            } => {
                if x < RAIL_W + self.left_w && self.left_tab == LeftTab::Explorer {
                    let mut reveal = None;
                    for zone in self.hit_zones.iter().rev() {
                        if zone.rect.contains(x, y) {
                            if let UiAction::OpenFile(abs, _, _, _) = &zone.action {
                                reveal = Some(abs.clone());
                                break;
                            }
                        }
                    }
                    if let Some(p) = reveal {
                        self.execute_action(UiAction::RevealFile(p));
                    }
                }
            }
            WindowEvent::MouseDown { .. } => {}
            WindowEvent::MouseUp { .. } => {
                self.active_splitter = None;
                if self.editor_dragging_selection {
                    self.editor_dragging_selection = false;
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        if let TabContent::Code {
                            cursor_line,
                            cursor_col,
                            sel_anchor,
                            ..
                        } = &mut tab.content
                        {
                            if *sel_anchor == Some((*cursor_line, *cursor_col)) {
                                *sel_anchor = None;
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel {
                x,
                y,
                delta_lines,
                shift,
            } => {
                if self.modal == ModalDialog::None {
                    let steps = (delta_lines as isize) * 3;
                    // The OpenCode-style shell scrolls its own transcript.
                    if self.shell_view == ShellView::Chat {
                        // Wheel up unpins; scrolling back to the bottom re-pins.
                        let rows = shell::chat::build_rows(
                            &self.shell_chat_model(),
                            ((self.width / 8).max(20)) as usize,
                        )
                        .len();
                        let vis = ((self.height - shell::TABSTRIP_H - shell::STATUSBAR_H - 58)
                            / 20)
                            .max(3) as usize;
                        let max_off = rows.saturating_sub(vis);
                        let next = (self.chat_scroll as isize + steps).clamp(0, max_off as isize)
                            as usize;
                        self.chat_scroll = next;
                        self.chat_pinned = next >= max_off;
                        return self.compute_cursor();
                    }
                    if x < RAIL_W {
                        return self.compute_cursor();
                    }
                    if self.left_panel_open && x < RAIL_W + self.left_w {
                        match self.left_tab {
                            LeftTab::Explorer => {
                                self.explorer_scroll =
                                    (self.explorer_scroll as isize - steps).max(0) as usize;
                            }
                            LeftTab::Search => {
                                self.search_scroll =
                                    (self.search_scroll as isize - steps).max(0) as usize;
                            }
                            LeftTab::Git => {
                                self.git_scroll = (self.git_scroll as isize - steps).max(0) as usize;
                            }
                            LeftTab::Sessions => {
                                self.sessions_scroll =
                                    (self.sessions_scroll as isize - steps).max(0) as usize;
                            }
                            LeftTab::Extensions => {
                                self.extensions_scroll =
                                    (self.extensions_scroll as isize - steps).max(0) as usize;
                            }
                        }
                    } else if x >= self.width - self.right_w {
                        match self.agent_panel_tab {
                            AgentPanelTab::Chat => {
                                self.agent_scroll_lines =
                                    (self.agent_scroll_lines as isize + steps).max(0) as usize;
                            }
                            AgentPanelTab::Timeline => {
                                self.timeline_scroll =
                                    (self.timeline_scroll as isize - steps).max(0) as usize;
                            }
                        }
                    } else if self.terminal_open && self.term_grid_rect.contains(x, y) {
                        if let Some(term) = self.terminals.get_mut(self.active_term) {
                            let max_sb = term.screen.scrollback.len();
                            term.screen.scroll_offset = (term.screen.scroll_offset as isize + steps)
                                .clamp(0, max_sb as isize) as usize;
                        }
                    } else if self.center_view == CenterView::Browser
                        && self.browser_rect.contains(x, y)
                    {
                        self.browser_scroll = (self.browser_scroll - (steps * 12) as i32).max(0);
                    } else if self.editor_rect.contains(x, y) {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            match &mut tab.content {
                                TabContent::Code {
                                    lines,
                                    scroll_y,
                                    scroll_x,
                                    ..
                                } => {
                                    if shift {
                                        *scroll_x = (*scroll_x as isize - steps * 2).max(0) as usize;
                                    } else {
                                        let max_y = lines.len().saturating_sub(1);
                                        *scroll_y = (*scroll_y as isize - steps)
                                            .clamp(0, max_y as isize)
                                            as usize;
                                    }
                                }
                                TabContent::Diff { rows, scroll_y } => {
                                    let max_y = rows.len().saturating_sub(1);
                                    *scroll_y = (*scroll_y as isize - steps)
                                        .clamp(0, max_y as isize)
                                        as usize;
                                }
                            }
                        }
                    }
                }
            }
            WindowEvent::KeyDown { key, mods } => self.handle_key_down(key, mods),
            WindowEvent::CharInput(ch) => self.handle_char_input(ch),
        }

        self.compute_cursor()
    }

    fn render(&mut self, canvas: &mut dyn SurfaceCanvas) {
        self.width = canvas.width().max(760);
        self.height = canvas.height().max(520);
        self.hit_zones.clear();
        self.shell_flash.tick();

        // The OpenCode-style shell owns Chat / Connect; Workspace keeps the
        // classic IDE panels.
        if self.shell_view == ShellView::Workspace {
            self.render_workspace(canvas);
            return;
        }
        self.render_shell(canvas);
    }

    /// Classic IDE layout (explorer + editor + terminal + git + browser).
    fn render_workspace(&mut self, canvas: &mut dyn SurfaceCanvas) {
        canvas.clear(self.palette.bg_app);

        let modal_open = self.modal != ModalDialog::None
            || self.pending_approval.is_some()
            || self.pending_delete.is_some();

        let top = SHELL_CHROME_TOP;
        let bottom = self.height - SHELL_CHROME_BOTTOM;
        let panel_x = if self.left_panel_open {
            RAIL_W + self.left_w
        } else {
            RAIL_W
        };
        let center_x = panel_x;
        let center_w = (self.width - panel_x - self.right_w).max(220);
        let term_h = if self.terminal_open {
            self.term_h.min((bottom - top - 140).max(90))
        } else {
            0
        };
        let center_h = (bottom - top - term_h).max(120);

        self.draw_shell_chrome(canvas);
        self.draw_left_rail(canvas, top, bottom);
        if self.left_panel_open {
            self.draw_left_panel(canvas, top, bottom);
        }
        self.draw_center(canvas, center_x, top, center_w, center_h);
        if self.terminal_open {
            self.draw_bottom_terminal(canvas, center_x, top + center_h, center_w, term_h);
        }
        self.draw_right_agent_panel(canvas, top, bottom);
        self.draw_status_footer(canvas);

        if modal_open {
            self.hit_zones.clear();
            if let Some(req) = self.pending_approval.clone() {
                self.draw_approval_modal(canvas, &req);
            } else if let Some(del) = self.pending_delete.clone() {
                self.draw_delete_modal(canvas, &del);
            } else {
                match self.modal {
                    ModalDialog::CommandPalette => self.draw_command_palette(canvas),
                    ModalDialog::ConnectWizard => self.draw_connect_wizard(canvas),
                    ModalDialog::ThemePicker => self.draw_theme_picker(canvas),
                    ModalDialog::SafetySettings => self.draw_safety_modal(canvas),
                    ModalDialog::None => {}
                }
            }
        }
    }

}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------
fn clean_win_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        p.to_path_buf()
    }
}

fn lighten(rgb: u32, delta: u32) -> u32 {
    let r = (((rgb >> 16) & 0xFF) + delta).min(255);
    let g = (((rgb >> 8) & 0xFF) + delta).min(255);
    let b = ((rgb & 0xFF) + delta).min(255);
    (r << 16) | (g << 8) | b
}

/// Blends two RGB colors (t = 0 -> a, t = 1 -> b).
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ar = ((a >> 16) & 0xFF) as f32;
    let ag = ((a >> 8) & 0xFF) as f32;
    let ab = (a & 0xFF) as f32;
    let br = ((b >> 16) & 0xFF) as f32;
    let bg = ((b >> 8) & 0xFF) as f32;
    let bb = (b & 0xFF) as f32;
    let r = (ar + (br - ar) * t).round() as u32;
    let g = (ag + (bg - ag) * t).round() as u32;
    let bl = (ab + (bb - ab) * t).round() as u32;
    (r << 16) | (g << 8) | bl
}

fn truncate_right(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let head: String = chars[..max_chars.saturating_sub(2)].iter().collect();
        format!("{}..", head)
    }
}

/// Shortens a status string for compact badges.
pub fn truncate_for_status(s: &str) -> String {
    truncate_right(s, 44)
}

fn wrap_text(text: &str, max_cols: usize) -> Vec<String> {
    let width = max_cols.max(16);
    let mut out = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            if current.is_empty() {
                if word.chars().count() > width {
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        chunk.push(ch);
                        if chunk.chars().count() >= width {
                            out.push(chunk.clone());
                            chunk.clear();
                        }
                    }
                    current = chunk;
                } else {
                    current.push_str(word);
                }
            } else if current.chars().count() + 1 + word.chars().count() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn fam_color(fam: crate::syntax::LangFamily, p: &ThemePalette) -> u32 {
    match fam {
        crate::syntax::LangFamily::Rust => 0xF97316,
        crate::syntax::LangFamily::TypeScript => p.accent,
        crate::syntax::LangFamily::Python => 0xFACC15,
        crate::syntax::LangFamily::CFamily => 0x60A5FA,
        crate::syntax::LangFamily::Go => 0x22D3EE,
        crate::syntax::LangFamily::Json | crate::syntax::LangFamily::TomlYaml => p.purple,
        crate::syntax::LangFamily::Markdown => p.success,
        _ => p.text_muted,
    }
}

/// Compact unicode-ish glyph set used across the native chrome (ASCII-safe fallbacks).
fn glyph(name: &str) -> &'static str {
    match name {
        "folder" => "[d]",
        "search" => "[?]",
        "plug" => "[p]",
        "chip" => "[m]",
        "bolt" => "[!]",
        "brush" => "[t]",
        "chev_r" => ">",
        "chev_d" => "v",
        "branch" => "~",
        "term" => ">_",
        "globe" => "@",
        "arrow_l" => "<",
        "arrow_r" => ">",
        "reload" => "o",
        "user" => ">",
        "spark" => "*",
        "brain" => "#",
        "wrench" => "+",
        "robot" => "&",
        "check" => "v",
        "lock" => "#",
        _ => "-",
    }
}

// ---------------------------------------------------------------------------
// Native platform handler bridge (Win32 / X11 message pump)
// ---------------------------------------------------------------------------
impl DesktopAppHandler for StudioApp {
    fn on_event(&mut self, event: WindowEvent) -> CursorShape {
        StudioApp::on_event(self, event)
    }

    fn render(&mut self, canvas: &mut dyn SurfaceCanvas) {
        StudioApp::render(self, canvas);
    }

    fn set_code_font_size(&self) -> i32 {
        self.settings.editor_font_size
    }
}
