//! A plain-data view of the running application.
//!
//! The native GUI renders through [`crate::platform::SurfaceCanvas`], which a
//! terminal or a browser cannot use. Rather than fork the app logic per
//! front-end, every front-end reads the same state through [`AppSnapshot`] and
//! writes through the same `StudioApp` methods the GUI uses. That keeps one
//! source of truth for sessions, the agent stream, the permission gate and the
//! provider wizard.

use std::path::PathBuf;

use crate::agent::PermissionPrompt;
use crate::sqlite::ToolExecutionRecord;

/// One assistant/user turn, flattened for a renderer that has no canvas.
#[derive(Clone, Debug, Default)]
pub struct SnapshotMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub model: String,
    pub provider: String,
    pub mode: String,
    pub created_at: String,
    pub tools: Vec<SnapshotTool>,
}

/// A tool call attached to a turn (or one currently streaming).
#[derive(Clone, Debug, Default)]
pub struct SnapshotTool {
    pub id: String,
    pub tool_name: String,
    pub status: String,
    pub summary: String,
    pub file_path: String,
    pub diff_text: String,
    pub duration_ms: u64,
}

impl SnapshotTool {
    pub fn from_record(rec: &ToolExecutionRecord) -> Self {
        Self {
            id: rec.id.clone(),
            tool_name: rec.tool_name.clone(),
            status: rec.status.clone(),
            summary: rec.result_summary.clone(),
            file_path: rec.file_path.clone(),
            diff_text: rec.diff_text.clone(),
            duration_ms: rec.duration_ms,
        }
    }
}

/// One command-palette row, flattened.
#[derive(Clone, Debug, Default)]
pub struct SnapshotPaletteItem {
    pub id: String,
    pub slash: String,
    pub title: String,
    pub subtitle: String,
    pub shortcut: String,
    pub is_file: bool,
}

/// The `/connect` wizard, flattened.
#[derive(Clone, Debug, Default)]
pub struct SnapshotConnect {
    /// True when the connect screen is what the user is looking at.
    pub open: bool,
    pub provider_id: String,
    pub model: String,
    pub key_input: String,
    pub url_input: String,
    /// `"zen"` or `"local-serve"`.
    pub auth_mode: String,
    pub detected: Vec<String>,
    pub detected_file: Option<String>,
    pub local_server_alive: bool,
    pub ollama_alive: bool,
    pub zen_models: Vec<String>,
    pub zen_status: String,
}

/// Everything a headless front-end needs to draw a frame and react to input.
#[derive(Clone, Debug)]
pub struct AppSnapshot {
    pub version: String,
    pub workspace: String,
    /// `"chat"` or `"workspace"` — which half of the shell is showing.
    pub shell_view: String,
    /// True while the command palette is capturing keys.
    pub palette_open: bool,

    pub sessions: Vec<crate::sqlite::SessionSummary>,
    pub active_session_id: String,
    pub active_title: String,
    pub messages: Vec<SnapshotMessage>,

    pub agent_running: bool,
    pub compacting: bool,
    pub status_line: String,
    pub streaming_thinking: String,
    pub streaming_text: String,
    pub live_tools: Vec<SnapshotTool>,

    pub prompt_text: String,
    pub prompt_history: Vec<String>,
    /// Slash commands matching what is typed, for an inline autocomplete row.
    pub slash_hits: Vec<(String, String)>,
    /// `@file` matches for the same reason.
    pub file_hits: Vec<String>,
    /// Command-palette rows matching the palette query.
    pub palette_items: Vec<SnapshotPaletteItem>,
    pub palette_query: String,
    pub toast: String,

    pub model: String,
    pub provider: String,
    pub mode: String,
    pub tokens: u64,
    /// Fraction of the context window already used, 0.0 - 1.0.
    pub context_pct: f32,
    pub cost: f64,

    pub mcp_servers: Vec<(String, String)>,
    pub todos: Vec<String>,
    pub modified_files: Vec<(String, usize, usize)>,

    /// A tool is waiting for approval; the front-end must surface it.
    pub pending_approval: Option<PermissionPrompt>,

    pub connect: SnapshotConnect,

    /// The active theme, so a terminal or browser front-end can match the
    /// desktop app's colours instead of inventing its own.
    pub palette: crate::themes::ThemePalette,

    pub sidebar_open: bool,
    pub expanded_blocks: Vec<String>,
    pub show_tool_blocks: bool,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            palette: crate::themes::ThemePalette::low_cortisol(),
            ..Self::empty()
        }
    }
}

impl AppSnapshot {
    fn empty() -> Self {
        // `palette` is filled in by the `Default` impl above; every other field
        // is its natural zero value.
        Self {
            version: String::new(),
            workspace: String::new(),
            shell_view: String::new(),
            palette_open: false,
            sessions: Vec::new(),
            active_session_id: String::new(),
            active_title: String::new(),
            messages: Vec::new(),
            agent_running: false,
            compacting: false,
            status_line: String::new(),
            streaming_thinking: String::new(),
            streaming_text: String::new(),
            live_tools: Vec::new(),
            prompt_text: String::new(),
            prompt_history: Vec::new(),
            slash_hits: Vec::new(),
            file_hits: Vec::new(),
            palette_items: Vec::new(),
            palette_query: String::new(),
            toast: String::new(),
            model: String::new(),
            provider: String::new(),
            mode: String::new(),
            tokens: 0,
            context_pct: 0.0,
            cost: 0.0,
            mcp_servers: Vec::new(),
            todos: Vec::new(),
            modified_files: Vec::new(),
            pending_approval: None,
            connect: SnapshotConnect::default(),
            palette: crate::themes::ThemePalette::low_cortisol(),
            sidebar_open: false,
            expanded_blocks: Vec::new(),
            show_tool_blocks: false,
        }
    }

    /// The transcript, newest last, including anything currently streaming.
    ///
    /// Front-ends should render this rather than poking at `streaming_*` so the
    /// "assistant is still typing" row looks identical in every front-end.
    pub fn transcript(&self) -> Vec<SnapshotMessage> {
        let mut out = self.messages.clone();
        if !self.streaming_text.is_empty() || !self.streaming_thinking.is_empty() {
            out.push(SnapshotMessage {
                id: "__streaming__".to_string(),
                role: "assistant".to_string(),
                content: self.streaming_text.clone(),
                thinking: self.streaming_thinking.clone(),
                model: self.model.clone(),
                provider: self.provider.clone(),
                mode: self.mode.clone(),
                created_at: String::new(),
                tools: self.live_tools.clone(),
            });
        }
        out
    }

    /// Short label for the model box in a status bar.
    pub fn model_box(&self) -> String {
        if self.model.is_empty() {
            "no model".to_string()
        } else {
            self.model.clone()
        }
    }
}

/// Where a front-end should send the user's next keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendTarget {
    /// The conversation composer.
    Chat,
    /// The `/connect` wizard's text fields.
    Connect,
    /// The command palette.
    Palette,
}

/// Convenience for tests and for the web front-end's JSON encoding.
pub fn workspace_label(path: &PathBuf) -> String {
    path.display().to_string()
}
