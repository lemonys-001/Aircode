use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub active_provider: String,
    pub active_model: String,
    pub default_mode: String,
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
    pub editor_font_size: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            active_provider: "ollama".to_string(),
            active_model: "qwen2.5-coder:7b".to_string(),
            default_mode: "autonomous".to_string(),
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
            max_agent_iterations: 15,
            default_shell: "powershell".to_string(),
            editor_font_size: 13,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionRecord {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    pub args_json: String,
    pub result_summary: String,
    pub status: String,
    pub duration_ms: u64,
    pub diff_text: Option<String>,
    pub file_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub mode: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<ToolExecutionRecord>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub mode: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub mode: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
    pub db_path: PathBuf,
}

impl SqliteStore {
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
        let db_path = app_data_dir.join("agent.db");
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                mode TEXT NOT NULL,
                provider TEXT,
                model TEXT,
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
                diff_text TEXT,
                file_path TEXT,
                created_at TEXT NOT NULL
            );
            ",
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
        })
    }

    pub fn load_settings(&self) -> AppSettings {
        let conn = self.conn.lock().unwrap();
        let res: Result<String, _> =
            conn.query_row("SELECT json_blob FROM settings WHERE id = 1", [], |r| {
                r.get(0)
            });
        if let Ok(raw) = res {
            if let Ok(parsed) = serde_json::from_str::<AppSettings>(&raw) {
                return parsed;
            }
        }
        let def = AppSettings::default();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (id, json_blob) VALUES (1, ?1)",
            params![serde_json::to_string(&def).unwrap_or_default()],
        );
        def
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let raw = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (id, json_blob) VALUES (1, ?1)",
            params![raw],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn record_recent_project(&self, path: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO recent_projects (path, visited_at) VALUES (?1, ?2)",
            params![path, Utc::now().to_rfc3339()],
        );
    }

    pub fn list_recent_projects(&self) -> Vec<String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn
            .prepare("SELECT path FROM recent_projects ORDER BY visited_at DESC LIMIT 12")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |r| r.get(0))
            .ok()
            .map(|iter| iter.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn ensure_default_session(&self, project_path: &str) -> SessionDetail {
        let list = self.list_sessions();
        if let Some(first) = list.first() {
            if let Some(detail) = self.load_session(&first.id) {
                return detail;
            }
        }
        self.create_session("Initial Coding Session", project_path, "autonomous")
    }

    pub fn create_session(&self, title: &str, project_path: &str, mode: &str) -> SessionDetail {
        let id = format!("ses_{}", Uuid::new_v4().simple());
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO sessions (id, title, project_path, mode, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, title, project_path, mode, now, now],
        );
        SessionDetail {
            id,
            title: title.to_string(),
            project_path: project_path.to_string(),
            mode: mode.to_string(),
            created_at: now.clone(),
            updated_at: now,
            messages: Vec::new(),
        }
    }

    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT s.id, s.title, s.project_path, s.mode, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id)
             FROM sessions s ORDER BY s.updated_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |r| {
            Ok(SessionSummary {
                id: r.get(0)?,
                title: r.get(1)?,
                project_path: r.get(2)?,
                mode: r.get(3)?,
                updated_at: r.get(4)?,
                message_count: r.get::<_, i64>(5)? as usize,
            })
        })
        .ok()
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    pub fn load_session(&self, session_id: &str) -> Option<SessionDetail> {
        let conn = self.conn.lock().unwrap();
        let mut detail = conn
            .query_row(
                "SELECT id, title, project_path, mode, created_at, updated_at FROM sessions WHERE id = ?1",
                params![session_id],
                |r| {
                    Ok(SessionDetail {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        project_path: r.get(2)?,
                        mode: r.get(3)?,
                        created_at: r.get(4)?,
                        updated_at: r.get(5)?,
                        messages: Vec::new(),
                    })
                },
            )
            .ok()?;

        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at
                 FROM messages WHERE session_id = ?1 ORDER BY created_at ASC",
            )
            .ok()?;

        detail.messages = stmt
            .query_map(params![session_id], |r| {
                let tools_raw: String = r.get(8)?;
                let tools = serde_json::from_str(&tools_raw).unwrap_or_default();
                Ok(ChatMessage {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    thinking: r.get(4)?,
                    mode: r.get(5)?,
                    provider: r.get(6)?,
                    model: r.get(7)?,
                    tools,
                    created_at: r.get(9)?,
                })
            })
            .ok()?
            .filter_map(Result::ok)
            .collect();

        Some(detail)
    }

    pub fn append_message(&self, msg: &ChatMessage) {
        let conn = self.conn.lock().unwrap();
        let tools_json = serde_json::to_string(&msg.tools).unwrap_or_else(|_| "[]".to_string());
        let _ = conn.execute(
            "INSERT INTO messages (id, session_id, role, content, thinking, mode, provider, model, tools_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                msg.id,
                msg.session_id,
                msg.role,
                msg.content,
                msg.thinking,
                msg.mode,
                msg.provider,
                msg.model,
                tools_json,
                msg.created_at
            ],
        );
        if msg.role == "user" {
            let short_title: String = msg.content.chars().take(48).collect();
            let _ = conn.execute(
                "UPDATE sessions SET updated_at = ?1, title = CASE WHEN title IN ('Initial Coding Session', 'New Agent Session') THEN ?2 ELSE title END WHERE id = ?3",
                params![msg.created_at, short_title, msg.session_id],
            );
        } else {
            let _ = conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![msg.created_at, msg.session_id],
            );
        }
    }

    pub fn delete_session(&self, session_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id]);
        let _ = conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id]);
    }

    pub fn record_tool_execution(&self, rec: &ToolExecutionRecord) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO tool_executions
             (id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                rec.id,
                rec.session_id,
                rec.tool_name,
                rec.args_json,
                rec.result_summary,
                rec.status,
                rec.duration_ms as i64,
                rec.diff_text,
                rec.file_path,
                rec.created_at
            ],
        );
    }

    pub fn list_tool_history(&self, limit: usize) -> Vec<ToolExecutionRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT id, session_id, tool_name, args_json, result_summary, status, duration_ms, diff_text, file_path, created_at
             FROM tool_executions ORDER BY created_at DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![limit as i64], |r| {
            Ok(ToolExecutionRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                tool_name: r.get(2)?,
                args_json: r.get(3)?,
                result_summary: r.get(4)?,
                status: r.get(5)?,
                duration_ms: r.get::<_, i64>(6)? as u64,
                diff_text: r.get(7)?,
                file_path: r.get(8)?,
                created_at: r.get(9)?,
            })
        })
        .ok()
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }
}
