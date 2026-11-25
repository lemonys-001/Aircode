//! Dynamic Runtime Theme System for AirCode (`Tokyo Night`, `Catppuccin Mocha`, `One Dark Pro`,
//! `Rosé Pine`, `Gruvbox Dark`, `Alabaster Light`, plus live `.aircode/theme.json` customization).

use crate::json::JsonValue;
use crate::syntax::TokenKind;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ThemePalette {
    pub id: String,
    pub name: String,
    pub is_light: bool,

    // Core surfaces
    pub bg_app: u32,
    pub bg_rail: u32,
    pub bg_header: u32,
    pub bg_sidebar: u32,
    pub bg_editor: u32,
    pub bg_gutter: u32,
    pub bg_active_line: u32,
    pub bg_selection: u32,
    pub bg_terminal: u32,
    pub bg_agent: u32,
    pub bg_card: u32,
    pub bg_card_hover: u32,
    pub bg_input: u32,
    pub bg_modal: u32,
    pub bg_button: u32,

    // Borders
    pub border: u32,
    pub border_subtle: u32,
    pub border_focus: u32,

    // Typography
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_muted: u32,

    // Semantic accents
    pub accent: u32,
    pub accent_secondary: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
    pub purple: u32,

    // Diff backgrounds
    pub diff_add_bg: u32,
    pub diff_del_bg: u32,

    // Syntax highlighting tokens
    pub syn_keyword: u32,
    pub syn_type: u32,
    pub syn_string: u32,
    pub syn_number: u32,
    pub syn_comment: u32,
    pub syn_macro: u32,
    pub syn_func: u32,
    pub syn_operator: u32,
}

impl ThemePalette {
    pub fn builtin_list() -> &'static [(&'static str, &'static str)] {
        &[
            ("tokyo-night", "Tokyo Night"),
            ("catppuccin-mocha", "Catppuccin Mocha"),
            ("one-dark", "One Dark Pro"),
            ("rose-pine", "Rose Pine"),
            ("gruvbox-dark", "Gruvbox Dark"),
            ("alabaster-light", "Alabaster Daylight"),
            ("custom", "Custom (.aircode/theme.json)"),
        ]
    }

    pub fn by_id(id: &str, workspace: Option<&Path>) -> Self {
        match id {
            "catppuccin-mocha" => Self::catppuccin_mocha(),
            "one-dark" => Self::one_dark(),
            "rose-pine" => Self::rose_pine(),
            "gruvbox-dark" => Self::gruvbox_dark(),
            "alabaster-light" => Self::alabaster_light(),
            "custom" => {
                if let Some(ws) = workspace {
                    let custom_path = ws.join(".aircode").join("theme.json");
                    if let Ok(raw) = std::fs::read_to_string(&custom_path) {
                        if let Ok(json) = JsonValue::parse(&raw) {
                            return Self::from_custom_json(&json);
                        }
                    }
                }
                Self::tokyo_night()
            }
            _ => {
                // Also auto-apply .aircode/theme.json overrides if present and id == "custom"
                Self::tokyo_night()
            }
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            id: "tokyo-night".into(),
            name: "Tokyo Night".into(),
            is_light: false,
            bg_app: 0x16161E,
            bg_rail: 0x13131A,
            bg_header: 0x13131A,
            bg_sidebar: 0x16161E,
            bg_editor: 0x1A1B26,
            bg_gutter: 0x1A1B26,
            bg_active_line: 0x222436,
            bg_selection: 0x2D3F76,
            bg_terminal: 0x14151E,
            bg_agent: 0x16161E,
            bg_card: 0x202230,
            bg_card_hover: 0x282B3D,
            bg_input: 0x12131A,
            bg_modal: 0x1E202E,
            bg_button: 0x26293B,
            border: 0x292E42,
            border_subtle: 0x202332,
            border_focus: 0x7AA2F7,
            text_primary: 0xC0CAF5,
            text_secondary: 0x9AA5CE,
            text_muted: 0x565F89,
            accent: 0x7AA2F7,
            accent_secondary: 0x7DCFFF,
            success: 0x9ECE6A,
            warning: 0xE0AF68,
            danger: 0xF7768E,
            purple: 0xBB9AF7,
            diff_add_bg: 0x1C2E28,
            diff_del_bg: 0x331E26,
            syn_keyword: 0xBB9AF7,
            syn_type: 0x2AC3DE,
            syn_string: 0x9ECE6A,
            syn_number: 0xFF9E64,
            syn_comment: 0x565F89,
            syn_macro: 0x7DCFFF,
            syn_func: 0x7AA2F7,
            syn_operator: 0x89DDFF,
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Self {
            id: "catppuccin-mocha".into(),
            name: "Catppuccin Mocha".into(),
            is_light: false,
            bg_app: 0x181825,
            bg_rail: 0x11111B,
            bg_header: 0x11111B,
            bg_sidebar: 0x181825,
            bg_editor: 0x1E1E2E,
            bg_gutter: 0x1E1E2E,
            bg_active_line: 0x2A2B3C,
            bg_selection: 0x45475A,
            bg_terminal: 0x151521,
            bg_agent: 0x181825,
            bg_card: 0x242538,
            bg_card_hover: 0x313244,
            bg_input: 0x11111B,
            bg_modal: 0x242538,
            bg_button: 0x313244,
            border: 0x313244,
            border_subtle: 0x262738,
            border_focus: 0xCBA6F7,
            text_primary: 0xCDD6F4,
            text_secondary: 0xA6ADC8,
            text_muted: 0x6C7086,
            accent: 0x89B4FA,
            accent_secondary: 0x94E2D5,
            success: 0xA6E3A1,
            warning: 0xF9E2AF,
            danger: 0xF38BA8,
            purple: 0xCBA6F7,
            diff_add_bg: 0x1E3329,
            diff_del_bg: 0x38212B,
            syn_keyword: 0xCBA6F7,
            syn_type: 0xF9E2AF,
            syn_string: 0xA6E3A1,
            syn_number: 0xFAB387,
            syn_comment: 0x6C7086,
            syn_macro: 0x94E2D5,
            syn_func: 0x89B4FA,
            syn_operator: 0x89DCEB,
        }
    }

    pub fn one_dark() -> Self {
        Self {
            id: "one-dark".into(),
            name: "One Dark Pro".into(),
            is_light: false,
            bg_app: 0x21252B,
            bg_rail: 0x1B1E23,
            bg_header: 0x1B1E23,
            bg_sidebar: 0x21252B,
            bg_editor: 0x282C34,
            bg_gutter: 0x282C34,
            bg_active_line: 0x2C313C,
            bg_selection: 0x3E4451,
            bg_terminal: 0x1E2227,
            bg_agent: 0x21252B,
            bg_card: 0x2C313A,
            bg_card_hover: 0x353B45,
            bg_input: 0x1B1D23,
            bg_modal: 0x282C34,
            bg_button: 0x353B45,
            border: 0x333842,
            border_subtle: 0x2A2E36,
            border_focus: 0x61AFEF,
            text_primary: 0xABB2BF,
            text_secondary: 0x9198A5,
            text_muted: 0x5C6370,
            accent: 0x61AFEF,
            accent_secondary: 0x56B6C2,
            success: 0x98C379,
            warning: 0xE5C07B,
            danger: 0xE06C75,
            purple: 0xC678DD,
            diff_add_bg: 0x243629,
            diff_del_bg: 0x3A2428,
            syn_keyword: 0xC678DD,
            syn_type: 0xE5C07B,
            syn_string: 0x98C379,
            syn_number: 0xD19A66,
            syn_comment: 0x5C6370,
            syn_macro: 0x56B6C2,
            syn_func: 0x61AFEF,
            syn_operator: 0x56B6C2,
        }
    }

    pub fn rose_pine() -> Self {
        Self {
            id: "rose-pine".into(),
            name: "Rose Pine".into(),
            is_light: false,
            bg_app: 0x191724,
            bg_rail: 0x14121D,
            bg_header: 0x14121D,
            bg_sidebar: 0x1F1D2E,
            bg_editor: 0x191724,
            bg_gutter: 0x191724,
            bg_active_line: 0x26233A,
            bg_selection: 0x403D52,
            bg_terminal: 0x15131F,
            bg_agent: 0x1F1D2E,
            bg_card: 0x26233A,
            bg_card_hover: 0x312E47,
            bg_input: 0x14121D,
            bg_modal: 0x232035,
            bg_button: 0x2E2B44,
            border: 0x312E47,
            border_subtle: 0x242136,
            border_focus: 0xC4A7E7,
            text_primary: 0xE0DEF4,
            text_secondary: 0x908CAA,
            text_muted: 0x6E6A86,
            accent: 0x9CCFD8,
            accent_secondary: 0xEBBCBA,
            success: 0x31748F,
            warning: 0xF6C177,
            danger: 0xEB6F92,
            purple: 0xC4A7E7,
            diff_add_bg: 0x1D3138,
            diff_del_bg: 0x37202D,
            syn_keyword: 0x31748F,
            syn_type: 0x9CCFD8,
            syn_string: 0xF6C177,
            syn_number: 0xEBBCBA,
            syn_comment: 0x6E6A86,
            syn_macro: 0xC4A7E7,
            syn_func: 0xEBBCBA,
            syn_operator: 0x908CAA,
        }
    }

    pub fn gruvbox_dark() -> Self {
        Self {
            id: "gruvbox-dark".into(),
            name: "Gruvbox Dark".into(),
            is_light: false,
            bg_app: 0x1D2021,
            bg_rail: 0x181A1B,
            bg_header: 0x181A1B,
            bg_sidebar: 0x242424,
            bg_editor: 0x282828,
            bg_gutter: 0x282828,
            bg_active_line: 0x32302F,
            bg_selection: 0x504945,
            bg_terminal: 0x1D2021,
            bg_agent: 0x242424,
            bg_card: 0x32302F,
            bg_card_hover: 0x3C3836,
            bg_input: 0x1B1D1E,
            bg_modal: 0x282828,
            bg_button: 0x3C3836,
            border: 0x3C3836,
            border_subtle: 0x32302F,
            border_focus: 0xFABD2F,
            text_primary: 0xEBDBB2,
            text_secondary: 0xBDAE93,
            text_muted: 0x7C6F64,
            accent: 0x83A598,
            accent_secondary: 0x8EC07C,
            success: 0xB8BB26,
            warning: 0xFABD2F,
            danger: 0xFB4934,
            purple: 0xD3869B,
            diff_add_bg: 0x2D3320,
            diff_del_bg: 0x3C2220,
            syn_keyword: 0xFB4934,
            syn_type: 0xFABD2F,
            syn_string: 0xB8BB26,
            syn_number: 0xD3869B,
            syn_comment: 0x928374,
            syn_macro: 0x8EC07C,
            syn_func: 0xB8BB26,
            syn_operator: 0xFE8019,
        }
    }

    pub fn alabaster_light() -> Self {
        Self {
            id: "alabaster-light".into(),
            name: "Alabaster Daylight".into(),
            is_light: true,
            bg_app: 0xF3F4F6,
            bg_rail: 0xE5E7EB,
            bg_header: 0xEAECEF,
            bg_sidebar: 0xF7F8FA,
            bg_editor: 0xFFFFFF,
            bg_gutter: 0xF8FAFC,
            bg_active_line: 0xEFF6FF,
            bg_selection: 0xBFDBFE,
            bg_terminal: 0xF8FAFC,
            bg_agent: 0xF7F8FA,
            bg_card: 0xFFFFFF,
            bg_card_hover: 0xEEF2F6,
            bg_input: 0xFFFFFF,
            bg_modal: 0xFFFFFF,
            bg_button: 0xE5E7EB,
            border: 0xD1D5DB,
            border_subtle: 0xE5E7EB,
            border_focus: 0x2563EB,
            text_primary: 0x111827,
            text_secondary: 0x374151,
            text_muted: 0x6B7280,
            accent: 0x2563EB,
            accent_secondary: 0x0284C7,
            success: 0x059669,
            warning: 0xD97706,
            danger: 0xDC2626,
            purple: 0x7C3AED,
            diff_add_bg: 0xDCFCE7,
            diff_del_bg: 0xFEE2E2,
            syn_keyword: 0x7C3AED,
            syn_type: 0x0284C7,
            syn_string: 0x15803D,
            syn_number: 0xC2410C,
            syn_comment: 0x6B7280,
            syn_macro: 0x0369A1,
            syn_func: 0x1D4ED8,
            syn_operator: 0x4B5563,
        }
    }

    pub fn from_custom_json(v: &JsonValue) -> Self {
        let base_id = v.string_or("base", "tokyo-night");
        let mut pal = match base_id.as_str() {
            "catppuccin-mocha" => Self::catppuccin_mocha(),
            "one-dark" => Self::one_dark(),
            "rose-pine" => Self::rose_pine(),
            "gruvbox-dark" => Self::gruvbox_dark(),
            "alabaster-light" => Self::alabaster_light(),
            _ => Self::tokyo_night(),
        };
        pal.id = "custom".into();
        pal.name = v.string_or("name", "Custom Theme");
        let apply = |field: &mut u32, key: &str| {
            let s = v.string_or(key, "");
            if let Some(hex) = parse_hex_color(&s) {
                *field = hex;
            }
        };
        apply(&mut pal.bg_app, "bgApp");
        apply(&mut pal.bg_rail, "bgRail");
        apply(&mut pal.bg_header, "bgHeader");
        apply(&mut pal.bg_sidebar, "bgSidebar");
        apply(&mut pal.bg_editor, "bgEditor");
        apply(&mut pal.bg_terminal, "bgTerminal");
        apply(&mut pal.bg_agent, "bgAgent");
        apply(&mut pal.bg_card, "bgCard");
        apply(&mut pal.bg_input, "bgInput");
        apply(&mut pal.border, "border");
        apply(&mut pal.border_focus, "borderFocus");
        apply(&mut pal.text_primary, "textPrimary");
        apply(&mut pal.text_secondary, "textSecondary");
        apply(&mut pal.text_muted, "textMuted");
        apply(&mut pal.accent, "accent");
        apply(&mut pal.accent_secondary, "accentSecondary");
        apply(&mut pal.success, "success");
        apply(&mut pal.warning, "warning");
        apply(&mut pal.danger, "danger");
        pal
    }

    pub fn token_rgb(&self, kind: TokenKind) -> u32 {
        match kind {
            TokenKind::Plain => self.text_primary,
            TokenKind::Keyword => self.syn_keyword,
            TokenKind::TypeName => self.syn_type,
            TokenKind::StringLit => self.syn_string,
            TokenKind::NumberLit => self.syn_number,
            TokenKind::Comment => self.syn_comment,
            TokenKind::MacroAttr => self.syn_macro,
            TokenKind::Function => self.syn_func,
            TokenKind::Operator => self.syn_operator,
        }
    }

    pub fn on_accent_text(&self, bg: u32) -> u32 {
        let r = (bg >> 16) & 0xFF;
        let g = (bg >> 8) & 0xFF;
        let b = bg & 0xFF;
        let lum = (r * 299 + g * 587 + b * 114) / 1000;
        if lum > 150 {
            0x0F172A
        } else {
            0xFFFFFF
        }
    }
}

pub fn parse_hex_color(s: &str) -> Option<u32> {
    let clean = s.trim().trim_start_matches('#').trim_start_matches("0x");
    if clean.len() == 6 {
        u32::from_str_radix(clean, 16).ok()
    } else {
        None
    }
}
