use crate::db::{AppSettings, ChatMessage, SqliteStore, ToolExecutionRecord};
use crate::fs_ops::{compute_unified_diff, resolve_project_path, scan_project_tree, search_project_files};
use crate::git_ops::run_git;
use crate::permissions::PermissionGate;
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
struct NormalizedToolCall {
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug)]
struct ProviderTurnResult {
    text: String,
    thinking: Option<String>,
    tool_calls: Vec<NormalizedToolCall>,
}

pub fn openai_tool_schemas() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file in the project with optional 1-indexed start_line and end_line.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative or absolute path within workspace" },
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
                "description": "Create a new file or overwrite an existing file with complete contents.",
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
                "description": "Search across project files for code symbols, strings, or definitions.",
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
                "description": "Execute a PowerShell or CMD command in the project workspace and capture stdout/stderr.",
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
                "description": "Install packages using npm, pnpm, yarn, cargo, or pip in the project.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "manager": { "type": "string", "enum": ["npm", "pnpm", "yarn", "cargo", "pip"] },
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
                "description": "Run the project's test suite (auto-detects Cargo.toml, package.json, pytest, or custom command) and return pass/fail output.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Optional explicit test command" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "diagnose_errors",
                "description": "Run compiler/typecheck/lint diagnostics (tsc --noEmit, cargo check, python -m py_compile) on the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "description": "Optional target subpath" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_operation",
                "description": "Run safe Git inspection or staging operations (status, diff, log, branch, add, commit).",
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
    ])
}

fn build_system_prompt(project_root: &Path, mode: &str) -> String {
    let tree = scan_project_tree(project_root, 120);
    let sample_files: Vec<String> = tree
        .iter()
        .take(60)
        .map(|e| {
            if e.is_dir {
                format!("{}/", e.rel_path)
            } else {
                e.rel_path.clone()
            }
        })
        .collect();

    let mode_rules = match mode {
        "plan" => "MODE: PLAN (Read-Only Architecture Mode). Inspect files using read_file, search_project, diagnose_errors, and git_operation, then deliver a thorough step-by-step architectural and file-level implementation plan. Do NOT modify files or execute mutating commands.",
        "ask" => "MODE: ASK (Interactive Approval Mode). Inspect code freely; any file edits or shell executions will prompt the developer for explicit approval.",
        _ => "MODE: AUTONOMOUS (Continuous Execution Loop). Proactively inspect the codebase, implement complete production code across all required files, run diagnose_errors / run_tests to verify your changes, automatically fix any compiler or test failures, and summarize verified results.",
    };

    format!(
        "You are AI Coding Agent, a native Windows x64 desktop autonomous software engineering agent.\n\
         Active Windows Workspace: {}\n\
         {}\n\n\
         Workspace Tree Snapshot:\n{}\n\n\
         Engineering Principles:\n\
         1. Always read relevant files before editing.\n\
         2. Prefer surgical edit_file calls for existing files and write_file for new files.\n\
         3. After making code modifications in autonomous mode, verify compilation or tests with diagnose_errors or run_tests and fix any errors before finishing.",
        project_root.display(),
        mode_rules,
        sample_files.join("\n")
    )
}

fn run_shell_silent(cwd: &Path, command_str: &str) -> Result<(i32, String, String), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", command_str]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command_str]);
        c
    };
    cmd.current_dir(cwd);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to spawn shell command: {}", e))?;
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).chars().take(16_000).collect();
    let stderr = String::from_utf8_lossy(&out.stderr).chars().take(16_000).collect();
    Ok((code, stdout, stderr))
}

async fn execute_single_tool(
    app: &AppHandle,
    gate: &PermissionGate,
    settings: &AppSettings,
    project_root: &Path,
    session_id: &str,
    mode: &str,
    call: &NormalizedToolCall,
) -> (String, ToolExecutionRecord) {
    let started = Instant::now();
    let args_str = serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
    let name = call.name.as_str();

    let is_mutating_file = matches!(name, "write_file" | "edit_file");
    let is_shell = matches!(name, "execute_command" | "install_dependencies" | "run_tests");
    let is_mutating_git = name == "git_operation"
        && matches!(
            call.arguments.get("subcommand").and_then(|v| v.as_str()),
            Some("add") | Some("commit")
        );

    if mode == "plan" && (is_mutating_file || is_shell || is_mutating_git) {
        let msg = format!(
            "Blocked in Plan Mode: '{}' cannot mutate files or run shell commands. Formulate the step-by-step plan using read-only tools.",
            name
        );
        let rec = ToolExecutionRecord {
            id: format!("tool_{}", Uuid::new_v4().simple()),
            session_id: session_id.to_string(),
            tool_name: name.to_string(),
            args_json: args_str,
            result_summary: msg.clone(),
            status: "denied".to_string(),
            duration_ms: started.elapsed().as_millis() as u64,
            diff_text: None,
            file_path: None,
            created_at: Utc::now().to_rfc3339(),
        };
        return (msg, rec);
    }

    // Permission checks
    if mode == "ask" && (is_mutating_file || is_shell || is_mutating_git) {
        let ok = gate
            .request_approval(
                app,
                if is_shell { "shell" } else { "outside_workspace" },
                &format!("Ask Mode Approval: {}", name),
                "Ask Mode requires explicit user confirmation before running mutating tools.",
                if is_shell { Some(args_str.clone()) } else { None },
                call.arguments.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                Some(project_root.to_string_lossy().to_string()),
            )
            .await;
        if !ok {
            let msg = format!("User denied execution of tool '{}'.", name);
            let rec = ToolExecutionRecord {
                id: format!("tool_{}", Uuid::new_v4().simple()),
                session_id: session_id.to_string(),
                tool_name: name.to_string(),
                args_json: args_str,
                result_summary: msg.clone(),
                status: "denied".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
                diff_text: None,
                file_path: None,
                created_at: Utc::now().to_rfc3339(),
            };
            return (msg, rec);
        }
    } else if is_shell && settings.require_shell_approval && name == "execute_command" {
        let cmd_preview = call
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ok = gate
            .request_approval(
                app,
                "shell",
                "Approve Shell Command Execution",
                "Autonomous agent requested PowerShell command execution in your Windows workspace.",
                Some(cmd_preview.to_string()),
                None,
                Some(project_root.to_string_lossy().to_string()),
            )
            .await;
        if !ok {
            let msg = "User denied shell command execution.".to_string();
            let rec = ToolExecutionRecord {
                id: format!("tool_{}", Uuid::new_v4().simple()),
                session_id: session_id.to_string(),
                tool_name: name.to_string(),
                args_json: args_str,
                result_summary: msg.clone(),
                status: "denied".to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
                diff_text: None,
                file_path: None,
                created_at: Utc::now().to_rfc3339(),
            };
            return (msg, rec);
        }
    }

    let mut diff_out: Option<String> = None;
    let mut touched_path: Option<String> = None;
    let mut status = "ok".to_string();

    let output = match name {
        "read_file" => {
            let p = call.arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match resolve_project_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => match std::fs::read_to_string(&resolved) {
                    Ok(content) => {
                        touched_path = Some(resolved.to_string_lossy().to_string());
                        let lines: Vec<&str> = content.lines().collect();
                        let s = call
                            .arguments
                            .get("start_line")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1)
                            .max(1) as usize;
                        let e = call
                            .arguments
                            .get("end_line")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize)
                            .unwrap_or(lines.len().min(s + 350));
                        let slice_end = e.min(lines.len());
                        let mut numbered = String::new();
                        for (idx, line) in lines.iter().enumerate().take(slice_end).skip(s.saturating_sub(1)) {
                            numbered.push_str(&format!("{:>4} | {}\n", idx + 1, line));
                        }
                        format!("File: {} ({} total lines)\n{}", p, lines.len(), numbered)
                    }
                    Err(e) => {
                        status = "error".to_string();
                        format!("Failed to read '{}': {}", p, e)
                    }
                },
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "write_file" => {
            let p = call.arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = call.arguments.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match resolve_project_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => {
                    let old = std::fs::read_to_string(&resolved).unwrap_or_default();
                    if let Some(parent) = resolved.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&resolved, content) {
                        Ok(_) => {
                            touched_path = Some(resolved.to_string_lossy().to_string());
                            diff_out = Some(compute_unified_diff(p, &old, content));
                            let _ = app.emit("fs://changed", json!({ "kind": "write", "path": resolved.to_string_lossy() }));
                            format!("Wrote {} bytes to {}", content.len(), p)
                        }
                        Err(e) => {
                            status = "error".to_string();
                            format!("Failed to write '{}': {}", p, e)
                        }
                    }
                }
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "edit_file" => {
            let p = call.arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let old_text = call.arguments.get("old_text").and_then(|v| v.as_str()).unwrap_or("");
            let new_text = call.arguments.get("new_text").and_then(|v| v.as_str()).unwrap_or("");
            match resolve_project_path(project_root, p, settings.restrict_to_workspace) {
                Ok(resolved) => match std::fs::read_to_string(&resolved) {
                    Ok(existing) => {
                        if !existing.contains(old_text) {
                            status = "error".to_string();
                            format!("old_text was not found in '{}'. Read the file first to verify exact context.", p)
                        } else {
                            let updated = existing.replacen(old_text, new_text, 1);
                            match std::fs::write(&resolved, &updated) {
                                Ok(_) => {
                                    touched_path = Some(resolved.to_string_lossy().to_string());
                                    diff_out = Some(compute_unified_diff(p, &existing, &updated));
                                    let _ = app.emit("fs://changed", json!({ "kind": "edit", "path": resolved.to_string_lossy() }));
                                    format!("Updated '{}' successfully.", p)
                                }
                                Err(e) => {
                                    status = "error".to_string();
                                    format!("Failed writing '{}': {}", p, e)
                                }
                            }
                        }
                    }
                    Err(e) => {
                        status = "error".to_string();
                        format!("Cannot read '{}': {}", p, e)
                    }
                },
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "search_project" => {
            let query = call.arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let cs = call.arguments.get("case_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);
            let hits = search_project_files(project_root, query, cs, 60);
            if hits.is_empty() {
                format!("No matches found for '{}'.", query)
            } else {
                let mut buf = format!("Found {} matches for '{}':\n", hits.len(), query);
                for h in hits {
                    buf.push_str(&format!("{}:{} -> {}\n", h.rel_path, h.line, h.preview));
                }
                buf
            }
        }
        "execute_command" => {
            let cmd_str = call.arguments.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let sub_cwd = call
                .arguments
                .get("cwd")
                .and_then(|v| v.as_str())
                .and_then(|c| resolve_project_path(project_root, c, settings.restrict_to_workspace).ok())
                .unwrap_or_else(|| project_root.to_path_buf());
            match run_shell_silent(&sub_cwd, cmd_str) {
                Ok((code, out, err)) => {
                    if code != 0 {
                        status = "error".to_string();
                    }
                    format!("Exit Code: {}\nSTDOUT:\n{}\nSTDERR:\n{}", code, out, err)
                }
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "install_dependencies" => {
            let mgr = call.arguments.get("manager").and_then(|v| v.as_str()).unwrap_or("npm");
            let pkgs: Vec<String> = call
                .arguments
                .get("packages")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let dev = call.arguments.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
            let pkg_joined = pkgs.join(" ");
            let cmd_str = match mgr {
                "cargo" => {
                    if pkgs.is_empty() {
                        "cargo fetch".to_string()
                    } else {
                        format!("cargo add {}", pkg_joined)
                    }
                }
                "pip" => {
                    if pkgs.is_empty() {
                        "python -m pip install -r requirements.txt".to_string()
                    } else {
                        format!("python -m pip install {}", pkg_joined)
                    }
                }
                "pnpm" => format!("pnpm add {} {}", if dev { "-D" } else { "" }, pkg_joined),
                "yarn" => format!("yarn add {} {}", if dev { "-D" } else { "" }, pkg_joined),
                _ => {
                    if pkgs.is_empty() {
                        "npm install".to_string()
                    } else {
                        format!("npm install {} {}", if dev { "--save-dev" } else { "" }, pkg_joined)
                    }
                }
            };
            match run_shell_silent(project_root, &cmd_str) {
                Ok((code, out, err)) => {
                    if code != 0 {
                        status = "error".to_string();
                    }
                    format!("Command: {}\nExit Code: {}\n{}\n{}", cmd_str, code, out, err)
                }
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "run_tests" => {
            let explicit = call.arguments.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let cmd_str = if !explicit.is_empty() {
                explicit.to_string()
            } else if project_root.join("Cargo.toml").exists() {
                "cargo test".to_string()
            } else if project_root.join("package.json").exists() {
                "npm test -- --watchAll=false".to_string()
            } else {
                "pytest".to_string()
            };
            match run_shell_silent(project_root, &cmd_str) {
                Ok((code, out, err)) => {
                    if code != 0 {
                        status = "error".to_string();
                    }
                    format!("Test Command: {}\nExit Code: {}\nSTDOUT:\n{}\nSTDERR:\n{}", cmd_str, code, out, err)
                }
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        "diagnose_errors" => {
            let mut report = String::new();
            if project_root.join("tsconfig.json").exists() {
                if let Ok((c, o, e)) = run_shell_silent(project_root, "npx tsc --noEmit") {
                    report.push_str(&format!("[TypeScript tsc --noEmit] exit={}\n{}{}\n", c, o, e));
                    if c != 0 {
                        status = "error".to_string();
                    }
                }
            }
            if project_root.join("Cargo.toml").exists() {
                if let Ok((c, o, e)) = run_shell_silent(project_root, "cargo check --message-format=short") {
                    report.push_str(&format!("[Rust cargo check] exit={}\n{}{}\n", c, o, e));
                    if c != 0 {
                        status = "error".to_string();
                    }
                }
            }
            if report.is_empty() {
                report.push_str("No tsconfig.json or Cargo.toml detected at root; run execute_command with your compiler/linter.");
            }
            report
        }
        "git_operation" => {
            let sub = call.arguments.get("subcommand").and_then(|v| v.as_str()).unwrap_or("status");
            let extra: Vec<String> = call
                .arguments
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            let mut argv: Vec<&str> = vec![sub];
            for item in &extra {
                argv.push(item.as_str());
            }
            match run_git(project_root, &argv) {
                Ok((c, o, e)) => {
                    if c != 0 {
                        status = "error".to_string();
                    }
                    format!("git {} -> exit {}\n{}{}", argv.join(" "), c, o, e)
                }
                Err(e) => {
                    status = "error".to_string();
                    e
                }
            }
        }
        other => {
            status = "error".to_string();
            format!("Unknown tool '{}'", other)
        }
    };

    let summary_preview: String = output.lines().take(3).collect::<Vec<_>>().join(" | ").chars().take(220).collect();
    let rec = ToolExecutionRecord {
        id: format!("tool_{}", Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        tool_name: name.to_string(),
        args_json: args_str,
        result_summary: summary_preview,
        status,
        duration_ms: started.elapsed().as_millis() as u64,
        diff_text: diff_out,
        file_path: touched_path,
        created_at: Utc::now().to_rfc3339(),
    };

    (output, rec)
}

async fn call_provider(
    client: &Client,
    settings: &AppSettings,
    provider: &str,
    model: &str,
    system_prompt: &str,
    conversation: &[Value],
) -> Result<ProviderTurnResult, String> {
    match provider {
        "anthropic" => {
            if settings.anthropic_api_key.trim().is_empty() {
                return Err("Anthropic API key is empty. Open Settings in the top-right toolbar and enter your Anthropic key, or switch to Ollama.".to_string());
            }
            let tools_oa = openai_tool_schemas();
            let mut anth_tools = Vec::new();
            if let Some(arr) = tools_oa.as_array() {
                for item in arr {
                    if let Some(f) = item.get("function") {
                        anth_tools.push(json!({
                            "name": f["name"],
                            "description": f["description"],
                            "input_schema": f["parameters"]
                        }));
                    }
                }
            }
            let mut anth_msgs = Vec::new();
            for m in conversation {
                let role = m["role"].as_str().unwrap_or("user");
                if role == "tool" {
                    anth_msgs.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": m["tool_call_id"],
                            "content": m["content"]
                        }]
                    }));
                } else if role == "assistant" {
                    let mut blocks = Vec::new();
                    if let Some(txt) = m.get("content").and_then(|v| v.as_str()) {
                        if !txt.is_empty() {
                            blocks.push(json!({ "type": "text", "text": txt }));
                        }
                    }
                    if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let parsed_input: Value = serde_json::from_str(
                                tc["function"]["arguments"].as_str().unwrap_or("{}"),
                            )
                            .unwrap_or_else(|_| json!({}));
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": tc["id"],
                                "name": tc["function"]["name"],
                                "input": parsed_input
                            }));
                        }
                    }
                    if !blocks.is_empty() {
                        anth_msgs.push(json!({ "role": "assistant", "content": blocks }));
                    }
                } else {
                    anth_msgs.push(json!({
                        "role": "user",
                        "content": m["content"].as_str().unwrap_or("")
                    }));
                }
            }

            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", settings.anthropic_api_key.trim())
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": model,
                    "max_tokens": 4096,
                    "system": system_prompt,
                    "tools": anth_tools,
                    "messages": anth_msgs
                }))
                .send()
                .await
                .map_err(|e| format!("Anthropic HTTP error: {}", e))?;

            let status = resp.status();
            let body: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("Anthropic API Error ({}): {}", status, body));
            }
            let mut text = String::new();
            let mut thinking = None;
            let mut tool_calls = Vec::new();
            if let Some(arr) = body.get("content").and_then(|v| v.as_array()) {
                for block in arr {
                    match block["type"].as_str().unwrap_or("") {
                        "text" => {
                            if let Some(t) = block["text"].as_str() {
                                text.push_str(t);
                            }
                        }
                        "thinking" => {
                            if let Some(t) = block["thinking"].as_str() {
                                thinking = Some(t.to_string());
                            }
                        }
                        "tool_use" => {
                            tool_calls.push(NormalizedToolCall {
                                id: block["id"].as_str().unwrap_or("call_1").to_string(),
                                name: block["name"].as_str().unwrap_or("").to_string(),
                                arguments: block["input"].clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            Ok(ProviderTurnResult {
                text,
                thinking,
                tool_calls,
            })
        }
        "gemini" => {
            if settings.gemini_api_key.trim().is_empty() {
                return Err("Gemini API key is empty. Open Settings and configure your Google Gemini API key.".to_string());
            }
            let tools_oa = openai_tool_schemas();
            let mut decls = Vec::new();
            if let Some(arr) = tools_oa.as_array() {
                for item in arr {
                    if let Some(f) = item.get("function") {
                        decls.push(json!({
                            "name": f["name"],
                            "description": f["description"],
                            "parameters": f["parameters"]
                        }));
                    }
                }
            }
            let mut contents = Vec::new();
            for m in conversation {
                let role = m["role"].as_str().unwrap_or("user");
                if role == "tool" {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": m["name"].as_str().unwrap_or("tool"),
                                "response": { "result": m["content"] }
                            }
                        }]
                    }));
                } else if role == "assistant" {
                    let mut parts = Vec::new();
                    if let Some(t) = m.get("content").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            parts.push(json!({ "text": t }));
                        }
                    }
                    if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let args_val: Value = serde_json::from_str(
                                tc["function"]["arguments"].as_str().unwrap_or("{}"),
                            )
                            .unwrap_or_else(|_| json!({}));
                            parts.push(json!({
                                "functionCall": {
                                    "name": tc["function"]["name"],
                                    "args": args_val
                                }
                            }));
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(json!({ "role": "model", "parts": parts }));
                    }
                } else {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{ "text": m["content"].as_str().unwrap_or("") }]
                    }));
                }
            }

            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                model,
                settings.gemini_api_key.trim()
            );
            let resp = client
                .post(&url)
                .json(&json!({
                    "systemInstruction": { "parts": [{ "text": system_prompt }] },
                    "contents": contents,
                    "tools": [{ "functionDeclarations": decls }]
                }))
                .send()
                .await
                .map_err(|e| format!("Gemini HTTP error: {}", e))?;

            let status = resp.status();
            let body: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("Gemini API Error ({}): {}", status, body));
            }
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            if let Some(parts) = body["candidates"][0]["content"]["parts"].as_array() {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                    if let Some(fc) = p.get("functionCall") {
                        tool_calls.push(NormalizedToolCall {
                            id: format!("gem_{}", Uuid::new_v4().simple()),
                            name: fc["name"].as_str().unwrap_or("").to_string(),
                            arguments: fc["args"].clone(),
                        });
                    }
                }
            }
            Ok(ProviderTurnResult {
                text,
                thinking: None,
                tool_calls,
            })
        }
        _ => {
            // OpenAI-compatible protocol: openai, openrouter, ollama, custom
            let (endpoint, api_key) = match provider {
                "openai" => {
                    if settings.openai_api_key.trim().is_empty() {
                        return Err("OpenAI API key is empty. Configure it in Settings or select local Ollama.".to_string());
                    }
                    (
                        "https://api.openai.com/v1/chat/completions".to_string(),
                        settings.openai_api_key.trim().to_string(),
                    )
                }
                "openrouter" => {
                    if settings.openrouter_api_key.trim().is_empty() {
                        return Err("OpenRouter API key is empty. Configure it in Settings.".to_string());
                    }
                    (
                        "https://openrouter.ai/api/v1/chat/completions".to_string(),
                        settings.openrouter_api_key.trim().to_string(),
                    )
                }
                "custom" => {
                    let base = settings.custom_base_url.trim().trim_end_matches('/');
                    if base.is_empty() {
                        return Err("Custom Base URL is empty in Settings.".to_string());
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

            let mut messages = vec![json!({ "role": "system", "content": system_prompt })];
            messages.extend_from_slice(conversation);

            let mut req = client.post(&endpoint).json(&json!({
                "model": model,
                "messages": messages,
                "tools": openai_tool_schemas(),
                "tool_choice": "auto"
            }));
            if !api_key.is_empty() {
                req = req.bearer_auth(api_key);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Provider '{}' connection error ({}): {}", provider, endpoint, e))?;

            let status = resp.status();
            let body: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("Provider '{}' HTTP {}: {}", provider, status, body));
            }
            let msg = &body["choices"][0]["message"];
            let text = msg["content"].as_str().unwrap_or("").to_string();
            let thinking = msg
                .get("reasoning_content")
                .or_else(|| msg.get("reasoning"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut tool_calls = Vec::new();
            if let Some(arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in arr {
                    let raw_args = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let parsed_args =
                        serde_json::from_str::<Value>(raw_args).unwrap_or_else(|_| json!({}));
                    tool_calls.push(NormalizedToolCall {
                        id: tc["id"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple())),
                        name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                        arguments: parsed_args,
                    });
                }
            }
            Ok(ProviderTurnResult {
                text,
                thinking,
                tool_calls,
            })
        }
    }
}

pub async fn run_autonomous_loop(
    app: AppHandle,
    store: &SqliteStore,
    gate: &PermissionGate,
    project_root: &Path,
    session_id: &str,
    prompt: &str,
    mode: &str,
    provider_override: Option<String>,
    model_override: Option<String>,
) -> Result<ChatMessage, String> {
    gate.reset_stop();
    let settings = store.load_settings();
    let provider = provider_override.unwrap_or_else(|| settings.active_provider.clone());
    let model = model_override.unwrap_or_else(|| settings.active_model.clone());

    let user_msg = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        role: "user".to_string(),
        content: prompt.to_string(),
        thinking: None,
        mode: mode.to_string(),
        provider: Some(provider.clone()),
        model: Some(model.clone()),
        tools: Vec::new(),
        created_at: Utc::now().to_rfc3339(),
    };
    store.append_message(&user_msg);

    let mut conv: Vec<Value> = Vec::new();
    if let Some(detail) = store.load_session(session_id) {
        for m in detail.messages.iter().rev().take(24).rev() {
            if m.role == "user" || m.role == "assistant" {
                conv.push(json!({ "role": m.role, "content": m.content }));
            }
        }
    }

    let system_prompt = build_system_prompt(project_root, mode);
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let max_iters = settings.max_agent_iterations.clamp(1, 30);
    let mut collected_tools: Vec<ToolExecutionRecord> = Vec::new();
    let mut final_text = String::new();
    let mut final_thinking: Option<String> = None;

    for step in 1..=max_iters {
        if gate.is_stopped() {
            break;
        }

        let _ = app.emit(
            "agent://stream",
            json!({
                "sessionId": session_id,
                "kind": "status",
                "statusText": format!("Step {}/{}: Querying {} ({})", step, max_iters, provider, model)
            }),
        );

        let turn = match call_provider(&client, &settings, &provider, &model, &system_prompt, &conv).await {
            Ok(t) => t,
            Err(err) => {
                let _ = app.emit(
                    "agent://stream",
                    json!({
                        "sessionId": session_id,
                        "kind": "error",
                        "text": err
                    }),
                );
                let _ = app.emit(
                    "agent://stream",
                    json!({
                        "sessionId": session_id,
                        "kind": "done",
                        "session": store.load_session(session_id)
                    }),
                );
                return Err(err);
            }
        };

        if let Some(ref th) = turn.thinking {
            final_thinking = Some(th.clone());
            let _ = app.emit(
                "agent://stream",
                json!({
                    "sessionId": session_id,
                    "kind": "thinking",
                    "text": th
                }),
            );
        }

        if !turn.text.is_empty() {
            if !final_text.is_empty() {
                final_text.push_str("\n\n");
            }
            final_text.push_str(&turn.text);
            let _ = app.emit(
                "agent://stream",
                json!({
                    "sessionId": session_id,
                    "kind": "text",
                    "text": turn.text
                }),
            );
        }

        if turn.tool_calls.is_empty() {
            break;
        }

        // Record assistant tool_calls turn in conversation history
        let oa_tcs: Vec<Value> = turn
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                    }
                })
            })
            .collect();
        conv.push(json!({
            "role": "assistant",
            "content": turn.text,
            "tool_calls": oa_tcs
        }));

        for tc in &turn.tool_calls {
            if gate.is_stopped() {
                break;
            }
            let pending_rec = ToolExecutionRecord {
                id: tc.id.clone(),
                session_id: session_id.to_string(),
                tool_name: tc.name.clone(),
                args_json: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                result_summary: "Running...".to_string(),
                status: "running".to_string(),
                duration_ms: 0,
                diff_text: None,
                file_path: None,
                created_at: Utc::now().to_rfc3339(),
            };
            let _ = app.emit(
                "agent://stream",
                json!({
                    "sessionId": session_id,
                    "kind": "tool_start",
                    "tool": pending_rec
                }),
            );

            let (tool_output, finished_rec) = execute_single_tool(
                &app,
                gate,
                &settings,
                project_root,
                session_id,
                mode,
                tc,
            )
            .await;

            store.record_tool_execution(&finished_rec);
            let _ = app.emit(
                "agent://stream",
                json!({
                    "sessionId": session_id,
                    "kind": "tool_end",
                    "tool": finished_rec
                }),
            );
            collected_tools.push(finished_rec);

            conv.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "name": tc.name,
                "content": tool_output
            }));
        }
    }

    if final_text.trim().is_empty() {
        final_text = if gate.is_stopped() {
            "Execution halted immediately via Emergency Stop.".to_string()
        } else {
            "Completed tool execution cycle.".to_string()
        };
    }

    let assistant_msg = ChatMessage {
        id: format!("msg_{}", Uuid::new_v4().simple()),
        session_id: session_id.to_string(),
        role: "assistant".to_string(),
        content: final_text,
        thinking: final_thinking,
        mode: mode.to_string(),
        provider: Some(provider),
        model: Some(model),
        tools: collected_tools,
        created_at: Utc::now().to_rfc3339(),
    };
    store.append_message(&assistant_msg);

    let _ = app.emit(
        "agent://stream",
        json!({
            "sessionId": session_id,
            "kind": "done",
            "session": store.load_session(session_id)
        }),
    );

    Ok(assistant_msg)
}
