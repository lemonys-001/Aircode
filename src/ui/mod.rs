//! Pure-Rust Native Studio UI theme, hit-testing primitives, focus state, and modals.

pub mod studio;

use std::path::PathBuf;

pub struct Theme;

impl Theme {
    pub const BG_APP: u32 = 0x090D16;
    pub const BG_TITLEBAR: u32 = 0x060911;
    pub const BG_SIDEBAR: u32 = 0x0C121E;
    pub const BG_EDITOR: u32 = 0x0E1524;
    pub const BG_EDITOR_GUTTER: u32 = 0x0B111D;
    pub const BG_ACTIVE_LINE: u32 = 0x152036;
    pub const BG_SELECTION: u32 = 0x1E3A5F;
    pub const BG_TERMINAL: u32 = 0x070B12;
    pub const BG_AGENT: u32 = 0x0B101B;
    pub const BG_CARD: u32 = 0x131C2E;
    pub const BG_CARD_ALT: u32 = 0x172238;
    pub const BG_INPUT: u32 = 0x080D16;
    pub const BG_MODAL: u32 = 0x101827;
    pub const BG_BUTTON: u32 = 0x1B2840;
    pub const BG_BUTTON_HOVER: u32 = 0x243554;

    pub const BORDER: u32 = 0x1E293B;
    pub const BORDER_FOCUS: u32 = 0x38BDF8;

    pub const TEXT_PRIMARY: u32 = 0xF1F5F9;
    pub const TEXT_SECONDARY: u32 = 0x94A3B8;
    pub const TEXT_MUTED: u32 = 0x64748B;

    pub const ACCENT_BLUE: u32 = 0x2563EB;
    pub const ACCENT_CYAN: u32 = 0x38BDF8;
    pub const ACCENT_GREEN: u32 = 0x10B981;
    pub const ACCENT_AMBER: u32 = 0xF59E0B;
    pub const ACCENT_RED: u32 = 0xEF4444;
    pub const ACCENT_PURPLE: u32 = 0xA855F7;

    pub const DIFF_ADD_BG: u32 = 0x0D281E;
    pub const DIFF_DEL_BG: u32 = 0x2D1218;
    pub const DIFF_MOD_BG: u32 = 0x272011;
}

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Providers,
    Permissions,
    ToolAudit,
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
    SettingsModel,
    SettingsOpenAiKey,
    SettingsAnthropicKey,
    SettingsGeminiKey,
    SettingsOpenRouterKey,
    SettingsOllamaUrl,
    SettingsCustomUrl,
    SettingsCustomKey,
}

#[derive(Clone, Debug)]
pub enum UiAction {
    Focus(FocusTarget),
    PickFolder,
    SwitchDrive(String),
    SelectLeftTab(LeftTab),
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

    SelectPtyTab(usize),
    ClosePtyTab(usize),
    SpawnPtyTab(&'static str),
    PtySendCtrlC,
    PtySyncCwd,
    PtyClearScreen,

    CycleProvider,
    CycleModel,
    SetMode(&'static str),
    SendAgentPrompt,
    QuickPrompt(&'static str),
    EmergencyStop,
    NewSession,
    SelectSession(String),
    DeleteSession(String),

    OpenSettings(SettingsTab),
    CloseModal,
    SwitchSettingsTab(SettingsTab),
    SelectSettingsProvider(&'static str),
    SelectPresetModel(String),
    ProbeOllama,
    ToggleSettingBool(&'static str),
    AdjustMaxIters(isize),
    AdjustFontSize(i32),
    ToggleDefaultShell,
    SaveSettingsModal,

    ApprovePermission { approved: bool, remember: bool },
    ConfirmFileDelete { confirmed: bool },
}

#[derive(Clone, Debug)]
pub struct HitZone {
    pub rect: UiRect,
    pub action: UiAction,
}
