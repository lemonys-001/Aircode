//! Pure-Rust Multi-Provider Autonomous AI Coding Agent Engine (9 Tools + Permission Gate + Emergency Stop).

use crate::browser::SharedBrowser;
use crate::fs_ops::{
    compute_unified_diff, resolve_workspace_path, scan_project_tree, search_project_files,
};
use crate::git_ops::run_git;
use crate::json::JsonValue;
use crate::mcp::{call_mcp_tool, load_mcp_configs};
use crate::opencode_auth::{call_local_opencode_server, discover_local_credentials};
use crate::platform::{app_data_dir, http_request};
use crate::skills::{format_enabled_skills_for_prompt, load_all_skills, save_workspace_skill};
use crate::sqlite::{
    make_unique_id, now_timestamp_label, AppSettings, ChatMessage, SessionDetail, SqliteStore,
    ToolExecutionRecord,
};
use crate::subagents::{load_all_subagents, save_subagent, SubAgentSpec};
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
    OpenBrowserViewport,
    ReloadCustomizations,
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
      },
      {
        "type": "function",
        "function": {
          "name": "browser_navigate",
          "description": "Open a URL (localhost dev server, docs, or web app) in AirCode's inbuilt Agentic Browser and return a Playwright-style [eN] snapshot.",
          "parameters": {
            "type": "object",
            "properties": {
              "url": { "type": "string", "description": "Target URL such as http://localhost:3000 or https://..." }
            },
            "required": ["url"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "browser_snapshot",
          "description": "Return the current Playwright-style accessibility & [eN] interactive element snapshot of AirCode's inbuilt Agentic Browser.",
          "parameters": {
            "type": "object",
            "properties": {}
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "browser_click",
          "description": "Click an interactive link or button in AirCode's inbuilt Agentic Browser by its numeric ref (e.g. ref_id=3 for [e3]).",
          "parameters": {
            "type": "object",
            "properties": {
              "ref_id": { "type": "integer", "description": "Numeric element ref ID from browser_snapshot" }
            },
            "required": ["ref_id"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "browser_type",
          "description": "Type text into an input element [eN] in AirCode's inbuilt Agentic Browser, optionally submitting its form.",
          "parameters": {
            "type": "object",
            "properties": {
              "ref_id": { "type": "integer" },
              "text": { "type": "string" },
              "submit": { "type": "boolean" }
            },
            "required": ["ref_id", "text"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "create_skill",
          "description": "Create or update a reusable workspace skill in .aircode/skills/<slug>/SKILL.md.",
          "parameters": {
            "type": "object",
            "properties": {
              "slug": { "type": "string" },
              "title": { "type": "string" },
              "description": { "type": "string" },
              "instructions": { "type": "string" }
            },
            "required": ["slug", "title", "instructions"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "create_subagent",
          "description": "Create or update a specialized AI sub-agent in .aircode/subagents/<slug>.json so you can delegate tasks to it via run_subagent.",
          "parameters": {
            "type": "object",
            "properties": {
              "slug": { "type": "string" },
              "title": { "type": "string" },
              "effort": { "type": "string", "enum": ["low", "medium", "max", "ultra"] },
              "readonly": { "type": "boolean" },
              "system_prompt": { "type": "string" }
            },
            "required": ["slug", "title", "system_prompt"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "run_subagent",
          "description": "Delegate a focused subtask to a built-in or self-built sub-agent (e.g. explorer, architect, test-healer, browser-verifier, or any custom slug).",
          "parameters": {
            "type": "object",
            "properties": {
              "slug": { "type": "string" },
              "task": { "type": "string" }
            },
            "required": ["slug", "task"]
          }
        }
      },
      {
        "type": "function",
        "function": {
          "name": "mcp_call",
          "description": "Invoke a Model Context Protocol (MCP) tool exposed by a server configured in .aircode/mcp.json.",
          "parameters": {
            "type": "object",
            "properties": {
              "server": { "type": "string" },
              "tool": { "type": "string" },
              "arguments": { "type": "object" }
            },
            "required": ["server", "tool"]
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
    browser: &SharedBrowser,
    tx: &Sender<AgentUiEvent>,
    session_id: &str,
    mode: &str,
    allow_subagent_recursion: bool,
    call: &NormalizedCall,
) -> (String, ToolExecutionRecord) {
    let started = Instant::now();
    let args_json = call.args.to_json_string();
    let name = call.name.as_str();

    let is_mut_file = matches!(
        name,
        "write_file" | "edit_file" | "create_skill" | "create_subagent"
    );
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
                            if p.contains(".aircode") {
                                let _ = tx.send(AgentUiEvent::ReloadCustomizations);
                            }
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
                                    if p.contains(".aircode") {
                                        let _ = tx.send(AgentUiEvent::ReloadCustomizations);
                                    }
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
        "browser_navigate" => {
            let url = call.args.str_or("url", "aircode://welcome");
            let _ = tx.send(AgentUiEvent::OpenBrowserViewport);
            match browser.navigate(url, true) {
                Ok(snap) => (
                    snap,
                    finish(
                        "ok",
                        format!("Browsed {}", url),
                        String::new(),
                        String::new(),
                    ),
                ),
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "browser_snapshot" => {
            let _ = tx.send(AgentUiEvent::OpenBrowserViewport);
            let snap = browser.snapshot(50);
            (
                snap,
                finish(
                    "ok",
                    "Captured Agentic Browser DOM snapshot".into(),
                    String::new(),
                    String::new(),
                ),
            )
        }
        "browser_click" => {
            let ref_id = call.args.i64_or("ref_id", 1).max(1) as usize;
            let _ = tx.send(AgentUiEvent::OpenBrowserViewport);
            match browser.click_ref(ref_id, true) {
                Ok(snap) => (
                    snap,
                    finish(
                        "ok",
                        format!("Clicked browser element [e{}]", ref_id),
                        String::new(),
                        String::new(),
                    ),
                ),
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "browser_type" => {
            let ref_id = call.args.i64_or("ref_id", 1).max(1) as usize;
            let txt = call.args.str_or("text", "");
            let submit = call.args.bool_or("submit", false);
            let _ = tx.send(AgentUiEvent::OpenBrowserViewport);
            match browser.type_into_ref(ref_id, txt, submit, true) {
                Ok(snap) => (
                    snap,
                    finish(
                        "ok",
                        format!("Typed into browser [e{}]", ref_id),
                        String::new(),
                        String::new(),
                    ),
                ),
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "create_skill" => {
            let slug = call.args.str_or("slug", "custom-skill");
            let title = call.args.str_or("title", "Custom Skill");
            let desc = call.args.str_or("description", "AI-created workspace skill");
            let instr = call.args.str_or("instructions", "");
            match save_workspace_skill(project_root, slug, title, desc, instr) {
                Ok(path) => {
                    let _ = tx.send(AgentUiEvent::WorkspaceMutated);
                    let _ = tx.send(AgentUiEvent::ReloadCustomizations);
                    let m = format!("Created skill '{}' at {}", slug, path.display());
                    (m.clone(), finish("ok", m, String::new(), path.to_string_lossy().into_owned()))
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "create_subagent" => {
            let spec = SubAgentSpec {
                slug: call.args.string_or("slug", "helper-agent"),
                title: call.args.string_or("title", "Helper Sub-Agent"),
                effort: call.args.string_or("effort", "medium"),
                readonly: call.args.bool_or("readonly", false),
                system_prompt: call.args.string_or("system_prompt", ""),
                builtin: false,
            };
            match save_subagent(project_root, &spec) {
                Ok(path) => {
                    let _ = tx.send(AgentUiEvent::WorkspaceMutated);
                    let _ = tx.send(AgentUiEvent::ReloadCustomizations);
                    let m = format!("Built sub-agent '{}' ({})", spec.slug, path.display());
                    (m.clone(), finish("ok", m, String::new(), path.to_string_lossy().into_owned()))
                }
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        "run_subagent" => {
            if !allow_subagent_recursion {
                let m = "Nested sub-agents cannot spawn further sub-agents.".to_string();
                return (m.clone(), finish("denied", m, String::new(), String::new()));
            }
            let slug = call.args.str_or("slug", "explorer");
            let task = call.args.str_or("task", "");
            let specs = load_all_subagents(project_root);
            let Some(spec) = specs.into_iter().find(|s| s.slug == slug) else {
                let m = format!("Sub-agent '{}' not found. Call create_subagent first or use explorer/architect/test-healer/browser-verifier.", slug);
                return (m.clone(), finish("error", m, String::new(), String::new()));
            };
            let sub_mode = if mode == "plan" || spec.readonly {
                "plan"
            } else {
                mode
            };
            let mut sub_settings = settings.clone();
            sub_settings.agent_effort = spec.effort.clone();
            let sub_sys = format!(
                "You are AirCode Sub-Agent `{}` ({}).\n{}\nWorkspace: {}\nReturn a concise, verified engineering report to the parent agent.",
                spec.slug,
                spec.title,
                spec.system_prompt,
                project_root.display()
            );
            let mut sub_conv = vec![JsonValue::obj(vec![
                ("role", "user".into()),
                ("content", task.into()),
            ])];
            let sub_iters = sub_settings.effort_max_iters().min(8);
            let mut sub_report = String::new();
            for step in 1..=sub_iters {
                if gate.is_stopped() {
                    break;
                }
                let _ = tx.send(AgentUiEvent::Status(format!(
                    "Sub-Agent `{}` step {}/{}",
                    spec.slug, step, sub_iters
                )));
                match call_model_provider(
                    &sub_settings,
                    &sub_settings.active_provider,
                    &sub_settings.active_model,
                    &sub_sys,
                    &sub_conv,
                ) {
                    Ok(rep) => {
                        if !rep.text.is_empty() {
                            sub_report = rep.text.clone();
                        }
                        if rep.calls.is_empty() {
                            break;
                        }
                        let oa_calls: Vec<JsonValue> = rep
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
                        sub_conv.push(JsonValue::obj(vec![
                            ("role", "assistant".into()),
                            ("content", rep.text.into()),
                            ("tool_calls", JsonValue::arr(oa_calls)),
                        ]));
                        for sc in &rep.calls {
                            let (tout, _) = execute_tool(
                                project_root,
                                &sub_settings,
                                gate,
                                browser,
                                tx,
                                session_id,
                                sub_mode,
                                false,
                                sc,
                            );
                            sub_conv.push(JsonValue::obj(vec![
                                ("role", "tool".into()),
                                ("tool_call_id", sc.id.clone().into()),
                                ("name", sc.name.clone().into()),
                                ("content", tout.into()),
                            ]));
                        }
                    }
                    Err(e) => {
                        sub_report = format!("Sub-agent `{}` error: {}", spec.slug, e);
                        break;
                    }
                }
            }
            if sub_report.is_empty() {
                sub_report = format!("Sub-agent `{}` completed delegated task.", spec.slug);
            }
            (
                format!("[Sub-Agent `{}` Report]\n{}", spec.slug, sub_report),
                finish(
                    "ok",
                    format!("Sub-agent `{}` finished", spec.slug),
                    String::new(),
                    String::new(),
                ),
            )
        }
        "mcp_call" => {
            let server = call.args.str_or("server", "");
            let tool = call.args.str_or("tool", "");
            let args = call
                .args
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| JsonValue::obj(vec![]));
            let servers = load_mcp_configs(project_root, &app_data_dir());
            match call_mcp_tool(&servers, project_root, server, tool, args) {
                Ok(out) => (
                    out,
                    finish(
                        "ok",
                        format!("MCP {}/{}", server, tool),
                        String::new(),
                        String::new(),
                    ),
                ),
                Err(e) => (e.clone(), finish("error", e, String::new(), String::new())),
            }
        }
        other => {
            let m = format!("Unknown tool '{}'", other);
            (m.clone(), finish("error", m, String::new(), String::new()))
        }
    }
}

fn split_think_tags(raw_text: &str, existing_thinking: &str) -> (String, String) {
    let mut thinking = existing_thinking.to_string();
    let mut clean_text = raw_text.to_string();
    while let Some(start) = clean_text.find("<think>") {
        if let Some(end_rel) = clean_text[start + 7..].find("</think>") {
            let end = start + 7 + end_rel;
            let inner = clean_text[start + 7..end].trim();
            if !inner.is_empty() {
                if !thinking.is_empty() {
                    thinking.push('\n');
                }
                thinking.push_str(inner);
            }
            clean_text.replace_range(start..(end + 8), "");
        } else {
            break;
        }
    }
    (clean_text.trim().to_string(), thinking)
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
        let disc = discover_local_credentials(&settings.opencode_server_url, &settings.ollama_base_url);
        let anth_key = if !settings.anthropic_api_key.trim().is_empty() {
            settings.anthropic_api_key.trim().to_string()
        } else {
            disc.anthropic_key.unwrap_or_default()
        };
        if anth_key.is_empty() {
            return Err("Anthropic API key is empty. Press Ctrl+K -> /connect to paste your key or switch to OpenCode Zen.".to_string());
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

        let use_ext_thinking = matches!(settings.agent_effort.as_str(), "max" | "ultra")
            && (model.contains("3-7") || model.contains("sonnet-4") || model.contains("opus-4"));
        let budget = settings.effort_thinking_budget().clamp(1024, 16000) as i64;
        let max_toks = if use_ext_thinking { budget + 4096 } else { 4096 };
        let mut p_fields = vec![
            ("model", model.into()),
            ("max_tokens", max_toks.into()),
            ("system", system_prompt.into()),
            ("tools", JsonValue::arr(anth_tools)),
            ("messages", JsonValue::arr(anth_msgs)),
        ];
        if use_ext_thinking {
            p_fields.push((
                "thinking",
                JsonValue::obj(vec![
                    ("type", "enabled".into()),
                    ("budget_tokens", budget.into()),
                ]),
            ));
        }
        let payload = JsonValue::obj(p_fields);
        let resp = http_request(
            "POST",
            "https://api.anthropic.com/v1/messages",
            &[
                ("Content-Type", "application/json"),
                ("x-api-key", &anth_key),
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

    // OpenCode local server bridge workaround (`opencode serve --port 4096`)
    if provider == "opencode" && settings.opencode_auth_mode == "local-serve" {
        let joined_prompt = conv
            .last()
            .map(|m| m.string_or("content", ""))
            .unwrap_or_default();
        let text = call_local_opencode_server(
            &settings.opencode_server_url,
            model,
            system_prompt,
            &joined_prompt,
        )?;
        let (clean_t, th) = split_think_tags(&text, "");
        return Ok(TurnReply {
            text: clean_t,
            thinking: th,
            calls: Vec::new(),
        });
    }

    // OpenAI-compatible protocol: opencode (Zen gateway + public/key workaround), openai, openrouter, ollama, custom
    let discovered = discover_local_credentials(&settings.opencode_server_url, &settings.ollama_base_url);
    let (endpoint, bearer, is_opencode_zen) = match provider {
        "opencode" => {
            let key = if !settings.opencode_api_key.trim().is_empty() {
                settings.opencode_api_key.trim().to_string()
            } else if let Some(k) = discovered.opencode_zen_key {
                k
            } else {
                // Zero-key OpenCode Zen public client workaround for free coding models!
                "public".to_string()
            };
            (
                "https://opencode.ai/zen/v1/chat/completions".to_string(),
                key,
                true,
            )
        }
        "openai" => {
            let key = if !settings.openai_api_key.trim().is_empty() {
                settings.openai_api_key.trim().to_string()
            } else {
                discovered.openai_key.unwrap_or_default()
            };
            if key.is_empty() {
                return Err(
                    "OpenAI API key is empty. Press Ctrl+K -> /connect to paste your key or switch to OpenCode Zen."
                        .to_string(),
                );
            }
            (
                "https://api.openai.com/v1/chat/completions".to_string(),
                key,
                false,
            )
        }
        "openrouter" => {
            let key = if !settings.openrouter_api_key.trim().is_empty() {
                settings.openrouter_api_key.trim().to_string()
            } else {
                discovered.openrouter_key.unwrap_or_default()
            };
            if key.is_empty() {
                return Err(
                    "OpenRouter API key is empty. Press Ctrl+K -> /connect to configure it."
                        .to_string(),
                );
            }
            (
                "https://openrouter.ai/api/v1/chat/completions".to_string(),
                key,
                false,
            )
        }
        "custom" => {
            let base = settings.custom_base_url.trim().trim_end_matches('/');
            if base.is_empty() {
                return Err("Custom Base URL is empty. Open /connect to set it.".to_string());
            }
            (
                format!("{}/chat/completions", base),
                settings.custom_api_key.trim().to_string(),
                false,
            )
        }
        _ => {
            let base = settings.ollama_base_url.trim().trim_end_matches('/');
            (format!("{}/v1/chat/completions", base), String::new(), false)
        }
    };

    let mut msgs = vec![JsonValue::obj(vec![
        ("role", "system".into()),
        ("content", system_prompt.into()),
    ])];
    msgs.extend_from_slice(conv);

    let mut fields = vec![
        ("model", model.into()),
        ("messages", JsonValue::arr(msgs)),
        ("tools", oa_tools),
        ("tool_choice", "auto".into()),
    ];
    if provider == "openai" && model.starts_with('o') {
        fields.push(("reasoning_effort", settings.effort_openai_tag().into()));
    }
    let payload = JsonValue::obj(fields);

    let auth_val = format!("Bearer {}", bearer);
    let mut hdrs = vec![("Content-Type", "application/json")];
    if !bearer.is_empty() {
        hdrs.push(("Authorization", auth_val.as_str()));
    }
    if is_opencode_zen {
        hdrs.push(("User-Agent", "opencode/1.0"));
        hdrs.push(("HTTP-Referer", "https://opencode.ai/"));
        hdrs.push(("X-Title", "opencode"));
        hdrs.push(("x-opencode-client", "cli"));
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

    let raw_text = msg.string_or("content", "");
    let raw_reasoning = msg
        .get("reasoning_content")
        .or_else(|| msg.get("reasoning"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let (text, thinking) = split_think_tags(&raw_text, raw_reasoning);

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

pub fn spawn_session_compaction(
    store: Arc<SqliteStore>,
    session_id: String,
    tx: Sender<AgentUiEvent>,
) {
    std::thread::spawn(move || {
        let Some(detail) = store.load_session(&session_id) else {
            let _ = tx.send(AgentUiEvent::Done(None));
            return;
        };
        if detail.messages.len() <= 2 {
            let _ = tx.send(AgentUiEvent::Status(
                "Session is already compact (<= 2 messages).".into(),
            ));
            let _ = tx.send(AgentUiEvent::Done(Some(detail)));
            return;
        }
        let _ = tx.send(AgentUiEvent::Status(
            "Compacting session context into a clean checkpoint...".into(),
        ));

        let mut transcript = String::new();
        let mut bullet_fallback = Vec::new();
        for (idx, m) in detail.messages.iter().enumerate() {
            let snippet: String = m.content.chars().take(300).collect();
            transcript.push_str(&format!("[#{} {} @ {}]: {}\n", idx + 1, m.role, m.created_at, snippet));
            if m.role == "user" {
                let first_line = m.content.lines().next().unwrap_or("").chars().take(90).collect::<String>();
                bullet_fallback.push(format!("- Prompt #{}: {}", idx + 1, first_line));
            } else if !m.tools.is_empty() {
                let tnames: Vec<String> = m.tools.iter().map(|t| t.tool_name.clone()).collect();
                bullet_fallback.push(format!("- Tools executed: {}", tnames.join(", ")));
            }
        }

        let settings = store.load_settings();
        let sum_conv = vec![JsonValue::obj(vec![
            ("role", "user".into()),
            (
                "content",
                format!(
                    "Summarize this coding session concisely into bullet points covering: (1) User goals, (2) Files modified & key architecture decisions, (3) Remaining next steps:\n\n{}",
                    transcript
                )
                .into(),
            ),
        ])];

        let summary = match call_model_provider(
            &settings,
            &settings.active_provider,
            &settings.active_model,
            "You are AirCode's Context Compactor. Return a concise markdown bullet checkpoint.",
            &sum_conv,
        ) {
            Ok(rep) if !rep.text.trim().is_empty() => rep.text,
            _ => {
                if bullet_fallback.is_empty() {
                    format!("Compacted {} earlier messages.", detail.messages.len())
                } else {
                    bullet_fallback.join("\n")
                }
            }
        };

        let updated = store.compact_session(&session_id, &summary);
        let _ = tx.send(AgentUiEvent::Done(updated));
    });
}

pub fn spawn_autonomous_agent_run(
    store: Arc<SqliteStore>,
    gate: Arc<PermissionGate>,
    browser: SharedBrowser,
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
        let effort = settings.agent_effort.clone();

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
            let skip_n = detail.messages.len().saturating_sub(24);
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
            "plan" => "MODE: PLAN (Strict Read-Only Blueprint). Do NOT modify files or run shell commands. Research the codebase with read_file / search_project / browser_snapshot and output a crisp, file-by-file implementation plan.",
            _ => "MODE: BUILD (Autonomous Engineering). Proactively read target files, apply clean edits, verify with diagnose_errors / run_tests / Agentic Browser tools, and report verified results.",
        };

        let effort_instruction = match effort.as_str() {
            "low" => "EFFORT: LOW — Be fast, direct, and minimal. Avoid unnecessary exploration.",
            "max" => "EFFORT: MAX — Think deeply inside <think>...</think> before acting, inspect edge cases across callers, and run compiler/test diagnostics to verify.",
            "ultra" => "EFFORT: ULTRA — Maximum first-principles reasoning inside <think>...</think>, thorough multi-file audit, sub-agent delegation when helpful, and full compile/test verification.",
            _ => "EFFORT: MEDIUM — Balanced reasoning inside <think>...</think>, clean edits, and verification.",
        };

        let disabled_set: HashSet<String> = settings.disabled_skills.iter().cloned().collect();
        let active_skills = load_all_skills(&project_root, &app_data_dir(), &disabled_set);
        let skills_block = format_enabled_skills_for_prompt(&active_skills);

        let subagents = load_all_subagents(&project_root);
        let subagent_names: Vec<String> = subagents
            .iter()
            .map(|s| format!("`{}` ({})", s.slug, s.title))
            .collect();

        let mcp_servers = load_mcp_configs(&project_root, &app_data_dir());
        let mcp_note = if mcp_servers.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = mcp_servers
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.name.clone())
                .collect();
            format!("\nConfigured MCP Servers (use `mcp_call`): {}\n", names.join(", "))
        };

        let sys_prompt = format!(
            "You are AirCode, a 100% Pure-Rust Native Desktop AI Coding Studio.\n\
             Workspace: {}\n{}\n{}\n\
             Available Sub-Agents (`run_subagent` or `create_subagent`): {}\n{}\
             When reasoning before your answer or tool calls, wrap your internal reasoning in <think>...</think> so AirCode can display or hide your thinking cleanly.\n\
             \nWorkspace Tree:\n{}{}",
            project_root.display(),
            mode_instruction,
            effort_instruction,
            subagent_names.join(", "),
            mcp_note,
            sample_files.join("\n"),
            skills_block
        );

        let max_iters = settings.effort_max_iters();
        let mut collected_tools = Vec::new();
        let mut final_text = String::new();
        let mut final_thinking = String::new();

        for step in 1..=max_iters {
            if gate.is_stopped() {
                break;
            }
            let _ = tx.send(AgentUiEvent::Status(format!(
                "[{} · {}] Step {}/{}: {} ({})",
                mode.to_uppercase(),
                effort.to_uppercase(),
                step,
                max_iters,
                provider,
                model
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
                if !final_thinking.is_empty() {
                    final_thinking.push('\n');
                }
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
                    &browser,
                    &tx,
                    &session_id,
                    &mode,
                    true,
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
                "Completed autonomous execution cycle.".to_string()
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
