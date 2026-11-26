//! The conversation column — the OpenCode-style transcript.
//!
//! Each assistant turn renders as:
//!   * a turn header (`ModelName (12:04)`)
//!   * the user prompt prefixed with `>`
//!   * the assistant markdown
//!   * collapsible `+ Thought:` / `+ Tool` disclosure rows
//!   * inline green/red diffs
//!   * a `Done. <summary>` closer and a footer strip
//!
//! Rows are built once per frame by `build_rows()` and drawn by `draw()`, which
//! keeps scrolling, hit-testing and rendering in lock-step.

use super::markdown;
use super::theme::{self, CARD_RADIUS, SHELL_PAD};
use crate::platform::{FontRole, SurfaceCanvas};
use crate::sqlite::{SessionDetail, ToolExecutionRecord};
use crate::themes::ThemePalette;
use crate::ui::{UiAction, UiRect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Blank,
    TurnHeader,
    UserText,
    AssistantText,
    ToolHeader,
    ToolBody,
    ThoughtHeader,
    ThoughtBody,
    DiffAdd,
    DiffDel,
    DiffMeta,
    Closer,
    Footer,
    Rule,
}

#[derive(Clone, Debug)]
pub struct ChatRow {
    pub kind: RowKind,
    pub text: String,
    /// Index into the session's message list when this row starts a message.
    pub msg_idx: Option<usize>,
    /// Stable id of the collapsible block this row belongs to (if any).
    pub block_id: String,
    pub open: bool,
}

/// Everything the chat column needs from the app, passed in as one bundle so
/// the panel stays decoupled from `StudioApp`.
pub struct ChatModel<'a> {
    pub session: &'a SessionDetail,
    pub streaming_text: &'a str,
    pub streaming_thinking: &'a str,
    pub live_tools: &'a [ToolExecutionRecord],
    pub running: bool,
    pub status_line: &'a str,
    pub show_thinking: bool,
    pub expanded: &'a [String],
    pub active_model: &'a str,
    pub active_mode: &'a str,
    pub effort: &'a str,
}

pub fn build_rows(model: &ChatModel, cols: usize) -> Vec<ChatRow> {
    let cols = cols.max(20);
    let mut rows: Vec<ChatRow> = Vec::new();
    let expanded = |id: &str| model.expanded.iter().any(|e| e == id);

    for (idx, msg) in model.session.messages.iter().enumerate() {
        match msg.role.as_str() {
            "user" => {
                rows.push(ChatRow {
                    kind: RowKind::Blank,
                    text: String::new(),
                    msg_idx: Some(idx),
                    block_id: String::new(),
                    open: false,
                });
                rows.push(ChatRow {
                    kind: RowKind::TurnHeader,
                    text: format!("You ({})", short_time(&msg.created_at)),
                    msg_idx: Some(idx),
                    block_id: String::new(),
                    open: false,
                });
                for line in wrap_prompt(&msg.content, cols.saturating_sub(2)) {
                    rows.push(ChatRow {
                        kind: RowKind::UserText,
                        text: line,
                        msg_idx: Some(idx),
                        block_id: String::new(),
                        open: false,
                    });
                }
            }
            "assistant" => {
                rows.push(ChatRow {
                    kind: RowKind::TurnHeader,
                    text: format!(
                        "{} ({})",
                        if msg.model.is_empty() {
                            model.active_model
                        } else {
                            &msg.model
                        },
                        short_time(&msg.created_at)
                    ),
                    msg_idx: Some(idx),
                    block_id: String::new(),
                    open: false,
                });

                // Thinking first (collapsible), matching OpenCode's ordering.
                if !msg.thinking.trim().is_empty() {
                    let id = format!("think:{}", msg.id);
                    let open = model.show_thinking && expanded(&id);
                    rows.push(ChatRow {
                        kind: RowKind::ThoughtHeader,
                        text: format!(
                            "{} Thought: {}",
                            theme::disclosure(open),
                            first_line(&msg.thinking)
                        ),
                        msg_idx: Some(idx),
                        block_id: id.clone(),
                        open,
                    });
                    if open {
                        for line in wrap_body(&msg.thinking, cols.saturating_sub(4)) {
                            rows.push(ChatRow {
                                kind: RowKind::ThoughtBody,
                                text: line,
                                msg_idx: Some(idx),
                                block_id: id.clone(),
                                open,
                            });
                        }
                    }
                }

                for line in markdown::render_markdown(&msg.content, cols.saturating_sub(2)) {
                    rows.push(ChatRow {
                        kind: RowKind::AssistantText,
                        text: flatten(&line),
                        msg_idx: Some(idx),
                        block_id: String::new(),
                        open: false,
                    });
                }

                for (ti, tool) in msg.tools.iter().enumerate() {
                    let id = format!("tool:{}:{}", msg.id, ti);
                    let open = expanded(&id);
                    rows.push(ChatRow {
                        kind: RowKind::ToolHeader,
                        text: format!("{} {}", theme::disclosure(open), tool_summary(tool)),
                        msg_idx: Some(idx),
                        block_id: id.clone(),
                        open,
                    });
                    if open {
                        for line in wrap_body(&tool.result_summary, cols.saturating_sub(4)) {
                            rows.push(ChatRow {
                                kind: RowKind::ToolBody,
                                text: line,
                                msg_idx: Some(idx),
                                block_id: id.clone(),
                                open,
                            });
                        }
                        for line in diff_lines(&tool.diff_text, cols.saturating_sub(4)) {
                            rows.push(ChatRow {
                                kind: line_kind(&line),
                                text: line,
                                msg_idx: Some(idx),
                                block_id: id.clone(),
                                open,
                            });
                        }
                    }
                }

                rows.push(ChatRow {
                    kind: RowKind::Closer,
                    text: "Done.".to_string(),
                    msg_idx: Some(idx),
                    block_id: String::new(),
                    open: false,
                });
                rows.push(ChatRow {
                    kind: RowKind::Footer,
                    text: format!(
                        "Build - {} - {} - {}",
                        if msg.model.is_empty() {
                            model.active_model
                        } else {
                            &msg.model
                        },
                        if msg.provider.is_empty() {
                            "opencode"
                        } else {
                            &msg.provider
                        },
                        model.effort
                    ),
                    msg_idx: Some(idx),
                    block_id: String::new(),
                    open: false,
                });
            }
            _ => {}
        }
    }

    // ---- live streaming tail ------------------------------------------------
    if model.running {
        if !model.streaming_thinking.trim().is_empty() && model.show_thinking {
            let id = "live:think".to_string();
            let open = expanded(&id);
            rows.push(ChatRow {
                kind: RowKind::ThoughtHeader,
                text: format!(
                    "{} Thought: {}",
                    theme::disclosure(open),
                    first_line(model.streaming_thinking)
                ),
                msg_idx: None,
                block_id: id.clone(),
                open,
            });
            if open {
                for line in wrap_body(model.streaming_thinking, cols.saturating_sub(4)) {
                    rows.push(ChatRow {
                        kind: RowKind::ThoughtBody,
                        text: line,
                        msg_idx: None,
                        block_id: id.clone(),
                        open,
                    });
                }
            }
        }
        if !model.streaming_text.trim().is_empty() {
            rows.push(ChatRow {
                kind: RowKind::TurnHeader,
                text: format!("{} (working)", model.active_model),
                msg_idx: None,
                block_id: String::new(),
                open: false,
            });
            for line in markdown::render_markdown(model.streaming_text, cols.saturating_sub(2)) {
                rows.push(ChatRow {
                    kind: RowKind::AssistantText,
                    text: flatten(&line),
                    msg_idx: None,
                    block_id: String::new(),
                    open: false,
                });
            }
        }
        for (ti, tool) in model.live_tools.iter().enumerate() {
            let id = format!("live:tool:{}", ti);
            let open = expanded(&id);
            rows.push(ChatRow {
                kind: RowKind::ToolHeader,
                text: format!("{} {}", theme::disclosure(open), tool_summary(tool)),
                msg_idx: None,
                block_id: id.clone(),
                open,
            });
            if open {
                for line in wrap_body(&tool.result_summary, cols.saturating_sub(4)) {
                    rows.push(ChatRow {
                        kind: RowKind::ToolBody,
                        text: line,
                        msg_idx: None,
                        block_id: id.clone(),
                        open,
                    });
                }
                for line in diff_lines(&tool.diff_text, cols.saturating_sub(4)) {
                    rows.push(ChatRow {
                        kind: line_kind(&line),
                        text: line,
                        msg_idx: None,
                        block_id: id.clone(),
                        open,
                    });
                }
            }
        }
        if !model.status_line.trim().is_empty() {
            rows.push(ChatRow {
                kind: RowKind::ToolBody,
                text: format!("... {}", model.status_line),
                msg_idx: None,
                block_id: String::new(),
                open: false,
            });
        }
    }

    if rows.is_empty() {
        rows.push(ChatRow {
            kind: RowKind::Blank,
            text: String::new(),
            msg_idx: None,
            block_id: String::new(),
            open: false,
        });
    }
    rows
}

fn line_kind(line: &str) -> RowKind {
    let t = line.trim_start();
    if t.starts_with('+') {
        RowKind::DiffAdd
    } else if t.starts_with('-') {
        RowKind::DiffDel
    } else {
        RowKind::DiffMeta
    }
}

fn flatten(line: &markdown::MdLine) -> String {
    line.spans.iter().map(|s| s.text.clone()).collect()
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(72)
        .collect()
}

fn short_time(ts: &str) -> String {
    crate::sqlite::short_time_label(ts)
}

fn wrap_prompt(text: &str, cols: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    for line in theme::wrap_text(text.trim_end(), cols.max(10)) {
        out.push(format!("> {}", line));
    }
    out
}

fn wrap_body(text: &str, cols: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![String::new()];
    }
    theme::wrap_text(text.trim_end(), cols.max(10))
}

/// Render a unified diff as coloured rows (`+`/`-`/` ` prefixes kept).
fn diff_lines(diff: &str, cols: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in diff.lines() {
        if raw.starts_with("diff --git")
            || raw.starts_with("index ")
            || raw.starts_with("--- ")
            || raw.starts_with("+++ ")
            || raw.starts_with("@@")
        {
            continue;
        }
        for line in theme::wrap_text(raw, cols.max(10)) {
            out.push(line);
        }
        if out.len() > 400 {
            out.push("... diff truncated ...".to_string());
            break;
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn tool_summary(t: &ToolExecutionRecord) -> String {
    let target = if !t.file_path.is_empty() {
        theme::shorten_path(&t.file_path, 48)
    } else if !t.args_json.is_empty() {
        theme::truncate_right(&t.args_json.replace('\n', " "), 48)
    } else {
        String::new()
    };
    let dur = if t.duration_ms > 0 {
        format!(" ({}ms)", t.duration_ms)
    } else {
        String::new()
    };
    if target.is_empty() {
        format!("{}{}", t.tool_name, dur)
    } else {
        format!("{} {}{}", t.tool_name, target, dur)
    }
}

/// Layout of the conversation column for a given window size.
pub struct ChatLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub cols: usize,
    pub rows_visible: usize,
}

pub fn layout(x: i32, y: i32, w: i32, h: i32, cell_w: i32, cell_h: i32) -> ChatLayout {
    let cols = ((w - SHELL_PAD * 2) / cell_w.max(1)).max(20) as usize;
    let rows_visible = (h / cell_h.max(1)).max(1) as usize;
    ChatLayout {
        x,
        y,
        w,
        h,
        cols,
        rows_visible,
    }
}

/// Draw the transcript. Returns hit zones for interactive rows.
pub fn draw(
    canvas: &mut dyn SurfaceCanvas,
    lay: &ChatLayout,
    rows: &[ChatRow],
    scroll: usize,
    pal: &ThemePalette,
    cell_w: i32,
    cell_h: i32,
) -> Vec<(UiRect, UiAction)> {
    let mut hits: Vec<(UiRect, UiAction)> = Vec::new();
    canvas.fill_rect(lay.x, lay.y, lay.w, lay.h, pal.bg_app);

    let first = scroll.min(rows.len().saturating_sub(1));
    let last = (first + lay.rows_visible).min(rows.len());
    let top = lay.y + SHELL_PAD;

    for (vi, row) in rows[first..last].iter().enumerate() {
        let y = top + vi as i32 * cell_h;
        if y + cell_h > lay.y + lay.h {
            break;
        }
        let (fg, font, indent) = match row.kind {
            RowKind::Blank => (pal.bg_app, FontRole::UiRegular, 0usize),
            RowKind::TurnHeader => (pal.accent, FontRole::UiSemibold, 0),
            RowKind::UserText => (pal.text_primary, FontRole::CodeMono, 0),
            RowKind::AssistantText => (pal.text_primary, FontRole::CodeMono, 0),
            RowKind::ThoughtHeader | RowKind::ToolHeader => (pal.text_secondary, FontRole::CodeMono, 0),
            RowKind::ThoughtBody | RowKind::ToolBody => (pal.text_muted, FontRole::CodeMono, 4),
            RowKind::DiffAdd => (pal.success, FontRole::CodeMono, 4),
            RowKind::DiffDel => (pal.danger, FontRole::CodeMono, 4),
            RowKind::DiffMeta => (pal.text_muted, FontRole::CodeMono, 4),
            RowKind::Closer => (pal.success, FontRole::CodeMono, 0),
            RowKind::Footer => (pal.text_muted, FontRole::UiSmall, 0),
            RowKind::Rule => (pal.border_subtle, FontRole::UiRegular, 0),
        };

        if row.kind == RowKind::Rule {
            canvas.draw_hline(lay.x + SHELL_PAD, y + cell_h / 2, lay.w - SHELL_PAD * 2, pal.border_subtle);
            continue;
        }
        if row.kind == RowKind::Blank {
            continue;
        }

        // Diff rows get a full-width tinted background.
        if matches!(row.kind, RowKind::DiffAdd | RowKind::DiffDel) {
            canvas.fill_rect(
                lay.x + SHELL_PAD,
                y,
                lay.w - SHELL_PAD * 2,
                cell_h,
                if row.kind == RowKind::DiffAdd {
                    pal.diff_add_bg
                } else {
                    pal.diff_del_bg
                },
            );
        }
        // Tool / thought disclosure rows read as cards when open.
        if row.open && matches!(row.kind, RowKind::ToolHeader | RowKind::ThoughtHeader) {
            canvas.fill_rounded_rect(
                lay.x + SHELL_PAD,
                y,
                lay.w - SHELL_PAD * 2,
                cell_h,
                4,
                pal.bg_card,
            );
        }

        canvas.push_clip(lay.x + SHELL_PAD, lay.y, lay.w - SHELL_PAD * 2, lay.h);
        let px = lay.x + SHELL_PAD + indent as i32 * cell_w;
        let text_y = y + (cell_h - 16).max(2) / 2;
        canvas.draw_text_clipped(px, text_y, lay.w - SHELL_PAD * 2 - indent as i32 * cell_w, &row.text, font, fg);
        canvas.pop_clip();

        // Clicking a disclosure row toggles it.
        if !row.block_id.is_empty()
            && matches!(row.kind, RowKind::ToolHeader | RowKind::ThoughtHeader)
        {
            hits.push((
                UiRect::new(lay.x + SHELL_PAD, y, lay.w - SHELL_PAD * 2, cell_h),
                UiAction::ToggleChatBlock(row.block_id.clone()),
            ));
        }
    }

    // Empty-state hint
    if rows.len() <= 1 {
        let hint = "No messages yet. Type a prompt below and press Enter.";
        canvas.draw_text(
            lay.x + SHELL_PAD,
            lay.y + lay.h / 2,
            hint,
            FontRole::UiRegular,
            pal.text_muted,
        );
    }

    theme::scrollbar(
        canvas,
        lay.x + lay.w - 8,
        top,
        lay.h - SHELL_PAD * 2,
        rows.len(),
        lay.rows_visible,
        scroll,
        pal,
    );

    // A subtle rounded container so the column reads as a surface.
    let _ = CARD_RADIUS;
    hits
}
