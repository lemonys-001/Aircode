import React, { useEffect, useRef, useState } from "react";
import {
  Send,
  Plus,
  Trash2,
  OctagonX,
  Wrench,
  Sparkles,
  Play,
  FileDiff,
  ChevronDown,
  ChevronRight,
} from "lucide-react";
import {
  AgentMode,
  ChatMessage,
  SessionDetail,
  SessionSummary,
  ToolExecutionRecord,
} from "../types/desktop";

interface Props {
  sessions: SessionSummary[];
  activeSession: SessionDetail | null;
  mode: AgentMode;
  running: boolean;
  liveStatus: string;
  liveThinking: string;
  liveText: string;
  liveTools: ToolExecutionRecord[];
  onSelectSession: (id: string) => void;
  onNewSession: () => void;
  onDeleteSession: (id: string) => void;
  onSendPrompt: (prompt: string, overrideMode?: AgentMode) => void;
  onEmergencyStop: () => void;
  onOpenFileDiff: (relPath: string) => void;
}

export const AgentPanel: React.FC<Props> = ({
  sessions,
  activeSession,
  mode,
  running,
  liveStatus,
  liveThinking,
  liveText,
  liveTools,
  onSelectSession,
  onNewSession,
  onDeleteSession,
  onSendPrompt,
  onEmergencyStop,
  onOpenFileDiff,
}) => {
  const [input, setInput] = useState("");
  const [openToolIds, setOpenToolIds] = useState<Record<string, boolean>>({});
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [activeSession?.messages, liveText, liveTools, liveThinking]);

  const toggleTool = (id: string) => {
    setOpenToolIds((prev) => ({ ...prev, [id]: !prev[id] }));
  };

  const renderToolCard = (t: ToolExecutionRecord) => {
    const expanded = !!openToolIds[t.id];
    return (
      <div key={t.id} className={`agent-tool-card ${t.status}`}>
        <div className="agent-tool-head" onClick={() => toggleTool(t.id)}>
          <div className="agent-tool-left">
            {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
            <Wrench size={12} className="accent-icon" />
            <span className="tool-fn-name">{t.toolName}</span>
            <span className="tool-summary-text">{t.resultSummary}</span>
          </div>
          <div className="agent-tool-right">
            {t.filePath && (
              <button
                className="tiny-icon-btn"
                title="Inspect Diff in Monaco DiffEditor"
                onClick={(e) => {
                  e.stopPropagation();
                  onOpenFileDiff(t.filePath!);
                }}
              >
                <FileDiff size={12} />
              </button>
            )}
            <span className={`tool-status-tag ${t.status}`}>
              {t.status === "running" ? "RUNNING..." : `${t.durationMs}ms`}
            </span>
          </div>
        </div>
        {expanded && (
          <div className="agent-tool-body">
            <div className="tool-json-label">Arguments:</div>
            <pre>{t.argsJson}</pre>
            {t.diffText && (
              <>
                <div className="tool-json-label">Diff:</div>
                <pre className="tool-diff-pre">{t.diffText}</pre>
              </>
            )}
          </div>
        )}
      </div>
    );
  };

  const renderFormattedContent = (content: string) => {
    const parts = content.split(/(```[\s\S]*?```)/g);
    return parts.map((part, idx) => {
      if (part.startsWith("```") && part.endsWith("```")) {
        const lines = part.slice(3, -3).split("\n");
        const lang = lines[0]?.trim() || "code";
        const body = lines.slice(1).join("\n");
        return (
          <div key={idx} className="chat-codeblock">
            <div className="chat-codeblock-head">{lang}</div>
            <pre>{body}</pre>
          </div>
        );
      }
      return (
        <p key={idx} className="chat-prose">
          {part}
        </p>
      );
    });
  };

  const handleSend = () => {
    const trimmed = input.trim();
    if (!trimmed || running) return;
    setInput("");
    onSendPrompt(trimmed);
  };

  return (
    <aside className="agent-sidebar-panel">
      <div className="agent-session-bar">
        <select
          className="session-select"
          value={activeSession?.id || ""}
          onChange={(e) => onSelectSession(e.target.value)}
        >
          {sessions.map((s) => (
            <option key={s.id} value={s.id}>
              {s.title} ({s.messageCount})
            </option>
          ))}
        </select>
        <button
          className="glass-btn tiny accent"
          onClick={onNewSession}
          title="New Persistent SQLite Session"
        >
          <Plus size={13} /> New
        </button>
        {activeSession && sessions.length > 1 && (
          <button
            className="tiny-icon-btn danger"
            onClick={() => onDeleteSession(activeSession.id)}
            title="Delete Session from SQLite"
          >
            <Trash2 size={13} />
          </button>
        )}
      </div>

      <div className="agent-mode-explainer">
        {mode === "ask" &&
          "💬 Ask Mode — Reads files & answers questions; requests permission before any write or shell command."}
        {mode === "plan" &&
          "📐 Plan Mode — Read-only architectural exploration; produces executable step-by-step plans without mutating files."}
        {mode === "autonomous" &&
          "⚡ Autonomous Mode — Continuous multi-step coding loop with verification & error recovery."}
      </div>

      <div className="agent-messages-scroll" ref={scrollRef}>
        {(!activeSession || activeSession.messages.length === 0) && !running && (
          <div className="agent-welcome-card">
            <div className="welcome-badge">
              <Sparkles size={13} /> Autonomous Coding Agent Ready
            </div>
            <h4>Inspect, Edit, Test &amp; Debug Local Windows Projects</h4>
            <p>
              Equipped with 9 real workspace tools: <code>read_file</code>,{" "}
              <code>write_file</code>, <code>edit_file</code>,{" "}
              <code>search_project</code>, <code>execute_command</code>,{" "}
              <code>install_dependencies</code>, <code>run_tests</code>,{" "}
              <code>diagnose_errors</code>, and <code>git_operation</code>.
            </p>
            <div className="quick-chip-grid">
              <button
                onClick={() =>
                  onSendPrompt(
                    "Explore this project directory, read key files, and summarize the architecture."
                  )
                }
              >
                🔍 Map Project Architecture
              </button>
              <button
                onClick={() =>
                  onSendPrompt(
                    "Run diagnose_errors across the project and fix any issues found."
                  )
                }
              >
                🩺 Diagnose &amp; Fix Errors
              </button>
              <button
                onClick={() =>
                  onSendPrompt(
                    "Run project tests and report pass/fail results."
                  )
                }
              >
                🧪 Run Project Tests
              </button>
              <button
                onClick={() =>
                  onSendPrompt(
                    "Inspect git status and diff, then review uncommitted changes for bugs."
                  )
                }
              >
                🌿 Review Git Changes
              </button>
            </div>
          </div>
        )}

        {activeSession?.messages.map((msg: ChatMessage) => (
          <div key={msg.id} className={`chat-bubble ${msg.role}`}>
            <div className="chat-bubble-meta">
              <strong>{msg.role === "user" ? "You" : "AI Agent"}</strong>
              <span>
                {msg.mode.toUpperCase()}
                {msg.model ? ` · ${msg.model}` : ""}
              </span>
            </div>

            {msg.thinking && (
              <details className="thinking-details">
                <summary>Reasoning Trace</summary>
                <pre>{msg.thinking}</pre>
              </details>
            )}

            {msg.tools && msg.tools.length > 0 && (
              <div className="tools-stack">
                {msg.tools.map((t) => renderToolCard(t))}
              </div>
            )}

            <div className="chat-bubble-content">
              {renderFormattedContent(msg.content)}
            </div>

            {msg.role === "assistant" && msg.mode === "plan" && (
              <div className="plan-execute-footer">
                <button
                  className="glass-btn small accent"
                  onClick={() =>
                    onSendPrompt(
                      `Execute this approved plan step-by-step and verify changes:\n\n${msg.content}`,
                      "autonomous"
                    )
                  }
                >
                  <Play size={12} /> Execute Plan in Autonomous Mode
                </button>
              </div>
            )}
          </div>
        ))}

        {running && (
          <div className="chat-bubble assistant streaming">
            <div className="chat-bubble-meta">
              <strong>AI Agent</strong>
              <span className="live-pulse">{liveStatus || "Working..."}</span>
            </div>

            {liveThinking && (
              <details className="thinking-details" open>
                <summary>Streaming Reasoning...</summary>
                <pre>{liveThinking}</pre>
              </details>
            )}

            {liveTools.length > 0 && (
              <div className="tools-stack">
                {liveTools.map((t) => renderToolCard(t))}
              </div>
            )}

            <div className="chat-bubble-content">
              {liveText ? (
                renderFormattedContent(liveText)
              ) : (
                <span className="muted-italic">Executing agent loop...</span>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="agent-composer">
        <textarea
          rows={3}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
          placeholder="Describe features to implement, files to refactor, tests to run, or bugs to fix..."
        />
        <div className="composer-bottom-row">
          <span className="composer-hint">
            Enter to send · Shift+Enter for newline
          </span>
          {running ? (
            <button className="emergency-stop-btn" onClick={onEmergencyStop}>
              <OctagonX size={14} /> Stop Agent
            </button>
          ) : (
            <button
              className="glass-btn accent"
              disabled={!input.trim()}
              onClick={handleSend}
            >
              <Send size={13} /> Run Agent
            </button>
          )}
        </div>
      </div>
    </aside>
  );
};
