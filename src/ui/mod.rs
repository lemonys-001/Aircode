//! Pure-Rust Native Studio UI primitives, clean navigation state, hit zones, and modals.

pub mod commands;
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
    OpenProviderKeyUrl(&'static str, bool),
    ImportDetectedAuth,
    SpawnLocalOpenCodeServe,
    SaveConnectWizard,

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
}

#[derive(Clone, Debug)]
pub struct HitZone {
    pub rect: UiRect,
    pub action: UiAction,
}
