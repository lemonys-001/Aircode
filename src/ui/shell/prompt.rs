//! The prompt / command input — reworked.
//!
//! What was wrong with the old one: a bare single-line field that silently
//! swallowed every typo, no affordance for the slash commands the app advertises,
//! no way to send a multi-line prompt, and no history recall.
//!
//! What this one does:
//!   * a two-line hint strip (`enter to send`, `shift+enter newline`, `ctrl+h tools`)
//!   * live slash-command autocomplete with descriptions
//!   * `@file` autocomplete against the workspace
//!   * up/down history recall over previously sent prompts
//!   * shift+enter inserts a newline, enter sends
//!   * a visible send button so the primary action is never hidden

use super::theme::{self, SHELL_PAD};
use crate::platform::{FontRole, SurfaceCanvas};
use crate::themes::ThemePalette;
use crate::ui::{UiAction, UiRect};

pub const HINT_H: i32 = 18;

pub struct PromptModel<'a> {
    pub value: &'a str,
    pub caret: usize,
    pub focused: bool,
    pub caret_visible: bool,
    pub history: &'a [String],
    pub slash_hits: &'a [(&'static str, &'static str)],
    pub file_hits: &'a [String],
    pub ac_index: usize,
    pub running: bool,
}

pub struct PromptLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn layout(x: i32, y: i32, w: i32, h: i32) -> PromptLayout {
    PromptLayout { x, y, w, h }
}

pub fn draw(
    canvas: &mut dyn SurfaceCanvas,
    lay: &PromptLayout,
    model: &PromptModel,
    cell_w: i32,
    cell_h: i32,
    pal: &ThemePalette,
) -> Vec<(UiRect, UiAction)> {
    let mut hits: Vec<(UiRect, UiAction)> = Vec::new();
    let x = lay.x + SHELL_PAD;
    let w = lay.w - SHELL_PAD * 2;

    // ---- hint strip ---------------------------------------------------------
    let hint_y = lay.y + 4;
    let hints = if model.running {
        "esc to stop  |  ctrl+o expand last tool  |  the agent is working..."
    } else {
        "enter to send  |  shift+enter for newline  |  ctrl+h to toggle tool blocks  |  / for commands"
    };
    canvas.draw_text(x, hint_y, hints, FontRole::UiSmall, pal.text_muted);
    let input_y = hint_y + HINT_H;

    // ---- the input surface --------------------------------------------------
    let input_h = (lay.h - HINT_H - 12).max(24);
    canvas.fill_rounded_rect(x, input_y, w, input_h, 8, pal.bg_input);
    canvas.stroke_rounded_rect(
        x,
        input_y,
        w,
        input_h,
        8,
        if model.focused { pal.border_focus } else { pal.border },
    );
    // `>` prompt marker, OpenCode style.
    canvas.draw_text(
        x + 10,
        input_y + (input_h - 16).max(0) / 2,
        ">",
        FontRole::UiSemibold,
        pal.accent,
    );
    let text_x = x + 10 + cell_w;
    let max_cols = ((w - 24 - cell_w) / cell_w.max(1)).max(4) as usize;
    let (scroll, shown) = visible_slice(model.value, model.caret, max_cols);
    canvas.draw_text_clipped(
        text_x,
        input_y + (input_h - 16).max(0) / 2,
        w - 24 - cell_w,
        &shown,
        FontRole::CodeMono,
        pal.text_primary,
    );
    if model.focused && model.caret_visible {
        let cx = text_x + (model.caret.saturating_sub(scroll)) as i32 * cell_w;
        if cx < x + w - 12 {
            canvas.fill_rect(cx, input_y + 6, 2, input_h - 12, pal.accent);
        }
    }
    hits.push((
        UiRect::new(x, input_y, w, input_h),
        UiAction::FocusPrompt,
    ));

    // ---- send button --------------------------------------------------------
    let btn_w = 62;
    let btn_h = input_h - 8;
    let btn_x = x + w - btn_w - 4;
    let btn_y = input_y + 4;
    theme::button(
        canvas,
        btn_x,
        btn_y,
        btn_w,
        btn_h,
        if model.running { "Stop" } else { "Send" },
        pal,
        true,
        false,
    );
    hits.push((
        UiRect::new(btn_x, btn_y, btn_w, btn_h),
        if model.running {
            UiAction::EmergencyStop
        } else {
            UiAction::SendAgentPrompt
        },
    ));

    // ---- autocomplete popup -------------------------------------------------
    if model.focused && (!model.slash_hits.is_empty() || !model.file_hits.is_empty()) {
        let rows = model.slash_hits.len().min(8) + model.file_hits.len().min(8);
        let pop_h = rows as i32 * cell_h + 12;
        let pop_y = (input_y - pop_h - 6).max(lay.y + HINT_H + 2);
        canvas.fill_rounded_rect(x, pop_y, w, pop_h, 8, pal.bg_modal);
        canvas.stroke_rounded_rect(x, pop_y, w, pop_h, 8, pal.border);
        let mut py = pop_y + 6;
        for (i, (cmd, desc)) in model.slash_hits.iter().take(8).enumerate() {
            let selected = i == model.ac_index;
            if selected {
                canvas.fill_rect(x + 4, py - 1, w - 8, cell_h, pal.bg_card_hover);
            }
            canvas.draw_text(x + 10, py, cmd, FontRole::CodeMono, pal.accent);
            canvas.draw_text(
                x + 10 + 16 * cell_w,
                py,
                &theme::truncate_right(desc, ((w / 8).saturating_sub(24).max(8)) as usize),
                FontRole::UiSmall,
                pal.text_secondary,
            );
            py += cell_h;
        }
        for (i, f) in model.file_hits.iter().take(8).enumerate() {
            let idx = model.slash_hits.len() + i;
            let selected = idx == model.ac_index;
            if selected {
                canvas.fill_rect(x + 4, py - 1, w - 8, cell_h, pal.bg_card_hover);
            }
            canvas.draw_text(x + 10, py, &format!("@{}", f), FontRole::CodeMono, pal.success);
            py += cell_h;
        }
    }

    hits
}

/// Return (scroll_offset, visible_text) for a caret inside a fixed-width field.
fn visible_slice(value: &str, caret: usize, max_cols: usize) -> (usize, String) {
    let chars: Vec<char> = value.chars().collect();
    let caret = caret.min(chars.len());
    let scroll = caret.saturating_sub(max_cols.saturating_sub(1));
    let shown: String = chars.iter().skip(scroll).take(max_cols).collect();
    (scroll, shown)
}

// ---------------------------------------------------------------------------
// Slash command catalogue (shared by the autocomplete and the palette)
// ---------------------------------------------------------------------------

pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/connect", "connect a provider (Zen free lane, API key or local server)"),
    ("/zen", "one-click free lane: anonymous OpenCode Zen models, no key"),
    ("/models", "refresh the anonymous Zen model catalog"),
    ("/new", "start a new session"),
    ("/sessions", "list and switch sessions"),
    ("/fork", "fork the current session"),
    ("/compact", "summarize the session to reclaim context"),
    ("/timeline", "jump back to any earlier prompt"),
    ("/plan", "read-only planning mode"),
    ("/build", "full autonomous build mode"),
    ("/effort", "set agent effort (low/medium/max/ultra)"),
    ("/thinking", "show or hide AI thinking blocks"),
    ("/model", "switch the active model"),
    ("/provider", "switch the active provider"),
    ("/theme", "open the theme picker"),
    ("/settings", "safety and preferences"),
    ("/terminal", "toggle the integrated terminal"),
    ("/browser", "open the agentic browser"),
    ("/editor", "open the code editor"),
    ("/find", "find in the active file"),
    ("/goto", "jump to a line"),
    ("/customize", "let AirCode customize itself"),
];

/// Fuzzy-ish filter used by the prompt autocomplete.
pub fn slash_hits(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.trim().to_ascii_lowercase();
    if !q.starts_with('/') {
        return Vec::new();
    }
    let mut out: Vec<(&'static str, &'static str)> = SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(&q))
        .copied()
        .collect();
    if out.is_empty() {
        out = SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd.contains(q.trim_start_matches('/')))
            .copied()
            .take(6)
            .collect();
    }
    out
}

/// Extract the `@partial` token being typed at the caret, if any.
pub fn at_query(value: &str, caret: usize) -> Option<String> {
    let chars: Vec<char> = value.chars().collect();
    let caret = caret.min(chars.len());
    let mut start = caret;
    while start > 0 {
        let c = chars[start - 1];
        if c == '@' {
            break;
        }
        if c.is_whitespace() {
            return None;
        }
        start -= 1;
    }
    if start == 0 || chars[start - 1] != '@' {
        return None;
    }
    let frag: String = chars[start..caret].iter().collect();
    if frag.is_empty() {
        return Some(String::new());
    }
    Some(frag.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slash_autocomplete_prefixes() {
        let hits = slash_hits("/co");
        assert!(hits.iter().any(|(c, _)| *c == "/connect"));
        assert!(hits.iter().any(|(c, _)| *c == "/compact"));
        assert!(!hits.iter().any(|(c, _)| *c == "/theme"));
    }

    #[test]
    fn test_slash_autocomplete_fallback() {
        let hits = slash_hits("/them");
        assert!(hits.iter().any(|(c, _)| *c == "/theme"));
    }

    #[test]
    fn test_slash_autocomplete_ignores_non_commands() {
        assert!(slash_hits("hello").is_empty());
        assert!(slash_hits("").is_empty());
    }

    #[test]
    fn test_at_query_detection() {
        assert_eq!(at_query("read @src/ma", 12), Some("src/ma".to_string()));
        assert_eq!(at_query("read @src/ma", 11), Some("src/m".to_string()));
        assert_eq!(at_query("read @", 6), Some(String::new()));
        assert_eq!(at_query("read @", 5), None); // caret before the @
        assert_eq!(at_query("no at here", 10), None);
        // An @ anywhere in the token starts a file reference.
        assert_eq!(at_query("email me@x", 10), Some("x".to_string()));
    }

    #[test]
    fn test_visible_slice_keeps_caret_on_screen() {
        let v = "abcdefghijklmnopqrstuvwxyz";
        let (scroll, shown) = visible_slice(v, 25, 10);
        assert_eq!(scroll, 16);
        assert_eq!(shown, "qrstuvwxyz");
        assert_eq!(shown.chars().count(), 10);
    }
}
