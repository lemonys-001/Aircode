import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, UnlistenFn } from "@tauri-apps/api/event";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    chrome?: {
      webview?: {
        postMessage: (msg: unknown) => void;
        addEventListener: (
          type: "message",
          listener: (ev: { data: unknown }) => void
        ) => void;
        removeEventListener: (
          type: "message",
          listener: (ev: { data: unknown }) => void
        ) => void;
      };
    };
  }
}

type PendingCall = {
  resolve: (val: any) => void;
  reject: (err: Error) => void;
};

const pendingCalls = new Map<string, PendingCall>();
const eventListeners = new Map<string, Set<(payload: any) => void>>();
let webviewBridgeInitialized = false;

function ensureWebView2Bridge() {
  if (webviewBridgeInitialized || !window.chrome?.webview) return;
  webviewBridgeInitialized = true;

  window.chrome.webview.addEventListener("message", (ev) => {
    const packet =
      typeof ev.data === "string" ? JSON.parse(ev.data) : (ev.data as any);
    if (!packet) return;

    if (packet.type === "ipc_response" && packet.id) {
      const pending = pendingCalls.get(packet.id);
      if (pending) {
        pendingCalls.delete(packet.id);
        if (packet.ok) {
          pending.resolve(packet.result);
        } else {
          pending.reject(new Error(packet.error || "Native IPC command failed"));
        }
      }
    } else if (packet.type === "ipc_event" && packet.event) {
      const handlers = eventListeners.get(packet.event);
      if (handlers) {
        handlers.forEach((fn) => {
          try {
            fn(packet.payload);
          } catch (err) {
            console.error("Event listener error:", err);
          }
        });
      }
    }
  });
}

export async function desktopInvoke<T = any>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (window.__TAURI_INTERNALS__) {
    return tauriInvoke<T>(cmd, args);
  }
  if (window.chrome?.webview) {
    ensureWebView2Bridge();
    const id = `req_${Date.now()}_${Math.random().toString(16).slice(2, 10)}`;
    return new Promise<T>((resolve, reject) => {
      pendingCalls.set(id, { resolve, reject });
      window.chrome!.webview!.postMessage({
        type: "ipc_invoke",
        id,
        cmd,
        args,
      });
    });
  }
  throw new Error(
    "Desktop Native Runtime Required: AI Coding Agent must be launched via AI-Coding-Agent.exe (WebView2 / Tauri 2 native host)."
  );
}

export async function desktopListen<T = any>(
  event: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  if (window.__TAURI_INTERNALS__) {
    return tauriListen<T>(event, (ev) => handler(ev.payload));
  }
  ensureWebView2Bridge();
  if (!eventListeners.has(event)) {
    eventListeners.set(event, new Set());
  }
  const set = eventListeners.get(event)!;
  set.add(handler as (payload: any) => void);
  return () => {
    set.delete(handler as (payload: any) => void);
  };
}

export function inferMonacoLanguage(filePath: string): string {
  const ext = filePath.split(".").pop()?.toLowerCase() || "";
  const map: Record<string, string> = {
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    mjs: "javascript",
    cjs: "javascript",
    rs: "rust",
    py: "python",
    cpp: "cpp",
    cc: "cpp",
    cxx: "cpp",
    hpp: "cpp",
    h: "cpp",
    c: "c",
    cs: "csharp",
    go: "go",
    java: "java",
    json: "json",
    html: "html",
    htm: "html",
    css: "css",
    scss: "scss",
    md: "markdown",
    ps1: "powershell",
    bat: "bat",
    cmd: "bat",
    sh: "shell",
    yml: "yaml",
    yaml: "yaml",
    toml: "ini",
    sql: "sql",
    xml: "xml",
  };
  return map[ext] || "plaintext";
}
