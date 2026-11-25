//! Right-hand context sidebar — the OpenCode "context rail".
//!
//! Sections, top to bottom: session identity, Context (tokens / % / cost),
//! MCP servers, LSP servers, Todos, Modified files. Every section with more than
//! two entries is collapsible, exactly like the real thing.

use super::theme::{self, SHELL_PAD};
use crate::mcp::McpServerConfig;
use crate::platform::{FontRole, SurfaceCanvas};
use crate::sqlite::SessionDetail;
use crate::themes::ThemePalette;
use crate::ui::{UiAction, UiRect};

pub struct SidebarModel<'a> {
    pub session: &'a SessionDetail,
    pub tokens: u64,
    pub token_pct: f32,
    pub cost_usd: f64,
    pub mcp_servers: &'a [McpServerConfig],
    pub mcp_status: &'a [(String, String)],
    pub todos: &'a [String],
    pub modified: &'a [(String, usize, usize)],
    pub collapsed: &'a [String],
    pub version: &'a str,
}

pub struct SidebarLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn layout(x: i32, y: i32, w: i32, h: i32) -> SidebarLayout {
    SidebarLayout { x, y, w, h }
}

pub fn draw(
    canvas: &mut dyn SurfaceCanvas,
    lay: &SidebarLayout,
    model: &SidebarModel,
    cell_h: i32,
    pal: &ThemePalette,
) -> Vec<(UiRect, UiAction)> {
    let mut hits: Vec<(UiRect, UiAction)> = Vec::new();
    canvas.fill_rect(lay.x, lay.y, lay.w, lay.h, pal.bg_sidebar);
    canvas.draw_vline(lay.x, lay.y, lay.h, pal.border_subtle);

    let x = lay.x + SHELL_PAD;
    let mut y = lay.y + SHELL_PAD;
    let w = lay.w - SHELL_PAD * 2;

    // ---- session identity ---------------------------------------------------
    let title = if model.session.title.is_empty() {
        "Untitled session"
    } else {
        &model.session.title
    };
    for line in theme::wrap_text(title, ((w / 8).max(12)) as usize) {
        canvas.draw_text(x, y, &line, FontRole::UiSemibold, pal.text_primary);
        y += cell_h;
    }
    canvas.draw_text(
        x,
        y,
        &theme::shorten_path(&model.session.project_path, 40),
        FontRole::UiSmall,
        pal.text_muted,
    );
    y += cell_h + 4;
    theme::section_label(canvas, x, y, "Context", pal);
    y += cell_h;

    // ---- Context ------------------------------------------------------------
    let pct = (model.token_pct.clamp(0.0, 1.0) * 100.0) as i32;
    let bar_w = w;
    canvas.fill_rounded_rect(x, y, bar_w, 8, 4, pal.bg_editor);
    let fill = ((bar_w as f32) * model.token_pct.clamp(0.0, 1.0)) as i32;
    if fill > 0 {
        let color = if pct >= 85 {
            pal.danger
        } else if pct >= 60 {
            pal.warning
        } else {
            pal.accent
        };
        canvas.fill_rounded_rect(x, y, fill.min(bar_w), 8, 4, color);
    }
    y += 14;
    canvas.draw_text(
        x,
        y,
        &format!(
            "{} tokens ({}%)  {}",
            theme::fmt_tokens(model.tokens),
            pct,
            theme::fmt_cost(model.cost_usd)
        ),
        FontRole::UiSmall,
        pal.text_secondary,
    );
    y += cell_h + 6;

    // ---- MCP servers --------------------------------------------------------
    let mcp_open = !model.collapsed.iter().any(|c| c == "mcp");
    y = section(
        canvas,
        lay,
        &mut hits,
        x,
        y,
        w,
        "MCP Servers",
        model.mcp_servers.len(),
        mcp_open,
        pal,
    );
    if mcp_open {
        if model.mcp_servers.is_empty() {
            canvas.draw_text(x + 10, y, "no servers configured", FontRole::UiSmall, pal.text_muted);
            y += cell_h;
        }
        for srv in model.mcp_servers.iter() {
            let status = model
                .mcp_status
                .iter()
                .find(|(n, _)| n == &srv.name)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let (dot_color, label) = match status.as_str() {
                "connected" | "ok" | "ready" => (pal.success, "connected"),
                "failed" | "error" => (pal.danger, "failed"),
                "needs_auth" => (pal.warning, "needs auth"),
                "disabled" => (pal.text_muted, "disabled"),
                _ => (pal.text_muted, "unknown"),
            };
            theme::dot(canvas, x + 4, y + 6, 4, dot_color);
            canvas.draw_text(x + 16, y, &srv.name, FontRole::UiSmall, pal.text_primary);
            canvas.draw_text(x + w - 66, y, label, FontRole::UiSmall, dot_color);
            y += cell_h;
        }
    }
    y += 4;

    // ---- LSP servers --------------------------------------------------------
    let lsp_open = !model.collapsed.iter().any(|c| c == "lsp");
    y = section(canvas, lay, &mut hits, x, y, w, "LSP Servers", 1, lsp_open, pal);
    if lsp_open {
        theme::dot(canvas, x + 4, y + 6, 4, pal.success);
        canvas.draw_text(x + 16, y, "rust-analyzer", FontRole::UiSmall, pal.text_primary);
        canvas.draw_text(x + w - 66, y, "connected", FontRole::UiSmall, pal.success);
        y += cell_h;
    }
    y += 4;

    // ---- Todos --------------------------------------------------------------
    let todo_open = !model.collapsed.iter().any(|c| c == "todos");
    y = section(
        canvas,
        lay,
        &mut hits,
        x,
        y,
        w,
        "Todos",
        model.todos.len(),
        todo_open,
        pal,
    );
    if todo_open {
        if model.todos.is_empty() {
            canvas.draw_text(x + 10, y, "nothing tracked", FontRole::UiSmall, pal.text_muted);
            y += cell_h;
        }
        for t in model.todos.iter() {
            canvas.draw_text(x + 4, y, "[ ]", FontRole::CodeMono, pal.text_muted);
            let line = theme::truncate_right(t, ((w / 8).max(10)) as usize);
            canvas.draw_text(x + 34, y, &line, FontRole::UiSmall, pal.text_primary);
            y += cell_h;
        }
    }
    y += 4;

    // ---- Modified files -----------------------------------------------------
    let files_open = !model.collapsed.iter().any(|c| c == "files");
    y = section(
        canvas,
        lay,
        &mut hits,
        x,
        y,
        w,
        "Modified Files",
        model.modified.len(),
        files_open,
        pal,
    );
    if files_open {
        if model.modified.is_empty() {
            canvas.draw_text(x + 10, y, "workspace clean", FontRole::UiSmall, pal.text_muted);
            y += cell_h;
        }
        for (path, add, del) in model.modified.iter() {
            let name = theme::shorten_path(path, ((w / 8).saturating_sub(12).max(10)) as usize);
            canvas.draw_text(x + 4, y, &name, FontRole::UiSmall, pal.text_primary);
            let stats = format!("+{} -{}", add, del);
            let sw = canvas.measure_text_width(&stats, FontRole::UiSmall);
            canvas.draw_text(x + w - sw - 4, y, &stats, FontRole::UiSmall, pal.text_muted);
            hits.push((
                UiRect::new(x, y, w, cell_h),
                UiAction::OpenWorkspaceFile(path.clone()),
            ));
            y += cell_h;
        }
    }

    // ---- version footer -----------------------------------------------------
    canvas.draw_text(
        x,
        lay.y + lay.h - cell_h - 4,
        &format!("AirCode v{}", model.version),
        FontRole::UiSmall,
        pal.text_muted,
    );

    hits
}

#[allow(clippy::too_many_arguments)]
fn section(
    canvas: &mut dyn SurfaceCanvas,
    lay: &SidebarLayout,
    hits: &mut Vec<(UiRect, UiAction)>,
    x: i32,
    y: i32,
    w: i32,
    title: &str,
    count: usize,
    open: bool,
    pal: &ThemePalette,
) -> i32 {
    let row_h = 20;
    let rect = UiRect::new(x - 4, y - 2, w + 8, row_h);
    hits.push((
        rect,
        UiAction::ToggleSidebarSection(title.to_string()),
    ));
    let _ = lay;
    let marker = if count > 2 {
        theme::disclosure(open)
    } else {
        ""
    };
    let label = if marker.is_empty() {
        format!("{} ({})", title, count)
    } else {
        format!("{} {} ({})", marker, title, count)
    };
    canvas.draw_text(x, y, &label, FontRole::UiSemibold, pal.accent_secondary);
    y + row_h + 2
}
