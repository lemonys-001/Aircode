//! Native Git integration (status, branch switching, staging, commits, and HEAD file diffs).

use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug)]
pub struct GitChangedFile {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

#[derive(Clone, Debug)]
pub struct GitCommitSummary {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

#[derive(Clone, Debug, Default)]
pub struct GitStatusSnapshot {
    pub is_repo: bool,
    pub branch: String,
    pub branches: Vec<String>,
    pub files: Vec<GitChangedFile>,
    pub commits: Vec<GitCommitSummary>,
}

pub fn run_git(cwd: &Path, args: &[&str]) -> Result<(i32, String, String), String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args);
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed running git: {}", e))?;
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok((code, stdout, stderr))
}

pub fn inspect_git_status(root: &Path) -> GitStatusSnapshot {
    let Ok((code, _, _)) = run_git(root, &["rev-parse", "--is-inside-work-tree"]) else {
        return GitStatusSnapshot::default();
    };
    if code != 0 {
        return GitStatusSnapshot::default();
    }

    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|(_, o, _)| o.trim().to_string())
        .unwrap_or_else(|_| "HEAD".to_string());

    let branches = run_git(root, &["branch", "--format=%(refname:short)"])
        .map(|(_, o, _)| {
            o.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut files = Vec::new();
    if let Ok((_, porcelain, _)) = run_git(root, &["status", "--porcelain=v1"]) {
        for line in porcelain.lines() {
            if line.len() >= 3 {
                let idx = &line[0..1];
                let wt = &line[1..2];
                let path = line[3..].trim().trim_matches('"').to_string();
                let staged = idx != " " && idx != "?";
                let status = if idx == "?" {
                    "U".to_string()
                } else if staged {
                    idx.to_string()
                } else {
                    wt.to_string()
                };
                files.push(GitChangedFile {
                    path,
                    status,
                    staged,
                });
            }
        }
    }

    let mut commits = Vec::new();
    if let Ok((_, log_out, _)) = run_git(
        root,
        &["log", "-n", "10", "--pretty=format:%h\t%s\t%an\t%ar"],
    ) {
        for line in log_out.lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() == 4 {
                commits.push(GitCommitSummary {
                    hash: parts[0].to_string(),
                    subject: parts[1].to_string(),
                    author: parts[2].to_string(),
                    date: parts[3].to_string(),
                });
            }
        }
    }

    GitStatusSnapshot {
        is_repo: true,
        branch,
        branches,
        files,
        commits,
    }
}

pub fn load_head_and_working_diff(root: &Path, rel_path: &str) -> (String, String) {
    let norm = rel_path.replace('\\', "/");
    let spec = format!("HEAD:{}", norm);
    let orig = match run_git(root, &["show", &spec]) {
        Ok((0, out, _)) => out,
        _ => String::new(),
    };
    let modified = std::fs::read_to_string(root.join(rel_path)).unwrap_or_default();
    (orig, modified)
}
