import React from "react";
import Editor, { DiffEditor } from "@monaco-editor/react";
import { Save, X, FileDiff, FileCode, FolderOpen } from "lucide-react";
import { OpenEditorTab } from "../types/desktop";

interface Props {
  tabs: OpenEditorTab[];
  activeTabId: string | null;
  fontSize: number;
  projectRoot: string;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onChangeContent: (id: string, value: string) => void;
  onSaveTab: (id: string) => void;
  onPickFolder: () => void;
}

export const MonacoWorkbench: React.FC<Props> = ({
  tabs,
  activeTabId,
  fontSize,
  projectRoot,
  onSelectTab,
  onCloseTab,
  onChangeContent,
  onSaveTab,
  onPickFolder,
}) => {
  const activeTab = tabs.find((t) => t.id === activeTabId) || null;

  return (
    <section className="monaco-workbench">
      <div className="workbench-tabstrip">
        <div className="workbench-tabs-scroll">
          {tabs.map((tab) => {
            const isDirty =
              tab.kind === "code" && tab.content !== tab.savedContent;
            const isActive = tab.id === activeTabId;
            return (
              <div
                key={tab.id}
                className={`wb-tab ${isActive ? "active" : ""}`}
                onClick={() => onSelectTab(tab.id)}
                title={tab.path}
              >
                {tab.kind === "diff" ? (
                  <FileDiff size={13} className="diff-tab-icon" />
                ) : (
                  <FileCode size={13} className="code-tab-icon" />
                )}
                <span>
                  {tab.title}
                  {isDirty ? " ●" : ""}
                </span>
                <button
                  className="wb-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(tab.id);
                  }}
                >
                  <X size={12} />
                </button>
              </div>
            );
          })}
        </div>

        {activeTab && activeTab.kind === "code" && (
          <div className="workbench-actions">
            <span className="wb-path-crumb">{activeTab.relPath}</span>
            <button
              className="glass-btn small accent"
              onClick={() => onSaveTab(activeTab.id)}
              disabled={activeTab.readonly}
            >
              <Save size={13} />
              <span>Save</span>
            </button>
          </div>
        )}
      </div>

      <div className="workbench-editor-host">
        {!activeTab ? (
          <div className="empty-workbench">
            <div className="empty-wb-card">
              <div className="empty-wb-logo">⚡</div>
              <h2>AI Coding Agent for Windows</h2>
              <p>
                Active Workspace: <code>{projectRoot || "None selected"}</code>
              </p>
              <div className="empty-wb-grid">
                <button className="glass-btn" onClick={onPickFolder}>
                  <FolderOpen size={15} /> Open Local Windows Folder (C:\ / D:\)
                </button>
              </div>
              <div className="shortcut-hints">
                <span>
                  <kbd>Ctrl+S</kbd> Save File to Disk
                </span>
                <span>
                  <kbd>Monaco Editor</kbd> Multi-Tab + Side-by-Side Diffs
                </span>
                <span>
                  <kbd>ConPTY</kbd> Real PowerShell &amp; CMD Below
                </span>
              </div>
            </div>
          </div>
        ) : activeTab.kind === "diff" ? (
          <DiffEditor
            height="100%"
            theme="agent-obsidian"
            language={activeTab.language}
            original={activeTab.originalContent || ""}
            modified={activeTab.content}
            options={{
              readOnly: true,
              renderSideBySide: true,
              fontSize,
              fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
              minimap: { enabled: false },
              scrollBeyondLastLine: false,
              automaticLayout: true,
            }}
          />
        ) : (
          <Editor
            height="100%"
            path={activeTab.path}
            theme="agent-obsidian"
            language={activeTab.language}
            value={activeTab.content}
            onChange={(val) => onChangeContent(activeTab.id, val ?? "")}
            onMount={(editor, monaco) => {
              editor.addCommand(
                monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
                () => {
                  onSaveTab(activeTab.id);
                }
              );
            }}
            options={{
              readOnly: activeTab.readonly,
              fontSize,
              fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
              fontLigatures: true,
              minimap: { enabled: true },
              bracketPairColorization: { enabled: true },
              smoothScrolling: true,
              cursorBlinking: "smooth",
              scrollBeyondLastLine: false,
              automaticLayout: true,
              tabSize: 2,
            }}
          />
        )}
      </div>
    </section>
  );
};
