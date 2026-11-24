use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyTabInfo {
    pub pty_id: String,
    pub shell: String,
    pub title: String,
    pub cwd: String,
    pub alive: bool,
}

struct ActivePtySession {
    info: PtyTabInfo,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, ActivePtySession>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn spawn(
        &self,
        app: AppHandle,
        shell: &str,
        cwd: &str,
        cols: u16,
        rows: u16,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<PtyTabInfo, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: rows.max(10),
                cols: cols.max(30),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("ConPTY openpty failed: {}", e))?;

        let (exe, args, label) = if shell.eq_ignore_ascii_case("cmd") {
            ("cmd.exe", vec![], "CMD (ConPTY)")
        } else {
            ("powershell.exe", vec!["-NoLogo"], "PowerShell (ConPTY)")
        };

        let mut cmd = CommandBuilder::new(exe);
        for a in args {
            cmd.arg(a);
        }
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        if let Some(extra) = env_vars {
            for (k, v) in extra {
                cmd.env(k, v);
            }
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn {} in ConPTY: {}", exe, e))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone ConPTY reader: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to take ConPTY writer: {}", e))?;

        let id = format!("pty_{}", Uuid::new_v4().simple());
        let info = PtyTabInfo {
            pty_id: id.clone(),
            shell: shell.to_lowercase(),
            title: label.to_string(),
            cwd: cwd.to_string(),
            alive: true,
        };

        {
            let mut guard = self.sessions.lock().unwrap();
            guard.insert(
                id.clone(),
                ActivePtySession {
                    info: info.clone(),
                    master: pair.master,
                    writer,
                    child,
                },
            );
        }

        let id_reader = id.clone();
        let sessions_ref = self.sessions.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        let _ = app.emit(
                            "pty://data",
                            serde_json::json!({
                                "ptyId": id_reader,
                                "data": text
                            }),
                        );
                    }
                    Err(_) => break,
                }
            }
            let code = {
                let mut guard = sessions_ref.lock().unwrap();
                if let Some(sess) = guard.get_mut(&id_reader) {
                    sess.info.alive = false;
                    sess.child.wait().map(|s| s.exit_code() as i32).unwrap_or(0)
                } else {
                    0
                }
            };
            let _ = app.emit(
                "pty://exit",
                serde_json::json!({
                    "ptyId": id_reader,
                    "exitCode": code
                }),
            );
        });

        Ok(info)
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let mut guard = self.sessions.lock().unwrap();
        let sess = guard
            .get_mut(id)
            .ok_or_else(|| format!("PTY session '{}' not found", id))?;
        sess.writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        sess.writer.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let guard = self.sessions.lock().unwrap();
        if let Some(sess) = guard.get(id) {
            sess.master
                .resize(PtySize {
                    rows: rows.max(5),
                    cols: cols.max(20),
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn kill(&self, id: &str) -> Result<(), String> {
        let mut guard = self.sessions.lock().unwrap();
        if let Some(mut sess) = guard.remove(id) {
            let _ = sess.child.kill();
        }
        Ok(())
    }
}
