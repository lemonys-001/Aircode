//! Pure-Rust Native Studio UI primitives, clean navigation state, hit zones, and modals.

pub mod commands;
pub mod shell;
pub mod studio;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub struct UiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl UiRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftTab {
    Explorer,
    Search,
    Git,
    Sessions,
    Extensions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterView {
    Editor,
    Browser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalDialog {
    None,
    CommandPalette,
    ConnectWizard,
    ThemePicker,
    SafetySettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusTarget {
    Editor,
    Terminal,
    AgentPrompt,
    SearchInput,
    NewEntryInput,
    CommitInput,
    FindInput,
    GotoLineInput,
    CommandPaletteInput,
    BrowserUrlInput,
    BrowserPageInput,
    ConnectKeyInput,
    ConnectModelInput,
    ConnectUrlInput,
    McpNameInput,
    McpTargetInput,
}

#[derive(Clone, Debug)]
pub enum UiAction {
    Focus(FocusTarget),
    PickFolder,
    SwitchDrive(String),
    ToggleRailTab(LeftTab),
    ToggleTerminalDrawer,
    ToggleAgentPane,
    SwitchCenterView(CenterView),
    RefreshWorkspace,
    StartCreateEntry { is_dir: bool },
    CancelCreateEntry,
    ConfirmCreateEntry,
    ToggleDir(String),
    OpenFile(PathBuf, String, String, bool),
    RequestDeleteFile(PathBuf, String),
    RevealFile(PathBuf),
    RunSearch,
    OpenSearchHit(PathBuf, String, usize),

    GitStageFile { path: String, currently_staged: bool },
    GitStageAll,
    GitCommit,
    GitCheckout(String),
    GitOpenDiff(String),

    SelectEditorTab(usize),
    CloseEditorTab(usize),
    SaveActiveTab,
    NewScratchTab,
    ToggleFindBar,
    FindNext,
    ToggleGotoLine,
    ConfirmGotoLine,

    SelectPtyTab(usize),
    ClosePtyTab(usize),
    SpawnPtyTab(&'static str),
    PtySendCtrlC,
    PtySyncCwd,
    PtyClearScreen,

    SetMode(&'static str),
    CycleEffort,
    SetEffort(&'static str),
    ToggleThinkingVisibility,
    ToggleMessageThinking(String),
    SendAgentPrompt,
    QuickPrompt(&'static str),
    EmergencyStop,

    NewSession,
    SelectSession(String),
    DeleteSession(String),
    ForkActiveSession,
    ForkFromMessage(usize),
    CompactActiveSession,
    ScrollToMessage(usize),
    OpenDiffPreview { title: String, diff: String },

    OpenModal(ModalDialog),
    CloseModal,
    ExecuteCommandId(String),

    // Guided /connect OpenCode-style Wizard actions
    SelectConnectProvider(&'static str),
    SelectConnectModel(String),
    SetOpenCodeAuthMode(&'static str),
    PasteKeyFromClipboard,
    OpenProviderKeyUrl(String, bool),
    ImportDetectedAuth,
    SpawnLocalOpenCodeServe,
    SaveConnectWizard,
    // OpenCode Zen free lane (OpenCode-client emulation, zero key)
    UseZenFreeLane,
    FetchZenFreeModels,

    // Theme Picker
    ApplyTheme(String),

    // Skills, Sub-Agents & MCP Hub
    ToggleSkill(String),
    OpenCustomizerGuide,
    ProbeAllMcpServers,
    AddQuickMcpServer,
    ToggleMcpServer(usize),

    // Safety / Preferences Modal
    ToggleSettingBool(&'static str),
    AdjustFontSize(i32),

    // Inbuilt Agentic Browser Actions
    BrowserBack,
    BrowserForward,
    BrowserReload,
    BrowserGoUrl,
    BrowserClickRef(usize),
    BrowserInterruptTakeover,
    BrowserHandBackToAi,
    BrowserToggleEngine,
    BrowserToggleDomInspector,
    BrowserPopOutExternal,

    ApprovePermission { approved: bool, remember: bool },
    ConfirmFileDelete { confirmed: bool },

    // ---- OpenCode-style shell -------------------------------------------
    /// Switch between the conversation view and the workspace IDE.
    ToggleShellView,
    /// Open the provider setup screen.
    ShowConnect,
    /// Select a session tab in the top strip.
    SelectSessionTab(usize),
    /// Expand / collapse a `+ Thought` / `+ Tool` block in the transcript.
    ToggleChatBlock(String),
    /// Expand / collapse a right-sidebar section.
    ToggleSidebarSection(String),
    /// Jump to a workspace file from the sidebar's modified-files list.
    OpenWorkspaceFile(String),
    /// Pick the highlighted row in the command palette.
    PaletteSelect(usize),
    /// Put the caret in the prompt.
    FocusPrompt,
    /// Scroll the transcript by `delta` rows (negative = up).
    ScrollChat(i32),
    /// Show / hide the right context sidebar.
    ToggleSidebar,
    /// Toggle visibility of `+ Tool` blocks in the transcript.
    ToggleToolBlocks,
    /// Cycle the active model (status-bar model box).
    CycleModel,
}

#[derive(Clone, Debug)]
pub struct HitZone {
    pub rect: UiRect,
    pub action: UiAction,
}
