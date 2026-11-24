use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequestEvent {
    pub request_id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub command: Option<String>,
    pub target_path: Option<String>,
    pub cwd: Option<String>,
}

pub struct PermissionGate {
    stop_flag: AtomicBool,
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl PermissionGate {
    pub fn new() -> Self {
        Self {
            stop_flag: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn reset_stop(&self) {
        self.stop_flag.store(false, Ordering::SeqCst);
    }

    pub fn trigger_emergency_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let mut guard = self.pending.lock().unwrap();
        for (_, tx) in guard.drain() {
            let _ = tx.send(false);
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    pub fn resolve(&self, request_id: &str, approved: bool) -> bool {
        let mut guard = self.pending.lock().unwrap();
        if let Some(tx) = guard.remove(request_id) {
            let _ = tx.send(approved);
            true
        } else {
            false
        }
    }

    pub async fn request_approval(
        &self,
        app: &AppHandle,
        kind: &str,
        title: &str,
        detail: &str,
        command: Option<String>,
        target_path: Option<String>,
        cwd: Option<String>,
    ) -> bool {
        if self.is_stopped() {
            return false;
        }
        let request_id = format!("perm_{}", Uuid::new_v4().simple());
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.pending.lock().unwrap();
            guard.insert(request_id.clone(), tx);
        }

        let payload = PermissionRequestEvent {
            request_id,
            kind: kind.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            command,
            target_path,
            cwd,
        };
        let _ = app.emit("agent://permission_request", payload);

        rx.await.unwrap_or(false)
    }
}
