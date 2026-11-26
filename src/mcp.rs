//! Pure-Rust Model Context Protocol (MCP `2024-11-05`) JSON-RPC 2.0 Client & Server Registry.
//! Supports both `stdio` subprocess MCP servers and `http` JSON-RPC MCP endpoints.

use crate::json::JsonValue;
use crate::platform::http_request;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug)]
pub struct McpToolSpec {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub input_schema: JsonValue,
}

#[derive(Clone, Debug)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: String, // "stdio" or "http"
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub enabled: bool,
    pub status: String,
    pub discovered_tools: Vec<McpToolSpec>,
}

impl McpServerConfig {
    pub fn to_json(&self) -> JsonValue {
        JsonValue::obj(vec![
            ("name", self.name.clone().into()),
            ("transport", self.transport.clone().into()),
            ("command", self.command.clone().into()),
            (
                "args",
                JsonValue::arr(self.args.iter().cloned().map(JsonValue::from).collect()),
            ),
            ("url", self.url.clone().into()),
            ("enabled", self.enabled.into()),
        ])
    }

    pub fn from_json(v: &JsonValue) -> Self {
        let args = v
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|i| i.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            name: v.string_or("name", "mcp-server"),
            transport: v.string_or("transport", "stdio"),
            command: v.string_or("command", ""),
            args,
            url: v.string_or("url", ""),
            enabled: v.bool_or("enabled", true),
            status: "idle".into(),
            discovered_tools: Vec::new(),
        }
    }
}

pub fn load_mcp_configs(workspace: &Path, app_data_dir: &Path) -> Vec<McpServerConfig> {
    let mut out = Vec::new();
    for path in [
        app_data_dir.join("mcp.json"),
        workspace.join(".aircode").join("mcp.json"),
    ] {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = JsonValue::parse(&raw) else {
            continue;
        };
        if let Some(arr) = json.get("servers").and_then(|s| s.as_array()) {
            for item in arr {
                let cfg = McpServerConfig::from_json(item);
                if let Some(existing) = out.iter_mut().find(|c: &&mut McpServerConfig| c.name == cfg.name) {
                    *existing = cfg;
                } else {
                    out.push(cfg);
                }
            }
        } else if let Some(JsonValue::Object(map)) = json.get("mcpServers") {
            // Also compatible with Claude Desktop / Cursor / OpenCode `mcpServers` map format!
            for (name, val) in map {
                let mut cfg = McpServerConfig::from_json(val);
                cfg.name = name.clone();
                if !cfg.url.is_empty() && cfg.command.is_empty() {
                    cfg.transport = "http".into();
                }
                if let Some(existing) = out.iter_mut().find(|c| c.name == cfg.name) {
                    *existing = cfg;
                } else {
                    out.push(cfg);
                }
            }
        }
    }
    out
}

pub fn save_workspace_mcp_configs(workspace: &Path, servers: &[McpServerConfig]) -> Result<(), String> {
    let dir = workspace.join(".aircode");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let payload = JsonValue::obj(vec![(
        "servers",
        JsonValue::arr(servers.iter().map(|s| s.to_json()).collect()),
    )]);
    std::fs::write(dir.join("mcp.json"), payload.to_json_string().as_bytes())
        .map_err(|e| e.to_string())
}

pub fn probe_mcp_server_tools(
    cfg: &mut McpServerConfig,
    workspace: &Path,
) -> Result<Vec<McpToolSpec>, String> {
    if !cfg.enabled {
        cfg.status = "disabled".into();
        return Ok(Vec::new());
    }
    let result = if cfg.transport == "http" {
        mcp_http_rpc(&cfg.url, "tools/list", JsonValue::obj(vec![]))
    } else {
        mcp_stdio_rpc(cfg, workspace, "tools/list", JsonValue::obj(vec![]))
    };

    match result {
        Ok(res) => {
            let mut specs = Vec::new();
            if let Some(tools) = res.get("tools").and_then(|t| t.as_array()) {
                for t in tools {
                    specs.push(McpToolSpec {
                        server_name: cfg.name.clone(),
                        tool_name: t.string_or("name", "tool"),
                        description: t.string_or("description", ""),
                        input_schema: t
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or_else(|| JsonValue::obj(vec![("type", "object".into())])),
                    });
                }
            }
            cfg.status = format!("online ({} tools)", specs.len());
            cfg.discovered_tools = specs.clone();
            Ok(specs)
        }
        Err(err) => {
            cfg.status = format!("error: {}", err);
            Err(err)
        }
    }
}

pub fn call_mcp_tool(
    servers: &[McpServerConfig],
    workspace: &Path,
    server_name: &str,
    tool_name: &str,
    arguments: JsonValue,
) -> Result<String, String> {
    let cfg = servers
        .iter()
        .find(|s| s.name == server_name && s.enabled)
        .ok_or_else(|| format!("MCP server '{}' not found or disabled", server_name))?;

    let params = JsonValue::obj(vec![
        ("name", tool_name.into()),
        ("arguments", arguments),
    ]);

    let res = if cfg.transport == "http" {
        mcp_http_rpc(&cfg.url, "tools/call", params)?
    } else {
        mcp_stdio_rpc(cfg, workspace, "tools/call", params)?
    };

    if let Some(content) = res.get("content").and_then(|c| c.as_array()) {
        let mut texts = Vec::new();
        for item in content {
            let t = item.string_or("text", "");
            if !t.is_empty() {
                texts.push(t);
            } else {
                texts.push(item.to_json_string());
            }
        }
        return Ok(texts.join("\n"));
    }
    Ok(res.to_json_string())
}

fn mcp_http_rpc(url: &str, method: &str, params: JsonValue) -> Result<JsonValue, String> {
    if url.trim().is_empty() {
        return Err("Empty MCP HTTP URL".into());
    }
    let body = JsonValue::obj(vec![
        ("jsonrpc", "2.0".into()),
        ("id", 1i64.into()),
        ("method", method.into()),
        ("params", params),
    ])
    .to_json_string();

    let resp = http_request(
        "POST",
        url,
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ],
        &body,
        20_000,
    );
    if let Some(err) = resp.error {
        return Err(err);
    }
    if resp.status >= 400 || resp.status == 0 {
        return Err(format!("HTTP {}", resp.status));
    }
    let parsed = parse_json_or_sse(&resp.body)?;
    if let Some(err_obj) = parsed.get("error") {
        return Err(err_obj.to_json_string());
    }
    Ok(parsed.get("result").cloned().unwrap_or(JsonValue::Null))
}

fn parse_json_or_sse(raw: &str) -> Result<JsonValue, String> {
    if let Ok(v) = JsonValue::parse(raw.trim()) {
        return Ok(v);
    }
    for line in raw.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            if let Ok(v) = JsonValue::parse(data.trim()) {
                return Ok(v);
            }
        }
    }
    Err("Invalid JSON-RPC response from MCP server".into())
}

fn mcp_stdio_rpc(
    cfg: &McpServerConfig,
    workspace: &Path,
    method: &str,
    params: JsonValue,
) -> Result<JsonValue, String> {
    if cfg.command.trim().is_empty() {
        return Err("Empty MCP command".into());
    }

    let mut cmd = Command::new(&cfg.command);
    cmd.args(&cfg.args)
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed spawning MCP '{}': {}", cfg.command, e))?;

    let mut stdin = child.stdin.take().ok_or("No MCP stdin")?;
    let stdout = child.stdout.take().ok_or("No MCP stdout")?;

    let init_req = JsonValue::obj(vec![
        ("jsonrpc", "2.0".into()),
        ("id", 1i64.into()),
        ("method", "initialize".into()),
        (
            "params",
            JsonValue::obj(vec![
                ("protocolVersion", "2024-11-05".into()),
                ("capabilities", JsonValue::obj(vec![])),
                (
                    "clientInfo",
                    JsonValue::obj(vec![
                        ("name", "aircode-native-rust".into()),
                        ("version", "2.2.0".into()),
                    ]),
                ),
            ]),
        ),
    ])
    .to_json_string();

    let initialized_notif = JsonValue::obj(vec![
        ("jsonrpc", "2.0".into()),
        ("method", "notifications/initialized".into()),
    ])
    .to_json_string();

    let target_req = JsonValue::obj(vec![
        ("jsonrpc", "2.0".into()),
        ("id", 2i64.into()),
        ("method", method.into()),
        ("params", params),
    ])
    .to_json_string();

    let (tx, rx) = mpsc::channel::<Result<JsonValue, String>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        let _ = writeln!(stdin, "{}", init_req);
        let _ = stdin.flush();

        // Wait for id == 1 initialize response
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(Err("MCP server closed stdout during initialize".into()));
                    return;
                }
                Ok(_) => {
                    if let Ok(v) = JsonValue::parse(line.trim()) {
                        if v.i64_or("id", 0) == 1 {
                            break;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            }
        }

        let _ = writeln!(stdin, "{}", initialized_notif);
        let _ = writeln!(stdin, "{}", target_req);
        let _ = stdin.flush();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(Err("MCP server exited before responding".into()));
                    return;
                }
                Ok(_) => {
                    if let Ok(v) = JsonValue::parse(line.trim()) {
                        if v.i64_or("id", 0) == 2 {
                            if let Some(err_obj) = v.get("error") {
                                let _ = tx.send(Err(err_obj.to_json_string()));
                            } else {
                                let _ = tx.send(Ok(v
                                    .get("result")
                                    .cloned()
                                    .unwrap_or(JsonValue::Null)));
                            }
                            return;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            }
        }
    });

    let outcome = rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| format!("Timed out waiting for MCP server '{}'", cfg.name))?;
    let _ = child.kill();
    outcome
}
