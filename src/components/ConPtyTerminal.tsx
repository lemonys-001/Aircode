import React, { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  TerminalSquare,
  Plus,
  X,
  Square,
  FolderSync,
  SlidersHorizontal,
} from "lucide-react";
import { desktopInvoke, desktopListen } from "../lib/tauri";
import { PtyTabSession } from "../types/desktop";

interface Props {
  projectRoot: string;
  defaultShell: "powershell" | "cmd";
}

interface InternalTermRef {
  term: Terminal;
  fit: FitAddon;
  container: HTMLDivElement | null;
}

export const ConPtyTerminal: React.FC<Props> = ({
  projectRoot,
  defaultShell,
}) => {
  const [tabs, setTabs] = useState<PtyTabSession[]>([]);
  const [activePtyId, setActivePtyId] = useState<string | null>(null);
  const [customEnvText, setCustomEnvText] = useState("");
  const [showEnvBox, setShowEnvBox] = useState(false);

  const xtermMapRef = useRef<Map<string, InternalTermRef>>(new Map());
  const stageRef = useRef<HTMLDivElement | null>(null);
  const bootedRef = useRef(false);

  useEffect(() => {
    let unlistenData: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;

    desktopListen<{ ptyId: string; data: string }>("pty://data", (ev) => {
      const entry = xtermMapRef.current.get(ev.ptyId);
      if (entry) {
        entry.term.write(ev.data);
      }
    }).then((fn) => {
      unlistenData = fn;
    });

    desktopListen<{ ptyId: string; exitCode: number }>("pty://exit", (ev) => {
      const entry = xtermMapRef.current.get(ev.ptyId);
      if (entry) {
        entry.term.writeln(
          `\r\n\x1b[33m[Process exited with code ${ev.exitCode}]\x1b[0m`
        );
      }
      setTabs((prev) =>
        prev.map((t) => (t.ptyId === ev.ptyId ? { ...t, alive: false } : t))
      );
    }).then((fn) => {
      unlistenExit = fn;
    });

    return () => {
      unlistenData?.();
      unlistenExit?.();
      xtermMapRef.current.forEach((v) => v.term.dispose());
      xtermMapRef.current.clear();
    };
  }, []);

  useEffect(() => {
    if (!bootedRef.current && projectRoot) {
      bootedRef.current = true;
      spawnTab(defaultShell);
    }
  }, [projectRoot]);

  useEffect(() => {
    if (!stageRef.current) return;
    const ro = new ResizeObserver(() => {
      if (!activePtyId) return;
      const entry = xtermMapRef.current.get(activePtyId);
      if (entry && entry.container) {
        try {
          entry.fit.fit();
          desktopInvoke("pty_resize", {
            ptyId: activePtyId,
            cols: entry.term.cols,
            rows: entry.term.rows,
          }).catch(() => {});
        } catch {}
      }
    });
    ro.observe(stageRef.current);
    return () => ro.disconnect();
  }, [activePtyId]);

  const parseCustomEnv = (): Record<string, string> => {
    const out: Record<string, string> = {};
    for (const ln of customEnvText.split("\n")) {
      const eq = ln.indexOf("=");
      if (eq > 0) {
        const k = ln.slice(0, eq).trim();
        const v = ln.slice(eq + 1).trim();
        if (k) out[k] = v;
      }
    }
    return out;
  };

  const spawnTab = async (shell: "powershell" | "cmd") => {
    try {
      const res = await desktopInvoke<{
        ptyId: string;
        shell: string;
        cwd: string;
      }>("pty_spawn", {
        shell,
        cwd: projectRoot,
        cols: 110,
        rows: 22,
        env: parseCustomEnv(),
      });

      const term = new Terminal({
        cursorBlink: true,
        fontSize: 12.5,
        fontFamily: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace",
        theme: {
          background: "#080C14",
          foreground: "#E2E8F0",
          cursor: "#38BDF8",
          selectionBackground: "#1E3A5F",
          black: "#0F172A",
          green: "#34D399",
          blue: "#38BDF8",
          cyan: "#22D3EE",
          yellow: "#FBBF24",
          red: "#F87171",
          magenta: "#C084FC",
        },
      });
      const fit = new FitAddon();
      term.loadAddon(fit);

      term.onData((data) => {
        desktopInvoke("pty_write", { ptyId: res.ptyId, data }).catch(() => {});
      });

      xtermMapRef.current.set(res.ptyId, { term, fit, container: null });

      const newSession: PtyTabSession = {
        ptyId: res.ptyId,
        title: shell === "powershell" ? "PowerShell" : "CMD",
        shell,
        cwd: res.cwd || projectRoot,
        alive: true,
      };
      setTabs((prev) => [...prev, newSession]);
      setActivePtyId(res.ptyId);
    } catch (err: any) {
      console.error("Failed to spawn ConPTY:", err);
    }
  };

  const closeTab = async (ptyId: string) => {
    await desktopInvoke("pty_kill", { ptyId }).catch(() => {});
    const entry = xtermMapRef.current.get(ptyId);
    entry?.term.dispose();
    xtermMapRef.current.delete(ptyId);
    setTabs((prev) => {
      const next = prev.filter((t) => t.ptyId !== ptyId);
      if (activePtyId === ptyId) {
        setActivePtyId(next[next.length - 1]?.ptyId || null);
      }
      return next;
    });
  };

  const sendCtrlC = () => {
    if (!activePtyId) return;
    desktopInvoke("pty_write", { ptyId: activePtyId, data: "\x03" }).catch(
      () => {}
    );
  };

  const cdToProjectRoot = () => {
    if (!activePtyId || !projectRoot) return;
    const active = tabs.find((t) => t.ptyId === activePtyId);
    const cmd =
      active?.shell === "cmd"
        ? `cd /d "${projectRoot}"\r`
        : `Set-Location -LiteralPath "${projectRoot}"\r`;
    desktopInvoke("pty_write", { ptyId: activePtyId, data: cmd }).catch(
      () => {}
    );
  };

  return (
    <div className="conpty-panel">
      <div className="conpty-header">
        <div className="conpty-tabs">
          <span className="conpty-badge">
            <TerminalSquare size={13} /> ConPTY
          </span>
          {tabs.map((t) => (
            <div
              key={t.ptyId}
              className={`conpty-tab ${t.ptyId === activePtyId ? "active" : ""}`}
              onClick={() => setActivePtyId(t.ptyId)}
            >
              <span className={`pty-dot ${t.alive ? "alive" : "dead"}`} />
              <span>{t.title}</span>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.ptyId);
                }}
              >
                <X size={11} />
              </button>
            </div>
          ))}
        </div>

        <div className="conpty-controls">
          <button
            className="glass-btn tiny"
            onClick={() => spawnTab("powershell")}
            title="New Windows PowerShell ConPTY Tab"
          >
            <Plus size={12} /> PowerShell
          </button>
          <button
            className="glass-btn tiny"
            onClick={() => spawnTab("cmd")}
            title="New Windows CMD ConPTY Tab"
          >
            <Plus size={12} /> CMD
          </button>
          <button
            className="glass-btn tiny"
            onClick={cdToProjectRoot}
            title="Switch Terminal Working Directory to Active Project"
          >
            <FolderSync size={12} /> Sync CWD
          </button>
          <button
            className="glass-btn tiny danger-text"
            onClick={sendCtrlC}
            title="Cancel Running Process (Send SIGINT / Ctrl+C)"
          >
            <Square size={11} /> Interrupt
          </button>
          <button
            className="glass-btn tiny"
            onClick={() => setShowEnvBox((v) => !v)}
            title="Custom Environment Variables for Spawned Terminals"
          >
            <SlidersHorizontal size={12} /> Env
          </button>
        </div>
      </div>

      {showEnvBox && (
        <div className="conpty-env-drawer">
          <span>Custom Terminal Environment Variables (`KEY=VALUE` per line):</span>
          <textarea
            rows={2}
            value={customEnvText}
            onChange={(e) => setCustomEnvText(e.target.value)}
            placeholder="RUST_LOG=info&#10;NODE_ENV=development"
          />
        </div>
      )}

      <div className="conpty-stage" ref={stageRef}>
        {tabs.map((t) => (
          <div
            key={t.ptyId}
            className={`xterm-slot ${t.ptyId === activePtyId ? "visible" : "hidden-slot"}`}
            ref={(el) => {
              if (!el) return;
              const entry = xtermMapRef.current.get(t.ptyId);
              if (entry && !entry.container) {
                entry.container = el;
                entry.term.open(el);
                setTimeout(() => {
                  try {
                    entry.fit.fit();
                    desktopInvoke("pty_resize", {
                      ptyId: t.ptyId,
                      cols: entry.term.cols,
                      rows: entry.term.rows,
                    }).catch(() => {});
                  } catch {}
                }, 40);
              }
            }}
          />
        ))}
      </div>
    </div>
  );
};
