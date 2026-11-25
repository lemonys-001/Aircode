//! Pure-Rust Multi-Provider Autonomous AI Coding Agent Engine (9 Tools + Permission Gate + Emergency Stop).

use crate::fs_ops::{
    compute_unified_diff, resolve_workspace_path, scan_project_tree, search_project_files,
};
use crate::git_ops::run_git;
use crate::json::JsonValue;
use crate::platform::http_request;
use crate::sqlite::{
    make_unique_id, now_timestamp_label, AppSettings, ChatMessage, SessionDetail, SqliteStore,
    ToolExecutionRecord,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug)]
pub struct PermissionPrompt {
    pub request_id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub command_or_path: String,
}

#[derive(Clone, Debug)]
pub enum AgentUiEvent {
    Status(String),
    ThinkingDelta(String),
    TextDelta(String),
    ToolStarted(ToolExecutionRecord),
    ToolFinished(ToolExecutionRecord),
    PermissionRequested(PermissionPrompt),
    WorkspaceMutated,
    Error(String),
    Done(Option<SessionDetail>),
}

struct GateDecision {
    resolved: bool,
    approved: bool,
}

pub struct PermissionGate {
    stop_flag: AtomicBool,
    pending: Mutex<HashMap<String, GateDecision>>,
    remembered_kinds: Mutex<HashSet<String>>,
    cv: Condvar,
}

impl PermissionGate {
    pub fn new() -> Self {
        Self {
            stop_flag: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
            remembered_kinds: Mutex::new(HashSet::new()),
            cv: Condvar::new(),
        }
    }

    pub fn reset_stop(&self) {
        self.stop_flag.store(false, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    pub fn trigger_emergency_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let mut map = self.pending.lock().unwrap();
        for dec in map.values_mut() {
            dec.resolved = true;
            dec.approved = false;
        }
        self.cv.notify_all();
    }

    pub fn resolve(&self, request_id: &str, kind: &str, approved: bool, remember_session: bool) {
        if approved && remember_session && !kind.is_empty() {
            self.remembered_kinds
                .lock()
                .unwrap()
                .insert(kind.to_string());
        }
        let mut map = self.pending.lock().unwrap();
        if let Some(entry) = map.get_mut(request_id) {
            entry.resolved = true;
            entry.approved = approved;
        }
        self.cv.notify_all();
    }

    pub fn request_approval(
        &self,
        tx: &Sender<AgentUiEvent>,
        kind: &str,
        title: &str,
        detail: &str,
        command_or_path: &str,
    ) -> bool {
        if self.is_stopped() {
            return false;
        }
        if self.remembered_kinds.lock().unwrap().contains(kind) {
            return true;
        }

        let req_id = make_unique_id("perm");
        {
            let mut map = self.pending.lock().unwrap();
            map.insert(
                req_id.clone(),
                GateDecision {
                    resolved: false,
                    approved: false,
                },
            );
        }

        let _ = tx.send(AgentUiEvent::PermissionRequested(PermissionPrompt {
            request_id: req_id.clone(),
            kind: kind.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            command_or_path: command_or_path.to_string(),
        }));

        let mut map = self.pending.lock().unwrap();
        while !self.is_stopped() {
            if let Some(d) = map.get(&req_id) {
                if d.resolved {
                    let ok = d.approved;
                    map.remove(&req_id);
                    return ok;
                }
            } else {
                return false;
            }
            map = self.cv.wait(map).unwrap();
        }
        map.remove(&req_id);
        false
    }
}

pub fn probe_ollama_models(base_url: &str) -> (bool, Vec<String>) {
    let clean = base_url.trim().trim_end_matches('/');
    let resp = http_request("GET", &format!("{}/api/tags", clean), &[], "", 4000);
    if resp.error.is_some() || resp.status != 200 {
        return (false, Vec::new());
    }
    let mut models = Vec::new();
    if let Ok(parsed) = JsonValue::parse(&resp.body) {
        if let Some(arr) = parsed.get("models").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                    models.push(name.to_string());
                }
            }
        }
    }
    (true, models)
}

fn tool_schemas_openai() -> JsonValue {
    JsonValue::parse(
        r#"[
      {
        "type": "function",
        "function": {
          "name": "read_file",
          "description": "Read a project file with optional 1-indexed start_line and end_line.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "start_line": { "type": "integer" },
              "end_line": { "type": "integer" }
            },
            "required": ["path"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "write_file",
          "description": "Create or overwrite a file in the project workspace.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "content": { "type": "string" }
            },
            "required": ["path", "content"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "edit_file",
          "description": "Surgically replace exact old_text with new_text in an existing file.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string" },
              "old_text": { "type": "string" },
              "new_text": { "type": "string" }
            },
            "required": ["path", "old_text", "new_text"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "search_project",
          "description": "Search across project files for symbols, strings, or definitions.",
          "parameters": {
            "type": "object",
            "properties": {
              "query": { "type": "string" },
              "case_sensitive": { "type": "boolean" }
            },
            "required": ["query"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "execute_command",
          "description": "Execute a shell command in the workspace and capture stdout/stderr.",
          "parameters": {
            "type": "object",
            "properties": {
              "command": { "type": "string" },
              "cwd": { "type": "string" }
            },
            "required": ["command"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "install_dependencies",
          "description": "Install packages via cargo, npm, pnpm, yarn, or pip.",
          "parameters": {
            "type": "object",
            "properties": {
              "manager": { "type": "string", "enum": ["cargo", "npm", "pnpm", "yarn", "pip"] },
              "packages": { "type": "array", "items": { "type": "string" } },
              "dev": { "type": "boolean" }
            },
            "required": ["manager"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "run_tests",
          "description": "Run the project's test suite and return pass/fail output.",
          "parameters": {
            "type": "object",
            "properties": {
              "command": { "type": "string" }
            }
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "diagnose_errors",
          "description": "Run compiler/typecheck diagnostics (cargo check, tsc --noEmit) on the workspace.",
          "parameters": {
            "type": "object",
            "properties": {
              "target": { "type": "string" }
            }
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "git_operation",
          "description": "Execute Git status, diff, log, branch, add, or commit.",
          "parameters": {
            "type": "object",
            "properties": {
              "subcommand": { "type": "string", "enum": ["status", "diff", "log", "branch", "add", "commit"] },
              "args": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["subcommand"]
          }
        }
      }
    ]"#,
    )
    .expect("valid schema json")
}

#[derive(Clone, Debug)]
struct NormalizedCall {
    id: String,
    name: String,
    args: JsonValue,
}

struct TurnReply {
    text: String,
    thinking: String,
    calls: Vec<NormalizedCall>,
}

fn run_shell_silent(cwd: &Path, command_str: &str) -> Result<(i32, String, String), String> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("powershell.exe");
        c.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            command_str,
        ]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command_str]);
        c
    };
    cmd.current_dir(cwd);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed spawning shell: {}", e))?;
    let code = out.status.code().unwrap_or(-1);
    let stdout: String = String::from_utf8_lossy(&out.stdout)
        .chars()
        .take(14_000)
        .collect();
    let stderr: String = String::from_utf8_lossy(&out.stderr)
        .chars()
        .take(14_000)
        .collect();
    Ok((code, stdout, stderr))
}

fn execute_tool(
    project_root: &Path,
    settings: &AppSettings,
    gate: &PermissionGate,
    tx: &Sender<AgentUiEvent>,
    session_id: &str,
    mode: &str,
    call: &NormalizedCall,
) -> (String, ToolExecutionRecord) {
    let started = Instant::now();
    let args_json = call.args.to_json_string();
    let name = call.name.as_str();

    let is_mut_file = matches!(name, "write_file" | "edit_file");
    let is_shell = matches!(name, "execute_command" | "install_dependencies" | "run_tests");
    let is_mut_git = name == "git_operation"
        && matches!(call.args.str_or("subcommand", ""), "add" | "commit");

    let finish = |status: &str, summary: String, diff: String, fpath: String| -> ToolExecutionRecord {
        ToolExecutionRecord {
            id: call.id.clone(),
            session_id: session_id.to_string(),
            tool_name: call.name.clone(),
            args_json: args_json.clone(),
            result_summary: summary.chars().take(180).collect(),
            status: status.to_string(),
            duration_ms: started.elapsed().as_millis() as u64,
            diff_text: diff,
            file_path: fpath,
            created_at: now_timestamp_label(),
        }
    };

    if mode == "plan" && (is_mut_file || is_shell || is_mut_git) {
        let msg = format!(
            "Blocked in Plan Mode: '{}' cannot mutate files or execute commands. Produce a read-only architectural plan.",
            name
        );
        return (msg.clone(), finish("denied", msg, String::new(), String::new()));
    }

    if mode == "ask" && (is_mut_file || is_shell || is_mut_git) {
        let ok = gate.request_approval(
            tx,
            if is_shell { "shell" } else { "file_write" },
            &format!("Ask Mode Confirmation: {}", name),
            "Ask Mode requires explicit user approval before modifying files or running commands.",
            &args_json,
        );
        if !ok {
            let msg = format!("User denied execution of '{}'.", name);
            return (msg.clone(), finish("denied", msg, String::new(), String::new()));
        }
    } else if is_shell && settings.require_shell_approval && name == "execute_command" {
        let cmd_preview = call.args.str_or("command", "");
        let ok = gate.request_approval(
            tx,
            "shell",
            "Approve Shell Command Execution",
            "Autonomous agent requested shell execution inside your project workspace.",
            cmd_preview,
        );
        if !ok {
            let msg = "User denied shell command execution.".to_string();
            return (msg.clone(), finish("denied", msg, String::new(), String::new()));
        }
    }

    match name {
        "read_file" => {
            let p = call.args.str_or("path", "");
            match resolve_workspace_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => match std::fs::read_to_string(&resolved) {
                    Ok(content) => {
                        let lines: Vec<&str> = content.lines().collect();
                        let s = call.args.i64_or("start_line", 1).max(1) as usize;
                        let e = call
                            .args
                            .get("end_line")
                            .and_then(|v| v.as_i64())
                            .map(|v| v.max(1) as usize)
                            .unwrap_or_else(|| lines.len().min(s + 320));
                        let mut out = format!("File: {} ({} lines)\n", p, lines.len());
                        for (idx, ln) in lines
                            .iter()
                            .enumerate()
                            .take(e.min(lines.len()))
                            .skip(s.saturating_sub(1))
                        {
                            out.push_str(&format!("{:>4} | {}\n", idx + 1, ln));
                        }
                        (
                            out,
                            finish("ok", format!("Read {}", p), String::new(), p.to_string()),
                        )
                    }
                    Err(e) => {
                        let m = format!("Cannot read '{}': {}", p, e);
                        (m.clone(), finish("error", m, String::new(), String::new()))
                    }
                },
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "write_file" => {
            let p = call.args.str_or("path", "");
            let content = call.args.str_or("content", "");
            match resolve_workspace_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => {
                    let old = std::fs::read_to_string(&resolved).unwrap_or_default();
                    if let Some(parent) = resolved.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&resolved, content) {
                        Ok(_) => {
                            let diff = compute_unified_diff(p, &old, content);
                            let _ = tx.send(AgentUiEvent::WorkspaceMutated);
                            let m = format!("Wrote {} bytes to {}", content.len(), p);
                            (m.clone(), finish("ok", m, diff, p.to_string()))
                        }
                        Err(e) => {
                            let m = format!("Failed writing '{}': {}", p, e);
                            (m.clone(), finish("error", m, String::new(), String::new()))
                        }
                    }
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "edit_file" => {
            let p = call.args.str_or("path", "");
            let old_t = call.args.str_or("old_text", "");
            let new_t = call.args.str_or("new_text", "");
            match resolve_workspace_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => match std::fs::read_to_string(&resolved) {
                    Ok(existing) => {
                        if !existing.contains(old_t) {
                            let m = format!(
                                "old_text not found in '{}'. Read the file first for exact context.",
                                p
                            );
                            (m.clone(), finish("error", m, String::new(), String::new()))
                        } else {
                            let updated = existing.replacen(old_t, new_t, 1);
                            match std::fs::write(&resolved, &updated) {
                                Ok(_) => {
                                    let diff = compute_unified_diff(p, &existing, &updated);
                                    let _ = tx.send(AgentUiEvent::WorkspaceMutated);
                                    let m = format!("Edited {} successfully", p);
                                    (m.clone(), finish("ok", m, diff, p.to_string()))
                                }
                                Err(e) => {
                                    let m = format!("Write error on '{}': {}", p, e);
                                    (m.clone(), finish("error", m, String::new(), String::new()))
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let m = format!("Read error on '{}': {}", p, e);
                        (m.clone(), finish("error", m, String::new(), String::new()))
                    }
                },
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "search_project" => {
            let q = call.args.str_or("query", "");
            let cs = call.args.bool_or("case_sensitive", false);
            let hits = search_project_files(project_root, q, cs, 60);
            if hits.is_empty() {
                let m = format!("No matches found for '{}'.", q);
                (m.clone(), finish("ok", m, String::new(), String::new()))
            } else {
                let mut out = format!("Found {} matches:\n", hits.len());
                for h in &hits {
                    out.push_str(&format!("{}:{} -> {}\n", h.rel_path, h.line, h.preview));
                }
                (
                    out,
                    finish(
                        "ok",
                        format!("{} matches for '{}'", hits.len(), q),
                        String::new(),
                        String::new(),
                    ),
                )
            }
        }
        "execute_command" => {
            let cmd_s = call.args.str_or("command", "");
            let sub_cwd = call
                .args
                .get("cwd")
                .and_then(|v| v.as_str())
                .and_then(|c| resolve_workspace_path(project_root, c, settings.restrict_to_workspace).ok())
                .unwrap_or_else(|| project_root.to_path_buf());
            match run_shell_silent(&sub_cwd, cmd_s) {
                Ok((code, o, e)) => {
                    let st = if code == 0 { "ok" } else { "error" };
                    let out = format!("Exit Code: {}\nSTDOUT:\n{}\nSTDERR:\n{}", code, o, e);
                    (
                        out,
                        finish(
                            st,
                            format!("Exit {}: {}", code, cmd_s),
                            String::new(),
                            String::new(),
                        ),
                    )
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "install_dependencies" => {
            let mgr = call.args.str_or("manager", "cargo");
            let pkgs: Vec<String> = call
                .args
                .get("packages")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let dev = call.args.bool_or("dev", false);
            let joined = pkgs.join(" ");
            let cmd_s = match mgr {
                "cargo" => {
                    if pkgs.is_empty() {
                        "cargo fetch".to_string()
                    } else {
                        format!("cargo add {}", joined)
                    }
                }
                "pip" => {
                    if pkgs.is_empty() {
                        "python -m pip install -r requirements.txt".to_string()
                    } else {
                        format!("python -m pip install {}", joined)
                    }
                }
                "pnpm" => format!("pnpm add {} {}", if dev { "-D" } else { "" }, joined),
                "yarn" => format!("yarn add {} {}", if dev { "-D" } else { "" }, joined),
                _ => {
                    if pkgs.is_empty() {
                        "npm install".to_string()
                    } else {
                        format!("npm install {} {}", if dev { "--save-dev" } else { "" }, joined)
                    }
                }
            };
            match run_shell_silent(project_root, &cmd_s) {
                Ok((code, o, e)) => {
                    let st = if code == 0 { "ok" } else { "error" };
                    (
                        format!("Command: {}\nExit Code: {}\n{}\n{}", cmd_s, code, o, e),
                        finish(st, cmd_s, String::new(), String::new()),
                    )
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "run_tests" => {
            let explicit = call.args.str_or("command", "");
            let cmd_s = if !explicit.is_empty() {
                explicit.to_string()
            } else if project_root.join("Cargo.toml").exists() {
                "cargo test".to_string()
            } else if project_root.join("package.json").exists() {
                "npm test -- --watchAll=false".to_string()
            } else {
                "pytest".to_string()
            };
            match run_shell_silent(project_root, &cmd_s) {
                Ok((code, o, e)) => {
                    let st = if code == 0 { "ok" } else { "error" };
                    (
                        format!("Test Command: {}\nExit Code: {}\n{}\n{}", cmd_s, code, o, e),
                        finish(
                            st,
                            format!("{} -> exit {}", cmd_s, code),
                            String::new(),
                            String::new(),
                        ),
                    )
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "diagnose_errors" => {
            let mut report = String::new();
            let mut any_err = false;
            if project_root.join("Cargo.toml").exists() {
                if let Ok((c, o, e)) = run_shell_silent(project_root, "cargo check --message-format=short") {
                    report.push_str(&format!("[Rust cargo check] exit={}\n{}{}\n", c, o, e));
                    if c != 0 {
                        any_err = true;
                    }
                }
            }
            if project_root.join("tsconfig.json").exists() {
                if let Ok((c, o, e)) = run_shell_silent(project_root, "npx tsc --noEmit") {
                    report.push_str(&format!("[TypeScript tsc --noEmit] exit={}\n{}{}\n", c, o, e));
                    if c != 0 {
                        any_err = true;
                    }
                }
            }
            if report.is_empty() {
                report.push_str("No Cargo.toml or tsconfig.json detected at workspace root.");
            }
            (
                report,
                finish(
                    if any_err { "error" } else { "ok" },
                    if any_err {
                        "Compiler diagnostics reported errors".to_string()
                    } else {
                        "Diagnostics clean".to_string()
                    },
                    String::new(),
                    String::new(),
                ),
            )
        }
        "git_operation" => {
            let sub = call.args.str_or("subcommand", "status");
            let extra: Vec<String> = call
                .args
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let mut argv = vec![sub];
            for item in &extra {
                argv.push(item.as_str());
            }
            match run_git(project_root, &argv) {
                Ok((c, o, e)) => {
                    let st = if c == 0 { "ok" } else { "error" };
                    (
                        format!("git {} -> exit {}\n{}{}", argv.join(" "), c, o, e),
                        finish(st, format!("git {}", argv.join(" ")), String::new(), String::new()),
                    )
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        other => {
            let m = format!("Unknown tool '{}'", other);
            (m.clone(), finish("error", m, String::new(), String::new()))
        }
    }
}

fn call_model_provider(
    settings: &AppSettings,
    provider: &str,
    model: &str,
    system_prompt: &str,
    conv: &[JsonValue],
) -> Result<TurnReply, String> {
    let oa_tools = tool_schemas_openai();

    if provider == "anthropic" {
        if settings.anthropic_api_key.trim().is_empty() {
            return Err("Anthropic API key is empty. Click [Settings] in the top bar to configure your key, or switch to Ollama.".to_string());
        }
        let mut anth_tools = Vec::new();
        if let Some(arr) = oa_tools.as_array() {
            for t in arr {
                if let Some(f) = t.get("function") {
                    anth_tools.push(JsonValue::obj(vec![
                        ("name", f.get("name").cloned().unwrap_or(JsonValue::Null)),
                        (
                            "description",
                            f.get("description").cloned().unwrap_or(JsonValue::Null),
                        ),
                        (
                            "input_schema",
                            f.get("parameters").cloned().unwrap_or(JsonValue::Null),
                        ),
                    ]));
                }
            }
        }
        let mut anth_msgs = Vec::new();
        for m in conv {
            let role = m.str_or("role", "user");
            if role == "tool" {
                anth_msgs.push(JsonValue::obj(vec![
                    ("role", "user".into()),
                    (
                        "content",
                        JsonValue::arr(vec![JsonValue::obj(vec![
                            ("type", "tool_result".into()),
                            ("tool_use_id", m.string_or("tool_call_id", "").into()),
                            ("content", m.string_or("content", "").into()),
                        ])]),
                    ),
                ]));
            } else if role == "assistant" {
                let mut blocks = Vec::new();
                let txt = m.str_or("content", "");
                if !txt.is_empty() {
                    blocks.push(JsonValue::obj(vec![
                        ("type", "text".into()),
                        ("text", txt.into()),
                    ]));
                }
                if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let raw_in = tc
                            .get("function")
                            .map(|f| f.str_or("arguments", "{}"))
                            .unwrap_or("{}");
                        let parsed_in = JsonValue::parse(raw_in)
                            .unwrap_or_else(|_| JsonValue::obj(Vec::new()));
                        blocks.push(JsonValue::obj(vec![
                            ("type", "tool_use".into()),
                            ("id", tc.string_or("id", "call_1").into()),
                            (
                                "name",
                                tc.get("function")
                                    .map(|f| f.string_or("name", ""))
                                    .unwrap_or_default()
                                    .into(),
                            ),
                            ("input", parsed_in),
                        ]));
                    }
                }
                if !blocks.is_empty() {
                    anth_msgs.push(JsonValue::obj(vec![
                        ("role", "assistant".into()),
                        ("content", JsonValue::arr(blocks)),
                    ]));
                }
            } else {
                anth_msgs.push(JsonValue::obj(vec![
                    ("role", "user".into()),
                    ("content", m.string_or("content", "").into()),
                ]));
            }
        }

        let payload = JsonValue::obj(vec![
            ("model", model.into()),
            ("max_tokens", 4096i64.into()),
            ("system", system_prompt.into()),
            ("tools", JsonValue::arr(anth_tools)),
            ("messages", JsonValue::arr(anth_msgs)),
        ]);
        let resp = http_request(
            "POST",
            "https://api.anthropic.com/v1/messages",
            &[
                ("Content-Type", "application/json"),
                ("x-api-key", settings.anthropic_api_key.trim()),
                ("anthropic-version", "2023-06-01"),
            ],
            &payload.to_json_string(),
            120_000,
        );
        if let Some(e) = resp.error {
            return Err(e);
        }
        if resp.status < 200 || resp.status >= 300 {
            return Err(format!(
                "Anthropic HTTP {}: {}",
                resp.status,
                resp.body.chars().take(400).collect::<String>()
            ));
        }
        let body = JsonValue::parse(&resp.body)?;
        let mut text = String::new();
        let mut thinking = String::new();
        let mut calls = Vec::new();
        if let Some(arr) = body.get("content").and_then(|v| v.as_array()) {
            for b in arr {
                match b.str_or("type", "") {
                    "text" => text.push_str(b.str_or("text", "")),
                    "thinking" => thinking.push_str(b.str_or("thinking", "")),
                    "tool_use" => calls.push(NormalizedCall {
                        id: b.string_or("id", &make_unique_id("call")),
                        name: b.string_or("name", ""),
                        args: b
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| JsonValue::obj(Vec::new())),
                    }),
                    _ => {}
                }
            }
        }
        return Ok(TurnReply {
            text,
            thinking,
            calls,
        });
    }

    if provider == "gemini" {
        if settings.gemini_api_key.trim().is_empty() {
            return Err("Google Gemini API key is empty. Configure it in [Settings].".to_string());
        }
        let mut decls = Vec::new();
        if let Some(arr) = oa_tools.as_array() {
            for t in arr {
                if let Some(f) = t.get("function") {
                    decls.push(JsonValue::obj(vec![
                        ("name", f.get("name").cloned().unwrap_or(JsonValue::Null)),
                        (
                            "description",
                            f.get("description").cloned().unwrap_or(JsonValue::Null),
                        ),
                        (
                            "parameters",
                            f.get("parameters").cloned().unwrap_or(JsonValue::Null),
                        ),
                    ]));
                }
            }
        }
        let mut contents = Vec::new();
        for m in conv {
            let role = m.str_or("role", "user");
            if role == "tool" {
                contents.push(JsonValue::obj(vec![
                    ("role", "user".into()),
                    (
                        "parts",
                        JsonValue::arr(vec![JsonValue::obj(vec![(
                            "functionResponse",
                            JsonValue::obj(vec![
                                ("name", m.string_or("name", "tool").into()),
                                (
                                    "response",
                                    JsonValue::obj(vec![(
                                        "result",
                                        m.string_or("content", "").into(),
                                    )]),
                                ),
                            ]),
                        )])]),
                    ),
                ]));
            } else if role == "assistant" {
                let mut parts = Vec::new();
                let txt = m.str_or("content", "");
                if !txt.is_empty() {
                    parts.push(JsonValue::obj(vec![("text", txt.into())]));
                }
                if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let raw_a = tc
                            .get("function")
                            .map(|f| f.str_or("arguments", "{}"))
                            .unwrap_or("{}");
                        let parsed_a = JsonValue::parse(raw_a)
                            .unwrap_or_else(|_| JsonValue::obj(Vec::new()));
                        parts.push(JsonValue::obj(vec![(
                            "functionCall",
                            JsonValue::obj(vec![
                                (
                                    "name",
                                    tc.get("function")
                                        .map(|f| f.string_or("name", ""))
                                        .unwrap_or_default()
                                        .into(),
                                ),
                                ("args", parsed_a),
                            ]),
                        )]));
                    }
                }
                if !parts.empty_check() {
                    contents.push(JsonValue::obj(vec![
                        ("role", "model".into()),
                        ("parts", JsonValue::arr(parts)),
                    ]));
                }
            } else {
                contents.push(JsonValue::obj(vec![
                    ("role", "user".into()),
                    (
                        "parts",
                        JsonValue::arr(vec![JsonValue::obj(vec![(
                            "text",
                            m.string_or("content", "").into(),
                        )])]),
                    ),
                ]));
            }
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model,
            settings.gemini_api_key.trim()
        );
        let payload = JsonValue::obj(vec![
            (
                "systemInstruction",
                JsonValue::obj(vec![(
                    "parts",
                    JsonValue::arr(vec![JsonValue::obj(vec![("text", system_prompt.into())])]),
                )]),
            ),
            ("contents", JsonValue::arr(contents)),
            (
                "tools",
                JsonValue::arr(vec![JsonValue::obj(vec![(
                    "functionDeclarations",
                    JsonValue::arr(decls),
                )])]),
            ),
        ]);
        let resp = http_request(
            "POST",
            &url,
            &[("Content-Type", "application/json")],
            &payload.to_json_string(),
            120_000,
        );
        if let Some(e) = resp.error {
            return Err(e);
        }
        if resp.status < 200 || resp.status >= 300 {
            return Err(format!(
                "Gemini HTTP {}: {}",
                resp.status,
                resp.body.chars().take(400).collect::<String>()
            ));
        }
        let body = JsonValue::parse(&resp.body)?;
        let mut text = String::new();
        let mut calls = Vec::new();
        if let Some(parts) = body
            .get("candidates")
            .and_then(|c| c.get_index(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for p in parts {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    text.push_str(t);
                }
                if let Some(fc) = p.get("functionCall") {
                    calls.push(NormalizedCall {
                        id: make_unique_id("gem"),
                        name: fc.string_or("name", ""),
                        args: fc
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| JsonValue::obj(Vec::new())),
                    });
                }
            }
        }
        return Ok(TurnReply {
            text,
            thinking: String::new(),
            calls,
        });
    }

    // OpenAI-compatible protocol: openai, openrouter, ollama, custom
    let (endpoint, bearer) = match provider {
        "openai" => {
            if settings.openai_api_key.trim().is_empty() {
                return Err(
                    "OpenAI API key is empty. Configure it in [Settings] or switch to local Ollama."
                        .to_string(),
                );
            }
            (
                "https://api.openai.com/v1/chat/completions".to_string(),
                settings.openai_api_key.trim().to_string(),
            )
        }
        "openrouter" => {
            if settings.openrouter_api_key.trim().is_empty() {
                return Err("OpenRouter API key is empty. Configure it in [Settings].".to_string());
            }
            (
                "https://openrouter.ai/api/v1/chat/completions".to_string(),
                settings.openrouter_api_key.trim().to_string(),
            )
        }
        "custom" => {
            let base = settings.custom_base_url.trim().trim_end_matches('/');
            if base.is_empty() {
                return Err("Custom Base URL is empty in [Settings].".to_string());
            }
            (
                format!("{}/chat/completions", base),
                settings.custom_api_key.trim().to_string(),
            )
        }
        _ => {
            let base = settings.ollama_base_url.trim().trim_end_matches('/');
            (format!("{}/v1/chat/completions", base), String::new())
        }
    };

    let mut msgs = vec![JsonValue::obj(vec![
        ("role", "system".into()),
        ("content", system_prompt.into()),
    ])];
    msgs.extend_from_slice(conv);

    let payload = JsonValue::obj(vec![
        ("model", model.into()),
        ("messages", JsonValue::arr(msgs)),
        ("tools", oa_tools),
        ("tool_choice", "auto".into()),
    ]);

    let auth_val = format!("Bearer {}", bearer);
    let mut hdrs = vec![("Content-Type", "application/json")];
    if !bearer.is_empty() {
        hdrs.push(("Authorization", auth_val.as_str()));
    }

    let resp = http_request("POST", &endpoint, &hdrs, &payload.to_json_string(), 120_000);
    if let Some(e) = resp.error {
        return Err(e);
    }
    if resp.status < 200 || resp.status >= 300 {
        return Err(format!(
            "Provider '{}' HTTP {}: {}",
            provider,
            resp.status,
            resp.body.chars().take(400).collect::<String>()
        ));
    }

    let body = JsonValue::parse(&resp.body)?;
    let msg = body
        .get("choices")
        .and_then(|c| c.get_index(0))
        .and_then(|c| c.get("message"))
        .ok_or_else(|| "Missing choices[0].message in response".to_string())?;

    let text = msg.string_or("content", "");
    let thinking = msg
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut calls = Vec::new();
    if let Some(arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in arr {
            let fn_obj = tc.get("function");
            let raw_args = fn_obj.map(|f| f.str_or("arguments", "{}")).unwrap_or("{}");
            let parsed_args =
                JsonValue::parse(raw_args).unwrap_or_else(|_| JsonValue::obj(Vec::new()));
            calls.push(NormalizedCall {
                id: tc.string_or("id", &make_unique_id("call")),
                name: fn_obj
                    .map(|f| f.string_or("name", ""))
                    .unwrap_or_default(),
                args: parsed_args,
            });
        }
    }

    Ok(TurnReply {
        text,
        thinking,
        calls,
    })
}

trait VecExt {
    fn empty_check(&self) -> bool;
}
impl<T> VecExt for Vec<T> {
    fn empty_check(&self) -> bool {
        self.is_empty()
    }
}

pub fn spawn_autonomous_agent_run(
    store: Arc<SqliteStore>,
    gate: Arc<PermissionGate>,
    project_root: PathBuf,
    session_id: String,
    prompt: String,
    mode: String,
    tx: Sender<AgentUiEvent>,
) {
    std::thread::spawn(move || {
        gate.reset_stop();
        let settings = store.load_settings();
        let provider = settings.active_provider.clone();
        let model = settings.active_model.clone();

        store.append_message(&ChatMessage {
            id: make_unique_id("msg"),
            session_id: session_id.clone(),
            role: "user".to_string(),
            content: prompt.clone(),
            thinking: String::new(),
            mode: mode.clone(),
            provider: provider.clone(),
            model: model.clone(),
            tools: Vec::new(),
            created_at: now_timestamp_label(),
        });

        let mut conv: Vec<JsonValue> = Vec::new();
        if let Some(detail) = store.load_session(&session_id) {
            let skip_n = detail.messages.len().saturating_sub(20);
            for m in detail.messages.into_iter().skip(skip_n) {
                if m.role == "user" || m.role == "assistant" {
                    conv.push(JsonValue::obj(vec![
                        ("role", m.role.into()),
                        ("content", m.content.into()),
                    ]));
                }
            }
        }

        let sample_files: Vec<String> =
            scan_project_tree(&project_root, &HashSet::new(), true, 80)
                .into_iter()
                .take(50)
                .map(|f| {
                    if f.is_dir {
                        format!("{}/", f.rel_path)
                    } else {
                        f.rel_path
                    }
                })
                .collect();

        let mode_instruction = match mode.as_str() {
            "plan" => "MODE: PLAN (Strict Read-Only). Inspect files and return a thorough file-by-file architectural plan without modifying files.",
            "ask" => "MODE: ASK. Mutating tools will prompt the user for confirmation.",
            _ => "MODE: AUTONOMOUS. Proactively inspect files, implement complete multi-file changes, run diagnose_errors / run_tests to verify compilation, fix any errors automatically, and summarize verified results.",
        };

        let sys_prompt = format!(
            "You are AI Coding Agent, a native Pure-Rust desktop autonomous coding agent.\n\
             Workspace: {}\n{}\n\nWorkspace Tree:\n{}",
            project_root.display(),
            mode_instruction,
            sample_files.join("\n")
        );

        let max_iters = settings.max_agent_iterations.clamp(1, 30);
        let mut collected_tools = Vec::new();
        let mut final_text = String::new();
        let mut final_thinking = String::new();

        for step in 1..=max_iters {
            if gate.is_stopped() {
                break;
            }
            let _ = tx.send(AgentUiEvent::Status(format!(
                "Step {}/{}: Querying {} ({})",
                step, max_iters, provider, model
            )));

            let reply = match call_model_provider(&settings, &provider, &model, &sys_prompt, &conv)
            {
                Ok(r) => r,
                Err(err) => {
                    let _ = tx.send(AgentUiEvent::Error(err.clone()));
                    store.append_message(&ChatMessage {
                        id: make_unique_id("msg"),
                        session_id: session_id.clone(),
                        role: "assistant".to_string(),
                        content: format!("Error: {}", err),
                        thinking: final_thinking,
                        mode,
                        provider,
                        model,
                        tools: collected_tools,
                        created_at: now_timestamp_label(),
                    });
                    let _ = tx.send(AgentUiEvent::Done(store.load_session(&session_id)));
                    return;
                }
            };

            if !reply.thinking.is_empty() {
                final_thinking.push_str(&reply.thinking);
                let _ = tx.send(AgentUiEvent::ThinkingDelta(reply.thinking));
            }
            if !reply.text.is_empty() {
                if !final_text.is_empty() {
                    final_text.push_str("\n\n");
                }
                final_text.push_str(&reply.text);
                let _ = tx.send(AgentUiEvent::TextDelta(reply.text.clone()));
            }

            if reply.calls.is_empty() {
                break;
            }

            let oa_calls: Vec<JsonValue> = reply
                .calls
                .iter()
                .map(|c| {
                    JsonValue::obj(vec![
                        ("id", c.id.clone().into()),
                        ("type", "function".into()),
                        (
                            "function",
                            JsonValue::obj(vec![
                                ("name", c.name.clone().into()),
                                ("arguments", c.args.to_json_string().into()),
                            ]),
                        ),
                    ])
                })
                .collect();

            conv.push(JsonValue::obj(vec![
                ("role", "assistant".into()),
                ("content", reply.text.into()),
                ("tool_calls", JsonValue::arr(oa_calls)),
            ]));

            for call in &reply.calls {
                if gate.is_stopped() {
                    break;
                }
                let start_rec = ToolExecutionRecord {
                    id: call.id.clone(),
                    session_id: session_id.clone(),
                    tool_name: call.name.clone(),
                    args_json: call.args.to_json_string(),
                    result_summary: "Running...".to_string(),
                    status: "running".to_string(),
                    duration_ms: 0,
                    diff_text: String::new(),
                    file_path: String::new(),
                    created_at: now_timestamp_label(),
                };
                let _ = tx.send(AgentUiEvent::ToolStarted(start_rec));

                let (tool_out, done_rec) = execute_tool(
                    &project_root,
                    &settings,
                    &gate,
                    &tx,
                    &session_id,
                    &mode,
                    call,
                );
                store.record_tool_execution(&done_rec);
                let _ = tx.send(AgentUiEvent::ToolFinished(done_rec.clone()));
                collected_tools.push(done_rec);

                conv.push(JsonValue::obj(vec![
                    ("role", "tool".into()),
                    ("tool_call_id", call.id.clone().into()),
                    ("name", call.name.clone().into()),
                    ("content", tool_out.into()),
                ]));
            }
        }

        if final_text.trim().is_empty() {
            final_text = if gate.is_stopped() {
                "Execution halted immediately via Emergency Stop.".to_string()
            } else {
                "Completed autonomous tool execution cycle.".to_string()
            };
        }

        store.append_message(&ChatMessage {
            id: make_unique_id("msg"),
            session_id: session_id.clone(),
            role: "assistant".to_string(),
            content: final_text,
            thinking: final_thinking,
            mode,
            provider,
            model,
            tools: collected_tools,
            created_at: now_timestamp_label(),
        });

        let _ = tx.send(AgentUiEvent::Done(store.load_session(&session_id)));
    });
}
