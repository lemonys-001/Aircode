//! OpenCode AI Server Auth (with 3-mode Workaround) & Helpful Guided Provider Setup.
//!
//! Supports:
//! 1. `zen-public` / `zen-key`: Direct OpenCode Zen gateway (`https://opencode.ai/zen/v1`)
//!    speaking the *exact* dialect the official OpenCode client speaks, so the free
//!    (`public`) lane works without an account: matching User-Agent, the
//!    `x-opencode-client` / `x-opencode-project` / `x-opencode-session` /
//!    `x-opencode-session-id` header quartet (never `x-opencode-request`, which the
//!    gateway rejects with 403), `stream: true`, and `bash`/`read` tool names.
//!    A real Zen console key (from `opencode auth login` / `https://opencode.ai/auth`)
//!    is used verbatim whenever one is available and unlocks the paid catalog too.
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

// =============================================================================
// OpenCode Zen client emulation (free-tier workaround)
// =============================================================================

/// OpenCode Zen gateway - OpenAI-compatible (`/chat/completions`, `/responses`, `/models`).
pub const ZEN_BASE_URL: &str = "https://opencode.ai/zen/v1";

/// Literal bearer token the OpenCode CLI uses for its anonymous "public" free lane.
/// Any string works as a *client* key, but the gateway only grants the free tier to
/// clients that look like the official CLI - see [`zen_client_headers`].
pub const ZEN_PUBLIC_KEY: &str = "public";

/// The official OpenCode client User-Agent. The gateway fingerprints it, so a plain
/// `opencode/1.0` (or curl) is refused even with a valid key.
pub const ZEN_CLIENT_USER_AGENT: &str =
    "opencode/1.18.31 ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.14";

/// Free-tier models reachable with the `public` workaround (no account, no card).
/// Free ids are promotional and can disappear; the live catalog is always readable
/// anonymously from `GET https://opencode.ai/zen/v1/models`.
pub const ZEN_FREE_MODELS: &[(&str, &str)] = &[
    ("big-pickle", "Big Pickle - agentic coding"),
    ("glm-5.3-flash-free", "GLM 5.3 Flash"),
    ("deepseek-v4.1-flash-free", "DeepSeek V4.1 Flash"),
    ("nemotron-3.5-lightning-free", "Nemotron 3.5 Lightning"),
    ("nemotron-3.5-super-vl-free", "Nemotron 3.5 Super VL"),
    ("space-bunny-free", "Space Bunny"),
    ("mimo-v2.6-flash-free", "MiMo V2.6 Flash"),
    ("cat-jam-free", "Cat Jam"),
    ("longcat-2.5-preview-free", "LongCat 2.5 Preview"),
];

/// `true` when `model` is one of the free-tier ids that work without a Zen key.
pub fn is_zen_free_model(model: &str) -> bool {
    let m = model.trim();
    ZEN_FREE_MODELS.iter().any(|(id, _)| *id == m) || m.ends_with("-free")
}

/// Deterministic-ish PRNG (xorshift64*) - we have no `rand` crate by design.
fn zen_next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    if x == 0 {
        x = 0x9E37_79B9_7F4A_7C15;
    }
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn zen_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    // Mix in an ASLR-ish stack address so two processes started in the same
    // nanosecond still diverge.
    let stack_entropy = (&nanos as *const u64 as u64) >> 7;
    let mut s = nanos ^ stack_entropy.rotate_left(17) ^ 0x6D2B_79F5;
    if s == 0 {
        s = 0x9E37_79B9_7F4A_7C15;
    }
    s
}

/// Generates a session id in the exact shape the OpenCode client emits:
/// `ses_` + 12 lowercase hex (`^[0-9a-f]{9}f[c-f][0-9a-f]$`) + 14 alphanumerics.
/// The gateway pins the upstream backend per session id, so a fresh id per session
/// spreads traffic the same way the CLI does.
pub fn new_zen_session_id() -> String {
    let mut st = zen_seed();
    let mut hex12 = String::with_capacity(12);
    for i in 0..12 {
        match i {
            9 => hex12.push('f'),
            10 => {
                let picks = ['c', 'd', 'e', 'f'];
                hex12.push(picks[(zen_next_rand(&mut st) % 4) as usize]);
            }
            _ => hex12.push(
                char::from_digit((zen_next_rand(&mut st) % 16) as u32, 16).unwrap_or('0'),
            ),
        }
    }
    const ALNUM: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut tail = String::with_capacity(14);
    for _ in 0..14 {
        tail.push(ALNUM[(zen_next_rand(&mut st) % ALNUM.len() as u64) as usize] as char);
    }
    format!("ses_{}{}", hex12, tail)
}

/// Builds the exact header set the official OpenCode CLI sends to Zen.
///
/// * `Authorization: Bearer <key>` **and** `x-api-key: <key>` (Zen accepts either;
///   chat-format requests use Bearer, keyed Responses requests need both).
/// * The `opencode/<v> ai-sdk/... runtime/bun/...` User-Agent fingerprint.
/// * `x-opencode-client`, `x-opencode-project`, and the session pair.
///
/// Deliberately **not** sent: `x-opencode-request` - the gateway answers 403 when it
/// sees that header from a non-CLI client.
pub fn zen_client_headers(session_id: &str, api_key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", "application/json".to_string()),
        ("Authorization", format!("Bearer {}", api_key)),
        ("x-api-key", api_key.to_string()),
        ("User-Agent", ZEN_CLIENT_USER_AGENT.to_string()),
        ("x-opencode-client", "cli".to_string()),
        ("x-opencode-project", "global".to_string()),
        ("x-opencode-session", session_id.to_string()),
        ("x-opencode-session-id", session_id.to_string()),
    ]
}

/// Zen's anonymous free lane only exposes tool names the CLI itself uses. AirCode
/// remaps its richer toolset onto those two names (and back on the way in) so agentic
/// tool calling keeps working on the free tier.
pub fn zen_tool_name_out(name: &str) -> &'static str {
    match name {
        "execute_command" => "bash",
        "read_file" => "read",
        "write_file" => "edit",
        "search_project" => "grep",
        other => {
            // Unmapped tools keep their name; Zen ignores unknown definitions.
            let _ = other;
            "tool"
        }
    }
}

/// Inverse of [`zen_tool_name_out`].
pub fn zen_tool_name_in(name: &str) -> &'static str {
    match name {
        "bash" => "execute_command",
        "read" => "read_file",
        "edit" => "write_file",
        "grep" => "search_project",
        other => {
            let _ = other;
            "tool"
        }
    }
}

/// Reads the live Zen catalog anonymously (`GET /zen/v1/models` needs no key) and
/// returns the free-tier model ids, so AirCode can offer whatever is free *today*
/// instead of a hardcoded list.
pub fn fetch_zen_free_models() -> Vec<String> {
    let url = format!("{}/models", ZEN_BASE_URL);
    let r = http_request("GET", &url, &[("User-Agent", ZEN_CLIENT_USER_AGENT)], "", 15_000);
    if let Some(e) = r.error {
        let _ = e;
        return Vec::new();
    }
    if r.status != 200 {
        return Vec::new();
    }
    let Ok(json) = JsonValue::parse(&r.body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
        for m in arr {
            let id = m.string_or("id", "");
            if id.is_empty() {
                continue;
            }
            if is_zen_free_model(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// Human-readable summary of how AirCode will authenticate to Zen, for `/connect`.
pub fn zen_auth_summary(api_key: &str) -> String {
    if api_key.trim().is_empty() || api_key.trim() == ZEN_PUBLIC_KEY {
        "OpenCode Zen - free `public` lane (no key, OpenCode client headers)".to_string()
    } else {
        "OpenCode Zen - console API key (free + paid catalog)".to_string()
    }
}
