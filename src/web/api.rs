//! JSON API for the browser front-end.
//!
//! Every endpoint is a thin translation between HTTP and the same `StudioApp`
//! the GUI and the TUI drive. Nothing here reimplements application logic — a
//! key press arrives as a key name and is handed to `handle_key_down` /
//! `handle_char_input` exactly as a canvas click would be.

use crate::json::{JsonValue, JsonValue as J};
use crate::platform::KeyCode;
use crate::ui::snapshot::{AppSnapshot, SnapshotMessage, SnapshotTool};
use crate::ui::studio::StudioApp;
use crate::web::http::{Request, Response};

/// Routes one request. Returns None when the path is unknown.
pub fn route(app: &mut StudioApp, req: &Request) -> Option<Response> {
    let p = req.path.as_str();
    match (req.method.as_str(), p) {
        // ---- static assets -------------------------------------------------
        ("GET", "/") => Some(Response::html(crate::web::page::INDEX_HTML)),
        ("GET", "/app.css") => Some(Response::asset(
            "text/css; charset=utf-8",
            crate::web::page::APP_CSS,
        )),
        ("GET", "/app.js") => Some(Response::asset(
            "application/javascript; charset=utf-8",
            crate::web::page::APP_JS,
        )),
        ("GET", "/favicon.svg") => Some(Response::asset(
            "image/svg+xml",
            crate::web::page::FAVICON_SVG,
        )),

        // ---- state ---------------------------------------------------------
        ("GET", "/api/state") => Some(Response::json(&encode_state(&app.snapshot()))),
        ("GET", "/api/sessions") => Some(Response::json(&encode_sessions(app))),
        ("GET", "/api/health") => Some(Response::json(&J::obj(vec![(
            "ok",
            J::Bool(true),
        )]))),

        // ---- input ---------------------------------------------------------
        ("POST", "/api/send") => Some(send(app, req)),
        ("POST", "/api/key") => Some(key(app, req)),
        ("POST", "/api/char") => Some(char_input(app, req)),
        ("POST", "/api/action") => Some(action(app, req)),
        ("POST", "/api/slash") => Some(slash(app, req)),

        // ---- sessions & providers -----------------------------------------
        ("POST", "/api/session/switch") => Some(switch_session(app, req)),
        ("POST", "/api/session/new") => Some(new_session(app)),
        ("POST", "/api/connect/zen") => Some(connect_zen(app)),
        ("POST", "/api/connect/save") => Some(connect_save(app, req)),
        ("POST", "/api/connect/close") => Some(close_connect(app)),
        ("POST", "/api/permission") => Some(permission(app, req)),
        ("POST", "/api/stop") => Some(stop(app)),

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

/// Serialises the whole snapshot, for both `/api/state` and SSE frames.
pub fn encode_state_json(snap: &AppSnapshot) -> String {
    encode_state(snap).to_json_string()
}

fn encode_state(snap: &AppSnapshot) -> JsonValue {
    let palette = &snap.palette;
    J::obj(vec![
        ("version", J::Str(snap.version.clone())),
        ("workspace", J::Str(snap.workspace.clone())),
        ("view", J::Str(snap.shell_view.clone())),
        ("paletteOpen", J::Bool(snap.palette_open)),
        ("sidebarOpen", J::Bool(snap.sidebar_open)),
        ("showToolBlocks", J::Bool(snap.show_tool_blocks)),
        (
            "activeSessionId",
            J::Str(snap.active_session_id.clone()),
        ),
        (
            "sessions",
            J::arr(snap.sessions.iter().map(encode_session_summary).collect()),
        ),
        ("activeTitle", J::Str(snap.active_title.clone())),
        ("messages", J::arr(snap.messages.iter().map(encode_message).collect())),
        (
            "streaming",
            J::obj(vec![
                ("thinking", J::Str(snap.streaming_thinking.clone())),
                ("text", J::Str(snap.streaming_text.clone())),
                (
                    "tools",
                    J::arr(snap.live_tools.iter().map(encode_tool).collect()),
                ),
            ]),
        ),
        ("agentRunning", J::Bool(snap.agent_running)),
        ("compacting", J::Bool(snap.compacting)),
        ("statusLine", J::Str(snap.status_line.clone())),
        ("prompt", J::Str(snap.prompt_text.clone())),
        (
            // Autocomplete, so the browser shows the same inline suggestion row
            // the TUI and the GUI composer do.
            "slashHits",
            J::arr(
                snap.slash_hits
                    .iter()
                    .map(|(c, d)| {
                        J::obj(vec![
                            ("cmd", J::Str(c.clone())),
                            ("desc", J::Str(d.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("fileHits", J::arr(snap.file_hits.iter().map(|f| J::Str(f.clone())).collect())),
        (
            "paletteItems",
            J::arr(
                snap.palette_items
                    .iter()
                    .map(|i| {
                        J::obj(vec![
                            ("id", J::Str(i.id.clone())),
                            ("slash", J::Str(i.slash.clone())),
                            ("title", J::Str(i.title.clone())),
                            ("subtitle", J::Str(i.subtitle.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("paletteQuery", J::Str(snap.palette_query.clone())),
        (
            "history",
            J::arr(
                snap.prompt_history
                    .iter()
                    .map(|h| J::Str(h.clone()))
                    .collect(),
            ),
        ),
        ("toast", J::Str(snap.toast.clone())),
        ("model", J::Str(snap.model.clone())),
        ("provider", J::Str(snap.provider.clone())),
        ("mode", J::Str(snap.mode.clone())),
        ("tokens", J::Number(snap.tokens as f64)),
        ("contextPct", J::Number(snap.context_pct as f64)),
        ("cost", J::Number(snap.cost)),
        (
            "mcp",
            J::arr(
                snap.mcp_servers
                    .iter()
                    .map(|(n, s)| {
                        J::obj(vec![("name", J::Str(n.clone())), ("status", J::Str(s.clone()))])
                    })
                    .collect(),
            ),
        ),
        (
            "todos",
            J::arr(snap.todos.iter().map(|t| J::Str(t.clone())).collect()),
        ),
        (
            "modified",
            J::arr(
                snap.modified_files
                    .iter()
                    .map(|(p, a, d)| {
                        J::obj(vec![
                            ("path", J::Str(p.clone())),
                            ("add", J::Number(*a as f64)),
                            ("del", J::Number(*d as f64)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "approval",
            match &snap.pending_approval {
                Some(a) => J::obj(vec![
                    ("id", J::Str(a.request_id.clone())),
                    ("kind", J::Str(a.kind.clone())),
                    ("title", J::Str(a.title.clone())),
                    ("detail", J::Str(a.detail.clone())),
                    ("target", J::Str(a.command_or_path.clone())),
                ]),
                None => J::Null,
            },
        ),
        (
            "connect",
            J::obj(vec![
                ("open", J::Bool(snap.connect.open)),
                ("provider", J::Str(snap.connect.provider_id.clone())),
                ("model", J::Str(snap.connect.model.clone())),
                ("key", J::Str(snap.connect.key_input.clone())),
                ("url", J::Str(snap.connect.url_input.clone())),
                ("authMode", J::Str(snap.connect.auth_mode.clone())),
                (
                    "detected",
                    J::arr(
                        snap.connect
                            .detected
                            .iter()
                            .map(|d| J::Str(d.clone()))
                            .collect(),
                    ),
                ),
                (
                    "detectedFile",
                    match &snap.connect.detected_file {
                        Some(f) => J::Str(f.clone()),
                        None => J::Null,
                    },
                ),
                ("localAlive", J::Bool(snap.connect.local_server_alive)),
                ("ollamaAlive", J::Bool(snap.connect.ollama_alive)),
                (
                    "zenModels",
                    J::arr(
                        snap.connect
                            .zen_models
                            .iter()
                            .map(|m| J::Str(m.clone()))
                            .collect(),
                    ),
                ),
                ("zenStatus", J::Str(snap.connect.zen_status.clone())),
            ]),
        ),
        (
            "theme",
            J::Object(
                [
                    ("bgApp", palette.bg_app),
                    ("bgHeader", palette.bg_header),
                    ("bgSidebar", palette.bg_sidebar),
                    ("bgInput", palette.bg_input),
                    ("bgAgent", palette.bg_agent),
                    ("border", palette.border),
                    ("textPrimary", palette.text_primary),
                    ("textSecondary", palette.text_secondary),
                    ("textMuted", palette.text_muted),
                    ("accent", palette.accent),
                    ("accentSecondary", palette.accent_secondary),
                    ("success", palette.success),
                    ("warning", palette.warning),
                    ("danger", palette.danger),
                    ("purple", palette.purple),
                    ("diffAddBg", palette.diff_add_bg),
                    ("diffDelBg", palette.diff_del_bg),
                ]
                .iter()
                .map(|(k, v)| ((*k).to_string(), J::Number(*v as f64)))
                .collect(),
            ),
        ),
    ])
}

fn encode_message(m: &SnapshotMessage) -> JsonValue {
    J::obj(vec![
        ("id", J::Str(m.id.clone())),
        ("role", J::Str(m.role.clone())),
        ("content", J::Str(m.content.clone())),
        ("thinking", J::Str(m.thinking.clone())),
        ("model", J::Str(m.model.clone())),
        ("provider", J::Str(m.provider.clone())),
        ("mode", J::Str(m.mode.clone())),
        ("createdAt", J::Str(m.created_at.clone())),
        ("tools", J::arr(m.tools.iter().map(encode_tool).collect())),
    ])
}

fn encode_tool(t: &SnapshotTool) -> JsonValue {
    J::obj(vec![
        ("id", J::Str(t.id.clone())),
        ("name", J::Str(t.tool_name.clone())),
        ("status", J::Str(t.status.clone())),
        ("summary", J::Str(t.summary.clone())),
        ("filePath", J::Str(t.file_path.clone())),
        ("diff", J::Str(t.diff_text.clone())),
        ("durationMs", J::Number(t.duration_ms as f64)),
    ])
}

fn encode_session_summary(s: &crate::sqlite::SessionSummary) -> JsonValue {
    J::obj(vec![
        ("id", J::Str(s.id.clone())),
        ("title", J::Str(s.title.clone())),
        ("projectPath", J::Str(s.project_path.clone())),
        ("mode", J::Str(s.mode.clone())),
        ("updatedAt", J::Str(s.updated_at.clone())),
        ("messageCount", J::Number(s.message_count as f64)),
    ])
}

fn encode_sessions(app: &StudioApp) -> JsonValue {
    let snap = app.snapshot();
    J::obj(vec![
        (
            "active",
            J::Str(snap.active_session_id.clone()),
        ),
        (
            "items",
            J::arr(snap.sessions.iter().map(encode_session_summary).collect()),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn body_json(req: &Request) -> Result<JsonValue, String> {
    if req.body.trim().is_empty() {
        return Ok(J::obj(vec![]));
    }
    J::parse(&req.body).map_err(|e| e)
}

fn send(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let prompt = body.string_or("prompt", "");
    if prompt.trim().is_empty() {
        return Response::bad_request("prompt is empty");
    }
    // Route through the same action the GUI's Send button and the TUI's Enter
    // key fire. Calling `launch_agent_prompt` directly skipped the slash-command
    // dispatch, so "/theme" was sent to the model as literal chat text.
    app.prompt_text_set(&prompt);
    app.execute_action(crate::ui::UiAction::SendAgentPrompt);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

/// Accepts a key by name: `{"key":"Enter"}` or `{"key":"char","ch":"a"}`.
fn key(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let name = body.string_or("key", "");
    let code = match name.to_ascii_lowercase().as_str() {
        "enter" => KeyCode::Enter,
        "escape" | "esc" => KeyCode::Escape,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "tab" => KeyCode::Tab,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        other => {
            // A single letter, e.g. "p" for ctrl+p.
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Letter(c),
                _ => return Response::bad_request(&format!("unknown key: {}", name)),
            }
        }
    };
    let ctrl = body.bool_or("ctrl", false);
    let shift = body.bool_or("shift", false);
    let mods = crate::platform::Modifiers { ctrl, shift, ..Default::default() };
    app.handle_key_down(code, mods);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn char_input(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let text = body.string_or("ch", "");
    let mut n = 0;
    for ch in text.chars() {
        app.handle_char_input(ch);
        n += 1;
    }
    if n == 0 {
        return Response::bad_request("ch is empty");
    }
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn action(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let id = body.string_or("action", "");
    // Direct shell actions first, then anything that is a command-palette id
    // ("cmd:connect", "cmd:compact", …) — the same table the GUI palette uses.
    let action = match crate::ui::action_from_id(&id) {
        Some(a) => a,
        None if id.starts_with("cmd:") => {
            crate::ui::UiAction::ExecuteCommandId(id)
        }
        None => return Response::bad_request(&format!("unknown action: {}", id)),
    };
    app.execute_action(action);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn slash(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let cmd = body.string_or("cmd", "");
    if cmd.trim().is_empty() {
        return Response::bad_request("cmd is empty");
    }
    let handled = app.execute_slash_command(cmd.trim());
    Response::json(&J::obj(vec![
        ("ok", J::Bool(true)),
        ("handled", J::Bool(handled)),
    ]))
}

fn switch_session(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let id = body.string_or("id", "");
    if id.is_empty() {
        return Response::bad_request("id is empty");
    }
    app.execute_action(crate::ui::UiAction::SelectSession(id));
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn new_session(app: &mut StudioApp) -> Response {
    app.start_new_session();
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

/// Starts the OpenCode Zen free lane — no key, no account, no card.
fn connect_zen(app: &mut StudioApp) -> Response {
    app.show_connect();
    app.execute_action(crate::ui::UiAction::UseZenFreeLane);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn connect_save(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    // `string_or` borrows its default, so read the fields out first.
    let provider = body.string_or("provider", &app.connect.provider_id).to_string();
    let model = body.string_or("model", &app.connect.model).to_string();
    let key = body.string_or("key", &app.connect.key_input).to_string();
    let url = body.string_or("url", &app.connect.url_input).to_string();
    app.connect.provider_id = provider;
    app.connect.model = model;
    app.connect.key_input = key;
    app.connect.url_input = url;
    app.execute_action(crate::ui::UiAction::SaveConnectWizard);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn close_connect(app: &mut StudioApp) -> Response {
    // `ToggleShellView` flips Chat <-> Workspace; from Connect we want Chat.
    app.execute_action(crate::ui::UiAction::ToggleShellView);
    if app.shell_view() != crate::ui::shell::ShellView::Chat {
        app.execute_action(crate::ui::UiAction::ToggleShellView);
    }
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn permission(app: &mut StudioApp, req: &Request) -> Response {
    let body = match body_json(req) {
        Ok(b) => b,
        Err(e) => return Response::bad_request(&e),
    };
    let allow = body.bool_or("allow", false);
    app.execute_action(crate::ui::UiAction::ApprovePermission {
        approved: allow,
        remember: false,
    });
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}

fn stop(app: &mut StudioApp) -> Response {
    app.execute_action(crate::ui::UiAction::EmergencyStop);
    Response::json(&J::obj(vec![("ok", J::Bool(true))]))
}
