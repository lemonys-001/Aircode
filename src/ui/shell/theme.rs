//! Shared drawing primitives for the OpenCode-style shell.
//!
//! Everything here is deliberately small and side-effect free: the shell renders
//! rows of text through a monospace grid, so most helpers are one-liners around
//! `SurfaceCanvas`. Keeping them in one module is what makes the whole shell
//! readable — a panel is ~20 lines of `text(...)` calls, not 300.

use crate::platform::FontRole;
use crate::platform::SurfaceCanvas;
use crate::themes::ThemePalette;

// ---------------------------------------------------------------------------
// Geometry (all in device pixels; text is laid out on a monospace grid)
// ---------------------------------------------------------------------------

pub const TABSTRIP_H: i32 = 34;
pub const STATUSBAR_H: i32 = 26;
pub const SIDEBAR_W: i32 = 336;
pub const SIDEBAR_MIN_W: i32 = 260;
pub const SHELL_PAD: i32 = 14;
pub const ROW_H: i32 = 20;
pub const CARD_RADIUS: i32 = 8;

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
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

pub fn lighten(rgb: u32, delta: u32) -> u32 {
    let r = ((rgb >> 16) & 0xFF).saturating_add(delta).min(255);
    let g = ((rgb >> 8) & 0xFF).saturating_add(delta).min(255);
    let b = (rgb & 0xFF).saturating_add(delta).min(255);
    (r << 16) | (g << 8) | b
}

pub fn darken(rgb: u32, delta: u32) -> u32 {
    let r = ((rgb >> 16) & 0xFF).saturating_sub(delta);
    let g = ((rgb >> 8) & 0xFF).saturating_sub(delta);
    let b = (rgb & 0xFF).saturating_sub(delta);
    (r << 16) | (g << 8) | b
}

/// Blend `fg` over `bg` with `alpha` (0..255).
pub fn alpha_blend(fg: u32, bg: u32, alpha: u32) -> u32 {
    let a = alpha.min(255) as f32 / 255.0;
    let inv = 1.0 - a;
    let r = (((fg >> 16) & 0xFF) as f32 * a + ((bg >> 16) & 0xFF) as f32 * inv).round() as u32;
    let g = (((fg >> 8) & 0xFF) as f32 * a + ((bg >> 8) & 0xFF) as f32 * inv).round() as u32;
    let b = ((fg & 0xFF) as f32 * a + (bg & 0xFF) as f32 * inv).round() as u32;
    (r << 16) | (g << 8) | b
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

pub fn truncate_right(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let keep = max_chars - 1;
    let mut out: String = s.chars().take(keep).collect();
    out.push('~');
    out
}

pub fn truncate_left(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let skip = count - (max_chars - 1);
    let mut out = String::from("~");
    out.extend(s.chars().skip(skip));
    out
}

pub fn shorten_path(p: &str, max_chars: usize) -> String {
    if p.chars().count() <= max_chars {
        return p.to_string();
    }
    let name = p.rsplit(['/', '\\']).next().unwrap_or(p);
    let parent: String = p
        .rsplit_once(['/', '\\'])
        .map(|(head, _)| {
            let parts: Vec<&str> = head.split(['/', '\\']).collect();
            if parts.len() <= 2 {
                parts.join("/")
            } else {
                format!("{}/...", parts[parts.len() - 2])
            }
        })
        .unwrap_or_default();
    let joined = if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    };
    truncate_left(&joined, max_chars.max(name.chars().count() + 1))
}

/// Greedy word wrap on a character grid.
pub fn wrap_text(text: &str, max_cols: usize) -> Vec<String> {
    if max_cols == 0 {
        return vec![String::new()];
    }
    let mut out: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_len = 0usize;
        for word in line.split(' ') {
            let wlen = word.chars().count();
            if cur_len == 0 {
                cur.push_str(word);
                cur_len = wlen;
            } else if cur_len + 1 + wlen <= max_cols {
                cur.push(' ');
                cur.push_str(word);
                cur_len += 1 + wlen;
            } else {
                out.push(std::mem::take(&mut cur));
                cur.push_str(word);
                cur_len = wlen;
            }
            // Hard-break words longer than the column budget.
            while cur_len > max_cols {
                let split_at = max_cols;
                let head: String = cur.chars().take(split_at).collect();
                let tail: String = cur.chars().skip(split_at).collect();
                out.push(head);
                cur = tail;
                cur_len = cur.chars().count();
            }
        }
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Compact human-readable token/cost formatting used across the shell.
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

pub fn fmt_cost(usd: f64) -> String {
    if usd >= 1.0 {
        format!("${:.2}", usd)
    } else if usd >= 0.01 {
        format!("${:.3}", usd)
    } else {
        format!("${:.4}", usd)
    }
}

// ---------------------------------------------------------------------------
// Canvas primitives
// ---------------------------------------------------------------------------

/// Draw one grid line of text at grid column `col`, row `row` (top-left based).
pub fn grid_text(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    cell_w: i32,
    cell_h: i32,
    col: usize,
    row: usize,
    text: &str,
    font: FontRole,
    rgb: u32,
) {
    let px = x + col as i32 * cell_w;
    let py = y + row as i32 * cell_h + (cell_h - 16).max(2) / 2;
    canvas.draw_text(px, py, text, font, rgb);
}

pub fn panel(canvas: &mut dyn SurfaceCanvas, x: i32, y: i32, w: i32, h: i32, pal: &ThemePalette) {
    canvas.fill_rect(x, y, w, h, pal.bg_app);
}

pub fn card(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pal: &ThemePalette,
    hovered: bool,
) {
    canvas.fill_rounded_rect(
        x,
        y,
        w,
        h,
        CARD_RADIUS,
        if hovered { pal.bg_card_hover } else { pal.bg_card },
    );
    canvas.stroke_rounded_rect(x, y, w, h, CARD_RADIUS, pal.border_subtle);
}

pub fn section_label(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    text: &str,
    pal: &ThemePalette,
) {
    canvas.draw_text(x, y, text, FontRole::UiSemibold, pal.accent_secondary);
}

pub fn pill(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    h: i32,
    text: &str,
    fg: u32,
    bg: u32,
) -> i32 {
    let w = canvas.measure_text_width(text, FontRole::UiSmall) + 16;
    canvas.fill_rounded_rect(x, y, w, h, h / 2, bg);
    canvas.draw_text(x + 8, y + (h - 14).max(0) / 2, text, FontRole::UiSmall, fg);
    w
}

pub fn button(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &str,
    pal: &ThemePalette,
    primary: bool,
    hovered: bool,
) {
    let bg = if primary {
        if hovered {
            lighten(pal.accent, 18)
        } else {
            pal.accent
        }
    } else if hovered {
        pal.bg_card_hover
    } else {
        pal.bg_button
    };
    let fg = if primary {
        pal.on_accent_text(pal.accent)
    } else {
        pal.text_primary
    };
    canvas.fill_rounded_rect(x, y, w, h, 6, bg);
    canvas.stroke_rounded_rect(
        x,
        y,
        w,
        h,
        6,
        if primary { lighten(pal.accent, 24) } else { pal.border },
    );
    let tw = canvas.measure_text_width(label, FontRole::UiSmall);
    canvas.draw_text(
        x + (w - tw).max(0) / 2,
        y + (h - 14).max(0) / 2,
        label,
        FontRole::UiSmall,
        fg,
    );
}

/// A single-line text field with a caret. Returns nothing; caller handles input.
pub fn text_field(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: &str,
    placeholder: &str,
    pal: &ThemePalette,
    focused: bool,
    caret_visible: bool,
    caret_col: usize,
) {
    canvas.fill_rounded_rect(x, y, w, h, 6, pal.bg_input);
    canvas.stroke_rounded_rect(
        x,
        y,
        w,
        h,
        6,
        if focused { pal.border_focus } else { pal.border },
    );
    let pad = 8;
    let show = if value.is_empty() { placeholder } else { value };
    let fg = if value.is_empty() { pal.text_muted } else { pal.text_primary };
    let (cell_w, _) = canvas.mono_cell_size(FontRole::CodeMono);
    let max_cols = ((w - pad * 2) / cell_w.max(1)).max(1) as usize;
    let scroll = caret_col.saturating_sub(max_cols.saturating_sub(1));
    let visible: String = show
        .chars()
        .skip(scroll)
        .take(max_cols)
        .collect();
    canvas.draw_text_clipped(x + pad, y + (h - 16).max(0) / 2, w - pad * 2, &visible, FontRole::CodeMono, fg);
    if focused && caret_visible {
        let cx = x + pad + (caret_col.saturating_sub(scroll)) as i32 * cell_w;
        if cx < x + w - pad {
            canvas.fill_rect(cx, y + 5, 2, h - 10, pal.accent);
        }
    }
}

/// Thin scrollbar track + thumb.
pub fn scrollbar(
    canvas: &mut dyn SurfaceCanvas,
    x: i32,
    y: i32,
    h: i32,
    total: usize,
    visible: usize,
    offset: usize,
    pal: &ThemePalette,
) {
    if total <= visible || h <= 8 {
        return;
    }
    canvas.fill_rect(x, y, 6, h, pal.bg_editor);
    let track = h as f32;
    let thumb = ((visible as f32 / total as f32) * track).max(24.0).min(track);
    let max_off = (total - visible) as f32;
    let frac = if max_off <= 0.0 {
        0.0
    } else {
        (offset as f32 / max_off).clamp(0.0, 1.0)
    };
    let ty = y + ((track - thumb) * frac) as i32;
    canvas.fill_rounded_rect(x, ty, 6, thumb as i32, 3, mix(pal.text_muted, pal.bg_editor, 0.35));
}

/// Clamp a scroll offset so the view stays inside `total` rows.
pub fn clamp_scroll(offset: &mut usize, total: usize, visible: usize) {
    if total <= visible {
        *offset = 0;
    } else {
        let max = total - visible;
        if *offset > max {
            *offset = max;
        }
    }
}

/// OpenCode-style collapsible disclosure row: `+` when closed, `-` when open.
pub fn disclosure(open: bool) -> &'static str {
    if open {
        "-"
    } else {
        "+"
    }
}

/// Status dot used by the MCP / LSP sidebar sections.
pub fn dot(canvas: &mut dyn SurfaceCanvas, x: i32, y: i32, r: i32, rgb: u32) {
    canvas.fill_rounded_rect(x, y, r * 2, r * 2, r, rgb);
}
