//! Command palette (ctrl+P / ctrl+K), grouped the way OpenCode groups its own:
//! Session, Model, Agent, Provider, System.

use super::theme::{self, SHELL_PAD};
use crate::platform::{FontRole, SurfaceCanvas};
use crate::themes::ThemePalette;
use crate::ui::commands::{builtin_palette_commands, filter_palette_items, PaletteItem};
use crate::ui::{UiAction, UiRect};

pub struct PaletteModel<'a> {
    pub query: &'a str,
    pub index: usize,
    pub items: &'a [PaletteItem],
    pub caret_visible: bool,
}

pub struct PaletteLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Group label for a command id, mirroring OpenCode's palette sections.
pub fn group_of(id: &str) -> &'static str {
    if id.starts_with("cmd:new_session")
        || id.starts_with("cmd:fork")
        || id.starts_with("cmd:compact")
        || id.starts_with("cmd:timeline")
        || id.starts_with("panel:")
    {
        "Session"
    } else if id.starts_with("cmd:zen") || id.starts_with("cmd:connect") {
        "Provider"
    } else if id.starts_with("cmd:mode") || id.starts_with("cmd:effort") || id.starts_with("cmd:toggle_thinking") {
        "Agent"
    } else if id.starts_with("cmd:shell") {
        "View"
    } else if id.starts_with("theme") {
        "System"
    } else {
        "System"
    }
}

pub fn draw(
    canvas: &mut dyn SurfaceCanvas,
    lay: &PaletteLayout,
    model: &PaletteModel,
    cell_h: i32,
    pal: &ThemePalette,
) -> Vec<(UiRect, UiAction)> {
    let mut hits: Vec<(UiRect, UiAction)> = Vec::new();
    // Scrim
    canvas.fill_rect(lay.x, lay.y, lay.w, lay.h, theme::alpha_blend(0x000000, pal.bg_app, 120));

    let box_w = (lay.w * 3 / 5).max(420).min(lay.w - 40);
    let box_h = (lay.h * 3 / 4).max(240).min(lay.h - 40);
    let bx = lay.x + (lay.w - box_w) / 2;
    let by = lay.y + (lay.h - box_h) / 2;

    canvas.fill_rounded_rect(bx, by, box_w, box_h, 10, pal.bg_modal);
    canvas.stroke_rounded_rect(bx, by, box_w, box_h, 10, pal.border);

    let inner_x = bx + 12;
    let inner_w = box_w - 24;
    let mut y = by + 12;

    // ---- search field -------------------------------------------------------
    canvas.fill_rounded_rect(inner_x, y, inner_w, 30, 6, pal.bg_input);
    canvas.stroke_rounded_rect(inner_x, y, inner_w, 30, 6, pal.border_focus);
    canvas.draw_text(inner_x + 10, y + 7, ">", FontRole::UiSemibold, pal.accent);
    canvas.draw_text_clipped(
        inner_x + 10 + 12,
        y + 7,
        inner_w - 24,
        if model.query.is_empty() {
            "type a command or file name..."
        } else {
            model.query
        },
        FontRole::CodeMono,
        if model.query.is_empty() {
            pal.text_muted
        } else {
            pal.text_primary
        },
    );
    y += 38;

    // ---- results ------------------------------------------------------------
    canvas.push_clip(inner_x, y, inner_w, box_h - (y - by) - 12);
    let mut last_group = "";
    let mut row = 0usize;
    let mut shown = 0usize;
    for item in model.items.iter() {
        if shown > 40 {
            break;
        }
        let g = group_of(&item.id);
        if g != last_group {
            if !last_group.is_empty() {
                y += 6;
            }
            canvas.draw_text(inner_x + 4, y, g, FontRole::UiSemibold, pal.accent_secondary);
            y += cell_h;
            last_group = g;
        }
        let selected = row == model.index;
        if selected {
            canvas.fill_rounded_rect(inner_x, y - 2, inner_w, cell_h + 2, 4, pal.bg_card_hover);
        }
        let marker = if item.is_file { "@" } else { ">" };
        let color = if item.is_file { pal.success } else { pal.accent };
        canvas.draw_text(inner_x + 8, y, marker, FontRole::CodeMono, color);
        canvas.draw_text(
            inner_x + 8 + 12,
            y,
            &theme::truncate_right(&item.title, ((inner_w / 8).saturating_sub(20).max(10)) as usize),
            FontRole::UiSmall,
            pal.text_primary,
        );
        hits.push((
            UiRect::new(inner_x, y - 2, inner_w, cell_h + 2),
            UiAction::PaletteSelect(row),
        ));
        y += cell_h + 2;
        row += 1;
        shown += 1;
    }
    canvas.pop_clip();

    if model.items.is_empty() {
        canvas.draw_text(
            inner_x + 4,
            y + 8,
            "no matches",
            FontRole::UiSmall,
            pal.text_muted,
        );
    }

    // ---- footer -------------------------------------------------------------
    canvas.draw_text(
        inner_x,
        by + box_h - 22,
        "up/down move  |  enter run  |  esc close",
        FontRole::UiSmall,
        pal.text_muted,
    );

    let _ = SHELL_PAD;
    hits
}

/// Convenience: build the item list the palette should show.
pub fn items_for(
    query: &str,
    show_thinking: bool,
    mode: &str,
    effort: &str,
    theme: &str,
    files: &[(String, String)],
) -> Vec<PaletteItem> {
    let commands = builtin_palette_commands(show_thinking, mode, effort, theme);
    filter_palette_items(query, &commands, files)
}
