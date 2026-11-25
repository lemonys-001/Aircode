//! Pure-Rust Native Desktop IDE Workbench (`StudioApp`).
//! Renders directly onto the OS 32-bit pixel surface (Win32 DIBSection + ClearType GDI on Windows,
//! X11 SoftwareSurface on Linux) — zero WebView, zero HTML/CSS/JS.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crate::agent::{
    probe_ollama_models, spawn_autonomous_agent_run, AgentUiEvent, PermissionGate, PermissionPrompt,
};
use crate::editor::{char_len, slice_chars, DiffLineKind, EditorTab, TabContent};
use crate::fs_ops::{
    resolve_workspace_path, scan_project_tree, search_project_files, DriveInfo, FileEntry,
    SearchMatch,
};
use crate::git_ops::{
    inspect_git_status, load_head_and_working_diff, run_git, GitStatusSnapshot,
};
use crate::platform::{
    self, CursorShape, DesktopAppHandler, DirectoryWatcher, FontRole, KeyCode, Modifiers, MouseBtn,
    NativePtySession, PtyEvent, SurfaceCanvas, WindowEvent,
};
use crate::sqlite::{
    AppSettings, SessionDetail, SessionSummary, SqliteStore, ToolExecutionRecord,
};
use crate::syntax::{tokenize_line, LangFamily};
use crate::ui::{FocusTarget, HitZone, LeftTab, SettingsTab, Theme, UiAction, UiRect};
use crate::vt_screen::VtScreen;

struct TerminalSession {
    title: String,
    shell_kind: String,
    pty: NativePtySession,
    rx: Receiver<PtyEvent>,
    screen: VtScreen,
}

#[derive(Clone, Debug)]
enum DragSplitter {
    LeftSidebar,
    RightAgent,
    BottomTerminal,
}

#[derive(Clone, Debug)]
struct DeleteConfirmState {
    abs_path: PathBuf,
    rel_path: String,
}

pub struct StudioApp {
    width: i32,
    height: i32,
    mouse_x: i32,
    mouse_y: i32,

    // Resizable Dock Dimensions
    left_w: i32,
    right_w: i32,
    term_h: i32,
    active_splitter: Option<DragSplitter>,
    editor_dragging_selection: bool,

    // SQLite store + settings
    db: Arc<SqliteStore>,
    settings: AppSettings,
    settings_draft: AppSettings,
    ollama_models: Vec<String>,
    tool_audit_log: Vec<ToolExecutionRecord>,

    // Workspace & Explorer (note: collapsed_dirs holds collapsed relative paths)
    workspace: PathBuf,
    drives: Vec<DriveInfo>,
    entries: Vec<FileEntry>,
    collapsed_dirs: HashSet<String>,
    explorer_scroll: usize,
    left_tab: LeftTab,
    watcher: Option<DirectoryWatcher>,
    fs_changed_flag: Arc<AtomicBool>,

    // Inline create file/folder bar
    creating_entry: Option<bool>, // Some(true)=dir, Some(false)=file
    new_entry_name: String,

    // Search panel
    search_query: String,
    search_results: Vec<SearchMatch>,
    search_scroll: usize,

    // Git panel
    git_snapshot: GitStatusSnapshot,
    commit_message: String,
    git_scroll: usize,

    // Code Editor & Side-by-Side Diff Viewer tabs
    tabs: Vec<EditorTab>,
    active_tab: usize,
    show_find_bar: bool,
    find_query: String,
    editor_rect: UiRect,

    // ConPTY Multi-Tab Terminal
    terminals: Vec<TerminalSession>,
    active_term: usize,
    term_grid_rect: UiRect,

    // Autonomous AI Agent & SQLite Sessions
    sessions: Vec<SessionSummary>,
    active_session: SessionDetail,
    live_tools: Vec<ToolExecutionRecord>,
    streaming_thinking: String,
    streaming_text: String,
    agent_prompt: String,
    agent_running: bool,
    agent_status_line: String,
    agent_rx: Option<Receiver<AgentUiEvent>>,
    permission_gate: Arc<PermissionGate>,
    agent_scroll_lines: usize,

    // Modals & Toast Notifications
    settings_modal: Option<SettingsTab>,
    pending_approval: Option<PermissionPrompt>,
    pending_delete: Option<DeleteConfirmState>,
    toast_text: String,
    toast_ticks: u32,

    // Focus & Hit-testing
    focus: FocusTarget,
    blink_phase: bool,
    tick_counter: u64,
    hit_zones: Vec<HitZone>,
}

impl StudioApp {
    pub fn new(initial_workspace: Option<PathBuf>) -> Self {
        Self::new_with_options(initial_workspace, true)
    }

    pub fn new_with_options(initial_workspace: Option<PathBuf>, spawn_default_shell: bool) -> Self {
        let data_dir = platform::app_data_dir();
        let db = Arc::new(
            SqliteStore::open(&data_dir).unwrap_or_else(|_| {
                let fallback = std::env::temp_dir().join("ai_coding_agent_fallback_db");
                SqliteStore::open(&fallback).expect("fallback sqlite open")
            }),
        );
        let settings = db.load_settings();
        let recent = db.list_recent_projects();

        let ws = initial_workspace
            .or_else(|| {
                recent
                    .first()
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let ws_clean = clean_win_verbatim(&std::fs::canonicalize(&ws).unwrap_or(ws));
        let ws_str = ws_clean.to_string_lossy().to_string();
        db.record_recent_project(&ws_str);

        let active_session = db.ensure_default_session(&ws_str);
        let sessions = db.list_sessions();
        let drives = platform::list_drives();
        let fs_changed_flag = Arc::new(AtomicBool::new(false));

        let mut app = Self {
            width: 1480,
            height: 920,
            mouse_x: 0,
            mouse_y: 0,
            left_w: 276,
            right_w: 404,
            term_h: 236,
            active_splitter: None,
            editor_dragging_selection: false,
            settings_draft: settings.clone(),
            db,
            settings,
            ollama_models: Vec::new(),
            tool_audit_log: Vec::new(),
            workspace: ws_clean,
            drives,
            entries: Vec::new(),
            collapsed_dirs: HashSet::new(),
            explorer_scroll: 0,
            left_tab: LeftTab::Explorer,
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
            tabs: Vec::new(),
            active_tab: 0,
            show_find_bar: false,
            find_query: String::new(),
            editor_rect: UiRect::new(276, 80, 800, 580),
            terminals: Vec::new(),
            active_term: 0,
            term_grid_rect: UiRect::new(276, 690, 800, 200),
            sessions,
            active_session,
            live_tools: Vec::new(),
            streaming_thinking: String::new(),
            streaming_text: String::new(),
            agent_prompt: String::new(),
            agent_running: false,
            agent_status_line: "Ready — 100% Pure-Rust Native Studio".to_string(),
            agent_rx: None,
            permission_gate: Arc::new(PermissionGate::new()),
            agent_scroll_lines: 0,
            settings_modal: None,
            pending_approval: None,
            pending_delete: None,
            toast_text: "AI Coding Agent (Pure-Rust Native GUI) initialized.".to_string(),
            toast_ticks: 90,
            focus: FocusTarget::AgentPrompt,
            blink_phase: true,
            tick_counter: 0,
            hit_zones: Vec::with_capacity(256),
        };

        app.reload_workspace(true);
        if spawn_default_shell {
            let shell = if app.settings.default_shell == "cmd" {
                "cmd"
            } else {
                "powershell"
            };
            app.spawn_terminal(shell);
        }
        app
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast_text = msg.into();
        self.toast_ticks = 95;
    }

    fn reload_workspace(&mut self, reset_watcher: bool) {
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
        self.sync_active_pty_cwd();
        self.show_toast(format!("Workspace: {}", self.workspace.display()));
    }

    fn open_file_in_tab(&mut self, abs_path: &Path, rel_path: &str) -> Result<(), String> {
        for (idx, tab) in self.tabs.iter().enumerate() {
            if tab.full_path.as_deref() == Some(abs_path) {
                self.active_tab = idx;
                self.focus = FocusTarget::Editor;
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
        Ok(())
    }

    fn open_git_diff_tab(&mut self, rel_path: &str) {
        let (orig, modified) = load_head_and_working_diff(&self.workspace, rel_path);
        for (idx, t) in self.tabs.iter_mut().enumerate() {
            if matches!(t.content, TabContent::Diff { .. }) && t.rel_path == rel_path {
                *t = EditorTab::new_diff(rel_path.to_string(), &orig, &modified);
                self.active_tab = idx;
                return;
            }
        }
        self.tabs
            .push(EditorTab::new_diff(rel_path.to_string(), &orig, &modified));
        self.active_tab = self.tabs.len() - 1;
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
                    "\x1b[36m[ConPTY Connected: {} in {}]\x1b[0m\r\n",
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

    fn launch_agent_prompt(&mut self, prompt_override: Option<String>) {
        if self.agent_running {
            self.show_toast("Agent is currently running — click STOP to interrupt first.");
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

        self.db.save_settings(&self.settings);
        self.live_tools.clear();
        self.streaming_thinking.clear();
        self.streaming_text.clear();
        self.agent_scroll_lines = 0;

        let (tx, rx) = mpsc::channel();
        let sid = self.active_session.id.clone();
        let mode = self.settings.default_mode.clone();

        spawn_autonomous_agent_run(
            self.db.clone(),
            self.permission_gate.clone(),
            self.workspace.clone(),
            sid.clone(),
            prompt,
            mode,
            tx,
        );

        // Reload active session immediately so user prompt appears right away
        if let Some(updated) = self.db.load_session(&sid) {
            self.active_session = updated;
        }
        self.agent_rx = Some(rx);
        self.agent_running = true;
        self.agent_status_line = "Starting autonomous agent loop...".to_string();
    }

    fn drain_background_streams(&mut self) {
        // 1. Drain ConPTY terminal streams
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

        // 2. Drain FS watcher
        if self.fs_changed_flag.swap(false, Ordering::SeqCst) {
            self.entries = scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
            self.git_snapshot = inspect_git_status(&self.workspace);
            self.refresh_open_code_tabs_from_disk();
        }

        // 3. Drain Autonomous AI Agent events
        let mut finished_session: Option<Option<SessionDetail>> = None;
        let mut workspace_mutated = false;

        if let Some(rx) = &self.agent_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    AgentUiEvent::Status(s) => {
                        self.agent_status_line = s;
                    }
                    AgentUiEvent::ThinkingDelta(d) => {
                        self.streaming_thinking.push_str(&d);
                    }
                    AgentUiEvent::TextDelta(d) => {
                        if !self.streaming_text.is_empty() {
                            self.streaming_text.push('\n');
                        }
                        self.streaming_text.push_str(&d);
                    }
                    AgentUiEvent::ToolStarted(rec) => {
                        self.live_tools.push(rec);
                    }
                    AgentUiEvent::ToolFinished(rec) => {
                        if !rec.file_path.is_empty()
                            || matches!(
                                rec.tool_name.as_str(),
                                "write_file" | "edit_file" | "execute_command" | "git_operation"
                            )
                        {
                            workspace_mutated = true;
                        }
                        if let Some(existing) =
                            self.live_tools.iter_mut().find(|r| r.id == rec.id)
                        {
                            *existing = rec;
                        } else {
                            self.live_tools.push(rec);
                        }
                    }
                    AgentUiEvent::PermissionRequested(prompt) => {
                        self.pending_approval = Some(prompt);
                    }
                    AgentUiEvent::WorkspaceMutated => {
                        workspace_mutated = true;
                    }
                    AgentUiEvent::Error(err) => {
                        self.agent_status_line = format!("Error: {}", err);
                    }
                    AgentUiEvent::Done(opt_detail) => {
                        finished_session = Some(opt_detail);
                    }
                }
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
                self.agent_status_line = "Completed autonomous turn.".to_string();
            }
        }
    }

    fn execute_action(&mut self, action: UiAction) {
        match action {
            UiAction::Focus(target) => {
                self.focus = target;
            }
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
            UiAction::SelectLeftTab(t) => {
                self.left_tab = t;
                match t {
                    LeftTab::Search => self.focus = FocusTarget::SearchInput,
                    LeftTab::Git => {
                        self.git_snapshot = inspect_git_status(&self.workspace);
                    }
                    LeftTab::Explorer => {}
                }
            }
            UiAction::RefreshWorkspace => {
                self.reload_workspace(false);
                self.refresh_open_code_tabs_from_disk();
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
                self.entries =
                    scan_project_tree(&self.workspace, &self.collapsed_dirs, false, 950);
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
            UiAction::RevealFile(abs) => {
                platform::reveal_in_file_manager(&abs);
            }
            UiAction::RunSearch => {
                let q = self.search_query.trim().to_string();
                if !q.is_empty() {
                    self.search_results = search_project_files(&self.workspace, &q, false, 250);
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
                        self.show_toast(out.lines().next().unwrap_or("Committed."));
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
            UiAction::GitOpenDiff(rel) => {
                self.open_git_diff_tab(&rel);
            }
            UiAction::SelectEditorTab(idx) => {
                if idx < self.tabs.len() {
                    self.active_tab = idx;
                    self.focus = FocusTarget::Editor;
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
                    "// Pure-Rust Scratch Buffer — press Ctrl+S to save into workspace\nfn main() {\n    println!(\"Hello from AI Coding Agent!\");\n}\n",
                    false,
                );
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
                self.focus = FocusTarget::Editor;
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
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    let vis_rows = ((self.editor_rect.h - 12).max(40) / 20) as usize;
                    if !tab.find_next(&self.find_query, vis_rows.max(5)) {
                        self.show_toast(format!("No match for '{}'", self.find_query));
                    }
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
            UiAction::CycleProvider => {
                let providers = ["ollama", "openai", "anthropic", "gemini", "openrouter", "custom"];
                let cur_idx = providers
                    .iter()
                    .position(|p| *p == self.settings.active_provider)
                    .unwrap_or(0);
                let next_p = providers[(cur_idx + 1) % providers.len()];
                self.settings.active_provider = next_p.to_string();
                if let Some(first_m) = AppSettings::preset_models_for_provider(next_p).first() {
                    self.settings.active_model = (*first_m).to_string();
                }
                self.db.save_settings(&self.settings);
                self.show_toast(format!(
                    "Provider -> {} ({})",
                    self.settings.active_provider, self.settings.active_model
                ));
            }
            UiAction::CycleModel => {
                let presets: Vec<String> = if self.settings.active_provider == "ollama"
                    && !self.ollama_models.is_empty()
                {
                    self.ollama_models.clone()
                } else {
                    AppSettings::preset_models_for_provider(&self.settings.active_provider)
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                };
                if !presets.is_empty() {
                    let pos = presets
                        .iter()
                        .position(|m| m == &self.settings.active_model)
                        .unwrap_or(0);
                    self.settings.active_model = presets[(pos + 1) % presets.len()].clone();
                    self.db.save_settings(&self.settings);
                }
            }
            UiAction::SetMode(mode) => {
                self.settings.default_mode = mode.to_string();
                self.db.save_settings(&self.settings);
                self.show_toast(format!("Agent mode set to {}", mode.to_uppercase()));
            }
            UiAction::SendAgentPrompt => {
                self.launch_agent_prompt(None);
            }
            UiAction::QuickPrompt(text) => {
                self.launch_agent_prompt(Some(text.to_string()));
            }
            UiAction::EmergencyStop => {
                if self.agent_running {
                    self.permission_gate.trigger_emergency_stop();
                    self.pending_approval = None;
                    self.agent_status_line = "Emergency Stop triggered by user!".to_string();
                    self.show_toast("Emergency Stop triggered — halting agent loop immediately.");
                } else {
                    self.show_toast("No active agent run to stop.");
                }
            }
            UiAction::NewSession => {
                self.active_session = self.db.create_session(
                    "New Session",
                    &self.workspace.to_string_lossy(),
                    &self.settings.default_mode,
                );
                self.sessions = self.db.list_sessions();
                self.live_tools.clear();
                self.agent_scroll_lines = 0;
                self.focus = FocusTarget::AgentPrompt;
            }
            UiAction::SelectSession(id) => {
                if let Some(detail) = self.db.load_session(&id) {
                    self.active_session = detail;
                    self.live_tools.clear();
                    self.agent_scroll_lines = 0;
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
            UiAction::OpenSettings(tab) => {
                self.settings_draft = self.settings.clone();
                self.tool_audit_log = self.db.list_tool_history(60);
                self.settings_modal = Some(tab);
                if tab == SettingsTab::Providers {
                    self.focus = FocusTarget::SettingsModel;
                }
            }
            UiAction::CloseModal => {
                self.settings_modal = None;
            }
            UiAction::SwitchSettingsTab(tab) => {
                if tab == SettingsTab::ToolAudit {
                    self.tool_audit_log = self.db.list_tool_history(60);
                }
                self.settings_modal = Some(tab);
            }
            UiAction::SelectSettingsProvider(p) => {
                self.settings_draft.active_provider = p.to_string();
                if let Some(first) = AppSettings::preset_models_for_provider(p).first() {
                    self.settings_draft.active_model = (*first).to_string();
                }
            }
            UiAction::SelectPresetModel(m) => {
                self.settings_draft.active_model = m;
            }
            UiAction::ProbeOllama => {
                let (reachable, models) = probe_ollama_models(&self.settings_draft.ollama_base_url);
                if reachable {
                    let count = models.len();
                    if let Some(first) = models.first() {
                        self.settings_draft.active_model = first.clone();
                    }
                    self.ollama_models = models;
                    self.show_toast(format!("Ollama online — {} local model(s) found.", count));
                } else {
                    self.show_toast("Ollama unreachable on configured base URL.");
                }
            }
            UiAction::ToggleSettingBool(field) => match field {
                "require_shell_approval" => {
                    self.settings_draft.require_shell_approval =
                        !self.settings_draft.require_shell_approval;
                }
                "require_delete_confirmation" => {
                    self.settings_draft.require_delete_confirmation =
                        !self.settings_draft.require_delete_confirmation;
                }
                "restrict_to_workspace" => {
                    self.settings_draft.restrict_to_workspace =
                        !self.settings_draft.restrict_to_workspace;
                }
                _ => {}
            },
            UiAction::AdjustMaxIters(delta) => {
                let next =
                    (self.settings_draft.max_agent_iterations as isize + delta).clamp(2, 30);
                self.settings_draft.max_agent_iterations = next as usize;
            }
            UiAction::AdjustFontSize(delta) => {
                self.settings_draft.editor_font_size =
                    (self.settings_draft.editor_font_size + delta).clamp(12, 22);
            }
            UiAction::ToggleDefaultShell => {
                self.settings_draft.default_shell = if self.settings_draft.default_shell == "cmd" {
                    "powershell".to_string()
                } else {
                    "cmd".to_string()
                };
            }
            UiAction::SaveSettingsModal => {
                self.settings = self.settings_draft.clone();
                self.db.save_settings(&self.settings);
                self.settings_modal = None;
                self.show_toast("Settings persisted to SQLite database.");
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
                self.tabs
                    .retain(|t| t.full_path.as_deref() != Some(abs));
                if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                    self.active_tab = self.tabs.len() - 1;
                }
                self.reload_workspace(false);
                self.show_toast(format!("Deleted {}", rel));
            }
            Err(e) => self.show_toast(format!("Delete failed: {}", e)),
        }
    }

    fn active_single_line_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            FocusTarget::SearchInput => Some(&mut self.search_query),
            FocusTarget::NewEntryInput => Some(&mut self.new_entry_name),
            FocusTarget::CommitInput => Some(&mut self.commit_message),
            FocusTarget::FindInput => Some(&mut self.find_query),
            FocusTarget::SettingsModel => Some(&mut self.settings_draft.active_model),
            FocusTarget::SettingsOpenAiKey => Some(&mut self.settings_draft.openai_api_key),
            FocusTarget::SettingsAnthropicKey => Some(&mut self.settings_draft.anthropic_api_key),
            FocusTarget::SettingsGeminiKey => Some(&mut self.settings_draft.gemini_api_key),
            FocusTarget::SettingsOpenRouterKey => Some(&mut self.settings_draft.openrouter_api_key),
            FocusTarget::SettingsOllamaUrl => Some(&mut self.settings_draft.ollama_base_url),
            FocusTarget::SettingsCustomUrl => Some(&mut self.settings_draft.custom_base_url),
            FocusTarget::SettingsCustomKey => Some(&mut self.settings_draft.custom_api_key),
            _ => None,
        }
    }

    fn handle_key_down(&mut self, key: KeyCode, mods: Modifiers) {
        if mods.ctrl {
            match key {
                KeyCode::Letter('S') => {
                    if self.settings_modal.is_some() {
                        self.execute_action(UiAction::SaveSettingsModal);
                    } else {
                        self.save_active_tab();
                    }
                    return;
                }
                KeyCode::Letter('O') => {
                    self.execute_action(UiAction::PickFolder);
                    return;
                }
                KeyCode::Letter('F') => {
                    if mods.shift {
                        self.execute_action(UiAction::SelectLeftTab(LeftTab::Search));
                    } else {
                        self.execute_action(UiAction::ToggleFindBar);
                    }
                    return;
                }
                KeyCode::Letter('N') => {
                    self.execute_action(UiAction::NewScratchTab);
                    return;
                }
                _ => {}
            }
        }

        if key == KeyCode::F5 {
            self.execute_action(UiAction::RefreshWorkspace);
            return;
        }
        if key == KeyCode::F3 {
            self.execute_action(UiAction::FindNext);
            return;
        }

        if key == KeyCode::Escape {
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
            if self.settings_modal.is_some() {
                self.settings_modal = None;
                return;
            }
            if self.creating_entry.is_some() {
                self.creating_entry = None;
                return;
            }
            if self.show_find_bar {
                self.show_find_bar = false;
                self.focus = FocusTarget::Editor;
                return;
            }
        }

        // Terminal focus keyboard forwarding
        if self.focus == FocusTarget::Terminal && self.settings_modal.is_none() {
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

        // Code editor focus
        if self.focus == FocusTarget::Editor && self.settings_modal.is_none() {
            let vis_rows = ((self.editor_rect.h - 12).max(40) / 20) as usize;
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
                        _ => {}
                    }
                }

                match key {
                    KeyCode::Left => tab.move_cursor(0, -1, mods.shift, vis_rows),
                    KeyCode::Right => tab.move_cursor(0, 1, mods.shift, vis_rows),
                    KeyCode::Up => tab.move_cursor(-1, 0, mods.shift, vis_rows),
                    KeyCode::Down => tab.move_cursor(1, 0, mods.shift, vis_rows),
                    KeyCode::Home => tab.jump_home_end(false, mods.shift),
                    KeyCode::End => tab.jump_home_end(true, mods.shift),
                    KeyCode::PageUp => {
                        tab.move_cursor(-(vis_rows as isize), 0, mods.shift, vis_rows)
                    }
                    KeyCode::PageDown => {
                        tab.move_cursor(vis_rows as isize, 0, mods.shift, vis_rows)
                    }
                    KeyCode::Backspace => tab.backspace(),
                    KeyCode::Delete => tab.delete_forward(),
                    KeyCode::Enter => tab.insert_newline_auto_indent(),
                    KeyCode::Tab => tab.insert_str("    "),
                    _ => {}
                }
            }
            return;
        }

        // Agent prompt composer
        if self.focus == FocusTarget::AgentPrompt && self.settings_modal.is_none() {
            if mods.ctrl && key == KeyCode::Letter('V') {
                if let Some(clip) = platform::clipboard_get_text() {
                    self.agent_prompt.push_str(&clip);
                }
                return;
            }
            match key {
                KeyCode::Enter => {
                    if mods.shift {
                        self.agent_prompt.push('\n');
                    } else {
                        self.launch_agent_prompt(None);
                    }
                }
                KeyCode::Backspace => {
                    self.agent_prompt.pop();
                }
                _ => {}
            }
            return;
        }

        // Single-line text inputs
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
                    FocusTarget::NewEntryInput => self.execute_action(UiAction::ConfirmCreateEntry),
                    FocusTarget::CommitInput => self.execute_action(UiAction::GitCommit),
                    FocusTarget::FindInput => self.execute_action(UiAction::FindNext),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn handle_char_input(&mut self, ch: char) {
        if ch < ' ' {
            return;
        }
        if self.focus == FocusTarget::Terminal && self.settings_modal.is_none() {
            if let Some(term) = self.terminals.get_mut(self.active_term) {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                term.screen.scroll_offset = 0;
                term.pty.write(s.as_bytes());
            }
            return;
        }

        if self.focus == FocusTarget::Editor && self.settings_modal.is_none() {
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                tab.insert_str(s);
            }
            return;
        }

        if self.focus == FocusTarget::AgentPrompt && self.settings_modal.is_none() {
            self.agent_prompt.push(ch);
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
        let gutter_w = 56;
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
        if self.settings_modal.is_some()
            || self.pending_approval.is_some()
            || self.pending_delete.is_some()
        {
            return CursorShape::Arrow;
        }
        let top = 44;
        let bottom = self.height - 24;
        if self.mouse_y >= top && self.mouse_y < bottom {
            if (self.mouse_x - self.left_w).abs() <= 4
                || (self.mouse_x - (self.width - self.right_w)).abs() <= 4
            {
                return CursorShape::ResizeH;
            }
            let term_top = bottom - self.term_h;
            if self.mouse_x > self.left_w
                && self.mouse_x < self.width - self.right_w
                && (self.mouse_y - term_top).abs() <= 4
            {
                return CursorShape::ResizeV;
            }
        }
        if self.editor_rect.contains(self.mouse_x, self.mouse_y) && !self.tabs.is_empty() {
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

    // =========================================================================
    // RENDERING PIPELINE (Pure-Rust Double-Buffered Pixel + ClearType Renderer)
    // =========================================================================

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
        let hovered = rect.contains(self.mouse_x, self.mouse_y);
        let fill = if hovered && action.is_some() {
            lighten(bg, 18)
        } else {
            bg
        };
        canvas.fill_rounded_rect(rect.x, rect.y, rect.w, rect.h, 5, fill);
        canvas.stroke_rounded_rect(rect.x, rect.y, rect.w, rect.h, 5, border);
        let tw = canvas.measure_text_width(label, FontRole::UiSemibold);
        let tx = rect.x + ((rect.w - tw) / 2).max(4);
        let ty = rect.y + ((rect.h - 15) / 2).max(2);
        canvas.draw_text_clipped(tx, ty, rect.w - 6, label, FontRole::UiSemibold, fg);
        if let Some(act) = action {
            self.push_hit(rect, act);
        }
    }

    fn draw_top_header(&mut self, canvas: &mut dyn SurfaceCanvas, w: i32) {
        let h = 44;
        canvas.fill_rect(0, 0, w, h, Theme::BG_TITLEBAR);
        canvas.draw_hline(0, h - 1, w, Theme::BORDER);

        // Brand emblem
        canvas.fill_rounded_rect(10, 8, 28, 28, 6, Theme::ACCENT_BLUE);
        canvas.draw_text(14, 13, "RS", FontRole::UiSemibold, 0xFFFFFF);
        canvas.draw_text(44, 7, "AI CODING AGENT", FontRole::UiTitle, Theme::TEXT_PRIMARY);
        canvas.draw_text(
            44,
            24,
            "Pure-Rust Native Studio",
            FontRole::UiSmall,
            Theme::ACCENT_CYAN,
        );

        let mut x = 216;
        self.draw_button(
            canvas,
            UiRect::new(x, 8, 98, 28),
            "Open Folder",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::PickFolder),
        );
        x += 106;

        let ws_str = truncate_left(&self.workspace.to_string_lossy(), 34);
        let ws_w = 240.min((w - 860).max(110));
        canvas.fill_rounded_rect(x, 8, ws_w, 28, 5, Theme::BG_CARD);
        canvas.stroke_rounded_rect(x, 8, ws_w, 28, 5, Theme::BORDER);
        canvas.draw_text_clipped(
            x + 8,
            14,
            ws_w - 16,
            &ws_str,
            FontRole::UiSmall,
            Theme::TEXT_SECONDARY,
        );
        self.push_hit(UiRect::new(x, 8, ws_w, 28), UiAction::PickFolder);
        x += ws_w + 8;

        let prov_label = format!("API: {}", self.settings.active_provider.to_uppercase());
        self.draw_button(
            canvas,
            UiRect::new(x, 8, 118, 28),
            &prov_label,
            Theme::BG_CARD_ALT,
            Theme::ACCENT_CYAN,
            Theme::BORDER,
            Some(UiAction::CycleProvider),
        );
        x += 124;

        let model_label = format!("Model: {}", truncate_right(&self.settings.active_model, 19));
        self.draw_button(
            canvas,
            UiRect::new(x, 8, 194, 28),
            &model_label,
            Theme::BG_CARD_ALT,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::CycleModel),
        );

        let mut rx = w - 10;
        rx -= 80;
        self.draw_button(
            canvas,
            UiRect::new(rx, 8, 76, 28),
            "Settings",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::OpenSettings(SettingsTab::Providers)),
        );

        rx -= 70;
        self.draw_button(
            canvas,
            UiRect::new(rx, 8, 64, 28),
            "SQLite",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::OpenSettings(SettingsTab::ToolAudit)),
        );

        rx -= 78;
        let stop_bg = if self.agent_running {
            Theme::ACCENT_RED
        } else {
            0x3B1820
        };
        let stop_fg = if self.agent_running {
            0xFFFFFF
        } else {
            0xFCA5A5
        };
        self.draw_button(
            canvas,
            UiRect::new(rx, 8, 72, 28),
            "STOP",
            stop_bg,
            stop_fg,
            Theme::ACCENT_RED,
            Some(UiAction::EmergencyStop),
        );

        let modes: [(&'static str, &str, u32); 3] = [
            ("autonomous", "Auto", Theme::ACCENT_GREEN),
            ("plan", "Plan", Theme::ACCENT_CYAN),
            ("ask", "Ask", Theme::ACCENT_AMBER),
        ];
        for (mode_id, label, color) in modes {
            rx -= 54;
            let active = self.settings.default_mode == mode_id;
            self.draw_button(
                canvas,
                UiRect::new(rx, 8, 50, 28),
                label,
                if active { color } else { Theme::BG_CARD },
                if active { 0x05080F } else { Theme::TEXT_SECONDARY },
                if active { color } else { Theme::BORDER },
                Some(UiAction::SetMode(mode_id)),
            );
        }
    }

    fn draw_left_sidebar(&mut self, canvas: &mut dyn SurfaceCanvas, top: i32, bottom: i32) {
        let w = self.left_w;
        let h = bottom - top;
        canvas.fill_rect(0, top, w, h, Theme::BG_SIDEBAR);
        canvas.draw_vline(w - 1, top, h, Theme::BORDER);

        let tab_w = (w - 16) / 3;
        let git_label = if self.git_snapshot.files.is_empty() {
            "Git".to_string()
        } else {
            format!("Git ({})", self.git_snapshot.files.len())
        };
        let tabs = [
            (LeftTab::Explorer, "Explorer"),
            (LeftTab::Search, "Search"),
            (LeftTab::Git, git_label.as_str()),
        ];
        for (i, (t, label)) in tabs.iter().enumerate() {
            let tx = 6 + i as i32 * (tab_w + 2);
            let active = self.left_tab == *t;
            self.draw_button(
                canvas,
                UiRect::new(tx, top + 6, tab_w, 26),
                label,
                if active {
                    Theme::ACCENT_BLUE
                } else {
                    Theme::BG_CARD
                },
                if active {
                    0xFFFFFF
                } else {
                    Theme::TEXT_SECONDARY
                },
                Theme::BORDER,
                Some(UiAction::SelectLeftTab(*t)),
            );
        }

        let body_top = top + 38;
        match self.left_tab {
            LeftTab::Explorer => self.draw_explorer_panel(canvas, body_top, bottom, w),
            LeftTab::Search => self.draw_search_panel(canvas, body_top, bottom, w),
            LeftTab::Git => self.draw_git_panel(canvas, body_top, bottom, w),
        }
    }

    fn draw_explorer_panel(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        mut y: i32,
        bottom: i32,
        w: i32,
    ) {
        // Logical Drives strip (C:\, D:\, ...)
        let mut dx = 6;
        let drives_clone = self.drives.clone();
        for drv in drives_clone.iter().take(5) {
            let active = self
                .workspace
                .to_string_lossy()
                .to_uppercase()
                .starts_with(&drv.letter.to_uppercase());
            let bw = 46;
            if dx + bw < w - 6 {
                self.draw_button(
                    canvas,
                    UiRect::new(dx, y, bw, 22),
                    &drv.letter,
                    if active {
                        Theme::BG_CARD_ALT
                    } else {
                        Theme::BG_CARD
                    },
                    if active {
                        Theme::ACCENT_CYAN
                    } else {
                        Theme::TEXT_SECONDARY
                    },
                    if active {
                        Theme::ACCENT_CYAN
                    } else {
                        Theme::BORDER
                    },
                    Some(UiAction::SwitchDrive(drv.mount_path.clone())),
                );
                dx += bw + 4;
            }
        }
        y += 28;

        let bw = (w - 20) / 3;
        self.draw_button(
            canvas,
            UiRect::new(6, y, bw, 24),
            "+ File",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::StartCreateEntry { is_dir: false }),
        );
        self.draw_button(
            canvas,
            UiRect::new(10 + bw, y, bw, 24),
            "+ Folder",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::StartCreateEntry { is_dir: true }),
        );
        self.draw_button(
            canvas,
            UiRect::new(14 + bw * 2, y, bw, 24),
            "Refresh",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::RefreshWorkspace),
        );
        y += 30;

        if let Some(is_dir) = self.creating_entry {
            canvas.fill_rounded_rect(6, y, w - 12, 52, 5, Theme::BG_CARD);
            canvas.stroke_rounded_rect(6, y, w - 12, 52, 5, Theme::ACCENT_CYAN);
            let prompt_lbl = if is_dir {
                "New folder relative path:"
            } else {
                "New file relative path:"
            };
            canvas.draw_text(12, y + 4, prompt_lbl, FontRole::UiSmall, Theme::ACCENT_CYAN);
            let inp_rect = UiRect::new(10, y + 22, w - 82, 24);
            canvas.fill_rect(inp_rect.x, inp_rect.y, inp_rect.w, inp_rect.h, Theme::BG_INPUT);
            canvas.stroke_rect(
                inp_rect.x,
                inp_rect.y,
                inp_rect.w,
                inp_rect.h,
                Theme::BORDER_FOCUS,
            );
            let shown = if self.focus == FocusTarget::NewEntryInput && self.blink_phase {
                format!("{}|", self.new_entry_name)
            } else {
                self.new_entry_name.clone()
            };
            canvas.draw_text_clipped(
                inp_rect.x + 6,
                inp_rect.y + 4,
                inp_rect.w - 10,
                &shown,
                FontRole::TerminalMono,
                Theme::TEXT_PRIMARY,
            );
            self.push_hit(inp_rect, UiAction::Focus(FocusTarget::NewEntryInput));
            self.draw_button(
                canvas,
                UiRect::new(w - 68, y + 22, 30, 24),
                "OK",
                Theme::ACCENT_GREEN,
                0x04120B,
                Theme::ACCENT_GREEN,
                Some(UiAction::ConfirmCreateEntry),
            );
            self.draw_button(
                canvas,
                UiRect::new(w - 35, y + 22, 26, 24),
                "X",
                Theme::BG_BUTTON,
                Theme::TEXT_SECONDARY,
                Theme::BORDER,
                Some(UiAction::CancelCreateEntry),
            );
            y += 58;
        }

        canvas.draw_hline(0, y - 2, w - 1, Theme::BORDER);

        let row_h = 23;
        let avail_rows = ((bottom - y - 4).max(0) / row_h) as usize;
        let max_scroll = self.entries.len().saturating_sub(avail_rows.max(1));
        if self.explorer_scroll > max_scroll {
            self.explorer_scroll = max_scroll;
        }

        let active_abs = self
            .tabs
            .get(self.active_tab)
            .and_then(|t| t.full_path.clone());

        let visible: Vec<FileEntry> = self
            .entries
            .iter()
            .skip(self.explorer_scroll)
            .take(avail_rows)
            .cloned()
            .collect();

        canvas.push_clip(0, y, w - 1, bottom - y);
        for (i, entry) in visible.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let row_rect = UiRect::new(2, ry, w - 6, row_h - 1);
            let is_open = active_abs.as_ref() == Some(&entry.path);
            let hovered = row_rect.contains(self.mouse_x, self.mouse_y);

            if is_open {
                canvas.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.w,
                    row_rect.h,
                    4,
                    0x182B49,
                );
            } else if hovered {
                canvas.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.w,
                    row_rect.h,
                    4,
                    0x131D31,
                );
            }

            let indent = 8 + (entry.depth.min(8) as i32) * 14;
            if entry.is_dir {
                let collapsed = self.collapsed_dirs.contains(&entry.rel_path);
                let glyph = if collapsed { ">" } else { "v" };
                canvas.draw_text(
                    indent,
                    ry + 3,
                    glyph,
                    FontRole::TerminalMono,
                    Theme::ACCENT_CYAN,
                );
                canvas.draw_text_clipped(
                    indent + 14,
                    ry + 3,
                    w - indent - 24,
                    &entry.name,
                    FontRole::UiSemibold,
                    Theme::TEXT_PRIMARY,
                );
                self.push_hit(row_rect, UiAction::ToggleDir(entry.rel_path));
            } else {
                let fam = LangFamily::from_path(&entry.name);
                let badge_col = match fam {
                    LangFamily::Rust => 0xF97316,
                    LangFamily::TypeScript => 0x38BDF8,
                    LangFamily::Python => 0xFACC15,
                    LangFamily::CFamily => 0x60A5FA,
                    LangFamily::Json | LangFamily::TomlYaml => 0xA855F7,
                    LangFamily::Markdown => 0x34D399,
                    _ => Theme::TEXT_MUTED,
                };
                canvas.draw_text(
                    indent,
                    ry + 4,
                    fam.badge(),
                    FontRole::TerminalMono,
                    badge_col,
                );
                let name_w = if hovered {
                    w - indent - 64
                } else {
                    w - indent - 32
                };
                canvas.draw_text_clipped(
                    indent + 28,
                    ry + 3,
                    name_w.max(24),
                    &entry.name,
                    FontRole::UiSmall,
                    if is_open {
                        0xFFFFFF
                    } else {
                        Theme::TEXT_SECONDARY
                    },
                );

                if hovered {
                    let del_rect = UiRect::new(w - 26, ry + 2, 20, row_h - 5);
                    canvas.fill_rounded_rect(
                        del_rect.x,
                        del_rect.y,
                        del_rect.w,
                        del_rect.h,
                        3,
                        0x3B1820,
                    );
                    canvas.draw_text(del_rect.x + 6, del_rect.y + 1, "x", FontRole::UiSmall, 0xFCA5A5);
                    self.push_hit(
                        del_rect,
                        UiAction::RequestDeleteFile(entry.path.clone(), entry.rel_path.clone()),
                    );
                    let open_rect =
                        UiRect::new(row_rect.x, row_rect.y, row_rect.w - 28, row_rect.h);
                    self.push_hit(
                        open_rect,
                        UiAction::OpenFile(entry.path, entry.rel_path, entry.name, false),
                    );
                } else {
                    self.push_hit(
                        row_rect,
                        UiAction::OpenFile(entry.path, entry.rel_path, entry.name, false),
                    );
                }
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
        let inp = UiRect::new(8, y, w - 62, 28);
        let focused = self.focus == FocusTarget::SearchInput;
        canvas.fill_rounded_rect(inp.x, inp.y, inp.w, inp.h, 5, Theme::BG_INPUT);
        canvas.stroke_rounded_rect(
            inp.x,
            inp.y,
            inp.w,
            inp.h,
            5,
            if focused {
                Theme::BORDER_FOCUS
            } else {
                Theme::BORDER
            },
        );
        let disp = if self.search_query.is_empty() && !focused {
            "Search workspace text...".to_string()
        } else if focused && self.blink_phase {
            format!("{}|", self.search_query)
        } else {
            self.search_query.clone()
        };
        canvas.draw_text_clipped(
            inp.x + 8,
            inp.y + 6,
            inp.w - 14,
            &disp,
            FontRole::UiSmall,
            if self.search_query.is_empty() && !focused {
                Theme::TEXT_MUTED
            } else {
                Theme::TEXT_PRIMARY
            },
        );
        self.push_hit(inp, UiAction::Focus(FocusTarget::SearchInput));
        self.draw_button(
            canvas,
            UiRect::new(w - 50, y, 42, 28),
            "Find",
            Theme::ACCENT_BLUE,
            0xFFFFFF,
            Theme::ACCENT_BLUE,
            Some(UiAction::RunSearch),
        );
        y += 34;

        canvas.draw_text(
            8,
            y,
            &format!("{} result(s)", self.search_results.len()),
            FontRole::UiSmall,
            Theme::TEXT_MUTED,
        );
        y += 20;

        let row_h = 38;
        let avail = ((bottom - y - 4).max(0) / row_h) as usize;
        let hits: Vec<SearchMatch> = self
            .search_results
            .iter()
            .skip(self.search_scroll)
            .take(avail)
            .cloned()
            .collect();

        canvas.push_clip(0, y, w - 1, bottom - y);
        for (i, hit) in hits.into_iter().enumerate() {
            let ry = y + i as i32 * row_h;
            let rect = UiRect::new(6, ry, w - 12, row_h - 3);
            let hov = rect.contains(self.mouse_x, self.mouse_y);
            canvas.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                4,
                if hov { Theme::BG_CARD_ALT } else { Theme::BG_CARD },
            );
            canvas.draw_text_clipped(
                rect.x + 6,
                ry + 3,
                rect.w - 12,
                &format!("{}:{}", hit.rel_path, hit.line),
                FontRole::UiSemibold,
                Theme::ACCENT_CYAN,
            );
            canvas.draw_text_clipped(
                rect.x + 6,
                ry + 19,
                rect.w - 12,
                &hit.preview,
                FontRole::TerminalMono,
                Theme::TEXT_SECONDARY,
            );
            self.push_hit(
                rect,
                UiAction::OpenSearchHit(hit.path, hit.rel_path, hit.line),
            );
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
        if !self.git_snapshot.is_repo {
            canvas.draw_text(
                12,
                y + 12,
                "Not a Git repository.",
                FontRole::UiRegular,
                Theme::TEXT_MUTED,
            );
            return;
        }

        canvas.fill_rounded_rect(8, y, w - 16, 26, 5, Theme::BG_CARD);
        canvas.draw_text_clipped(
            14,
            y + 5,
            w - 28,
            &format!("Branch: {}", self.git_snapshot.branch),
            FontRole::UiSemibold,
            Theme::ACCENT_GREEN,
        );
        y += 32;

        // Branch switcher pills
        if self.git_snapshot.branches.len() > 1 {
            let mut bx = 8;
            let branches = self.git_snapshot.branches.clone();
            for b in branches.into_iter().take(3) {
                let bw = ((w - 24) / 3).max(54);
                let active = b == self.git_snapshot.branch;
                self.draw_button(
                    canvas,
                    UiRect::new(bx, y, bw, 20),
                    &truncate_right(&b, 10),
                    if active { Theme::BG_CARD_ALT } else { Theme::BG_CARD },
                    if active {
                        Theme::ACCENT_CYAN
                    } else {
                        Theme::TEXT_MUTED
                    },
                    Theme::BORDER,
                    Some(UiAction::GitCheckout(b)),
                );
                bx += bw + 4;
            }
            y += 26;
        }

        let focused = self.focus == FocusTarget::CommitInput;
        let inp = UiRect::new(8, y, w - 16, 26);
        canvas.fill_rounded_rect(inp.x, inp.y, inp.w, inp.h, 4, Theme::BG_INPUT);
        canvas.stroke_rounded_rect(
            inp.x,
            inp.y,
            inp.w,
            inp.h,
            4,
            if focused {
                Theme::BORDER_FOCUS
            } else {
                Theme::BORDER
            },
        );
        let msg_disp = if self.commit_message.is_empty() && !focused {
            "Commit message...".to_string()
        } else if focused && self.blink_phase {
            format!("{}|", self.commit_message)
        } else {
            self.commit_message.clone()
        };
        canvas.draw_text_clipped(
            inp.x + 6,
            inp.y + 5,
            inp.w - 12,
            &msg_disp,
            FontRole::UiSmall,
            if self.commit_message.is_empty() && !focused {
                Theme::TEXT_MUTED
            } else {
                Theme::TEXT_PRIMARY
            },
        );
        self.push_hit(inp, UiAction::Focus(FocusTarget::CommitInput));
        y += 30;

        let half = (w - 20) / 2;
        self.draw_button(
            canvas,
            UiRect::new(8, y, half, 24),
            "Stage All",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::GitStageAll),
        );
        self.draw_button(
            canvas,
            UiRect::new(12 + half, y, half, 24),
            "Commit",
            Theme::ACCENT_GREEN,
            0x04120B,
            Theme::ACCENT_GREEN,
            Some(UiAction::GitCommit),
        );
        y += 30;

        canvas.draw_text(
            8,
            y,
            &format!("Working Tree ({})", self.git_snapshot.files.len()),
            FontRole::UiSemibold,
            Theme::TEXT_MUTED,
        );
        y += 18;

        let row_h = 26;
        let max_file_rows = ((bottom - y - 108).max(50) / row_h) as usize;
        let files_slice: Vec<_> = self
            .git_snapshot
            .files
            .iter()
            .skip(self.git_scroll)
            .take(max_file_rows)
            .cloned()
            .collect();

        for f in files_slice {
            let badge_col = match f.status.as_str() {
                "A" | "U" => Theme::ACCENT_GREEN,
                "D" => Theme::ACCENT_RED,
                _ => Theme::ACCENT_AMBER,
            };
            canvas.fill_rounded_rect(6, y, w - 12, row_h - 2, 4, Theme::BG_CARD);
            canvas.draw_text(12, y + 4, &f.status, FontRole::TerminalMono, badge_col);
            canvas.draw_text_clipped(
                30,
                y + 4,
                w - 114,
                &f.path,
                FontRole::UiSmall,
                Theme::TEXT_PRIMARY,
            );
            self.draw_button(
                canvas,
                UiRect::new(w - 78, y + 2, 36, row_h - 6),
                "Diff",
                Theme::BG_BUTTON,
                Theme::ACCENT_CYAN,
                Theme::BORDER,
                Some(UiAction::GitOpenDiff(f.path.clone())),
            );
            self.draw_button(
                canvas,
                UiRect::new(w - 38, y + 2, 28, row_h - 6),
                if f.staged { "-" } else { "+" },
                if f.staged { 0x3B2F14 } else { 0x123526 },
                if f.staged {
                    Theme::ACCENT_AMBER
                } else {
                    Theme::ACCENT_GREEN
                },
                Theme::BORDER,
                Some(UiAction::GitStageFile {
                    path: f.path,
                    currently_staged: f.staged,
                }),
            );
            y += row_h;
        }

        let log_y = (bottom - 94).max(y + 6);
        canvas.draw_hline(6, log_y - 4, w - 12, Theme::BORDER);
        canvas.draw_text(8, log_y, "Recent Commits", FontRole::UiSemibold, Theme::TEXT_MUTED);
        let mut cy = log_y + 18;
        for c in self.git_snapshot.commits.iter().take(4) {
            if cy + 16 > bottom {
                break;
            }
            canvas.draw_text(8, cy, &c.hash, FontRole::TerminalMono, Theme::ACCENT_PURPLE);
            canvas.draw_text_clipped(
                64,
                cy,
                w - 72,
                &c.subject,
                FontRole::UiSmall,
                Theme::TEXT_SECONDARY,
            );
            cy += 18;
        }
    }

    fn draw_center_editor_and_diff(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        top: i32,
        w: i32,
        h: i32,
    ) {
        canvas.fill_rect(x, top, w, h, Theme::BG_EDITOR);

        let strip_h = 34;
        canvas.fill_rect(x, top, w, strip_h, Theme::BG_SIDEBAR);
        canvas.draw_hline(x, top + strip_h - 1, w, Theme::BORDER);

        let mut tx = x + 4;
        let max_tab_x = x + w - 168;
        let tab_summaries: Vec<(String, bool, bool)> = self
            .tabs
            .iter()
            .map(|t| {
                let is_diff = matches!(t.content, TabContent::Diff { .. });
                (t.title.clone(), t.is_dirty(), is_diff)
            })
            .collect();

        for (idx, (title, dirty, is_diff)) in tab_summaries.into_iter().enumerate() {
            if tx + 90 > max_tab_x {
                break;
            }
            let tw = ((title.len() as i32 * 7) + 46).clamp(108, 195);
            let active = idx == self.active_tab;
            let tab_rect = UiRect::new(tx, top + 4, tw, strip_h - 5);
            canvas.fill_rounded_rect(
                tab_rect.x,
                tab_rect.y,
                tab_rect.w,
                tab_rect.h,
                4,
                if active {
                    Theme::BG_EDITOR
                } else {
                    Theme::BG_CARD
                },
            );
            if active {
                canvas.fill_rect(tab_rect.x, tab_rect.y, tab_rect.w, 2, Theme::ACCENT_CYAN);
            }
            let label = if dirty {
                format!("* {}", title)
            } else {
                title
            };
            canvas.draw_text_clipped(
                tx + 8,
                top + 9,
                tw - 30,
                &label,
                if active {
                    FontRole::UiSemibold
                } else {
                    FontRole::UiSmall
                },
                if is_diff {
                    Theme::ACCENT_AMBER
                } else if active {
                    Theme::TEXT_PRIMARY
                } else {
                    Theme::TEXT_SECONDARY
                },
            );
            let close_rect = UiRect::new(tx + tw - 22, top + 8, 18, 18);
            canvas.draw_text(
                close_rect.x + 5,
                close_rect.y + 1,
                "x",
                FontRole::UiSmall,
                Theme::TEXT_MUTED,
            );
            self.push_hit(
                UiRect::new(tx, top + 4, tw - 24, strip_h - 5),
                UiAction::SelectEditorTab(idx),
            );
            self.push_hit(close_rect, UiAction::CloseEditorTab(idx));
            tx += tw + 4;
        }

        self.draw_button(
            canvas,
            UiRect::new(x + w - 160, top + 5, 50, 24),
            "+ New",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::NewScratchTab),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 106, top + 5, 48, 24),
            "Find",
            if self.show_find_bar {
                Theme::ACCENT_BLUE
            } else {
                Theme::BG_BUTTON
            },
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::ToggleFindBar),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 54, top + 5, 48, 24),
            "Save",
            Theme::ACCENT_BLUE,
            0xFFFFFF,
            Theme::ACCENT_BLUE,
            Some(UiAction::SaveActiveTab),
        );

        let mut content_y = top + strip_h;

        if self.show_find_bar {
            canvas.fill_rect(x, content_y, w, 32, Theme::BG_CARD);
            canvas.draw_hline(x, content_y + 31, w, Theme::BORDER);
            canvas.draw_text(
                x + 10,
                content_y + 8,
                "Find:",
                FontRole::UiSmall,
                Theme::TEXT_SECONDARY,
            );
            let f_inp = UiRect::new(x + 48, content_y + 4, 240.min(w - 150), 24);
            let focused = self.focus == FocusTarget::FindInput;
            canvas.fill_rect(f_inp.x, f_inp.y, f_inp.w, f_inp.h, Theme::BG_INPUT);
            canvas.stroke_rect(
                f_inp.x,
                f_inp.y,
                f_inp.w,
                f_inp.h,
                if focused {
                    Theme::BORDER_FOCUS
                } else {
                    Theme::BORDER
                },
            );
            let shown = if focused && self.blink_phase {
                format!("{}|", self.find_query)
            } else {
                self.find_query.clone()
            };
            canvas.draw_text_clipped(
                f_inp.x + 6,
                f_inp.y + 4,
                f_inp.w - 10,
                &shown,
                FontRole::TerminalMono,
                Theme::TEXT_PRIMARY,
            );
            self.push_hit(f_inp, UiAction::Focus(FocusTarget::FindInput));
            self.draw_button(
                canvas,
                UiRect::new(f_inp.x + f_inp.w + 6, content_y + 4, 56, 24),
                "Next",
                Theme::ACCENT_BLUE,
                0xFFFFFF,
                Theme::ACCENT_BLUE,
                Some(UiAction::FindNext),
            );
            content_y += 32;
        }

        let body_h = (top + h - content_y).max(20);
        self.editor_rect = UiRect::new(x, content_y, w, body_h);

        if self.tabs.is_empty() {
            let cw = 430.min(w - 30);
            let cx = x + ((w - cw) / 2).max(12);
            let cy = content_y + (body_h / 2 - 105).max(16);
            canvas.fill_rounded_rect(cx, cy, cw, 206, 8, Theme::BG_CARD);
            canvas.stroke_rounded_rect(cx, cy, cw, 206, 8, Theme::BORDER);
            canvas.draw_text(
                cx + 22,
                cy + 20,
                "AI Coding Agent — 100% Pure Rust Native GUI",
                FontRole::UiTitle,
                Theme::TEXT_PRIMARY,
            );
            canvas.draw_text(
                cx + 22,
                cy + 46,
                "Zero WebView. Native ClearType Editor, Side-by-Side Diff & ConPTY.",
                FontRole::UiSmall,
                Theme::TEXT_SECONDARY,
            );
            self.draw_button(
                canvas,
                UiRect::new(cx + 22, cy + 82, 182, 32),
                "Open Project Folder",
                Theme::ACCENT_BLUE,
                0xFFFFFF,
                Theme::ACCENT_BLUE,
                Some(UiAction::PickFolder),
            );
            self.draw_button(
                canvas,
                UiRect::new(cx + 216, cy + 82, 182, 32),
                "New Scratch Buffer",
                Theme::BG_BUTTON,
                Theme::TEXT_PRIMARY,
                Theme::BORDER,
                Some(UiAction::NewScratchTab),
            );
            self.draw_button(
                canvas,
                UiRect::new(cx + 22, cy + 126, 182, 32),
                "Configure AI Keys",
                Theme::BG_BUTTON,
                Theme::ACCENT_CYAN,
                Theme::BORDER,
                Some(UiAction::OpenSettings(SettingsTab::Providers)),
            );
            self.draw_button(
                canvas,
                UiRect::new(cx + 216, cy + 126, 182, 32),
                "Explain This Repo",
                Theme::BG_BUTTON,
                Theme::ACCENT_GREEN,
                Theme::BORDER,
                Some(UiAction::QuickPrompt(
                    "Inspect the workspace architecture and summarize key modules.",
                )),
            );
            return;
        }

        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };

        let (char_w_raw, line_h_raw) = canvas.mono_cell_size(FontRole::CodeMono);
        let char_w = char_w_raw.max(7);
        let line_h = line_h_raw.max(19);

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
                let gutter_w = 56;
                canvas.fill_rect(x, content_y, gutter_w, body_h, Theme::BG_EDITOR_GUTTER);
                canvas.draw_vline(x + gutter_w - 1, content_y, body_h, Theme::BORDER);

                let vis_rows = (body_h / line_h) as usize + 1;
                let sel = tab.ordered_selection();
                let lang = tab.lang;
                let editor_focused =
                    self.focus == FocusTarget::Editor && self.settings_modal.is_none();

                for idx in 0..vis_rows {
                    let row = *scroll_y + idx;
                    if row >= lines.len() {
                        break;
                    }
                    let ry = content_y + 6 + idx as i32 * line_h;
                    if row == *cursor_line {
                        canvas.fill_rect(
                            x + gutter_w,
                            ry - 2,
                            w - gutter_w,
                            line_h,
                            Theme::BG_ACTIVE_LINE,
                        );
                    }

                    let ln_str = format!("{:>4}", row + 1);
                    canvas.draw_text(
                        x + 8,
                        ry,
                        &ln_str,
                        FontRole::CodeMono,
                        if row == *cursor_line {
                            Theme::ACCENT_CYAN
                        } else {
                            Theme::TEXT_MUTED
                        },
                    );

                    let line = &lines[row];
                    let total_chars = char_len(line);

                    // Selection highlight
                    if let Some(((sr, sc), (er, ec))) = sel {
                        if row >= sr && row <= er {
                            let s_c = if row == sr { sc } else { 0 };
                            let e_c = if row == er { ec } else { total_chars + 1 };
                            if e_c > *scroll_x {
                                let vis_s = s_c.saturating_sub(*scroll_x);
                                let vis_e = e_c.saturating_sub(*scroll_x);
                                let sx = x + gutter_w + 8 + (vis_s as i32) * char_w;
                                let sw = ((vis_e - vis_s) as i32 * char_w).max(6);
                                canvas.fill_rect(sx, ry - 2, sw, line_h, Theme::BG_SELECTION);
                            }
                        }
                    }

                    // Lexical syntax-highlighted spans
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
                                    sp.kind.rgb(),
                                );
                            }
                        }
                        col_cursor = sp_end;
                    }

                    // Blinking caret
                    if editor_focused
                        && self.blink_phase
                        && row == *cursor_line
                        && *cursor_col >= *scroll_x
                    {
                        let cx = x + gutter_w + 8 + ((*cursor_col - *scroll_x) as i32) * char_w;
                        canvas.fill_rect(cx, ry - 1, 2, line_h - 2, Theme::ACCENT_CYAN);
                    }
                }
            }
            TabContent::Diff { rows, scroll_y } => {
                let half_w = w / 2;
                canvas.fill_rect(x, content_y, w, 24, Theme::BG_CARD);
                canvas.draw_text_clipped(
                    x + 10,
                    content_y + 4,
                    half_w - 20,
                    "HEAD (Original)",
                    FontRole::UiSemibold,
                    0xFCA5A5,
                );
                canvas.draw_text_clipped(
                    x + half_w + 10,
                    content_y + 4,
                    half_w - 20,
                    "Working Copy (Modified)",
                    FontRole::UiSemibold,
                    0x6EE7B7,
                );
                canvas.draw_vline(x + half_w, content_y, body_h, Theme::BORDER);

                let d_line_h = 19;
                let vis_rows = ((body_h - 26) / d_line_h) as usize + 1;
                for idx in 0..vis_rows {
                    let r_i = *scroll_y + idx;
                    let Some(row) = rows.get(r_i) else {
                        break;
                    };
                    let ry = content_y + 26 + idx as i32 * d_line_h;
                    let (l_bg, r_bg) = match row.kind {
                        DiffLineKind::Unchanged => (Theme::BG_EDITOR, Theme::BG_EDITOR),
                        DiffLineKind::Removed => (Theme::DIFF_DEL_BG, 0x0B101B),
                        DiffLineKind::Added => (0x0B101B, Theme::DIFF_ADD_BG),
                        DiffLineKind::Changed => (Theme::DIFF_DEL_BG, Theme::DIFF_ADD_BG),
                    };
                    canvas.fill_rect(x, ry, half_w, d_line_h, l_bg);
                    canvas.fill_rect(x + half_w + 1, ry, half_w - 1, d_line_h, r_bg);

                    if let Some(ln) = row.left_no {
                        canvas.draw_text(
                            x + 6,
                            ry + 1,
                            &format!("{:>4}", ln),
                            FontRole::TerminalMono,
                            Theme::TEXT_MUTED,
                        );
                    }
                    canvas.draw_text_clipped(
                        x + 46,
                        ry + 1,
                        half_w - 52,
                        &row.left_text,
                        FontRole::TerminalMono,
                        Theme::TEXT_PRIMARY,
                    );

                    if let Some(rn) = row.right_no {
                        canvas.draw_text(
                            x + half_w + 8,
                            ry + 1,
                            &format!("{:>4}", rn),
                            FontRole::TerminalMono,
                            Theme::TEXT_MUTED,
                        );
                    }
                    canvas.draw_text_clipped(
                        x + half_w + 48,
                        ry + 1,
                        half_w - 54,
                        &row.right_text,
                        FontRole::TerminalMono,
                        Theme::TEXT_PRIMARY,
                    );
                }
            }
        }
        canvas.pop_clip();
    }

    fn draw_bottom_terminal(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        top: i32,
        w: i32,
        h: i32,
    ) {
        canvas.fill_rect(x, top, w, h, Theme::BG_TERMINAL);
        canvas.draw_hline(x, top, w, Theme::BORDER);

        let bar_h = 30;
        canvas.fill_rect(x, top + 1, w, bar_h, Theme::BG_SIDEBAR);
        canvas.draw_hline(x, top + bar_h, w, Theme::BORDER);

        let mut tx = x + 8;
        let term_titles: Vec<String> = self.terminals.iter().map(|t| t.title.clone()).collect();
        for (idx, title) in term_titles.into_iter().enumerate() {
            let tw = 128;
            let active = idx == self.active_term;
            canvas.fill_rounded_rect(
                tx,
                top + 4,
                tw,
                22,
                4,
                if active {
                    Theme::BG_TERMINAL
                } else {
                    Theme::BG_CARD
                },
            );
            canvas.draw_text_clipped(
                tx + 8,
                top + 6,
                tw - 28,
                &title,
                FontRole::UiSemibold,
                if active {
                    Theme::ACCENT_GREEN
                } else {
                    Theme::TEXT_SECONDARY
                },
            );
            canvas.draw_text(
                tx + tw - 16,
                top + 6,
                "x",
                FontRole::UiSmall,
                Theme::TEXT_MUTED,
            );
            self.push_hit(
                UiRect::new(tx, top + 4, tw - 20, 22),
                UiAction::SelectPtyTab(idx),
            );
            self.push_hit(
                UiRect::new(tx + tw - 20, top + 4, 20, 22),
                UiAction::ClosePtyTab(idx),
            );
            tx += tw + 4;
        }

        let ps_lbl = if cfg!(target_os = "windows") {
            "+ PowerShell"
        } else {
            "+ Shell"
        };
        self.draw_button(
            canvas,
            UiRect::new(x + w - 306, top + 4, 92, 22),
            ps_lbl,
            Theme::BG_BUTTON,
            Theme::ACCENT_CYAN,
            Theme::BORDER,
            Some(UiAction::SpawnPtyTab("powershell")),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 210, top + 4, 58, 22),
            "+ CMD",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::SpawnPtyTab("cmd")),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 148, top + 4, 46, 22),
            "CWD",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::PtySyncCwd),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 98, top + 4, 46, 22),
            "^C",
            0x3B1820,
            0xFCA5A5,
            Theme::BORDER,
            Some(UiAction::PtySendCtrlC),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 48, top + 4, 42, 22),
            "Clr",
            Theme::BG_BUTTON,
            Theme::TEXT_MUTED,
            Theme::BORDER,
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
            let focused = self.focus == FocusTarget::Terminal && self.settings_modal.is_none();

            for r in 0..term.screen.rows {
                let ry = grid_y + r as i32 * row_h;
                let Some(line) = term.screen.visible_row(r) else {
                    continue;
                };
                let mut col = 0usize;
                while col < line.len() {
                    let style = line[col];
                    let mut end = col + 1;
                    let mut run = String::new();
                    run.push(style.ch);
                    while end < line.len() && line[end].fg == style.fg && line[end].bg == style.bg {
                        run.push(line[end].ch);
                        end += 1;
                    }
                    let rx = self.term_grid_rect.x + col as i32 * char_w;
                    let rw = (end - col) as i32 * char_w;
                    if style.bg != Theme::BG_TERMINAL {
                        canvas.fill_rect(rx, ry, rw, row_h, style.bg);
                    }
                    if run.chars().any(|c| c != ' ') {
                        canvas.draw_text(rx, ry, &run, FontRole::TerminalMono, style.fg);
                    }
                    col = end;
                }
            }

            if focused && term.screen.scroll_offset == 0 && self.blink_phase {
                let cx = self.term_grid_rect.x + term.screen.cursor_col as i32 * char_w;
                let cy = grid_y + term.screen.cursor_row as i32 * row_h;
                canvas.fill_rect(cx, cy + 1, char_w, row_h - 2, Theme::ACCENT_GREEN);
            }
            canvas.pop_clip();
        }
    }

    fn draw_right_agent_panel(&mut self, canvas: &mut dyn SurfaceCanvas, top: i32, bottom: i32) {
        let x = self.width - self.right_w;
        let w = self.right_w;
        let h = bottom - top;
        canvas.fill_rect(x, top, w, h, Theme::BG_AGENT);
        canvas.draw_vline(x, top, h, Theme::BORDER);

        canvas.fill_rect(x + 1, top, w - 1, 36, Theme::BG_SIDEBAR);
        canvas.draw_hline(x + 1, top + 35, w - 1, Theme::BORDER);
        canvas.draw_text_clipped(
            x + 10,
            top + 9,
            w - 130,
            &format!("Session: {}", self.active_session.title),
            FontRole::UiSemibold,
            Theme::TEXT_PRIMARY,
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 114, top + 6, 56, 24),
            "+ New",
            Theme::ACCENT_BLUE,
            0xFFFFFF,
            Theme::ACCENT_BLUE,
            Some(UiAction::NewSession),
        );
        let cur_sid = self.active_session.id.clone();
        self.draw_button(
            canvas,
            UiRect::new(x + w - 52, top + 6, 44, 24),
            "Clear",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::DeleteSession(cur_sid)),
        );

        let mut sx = x + 8;
        let sess_y = top + 40;
        let sess_list: Vec<(String, String)> = self
            .sessions
            .iter()
            .take(3)
            .map(|s| (s.id.clone(), s.title.clone()))
            .collect();
        for (sid, stitle) in sess_list {
            let pw = (w - 24) / 3;
            let active = sid == self.active_session.id;
            self.draw_button(
                canvas,
                UiRect::new(sx, sess_y, pw, 22),
                &truncate_right(&stitle, 14),
                if active {
                    Theme::BG_CARD_ALT
                } else {
                    Theme::BG_CARD
                },
                if active {
                    Theme::ACCENT_CYAN
                } else {
                    Theme::TEXT_MUTED
                },
                Theme::BORDER,
                Some(UiAction::SelectSession(sid)),
            );
            sx += pw + 4;
        }

        let comp_h = 128;
        let comp_top = bottom - comp_h;
        let feed_top = sess_y + 28;
        let feed_bottom = comp_top - 26;

        let wrap_cols = ((w - 36).max(160) / 7) as usize;
        let mut rendered_blocks: Vec<(u32, u32, String, Option<String>)> = Vec::new();

        if self.active_session.messages.is_empty() && self.live_tools.is_empty() {
            rendered_blocks.push((
                Theme::BG_CARD,
                Theme::ACCENT_CYAN,
                "Autonomous Coding Agent Ready".to_string(),
                None,
            ));
            for info in [
                "Modes: Ask (approval-gated), Plan (read-only architect), Auto (multi-step edit + shell + verify).",
                "9 Built-in Tools: read_file, write_file, edit_file, search_project, execute_command, install_dependencies, run_tests, diagnose_errors, git_operation.",
                "Tip: Click a Quick Action chip below or type any coding task!",
            ] {
                for wl in wrap_text(info, wrap_cols) {
                    rendered_blocks.push((Theme::BG_CARD, Theme::TEXT_SECONDARY, wl, None));
                }
            }
        }

        for msg in &self.active_session.messages {
            let is_user = msg.role == "user";
            let role_hdr = if is_user { "YOU" } else { "AI AGENT" };
            let bg = if is_user { 0x152542 } else { Theme::BG_CARD };
            let hdr_fg = if is_user {
                Theme::ACCENT_CYAN
            } else {
                Theme::ACCENT_GREEN
            };
            rendered_blocks.push((bg, hdr_fg, format!("[{}]", role_hdr), None));
            if !msg.thinking.trim().is_empty() {
                for rl in wrap_text(&format!("Thinking: {}", msg.thinking.trim()), wrap_cols)
                    .into_iter()
                    .take(4)
                {
                    rendered_blocks.push((bg, Theme::TEXT_MUTED, rl, None));
                }
            }
            for tool in &msg.tools {
                let st_col = if tool.status == "ok" {
                    Theme::ACCENT_GREEN
                } else {
                    Theme::ACCENT_RED
                };
                let diff_rel = if tool.file_path.is_empty() {
                    None
                } else {
                    Some(tool.file_path.clone())
                };
                rendered_blocks.push((
                    0x101A2C,
                    st_col,
                    format!(
                        "TOOL {} [{} - {}ms]",
                        tool.tool_name,
                        tool.status.to_uppercase(),
                        tool.duration_ms
                    ),
                    diff_rel,
                ));
                if !tool.result_summary.is_empty() {
                    for tl in wrap_text(&tool.result_summary, wrap_cols)
                        .into_iter()
                        .take(2)
                    {
                        rendered_blocks.push((0x101A2C, Theme::TEXT_SECONDARY, tl, None));
                    }
                }
            }
            for wl in wrap_text(&msg.content, wrap_cols) {
                rendered_blocks.push((bg, Theme::TEXT_PRIMARY, wl, None));
            }
            rendered_blocks.push((Theme::BG_AGENT, Theme::TEXT_MUTED, String::new(), None));
        }

        // Live in-flight tools & streaming deltas
        for rec in &self.live_tools {
            let st_col = match rec.status.as_str() {
                "ok" => Theme::ACCENT_GREEN,
                "running" => Theme::ACCENT_AMBER,
                _ => Theme::ACCENT_RED,
            };
            let diff_rel = if rec.file_path.is_empty() {
                None
            } else {
                Some(rec.file_path.clone())
            };
            rendered_blocks.push((
                0x101A2C,
                st_col,
                format!(
                    "LIVE {} [{} - {}ms]",
                    rec.tool_name,
                    rec.status.to_uppercase(),
                    rec.duration_ms
                ),
                diff_rel,
            ));
            for wl in wrap_text(&rec.result_summary, wrap_cols).into_iter().take(2) {
                rendered_blocks.push((0x101A2C, Theme::TEXT_SECONDARY, wl, None));
            }
        }
        if !self.streaming_text.is_empty() {
            for wl in wrap_text(&self.streaming_text, wrap_cols) {
                rendered_blocks.push((Theme::BG_CARD, Theme::TEXT_PRIMARY, wl, None));
            }
        }

        let line_h = 19;
        let vis_lines = ((feed_bottom - feed_top).max(20) / line_h) as usize;
        let total = rendered_blocks.len();
        let max_off = total.saturating_sub(vis_lines);
        let start_idx = if self.agent_scroll_lines == 0 {
            max_off
        } else {
            max_off.saturating_sub(self.agent_scroll_lines.min(max_off))
        };

        canvas.push_clip(x + 2, feed_top, w - 4, (feed_bottom - feed_top).max(10));
        for (i, (bg, fg, text, diff_path)) in rendered_blocks
            .into_iter()
            .skip(start_idx)
            .take(vis_lines)
            .enumerate()
        {
            let ly = feed_top + i as i32 * line_h;
            if bg != Theme::BG_AGENT {
                canvas.fill_rect(x + 8, ly, w - 16, line_h, bg);
            }
            let text_max_w = if diff_path.is_some() { w - 82 } else { w - 24 };
            canvas.draw_text_clipped(x + 14, ly + 1, text_max_w, &text, FontRole::UiSmall, fg);
            if let Some(rel) = diff_path {
                self.draw_button(
                    canvas,
                    UiRect::new(x + w - 64, ly + 1, 52, line_h - 2),
                    "Diff",
                    Theme::BG_BUTTON,
                    Theme::ACCENT_CYAN,
                    Theme::BORDER,
                    Some(UiAction::GitOpenDiff(rel)),
                );
            }
        }
        canvas.pop_clip();

        canvas.fill_rect(x + 1, feed_bottom, w - 1, 24, Theme::BG_SIDEBAR);
        let st_col = if self.agent_running {
            Theme::ACCENT_AMBER
        } else {
            Theme::TEXT_MUTED
        };
        canvas.draw_text_clipped(
            x + 10,
            feed_bottom + 4,
            w - 20,
            &self.agent_status_line,
            FontRole::UiSmall,
            st_col,
        );

        canvas.fill_rect(x + 1, comp_top, w - 1, comp_h, Theme::BG_SIDEBAR);
        canvas.draw_hline(x + 1, comp_top, w - 1, Theme::BORDER);

        let chip_w = (w - 24) / 3;
        self.draw_button(
            canvas,
            UiRect::new(x + 6, comp_top + 6, chip_w, 22),
            "Explain Repo",
            Theme::BG_CARD,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::QuickPrompt(
                "Inspect the repository files and explain the architecture concisely.",
            )),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + 10 + chip_w, comp_top + 6, chip_w, 22),
            "Diagnose",
            Theme::BG_CARD,
            Theme::ACCENT_AMBER,
            Theme::BORDER,
            Some(UiAction::QuickPrompt(
                "Run diagnose_errors on this workspace and fix any compiler or test issues found.",
            )),
        );
        self.draw_button(
            canvas,
            UiRect::new(x + 14 + chip_w * 2, comp_top + 6, chip_w, 22),
            "Git Summary",
            Theme::BG_CARD,
            Theme::ACCENT_CYAN,
            Theme::BORDER,
            Some(UiAction::QuickPrompt(
                "Check git status & diff and summarize all uncommitted changes.",
            )),
        );

        let box_rect = UiRect::new(x + 8, comp_top + 34, w - 16, 58);
        let focused = self.focus == FocusTarget::AgentPrompt && self.settings_modal.is_none();
        canvas.fill_rounded_rect(
            box_rect.x,
            box_rect.y,
            box_rect.w,
            box_rect.h,
            6,
            Theme::BG_INPUT,
        );
        canvas.stroke_rounded_rect(
            box_rect.x,
            box_rect.y,
            box_rect.w,
            box_rect.h,
            6,
            if focused {
                Theme::BORDER_FOCUS
            } else {
                Theme::BORDER
            },
        );
        self.push_hit(box_rect, UiAction::Focus(FocusTarget::AgentPrompt));

        let prompt_disp = if self.agent_prompt.is_empty() && !focused {
            "Ask AI Agent to build features, refactor files, or fix bugs...".to_string()
        } else if focused && self.blink_phase {
            format!("{}|", self.agent_prompt)
        } else {
            self.agent_prompt.clone()
        };
        let prompt_lines = wrap_text(&prompt_disp, wrap_cols);
        let p_color = if self.agent_prompt.is_empty() && !focused {
            Theme::TEXT_MUTED
        } else {
            Theme::TEXT_PRIMARY
        };
        for (idx, pl) in prompt_lines.iter().rev().take(3).rev().enumerate() {
            canvas.draw_text_clipped(
                box_rect.x + 8,
                box_rect.y + 6 + idx as i32 * 17,
                box_rect.w - 16,
                pl,
                FontRole::UiSmall,
                p_color,
            );
        }

        canvas.draw_text(
            x + 10,
            comp_top + 98,
            "Enter: Run  |  Shift+Enter: Newline",
            FontRole::UiSmall,
            Theme::TEXT_MUTED,
        );
        self.draw_button(
            canvas,
            UiRect::new(x + w - 104, comp_top + 96, 96, 26),
            if self.agent_running {
                "Running..."
            } else {
                "Run Agent"
            },
            if self.agent_running {
                Theme::BG_BUTTON
            } else {
                Theme::ACCENT_BLUE
            },
            0xFFFFFF,
            Theme::ACCENT_BLUE,
            Some(UiAction::SendAgentPrompt),
        );
    }

    fn draw_status_footer(&mut self, canvas: &mut dyn SurfaceCanvas) {
        let h = 24;
        let y = self.height - h;
        canvas.fill_rect(0, y, self.width, h, Theme::BG_TITLEBAR);
        canvas.draw_hline(0, y, self.width, Theme::BORDER);

        let left_info = format!(
            "  Branch: {}   |   Mode: {}   |   Provider: {} ({})",
            if self.git_snapshot.is_repo {
                &self.git_snapshot.branch
            } else {
                "no-git"
            },
            self.settings.default_mode.to_uppercase(),
            self.settings.active_provider,
            self.settings.active_model
        );
        canvas.draw_text_clipped(
            6,
            y + 4,
            self.width / 2,
            &left_info,
            FontRole::UiSmall,
            Theme::TEXT_SECONDARY,
        );

        let right_info = if self.toast_ticks > 0 {
            self.toast_text.clone()
        } else if let Some(tab) = self.tabs.get(self.active_tab) {
            if let TabContent::Code {
                lines,
                cursor_line,
                cursor_col,
                ..
            } = &tab.content
            {
                format!(
                    "Ln {}, Col {}  |  {} lines  |  UTF-8  |  Pure-Rust Native GUI",
                    cursor_line + 1,
                    cursor_col + 1,
                    lines.len()
                )
            } else {
                "Side-by-Side Diff Viewer  |  Pure-Rust Native GUI".to_string()
            }
        } else {
            "100% Pure Rust Native GUI (Win32 GDI + ConPTY + WinHTTP + SQLite)".to_string()
        };
        let tw = canvas.measure_text_width(&right_info, FontRole::UiSmall);
        canvas.draw_text_clipped(
            (self.width - tw - 14).max(self.width / 2),
            y + 4,
            self.width / 2,
            &right_info,
            FontRole::UiSmall,
            if self.toast_ticks > 0 {
                Theme::ACCENT_CYAN
            } else {
                Theme::TEXT_MUTED
            },
        );
    }

    fn draw_settings_modal(&mut self, canvas: &mut dyn SurfaceCanvas, tab: SettingsTab) {
        let mw = 760.min(self.width - 60);
        let mh = 545.min(self.height - 50);
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;

        canvas.fill_rounded_rect(mx, my, mw, mh, 8, Theme::BG_MODAL);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 8, Theme::ACCENT_CYAN);

        canvas.draw_text(
            mx + 20,
            my + 14,
            "Settings & SQLite Audit Store",
            FontRole::UiTitle,
            Theme::TEXT_PRIMARY,
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 76, my + 10, 62, 26),
            "Close",
            Theme::BG_BUTTON,
            Theme::TEXT_SECONDARY,
            Theme::BORDER,
            Some(UiAction::CloseModal),
        );

        let tabs = [
            (SettingsTab::Providers, "AI Providers & Keys"),
            (SettingsTab::Permissions, "Safety & Shell"),
            (SettingsTab::ToolAudit, "SQLite Tool History"),
        ];
        for (i, (st, label)) in tabs.iter().enumerate() {
            self.draw_button(
                canvas,
                UiRect::new(mx + 20 + i as i32 * 184, my + 44, 176, 28),
                label,
                if *st == tab {
                    Theme::ACCENT_BLUE
                } else {
                    Theme::BG_CARD
                },
                0xFFFFFF,
                Theme::BORDER,
                Some(UiAction::SwitchSettingsTab(*st)),
            );
        }

        let mut y = my + 84;
        match tab {
            SettingsTab::Providers => {
                canvas.draw_text(
                    mx + 20,
                    y,
                    "Active Provider:",
                    FontRole::UiSemibold,
                    Theme::TEXT_SECONDARY,
                );
                let provs: [&'static str; 6] = [
                    "ollama",
                    "openai",
                    "anthropic",
                    "gemini",
                    "openrouter",
                    "custom",
                ];
                for (i, p) in provs.iter().enumerate() {
                    let active = self.settings_draft.active_provider == *p;
                    self.draw_button(
                        canvas,
                        UiRect::new(mx + 140 + i as i32 * 96, y - 4, 90, 26),
                        &p.to_uppercase(),
                        if active {
                            Theme::ACCENT_CYAN
                        } else {
                            Theme::BG_CARD
                        },
                        if active { 0x041019 } else { Theme::TEXT_PRIMARY },
                        Theme::BORDER,
                        Some(UiAction::SelectSettingsProvider(p)),
                    );
                }
                y += 36;

                let model_val = self.settings_draft.active_model.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 170,
                    "Active Model:",
                    &model_val,
                    false,
                    FocusTarget::SettingsModel,
                );
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 140, y, 120, 26),
                    "Probe Ollama",
                    Theme::BG_BUTTON,
                    Theme::ACCENT_GREEN,
                    Theme::BORDER,
                    Some(UiAction::ProbeOllama),
                );
                y += 34;

                if !self.ollama_models.is_empty() {
                    let mut ox = mx + 150;
                    let models = self.ollama_models.clone();
                    for m in models.into_iter().take(4) {
                        let short = truncate_right(&m, 16);
                        self.draw_button(
                            canvas,
                            UiRect::new(ox, y - 2, 136, 22),
                            &short,
                            Theme::BG_CARD,
                            Theme::ACCENT_CYAN,
                            Theme::BORDER,
                            Some(UiAction::SelectPresetModel(m)),
                        );
                        ox += 142;
                    }
                    y += 28;
                }

                let oai = self.settings_draft.openai_api_key.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "OpenAI API Key:",
                    &oai,
                    true,
                    FocusTarget::SettingsOpenAiKey,
                );
                y += 34;

                let ant = self.settings_draft.anthropic_api_key.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "Anthropic Key:",
                    &ant,
                    true,
                    FocusTarget::SettingsAnthropicKey,
                );
                y += 34;

                let gem = self.settings_draft.gemini_api_key.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "Gemini API Key:",
                    &gem,
                    true,
                    FocusTarget::SettingsGeminiKey,
                );
                y += 34;

                let or_key = self.settings_draft.openrouter_api_key.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "OpenRouter Key:",
                    &or_key,
                    true,
                    FocusTarget::SettingsOpenRouterKey,
                );
                y += 34;

                let oll_url = self.settings_draft.ollama_base_url.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "Ollama Base URL:",
                    &oll_url,
                    false,
                    FocusTarget::SettingsOllamaUrl,
                );
                y += 34;

                let cust_url = self.settings_draft.custom_base_url.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "Custom API URL:",
                    &cust_url,
                    false,
                    FocusTarget::SettingsCustomUrl,
                );
                y += 34;

                let cust_key = self.settings_draft.custom_api_key.clone();
                self.draw_modal_input_row(
                    canvas,
                    mx + 20,
                    y,
                    mw - 40,
                    "Custom API Key:",
                    &cust_key,
                    true,
                    FocusTarget::SettingsCustomKey,
                );
            }
            SettingsTab::Permissions => {
                let toggles: [(&'static str, &str, bool); 3] = [
                    (
                        "require_shell_approval",
                        "Require explicit approval before Agent runs shell/PowerShell commands",
                        self.settings_draft.require_shell_approval,
                    ),
                    (
                        "require_delete_confirmation",
                        "Require confirmation dialog before deleting workspace files",
                        self.settings_draft.require_delete_confirmation,
                    ),
                    (
                        "restrict_to_workspace",
                        "Strict Workspace Sandbox (prevent file writes outside active folder)",
                        self.settings_draft.restrict_to_workspace,
                    ),
                ];
                for (field, desc, val) in toggles {
                    canvas.fill_rounded_rect(mx + 20, y, mw - 40, 38, 6, Theme::BG_CARD);
                    canvas.draw_text(mx + 32, y + 10, desc, FontRole::UiSmall, Theme::TEXT_PRIMARY);
                    self.draw_button(
                        canvas,
                        UiRect::new(mx + mw - 104, y + 6, 72, 26),
                        if val { "ENABLED" } else { "OFF" },
                        if val {
                            Theme::ACCENT_GREEN
                        } else {
                            Theme::BG_BUTTON
                        },
                        if val { 0x04120B } else { Theme::TEXT_SECONDARY },
                        Theme::BORDER,
                        Some(UiAction::ToggleSettingBool(field)),
                    );
                    y += 46;
                }

                canvas.fill_rounded_rect(mx + 20, y, mw - 40, 38, 6, Theme::BG_CARD);
                canvas.draw_text(
                    mx + 32,
                    y + 10,
                    &format!(
                        "Max Autonomous Loop Iterations per Turn: {}",
                        self.settings_draft.max_agent_iterations
                    ),
                    FontRole::UiSmall,
                    Theme::TEXT_PRIMARY,
                );
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 116, y + 6, 38, 26),
                    "-",
                    Theme::BG_BUTTON,
                    Theme::TEXT_PRIMARY,
                    Theme::BORDER,
                    Some(UiAction::AdjustMaxIters(-2)),
                );
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 72, y + 6, 38, 26),
                    "+",
                    Theme::BG_BUTTON,
                    Theme::TEXT_PRIMARY,
                    Theme::BORDER,
                    Some(UiAction::AdjustMaxIters(2)),
                );
                y += 46;

                canvas.fill_rounded_rect(mx + 20, y, mw - 40, 38, 6, Theme::BG_CARD);
                canvas.draw_text(
                    mx + 32,
                    y + 10,
                    &format!(
                        "Code Editor Font Size: {} pt",
                        self.settings_draft.editor_font_size
                    ),
                    FontRole::UiSmall,
                    Theme::TEXT_PRIMARY,
                );
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 116, y + 6, 38, 26),
                    "-",
                    Theme::BG_BUTTON,
                    Theme::TEXT_PRIMARY,
                    Theme::BORDER,
                    Some(UiAction::AdjustFontSize(-1)),
                );
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 72, y + 6, 38, 26),
                    "+",
                    Theme::BG_BUTTON,
                    Theme::TEXT_PRIMARY,
                    Theme::BORDER,
                    Some(UiAction::AdjustFontSize(1)),
                );
                y += 46;

                canvas.fill_rounded_rect(mx + 20, y, mw - 40, 38, 6, Theme::BG_CARD);
                canvas.draw_text(
                    mx + 32,
                    y + 10,
                    "Default Windows ConPTY Terminal Shell:",
                    FontRole::UiSmall,
                    Theme::TEXT_PRIMARY,
                );
                let sh_lbl = self.settings_draft.default_shell.to_uppercase();
                self.draw_button(
                    canvas,
                    UiRect::new(mx + mw - 134, y + 6, 102, 26),
                    &sh_lbl,
                    Theme::BG_BUTTON,
                    Theme::ACCENT_CYAN,
                    Theme::BORDER,
                    Some(UiAction::ToggleDefaultShell),
                );
            }
            SettingsTab::ToolAudit => {
                canvas.draw_text(
                    mx + 20,
                    y,
                    "Recent Autonomous Tool Executions (persisted in local SQLite agent.db):",
                    FontRole::UiSmall,
                    Theme::TEXT_SECONDARY,
                );
                y += 24;
                let max_rows = ((mh - 170) / 26) as usize;
                let audit_rows = self.tool_audit_log.clone();
                for rec in audit_rows.into_iter().take(max_rows) {
                    canvas.fill_rounded_rect(mx + 20, y, mw - 40, 23, 4, Theme::BG_CARD);
                    let st_col = if rec.status == "ok" {
                        Theme::ACCENT_GREEN
                    } else {
                        Theme::ACCENT_RED
                    };
                    canvas.draw_text(
                        mx + 28,
                        y + 3,
                        &rec.status.to_uppercase(),
                        FontRole::TerminalMono,
                        st_col,
                    );
                    canvas.draw_text(
                        mx + 74,
                        y + 3,
                        &rec.tool_name,
                        FontRole::TerminalMono,
                        Theme::ACCENT_CYAN,
                    );
                    canvas.draw_text_clipped(
                        mx + 210,
                        y + 3,
                        mw - 310,
                        &rec.result_summary,
                        FontRole::UiSmall,
                        Theme::TEXT_SECONDARY,
                    );
                    canvas.draw_text(
                        mx + mw - 84,
                        y + 3,
                        &format!("{}ms", rec.duration_ms),
                        FontRole::TerminalMono,
                        Theme::TEXT_MUTED,
                    );
                    y += 26;
                }
            }
        }

        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 176, my + mh - 42, 156, 30),
            "Save to SQLite",
            Theme::ACCENT_GREEN,
            0x04120B,
            Theme::ACCENT_GREEN,
            Some(UiAction::SaveSettingsModal),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_modal_input_row(
        &mut self,
        canvas: &mut dyn SurfaceCanvas,
        x: i32,
        y: i32,
        total_w: i32,
        label: &str,
        value: &str,
        secret: bool,
        target: FocusTarget,
    ) {
        canvas.draw_text(x, y + 5, label, FontRole::UiSmall, Theme::TEXT_SECONDARY);
        let inp = UiRect::new(x + 130, y, total_w - 130, 26);
        let focused = self.focus == target;
        canvas.fill_rounded_rect(inp.x, inp.y, inp.w, inp.h, 4, Theme::BG_INPUT);
        canvas.stroke_rounded_rect(
            inp.x,
            inp.y,
            inp.w,
            inp.h,
            4,
            if focused {
                Theme::BORDER_FOCUS
            } else {
                Theme::BORDER
            },
        );
        let masked = if secret && !focused && !value.is_empty() {
            let tail: String = value
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("****************{}", tail)
        } else if focused && self.blink_phase {
            format!("{}|", value)
        } else if value.is_empty() {
            "(click to type or Ctrl+V to paste)".to_string()
        } else {
            value.to_string()
        };
        canvas.draw_text_clipped(
            inp.x + 8,
            inp.y + 5,
            inp.w - 16,
            &masked,
            FontRole::TerminalMono,
            if value.is_empty() && !focused {
                Theme::TEXT_MUTED
            } else {
                Theme::TEXT_PRIMARY
            },
        );
        self.push_hit(inp, UiAction::Focus(target));
    }

    fn draw_approval_modal(&mut self, canvas: &mut dyn SurfaceCanvas, req: &PermissionPrompt) {
        let mw = 580.min(self.width - 40);
        let mh = 230;
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;

        canvas.fill_rounded_rect(mx, my, mw, mh, 8, Theme::BG_MODAL);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 8, Theme::ACCENT_AMBER);
        canvas.draw_text(
            mx + 20,
            my + 16,
            &req.title,
            FontRole::UiTitle,
            Theme::ACCENT_AMBER,
        );
        canvas.draw_text_clipped(
            mx + 20,
            my + 44,
            mw - 40,
            &req.detail,
            FontRole::UiSmall,
            Theme::TEXT_SECONDARY,
        );

        canvas.fill_rounded_rect(mx + 20, my + 70, mw - 40, 86, 6, Theme::BG_INPUT);
        for (i, line) in wrap_text(&req.command_or_path, ((mw - 60) / 8) as usize)
            .into_iter()
            .take(4)
            .enumerate()
        {
            canvas.draw_text_clipped(
                mx + 28,
                my + 76 + i as i32 * 18,
                mw - 56,
                &line,
                FontRole::TerminalMono,
                Theme::TEXT_PRIMARY,
            );
        }

        self.draw_button(
            canvas,
            UiRect::new(mx + 20, my + mh - 46, 130, 30),
            "Allow Once",
            Theme::ACCENT_GREEN,
            0x04120B,
            Theme::ACCENT_GREEN,
            Some(UiAction::ApprovePermission {
                approved: true,
                remember: false,
            }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 160, my + mh - 46, 170, 30),
            "Allow for Session",
            Theme::ACCENT_BLUE,
            0xFFFFFF,
            Theme::ACCENT_BLUE,
            Some(UiAction::ApprovePermission {
                approved: true,
                remember: true,
            }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 120, my + mh - 46, 100, 30),
            "Deny",
            Theme::ACCENT_RED,
            0xFFFFFF,
            Theme::ACCENT_RED,
            Some(UiAction::ApprovePermission {
                approved: false,
                remember: false,
            }),
        );
    }

    fn draw_delete_modal(&mut self, canvas: &mut dyn SurfaceCanvas, del: &DeleteConfirmState) {
        let mw = 460.min(self.width - 40);
        let mh = 165;
        let mx = (self.width - mw) / 2;
        let my = (self.height - mh) / 2;

        canvas.fill_rounded_rect(mx, my, mw, mh, 8, Theme::BG_MODAL);
        canvas.stroke_rounded_rect(mx, my, mw, mh, 8, Theme::ACCENT_RED);
        canvas.draw_text(
            mx + 20,
            my + 18,
            "Confirm File Deletion",
            FontRole::UiTitle,
            Theme::ACCENT_RED,
        );
        canvas.draw_text_clipped(
            mx + 20,
            my + 52,
            mw - 40,
            &format!("Permanently delete '{}'?", del.rel_path),
            FontRole::UiRegular,
            Theme::TEXT_PRIMARY,
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + 20, my + mh - 46, 160, 30),
            "Delete Permanently",
            Theme::ACCENT_RED,
            0xFFFFFF,
            Theme::ACCENT_RED,
            Some(UiAction::ConfirmFileDelete { confirmed: true }),
        );
        self.draw_button(
            canvas,
            UiRect::new(mx + mw - 120, my + mh - 46, 100, 30),
            "Cancel",
            Theme::BG_BUTTON,
            Theme::TEXT_PRIMARY,
            Theme::BORDER,
            Some(UiAction::ConfirmFileDelete { confirmed: false }),
        );
    }
}

impl DesktopAppHandler for StudioApp {
    fn on_event(&mut self, event: WindowEvent) -> CursorShape {
        match event {
            WindowEvent::Resize { width, height } => {
                self.width = width.max(640);
                self.height = height.max(420);
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
                        DragSplitter::LeftSidebar => {
                            self.left_w = x.clamp(190, (self.width - self.right_w - 320).max(200));
                        }
                        DragSplitter::RightAgent => {
                            self.right_w = (self.width - x)
                                .clamp(290, (self.width - self.left_w - 320).max(300));
                        }
                        DragSplitter::BottomTerminal => {
                            let bottom = self.height - 24;
                            self.term_h = (bottom - y).clamp(110, (self.height - 220).max(120));
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
                    if let (Some((row, col)), Some(tab)) =
                        (hit, self.tabs.get_mut(self.active_tab))
                    {
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

                if self.settings_modal.is_some()
                    || self.pending_approval.is_some()
                    || self.pending_delete.is_some()
                {
                    return self.compute_cursor();
                }

                let top = 44;
                let bottom = self.height - 24;
                if y >= top && y < bottom {
                    if (x - self.left_w).abs() <= 4 {
                        self.active_splitter = Some(DragSplitter::LeftSidebar);
                        return CursorShape::ResizeH;
                    }
                    if (x - (self.width - self.right_w)).abs() <= 4 {
                        self.active_splitter = Some(DragSplitter::RightAgent);
                        return CursorShape::ResizeH;
                    }
                    let term_top = bottom - self.term_h;
                    if x > self.left_w
                        && x < self.width - self.right_w
                        && (y - term_top).abs() <= 4
                    {
                        self.active_splitter = Some(DragSplitter::BottomTerminal);
                        return CursorShape::ResizeV;
                    }
                }

                if self.editor_rect.contains(x, y) {
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
                    if let (Some((row, col)), Some(tab)) =
                        (hit, self.tabs.get_mut(self.active_tab))
                    {
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
                if x < self.left_w && self.left_tab == LeftTab::Explorer {
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
                if self.settings_modal.is_none() {
                    let steps = (delta_lines as isize) * 3;
                    if x < self.left_w {
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
                                self.git_scroll =
                                    (self.git_scroll as isize - steps).max(0) as usize;
                            }
                        }
                    } else if x >= self.width - self.right_w {
                        self.agent_scroll_lines =
                            (self.agent_scroll_lines as isize + steps).max(0) as usize;
                    } else if self.term_grid_rect.contains(x, y) {
                        if let Some(term) = self.terminals.get_mut(self.active_term) {
                            let max_sb = term.screen.scrollback.len();
                            term.screen.scroll_offset = (term.screen.scroll_offset as isize
                                + steps)
                                .clamp(0, max_sb as isize)
                                as usize;
                        }
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
                                        *scroll_x =
                                            (*scroll_x as isize - steps * 2).max(0) as usize;
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
        self.width = canvas.width().max(640);
        self.height = canvas.height().max(420);
        self.hit_zones.clear();

        canvas.clear(Theme::BG_APP);

        let modal_open = self.settings_modal.is_some()
            || self.pending_approval.is_some()
            || self.pending_delete.is_some();

        let top = 44;
        let bottom = self.height - 24;
        let center_x = self.left_w;
        let center_w = (self.width - self.left_w - self.right_w).max(220);
        let editor_h = (bottom - top - self.term_h).max(120);

        self.draw_top_header(canvas, self.width);
        self.draw_left_sidebar(canvas, top, bottom);
        self.draw_center_editor_and_diff(canvas, center_x, top, center_w, editor_h);
        self.draw_bottom_terminal(canvas, center_x, top + editor_h, center_w, self.term_h);
        self.draw_right_agent_panel(canvas, top, bottom);
        self.draw_status_footer(canvas);

        if modal_open {
            self.hit_zones.clear();
            if let Some(req) = self.pending_approval.clone() {
                self.draw_approval_modal(canvas, &req);
            } else if let Some(del) = self.pending_delete.clone() {
                self.draw_delete_modal(canvas, &del);
            } else if let Some(tab) = self.settings_modal {
                self.draw_settings_modal(canvas, tab);
            }
        }
    }

    fn set_code_font_size(&self) -> i32 {
        self.settings.editor_font_size
    }
}

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

fn truncate_right(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let head: String = chars[..max_chars.saturating_sub(2)].iter().collect();
        format!("{}..", head)
    }
}

fn truncate_left(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let tail: String = chars[chars.len() - max_chars.saturating_sub(2)..]
            .iter()
            .collect();
        format!("..{}", tail)
    }
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
