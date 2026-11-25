//! Cross-platform workspace file explorer tree, canonical sandbox guard, search, and diff helpers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DriveInfo {
    pub letter: String,
    pub mount_path: String,
    pub drive_type: String,
    pub total_gb: f64,
    pub free_gb: f64,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub rel_path: String,
    pub depth: usize,
    pub is_dir: bool,
    pub size: u64,
    pub readonly: bool,
}

#[derive(Clone, Debug)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub rel_path: String,
    pub line: usize,
    pub column: usize,
    pub preview: String,
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | ".next"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".vs"
            | "obj"
    )
}

pub fn scan_project_tree(
    root: &Path,
    collapsed_dirs: &HashSet<String>,
    include_collapsed_children: bool,
    max_entries: usize,
) -> Vec<FileEntry> {
    let mut out = Vec::new();

    fn walk(
        base: &Path,
        curr: &Path,
        depth: usize,
        collapsed: &HashSet<String>,
        include_all: bool,
        out: &mut Vec<FileEntry>,
        max: usize,
    ) {
        if out.len() >= max {
            return;
        }
        let Ok(rd) = std::fs::read_dir(curr) else {
            return;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
        entries.sort_by(|a, b| {
            let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
            match (a_dir, b_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .file_name()
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&b.file_name().to_string_lossy().to_lowercase()),
            }
        });

        for item in entries {
            if out.len() >= max {
                break;
            }
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_string();
            let Ok(meta) = item.metadata() else {
                continue;
            };
            let is_dir = meta.is_dir();
            if is_dir && is_ignored_dir(&name) {
                continue;
            }
            let rel_path = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            out.push(FileEntry {
                name,
                path: path.clone(),
                rel_path: rel_path.clone(),
                depth,
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                readonly: meta.permissions().readonly(),
            });

            if is_dir && (include_all || !collapsed.contains(&rel_path)) {
                walk(base, &path, depth + 1, collapsed, include_all, out, max);
            }
        }
    }

    walk(
        root,
        root,
        0,
        collapsed_dirs,
        include_collapsed_children,
        &mut out,
        max_entries,
    );
    out
}

pub fn resolve_workspace_path(
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
        let canon_root =
            std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
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
                "Workspace Sandbox Violation: '{}' is outside active workspace '{}'",
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
    if query.trim().is_empty() {
        return Vec::new();
    }
    let all = scan_project_tree(root, &HashSet::new(), true, 2500);
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut hits = Vec::new();

    for entry in all {
        if entry.is_dir || entry.size > 1_048_576 {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            let hay = if case_sensitive {
                line.to_string()
            } else {
                line.to_lowercase()
            };
            if let Some(pos) = hay.find(&needle) {
                hits.push(SearchMatch {
                    path: entry.path.clone(),
                    rel_path: entry.rel_path.clone(),
                    line: idx + 1,
                    column: pos + 1,
                    preview: line.trim().chars().take(120).collect(),
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
