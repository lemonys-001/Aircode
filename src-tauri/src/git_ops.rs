use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChangedFile {
    pub path: String,
    pub status: String,
    pub staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitSummary {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusPayload {
    pub is_repo: bool,
    pub branch: String,
    pub branches: Vec<String>,
    pub files: Vec<GitChangedFile>,
    pub commits: Vec<GitCommitSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileDiff {
    pub original: String,
    pub modified: String,
    pub unified_diff: String,
}

pub fn run_git(cwd: &Path, args: &[&str]) -> Result<(i32, String, String), String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed to invoke git: {}", e))?;
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok((code, stdout, stderr))
}

pub fn inspect_git_status(root: &Path) -> GitStatusPayload {
    let (code, _, _) = match run_git(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(r) => r,
        Err(_) => {
            return GitStatusPayload {
                is_repo: false,
                branch: String::new(),
                branches: Vec::new(),
                files: Vec::new(),
                commits: Vec::new(),
            }
        }
    };
    if code != 0 {
        return GitStatusPayload {
            is_repo: false,
            branch: String::new(),
            branches: Vec::new(),
            files: Vec::new(),
            commits: Vec::new(),
        };
    }

    let branch = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|(_, out, _)| out.trim().to_string())
        .unwrap_or_else(|_| "HEAD".to_string());

    let branches = run_git(root, &["branch", "--format=%(refname:short)"])
        .map(|(_, out, _)| {
            out.lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut files = Vec::new();
    if let Ok((_, porcelain, _)) = run_git(root, &["status", "--porcelain=v1"]) {
        for line in porcelain.lines() {
            if line.len() >= 3 {
                let idx = &line[0..1];
                let wt = &line[1..2];
                let p = line[3..].trim().trim_matches('"').to_string();
                let staged = idx != " " && idx != "?";
                let status_code = if idx == "?" {
                    "U".to_string()
                } else if staged {
                    idx.to_string()
                } else {
                    wt.to_string()
                };
                files.push(GitChangedFile {
                    path: p,
                    status: status_code,
                    staged,
                });
            }
        }
    }

    let mut commits = Vec::new();
    if let Ok((_, log_out, _)) = run_git(
        root,
        &["log", "-n", "12", "--pretty=format:%h\t%s\t%an\t%ar"],
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

    GitStatusPayload {
        is_repo: true,
        branch,
        branches,
        files,
        commits,
    }
}

pub fn get_file_diff(root: &Path, rel_path: &str) -> Result<GitFileDiff, String> {
    let norm = rel_path.replace('\\', "/");
    let head_spec = format!("HEAD:{}", norm);
    let original = match run_git(root, &["show", &head_spec]) {
        Ok((0, out, _)) => out,
        _ => String::new(),
    };
    let disk_path = root.join(rel_path);
    let modified = std::fs::read_to_string(&disk_path).unwrap_or_default();
    let unified_diff = run_git(root, &["diff", "HEAD", "--", &norm])
        .map(|(_, out, _)| out)
        .unwrap_or_default();

    Ok(GitFileDiff {
        original,
        modified,
        unified_diff,
    })
}
