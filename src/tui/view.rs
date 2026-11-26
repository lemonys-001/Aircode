//! Draws the OpenCode-style layout into a character grid.
//!
//! The layout mirrors the desktop shell so the two feel like the same product:
//! a session tab strip on top, the conversation in the middle with a context
//! rail on the right when there is room, and a status bar over the composer at
//! the bottom. The `/connect` screen and the command palette replace the
//! conversation area when they are open.

use crate::tui::screen::Screen;
use crate::ui::snapshot::{AppSnapshot, SnapshotTool};

/// Width reserved for the context rail when the terminal is wide enough.
const RAIL_W: usize = 34;

/// A styled run of text inside a line.
struct Seg {
    text: String,
    fg: u32,
    bold: bool,
}

impl Seg {
    fn new(text: impl Into<String>, fg: u32, bold: bool) -> Self {
        Self {
            text: text.into(),
            fg,
            bold,
        }
    }
}

/// One rendered row of the transcript.
struct Line {
    segs: Vec<Seg>,
}

impl Line {
    fn plain(text: impl Into<String>, fg: u32) -> Self {
        Self {
            segs: vec![Seg::new(text, fg, false)],
        }
    }

    fn push(&mut self, text: impl Into<String>, fg: u32, bold: bool) {
        self.segs.push(Seg::new(text, fg, bold));
    }
}

/// Scroll and focus state the view owns between frames.
#[derive(Default)]
pub struct ViewState {
    /// First transcript line shown.
    pub scroll: usize,
    /// Follow the tail while the agent streams.
    pub follow: bool,
    /// Set when the user scrolls up, cleared when they return to the bottom.
    pub user_scrolled: bool,
}

impl ViewState {
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
        self.user_scrolled = true;
    }

    pub fn scroll_down(&mut self, n: usize, max: usize) {
        self.scroll = (self.scroll + n).min(max);
        if self.scroll >= max {
            self.user_scrolled = false;
        }
    }

    pub fn page_up(&mut self, page: usize) {
        self.scroll_up(page);
    }

    pub fn page_down(&mut self, page: usize, max: usize) {
        self.scroll_down(page, max);
    }
}

/// Wraps `text` to `width` columns, preserving explicit newlines.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in line.split(' ') {
            let w: usize = word.chars().map(crate::tui::term::char_width).sum();
            if cur_w == 0 {
                cur = word.to_string();
                cur_w = w;
            } else if cur_w + 1 + w <= width {
                cur.push(' ');
                cur.push_str(word);
                cur_w += 1 + w;
            } else {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
                cur_w = w;
            }
        }
        if !cur.is_empty() || out.is_empty() {
            out.push(cur);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Builds the transcript's rendered lines at `width` columns.
fn build_lines(snap: &AppSnapshot, width: usize) -> Vec<Line> {
    let p = &snap.palette;
    let mut lines: Vec<Line> = Vec::new();

    for msg in snap.transcript() {
        let is_user = msg.role == "user";
        let head = if is_user {
            "you"
        } else if msg.model.is_empty() {
            "assistant"
        } else {
            msg.model.as_str()
        };

        // Turn header.
        let mut hdr = Line::plain(head, p.accent);
        hdr.push(
            if msg.created_at.is_empty() {
                String::new()
            } else {
                format!("  {}", msg.created_at)
            },
            p.text_muted,
            false,
        );
        lines.push(hdr);

        if is_user {
            for l in wrap(&msg.content, width.saturating_sub(2)) {
                let mut line = Line::plain("> ", p.success);
                line.push(l, p.text_primary, false);
                lines.push(line);
            }
        } else {
            for l in render_markdown(&msg.content, width, p) {
                lines.push(l);
            }
            if !msg.thinking.is_empty() {
                let mut t = Line::plain("+ Thought", p.accent_secondary);
                t.push(
                    format!("  ({} chars)", msg.thinking.chars().count()),
                    p.text_muted,
                    false,
                );
                lines.push(t);
            }
            for tool in &msg.tools {
                push_tool(&mut lines, tool, width, p);
            }
            let mut done = Line::plain("Done.", p.text_muted);
            done.push(
                format!(
                    "  Build · {} · {} · {}",
                    if msg.model.is_empty() { "model" } else { &msg.model },
                    if msg.provider.is_empty() {
                        "provider"
                    } else {
                        &msg.provider
                    },
                    if msg.mode.is_empty() { "chat" } else { &msg.mode }
                ),
                p.text_muted,
                false,
            );
            lines.push(done);
        }
        lines.push(Line::plain(String::new(), p.text_muted));
    }

    if lines.is_empty() {
        lines.push(Line::plain("No messages yet.", p.text_muted));
        lines.push(Line::plain(
            "Type a prompt below, or /connect to set up a provider.",
            p.text_muted,
        ));
    }
    lines
}

/// A deliberately small markdown pass — enough to make a transcript readable
/// without pulling a full parser into the terminal front-end.
fn render_markdown(text: &str, width: usize, p: &crate::themes::ThemePalette) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in text.split('\n') {
        let t = raw.trim_end_matches('\r');
        let trimmed = t.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            let mut l = Line::plain(String::new(), p.text_muted);
            l.push("  ── code ──", p.text_muted, false);
            out.push(l);
            continue;
        }
        if in_code {
            for l in wrap(t, width) {
                let mut line = Line::plain("  ", p.bg_editor);
                line.push(l, p.syn_string, false);
                out.push(line);
            }
            continue;
        }
        if trimmed.starts_with('#') {
            let body = trimmed.trim_start_matches('#').trim();
            let mut l = Line::plain(String::new(), p.accent);
            l.push(body.to_string(), p.accent, true);
            out.push(l);
            continue;
        }
        if trimmed.starts_with("+ ") || trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let body = &trimmed[2..];
            let mut l = Line::plain("  • ", p.accent_secondary);
            l.push(body.to_string(), p.text_primary, false);
            out.push(l);
            continue;
        }
        if trimmed.starts_with('+') && !trimmed.starts_with("++") {
            let mut l = Line::plain("  ", p.diff_add_bg);
            l.push(trimmed.to_string(), p.success, false);
            out.push(l);
            continue;
        }
        if trimmed.starts_with('-') && !trimmed.starts_with("--") {
            let mut l = Line::plain("  ", p.diff_del_bg);
            l.push(trimmed.to_string(), p.danger, false);
            out.push(l);
            continue;
        }
        for l in wrap(t, width) {
            let mut line = Line::plain(String::new(), p.text_primary);
            line.push(l, p.text_primary, false);
            out.push(line);
        }
    }
    if out.is_empty() {
        out.push(Line::plain(String::new(), p.text_primary));
    }
    out
}

/// One collapsible `+ Tool` row, with its diff underneath when present.
fn push_tool(
    lines: &mut Vec<Line>,
    tool: &SnapshotTool,
    width: usize,
    p: &crate::themes::ThemePalette,
) {
    let label = human_tool_name(&tool.tool_name);
    let mut l = Line::plain("  + ", p.accent_secondary);
    l.push(label.clone(), p.text_secondary, false);
    if !tool.file_path.is_empty() {
        l.push(format!(" {}", short_path(&tool.file_path)), p.text_muted, false);
    }
    if tool.duration_ms > 0 {
        l.push(format!("  {}ms", tool.duration_ms), p.text_muted, false);
    }
    lines.push(l);

    if !tool.summary.is_empty() {
        for s in wrap(&tool.summary, width.saturating_sub(6)) {
            let mut line = Line::plain("      ", p.bg_agent);
            line.push(s, p.text_muted, false);
            lines.push(line);
        }
    }
    for d in tool.diff_text.lines().take(24) {
        let t = d.trim_end_matches('\r');
        let color = if t.starts_with('+') {
            p.success
        } else if t.starts_with('-') {
            p.danger
        } else {
            p.text_muted
        };
        let mut line = Line::plain(String::new(), p.bg_editor);
        line.push(t.to_string(), color, false);
        lines.push(line);
    }
}

fn human_tool_name(name: &str) -> String {
    match name {
        "read_file" => "Read".to_string(),
        "write_file" => "Write".to_string(),
        "edit_file" => "Edit".to_string(),
        "execute_command" => "Bash".to_string(),
        "git_operation" => "Git".to_string(),
        "list_files" => "List".to_string(),
        "search" => "Search".to_string(),
        other => other.to_string(),
    }
}

fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    if parts.len() <= 3 {
        return path.to_string();
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_cost(c: f64) -> String {
    if c >= 1.0 {
        format!("${:.2}", c)
    } else {
        format!("${:.3}", c)
    }
}

/// Paints one `Line` at `(x, y)`, clipped to `width`.
fn paint_line(screen: &mut Screen, line: &Line, x: usize, y: usize, width: usize) {
    let mut cx = x;
    for seg in &line.segs {
        if cx >= x + width {
            break;
        }
        let room = (x + width) - cx;
        let mut used = 0;
        for ch in seg.text.chars() {
            let w = crate::tui::term::char_width(ch);
            if w == 0 || used + w > room {
                break;
            }
            screen.put(cx + used, y, ch, seg.fg, screen.cell(cx + used, y).bg, seg.bold);
            used += w;
        }
        cx += used;
    }
}

// ---------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------

/// Draws a whole frame. `state` carries scroll position between frames.
pub fn draw(screen: &mut Screen, snap: &AppSnapshot, state: &ViewState) {
    let (cols, rows) = (screen.cols, screen.rows);
    let p = &snap.palette;
    let bg = p.bg_app;

    // Row budget. The autocomplete row only exists while the composer has
    // matches, and the body has to stop above whatever occupies the bottom.
    let has_ac = !snap.slash_hits.is_empty() || !snap.file_hits.is_empty();
    let prompt_row = rows.saturating_sub(1);
    let ac_row = rows.saturating_sub(2);
    let hint_row = if has_ac {
        rows.saturating_sub(3)
    } else {
        rows.saturating_sub(2)
    };
    let status_row = rows.saturating_sub(3);
    let body_top = 2;
    let body_bottom = if has_ac {
        status_row.saturating_sub(2)
    } else {
        status_row
    };

    // ---- Header: session tab strip -----------------------------------------
    screen.fill(0, 0, cols, p.bg_header, p.bg_header, false);
    let title = if snap.active_title.is_empty() {
        "new session"
    } else {
        &snap.active_title
    };
    let mut x = 1;
    x += screen.text(x, 0, &format!("[{}]", title), p.text_primary, p.bg_header, true);
    x += 1;
    for s in snap.sessions.iter().take(4) {
        if s.id == snap.active_session_id {
            continue;
        }
        let label = format!("[{}]", shorten(&s.title, 18));
        if x + label.len() + 2 >= cols {
            break;
        }
        x += screen.text(x, 0, &label, p.text_muted, p.bg_header, false);
        x += 1;
    }
    let right = format!(
        "{}  Workspace  /connect ",
        if snap.agent_running {
            "● running"
        } else {
            "idle"
        }
    );
    if cols > right.len() + 2 {
        screen.text(
            cols - right.len() - 1,
            0,
            &right,
            p.text_muted,
            p.bg_header,
            false,
        );
    }
    // Hairline under the strip.
    screen.fill(0, 1, cols, p.border, p.border, false);

    // ---- Status bar ---------------------------------------------------------
    screen.fill(0, status_row, cols, p.bg_rail, p.bg_rail, false);
    let left = format!(
        " Tokens: {} ({}%)  Cost: {}  {}",
        fmt_tokens(snap.tokens),
        (snap.context_pct * 100.0).round() as i64,
        fmt_cost(snap.cost),
        if snap.status_line.is_empty() {
            "ready"
        } else {
            &snap.status_line
        }
    );
    screen.text(0, status_row, &left, p.text_secondary, p.bg_rail, false);
    let model = format!(" {} v{} ", snap.model_box(), snap.version);
    if cols > model.len() + 2 {
        let mx = cols - model.len() - 1;
        screen.fill(mx, status_row, model.len(), p.accent, p.accent, false);
        screen.text(mx, status_row, &model, on_accent(p), p.accent, true);
    }

    // ---- Body ---------------------------------------------------------------
    let body_h = body_bottom.saturating_sub(body_top);
    if body_h == 0 {
        return;
    }
    let rail_w = if cols >= 96 { RAIL_W } else { 0 };
    let main_w = cols - rail_w;

    if snap.connect.open {
        draw_connect(screen, snap, body_top, body_h, main_w);
    } else if snap.palette_open {
        draw_palette(screen, snap, body_top, body_h, main_w);
    } else {
        draw_transcript(screen, snap, state, body_top, body_h, main_w);
    }

    if rail_w > 0 {
        screen.fill(main_w, body_top, 1, p.border, p.border, false);
        draw_rail(
            screen,
            snap,
            body_top,
            body_h,
            main_w + 1,
            rail_w.saturating_sub(1),
        );
    }

    // ---- Autocomplete row ---------------------------------------------------
    // Directly above the prompt so Tab-completion is discoverable instead of a
    // promise in the hint line.
    if has_ac {
        screen.fill(0, ac_row, cols, p.bg_input, p.bg_input, false);
        let mut label = String::from("  ");
        if !snap.slash_hits.is_empty() {
            let (cmd, desc) = &snap.slash_hits[0];
            label.push_str(cmd);
            label.push(' ');
            label.push_str(desc);
            if snap.slash_hits.len() > 1 {
                label.push_str(&format!("   (+{} more)", snap.slash_hits.len() - 1));
            }
        } else {
            label.push('@');
            label.push_str(&snap.file_hits[0]);
            if snap.file_hits.len() > 1 {
                label.push_str(&format!("   (+{} more)", snap.file_hits.len() - 1));
            }
        }
        screen.text(
            0,
            ac_row,
            &shorten(&label, cols.saturating_sub(1)),
            p.accent_secondary,
            p.bg_input,
            true,
        );
    }

    // ---- Hint line ----------------------------------------------------------
    screen.text_line(
        0,
        hint_row,
        if has_ac {
            " tab to accept the highlighted completion · enter to send"
        } else {
            " enter send · shift+enter newline · tab complete · ↑↓ history · pgup/pgdn scroll · ctrl+c quit"
        },
        p.text_muted,
        bg,
        false,
    );

    // ---- Composer -----------------------------------------------------------
    screen.fill(0, prompt_row, cols, p.bg_input, p.bg_input, false);
    let shown = if snap.prompt_text.is_empty() {
        "Ask AirCode anything…  (/ for commands, @ for files)".to_string()
    } else {
        snap.prompt_text.replace('\n', "⏎ ")
    };
    let caret = if snap.agent_running { "▌" } else { ">" };
    let prefix = format!(" {} ", caret);
    screen.text(0, prompt_row, &prefix, p.accent, p.bg_input, true);
    screen.text(
        prefix.chars().count(),
        prompt_row,
        &shown,
        if snap.prompt_text.is_empty() {
            p.text_muted
        } else {
            p.text_primary
        },
        p.bg_input,
        false,
    );

    // A pending permission request must never be invisible.
    if let Some(req) = &snap.pending_approval {
        let row = body_top.min(body_bottom.saturating_sub(2));
        let msg = format!(" ⚠ {} — {} (y to allow, n to deny) ", req.title, req.command_or_path);
        if cols > msg.len() {
            screen.fill(0, row, cols, p.warning, p.warning, false);
            screen.text(0, row, &msg, on_accent(p), p.warning, true);
        }
    }
}


/// Text colour that stays legible on a filled accent surface.
fn on_accent(p: &crate::themes::ThemePalette) -> u32 {
    // The theme's own choice when it has one, otherwise derive from luminance.
    p.on_accent_text(p.accent)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn draw_transcript(
    screen: &mut Screen,
    snap: &AppSnapshot,
    state: &ViewState,
    top: usize,
    height: usize,
    width: usize,
) {
    let p = &snap.palette;
    let lines = build_lines(snap, width.saturating_sub(1).max(10));
    let max_scroll = lines.len().saturating_sub(height);
    let start = if state.user_scrolled {
        state.scroll.min(max_scroll)
    } else {
        max_scroll
    };
    for i in 0..height {
        let y = top + i;
        screen.fill(0, y, width, p.bg_app, p.bg_app, false);
        if let Some(line) = lines.get(start + i) {
            paint_line(screen, line, 0, y, width);
        }
    }
    // Scroll position indicator.
    if lines.len() > height {
        let frac = start as f32 / max_scroll.max(1) as f32;
        let y = top + (frac * (height.saturating_sub(1)) as f32) as usize;
        screen.text(width.saturating_sub(1), y, "▐", p.text_muted, p.bg_app, false);
    }
}

fn draw_rail(
    screen: &mut Screen,
    snap: &AppSnapshot,
    top: usize,
    height: usize,
    x: usize,
    width: usize,
) {
    let p = &snap.palette;
    for i in 0..height {
        screen.fill(x, top + i, width, p.bg_sidebar, p.bg_sidebar, false);
    }
    let mut y = top;
    let head = " Context ";
    screen.text(x, y, head, p.accent, p.bg_sidebar, true);
    y += 1;
    let ctx = format!(
        "  {} tokens  {}%",
        fmt_tokens(snap.tokens),
        (snap.context_pct * 100.0).round() as i64
    );
    screen.text(x, y, &ctx, p.text_secondary, p.bg_sidebar, false);
    y += 1;
    let cost = format!("  {}", fmt_cost(snap.cost));
    screen.text(x, y, &cost, p.text_secondary, p.bg_sidebar, false);
    y += 2;

    if y < top + height {
        screen.text(x, y, " MCP Servers ", p.accent, p.bg_sidebar, true);
        y += 1;
    }
    for (name, status) in snap.mcp_servers.iter().take(6) {
        if y >= top + height {
            break;
        }
        let dot = match status.as_str() {
            "connected" => "●",
            "failed" => "✕",
            "needs_auth" => "▲",
            _ => "○",
        };
        let color = match status.as_str() {
            "connected" => p.success,
            "failed" => p.danger,
            "needs_auth" => p.warning,
            _ => p.text_muted,
        };
        let label = format!("  {} {}", dot, shorten(name, width.saturating_sub(6)));
        screen.text(x, y, &label, color, p.bg_sidebar, false);
        y += 1;
    }
    if snap.mcp_servers.is_empty() {
        if y < top + height {
            screen.text(x, y, "  none configured", p.text_muted, p.bg_sidebar, false);
            y += 1;
        }
    }
    y += 1;

    if !snap.todos.is_empty() && y < top + height {
        screen.text(x, y, " Todos ", p.accent, p.bg_sidebar, true);
        y += 1;
        for t in snap.todos.iter().take(8) {
            if y >= top + height {
                break;
            }
            screen.text(
                x,
                y,
                &format!("  □ {}", shorten(t, width.saturating_sub(6))),
                p.text_secondary,
                p.bg_sidebar,
                false,
            );
            y += 1;
        }
        y += 1;
    }

    if !snap.modified_files.is_empty() && y < top + height {
        screen.text(x, y, " Modified Files ", p.accent, p.bg_sidebar, true);
        y += 1;
        for (path, add, del) in snap.modified_files.iter().take(10) {
            if y >= top + height {
                break;
            }
            let label = format!(
                "  {} +{} -{}",
                shorten(path, width.saturating_sub(14)),
                add,
                del
            );
            screen.text(x, y, &label, p.text_secondary, p.bg_sidebar, false);
            y += 1;
        }
    }
}

fn draw_connect(
    screen: &mut Screen,
    snap: &AppSnapshot,
    top: usize,
    height: usize,
    width: usize,
) {
    let p = &snap.palette;
    let c = &snap.connect;
    let rows: Vec<(String, u32, bool)> = vec![
        ("Connect a provider".to_string(), p.accent, true),
        (String::new(), p.text_muted, false),
        (
            "1) OpenCode Zen — free lane, no key, no account, no card".to_string(),
            p.text_primary,
            true,
        ),
        (
            format!(
                "   status: {}",
                if c.zen_status.is_empty() {
                    "not started"
                } else {
                    &c.zen_status
                }
            ),
            p.text_muted,
            false,
        ),
        (
            format!("   free models: {}", c.zen_models.len()),
            p.text_secondary,
            false,
        ),
        (String::new(), p.text_muted, false),
        (
            "2) Bring your own key (Anthropic, OpenAI, Ollama, …)".to_string(),
            p.text_primary,
            true,
        ),
        (
            format!("   provider: {}", if c.provider_id.is_empty() { "—" } else { &c.provider_id }),
            p.text_secondary,
            false,
        ),
        (
            format!("   model:    {}", if c.model.is_empty() { "—" } else { &c.model }),
            p.text_secondary,
            false,
        ),
        (
            format!("   key:      {}", mask(&c.key_input)),
            p.text_secondary,
            false,
        ),
        (
            format!("   base url: {}", if c.url_input.is_empty() { "—" } else { &c.url_input }),
            p.text_secondary,
            false,
        ),
        (String::new(), p.text_muted, false),
        (
            "3) Import detected local auth".to_string(),
            p.text_primary,
            true,
        ),
        (
            format!("   ollama: {}", if c.ollama_alive { "reachable" } else { "not found" }),
            if c.ollama_alive { p.success } else { p.text_muted },
            false,
        ),
        (
            format!(
                "   local server: {}",
                if c.local_server_alive { "reachable" } else { "not found" }
            ),
            if c.local_server_alive { p.success } else { p.text_muted },
            false,
        ),
        (String::new(), p.text_muted, false),
        (
            "esc to go back to the conversation".to_string(),
            p.text_muted,
            false,
        ),
    ];
    for (i, (text, fg, bold)) in rows.iter().enumerate() {
        if i >= height {
            break;
        }
        screen.fill(0, top + i, width, p.bg_app, p.bg_app, false);
        screen.text(1, top + i, text, *fg, p.bg_app, *bold);
    }
}

fn mask(s: &str) -> String {
    if s.is_empty() {
        return "—".to_string();
    }
    let n = s.chars().count();
    if n <= 4 {
        return "•".repeat(n);
    }
    format!("{}…{}", &s[..2], "•".repeat(n.saturating_sub(3)))
}

fn draw_palette(
    screen: &mut Screen,
    snap: &AppSnapshot,
    top: usize,
    height: usize,
    width: usize,
) {
    let p = &snap.palette;
    screen.fill(0, top, width, p.bg_modal, p.bg_modal, false);
    screen.text(
        1,
        top,
        "Command palette — type to filter, enter to run, esc to close",
        p.accent,
        p.bg_modal,
        true,
    );
    // The palette's live entries live in the app; the TUI shows the stable
    // command set so the screen is never empty.
    let cmds: &[(&str, &str)] = &[
        ("/connect", "provider setup"),
        ("/new", "fresh session"),
        ("/sessions", "browse sessions"),
        ("/model", "cycle model"),
        ("/models", "refresh free catalog"),
        ("/theme", "switch theme"),
        ("/compact", "compact context"),
        ("/fork", "fork this session"),
        ("/workspace", "toggle the IDE"),
        ("/help", "all commands"),
    ];
    let max = height.saturating_sub(3);
    for (i, (cmd, desc)) in cmds.iter().take(max).enumerate() {
        let y = top + 2 + i;
        if y >= top + height {
            break;
        }
        screen.fill(0, y, width, p.bg_modal, p.bg_modal, false);
        let mut line = Line::plain(String::new(), p.text_primary);
        line.push(format!("  {:<12}", cmd), p.accent, true);
        line.push(desc.to_string(), p.text_muted, false);
        paint_line(screen, &line, 0, y, width);
    }
}
