import React from "react";
import {
  FolderOpen,
  HardDrive,
  OctagonX,
  Settings,
  ShieldCheck,
  History,
  Cpu,
} from "lucide-react";
import {
  AgentMode,
  AppSettings,
  DriveInfo,
  ProviderId,
} from "../types/desktop";

interface Props {
  currentProject: string;
  drives: DriveInfo[];
  recentProjects: string[];
  settings: AppSettings;
  mode: AgentMode;
  agentRunning: boolean;
  onPickFolder: () => void;
  onSwitchProject: (path: string) => void;
  onChangeMode: (mode: AgentMode) => void;
  onChangeProviderModel: (provider: ProviderId, model: string) => void;
  onEmergencyStop: () => void;
  onOpenSettings: (tab?: "providers" | "permissions" | "history") => void;
}

export const PROVIDER_CATALOG: Record<
  ProviderId,
  { label: string; models: string[] }
> = {
  openai: {
    label: "OpenAI",
    models: ["gpt-4.1", "gpt-4.1-mini", "o4-mini", "gpt-4o"],
  },
  anthropic: {
    label: "Anthropic",
    models: [
      "claude-sonnet-4-5",
      "claude-3-7-sonnet-latest",
      "claude-3-5-haiku-latest",
      "claude-opus-4-5",
    ],
  },
  gemini: {
    label: "Gemini",
    models: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"],
  },
  openrouter: {
    label: "OpenRouter",
    models: [
      "qwen/qwen3-coder:free",
      "deepseek/deepseek-chat-v3-0324:free",
      "anthropic/claude-sonnet-4",
      "openai/gpt-4.1",
    ],
  },
  ollama: {
    label: "Ollama (Local)",
    models: [
      "qwen2.5-coder:7b",
      "deepseek-coder-v2:latest",
      "llama3.3:latest",
      "codellama:latest",
    ],
  },
  custom: {
    label: "Custom OpenAI API",
    models: ["custom-model"],
  },
};

export const TitlebarStatus: React.FC<Props> = ({
  currentProject,
  drives,
  settings,
  mode,
  agentRunning,
  onPickFolder,
  onSwitchProject,
  onChangeMode,
  onChangeProviderModel,
  onEmergencyStop,
  onOpenSettings,
}) => {
  const providerModels =
    PROVIDER_CATALOG[settings.activeProvider]?.models || [settings.activeModel];

  return (
    <header className="studio-topbar">
      <div className="topbar-group">
        <div className="app-brand">
          <div className="app-brand-mark">⚡</div>
          <div>
            <div className="app-brand-title">AI Coding Agent</div>
            <div className="app-brand-sub">Native Win64 · Tauri 2 · ConPTY · SQLite</div>
          </div>
        </div>

        <button
          className="glass-btn primary-outline"
          onClick={onPickFolder}
          title={currentProject}
        >
          <FolderOpen size={14} />
          <span className="truncate-path">{currentProject || "Select Project..."}</span>
        </button>

        {drives.length > 0 && (
          <div className="drive-selector" title="Switch Windows Drive Root">
            <HardDrive size={13} />
            <select
              value=""
              onChange={(e) => {
                if (e.target.value) onSwitchProject(e.target.value);
              }}
            >
              <option value="">Drives ({drives.map((d) => d.letter).join(", ")})</option>
              {drives.map((d) => (
                <option key={d.mountPath} value={d.mountPath}>
                  {d.mountPath} [{d.driveType}] — {d.freeGb.toFixed(1)} GB free
                </option>
              ))}
            </select>
          </div>
        )}
      </div>

      <div className="topbar-group">
        <div className="mode-pill-group" role="group" aria-label="Agent Permission Mode">
          {(["ask", "plan", "autonomous"] as AgentMode[]).map((m) => (
            <button
              key={m}
              className={`mode-pill ${mode === m ? "active " + m : ""}`}
              onClick={() => onChangeMode(m)}
            >
              {m === "ask" && "💬 Ask"}
              {m === "plan" && "📐 Plan"}
              {m === "autonomous" && "⚡ Autonomous"}
            </button>
          ))}
        </div>
      </div>

      <div className="topbar-group">
        <div className="model-selector-bar">
          <Cpu size={13} className="accent-icon" />
          <select
            value={settings.activeProvider}
            onChange={(e) => {
              const p = e.target.value as ProviderId;
              const defaultM = PROVIDER_CATALOG[p].models[0];
              onChangeProviderModel(p, defaultM);
            }}
          >
            {Object.entries(PROVIDER_CATALOG).map(([id, info]) => (
              <option key={id} value={id}>
                {info.label}
              </option>
            ))}
          </select>
          <select
            value={settings.activeModel}
            onChange={(e) =>
              onChangeProviderModel(settings.activeProvider, e.target.value)
            }
          >
            {!providerModels.includes(settings.activeModel) && (
              <option value={settings.activeModel}>{settings.activeModel}</option>
            )}
            {providerModels.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>

        <button
          className={`emergency-stop-btn ${agentRunning ? "pulsing" : ""}`}
          onClick={onEmergencyStop}
          title="Emergency Stop — Immediately halt AI agent & active subprocesses"
        >
          <OctagonX size={14} />
          <span>STOP</span>
        </button>

        <button
          className="glass-btn icon-only"
          onClick={() => onOpenSettings("history")}
          title="Tool Execution Audit History"
        >
          <History size={15} />
        </button>

        <button
          className="glass-btn icon-only"
          onClick={() => onOpenSettings("permissions")}
          title="Agent Permissions & Sandbox Rules"
        >
          <ShieldCheck size={15} />
        </button>

        <button
          className="glass-btn icon-only"
          onClick={() => onOpenSettings("providers")}
          title="Settings & API Keys (SQLite Persisted)"
        >
          <Settings size={15} />
        </button>
      </div>
    </header>
  );
};
