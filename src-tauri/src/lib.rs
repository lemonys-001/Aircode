pub mod agent;
pub mod db;
pub mod fs_ops;
pub mod git_ops;
pub mod permissions;
pub mod pty;

use db::{AppSettings, SessionDetail, SessionSummary, SqliteStore, ToolExecutionRecord};
use fs_ops::{
    list_windows_drives, resolve_project_path, scan_project_tree, search_project_files, DriveInfo,
    FileEntry, FsWatcherManager, SearchMatch,
};
use git_ops::{get_file_diff, inspect_git_status, run_git, GitFileDiff, GitStatusPayload};
use permissions::PermissionGate;
use pty::{PtyManager, PtyTabInfo};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

pub struct DesktopState {
    pub app_data_dir: PathBuf,
    pub store: Arc<SqliteStore>,
    pub project_root: Mutex<PathBuf>,
    pub watcher: FsWatcherManager,
    pub pty: PtyManager,
    pub gate: Arc<PermissionGate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInitPayload {
    pub app_version: String,
    pub platform: String,
    pub app_data_dir: String,
    pub sqlite_path: String,
    pub current_project: String,
    pub drives: Vec<DriveInfo>,
    pub files: Vec<FileEntry>,
    pub git: GitStatusPayload,
    pub settings: AppSettings,
    pub sessions: Vec<SessionSummary>,
    pub active_session: SessionDetail,
    pub recent_projects: Vec<String>,
}

#[tauri::command]
fn app_init(app: AppHandle, state: State<'_, DesktopState>) -> Result<AppInitPayload, String> {
    let root = state.project_root.lock().unwrap().clone();
    state.watcher.watch_directory(app, &root);
    let root_str = root.to_string_lossy().to_string();
    state.store.record_recent_project(&root_str);

    let settings = state.store.load_settings();
    let active_session = state.store.ensure_default_session(&root_str);
    let sessions = state.store.list_sessions();

    Ok(AppInitPayload {
        app_version: "1.0.0".to_string(),
        platform: "windows-x86_64".to_string(),
        app_data_dir: state.app_data_dir.to_string_lossy().to_string(),
        sqlite_path: state.store.db_path.to_string_lossy().to_string(),
        current_project: root_str,
        drives: list_windows_drives(),
        files: scan_project_tree(&root, 2000),
        git: inspect_git_status(&root),
        settings,
        sessions,
        active_session,
        recent_projects: state.store.list_recent_projects(),
    })
}

#[tauri::command]
fn pick_project_folder() -> Result<Value, String> {
    let picked = rfd::FileDialog::new()
        .set_title("Open Windows Project Workspace")
        .pick_folder();
    Ok(json!({
        "path": picked.map(|p| p.to_string_lossy().to_string())
    }))
}

#[tauri::command]
fn set_project_folder(
    app: AppHandle,
    state: State<'_, DesktopState>,
    path: String,
) -> Result<Value, String> {
    let candidate = PathBuf::from(&path);
    if !candidate.exists() || !candidate.is_dir() {
        return Err(format!("Directory does not exist: {}", path));
    }
    let canon = std::fs::canonicalize(&candidate).unwrap_or(candidate);
    {
        let mut guard = state.project_root.lock().unwrap();
        *guard = canon.clone();
    }
    state.watcher.watch_directory(app, &canon);
    let root_str = canon.to_string_lossy().to_string();
    state.store.record_recent_project(&root_str);

    Ok(json!({
        "root": root_str,
        "files": scan_project_tree(&canon, 2000),
        "git": inspect_git_status(&canon)
    }))
}

#[tauri::command]
fn list_project_files(state: State<'_, DesktopState>) -> Result<Vec<FileEntry>, String> {
    let root = state.project_root.lock().unwrap().clone();
    Ok(scan_project_tree(&root, 2000))
}

#[tauri::command]
fn read_project_file(state: State<'_, DesktopState>, path: String) -> Result<Value, String> {
    let root = state.project_root.lock().unwrap().clone();
    let settings = state.store.load_settings();
    let resolved = resolve_project_path(&root, &path, settings.restrict_to_workspace)?;
    let meta = std::fs::metadata(&resolved).map_err(|e| e.to_string())?;
    let content = std::fs::read_to_string(&resolved).map_err(|e| e.to_string())?;
    Ok(json!({
        "path": resolved.to_string_lossy().to_string(),
        "content": content,
        "readonly": meta.permissions().readonly()
    }))
}

#[tauri::command]
fn write_project_file(
    state: State<'_, DesktopState>,
    path: String,
    content: String,
) -> Result<Value, String> {
    let root = state.project_root.lock().unwrap().clone();
    let settings = state.store.load_settings();
    let resolved = resolve_project_path(&root, &path, settings.restrict_to_workspace)?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&resolved, content).map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn create_fs_item(
    state: State<'_, DesktopState>,
    path: String,
    is_dir: bool,
) -> Result<Value, String> {
    let root = state.project_root.lock().unwrap().clone();
    let settings = state.store.load_settings();
    let resolved = resolve_project_path(&root, &path, settings.restrict_to_workspace)?;
    if is_dir {
        std::fs::create_dir_all(&resolved).map_err(|e| e.to_string())?;
    } else {
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&resolved, "").map_err(|e| e.to_string())?;
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn delete_fs_item(
    state: State<'_, DesktopState>,
    path: String,
    confirmed: bool,
) -> Result<Value, String> {
    let settings = state.store.load_settings();
    if settings.require_delete_confirmation && !confirmed {
        return Err("Deletion confirmation required by safety settings.".to_string());
    }
    let root = state.project_root.lock().unwrap().clone();
    let resolved = resolve_project_path(&root, &path, settings.restrict_to_workspace)?;
    if resolved.is_dir() {
        std::fs::remove_dir_all(&resolved).map_err(|e| e.to_string())?;
    } else {
        std::fs::remove_file(&resolved).map_err(|e| e.to_string())?;
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn reveal_in_explorer(path: String) -> Result<Value, String> {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe")
            .arg(format!("/select,{}", path))
            .spawn();
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn search_in_project(
    state: State<'_, DesktopState>,
    query: String,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, String> {
    let root = state.project_root.lock().unwrap().clone();
    Ok(search_project_files(&root, &query, case_sensitive, 120))
}

#[tauri::command]
fn git_status(state: State<'_, DesktopState>) -> Result<GitStatusPayload, String> {
    let root = state.project_root.lock().unwrap().clone();
    Ok(inspect_git_status(&root))
}

#[tauri::command]
fn git_file_diff(state: State<'_, DesktopState>, path: String) -> Result<GitFileDiff, String> {
    let root = state.project_root.lock().unwrap().clone();
    get_file_diff(&root, &path)
}

#[tauri::command]
fn git_stage(
    state: State<'_, DesktopState>,
    path: String,
    stage_all: bool,
) -> Result<GitStatusPayload, String> {
    let root = state.project_root.lock().unwrap().clone();
    if stage_all {
        let _ = run_git(&root, &["add", "-A"])?;
    } else {
        let _ = run_git(&root, &["add", "--", &path])?;
    }
    Ok(inspect_git_status(&root))
}

#[tauri::command]
fn git_unstage(state: State<'_, DesktopState>, path: String) -> Result<GitStatusPayload, String> {
    let root = state.project_root.lock().unwrap().clone();
    let _ = run_git(&root, &["restore", "--staged", "--", &path]);
    Ok(inspect_git_status(&root))
}

#[tauri::command]
fn git_commit(state: State<'_, DesktopState>, message: String) -> Result<Value, String> {
    let root = state.project_root.lock().unwrap().clone();
    let (code, out, err) = run_git(&root, &["commit", "-m", &message])?;
    Ok(json!({
        "ok": code == 0,
        "output": format!("{}{}", out, err),
        "status": inspect_git_status(&root)
    }))
}

#[tauri::command]
fn git_checkout(
    state: State<'_, DesktopState>,
    branch: String,
    create_new: bool,
) -> Result<Value, String> {
    let root = state.project_root.lock().unwrap().clone();
    let (code, out, err) = if create_new {
        run_git(&root, &["checkout", "-b", &branch])?
    } else {
        run_git(&root, &["checkout", &branch])?
    };
    Ok(json!({
        "ok": code == 0,
        "output": format!("{}{}", out, err),
        "status": inspect_git_status(&root)
    }))
}

#[tauri::command]
fn save_settings(
    state: State<'_, DesktopState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    state.store.save_settings(&settings)?;
    Ok(state.store.load_settings())
}

#[tauri::command]
fn list_sessions(state: State<'_, DesktopState>) -> Result<Vec<SessionSummary>, String> {
    Ok(state.store.list_sessions())
}

#[tauri::command]
fn create_session(
    state: State<'_, DesktopState>,
    title: String,
    mode: String,
) -> Result<SessionDetail, String> {
    let root = state
        .project_root
        .lock()
        .unwrap()
        .to_string_lossy()
        .to_string();
    Ok(state.store.create_session(&title, &root, &mode))
}

#[tauri::command]
fn load_session(state: State<'_, DesktopState>, session_id: String) -> Result<SessionDetail, String> {
    state
        .store
        .load_session(&session_id)
        .ok_or_else(|| format!("Session '{}' not found", session_id))
}

#[tauri::command]
fn delete_session(state: State<'_, DesktopState>, session_id: String) -> Result<Value, String> {
    state.store.delete_session(&session_id);
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn list_tool_history(state: State<'_, DesktopState>) -> Result<Vec<ToolExecutionRecord>, String> {
    Ok(state.store.list_tool_history(150))
}

#[tauri::command]
async fn probe_ollama_models(base_url: String) -> Result<Value, String> {
    let url = format!("{}/api/tags", base_url.trim().trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let v: Value = resp.json().await.unwrap_or_default();
            let models: Vec<String> = v["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({ "online": true, "models": models }))
        }
        _ => Ok(json!({ "online": false, "models": [] })),
    }
}

#[tauri::command]
fn pty_spawn(
    app: AppHandle,
    state: State<'_, DesktopState>,
    shell: String,
    cwd: String,
    cols: u16,
    rows: u16,
    env: Option<HashMap<String, String>>,
) -> Result<PtyTabInfo, String> {
    let effective_cwd = if cwd.trim().is_empty() {
        state
            .project_root
            .lock()
            .unwrap()
            .to_string_lossy()
            .to_string()
    } else {
        cwd
    };
    state
        .pty
        .spawn(app, &shell, &effective_cwd, cols, rows, env)
}

#[tauri::command]
fn pty_write(state: State<'_, DesktopState>, pty_id: String, data: String) -> Result<Value, String> {
    state.pty.write(&pty_id, &data)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn pty_resize(
    state: State<'_, DesktopState>,
    pty_id: String,
    cols: u16,
    rows: u16,
) -> Result<Value, String> {
    state.pty.resize(&pty_id, cols, rows)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
fn pty_kill(state: State<'_, DesktopState>, pty_id: String) -> Result<Value, String> {
    state.pty.kill(&pty_id)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn start_agent_run(
    app: AppHandle,
    state: State<'_, DesktopState>,
    session_id: String,
    prompt: String,
    mode: String,
    provider: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    let store = state.store.clone();
    let gate = state.gate.clone();
    let root = state.project_root.lock().unwrap().clone();

    tokio::spawn(async move {
        let _ = agent::run_autonomous_loop(
            app,
            &store,
            &gate,
            &root,
            &session_id,
            &prompt,
            &mode,
            provider,
            model,
        )
        .await;
    });

    Ok(json!({ "started": true }))
}

#[tauri::command]
fn emergency_stop(state: State<'_, DesktopState>) -> Result<Value, String> {
    state.gate.trigger_emergency_stop();
    Ok(json!({ "stopped": true }))
}

#[tauri::command]
fn respond_permission(
    state: State<'_, DesktopState>,
    request_id: String,
    approved: bool,
) -> Result<Value, String> {
    let resolved = state.gate.resolve(&request_id, approved);
    Ok(json!({ "resolved": resolved }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("AI-Coding-Agent"));
            let store = Arc::new(
                SqliteStore::open(&app_data_dir).expect("Failed to initialize SQLite database"),
            );
            let default_project = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("C:\\"));

            app.manage(DesktopState {
                app_data_dir,
                store,
                project_root: Mutex::new(default_project),
                watcher: FsWatcherManager::new(),
                pty: PtyManager::new(),
                gate: Arc::new(PermissionGate::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_init,
            pick_project_folder,
            set_project_folder,
            list_project_files,
            read_project_file,
            write_project_file,
            create_fs_item,
            delete_fs_item,
            reveal_in_explorer,
            search_in_project,
            git_status,
            git_file_diff,
            git_stage,
            git_unstage,
            git_commit,
            git_checkout,
            save_settings,
            list_sessions,
            create_session,
            load_session,
            delete_session,
            list_tool_history,
            probe_ollama_models,
            pty_spawn,
            pty_write,
            pty_resize,
            pty_kill,
            start_agent_run,
            emergency_stop,
            respond_permission,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AI Coding Agent Tauri desktop application");
}
