//! Safe Pure-Rust persistent SQLite3 database engine (statically linked via vendor/sqlite3/sqlite3.c).
//! Stores AppSettings, Themes, OpenCode Auth, Sessions, Session Forks, Compaction Checkpoints,
//! Prompt Timestamps, and Tool Execution Audit History.

use crate::json::JsonValue;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;

#[repr(C)]
struct Sqlite3Db {
    _priv: [u8; 0],
}

#[repr(C)]
struct Sqlite3Stmt {
    _priv: [u8; 0],
}

type SqliteDestructor = Option<unsafe extern "C" fn(*mut c_void)>;

fn sqlite_transient() -> SqliteDestructor {
    unsafe { std::mem::transmute::<isize, SqliteDestructor>(-1) }
}

extern "C" {
    fn sqlite3_open(filename: *const c_char, pp_db: *mut *mut Sqlite3Db) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3Db) -> c_int;
    fn sqlite3_exec(
        db: *mut Sqlite3Db,
        sql: *const c_char,
        callback: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(ptr: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3Db,
        z_sql: *const c_char,
        n_byte: c_int,
        pp_stmt: *mut *mut Sqlite3Stmt,
        pz_tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        idx: c_int,
        val: *const c_char,
        n: c_int,
        destructor: SqliteDestructor,
    ) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, idx: c_int, val: i64) -> c_int;
    fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, i_col: c_int) -> *const u8;
    fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, i_col: c_int) -> i64;
    fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
}

pub fn now_timestamp_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z", year, m, d, hh, mm, ss)
}

pub fn short_time_label(ts: &str) -> String {
    // Extracts "HH:MM:SS" from "YYYY-MM-DD HH:MM:SSZ"
    if let Some((_, time_part)) = ts.split_once(' ') {
        return time_part.trim_end_matches('Z').to_string();
    }
    ts.to_string()
}

pub fn make_unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{:012x}{:04x}", prefix, nanos & 0xffffffffffff, n & 0xffff)
}

#[derive(Clone, Debug)]
pub struct AppSettings {
    pub active_provider: String,
    pub active_model: String,
    pub default_mode: String,   // "plan" | "build"
    pub agent_effort: String,   // "low" | "medium" | "max" | "ultra"
    pub theme: String,          // "tokyo-night" | "catppuccin-mocha" | ... | "custom"
    pub show_thinking: bool,
    pub opencode_api_key: String,
    pub opencode_auth_mode: String, // "zen" | "local-serve"
    pub opencode_server_url: String,
    pub openai_api_key: String,
    pub anthropic_api_key: String,
    pub gemini_api_key: String,
    pub openrouter_api_key: String,
    pub ollama_base_url: String,
    pub custom_base_url: String,
    pub custom_api_key: String,
    pub require_shell_approval: bool,
    pub require_delete_confirmation: bool,
    pub restrict_to_workspace: bool,
    pub max_agent_iterations: usize,
    pub default_shell: String,
    pub editor_font_size: i32,
    pub disabled_skills: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_provider: "opencode".to_string(),
            active_model: "big-pickle".to_string(),
            default_mode: "build".to_string(),
            agent_effort: "medium".to_string(),
            theme: "low-cortisol".to_string(),
            show_thinking: true,
            opencode_api_key: String::new(),
            opencode_auth_mode: "zen".to_string(),
            opencode_server_url: "http://127.0.0.1:4096".to_string(),
            openai_api_key: String::new(),
            anthropic_api_key: String::new(),
            gemini_api_key: String::new(),
            openrouter_api_key: String::new(),
            ollama_base_url: "http://127.0.0.1:11434".to_string(),
            custom_base_url: String::new(),
            custom_api_key: String::new(),
            require_shell_approval: true,
            require_delete_confirmation: true,
            restrict_to_workspace: true,
            max_agent_iterations: 16,
            default_shell: if cfg!(windows) {
                "powershell".to_string()
            } else {
                "sh".to_string()
            },
            editor_font_size: 15,
            disabled_skills: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn normalized_mode(&self) -> &'static str {
        if self.default_mode.eq_ignore_ascii_case("plan") {
            "plan"
        } else {
            "build"
        }
    }

    pub fn effort_max_iters(&self) -> usize {
        match self.agent_effort.as_str() {
            "low" => 6,
            "medium" => 14,
            "max" => 26,
            "ultra" => 44,
            _ => self.max_agent_iterations.clamp(4, 50),
        }
    }

    pub fn effort_thinking_budget(&self) -> usize {
        match self.agent_effort.as_str() {
            "low" => 1024,
            "medium" => 4096,
            "max" => 16000,
            "ultra" => 32000,
            _ => 4096,
        }
    }

    pub fn effort_openai_tag(&self) -> &'static str {
        match self.agent_effort.as_str() {
            "low" => "low",
            "medium" => "medium",
            "max" | "ultra" => "high",
            _ => "medium",
        }
    }

    pub fn to_json(&self) -> JsonValue {
        JsonValue::obj(vec![
            ("activeProvider", self.active_provider.clone().into()),
            ("activeModel", self.active_model.clone().into()),
            ("defaultMode", self.default_mode.clone().into()),
            ("agentEffort", self.agent_effort.clone().into()),
            ("theme", self.theme.clone().into()),
            ("showThinking", self.show_thinking.into()),
            ("opencodeApiKey", self.opencode_api_key.clone().into()),
            ("opencodeAuthMode", self.opencode_auth_mode.clone().into()),
            ("opencodeServerUrl", self.opencode_server_url.clone().into()),
            ("openaiApiKey", self.openai_api_key.clone().into()),
            ("anthropicApiKey", self.anthropic_api_key.clone().into()),
            ("geminiApiKey", self.gemini_api_key.clone().into()),
            ("openrouterApiKey", self.openrouter_api_key.clone().into()),
            ("ollamaBaseUrl", self.ollama_base_url.clone().into()),
            ("customBaseUrl", self.custom_base_url.clone().into()),
            ("customApiKey", self.custom_api_key.clone().into()),
            ("requireShellApproval", self.require_shell_approval.into()),
            (
                "requireDeleteConfirmation",
                self.require_delete_confirmation.into(),
            ),
            ("restrictToWorkspace", self.restrict_to_workspace.into()),
            ("maxAgentIterations", self.max_agent_iterations.into()),
            ("defaultShell", self.default_shell.clone().into()),
            ("editorFontSize", (self.editor_font_size as i64).into()),
            (
                "disabledSkills",
                JsonValue::arr(
                    self.disabled_skills
                        .iter()
                        .cloned()
                        .map(JsonValue::from)
                        .collect(),
                ),
            ),
        ])
    }

    pub fn from_json(v: &JsonValue) -> Self {
        let def = Self::default();
        let raw_mode = v.string_or("defaultMode", &def.default_mode);
        let norm_mode = if raw_mode == "plan" {
            "plan".to_string()
        } else {
            "build".to_string()
        };
        let disabled_skills = v
            .get("disabledSkills")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            active_provider: v.string_or("activeProvider", &def.active_provider),
            active_model: v.string_or("activeModel", &def.active_model),
            default_mode: norm_mode,
            agent_effort: v.string_or("agentEffort", &def.agent_effort),
            theme: v.string_or("theme", &def.theme),
            show_thinking: v.bool_or("showThinking", true),
            opencode_api_key: v.string_or("opencodeApiKey", ""),
            opencode_auth_mode: v.string_or("opencodeAuthMode", "zen"),
            opencode_server_url: v.string_or("opencodeServerUrl", &def.opencode_server_url),
            openai_api_key: v.string_or("openaiApiKey", ""),
            anthropic_api_key: v.string_or("anthropicApiKey", ""),
            gemini_api_key: v.string_or("geminiApiKey", ""),
            openrouter_api_key: v.string_or("openrouterApiKey", ""),
            ollama_base_url: v.string_or("ollamaBaseUrl", &def.ollama_base_url),
            custom_base_url: v.string_or("customBaseUrl", ""),
            custom_api_key: v.string_or("customApiKey", ""),
            require_shell_approval: v.bool_or("requireShellApproval", true),
            require_delete_confirmation: v.bool_or("requireDeleteConfirmation", true),
            restrict_to_workspace: v.bool_or("restrictToWorkspace", true),
            max_agent_iterations: v.i64_or("maxAgentIterations", 16).clamp(1, 50) as usize,
            default_shell: v.string_or("defaultShell", &def.default_shell),
            editor_font_size: v.i64_or("editorFontSize", 15).clamp(11, 24) as i32,
            disabled_skills,
        }
    }

    pub fn preset_models_for_provider(provider: &str) -> &'static [&'static str] {
        match provider {
            // Free lane first (no key needed - OpenCode client emulation), then the
            // paid Zen catalog that a console key unlocks.
            "opencode" => &[
                "big-pickle",
                "glm-5.3-flash-free",
                "deepseek-v4.1-flash-free",
                "nemotron-3.5-lightning-free",
                "space-bunny-free",
                "mimo-v2.6-flash-free",
                "cat-jam-free",
                "longcat-2.5-preview-free",
                "grok-code",
                "claude-sonnet-4-5",
                "gpt-5",
            ],
            "openai" => &["gpt-4.1", "o4-mini", "gpt-4o", "gpt-4.1-mini"],
            "anthropic" => &[
                "claude-sonnet-4-20250514",
                "claude-3-7-sonnet-latest",
                "claude-3-5-haiku-latest",
            ],
            "gemini" => &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.0-flash"],
            "openrouter" => &[
                "anthropic/claude-sonnet-4",
                "deepseek/deepseek-r1",
                "deepseek/deepseek-chat-v3-0324",
                "qwen/qwen-2.5-coder-32b-instruct",
            ],
            "ollama" => &[
                "qwen2.5-coder:7b",
                "qwen2.5-coder:14b",
                "deepseek-coder-v2:16b",
                "llama3.1:8b",
            ],
            _ => &["custom-coding-model"],
        }
    }

    pub fn has_credentials_for(&self, provider: &str) -> bool {
        match provider {
            "opencode" | "ollama" => true,
            "openai" => !self.openai_api_key.trim().is_empty(),
            "anthropic" => !self.anthropic_api_key.trim().is_empty(),
            "gemini" => !self.gemini_api_key.trim().is_empty(),
            "openrouter" => !self.openrouter_api_key.trim().is_empty(),
            "custom" => !self.custom_base_url.trim().is_empty(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolExecutionRecord {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    pub args_json: String,
    pub result_summary: String,
    pub status: String,
    pub duration_ms: u64,
    pub diff_text: String,
    pub file_path: String,
    pub created_at: String,
}

impl ToolExecutionRecord {
    pub fn to_json(&self) -> JsonValue {
        JsonValue::obj(vec![
            ("id", self.id.clone().into()),
            ("sessionId", self.session_id.clone().into()),
            ("toolName", self.tool_name.clone().into()),
            ("argsJson", self.args_json.clone().into()),
            ("resultSummary", self.result_summary.clone().into()),
            ("status", self.status.clone().into()),
            ("durationMs", (self.duration_ms as i64).into()),
            ("diffText", self.diff_text.clone().into()),
            ("filePath", self.file_path.clone().into()),
            ("createdAt", self.created_at.clone().into()),
        ])
    }

    pub fn from_json(v: &JsonValue) -> Self {
        Self {
            id: v.string_or("id", ""),
            session_id: v.string_or("sessionId", ""),
            tool_name: v.string_or("toolName", ""),
            args_json: v.string_or("argsJson", "{}"),
            result_summary: v.string_or("resultSummary", ""),
            status: v.string_or("status", "ok"),
            duration_ms: v.i64_or("durationMs", 0).max(0) as u64,
            diff_text: v.string_or("diffText", ""),
            file_path: v.string_or("filePath", ""),
            created_at: v.string_or("createdAt", ""),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub mode: String,
    pub provider: String,
    pub model: String,
    pub tools: Vec<ToolExecutionRecord>,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub mode: String,
    pub parent_id: String,
    pub compacted_count: usize,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Clone, Debug)]
pub struct SessionDetail {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub mode: String,
    pub parent_id: String,
    pub compacted_count: usize,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

struct RawHandle(*mut Sqlite3Db);
unsafe impl Send for RawHandle {}

pub struct SqliteStore {
    handle: Mutex<RawHandle>,
    pub db_path: PathBuf,
}

impl Drop for SqliteStore {
    fn drop(&mut self) {
        if let Ok(guard) = self.handle.lock() {
            if !guard.0.is_null() {
                unsafe {
                    sqlite3_close(guard.0);
                }
            }
        }
    }
}

unsafe fn col_string(stmt: *mut Sqlite3Stmt, col: c_int) -> String {
    let p = sqlite3_column_text(stmt, col);
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p as *const c_char)
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn bind_str(stmt: *mut Sqlite3Stmt, idx: c_int, s: &str) {
    sqlite3_bind_text(
        stmt,
        idx,
        s.as_ptr() as *const c_char,
        s.len() as c_int,
        sqlite_transient(),
    );
}

unsafe fn exec_silent(raw: *mut Sqlite3Db, sql_nul: &[u8]) {
    let mut err: *mut c_char = ptr::null_mut();
    sqlite3_exec(
        raw,
        sql_nul.as_ptr() as *const c_char,
        ptr::null(),
        ptr::null_mut(),
        &mut err,
    );
    if !err.is_null() {
        sqlite3_free(err as *mut c_void);
    }
}

impl SqliteStore {
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
        let db_path = app_data_dir.join("agent.db");
        let c_path = CString::new(db_path.to_string_lossy().as_bytes())
            .map_err(|_| "Invalid NUL in database path".to_string())?;

        let mut raw: *mut Sqlite3Db = ptr::null_mut();
        let rc = unsafe { sqlite3_open(c_path.as_ptr(), &mut raw) };
        if rc != SQLITE_OK || raw.is_null() {
            return Err(format!("sqlite3_open failed with code {}", rc));
        }

        let schema = CString::new(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS settings (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 json_blob TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS recent_projects (
                 path TEXT PRIMARY KEY,
                 visited_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 project_path TEXT NOT NULL,
                 mode TEXT NOT NULL,
                 parent_id TEXT NOT NULL DEFAULT '',
                 compacted_count INTEGER NOT NULL DEFAULT 0,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 role TEXT NOT NULL,
                 content TEXT NOT NULL,
                 thinking TEXT NOT NULL DEFAULT '',
                 mode TEXT NOT NULL,
                 provider TEXT NOT NULL DEFAULT '',
                 model TEXT NOT NULL DEFAULT '',
                 tools_json TEXT NOT NULL DEFAULT '[]',
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tool_executions (
                 id TEXT PRIMARY KEY,
                 session_id TEXT NOT NULL,
                 tool_name TEXT NOT NULL,
                 args_json TEXT NOT NULL,
                 result_summary TEXT NOT NULL,
                 status TEXT NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 diff_text TEXT NOT NULL DEFAULT '',
                 file_path TEXT NOT NULL DEFAULT '',
                 created_at TEXT NOT NULL
             );",
        )
        .unwrap();

        unsafe {
            let mut err_msg: *mut c_char = ptr::null_mut();
            let exec_rc = sqlite3_exec(
                raw,
                schema.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                &mut err_msg,
            );
            if !err_msg.is_null() {
                let s = CStr::from_ptr(err_msg).to_string_lossy().into_owned();
                sqlite3_free(err_msg as *mut c_void);
                if exec_rc != SQLITE_OK {
                    sqlite3_close(raw);
                    return Err(format!("SQLite schema error: {}", s));
                }
            }
            // Safe forward migrations for databases created by v2.0
            exec_silent(
                raw,
                b"ALTER TABLE sessions ADD COLUMN parent_id TEXT NOT NULL DEFAULT ''\0",
            );
            exec_silent(
                raw,
                b"ALTER TABLE sessions ADD COLUMN compacted_count INTEGER NOT NULL DEFAULT 0\0",
            );
        }

        Ok(Self {
            handle: Mutex::new(RawHandle(raw)),
            db_path,
        })
    }

    pub fn load_settings(&self) -> AppSettings {
        let guard = self.handle.lock().unwrap();
        let sql = b"SELECT json_blob FROM settings WHERE id = 1\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                if sqlite3_step(stmt) == SQLITE_ROW {
                    let raw = col_string(stmt, 0);
                    sqlite3_finalize(stmt);
                    if let Ok(parsed) = JsonValue::parse(&raw) {
                        return AppSettings::from_json(&parsed);
                    }
                    return AppSettings::default();
                }
                sqlite3_finalize(stmt);
            }
        }
        drop(guard);
        let def = AppSettings::default();
        self.save_settings(&def);
        def
    }

    pub fn save_settings(&self, settings: &AppSettings) {
        let guard = self.handle.lock().unwrap();
        let serialized = settings.to_json().to_json_string();
        let sql = b"INSERT OR REPLACE INTO settings (id, json_blob) VALUES (1, ?1)\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, &serialized);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }
        }
    }

    pub fn record_recent_project(&self, path: &str) {
        let guard = self.handle.lock().unwrap();
        let now = now_timestamp_label();
        let sql = b"INSERT OR REPLACE INTO recent_projects (path, visited_at) VALUES (?1, ?2)\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, path);
                bind_str(stmt, 2, &now);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }
        }
    }

    pub fn list_recent_projects(&self) -> Vec<String> {
        let guard = self.handle.lock().unwrap();
        let mut out = Vec::new();
        let sql = b"SELECT path FROM recent_projects ORDER BY visited_at DESC LIMIT 10\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                while sqlite3_step(stmt) == SQLITE_ROW {
                    out.push(col_string(stmt, 0));
                }
                sqlite3_finalize(stmt);
            }
        }
        out
    }

    pub fn create_session(&self, title: &str, project_path: &str, mode: &str) -> SessionDetail {
        self.create_session_with_parent(title, project_path, mode, "")
    }

    pub fn create_session_with_parent(
        &self,
        title: &str,
        project_path: &str,
        mode: &str,
        parent_id: &str,
    ) -> SessionDetail {
        let id = make_unique_id("ses");
        let now = now_timestamp_label();
        let guard = self.handle.lock().unwrap();
        let sql = b"INSERT INTO sessions (id, title, project_path, mode, parent_id, compacted_count, created_at, updated_at) \
                    VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, &id);
                bind_str(stmt, 2, title);
                bind_str(stmt, 3, project_path);
                bind_str(stmt, 4, mode);
                bind_str(stmt, 5, parent_id);
                bind_str(stmt, 6, &now);
                bind_str(stmt, 7, &now);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }
        }
        SessionDetail {
            id,
            title: title.to_string(),
            project_path: project_path.to_string(),
            mode: mode.to_string(),
            parent_id: parent_id.to_string(),
            compacted_count: 0,
            created_at: now.clone(),
            updated_at: now,
            messages: Vec::new(),
        }
    }

    pub fn rename_session(&self, session_id: &str, new_title: &str) {
        let guard = self.handle.lock().unwrap();
        let sql = b"UPDATE sessions SET title = ?1 WHERE id = ?2\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, new_title);
                bind_str(stmt, 2, session_id);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }
        }
    }

    /// Forks `source_session_id` (optionally up to `up_to_msg_idx` inclusive for Timeline point-in-time forking!)
    /// into a brand-new independent session with full message/tool lineage preserved.
    pub fn fork_session(
        &self,
        source_session_id: &str,
        up_to_msg_idx: Option<usize>,
    ) -> Option<SessionDetail> {
        let src = self.load_session(source_session_id)?;
        let fork_title = match up_to_msg_idx {
            Some(idx) => format!("Fork #{} · {}", idx + 1, src.title),
            None => format!("Fork · {}", src.title),
        };
        let created =
            self.create_session_with_parent(&fork_title, &src.project_path, &src.mode, &src.id);
        let limit = up_to_msg_idx
            .map(|i| (i + 1).min(src.messages.len()))
            .unwrap_or(src.messages.len());

        for orig in src.messages.into_iter().take(limit) {
            let mut cloned = orig;
            cloned.id = make_unique_id("msg");
            cloned.session_id = created.id.clone();
            self.append_message(&cloned);
        }
        self.load_session(&created.id)
    }

    /// Compacts a session's older messages into a structured checkpoint message while keeping
    /// the last 2 messages intact for immediate conversational continuity.
    pub fn compact_session(&self, session_id: &str, summary_markdown: &str) -> Option<SessionDetail> {
        let cur = self.load_session(session_id)?;
        if cur.messages.is_empty() {
            return Some(cur);
        }
        let tail_keep = if cur.messages.len() > 2 { 2 } else { 0 };
        let tail: Vec<ChatMessage> = cur.messages[cur.messages.len() - tail_keep..].to_vec();

        {
            let guard = self.handle.lock().unwrap();
            let d_sql = b"DELETE FROM messages WHERE session_id = ?1\0";
            unsafe {
                let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
                if sqlite3_prepare_v2(
                    guard.0,
                    d_sql.as_ptr() as *const c_char,
                    -1,
                    &mut stmt,
                    ptr::null_mut(),
                ) == SQLITE_OK
                {
                    bind_str(stmt, 1, session_id);
                    sqlite3_step(stmt);
                    sqlite3_finalize(stmt);
                }

                let u_sql = b"UPDATE sessions SET compacted_count = compacted_count + 1, updated_at = ?1 WHERE id = ?2\0";
                let mut ustmt: *mut Sqlite3Stmt = ptr::null_mut();
                if sqlite3_prepare_v2(
                    guard.0,
                    u_sql.as_ptr() as *const c_char,
                    -1,
                    &mut ustmt,
                    ptr::null_mut(),
                ) == SQLITE_OK
                {
                    let now = now_timestamp_label();
                    bind_str(ustmt, 1, &now);
                    bind_str(ustmt, 2, session_id);
                    sqlite3_step(ustmt);
                    sqlite3_finalize(ustmt);
                }
            }
        }

        self.append_message(&ChatMessage {
            id: make_unique_id("msg"),
            session_id: session_id.to_string(),
            role: "assistant".to_string(),
            content: format!("**[Compacted Context Checkpoint]**\n{}", summary_markdown.trim()),
            thinking: String::new(),
            mode: cur.mode.clone(),
            provider: "compactor".to_string(),
            model: "checkpoint".to_string(),
            tools: Vec::new(),
            created_at: now_timestamp_label(),
        });

        for mut m in tail {
            m.id = make_unique_id("msg");
            self.append_message(&m);
        }

        self.load_session(session_id)
    }

    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        let guard = self.handle.lock().unwrap();
        let mut out = Vec::new();
        let sql = b"SELECT s.id, s.title, s.project_path, s.mode, s.parent_id, s.compacted_count, s.updated_at, \
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) \
                    FROM sessions s ORDER BY s.updated_at DESC\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                while sqlite3_step(stmt) == SQLITE_ROW {
                    out.push(SessionSummary {
                        id: col_string(stmt, 0),
                        title: col_string(stmt, 1),
                        project_path: col_string(stmt, 2),
                        mode: col_string(stmt, 3),
                        parent_id: col_string(stmt, 4),
                        compacted_count: sqlite3_column_int64(stmt, 5).max(0) as usize,
                        updated_at: col_string(stmt, 6),
                        message_count: sqlite3_column_int64(stmt, 7).max(0) as usize,
                    });
                }
                sqlite3_finalize(stmt);
            }
        }
        out
    }

    pub fn load_session(&self, session_id: &str) -> Option<SessionDetail> {
        let guard = self.handle.lock().unwrap();
        let sql = b"SELECT id, title, project_path, mode, parent_id, compacted_count, created_at, updated_at \
                    FROM sessions WHERE id = ?1\0";
        let mut detail = unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) != SQLITE_OK
            {
                return None;
            }
            bind_str(stmt, 1, session_id);
            let res = if sqlite3_step(stmt) == SQLITE_ROW {
                Some(SessionDetail {
                    id: col_string(stmt, 0),
                    title: col_string(stmt, 1),
                    project_path: col_string(stmt, 2),
                    mode: col_string(stmt, 3),
                    parent_id: col_string(stmt, 4),
                    compacted_count: sqlite3_column_int64(stmt, 5).max(0) as usize,
                    created_at: col_string(stmt, 6),
                    updated_at: col_string(stmt, 7),
                    messages: Vec::new(),
                })
            } else {
                None
            };
            sqlite3_finalize(stmt);
            res?
        };

        let m_sql = b"SELECT id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at \
                      FROM messages WHERE session_id = ?1 ORDER BY rowid ASC\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                m_sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, session_id);
                while sqlite3_step(stmt) == SQLITE_ROW {
                    let tools_raw = col_string(stmt, 8);
                    let tools = JsonValue::parse(&tools_raw)
                        .ok()
                        .and_then(|v| {
                            v.as_array().map(|arr| {
                                arr.iter().map(ToolExecutionRecord::from_json).collect()
                            })
                        })
                        .unwrap_or_default();
                    detail.messages.push(ChatMessage {
                        id: col_string(stmt, 0),
                        session_id: col_string(stmt, 1),
                        role: col_string(stmt, 2),
                        content: col_string(stmt, 3),
                        thinking: col_string(stmt, 4),
                        mode: col_string(stmt, 5),
                        provider: col_string(stmt, 6),
                        model: col_string(stmt, 7),
                        tools,
                        created_at: col_string(stmt, 9),
                    });
                }
                sqlite3_finalize(stmt);
            }
        }
        Some(detail)
    }

    pub fn ensure_default_session(&self, project_path: &str) -> SessionDetail {
        let sessions = self.list_sessions();
        if let Some(first) = sessions.first() {
            if let Some(loaded) = self.load_session(&first.id) {
                return loaded;
            }
        }
        self.create_session("New Session", project_path, "build")
    }

    pub fn append_message(&self, msg: &ChatMessage) {
        let guard = self.handle.lock().unwrap();
        let tools_json =
            JsonValue::arr(msg.tools.iter().map(|t| t.to_json()).collect()).to_json_string();
        let sql = b"INSERT INTO messages \
            (id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at) \
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, &msg.id);
                bind_str(stmt, 2, &msg.session_id);
                bind_str(stmt, 3, &msg.role);
                bind_str(stmt, 4, &msg.content);
                bind_str(stmt, 5, &msg.thinking);
                bind_str(stmt, 6, &msg.mode);
                bind_str(stmt, 7, &msg.provider);
                bind_str(stmt, 8, &msg.model);
                bind_str(stmt, 9, &tools_json);
                bind_str(stmt, 10, &msg.created_at);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }

            if msg.role == "user" {
                let short_title: String = msg.content.chars().take(44).collect();
                let u_sql = b"UPDATE sessions SET updated_at = ?1, \
                    title = CASE WHEN title IN ('Initial Coding Session', 'New Session') THEN ?2 ELSE title END \
                    WHERE id = ?3\0";
                let mut ustmt: *mut Sqlite3Stmt = ptr::null_mut();
                if sqlite3_prepare_v2(
                    guard.0,
                    u_sql.as_ptr() as *const c_char,
                    -1,
                    &mut ustmt,
                    ptr::null_mut(),
                ) == SQLITE_OK
                {
                    bind_str(ustmt, 1, &msg.created_at);
                    bind_str(ustmt, 2, &short_title);
                    bind_str(ustmt, 3, &msg.session_id);
                    sqlite3_step(ustmt);
                    sqlite3_finalize(ustmt);
                }
            }
        }
    }

    pub fn delete_session(&self, session_id: &str) {
        let guard = self.handle.lock().unwrap();
        for sql in [
            &b"DELETE FROM messages WHERE session_id = ?1\0"[..],
            &b"DELETE FROM sessions WHERE id = ?1\0"[..],
        ] {
            unsafe {
                let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
                if sqlite3_prepare_v2(
                    guard.0,
                    sql.as_ptr() as *const c_char,
                    -1,
                    &mut stmt,
                    ptr::null_mut(),
                ) == SQLITE_OK
                {
                    bind_str(stmt, 1, session_id);
                    sqlite3_step(stmt);
                    sqlite3_finalize(stmt);
                }
            }
        }
    }

    pub fn record_tool_execution(&self, rec: &ToolExecutionRecord) {
        let guard = self.handle.lock().unwrap();
        let sql = b"INSERT OR REPLACE INTO tool_executions \
            (id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at) \
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                bind_str(stmt, 1, &rec.id);
                bind_str(stmt, 2, &rec.session_id);
                bind_str(stmt, 3, &rec.tool_name);
                bind_str(stmt, 4, &rec.args_json);
                bind_str(stmt, 5, &rec.result_summary);
                bind_str(stmt, 6, &rec.status);
                sqlite3_bind_int64(stmt, 7, rec.duration_ms as i64);
                bind_str(stmt, 8, &rec.diff_text);
                bind_str(stmt, 9, &rec.file_path);
                bind_str(stmt, 10, &rec.created_at);
                sqlite3_step(stmt);
                sqlite3_finalize(stmt);
            }
        }
    }

    pub fn list_tool_history(&self, limit: usize) -> Vec<ToolExecutionRecord> {
        let guard = self.handle.lock().unwrap();
        let mut out = Vec::new();
        let sql = b"SELECT id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at \
                    FROM tool_executions ORDER BY rowid DESC LIMIT ?1\0";
        unsafe {
            let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
            if sqlite3_prepare_v2(
                guard.0,
                sql.as_ptr() as *const c_char,
                -1,
                &mut stmt,
                ptr::null_mut(),
            ) == SQLITE_OK
            {
                sqlite3_bind_int64(stmt, 1, limit as i64);
                while sqlite3_step(stmt) == SQLITE_ROW {
                    out.push(ToolExecutionRecord {
                        id: col_string(stmt, 0),
                        session_id: col_string(stmt, 1),
                        tool_name: col_string(stmt, 2),
                        args_json: col_string(stmt, 3),
                        result_summary: col_string(stmt, 4),
                        status: col_string(stmt, 5),
                        duration_ms: sqlite3_column_int64(stmt, 6).max(0) as u64,
                        diff_text: col_string(stmt, 7),
                        file_path: col_string(stmt, 8),
                        created_at: col_string(stmt, 9),
                    });
                }
                sqlite3_finalize(stmt);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_persistence_fork_and_compact() {
        let dir = std::env::temp_dir().join(make_unique_id("sqltest"));
        let store = SqliteStore::open(&dir).expect("open sqlite");
        let mut s = store.load_settings();
        s.active_provider = "opencode".to_string();
        s.agent_effort = "ultra".to_string();
        store.save_settings(&s);
        assert_eq!(store.load_settings().agent_effort, "ultra");

        let ses = store.create_session("Build Auth", "/tmp/proj", "build");
        for i in 1..=4 {
            store.append_message(&ChatMessage {
                id: make_unique_id("msg"),
                session_id: ses.id.clone(),
                role: if i % 2 == 1 { "user".into() } else { "assistant".into() },
                content: format!("Turn {}", i),
                thinking: "Analyzing...".into(),
                mode: "build".into(),
                provider: "opencode".into(),
                model: "big-pickle".into(),
                tools: Vec::new(),
                created_at: now_timestamp_label(),
            });
        }

        // Test point-in-time fork at message index 1 (first 2 messages)
        let forked = store.fork_session(&ses.id, Some(1)).expect("forked");
        assert_eq!(forked.parent_id, ses.id);
        assert_eq!(forked.messages.len(), 2);

        // Test compaction
        let compacted = store
            .compact_session(&ses.id, "- Implemented JWT middleware\n- Tests green")
            .expect("compacted");
        assert_eq!(compacted.compacted_count, 1);
        assert_eq!(compacted.messages.len(), 3); // 1 checkpoint + last 2 kept
        let _ = std::fs::remove_dir_all(&dir);
    }
}
