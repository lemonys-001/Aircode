import React, { useEffect, useState } from "react";
import { X, Save, RefreshCw, Database, Shield, KeyRound, History } from "lucide-react";
import { AppSettings, ToolExecutionRecord } from "../types/desktop";
import { desktopInvoke } from "../lib/tauri";

interface Props {
  open: boolean;
  initialTab: "providers" | "permissions" | "history";
  settings: AppSettings;
  sqlitePath: string;
  appDataDir: string;
  onClose: () => void;
  onSaveSettings: (next: AppSettings) => Promise<void>;
}

export const SettingsModal: React.FC<Props> = ({
  open,
  initialTab,
  settings,
  sqlitePath,
  appDataDir,
  onClose,
  onSaveSettings,
}) => {
  const [tab, setTab] = useState<"providers" | "permissions" | "history">(
    initialTab
  );
  const [draft, setDraft] = useState<AppSettings>(settings);
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaOnline, setOllamaOnline] = useState<boolean | null>(null);
  const [toolHistory, setToolHistory] = useState<ToolExecutionRecord[]>([]);

  useEffect(() => {
    setTab(initialTab);
  }, [initialTab]);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  useEffect(() => {
    if (!open) return;
    if (tab === "history") {
      desktopInvoke<ToolExecutionRecord[]>("list_tool_history", {})
        .then(setToolHistory)
        .catch(() => {});
    }
  }, [open, tab]);

  if (!open) return null;

  const probeOllama = async () => {
    try {
      const res = await desktopInvoke<{ online: boolean; models: string[] }>(
        "probe_ollama_models",
        { baseUrl: draft.ollamaBaseUrl }
      );
      setOllamaOnline(res.online);
      setOllamaModels(res.models || []);
    } catch {
      setOllamaOnline(false);
    }
  };

  return (
    <div className="desktop-modal-backdrop">
      <div className="desktop-modal settings-dialog">
        <div className="modal-head">
          <div className="modal-title-row">
            <Database size={17} className="accent-icon" />
            <h3>Application Settings &amp; SQLite Audit Store</h3>
          </div>
          <button className="tiny-icon-btn" onClick={onClose}>
            <X size={16} />
          </button>
        </div>

        <div className="modal-subtabs">
          <button
            className={tab === "providers" ? "active" : ""}
            onClick={() => setTab("providers")}
          >
            <KeyRound size={13} /> AI Providers &amp; Endpoints
          </button>
          <button
            className={tab === "permissions" ? "active" : ""}
            onClick={() => setTab("permissions")}
          >
            <Shield size={13} /> Agent Permissions &amp; Terminal
          </button>
          <button
            className={tab === "history" ? "active" : ""}
            onClick={() => setTab("history")}
          >
            <History size={13} /> Tool Execution History ({toolHistory.length})
          </button>
        </div>

        <div className="modal-content scrollable">
          {tab === "providers" && (
            <div className="settings-form-grid">
              <div className="sqlite-meta-banner">
                <div>
                  <strong>SQLite Database:</strong> <code>{sqlitePath}</code>
                </div>
                <div>
                  <strong>AppData Directory:</strong> <code>{appDataDir}</code>
                </div>
              </div>

              <label>
                <span>OpenAI API Key</span>
                <input
                  type="password"
                  value={draft.openaiApiKey}
                  onChange={(e) =>
                    setDraft({ ...draft, openaiApiKey: e.target.value })
                  }
                  placeholder="sk-..."
                />
              </label>

              <label>
                <span>Anthropic API Key</span>
                <input
                  type="password"
                  value={draft.anthropicApiKey}
                  onChange={(e) =>
                    setDraft({ ...draft, anthropicApiKey: e.target.value })
                  }
                  placeholder="sk-ant-..."
                />
              </label>

              <label>
                <span>Google Gemini API Key</span>
                <input
                  type="password"
                  value={draft.geminiApiKey}
                  onChange={(e) =>
                    setDraft({ ...draft, geminiApiKey: e.target.value })
                  }
                  placeholder="AIza..."
                />
              </label>

              <label>
                <span>OpenRouter API Key</span>
                <input
                  type="password"
                  value={draft.openrouterApiKey}
                  onChange={(e) =>
                    setDraft({ ...draft, openrouterApiKey: e.target.value })
                  }
                  placeholder="sk-or-v1-..."
                />
              </label>

              <div className="ollama-row">
                <label style={{ flex: 1 }}>
                  <span>Ollama Local Base URL</span>
                  <input
                    type="text"
                    value={draft.ollamaBaseUrl}
                    onChange={(e) =>
                      setDraft({ ...draft, ollamaBaseUrl: e.target.value })
                    }
                    placeholder="http://127.0.0.1:11434"
                  />
                </label>
                <button className="glass-btn" onClick={probeOllama}>
                  <RefreshCw size={13} /> Detect Local Models
                </button>
              </div>
              {ollamaOnline !== null && (
                <div className="ollama-probe-status">
                  {ollamaOnline ? (
                    <>
                      <span className="ok-text">● Ollama Connected</span> —{" "}
                      {ollamaModels.length
                        ? ollamaModels.map((m) => (
                            <button
                              key={m}
                              className="glass-btn tiny"
                              onClick={() =>
                                setDraft({
                                  ...draft,
                                  activeProvider: "ollama",
                                  activeModel: m,
                                })
                              }
                            >
                              Use {m}
                            </button>
                          ))
                        : "No models pulled yet (`ollama pull qwen2.5-coder:7b`)"}
                    </>
                  ) : (
                    <span className="warn-text">
                      Ollama server not detected at {draft.ollamaBaseUrl}
                    </span>
                  )}
                </div>
              )}

              <label>
                <span>Custom OpenAI-Compatible Endpoint Base URL</span>
                <input
                  type="text"
                  value={draft.customBaseUrl}
                  onChange={(e) =>
                    setDraft({ ...draft, customBaseUrl: e.target.value })
                  }
                  placeholder="https://your-endpoint.example.com/v1"
                />
              </label>

              <label>
                <span>Custom Endpoint API Key</span>
                <input
                  type="password"
                  value={draft.customApiKey}
                  onChange={(e) =>
                    setDraft({ ...draft, customApiKey: e.target.value })
                  }
                  placeholder="Optional Bearer token"
                />
              </label>

              <label>
                <span>Active Model Override (Any Custom Model ID)</span>
                <input
                  type="text"
                  value={draft.activeModel}
                  onChange={(e) =>
                    setDraft({ ...draft, activeModel: e.target.value })
                  }
                />
              </label>
            </div>
          )}

          {tab === "permissions" && (
            <div className="settings-form-grid">
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={draft.requireShellApproval}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      requireShellApproval: e.target.checked,
                    })
                  }
                />
                <div>
                  <strong>Require Shell Command Approval</strong>
                  <p>
                    Prompt before the AI agent executes shell commands, dependency
                    installs, or test runners.
                  </p>
                </div>
              </label>

              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={draft.requireDeleteConfirmation}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      requireDeleteConfirmation: e.target.checked,
                    })
                  }
                />
                <div>
                  <strong>Require File Deletion Confirmation</strong>
                  <p>
                    Always show confirmation dialog before deleting any file or
                    directory from Windows disk.
                  </p>
                </div>
              </label>

              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={draft.restrictToWorkspace}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      restrictToWorkspace: e.target.checked,
                    })
                  }
                />
                <div>
                  <strong>Enforce Workspace Directory Restriction</strong>
                  <p>
                    Block agent writes, edits, and deletions outside the selected
                    Windows project root.
                  </p>
                </div>
              </label>

              <div className="two-col-fields">
                <label>
                  <span>Default ConPTY Shell</span>
                  <select
                    value={draft.defaultShell}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        defaultShell: e.target.value as "powershell" | "cmd",
                      })
                    }
                  >
                    <option value="powershell">Windows PowerShell</option>
                    <option value="cmd">Command Prompt (CMD.exe)</option>
                  </select>
                </label>

                <label>
                  <span>Max Continuous Agent Loop Steps</span>
                  <input
                    type="number"
                    min={1}
                    max={40}
                    value={draft.maxAgentIterations}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        maxAgentIterations: Number(e.target.value) || 15,
                      })
                    }
                  />
                </label>

                <label>
                  <span>Monaco Editor Font Size (px)</span>
                  <input
                    type="number"
                    min={11}
                    max={24}
                    value={draft.editorFontSize}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        editorFontSize: Number(e.target.value) || 13,
                      })
                    }
                  />
                </label>
              </div>
            </div>
          )}

          {tab === "history" && (
            <div className="tool-audit-list">
              {toolHistory.length === 0 ? (
                <div className="muted-pad">
                  No tool executions recorded in SQLite yet.
                </div>
              ) : (
                toolHistory.map((item) => (
                  <div key={item.id} className="audit-row">
                    <div className="audit-row-top">
                      <span className={`audit-badge ${item.status}`}>
                        {item.status.toUpperCase()}
                      </span>
                      <strong>{item.toolName}</strong>
                      <span className="audit-meta">
                        {item.durationMs}ms · {item.createdAt}
                      </span>
                    </div>
                    <div className="audit-summary">{item.resultSummary}</div>
                    <pre className="audit-args">{item.argsJson}</pre>
                  </div>
                ))
              )}
            </div>
          )}
        </div>

        {tab !== "history" && (
          <div className="modal-actions">
            <button className="glass-btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="glass-btn accent"
              onClick={async () => {
                await onSaveSettings(draft);
                onClose();
              }}
            >
              <Save size={14} /> Save to SQLite
            </button>
          </div>
        )}
      </div>
    </div>
  );
};
