use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub letter: String,
    pub mount_path: String,
    pub drive_type: String,
    pub total_gb: f64,
    pub free_gb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size: u64,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub file: String,
    pub rel_path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

pub struct FsWatcherManager {
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl FsWatcherManager {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
        }
    }

    pub fn watch_directory(&self, app: AppHandle, dir: &Path) {
        let mut guard = self.watcher.lock().unwrap();
        *guard = None;

        let dir_buf = dir.to_path_buf();
        let watcher_res = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if let Some(first) = event.paths.first() {
                        let p_str = first.to_string_lossy().to_string();
                        if !p_str.contains(".git") && !p_str.contains("node_modules") && !p_str.contains("target") {
                            let _ = app.emit(
                                "fs://changed",
                                serde_json::json!({
                                    "kind": format!("{:?}", event.kind),
                                    "path": p_str
                                }),
                            );
                        }
                    }
                }
            },
            Config::default(),
        );

        if let Ok(mut w) = watcher_res {
            let _ = w.watch(&dir_buf, RecursiveMode::Recursive);
            *guard = Some(w);
        }
    }
}

pub fn list_windows_drives() -> Vec<DriveInfo> {
    let mut out = Vec::new();
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Storage::FileSystem::{
            GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
        };
        let mask = GetLogicalDrives();
        for i in 0..26u32 {
            if (mask & (1 << i)) != 0 {
                let letter_char = (b'A' + i as u8) as char;
                let mount = format!("{}:\\", letter_char);
                let mut wide: Vec<u16> = mount.encode_utf16().collect();
                wide.push(0);

                let dtype_code = GetDriveTypeW(wide.as_ptr());
                let drive_type = match dtype_code {
                    2 => "Removable",
                    3 => "Local Disk",
                    4 => "Network",
                    5 => "Optical",
                    6 => "RAM Disk",
                    _ => "Drive",
                }
                .to_string();

                let mut free_avail: u64 = 0;
                let mut total_bytes: u64 = 0;
                let mut total_free: u64 = 0;
                GetDiskFreeSpaceExW(
                    wide.as_ptr(),
                    &mut free_avail,
                    &mut total_bytes,
                    &mut total_free,
                );

                out.push(DriveInfo {
                    letter: format!("{}:", letter_char),
                    mount_path: mount,
                    drive_type,
                    total_gb: (total_bytes as f64) / 1_073_741_824.0,
                    free_gb: (free_avail as f64) / 1_073_741_824.0,
                });
            }
        }
    }
    out
}

pub fn ignored_dir_names() -> HashSet<&'static str> {
    [
        ".git",
        "node_modules",
        "target",
        "ui-dist",
        ".next",
        ".venv",
        "venv",
        "__pycache__",
        ".vs",
        "obj",
    ]
    .into_iter()
    .collect()
}

pub fn scan_project_tree(root: &Path, max_entries: usize) -> Vec<FileEntry> {
    let ignored = ignored_dir_names();
    let mut entries = Vec::new();

    fn walk(
        base: &Path,
        curr: &Path,
        ignored: &HashSet<&'static str>,
        out: &mut Vec<FileEntry>,
        max: usize,
    ) {
        if out.len() >= max {
            return;
        }
        let rd = match std::fs::read_dir(curr) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut items: Vec<_> = rd.filter_map(Result::ok).collect();
        items.sort_by_key(|e| e.file_name());

        for item in items {
            if out.len() >= max {
                break;
            }
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_string();
            let meta = match item.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_dir = meta.is_dir();
            if is_dir && ignored.contains(name.as_str()) {
                continue;
            }
            let rel_path = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            out.push(FileEntry {
                name,
                path: path.to_string_lossy().to_string(),
                rel_path,
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                readonly: meta.permissions().readonly(),
            });

            if is_dir {
                walk(base, &path, ignored, out, max);
            }
        }
    }

    walk(root, root, &ignored, &mut entries, max_entries);
    entries
}

pub fn resolve_project_path(
    project_root: &Path,
    candidate: &str,
    restrict_to_workspace: bool,
) -> Result<PathBuf, String> {
    let raw = PathBuf::from(candidate);
    let joined = if raw.is_absolute() {
        raw
    } else {
        project_root.join(raw)
    };

    if restrict_to_workspace {
        let canon_root = std::fs::canonicalize(project_root)
            .unwrap_or_else(|_| project_root.to_path_buf());
        let check_target = if joined.exists() {
            std::fs::canonicalize(&joined).unwrap_or_else(|_| joined.clone())
        } else if let Some(parent) = joined.parent() {
            std::fs::canonicalize(parent)
                .map(|p| p.join(joined.file_name().unwrap_or_default()))
                .unwrap_or_else(|_| joined.clone())
        } else {
            joined.clone()
        };

        let root_norm = canon_root.to_string_lossy().to_lowercase();
        let target_norm = check_target.to_string_lossy().to_lowercase();
        if !target_norm.starts_with(&root_norm) {
            return Err(format!(
                "Workspace Sandbox Violation: Path '{}' is outside active project '{}'",
                joined.display(),
                project_root.display()
            ));
        }
    }

    Ok(joined)
}

pub fn search_project_files(
    root: &Path,
    query: &str,
    case_sensitive: bool,
    max_hits: usize,
) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let tree = scan_project_tree(root, 2500);
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut hits = Vec::new();

    for entry in tree {
        if entry.is_dir || entry.size > 1_048_576 {
            continue;
        }
        let text = match std::fs::read_to_string(&entry.path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (idx, line) in text.lines().enumerate() {
            let hay = if case_sensitive {
                line.to_string()
            } else {
                line.to_lowercase()
            };
            if let Some(col) = hay.find(&needle) {
                hits.push(SearchMatch {
                    file: entry.path.clone(),
                    rel_path: entry.rel_path.clone(),
                    line: idx + 1,
                    column: col + 1,
                    preview: line.chars().take(200).collect(),
                });
                if hits.len() >= max_hits {
                    return hits;
                }
            }
        }
    }
    hits
}

pub fn compute_unified_diff(rel_path: &str, old_text: &str, new_text: &str) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > prefix
        && new_end > prefix
        && old_lines[old_end - 1] == new_lines[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }
    if prefix == old_end && prefix == new_end {
        return String::new();
    }
    let mut out = format!(
        "--- a/{0}\n+++ b/{0}\n@@ -{1},{2} +{1},{3} @@\n",
        rel_path,
        prefix + 1,
        old_end - prefix,
        new_end - prefix
    );
    for l in &old_lines[prefix..old_end] {
        out.push('-');
        out.push_str(l);
        out.push('\n');
    }
    for l in &new_lines[prefix..new_end] {
        out.push('+');
        out.push_str(l);
        out.push('\n');
    }
    out
}
