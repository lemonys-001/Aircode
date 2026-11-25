//! OpenCode AI Server Auth (with 3-mode Workaround) & Helpful Guided Provider Setup.
//!
//! Supports:
//! 1. `zen-public` / `zen-key`: Direct OpenCode Zen gateway (`https://opencode.ai/zen/v1`)
//!    with OpenCode client headers (`public` free tier workaround + Zen API tokens).
//! 2. `auth-json` auto-import: Reads existing `~/.local/share/opencode/auth.json` (or Windows
//!    `%USERPROFILE%\.local\share\opencode\auth.json`) tokens from `opencode auth login`.
//! 3. `local-serve` bridge workaround: Connects to (or spawns) `opencode serve` on
//!    `http://127.0.0.1:4096` so OAuth / Copilot / Claude Max sessions work seamlessly.

use crate::json::JsonValue;
use crate::platform::http_request;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug, Default)]
pub struct DiscoveredAuth {
    pub opencode_zen_key: Option<String>,
    pub anthropic_key: Option<String>,
    pub openai_key: Option<String>,
    pub gemini_key: Option<String>,
    pub openrouter_key: Option<String>,
    pub opencode_auth_file: Option<PathBuf>,
    pub local_opencode_server_alive: bool,
    pub local_ollama_alive: bool,
    pub sources_summary: Vec<String>,
}

pub struct ProviderGuide {
    pub id: &'static str,
    pub display_name: &'static str,
    pub badge: &'static str,
    pub tagline: &'static str,
    pub key_url: &'static str,
    pub needs_key: bool,
}

pub const PROVIDER_GUIDES: &[ProviderGuide] = &[
    ProviderGuide {
        id: "opencode",
        display_name: "OpenCode Zen & Server",
        badge: "RECOMMENDED",
        tagline: "Zero-setup free Zen models (`public` workaround), `auth.json` import, or local `opencode serve` bridge.",
        key_url: "https://opencode.ai/auth",
        needs_key: false,
    },
    ProviderGuide {
        id: "anthropic",
        display_name: "Anthropic Claude",
        badge: "TOP CODER",
        tagline: "Claude Sonnet 4 / Opus 4 with extended thinking & deep tool use.",
        key_url: "https://console.anthropic.com/settings/keys",
        needs_key: true,
    },
    ProviderGuide {
        id: "openai",
        display_name: "OpenAI",
        badge: "FAST + O-SERIES",
        tagline: "GPT-4.1, GPT-4o, and o4-mini with configurable reasoning effort.",
        key_url: "https://platform.openai.com/api-keys",
        needs_key: true,
    },
    ProviderGuide {
        id: "openrouter",
        display_name: "OpenRouter",
        badge: "200+ MODELS",
        tagline: "One key for Claude, DeepSeek R1/V3, Qwen3-Coder, Gemini & Llama.",
        key_url: "https://openrouter.ai/keys",
        needs_key: true,
    },
    ProviderGuide {
        id: "gemini",
        display_name: "Google Gemini",
        badge: "1M CONTEXT",
        tagline: "Gemini 2.5 Pro & 2.5 Flash via Google AI Studio key.",
        key_url: "https://aistudio.google.com/app/apikey",
        needs_key: true,
    },
    ProviderGuide {
        id: "ollama",
        display_name: "Ollama (Local Offline)",
        badge: "100% LOCAL",
        tagline: "Run Qwen2.5-Coder, DeepSeek-Coder, or Llama locally with zero API keys.",
        key_url: "http://127.0.0.1:11434",
        needs_key: false,
    },
    ProviderGuide {
        id: "custom",
        display_name: "Custom OpenAI-Compatible",
        badge: "SELF-HOSTED",
        tagline: "LM Studio, vLLM, llama.cpp, LiteLLM, or enterprise gateway.",
        key_url: "http://127.0.0.1:1234/v1",
        needs_key: false,
    },
];

pub fn candidate_opencode_auth_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        paths.push(PathBuf::from(xdg).join("opencode").join("auth.json"));
    }
    if let Ok(home) = std::env::var("HOME") {
        let h = PathBuf::from(home);
        paths.push(h.join(".local/share/opencode/auth.json"));
        paths.push(h.join(".opencode/auth.json"));
    }
    if let Ok(up) = std::env::var("USERPROFILE") {
        let u = PathBuf::from(up);
        paths.push(u.join(".local").join("share").join("opencode").join("auth.json"));
        paths.push(u.join(".opencode").join("auth.json"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("opencode").join("auth.json"));
    }
    paths
}

fn extract_token_entry(v: &JsonValue) -> Option<String> {
    if let Some(s) = v.as_str() {
        if !s.trim().is_empty() {
            return Some(s.trim().to_string());
        }
    }
    for field in ["key", "apiKey", "access", "token", "accessToken"] {
        let val = v.string_or(field, "");
        if !val.trim().is_empty() {
            return Some(val.trim().to_string());
        }
    }
    None
}

pub fn discover_local_credentials(opencode_server_url: &str, ollama_url: &str) -> DiscoveredAuth {
    let mut out = DiscoveredAuth::default();

    // 1. Check Environment Variables
    if let Ok(k) = std::env::var("OPENCODE_API_KEY") {
        if !k.trim().is_empty() {
            out.opencode_zen_key = Some(k.trim().to_string());
            out.sources_summary.push("Env: OPENCODE_API_KEY".into());
        }
    }
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.trim().is_empty() {
            out.anthropic_key = Some(k.trim().to_string());
            out.sources_summary.push("Env: ANTHROPIC_API_KEY".into());
        }
    }
    if let Ok(k) = std::env::var("OPENAI_API_KEY") {
        if !k.trim().is_empty() {
            out.openai_key = Some(k.trim().to_string());
            out.sources_summary.push("Env: OPENAI_API_KEY".into());
        }
    }
    if let Ok(k) = std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
        if !k.trim().is_empty() {
            out.gemini_key = Some(k.trim().to_string());
            out.sources_summary.push("Env: GEMINI_API_KEY".into());
        }
    }
    if let Ok(k) = std::env::var("OPENROUTER_API_KEY") {
        if !k.trim().is_empty() {
            out.openrouter_key = Some(k.trim().to_string());
            out.sources_summary.push("Env: OPENROUTER_API_KEY".into());
        }
    }

    // 2. Check OpenCode CLI `auth.json`
    for path in candidate_opencode_auth_paths() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = JsonValue::parse(&raw) else {
            continue;
        };
        out.opencode_auth_file = Some(path.clone());
        if let Some(v) = json.get("opencode").or_else(|| json.get("zen")) {
            if let Some(tok) = extract_token_entry(v) {
                out.opencode_zen_key.get_or_insert(tok);
                out.sources_summary.push("opencode/auth.json (Zen)".into());
            }
        }
        if let Some(v) = json.get("anthropic") {
            if let Some(tok) = extract_token_entry(v) {
                out.anthropic_key.get_or_insert(tok);
                out.sources_summary
                    .push("opencode/auth.json (Anthropic)".into());
            }
        }
        if let Some(v) = json.get("openai") {
            if let Some(tok) = extract_token_entry(v) {
                out.openai_key.get_or_insert(tok);
                out.sources_summary
                    .push("opencode/auth.json (OpenAI)".into());
            }
        }
        if let Some(v) = json.get("openrouter") {
            if let Some(tok) = extract_token_entry(v) {
                out.openrouter_key.get_or_insert(tok);
                out.sources_summary
                    .push("opencode/auth.json (OpenRouter)".into());
            }
        }
        if let Some(v) = json.get("google").or_else(|| json.get("gemini")) {
            if let Some(tok) = extract_token_entry(v) {
                out.gemini_key.get_or_insert(tok);
                out.sources_summary
                    .push("opencode/auth.json (Gemini)".into());
            }
        }
        break;
    }

    // 3. Probe local `opencode serve` (quick 1.2s timeout)
    let srv = opencode_server_url.trim_end_matches('/');
    if !srv.is_empty() {
        let r = http_request("GET", &format!("{}/session", srv), &[], "", 1200);
        if r.error.is_none() && (200..500).contains(&r.status) {
            out.local_opencode_server_alive = true;
            out.sources_summary
                .push(format!("Local opencode server ({})", srv));
        }
    }

    // 4. Probe local Ollama
    let oll = ollama_url.trim_end_matches('/');
    if !oll.is_empty() {
        let r = http_request("GET", &format!("{}/api/tags", oll), &[], "", 1200);
        if r.error.is_none() && r.status == 200 {
            out.local_ollama_alive = true;
            out.sources_summary.push("Local Ollama online".into());
        }
    }

    out
}

/// Spawns `opencode serve --port 4096` in the background as a local auth workaround bridge
/// if the user has the `opencode` CLI installed.
pub fn try_spawn_local_opencode_server(port: u16) -> Result<String, String> {
    let mut cmd = Command::new("opencode");
    cmd.args(["serve", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| {
        format!(
            "Could not spawn `opencode serve`: {} (Use OpenCode Zen Public/Key mode instead!)",
            e
        )
    })?;
    Ok(format!("http://127.0.0.1:{}", port))
}

/// Calls a running `opencode serve` HTTP server (`http://127.0.0.1:4096`) as an authenticated
/// workaround bridge, reusing whatever OAuth/Max/Copilot session `opencode` holds.
pub fn call_local_opencode_server(
    base_url: &str,
    model_spec: &str,
    system_prompt: &str,
    prompt_text: &str,
) -> Result<String, String> {
    let base = base_url.trim_end_matches('/');
    let sess_resp = http_request(
        "POST",
        &format!("{}/session", base),
        &[("Content-Type", "application/json")],
        "{}",
        10_000,
    );
    if let Some(e) = sess_resp.error {
        return Err(format!("opencode server unreachable at {}: {}", base, e));
    }
    let sess_json = JsonValue::parse(&sess_resp.body).unwrap_or(JsonValue::Null);
    let sid = sess_json.string_or("id", "");
    if sid.is_empty() {
        return Err(format!(
            "Unexpected response from opencode server ({}): {}",
            sess_resp.status, sess_resp.body
        ));
    }

    let (prov_id, mod_id) = model_spec.split_once('/').unwrap_or(("opencode", model_spec));
    let payload = JsonValue::obj(vec![
        (
            "model",
            JsonValue::obj(vec![
                ("providerID", prov_id.into()),
                ("modelID", mod_id.into()),
            ]),
        ),
        ("system", system_prompt.into()),
        (
            "parts",
            JsonValue::arr(vec![JsonValue::obj(vec![
                ("type", "text".into()),
                ("text", prompt_text.into()),
            ])]),
        ),
    ])
    .to_json_string();

    let msg_resp = http_request(
        "POST",
        &format!("{}/session/{}/message", base, sid),
        &[("Content-Type", "application/json")],
        &payload,
        120_000,
    );
    if let Some(e) = msg_resp.error {
        return Err(e);
    }
    let parsed = JsonValue::parse(&msg_resp.body)
        .map_err(|e| format!("Invalid opencode server JSON: {}", e))?;
    if let Some(parts) = parsed.get("parts").and_then(|p| p.as_array()) {
        let mut out = String::new();
        for p in parts {
            let t = p.string_or("text", "");
            if !t.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t);
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    Ok(msg_resp.body)
}
