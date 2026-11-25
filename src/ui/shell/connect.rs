//! `/connect` — provider setup, rebuilt as a first-class screen.
//!
//! The old flow was a cramped modal that only appeared if you already knew the
//! slash command, and its primary action ("Save & connect") sat below a wall of
//! inputs. This version is a full screen with the free lane front and centre:
//!
//!   1. a big "Start free lane" card for OpenCode Zen (no key, no account),
//!   2. a BYO-key card with provider / model / key / URL fields,
//!   3. an "import detected local auth" card,
//!   4. live detection feedback so you always know what was found.
//!
//! Every control here is wired to a real action — nothing is decorative.

use super::theme::{self, SHELL_PAD};
use crate::platform::{FontRole, SurfaceCanvas};
use crate::themes::ThemePalette;
use crate::ui::{FocusTarget, UiAction, UiRect};

pub struct ConnectModel<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub key: &'a str,
    pub url: &'a str,
    pub auth_mode: &'a str,
    pub zen_models: &'a [String],
    pub zen_status: &'a str,
    pub detected: &'a [String],
    pub detected_file: &'a Option<String>,
    pub local_server_alive: bool,
    pub ollama_alive: bool,
    pub focus_key: bool,
    pub caret_visible: bool,
    pub scroll: usize,
}

pub struct ConnectLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub const PROVIDERS: &[(&str, &str)] = &[
    ("opencode", "OpenCode (Zen gateway)"),
    ("anthropic", "Anthropic (Claude)"),
    ("openai", "OpenAI (GPT)"),
    ("gemini", "Google Gemini"),
    ("openrouter", "OpenRouter"),
    ("ollama", "Ollama (local)"),
    ("custom", "Custom OpenAI-compatible"),
];

pub fn draw(
    canvas: &mut dyn SurfaceCanvas,
    lay: &ConnectLayout,
    model: &ConnectModel,
    cell_h: i32,
    cell_w: i32,
    pal: &ThemePalette,
) -> Vec<(UiRect, UiAction)> {
    let mut hits: Vec<(UiRect, UiAction)> = Vec::new();
    canvas.fill_rect(lay.x, lay.y, lay.w, lay.h, pal.bg_app);
    let x = lay.x + SHELL_PAD + 8;
    let w = lay.w - (SHELL_PAD + 8) * 2;
    let mut y = lay.y + SHELL_PAD;

    // ---- title --------------------------------------------------------------
    canvas.draw_text(x, y, "Connect a provider", FontRole::UiTitle, pal.text_primary);
    y += 26;
    canvas.draw_text(
        x,
        y,
        "Start free in one click, or bring your own key. Nothing leaves this machine until you send a prompt.",
        FontRole::UiSmall,
        pal.text_muted,
    );
    y += cell_h + 10;

    // ---- free lane card -----------------------------------------------------
    let card_h = 116;
    theme::card(canvas, x, y, w, card_h, pal, false);
    canvas.draw_text(
        x + 14,
        y + 12,
        "OpenCode Zen — free lane",
        FontRole::UiSemibold,
        pal.accent,
    );
    canvas.draw_text(
        x + 14,
        y + 32,
        &format!(
            "{} free models, no account, no card, no key. AirCode speaks the same client",
            model.zen_models.len()
        ),
        FontRole::UiSmall,
        pal.text_secondary,
    );
    canvas.draw_text(
        x + 14,
        y + 48,
        "dialect as the OpenCode CLI, so the gateway serves the anonymous public lane.",
        FontRole::UiSmall,
        pal.text_secondary,
    );
    let models_line: String = model
        .zen_models
        .iter()
        .take(6)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    canvas.draw_text(
        x + 14,
        y + 66,
        &theme::truncate_right(&models_line, ((w / cell_w.max(1)).max(20)) as usize),
        FontRole::CodeMono,
        pal.success,
    );
    let btn_w = 148;
    let btn_h = 32;
    theme::button(
        canvas,
        x + 14,
        y + card_h - btn_h - 12,
        btn_w,
        btn_h,
        "Start free lane",
        pal,
        true,
        false,
    );
    hits.push((
        UiRect::new(x + 14, y + card_h - btn_h - 12, btn_w, btn_h),
        UiAction::UseZenFreeLane,
    ));
    let btn2_x = x + 14 + btn_w + 10;
    theme::button(
        canvas,
        btn2_x,
        y + card_h - btn_h - 12,
        130,
        btn_h,
        "Refresh models",
        pal,
        false,
        false,
    );
    hits.push((
        UiRect::new(btn2_x, y + card_h - btn_h - 12, 130, btn_h),
        UiAction::FetchZenFreeModels,
    ));
    if !model.zen_status.is_empty() {
        canvas.draw_text(
            btn2_x + 140,
            y + card_h - btn_h - 12 + 9,
            &theme::truncate_right(model.zen_status, ((w / cell_w.max(1)).saturating_sub(46).max(10)) as usize),
            FontRole::UiSmall,
            pal.text_muted,
        );
    }
    y += card_h + 16;

    // ---- BYO key card -------------------------------------------------------
    let field_w = (w - 28).max(120);
    let byo_h = 236;
    theme::card(canvas, x, y, w, byo_h, pal, false);
    canvas.draw_text(
        x + 14,
        y + 12,
        "Bring your own key",
        FontRole::UiSemibold,
        pal.text_primary,
    );
    let mut fy = y + 36;

    // Provider chips
    canvas.draw_text(x + 14, fy, "Provider", FontRole::UiSmall, pal.text_muted);
    fy += 16;
    let mut cx = x + 14;
    for (id, label) in PROVIDERS.iter() {
        let selected = *id == model.provider;
        let cw = canvas.measure_text_width(label, FontRole::UiSmall) + 20;
        let cw = cw.min(field_w);
        canvas.fill_rounded_rect(
            cx,
            fy,
            cw,
            24,
            12,
            if selected { pal.accent } else { pal.bg_button },
        );
        canvas.draw_text(
            cx + 10,
            fy + 5,
            label,
            FontRole::UiSmall,
            if selected {
                pal.on_accent_text(pal.accent)
            } else {
                pal.text_secondary
            },
        );
        hits.push((
            UiRect::new(cx, fy, cw, 24),
            UiAction::SelectConnectProvider(id),
        ));
        cx += cw + 8;
        if cx > x + w - 60 {
            cx = x + 14;
            fy += 30;
        }
    }
    fy += 34;

    // Model
    canvas.draw_text(x + 14, fy, "Model", FontRole::UiSmall, pal.text_muted);
    fy += 16;
    canvas.fill_rounded_rect(x + 14, fy, field_w, 28, 6, pal.bg_input);
    canvas.stroke_rounded_rect(x + 14, fy, field_w, 28, 6, pal.border);
    canvas.draw_text_clipped(
        x + 22,
        fy + 6,
        field_w - 16,
        if model.model.is_empty() {
            "e.g. big-pickle"
        } else {
            model.model
        },
        FontRole::CodeMono,
        if model.model.is_empty() {
            pal.text_muted
        } else {
            pal.text_primary
        },
    );
    hits.push((
        UiRect::new(x + 14, fy, field_w, 28),
        UiAction::Focus(FocusTarget::ConnectModelInput),
    ));
    fy += 36;

    // API key
    canvas.draw_text(x + 14, fy, "API key", FontRole::UiSmall, pal.text_muted);
    fy += 16;
    theme::text_field(
        canvas,
        x + 14,
        fy,
        field_w,
        28,
        model.key,
        "paste your key (leave empty for the free lane)",
        pal,
        model.focus_key,
        model.caret_visible,
        model.key.chars().count(),
    );
    hits.push((
        UiRect::new(x + 14, fy, field_w, 28),
        UiAction::Focus(FocusTarget::ConnectKeyInput),
    ));
    fy += 36;

    // Base URL
    canvas.draw_text(x + 14, fy, "Base URL / server", FontRole::UiSmall, pal.text_muted);
    fy += 16;
    canvas.fill_rounded_rect(x + 14, fy, field_w, 28, 6, pal.bg_input);
    canvas.stroke_rounded_rect(x + 14, fy, field_w, 28, 6, pal.border);
    canvas.draw_text_clipped(
        x + 22,
        fy + 6,
        field_w - 16,
        model.url,
        FontRole::CodeMono,
        pal.text_secondary,
    );
    hits.push((
        UiRect::new(x + 14, fy, field_w, 28),
        UiAction::Focus(FocusTarget::ConnectUrlInput),
    ));
    fy += 36;

    // Actions
    theme::button(canvas, x + 14, fy, 150, 32, "Save & connect", pal, true, false);
    hits.push((
        UiRect::new(x + 14, fy, 150, 32),
        UiAction::SaveConnectWizard,
    ));
    theme::button(
        canvas,
        x + 14 + 160,
        fy,
        140,
        32,
        "Paste key",
        pal,
        false,
        false,
    );
    hits.push((
        UiRect::new(x + 14 + 160, fy, 140, 32),
        UiAction::PasteKeyFromClipboard,
    ));
    theme::button(
        canvas,
        x + 14 + 310,
        fy,
        150,
        32,
        "Get a key",
        pal,
        false,
        false,
    );
    hits.push((
        UiRect::new(x + 14 + 310, fy, 150, 32),
        UiAction::OpenProviderKeyUrl(model.provider.to_string(), false),
    ));
    y += byo_h + 16;

    // ---- detected auth card -------------------------------------------------
    let det_h = 92;
    theme::card(canvas, x, y, w, det_h, pal, false);
    canvas.draw_text(
        x + 14,
        y + 12,
        "Already have OpenCode installed?",
        FontRole::UiSemibold,
        pal.text_primary,
    );
    canvas.draw_text(
        x + 14,
        y + 32,
        "AirCode can import the credentials already on this machine.",
        FontRole::UiSmall,
        pal.text_secondary,
    );
    let file_line = model
        .detected_file
        .clone()
        .unwrap_or_else(|| "no auth.json found yet".to_string());
    canvas.draw_text(
        x + 14,
        y + 50,
        &theme::truncate_right(&file_line, ((w / cell_w.max(1)).max(20)) as usize),
        FontRole::CodeMono,
        pal.text_muted,
    );
    if model.local_server_alive {
        canvas.draw_text(
            x + 14,
            y + 68,
            "local opencode server detected — bridge mode available",
            FontRole::UiSmall,
            pal.success,
        );
    } else if model.ollama_alive {
        canvas.draw_text(
            x + 14,
            y + 68,
            "local Ollama detected",
            FontRole::UiSmall,
            pal.success,
        );
    }
    theme::button(
        canvas,
        x + w - 164,
        y + det_h - 44,
        150,
        32,
        "Import detected auth",
        pal,
        false,
        false,
    );
    hits.push((
        UiRect::new(x + w - 164, y + det_h - 44, 150, 32),
        UiAction::ImportDetectedAuth,
    ));
    y += det_h + 14;

    // ---- detected sources list ---------------------------------------------
    if !model.detected.is_empty() {
        canvas.draw_text(x, y, "Detected sources", FontRole::UiSemibold, pal.accent_secondary);
        y += cell_h;
        for src in model.detected.iter() {
            canvas.draw_text(
                x + 10,
                y,
                &theme::truncate_right(src, ((w / cell_w.max(1)).max(20)) as usize),
                FontRole::UiSmall,
                pal.text_secondary,
            );
            y += cell_h;
        }
    }

    // ---- footer -------------------------------------------------------------
    canvas.draw_hline(lay.x + SHELL_PAD, lay.y + lay.h - 34, lay.w - SHELL_PAD * 2, pal.border_subtle);
    canvas.draw_text(
        lay.x + SHELL_PAD,
        lay.y + lay.h - 26,
        "esc to go back to the conversation",
        FontRole::UiSmall,
        pal.text_muted,
    );

    let _ = model.scroll;
    hits
}
