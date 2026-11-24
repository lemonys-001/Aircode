import React, { useState } from "react";
import {
  GitBranch,
  GitCommitHorizontal,
  Plus,
  Minus,
  RefreshCw,
  FileDiff,
} from "lucide-react";
import { GitStatusPayload } from "../types/desktop";

interface Props {
  git: GitStatusPayload;
  onRefresh: () => void;
  onStage: (path: string, stageAll?: boolean) => void;
  onUnstage: (path: string) => void;
  onCommit: (message: string) => void;
  onOpenDiff: (path: string) => void;
  onCheckoutBranch: (branch: string, createNew: boolean) => void;
}

export const GitPanel: React.FC<Props> = ({
  git,
  onRefresh,
  onStage,
  onUnstage,
  onCommit,
  onOpenDiff,
  onCheckoutBranch,
}) => {
  const [commitMsg, setCommitMsg] = useState("");

  if (!git.isRepo) {
    return (
      <div className="side-pane">
        <div className="pane-header">
          <span>GIT SOURCE CONTROL</span>
          <button className="tiny-icon-btn" onClick={onRefresh}>
            <RefreshCw size={14} />
          </button>
        </div>
        <div className="empty-side-note">
          Selected directory is not a Git repository.
        </div>
      </div>
    );
  }

  return (
    <div className="side-pane">
      <div className="pane-header">
        <span>SOURCE CONTROL ({git.files.length})</span>
        <div className="pane-header-btns">
          <button
            className="glass-btn tiny"
            onClick={() => onStage("", true)}
            title="Stage All Changes"
          >
            + All
          </button>
          <button className="tiny-icon-btn" onClick={onRefresh}>
            <RefreshCw size={14} />
          </button>
        </div>
      </div>

      <div className="git-branch-box">
        <div className="git-branch-row">
          <GitBranch size={14} className="accent-icon" />
          <select
            value={git.branch}
            onChange={(e) => onCheckoutBranch(e.target.value, false)}
          >
            {!git.branches.includes(git.branch) && (
              <option value={git.branch}>{git.branch}</option>
            )}
            {git.branches.map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
          <button
            className="glass-btn tiny"
            onClick={() => {
              const name = window.prompt("New branch name:");
              if (name) onCheckoutBranch(name, true);
            }}
          >
            + Branch
          </button>
        </div>

        <textarea
          className="git-commit-input"
          rows={2}
          value={commitMsg}
          onChange={(e) => setCommitMsg(e.target.value)}
          placeholder="Commit message..."
        />
        <button
          className="glass-btn accent full-width"
          disabled={!commitMsg.trim()}
          onClick={() => {
            onCommit(commitMsg.trim());
            setCommitMsg("");
          }}
        >
          <GitCommitHorizontal size={14} /> Commit Changes
        </button>
      </div>

      <div className="explorer-scroll">
        <div className="section-caption">Modified &amp; Untracked Files</div>
        {git.files.length === 0 ? (
          <div className="muted-pad">Working tree is clean.</div>
        ) : (
          git.files.map((f) => (
            <div key={f.path} className="git-file-row">
              <div
                className="git-file-main"
                onClick={() => onOpenDiff(f.path)}
                title="Click to inspect Side-by-Side Diff in Monaco Editor"
              >
                <span className="git-status-code">{f.status}</span>
                <span className="git-file-name">{f.path}</span>
              </div>
              <div className="git-file-btns">
                <button
                  className="tiny-icon-btn"
                  title="Open Diff"
                  onClick={() => onOpenDiff(f.path)}
                >
                  <FileDiff size={12} />
                </button>
                {f.staged ? (
                  <button
                    className="tiny-icon-btn"
                    title="Unstage"
                    onClick={() => onUnstage(f.path)}
                  >
                    <Minus size={12} />
                  </button>
                ) : (
                  <button
                    className="tiny-icon-btn"
                    title="Stage"
                    onClick={() => onStage(f.path, false)}
                  >
                    <Plus size={12} />
                  </button>
                )}
              </div>
            </div>
          ))
        )}

        <div className="section-caption">Recent Commits</div>
        {git.commits.map((c) => (
          <div key={c.hash} className="git-commit-row">
            <code>{c.hash}</code>
            <span className="git-commit-subj" title={c.subject}>
              {c.subject}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
};
