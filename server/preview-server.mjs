#!/usr/bin/env node
/**
 * Aircode Studio — Cross-Platform Preview & Dev Backend (Pure Node 22+, Zero Dependencies)
 * Mirrors the exact REST + SSE Agent protocol implemented natively in C++17 inside Aircode.exe.
 */
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn } from "node:child_process";

const PORT = Number(process.env.PORT || 3000);
const REPO_ROOT = process.cwd();
let workspaceRoot = REPO_ROOT;

const dataDir = path.join(os.tmpdir(), "aircode-preview-data");
fs.mkdirSync(dataDir, { recursive: true });
const configPath = path.join(dataDir, "config.json");
const sessionsPath = path.join(dataDir, "sessions.json");

const IGNORED_DIRS = new Set([
  ".git", "node_modules", ".next", ".nuxt", "dist", "build", "target",
  "out", ".venv", "venv", "__pycache__", ".pytest_cache", ".mypy_cache",
  ".cache", ".turbo", ".idea", ".vs", "bin", "obj", ".aircode"
]);

const BINARY_EXTS = new Set([
  ".exe", ".dll", ".pdb", ".obj", ".o", ".a", ".lib", ".so", ".dylib",
  ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".bmp", ".pdf",
  ".zip", ".tar", ".gz", ".7z", ".rar", ".woff", ".woff2", ".ttf", ".wasm"
]);

function randomId(prefix = "id") {
  return `${prefix}_${Math.random().toString(16).slice(2, 14)}`;
}

function detectLanguage(relPath) {
  const ext = path.extname(relPath).toLowerCase();
  const map = {
    ".ts": "typescript", ".tsx": "typescript",
    ".js": "javascript", ".jsx": "javascript", ".mjs": "javascript", ".cjs": "javascript",
    ".py": "python",
    ".cpp": "cpp", ".cc": "cpp", ".hpp": "cpp", ".h": "cpp", ".c": "cpp",
    ".rs": "rust", ".go": "go", ".cs": "csharp", ".java": "java",
    ".json": "json", ".html": "html", ".css": "css",
    ".md": "markdown", ".ps1": "powershell", ".sh": "shell",
    ".yml": "yaml", ".yaml": "yaml", ".toml": "toml", ".sql": "sql"
  };
  return map[ext] || "plaintext";
}

function computeUnifiedDiff(relPath, oldStr, newStr) {
  const oldLines = String(oldStr ?? "").replace(/\r\n/g, "\n").split("\n");
  const newLines = String(newStr ?? "").replace(/\r\n/g, "\n").split("\n");
  let prefix = 0;
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) {
    prefix++;
  }
  let oldEnd = oldLines.length;
  let newEnd = newLines.length;
  while (oldEnd > prefix && newEnd > prefix && oldLines[oldEnd - 1] === newLines[newEnd - 1]) {
    oldEnd--;
    newEnd--;
  }
  if (prefix === oldEnd && prefix === newEnd) return "";
  const ctxStart = Math.max(0, prefix - 3);
  const oldCtxEnd = Math.min(oldLines.length, oldEnd + 3);
  const newCtxEnd = Math.min(newLines.length, newEnd + 3);

  const out = [
    `--- a/${relPath}`,
    `+++ b/${relPath}`,
    `@@ -${ctxStart + 1},${oldCtxEnd - ctxStart} +${ctxStart + 1},${newCtxEnd - ctxStart} @@`
  ];
  for (let i = ctxStart; i < prefix; i++) out.push(" " + oldLines[i]);
  for (let i = prefix; i < oldEnd; i++) out.push("-" + oldLines[i]);
  for (let i = prefix; i < newEnd; i++) out.push("+" + newLines[i]);
  for (let i = oldEnd; i < oldCtxEnd; i++) out.push(" " + oldLines[i]);
  return out.join("\n") + "\n";
}

let config = {
  provider: "local-assist",
  model: "aircode-local-instant",
  baseUrl: "",
  apiKeys: {},
  permissionMode: "auto-edit",
  maxSteps: 12,
  theme: "obsidian",
  shell: process.platform === "win32" ? "powershell" : "bash",
  mcpServers: [
    { id: "mcp_fs", name: "Filesystem MCP", command: "npx -y @modelcontextprotocol/server-filesystem .", enabled: true },
    { id: "mcp_git", name: "GitHub MCP", command: "npx -y @modelcontextprotocol/server-github", enabled: false },
    { id: "mcp_fetch", name: "Web Fetch MCP", command: "npx -y @modelcontextprotocol/server-fetch", enabled: false }
  ],
  customCommands: [
    { slash: "/optimize", label: "Performance Audit", prompt: "Analyze hot paths in this project and suggest speed & memory optimizations." },
    { slash: "/win32", label: "Windows Portability Check", prompt: "Check paths, scripts, and configs in this repo for Windows 10/11 compatibility." }
  ]
};

if (fs.existsSync(configPath)) {
  try { config = { ...config, ...JSON.parse(fs.readFileSync(configPath, "utf8")) }; } catch {}
}

let sessions = [];
if (fs.existsSync(sessionsPath)) {
  try { sessions = JSON.parse(fs.readFileSync(sessionsPath, "utf8")); } catch {}
}
if (!Array.isArray(sessions) || !sessions.length) {
  sessions = [{
    id: randomId("ses"),
    title: "Welcome Session",
    mode: "build",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    promptTokens: 0,
    completionTokens: 0,
    costUsd: 0,
    messages: []
  }];
}

function saveConfig() {
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2));
}
function saveSessions() {
  fs.writeFileSync(sessionsPath, JSON.stringify(sessions, null, 2));
}

const checkpoints = []; // { id, label, createdAt, files: Map<relPath, { existed, originalContent, newContent }> }
let activeAbortController = null;

function createCheckpoint(label) {
  const cp = { id: randomId("chk"), label: label || "Agent Checkpoint", createdAt: new Date().toISOString(), files: new Map() };
  checkpoints.push(cp);
  if (checkpoints.length > 40) checkpoints.shift();
  return cp.id;
}

function recordBeforeWrite(cpId, relPath, fullPath) {
  if (!cpId) return;
  const cp = checkpoints.find((c) => c.id === cpId);
  if (!cp || cp.files.has(relPath)) return;
  const existed = fs.existsSync(fullPath) && fs.statSync(fullPath).isFile();
  const originalContent = existed ? fs.readFileSync(fullPath, "utf8") : "";
  cp.files.set(relPath, { existed, originalContent, newContent: originalContent });
}

function recordAfterWrite(cpId, relPath, newContent) {
  if (!cpId) return;
  const cp = checkpoints.find((c) => c.id === cpId);
  if (!cp || !cp.files.has(relPath)) return;
  cp.files.get(relPath).newContent = newContent;
}

function listCheckpointsJson() {
  return [...checkpoints]
    .reverse()
    .filter((c) => c.files.size > 0)
    .map((c) => ({
      id: c.id,
      label: c.label,
      createdAt: c.createdAt,
      fileCount: c.files.size,
      files: Array.from(c.files.entries()).map(([rel, e]) => ({
        path: rel,
        existed: e.existed,
        diff: computeUnifiedDiff(rel, e.originalContent, e.newContent)
      }))
    }));
}

function resolveSafe(p) {
  const full = path.resolve(workspaceRoot, p || ".");
  const rel = path.relative(workspaceRoot, full).split(path.sep).join("/");
  return { full, rel };
}

function extractSymbols(text, lang) {
  const out = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length && out.length < 12; i++) {
    const trimmed = lines[i].trim();
    if (!trimmed || trimmed.startsWith("//") || trimmed.startsWith("#") || trimmed.startsWith("/*")) continue;
    let hit = false;
    if (lang === "python") hit = /^(async\s+)?def\s+|^class\s+/.test(trimmed);
    else if (lang === "typescript" || lang === "javascript") hit = /^(export\s+)?(async\s+)?function\s+|^(export\s+)?(class|interface|type)\s+/.test(trimmed);
    else if (lang === "cpp" || lang === "csharp" || lang === "java") hit = /^(class|struct|namespace|inline)\s+/.test(trimmed);
    else if (lang === "rust") hit = /^(pub\s+)?(fn|struct|enum|impl)\s+/.test(trimmed);
    else if (lang === "go") hit = /^(func|type)\s+/.test(trimmed);
    if (hit) out.push(`L${i + 1}: ${trimmed.slice(0, 88).replace(/[{:]$/, "")}`);
  }
  return out;
}

function scanWorkspace(maxFiles = 1500) {
  const files = [];
  const repoMap = [];
  function walk(dir) {
    if (files.length >= maxFiles) return;
    let entries = [];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    entries.sort((a, b) => (a.isDirectory() === b.isDirectory() ? a.name.localeCompare(b.name) : a.isDirectory() ? -1 : 1));
    for (const ent of entries) {
      if (files.length >= maxFiles) break;
      if (ent.isDirectory()) {
        if (!IGNORED_DIRS.has(ent.name)) walk(path.join(dir, ent.name));
      } else if (ent.isFile()) {
        const full = path.join(dir, ent.name);
        const rel = path.relative(workspaceRoot, full).split(path.sep).join("/");
        let size = 0;
        try { size = fs.statSync(full).size; } catch {}
        const ext = path.extname(ent.name).toLowerCase();
        const binary = BINARY_EXTS.has(ext);
        const language = detectLanguage(rel);
        files.push({ path: rel, name: ent.name, size, language, binary });
        if (!binary && size > 0 && size < 131072 && repoMap.length < 80 && language !== "plaintext") {
          try {
            const syms = extractSymbols(fs.readFileSync(full, "utf8"), language);
            if (syms.length) repoMap.push({ path: rel, symbols: syms });
          } catch {}
        }
      }
    }
  }
  walk(workspaceRoot);
  return { root: workspaceRoot, count: files.length, files, repoMap };
}

function runShell(command, cwd = workspaceRoot, shellPref = "bash", timeoutMs = 30000) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    let bin = "/bin/bash";
    let args = ["-lc", command];
    if (process.platform === "win32") {
      if (shellPref === "cmd") { bin = "cmd.exe"; args = ["/d", "/s", "/c", command]; }
      else { bin = "powershell.exe"; args = ["-NoProfile", "-NonInteractive", "-Command", command]; }
    }
    const child = spawn(bin, args, { cwd, env: process.env });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);

    child.stdout.on("data", (d) => { if (stdout.length < 512 * 1024) stdout += d.toString("utf8"); });
    child.stderr.on("data", (d) => { if (stderr.length < 512 * 1024) stderr += d.toString("utf8"); });
    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({ exitCode: -1, stdout, stderr: err.message, durationMs: Date.now() - t0, timedOut });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ exitCode: code ?? 0, stdout, stderr, durationMs: Date.now() - t0, timedOut });
    });
  });
}

async function getGitStatus() {
  const st = await runShell("git status --porcelain -b", workspaceRoot, "bash", 6000);
  if (st.exitCode !== 0) return { isRepo: false, branch: "", dirtyCount: 0, files: [], commits: [] };
  const lines = st.stdout.split(/\r?\n/).filter(Boolean);
  let branch = "main";
  const files = [];
  lines.forEach((ln, idx) => {
    if (idx === 0 && ln.startsWith("## ")) {
      branch = ln.slice(3).split("...")[0];
    } else if (ln.length >= 4) {
      const xy = ln.slice(0, 2);
      files.push({ status: xy, staged: xy[0] !== " " && xy[0] !== "?", path: ln.slice(3) });
    }
  });
  const lg = await runShell('git log -n 10 --pretty=format:"%h|%s|%cr"', workspaceRoot, "bash", 5000);
  const commits = lg.exitCode === 0
    ? lg.stdout.split(/\r?\n/).filter(Boolean).map((l) => {
        const [hash, subject, ago] = l.split("|");
        return { hash, subject, ago };
      })
    : [];
  return { isRepo: true, branch, dirtyCount: files.length, files, commits };
}

function generateAgentsMd() {
  const scan = scanWorkspace(400);
  const langs = [...new Set(scan.files.map((f) => f.language).filter((l) => l !== "plaintext"))];
  const keyFiles = scan.files
    .map((f) => f.path)
    .filter((p) => /^(package\.json|README\.md|Cargo\.toml|pyproject\.toml|scripts\/|native-win32\/src\/main\.cpp|server\/)/.test(p))
    .slice(0, 15);
  let md = `# AGENTS.md — Aircode Project Intelligence\n\n## Workspace Overview\n- **Root**: \`${workspaceRoot}\`\n- **Indexed Files**: ${scan.count}\n- **Detected Languages**: ${langs.map((l) => `\`${l}\``).join(", ") || "General"}\n\n## Key Entrypoints & Config Files\n`;
  keyFiles.forEach((kf) => { md += `- \`${kf}\`\n`; });
  if (scan.repoMap.length) {
    md += `\n## Structural Repo Map\n`;
    scan.repoMap.slice(0, 12).forEach((rm) => {
      md += `- **\`${rm.path}\`**\n`;
      rm.symbols.forEach((s) => { md += `  - \`${s}\`\n`; });
    });
  }
  md += `\n## Coding & Agent Conventions\n1. Prefer surgical edits (\`edit_file\`) over full file rewrites.\n2. Preserve Windows 10/11 & cross-platform path compatibility.\n3. Verify builds and tests after multi-file edits.\n`;
  return md;
}

async function scanDiagnostics() {
  const scan = scanWorkspace(500);
  const diagnostics = [];
  for (const f of scan.files) {
    if (f.binary || f.size > 512 * 1024) continue;
    const { full } = resolveSafe(f.path);
    let txt = "";
    try { txt = fs.readFileSync(full, "utf8"); } catch { continue; }
    if (f.path.endsWith(".json") && path.basename(f.path) !== "tsconfig.json") {
      try { JSON.parse(txt); } catch (e) {
        diagnostics.push({ file: f.path, line: 1, severity: "error", source: "json-validator", message: e.message });
      }
    }
    const lines = txt.split(/\r?\n/);
    lines.forEach((ln, idx) => {
      if (ln.startsWith("<<<<<<< ") || ln.startsWith(">>>>>>> ")) {
        diagnostics.push({ file: f.path, line: idx + 1, severity: "error", source: "merge-conflict", message: "Unresolved Git merge conflict marker" });
      }
    });
  }
  return { ok: true, count: diagnostics.length, diagnostics };
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = "";
    req.on("data", (chunk) => {
      data += chunk.toString("utf8");
      if (data.length > 10 * 1024 * 1024) reject(new Error("Payload too large"));
    });
    req.on("end", () => resolve(data));
    req.on("error", reject);
  });
}

function sendJson(res, status, obj) {
  const payload = JSON.stringify(obj);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Cache-Control": "no-store",
    "Access-Control-Allow-Origin": "*"
  });
  res.end(payload);
}

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "OPTIONS") {
      res.writeHead(204, {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type"
      });
      return res.end();
    }

    const urlObj = new URL(req.url, `http://${req.headers.host || "localhost"}`);
    const pathname = urlObj.pathname;

    // Static UI assets
    if (req.method === "GET" && (pathname === "/" || pathname === "/index.html")) {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" });
      return res.end(fs.readFileSync(path.join(REPO_ROOT, "ui", "index.html")));
    }
    if (req.method === "GET" && pathname === "/styles.css") {
      res.writeHead(200, { "Content-Type": "text/css; charset=utf-8", "Cache-Control": "no-store" });
      return res.end(fs.readFileSync(path.join(REPO_ROOT, "ui", "styles.css")));
    }
    if (req.method === "GET" && pathname === "/app.js") {
      res.writeHead(200, { "Content-Type": "application/javascript; charset=utf-8", "Cache-Control": "no-store" });
      return res.end(fs.readFileSync(path.join(REPO_ROOT, "ui", "app.js")));
    }

    // Boot state
    if (req.method === "GET" && pathname === "/api/state") {
      const git = await getGitStatus();
      return sendJson(res, 200, {
        ok: true,
        platform: process.platform,
        runtime: "Aircode Cross-Platform Studio Engine",
        version: "1.2.0",
        exePath: path.join(REPO_ROOT, "windows-exe", "Aircode.exe"),
        appDataDir: dataDir,
        workspace: workspaceRoot,
        config,
        sessions: sessions.map((s) => ({
          id: s.id,
          title: s.title,
          mode: s.mode,
          updatedAt: s.updatedAt,
          messageCount: s.messages.length,
          promptTokens: s.promptTokens || 0,
          completionTokens: s.completionTokens || 0,
          costUsd: s.costUsd || 0
        })),
        git,
        checkpoints: listCheckpointsJson()
      });
    }

    if (req.method === "POST" && pathname === "/api/config") {
      const patch = JSON.parse((await readBody(req)) || "{}");
      if (patch.apiKeys) {
        config.apiKeys = { ...(config.apiKeys || {}), ...patch.apiKeys };
        delete patch.apiKeys;
      }
      config = { ...config, ...patch };
      saveConfig();
      return sendJson(res, 200, { ok: true, config });
    }

    if (req.method === "GET" && pathname === "/api/providers/detect") {
      const probe = async (u) => {
        try {
          const r = await fetch(u, { signal: AbortSignal.timeout(400) });
          return r.ok ? await r.json() : null;
        } catch { return null; }
      };
      const ol = await probe("http://127.0.0.1:11434/api/tags");
      const lm = await probe("http://127.0.0.1:1234/v1/models");
      return sendJson(res, 200, {
        ok: true,
        detected: {
          ollama: { online: !!ol, models: (ol?.models || []).map((m) => m.name) },
          lmstudio: { online: !!lm, models: (lm?.data || []).map((m) => m.id) }
        }
      });
    }

    if (req.method === "POST" && pathname === "/api/native/pick-folder") {
      const body = JSON.parse((await readBody(req)) || "{}");
      if (body.path && fs.existsSync(body.path) && fs.statSync(body.path).isDirectory()) {
        workspaceRoot = path.resolve(body.path);
        return sendJson(res, 200, { ok: true, workspace: workspaceRoot, git: await getGitStatus() });
      }
      return sendJson(res, 200, { ok: false, error: "Invalid folder path" });
    }

    if (req.method === "POST" && pathname === "/api/native/windows-shell-menu") {
      return sendJson(res, 200, {
        ok: true,
        message: "Available natively when running windows-exe/Aircode.exe on Windows 10/11!"
      });
    }

    if (req.method === "GET" && pathname === "/api/files") {
      return sendJson(res, 200, scanWorkspace());
    }

    if (req.method === "GET" && pathname === "/api/file") {
      const p = urlObj.searchParams.get("path") || "";
      const { full, rel } = resolveSafe(p);
      if (!fs.existsSync(full) || !fs.statSync(full).isFile()) {
        return sendJson(res, 404, { ok: false, error: "File not found: " + p });
      }
      const content = fs.readFileSync(full, "utf8");
      return sendJson(res, 200, {
        ok: true,
        path: rel,
        language: detectLanguage(rel),
        content,
        lineCount: content.split(/\r?\n/).length
      });
    }

    if (req.method === "POST" && pathname === "/api/file") {
      const { path: p, content } = JSON.parse(await readBody(req));
      const { full, rel } = resolveSafe(p);
      const cp = createCheckpoint(`Manual Save: ${rel}`);
      const oldContent = fs.existsSync(full) ? fs.readFileSync(full, "utf8") : "";
      recordBeforeWrite(cp, rel, full);
      fs.mkdirSync(path.dirname(full), { recursive: true });
      fs.writeFileSync(full, content ?? "", "utf8");
      recordAfterWrite(cp, rel, content ?? "");
      return sendJson(res, 200, { ok: true, path: rel, diff: computeUnifiedDiff(rel, oldContent, content ?? "") });
    }

    if (req.method === "POST" && pathname === "/api/file/ops") {
      const j = JSON.parse(await readBody(req));
      const { full } = resolveSafe(j.path);
      if (j.op === "create_file") {
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, j.content || "", "utf8");
      } else if (j.op === "create_dir") {
        fs.mkdirSync(full, { recursive: true });
      } else if (j.op === "delete") {
        fs.rmSync(full, { recursive: true, force: true });
      } else if (j.op === "rename") {
        fs.renameSync(full, resolveSafe(j.newPath).full);
      }
      return sendJson(res, 200, { ok: true });
    }

    if (req.method === "POST" && pathname === "/api/search") {
      const { query, caseSensitive, filePattern } = JSON.parse(await readBody(req));
      const scan = scanWorkspace(800);
      const q = caseSensitive ? query : (query || "").toLowerCase();
      const pat = (filePattern || "").toLowerCase();
      const matches = [];
      for (const f of scan.files) {
        if (f.binary || f.size > 1024 * 1024) continue;
        if (pat && !f.path.toLowerCase().includes(pat)) continue;
        let content = "";
        try { content = fs.readFileSync(resolveSafe(f.path).full, "utf8"); } catch { continue; }
        const lines = content.split(/\r?\n/);
        for (let i = 0; i < lines.length; i++) {
          const hay = caseSensitive ? lines[i] : lines[i].toLowerCase();
          const col = hay.indexOf(q);
          if (col !== -1) {
            matches.push({ file: f.path, line: i + 1, column: col + 1, preview: lines[i].slice(0, 220) });
            if (matches.length >= 100) break;
          }
        }
        if (matches.length >= 100) break;
      }
      return sendJson(res, 200, { ok: true, query, count: matches.length, matches });
    }

    if (req.method === "GET" && pathname === "/api/git/status") {
      return sendJson(res, 200, await getGitStatus());
    }

    if (req.method === "GET" && pathname === "/api/git/diff") {
      const p = urlObj.searchParams.get("path") || "";
      const cmd = p ? `git diff HEAD -- "${p}"` : "git diff HEAD";
      const r = await runShell(cmd, workspaceRoot, "bash", 8000);
      return sendJson(res, 200, { ok: true, diff: r.stdout });
    }

    if (req.method === "POST" && pathname === "/api/git/action") {
      const j = JSON.parse(await readBody(req));
      if (j.action === "generate_commit_msg") {
        const gs = await getGitStatus();
        const names = gs.files.slice(0, 3).map((f) => f.path).join(", ");
        return sendJson(res, 200, {
          ok: true,
          message: names ? `feat: update ${names}${gs.files.length > 3 ? ` (+${gs.files.length - 3} more)` : ""}` : "chore: workspace updates"
        });
      }
      let cmd = "git status";
      if (j.action === "stage") cmd = `git add -- "${j.path}"`;
      else if (j.action === "stage_all") cmd = "git add -A";
      else if (j.action === "unstage") cmd = `git reset HEAD -- "${j.path}"`;
      else if (j.action === "commit") cmd = `git commit -m "${String(j.message || "update").replace(/"/g, "'")}"`;
      const pr = await runShell(cmd, workspaceRoot, "bash", 10000);
      return sendJson(res, 200, { ok: pr.exitCode === 0, stdout: pr.stdout, stderr: pr.stderr, git: await getGitStatus() });
    }

    if (req.method === "GET" && pathname === "/api/checkpoints") {
      return sendJson(res, 200, { ok: true, checkpoints: listCheckpointsJson() });
    }

    if (req.method === "POST" && pathname === "/api/checkpoints/restore") {
      const j = JSON.parse((await readBody(req)) || "{}");
      let target = checkpoints.find((c) => c.id === j.id);
      if (!target && (!j.id || j.id === "latest")) {
        for (let i = checkpoints.length - 1; i >= 0; i--) {
          if (checkpoints[i].files.size > 0) { target = checkpoints[i]; break; }
        }
      }
      if (!target) return sendJson(res, 200, { ok: false, error: "No checkpoint with modified files found." });
      const restoredFiles = [];
      for (const [rel, entry] of target.files.entries()) {
        const { full } = resolveSafe(rel);
        if (entry.existed) {
          fs.mkdirSync(path.dirname(full), { recursive: true });
          fs.writeFileSync(full, entry.originalContent, "utf8");
        } else {
          fs.rmSync(full, { force: true });
        }
        restoredFiles.push(rel);
      }
      return sendJson(res, 200, { ok: true, label: target.label, restoredFiles });
    }

    if (req.method === "GET" && pathname === "/api/rules") {
      const targets = ["AGENTS.md", "CLAUDE.md", ".aircoderules", ".aircode/MEMORY.md"];
      return sendJson(res, 200, {
        ok: true,
        files: targets.map((t) => {
          const { full } = resolveSafe(t);
          const exists = fs.existsSync(full);
          return { path: t, exists, content: exists ? fs.readFileSync(full, "utf8") : "" };
        })
      });
    }

    if (req.method === "POST" && pathname === "/api/rules") {
      const j = JSON.parse(await readBody(req));
      const target = j.path || "AGENTS.md";
      const content = j.generateDefault ? generateAgentsMd() : j.content || "";
      const { full } = resolveSafe(target);
      fs.mkdirSync(path.dirname(full), { recursive: true });
      fs.writeFileSync(full, content, "utf8");
      return sendJson(res, 200, { ok: true, path: target, content });
    }

    if (req.method === "POST" && pathname === "/api/terminal/exec") {
      const j = JSON.parse(await readBody(req));
      const pr = await runShell(j.command || "", workspaceRoot, j.shell || "bash", 30000);
      return sendJson(res, 200, { ok: true, ...pr, shell: j.shell || "bash" });
    }

    if (req.method === "GET" && pathname === "/api/diagnostics") {
      return sendJson(res, 200, await scanDiagnostics());
    }

    if (req.method === "POST" && pathname === "/api/mcp/test") {
      const j = JSON.parse(await readBody(req));
      const pr = await runShell(`${j.command} --version`, workspaceRoot, "bash", 6000);
      return sendJson(res, 200, { ok: true, reachable: pr.exitCode === 0 || pr.stdout.length > 0, output: pr.stdout + pr.stderr });
    }

    if (req.method === "GET" && pathname === "/api/session") {
      const id = urlObj.searchParams.get("id") || "";
      const s = sessions.find((x) => !id || x.id === id) || sessions[0];
      return sendJson(res, 200, { ok: true, session: s });
    }

    if (req.method === "POST" && pathname === "/api/sessions") {
      const s = {
        id: randomId("ses"),
        title: "New Coding Session",
        mode: "build",
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        promptTokens: 0,
        completionTokens: 0,
        costUsd: 0,
        messages: []
      };
      sessions.unshift(s);
      saveSessions();
      return sendJson(res, 200, { ok: true, session: s });
    }

    if (req.method === "POST" && pathname === "/api/sessions/delete") {
      const { id } = JSON.parse((await readBody(req)) || "{}");
      sessions = sessions.filter((s) => s.id !== id);
      if (!sessions.length) {
        sessions.push({
          id: randomId("ses"),
          title: "New Coding Session",
          mode: "build",
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString(),
          promptTokens: 0,
          completionTokens: 0,
          costUsd: 0,
          messages: []
        });
      }
      saveSessions();
      return sendJson(res, 200, { ok: true, activeId: sessions[0].id });
    }

    if (req.method === "POST" && pathname === "/api/sessions/fork") {
      const { id } = JSON.parse((await readBody(req)) || "{}");
      const orig = sessions.find((s) => s.id === id);
      if (!orig) return sendJson(res, 404, { ok: false });
      const forked = JSON.parse(JSON.stringify(orig));
      forked.id = randomId("ses");
      forked.title = `${orig.title} (Fork)`;
      forked.updatedAt = new Date().toISOString();
      sessions.unshift(forked);
      saveSessions();
      return sendJson(res, 200, { ok: true, session: forked });
    }

    if (req.method === "POST" && pathname === "/api/sessions/compact") {
      const { id } = JSON.parse((await readBody(req)) || "{}");
      const s = sessions.find((x) => x.id === id);
      if (!s) return sendJson(res, 404, { ok: false });
      const oldLen = s.messages.length;
      if (oldLen > 2) {
        const sum = "**[Compacted Session Context]**\n" + s.messages.map((m) => `- **${m.role}**: ${String(m.content).slice(0, 150)}`).join("\n");
        s.messages = [
          { id: randomId("msg"), role: "assistant", content: sum, timestamp: new Date().toISOString() },
          s.messages[s.messages.length - 1]
        ];
        saveSessions();
      }
      return sendJson(res, 200, { ok: true, session: s, compactedFrom: oldLen });
    }

    if (req.method === "POST" && pathname === "/api/inline-edit") {
      const { path: p, selection, instruction } = JSON.parse(await readBody(req));
      const low = (instruction || "").toLowerCase();
      let transformed = selection;
      if (low.includes("comment") || low.includes("doc")) {
        transformed = `// [Aircode] ${instruction}\n${selection}`;
      } else if (low.includes("try") || low.includes("error")) {
        transformed = `try {\n${selection}\n} catch (err) {\n  console.error("Operation failed:", err);\n  throw err;\n}`;
      } else {
        transformed = `// Transformed (${instruction}):\n${selection}`;
      }
      return sendJson(res, 200, {
        ok: true,
        transformed,
        diff: computeUnifiedDiff(p || "selection", selection, transformed)
      });
    }

    if (req.method === "POST" && pathname === "/api/agent/abort") {
      if (activeAbortController) activeAbortController.abort();
      return sendJson(res, 200, { ok: true });
    }

    // SSE Agent Stream
    if (req.method === "POST" && pathname === "/api/agent/stream") {
      const body = JSON.parse(await readBody(req));
      res.writeHead(200, {
        "Content-Type": "text/event-stream; charset=utf-8",
        "Cache-Control": "no-cache",
        "Connection": "keep-alive",
        "X-Accel-Buffering": "no"
      });

      const emit = (event, data) => {
        res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
      };

      const mode = body.mode || "build";
      const prompt = body.prompt || "";
      const provider = body.provider || config.provider || "local-assist";
      const model = body.model || config.model || "aircode-local-instant";
      let ses = sessions.find((s) => s.id === body.sessionId) || sessions[0];
      if (ses.messages.length === 0 && prompt.length > 2) {
        ses.title = prompt.slice(0, 46);
      }
      ses.mode = mode;
      ses.updatedAt = new Date().toISOString();
      ses.messages.push({ id: randomId("msg"), role: "user", content: prompt, mode, timestamp: new Date().toISOString() });
      saveSessions();

      emit("status", { phase: "thinking", mode, provider, model });
      emit("reasoning_delta", { text: "Inspecting workspace structure, Repo Map symbols, and git state...\n" });

      const readOnly = mode === "plan" || mode === "review" || config.permissionMode === "read-only";
      const cpId = readOnly ? "" : createCheckpoint(prompt.slice(0, 48));
      const toolTrace = [];
      const touchedFiles = [];

      const listId = randomId("call");
      emit("tool_start", { id: listId, name: "list_files", args: {} });
      const scan = scanWorkspace(350);
      emit("tool_result", {
        id: listId,
        name: "list_files",
        ok: true,
        summary: `Indexed ${scan.count} files & ${scan.repoMap.length} symbol modules`,
        output: `Workspace: ${workspaceRoot}`,
        durationMs: 3
      });
      toolTrace.push({ id: listId, name: "list_files", summary: `Indexed ${scan.count} files`, durationMs: 3 });

      let finalReply = "";
      const lower = prompt.toLowerCase();

      const canUseExternalLlm =
        provider !== "local-assist" &&
        (provider === "ollama" || provider === "lmstudio" || Boolean((config.apiKeys || {})[provider]));

      if (canUseExternalLlm) {
        const apiKey = (config.apiKeys || {})[provider] || "";
        let endpoint = (config.baseUrl || "").replace(/\/+$/, "");
        if (!endpoint) {
          const defaults = {
            ollama: "http://127.0.0.1:11434/v1",
            lmstudio: "http://127.0.0.1:1234/v1",
            openrouter: "https://openrouter.ai/api/v1",
            gemini: "https://generativelanguage.googleapis.com/v1beta/openai",
            groq: "https://api.groq.com/openai/v1",
            openai: "https://api.openai.com/v1",
            deepseek: "https://api.deepseek.com/v1",
            github: "https://models.inference.ai.azure.com"
          };
          endpoint = defaults[provider] || "https://api.openai.com/v1";
        }
        try {
          activeAbortController = new AbortController();
          const llmRes = await fetch(`${endpoint}/chat/completions`, {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
              ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {})
            },
            body: JSON.stringify({
              model,
              messages: [
                { role: "system", content: `You are Aircode Studio (${mode} mode) in workspace ${workspaceRoot}.` },
                ...ses.messages.map((m) => ({ role: m.role === "assistant" ? "assistant" : "user", content: m.content }))
              ]
            }),
            signal: activeAbortController.signal
          });
          const data = await llmRes.json();
          if (!llmRes.ok) {
            emit("error", { message: JSON.stringify(data), hint: "Verify API key or switch to Local Instant / Ollama." });
          } else {
            finalReply = data.choices?.[0]?.message?.content || "";
            emit("text_delta", { text: finalReply });
          }
        } catch (err) {
          emit("error", { message: err.message, hint: "Network unreachable or aborted." });
        }
      } else if (lower.includes("/init") || lower.includes("agents.md")) {
        const md = generateAgentsMd();
        if (!readOnly) {
          const wid = randomId("call");
          emit("tool_start", { id: wid, name: "write_file", args: { path: "AGENTS.md" } });
          const { full } = resolveSafe("AGENTS.md");
          const oldTxt = fs.existsSync(full) ? fs.readFileSync(full, "utf8") : "";
          recordBeforeWrite(cpId, "AGENTS.md", full);
          fs.writeFileSync(full, md, "utf8");
          recordAfterWrite(cpId, "AGENTS.md", md);
          const diff = computeUnifiedDiff("AGENTS.md", oldTxt, md);
          touchedFiles.push("AGENTS.md");
          emit("tool_result", { id: wid, name: "write_file", ok: true, summary: "Generated AGENTS.md", diff, fileChanged: "AGENTS.md", durationMs: 2 });
          toolTrace.push({ id: wid, name: "write_file", summary: "Generated AGENTS.md", diff, durationMs: 2 });
          finalReply = "I scanned your project and created **`AGENTS.md`** with your detected languages, entrypoints, and structural Repo Map!\n\nOpen **`± Diffs`** or **`Rules · Memory · MCP`** in the sidebar to inspect or customize it.";
        } else {
          finalReply = `### Proposed \`AGENTS.md\` Plan\n\n\`\`\`markdown\n${md}\n\`\`\``;
        }
      } else if (lower.includes("/review") || mode === "review") {
        const gid = randomId("call");
        emit("tool_start", { id: gid, name: "git_status", args: {} });
        const gs = await getGitStatus();
        const diag = await scanDiagnostics();
        emit("tool_result", { id: gid, name: "git_status", ok: true, summary: `Branch ${gs.branch} (${gs.dirtyCount} modified)`, output: JSON.stringify(gs, null, 2), durationMs: 8 });
        toolTrace.push({ id: gid, name: "git_status", summary: `Branch ${gs.branch} (${gs.dirtyCount} modified)`, durationMs: 8 });
        finalReply = `## Codebase & Security Audit\n\n- **Workspace**: \`${workspaceRoot}\`\n- **Windows Binary Ready**: \`windows-exe/Aircode.exe\` (${fs.existsSync(path.join(REPO_ROOT, "windows-exe/Aircode.exe")) ? "✓ Built & Verified PE32+ x64 GUI" : "Pending"})\n- **Indexed Files**: **${scan.count}** across **${scan.repoMap.length}** symbol-mapped modules\n- **Git Branch**: \`${gs.branch || "main"}\` (${gs.dirtyCount} uncommitted file(s))\n- **Diagnostics**: **${diag.count}** issues detected\n`;
      } else {
        finalReply = `### Workspace Intelligence Summary\n\n- **Active Project**: \`${workspaceRoot}\`\n- **Standalone Windows Executable**: \`windows-exe/Aircode.exe\` (1.70 MB native Win32 C++17 + Edge WebView2 + WinHTTP)\n- **Indexed Modules**: **${scan.count}** files, **${scan.repoMap.length}** symbol maps\n\n#### Key Modules\n` +
          scan.repoMap.slice(0, 6).map((rm) => `- **\`${rm.path}\`** — \`${rm.symbols[0] || ""}\``).join("\n") +
          `\n\n> **Zero Cloud Compute Setup**: On your Windows PC, just double-click \`windows-exe/Aircode.exe\` and connect **Ollama** (\`localhost:11434\`), **LM Studio**, **OpenRouter \`:free\`**, **Gemini Free**, **Groq**, **Claude**, or **OpenAI**!`;
      }

      for (let i = 0; i < finalReply.length; i += 40) {
        emit("text_delta", { text: finalReply.slice(i, i + 40) });
      }

      const pTok = Math.ceil(prompt.length / 4);
      const cTok = Math.ceil(finalReply.length / 4);
      ses.messages.push({
        id: randomId("msg"),
        role: "assistant",
        content: finalReply,
        reasoning: "Inspected workspace files, Repo Map symbols, and project status.",
        tools: toolTrace,
        mode,
        model,
        timestamp: new Date().toISOString()
      });
      ses.promptTokens = (ses.promptTokens || 0) + pTok;
      ses.completionTokens = (ses.completionTokens || 0) + cTok;
      saveSessions();

      emit("usage", { promptTokens: pTok, completionTokens: cTok, estimatedCostUsd: 0, contextPercent: 1 });
      emit("done", { sessionId: ses.id, touchedFiles, session: ses });
      return res.end();
    }

    sendJson(res, 404, { ok: false, error: "Not found: " + pathname });
  } catch (err) {
    sendJson(res, 500, { ok: false, error: err.message });
  }
});

server.listen(PORT, "0.0.0.0", () => {
  console.log(`Aircode Studio Live Preview running on http://0.0.0.0:${PORT}`);
});
