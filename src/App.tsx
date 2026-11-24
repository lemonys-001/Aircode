import React, { useCallback, useEffect, useState } from "react";
import { FolderTree, GitBranch } from "lucide-react";
import {
  AgentMode,
  AgentStreamEvent,
  AppInitPayload,
  AppSettings,
  DriveInfo,
  FileEntry,
  GitStatusPayload,
  OpenEditorTab,
  PermissionRequestEvent,
  ProviderId,
  SearchMatch,
  SessionDetail,
  SessionSummary,
  ToolExecutionRecord,
} from "./types/desktop";
import {
  desktopInvoke,
  desktopListen,
  inferMonacoLanguage,
} from "./lib/tauri";
import { TitlebarStatus } from "./components/TitlebarStatus";
import { FileExplorer } from "./components/FileExplorer";
import { MonacoWorkbench } from "./components/MonacoWorkbench";
import { ConPtyTerminal } from "./components/ConPtyTerminal";
import { GitPanel } from "./components/GitPanel";
import { AgentPanel } from "./components/AgentPanel";
import { ApprovalModal } from "./components/ApprovalModal";
import { SettingsModal } from "./components/SettingsModal";

const DEFAULT_SETTINGS: AppSettings = {
  activeProvider: "ollama",
  activeModel: "qwen2.5-coder:7b",
  defaultMode: "autonomous",
  openaiApiKey: "",
  anthropicApiKey: "",
  geminiApiKey: "",
  openrouterApiKey: "",
  ollamaBaseUrl: "http://127.0.0.1:11434",
  customBaseUrl: "",
  customApiKey: "",
  requireShellApproval: true,
  requireDeleteConfirmation: true,
  restrictToWorkspace: true,
  maxAgentIterations: 15,
  defaultShell: "powershell",
  editorFontSize: 13,
};

export const App: React.FC = () => {
  const [bootError, setBootError] = useState<string | null>(null);
  const [currentProject, setCurrentProject] = useState<string>("");
  const [appDataDir, setAppDataDir] = useState<string>("");
  const [sqlitePath, setSqlitePath] = useState<string>("");
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [recentProjects, setRecentProjects] = useState<string[]>([]);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [git, setGit] = useState<GitStatusPayload>({
    isRepo: false,
    branch: "",
    branches: [],
    files: [],
    commits: [],
  });
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [mode, setMode] = useState<AgentMode>("autonomous");

  const [leftTab, setLeftTab] = useState<"explorer" | "git">("explorer");
  const [editorTabs, setEditorTabs] = useState<OpenEditorTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);

  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [activeSession, setActiveSession] = useState<SessionDetail | null>(
    null
  );

  // Live streaming state
  const [agentRunning, setAgentRunning] = useState(false);
  const [liveStatus, setLiveStatus] = useState("");
  const [liveThinking, setLiveThinking] = useState("");
  const [liveText, setLiveText] = useState("");
  const [liveTools, setLiveTools] = useState<ToolExecutionRecord[]>([]);

  // Modals & notifications
  const [pendingPermission, setPendingPermission] =
    useState<PermissionRequestEvent | null>(null);
  const [pendingUserDelete, setPendingUserDelete] = useState<FileEntry | null>(
    null
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<
    "providers" | "permissions" | "history"
  >("providers");
  const [toastMsg, setToastMsg] = useState<string | null>(null);

  const showToast = useCallback((msg: string) => {
    setToastMsg(msg);
    setTimeout(() => {
      setToastMsg((curr) => (curr === msg ? null : curr));
    }, 3200);
  }, []);

  const refreshWorkspace = useCallback(async () => {
    try {
      const [fileList, gitInfo] = await Promise.all([
        desktopInvoke<FileEntry[]>("list_project_files", {}),
        desktopInvoke<GitStatusPayload>("git_status", {}),
      ]);
      setFiles(fileList);
      setGit(gitInfo);
    } catch (err) {
      console.error("Workspace refresh failed:", err);
    }
  }, []);

  const refreshSessions = useCallback(async () => {
    const list = await desktopInvoke<SessionSummary[]>("list_sessions", {});
    setSessions(list);
  }, []);

  useEffect(() => {
    desktopInvoke<AppInitPayload>("app_init", {})
      .then((init) => {
        setCurrentProject(init.currentProject);
        setAppDataDir(init.appDataDir);
        setSqlitePath(init.sqlitePath);
        setDrives(init.drives || []);
        setFiles(init.files || []);
        setGit(init.git);
        setSettings(init.settings);
        setMode(init.settings.defaultMode || "autonomous");
        setSessions(init.sessions || []);
        setActiveSession(init.activeSession);
        setRecentProjects(init.recentProjects || []);
      })
      .catch((err: Error) => {
        setBootError(err.message);
      });
  }, []);

  useEffect(() => {
    let unFs: (() => void) | null = null;
    let unStream: (() => void) | null = null;
    let unPerm: (() => void) | null = null;

    desktopListen<{ kind: string; path: string }>("fs://changed", () => {
      refreshWorkspace();
    }).then((fn) => {
      unFs = fn;
    });

    desktopListen<PermissionRequestEvent>("agent://permission_request", (ev) => {
      setPendingPermission(ev);
    }).then((fn) => {
      unPerm = fn;
    });

    desktopListen<AgentStreamEvent>("agent://stream", (ev) => {
      if (ev.kind === "status") {
        setLiveStatus(ev.statusText || "");
      } else if (ev.kind === "thinking") {
        setLiveThinking((prev) => prev + (ev.text || ""));
      } else if (ev.kind === "text") {
        setLiveText((prev) => prev + (ev.text || ""));
      } else if (ev.kind === "tool_start" && ev.tool) {
        setLiveTools((prev) => [...prev, ev.tool!]);
      } else if (ev.kind === "tool_end" && ev.tool) {
        setLiveTools((prev) =>
          prev.map((t) => (t.id === ev.tool!.id ? ev.tool! : t))
        );
      } else if (ev.kind === "error") {
        setLiveText((prev) => prev + `\n\n⚠️ Error: ${ev.text || ""}`);
      } else if (ev.kind === "done") {
        setAgentRunning(false);
        setLiveStatus("");
        if (ev.session) {
          setActiveSession(ev.session);
        }
        refreshSessions();
        refreshWorkspace();
      }
    }).then((fn) => {
      unStream = fn;
    });

    return () => {
      unFs?.();
      unStream?.();
      unPerm?.();
    };
  }, [refreshSessions, refreshWorkspace]);

  const handlePickFolder = async () => {
    const res = await desktopInvoke<{ path: string | null }>(
      "pick_project_folder",
      {}
    );
    if (res.path) {
      await handleSwitchProject(res.path);
    }
  };

  const handleSwitchProject = async (path: string) => {
    try {
      const snap = await desktopInvoke<{
        root: string;
        files: FileEntry[];
        git: GitStatusPayload;
      }>("set_project_folder", { path });
      setCurrentProject(snap.root);
      setFiles(snap.files);
      setGit(snap.git);
      setEditorTabs([]);
      setActiveTabId(null);
      showToast(`Switched project to ${snap.root}`);
    } catch (err: any) {
      showToast(`Folder switch error: ${err.message}`);
    }
  };

  const handleOpenFile = async (entry: FileEntry) => {
    const existing = editorTabs.find(
      (t) => t.kind === "code" && t.path === entry.path
    );
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    try {
      const data = await desktopInvoke<{
        path: string;
        content: string;
        readonly: boolean;
      }>("read_project_file", { path: entry.path });
      const tabId = `code:${entry.path}`;
      const newTab: OpenEditorTab = {
        id: tabId,
        kind: "code",
        path: entry.path,
        relPath: entry.relPath,
        title: entry.name,
        language: inferMonacoLanguage(entry.name),
        content: data.content,
        savedContent: data.content,
        readonly: data.readonly,
      };
      setEditorTabs((prev) => [...prev, newTab]);
      setActiveTabId(tabId);
    } catch (err: any) {
      showToast(`Cannot read file: ${err.message}`);
    }
  };

  const handleSaveTab = async (tabId: string) => {
    const tab = editorTabs.find((t) => t.id === tabId);
    if (!tab || tab.kind !== "code") return;
    try {
      await desktopInvoke("write_project_file", {
        path: tab.path,
        content: tab.content,
      });
      setEditorTabs((prev) =>
        prev.map((t) =>
          t.id === tabId ? { ...t, savedContent: t.content } : t
        )
      );
      showToast(`Saved ${tab.relPath}`);
      refreshWorkspace();
    } catch (err: any) {
      showToast(`Save failed: ${err.message}`);
    }
  };

  const handleOpenDiff = async (relPath: string) => {
    try {
      const diffData = await desktopInvoke<{
        original: string;
        modified: string;
        unifiedDiff: string;
      }>("git_file_diff", { path: relPath });
      const tabId = `diff:${relPath}`;
      const diffTab: OpenEditorTab = {
        id: tabId,
        kind: "diff",
        path: relPath,
        relPath,
        title: `Diff: ${relPath.split(/[\\/]/).pop()}`,
        language: inferMonacoLanguage(relPath),
        content: diffData.modified,
        savedContent: diffData.modified,
        originalContent: diffData.original,
        readonly: true,
      };
      setEditorTabs((prev) => {
        const filtered = prev.filter((t) => t.id !== tabId);
        return [...filtered, diffTab];
      });
      setActiveTabId(tabId);
    } catch (err: any) {
      showToast(`Diff error: ${err.message}`);
    }
  };

  const handleCreateFsItem = async (relPath: string, isDir: boolean) => {
    await desktopInvoke("create_fs_item", { path: relPath, isDir });
    await refreshWorkspace();
    showToast(`Created ${relPath}`);
  };

  const handleDeleteRequest = async (entry: FileEntry) => {
    if (settings.requireDeleteConfirmation) {
      setPendingUserDelete(entry);
      return;
    }
    await executeConfirmedDelete(entry);
  };

  const executeConfirmedDelete = async (entry: FileEntry) => {
    await desktopInvoke("delete_fs_item", {
      path: entry.path,
      confirmed: true,
    });
    setEditorTabs((prev) => prev.filter((t) => t.path !== entry.path));
    setPendingUserDelete(null);
    await refreshWorkspace();
    showToast(`Deleted ${entry.relPath}`);
  };

  const handleSaveSettings = async (next: AppSettings) => {
    const saved = await desktopInvoke<AppSettings>("save_settings", {
      settings: next,
    });
    setSettings(saved);
    showToast("Settings persisted to SQLite.");
  };

  const handleChangeProviderModel = async (
    provider: ProviderId,
    model: string
  ) => {
    const next = { ...settings, activeProvider: provider, activeModel: model };
    setSettings(next);
    await desktopInvoke("save_settings", { settings: next });
  };

  const handleSendPrompt = async (
    prompt: string,
    overrideMode?: AgentMode
  ) => {
    if (!activeSession) return;
    const targetMode = overrideMode || mode;
    if (overrideMode) setMode(overrideMode);

    setAgentRunning(true);
    setLiveStatus("Initializing agent loop...");
    setLiveThinking("");
    setLiveText("");
    setLiveTools([]);

    // Optimistically show user message
    setActiveSession((prev) =>
      prev
        ? {
            ...prev,
            messages: [
              ...prev.messages,
              {
                id: `tmp_${Date.now()}`,
                sessionId: prev.id,
                role: "user",
                content: prompt,
                mode: targetMode,
                createdAt: new Date().toISOString(),
              },
            ],
          }
        : prev
    );

    try {
      await desktopInvoke("start_agent_run", {
        sessionId: activeSession.id,
        prompt,
        mode: targetMode,
        provider: settings.activeProvider,
        model: settings.activeModel,
      });
    } catch (err: any) {
      setAgentRunning(false);
      showToast(`Agent failed to start: ${err.message}`);
    }
  };

  const handleEmergencyStop = async () => {
    await desktopInvoke("emergency_stop", {});
    setAgentRunning(false);
    setPendingPermission(null);
    setLiveStatus("");
    showToast("Emergency Stop triggered — all agent tasks & child processes halted.");
  };

  if (bootError) {
    return (
      <div className="fatal-native-guard">
        <div className="fatal-card">
          <h2>Native Windows Desktop Host Required</h2>
          <p>{bootError}</p>
          <code>dist\AI-Coding-Agent.exe</code>
        </div>
      </div>
    );
  }

  const activeFilePath =
    editorTabs.find((t) => t.id === activeTabId && t.kind === "code")?.path ||
    null;

  return (
    <div className="desktop-root">
      <TitlebarStatus
        currentProject={currentProject}
        drives={drives}
        recentProjects={recentProjects}
        settings={settings}
        mode={mode}
        agentRunning={agentRunning}
        onPickFolder={handlePickFolder}
        onSwitchProject={handleSwitchProject}
        onChangeMode={setMode}
        onChangeProviderModel={handleChangeProviderModel}
        onEmergencyStop={handleEmergencyStop}
        onOpenSettings={(tab = "providers") => {
          setSettingsTab(tab);
          setSettingsOpen(true);
        }}
      />

      <div className="studio-workspace-grid">
        {/* Far-left Activity Rail */}
        <nav className="studio-rail">
          <button
            className={`rail-icon-btn ${leftTab === "explorer" ? "active" : ""}`}
            onClick={() => setLeftTab("explorer")}
            title="Windows Project Explorer & Search"
          >
            <FolderTree size={18} />
          </button>
          <button
            className={`rail-icon-btn ${leftTab === "git" ? "active" : ""}`}
            onClick={() => setLeftTab("git")}
            title="Git Source Control"
          >
            <GitBranch size={18} />
            {git.files.length > 0 && (
              <span className="rail-counter">{git.files.length}</span>
            )}
          </button>
        </nav>

        {/* Left Explorer / Git Pane */}
        {leftTab === "explorer" ? (
          <FileExplorer
            projectRoot={currentProject}
            files={files}
            activeFilePath={activeFilePath}
            watcherActive={Boolean(currentProject)}
            onOpenFile={(entry) => handleOpenFile(entry)}
            onRefresh={refreshWorkspace}
            onCreateItem={handleCreateFsItem}
            onDeleteItem={handleDeleteRequest}
            onRevealExplorer={(path) =>
              desktopInvoke("reveal_in_explorer", { path })
            }
            onRunSearch={(query) =>
              desktopInvoke<SearchMatch[]>("search_in_project", {
                query,
                caseSensitive: false,
                glob: "",
              })
            }
          />
        ) : (
          <GitPanel
            git={git}
            onRefresh={refreshWorkspace}
            onStage={async (path, stageAll = false) => {
              const next = await desktopInvoke<GitStatusPayload>("git_stage", {
                path,
                stageAll,
              });
              setGit(next);
            }}
            onUnstage={async (path) => {
              const next = await desktopInvoke<GitStatusPayload>(
                "git_unstage",
                { path }
              );
              setGit(next);
            }}
            onCommit={async (message) => {
              const res = await desktopInvoke<{
                ok: boolean;
                output: string;
                status: GitStatusPayload;
              }>("git_commit", { message });
              setGit(res.status);
              showToast(res.ok ? "Committed changes!" : res.output);
            }}
            onOpenDiff={handleOpenDiff}
            onCheckoutBranch={async (branch, createNew) => {
              const res = await desktopInvoke<{
                ok: boolean;
                output: string;
                status: GitStatusPayload;
              }>("git_checkout", { branch, createNew });
              setGit(res.status);
              refreshWorkspace();
              showToast(res.ok ? `Switched to ${branch}` : res.output);
            }}
          />
        )}

        {/* Center Stack: Multi-Tab Monaco Editor + Real Windows ConPTY Terminal */}
        <div className="center-editor-terminal-stack">
          <MonacoWorkbench
            tabs={editorTabs}
            activeTabId={activeTabId}
            fontSize={settings.editorFontSize}
            projectRoot={currentProject}
            onSelectTab={setActiveTabId}
            onCloseTab={(id) => {
              setEditorTabs((prev) => {
                const next = prev.filter((t) => t.id !== id);
                if (activeTabId === id) {
                  setActiveTabId(next[next.length - 1]?.id || null);
                }
                return next;
              });
            }}
            onChangeContent={(id, content) =>
              setEditorTabs((prev) =>
                prev.map((t) => (t.id === id ? { ...t, content } : t))
              )
            }
            onSaveTab={handleSaveTab}
            onPickFolder={handlePickFolder}
          />

          <ConPtyTerminal
            projectRoot={currentProject}
            defaultShell={settings.defaultShell}
          />
        </div>

        {/* Right Autonomous AI Coding Agent & Session History */}
        <AgentPanel
          sessions={sessions}
          activeSession={activeSession}
          mode={mode}
          running={agentRunning}
          liveStatus={liveStatus}
          liveThinking={liveThinking}
          liveText={liveText}
          liveTools={liveTools}
          onSelectSession={async (sessionId) => {
            const detail = await desktopInvoke<SessionDetail>("load_session", {
              sessionId,
            });
            setActiveSession(detail);
          }}
          onNewSession={async () => {
            const created = await desktopInvoke<SessionDetail>(
              "create_session",
              { title: "New Agent Session", mode }
            );
            setActiveSession(created);
            refreshSessions();
          }}
          onDeleteSession={async (sessionId) => {
            await desktopInvoke("delete_session", { sessionId });
            const list = await desktopInvoke<SessionSummary[]>(
              "list_sessions",
              {}
            );
            setSessions(list);
            if (list.length > 0) {
              const detail = await desktopInvoke<SessionDetail>(
                "load_session",
                { sessionId: list[0].id }
              );
              setActiveSession(detail);
            }
          }}
          onSendPrompt={handleSendPrompt}
          onEmergencyStop={handleEmergencyStop}
          onOpenFileDiff={handleOpenDiff}
        />
      </div>

      {/* Footer Statusbar */}
      <footer className="studio-statusbar">
        <div>
          <span>📁 {currentProject}</span>
          {git.isRepo && <span>🌿 {git.branch}</span>}
          <span>🗄️ SQLite Persistent</span>
        </div>
        <div>
          <span>
            🛡️ Workspace Sandbox:{" "}
            {settings.restrictToWorkspace ? "Restricted" : "Unrestricted"}
          </span>
          <span>
            ⚡ {settings.activeProvider} / {settings.activeModel}
          </span>
        </div>
      </footer>

      {/* Agent Permission Gate Modal */}
      <ApprovalModal
        request={pendingPermission}
        onDecision={async (requestId, approved, rememberSession) => {
          setPendingPermission(null);
          await desktopInvoke("respond_permission", {
            requestId,
            approved,
            rememberSession,
          });
        }}
      />

      {/* User File Deletion Confirmation Modal */}
      {pendingUserDelete && (
        <ApprovalModal
          request={{
            requestId: "user_delete",
            kind: "delete_file",
            title: "Confirm Permanent Disk Deletion",
            detail:
              "This action permanently deletes the selected file or directory from your Windows filesystem.",
            targetPath: pendingUserDelete.path,
          }}
          onDecision={(_, approved) => {
            if (approved) {
              executeConfirmedDelete(pendingUserDelete);
            } else {
              setPendingUserDelete(null);
            }
          }}
        />
      )}

      {/* SQLite Settings & Audit History Modal */}
      <SettingsModal
        open={settingsOpen}
        initialTab={settingsTab}
        settings={settings}
        sqlitePath={sqlitePath}
        appDataDir={appDataDir}
        onClose={() => setSettingsOpen(false)}
        onSaveSettings={handleSaveSettings}
      />

      {toastMsg && <div className="desktop-toast">{toastMsg}</div>}
    </div>
  );
};

export default App;
