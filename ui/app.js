(() => {
  "use strict";

  const PROVIDER_MODELS = {
    "local-assist": ["aircode-local-instant"],
    ollama: ["qwen2.5-coder:7b", "qwen2.5-coder:14b", "deepseek-coder-v2:latest", "qwen3:8b", "llama3.3:latest"],
    lmstudio: ["qwen2.5-coder-7b-instruct", "deepseek-r1-distill-qwen-7b", "local-model"],
    openrouter: [
      "qwen/qwen3-coder:free",
      "deepseek/deepseek-chat-v3-0324:free",
      "deepseek/deepseek-r1:free",
      "meta-llama/llama-3.3-70b-instruct:free",
      "google/gemma-3-27b-it:free",
      "anthropic/claude-sonnet-4",
      "openai/gpt-4.1"
    ],
    gemini: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"],
    groq: ["llama-3.3-70b-versatile", "deepseek-r1-distill-llama-70b", "qwen-qwq-32b"],
    anthropic: ["claude-sonnet-4-5", "claude-3-7-sonnet-latest", "claude-3-5-haiku-latest", "claude-opus-4-5"],
    openai: ["gpt-4.1", "gpt-4.1-mini", "o4-mini", "gpt-4o"],
    deepseek: ["deepseek-chat", "deepseek-reasoner"],
    github: ["gpt-4o", "DeepSeek-R1", "Codestral-2501"],
    custom: ["custom-model"]
  };

  const PROVIDER_NAMES = {
    "local-assist": "Local Instant",
    ollama: "Ollama Local",
    lmstudio: "LM Studio",
    openrouter: "OpenRouter",
    gemini: "Gemini",
    groq: "Groq LPU",
    anthropic: "Claude",
    openai: "OpenAI",
    deepseek: "DeepSeek",
    github: "GitHub Models",
    custom: "Custom API"
  };

  const SLASH_COMMANDS = [
    { slash: "/init", desc: "Scan workspace & generate AGENTS.md intelligence file" },
    { slash: "/review", desc: "Run comprehensive code, bug & security audit" },
    { slash: "/compact", desc: "Compress session context window to save tokens" },
    { slash: "/undo", desc: "Revert workspace files to latest checkpoint" },
    { slash: "/fix", desc: "Inspect workspace problems & terminal errors and fix them" },
    { slash: "/explain", desc: "Explain project architecture and key modules" },
    { slash: "/test", desc: "Generate or run unit tests for active module" },
    { slash: "/commit", desc: "Generate AI commit message from git diff" },
    { slash: "/clear", desc: "Start a clean coding session" }
  ];

  const MODES = ["build", "plan", "turbo", "debug", "review"];

  const state = {
    platform: "win32",
    workspace: "",
    config: {
      provider: "local-assist",
      model: "aircode-local-instant",
      permissionMode: "auto-edit",
      theme: "obsidian",
      shell: "powershell",
      maxSteps: 12,
      apiKeys: {},
      mcpServers: [],
      customCommands: []
    },
    files: [],
    repoMap: [],
    openTabs: [], // { id, type: 'file'|'diff'|'welcome', path, name, content, savedContent, language }
    activeTabId: "welcome",
    sessions: [],
    activeSession: null,
    activeMode: "build",
    attachments: [], // { id, label, content }
    checkpoints: [],
    git: { isRepo: false, branch: "", files: [], commits: [] },
    diagnostics: [],
    lastTerminalOutput: "",
    termHistory: [],
    termHistIdx: -1,
    streaming: false,
    pendingInlineTransform: null,
    detectedLocal: {}
  };

  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => Array.from(document.querySelectorAll(sel));

  function toast(msg) {
    const el = document.createElement("div");
    el.className = "toast";
    el.textContent = msg;
    $("#toastContainer").appendChild(el);
    setTimeout(() => el.remove(), 3200);
  }

  function escapeHtml(str) {
    return String(str ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  async function api(url, opts = {}) {
    const res = await fetch(url, {
      headers: { "Content-Type": "application/json", ...(opts.headers || {}) },
      ...opts
    });
    return res.json();
  }

  // ==================== SYNTAX HIGHLIGHTER & MARKDOWN RENDERER ====================
  function highlightCodeHtml(code, lang) {
    let s = escapeHtml(code);
    // Comments
    s = s.replace(/(\/\/[^\n]*|#[^\n]*)/g, '<span class="syn-cmt">$1</span>');
    // Strings
    s = s.replace(/(&quot;(?:\\.|[^&])*?&quot;|'(?:\\.|[^'\n])*?')/g, '<span class="syn-str">$1</span>');
    // Keywords
    s = s.replace(
      /\b(const|let|var|function|async|await|return|if|else|for|while|switch|case|break|continue|class|struct|interface|type|export|import|from|default|try|catch|throw|new|public|private|protected|namespace|inline|void|int|bool|auto|std|def|elif|None|True|False|fn|pub|impl|mut|use|match|package|func)\b/g,
      '<span class="syn-kw">$1</span>'
    );
    // Numbers
    s = s.replace(/\b(\d+(?:\.\d+)?)\b/g, '<span class="syn-num">$1</span>');
    return s;
  }

  function renderDiffHtml(diffText) {
    if (!diffText || !diffText.trim()) {
      return '<div class="muted" style="padding:8px 12px;">No line differences.</div>';
    }
    const lines = diffText.split("\n");
    return (
      '<div class="diff-pre">' +
      lines
        .map((ln) => {
          let cls = "diff-ln";
          if (ln.startsWith("+") && !ln.startsWith("+++")) cls += " add";
          else if (ln.startsWith("-") && !ln.startsWith("---")) cls += " del";
          else if (ln.startsWith("@@")) cls += " hunk";
          return `<div class="${cls}">${escapeHtml(ln)}</div>`;
        })
        .join("") +
      "</div>"
    );
  }

  function renderMarkdown(md) {
    if (!md) return "";
    const blocks = [];
    let processed = String(md).replace(/```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g, (_, lang, code) => {
      const idx = blocks.length;
      const encoded = encodeURIComponent(code);
      blocks.push(
        `<div class="md-codeblock">
          <div class="md-codehead">
            <span>${escapeHtml(lang || "code")}</span>
            <div style="display:flex;gap:6px;">
              <button class="mini-btn copy-code-btn" data-code="${encoded}">Copy</button>
              <button class="mini-btn apply-code-btn" data-code="${encoded}">Apply to Editor</button>
            </div>
          </div>
          <pre><code>${highlightCodeHtml(code, lang)}</code></pre>
        </div>`
      );
      return `@@CODEBLOCK_${idx}@@`;
    });

    processed = escapeHtml(processed);
    processed = processed.replace(/^### (.+)$/gm, "<h4>$1</h4>");
    processed = processed.replace(/^## (.+)$/gm, "<h3>$1</h3>");
    processed = processed.replace(/^# (.+)$/gm, "<h2>$1</h2>");
    processed = processed.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    processed = processed.replace(/`([^`\n]+)`/g, "<code>$1</code>");
    processed = processed.replace(/^- (.+)$/gm, "<li>$1</li>");
    processed = processed.replace(/(<li>[\s\S]*?<\/li>)/g, "<ul>$1</ul>");
    processed = processed.replace(/<\/ul>\s*<ul>/g, "");
    processed = processed.replace(/\n\n+/g, "</p><p>");
    processed = `<p>${processed}</p>`;

    blocks.forEach((html, idx) => {
      processed = processed.replace(`@@CODEBLOCK_${idx}@@`, html);
    });
    return processed;
  }

  // ==================== BOOT & STATE HYDRATION ====================
  async function boot() {
    bindEvents();
    await refreshState();
    await Promise.all([refreshFiles(), loadActiveSession(), refreshRules(), refreshDiagnostics()]);
    appendTerminalLine("Aircode Studio Ready — Native Multi-Shell Terminal initialized.\nType any command below or use the AI Composer on the right.", "info");
  }

  async function refreshState() {
    const data = await api("/api/state");
    if (!data.ok) return;
    state.platform = data.platform || "win32";
    state.workspace = data.workspace || "";
    state.config = { ...state.config, ...(data.config || {}) };
    state.sessions = data.sessions || [];
    state.git = data.git || state.git;
    state.checkpoints = data.checkpoints || [];

    $("#runtimeBadge").textContent = state.platform === "win32" ? "WIN32 NATIVE" : "PREVIEW LIVE";
    $("#workspacePathLabel").textContent = state.workspace.split(/[\\/]/).filter(Boolean).slice(-2).join("/") || state.workspace;
    $("#workspacePathLabel").title = state.workspace;
    $("#winExePathLabel").textContent = data.exePath || "windows-exe/Aircode.exe";
    $("#winAppDataLabel").textContent = data.appDataDir || "%APPDATA%\\Aircode";

    applyTheme(state.config.theme || "obsidian");
    $("#themeSelect").value = state.config.theme || "obsidian";
    $("#permissionSelect").value = state.config.permissionMode || "auto-edit";
    $("#shellSelect").value = state.config.shell || (state.platform === "win32" ? "powershell" : "bash");
    updateShellPromptLabel();

    updateModelPill();
    renderSessionsList();
    renderGitPanel();
    renderCheckpoints();
    renderMcpList();
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", theme);
  }

  function updateModelPill() {
    const p = state.config.provider || "local-assist";
    const m = state.config.model || "aircode-local-instant";
    $("#activeProviderLabel").textContent = PROVIDER_NAMES[p] || p;
    $("#activeModelLabel").textContent = m;
    const isFree =
      p === "local-assist" ||
      p === "ollama" ||
      p === "lmstudio" ||
      m.includes(":free") ||
      p === "gemini" ||
      p === "groq";
    $("#costTierBadge").textContent = isFree ? "FREE" : "API";
  }

  // ==================== EXPLORER & REPO MAP ====================
  async function refreshFiles() {
    const data = await api("/api/files");
    state.files = data.files || [];
    state.repoMap = data.repoMap || [];
    $("#sbFilesCount").textContent = `${state.files.length} files indexed`;
    renderFileTree();
    renderRepoMap();
  }

  function fileIcon(lang) {
    const map = {
      typescript: "🟦",
      javascript: "🟨",
      python: "🐍",
      cpp: "⚙️",
      rust: "🦀",
      go: "🐹",
      json: "📋",
      html: "🌐",
      css: "🎨",
      markdown: "📝",
      powershell: "💠",
      shell: "💲"
    };
    return map[lang] || "📄";
  }

  function renderFileTree() {
    const q = ($("#fileFilterInput").value || "").toLowerCase();
    const filtered = state.files.filter((f) => !q || f.path.toLowerCase().includes(q));
    const tree = $("#fileTree");
    if (!filtered.length) {
      tree.innerHTML = '<div class="muted" style="padding:10px;">No matching files.</div>';
      return;
    }
    tree.innerHTML = filtered
      .map((f) => {
        const active = state.activeTabId === `file:${f.path}` ? "active" : "";
        return `<div class="tree-row ${active}" data-open-file="${escapeHtml(f.path)}">
          <div class="tree-left">
            <span>${fileIcon(f.language)}</span>
            <span class="tree-name" title="${escapeHtml(f.path)}">${escapeHtml(f.path)}</span>
          </div>
          <button class="tree-at-btn" data-attach-file="${escapeHtml(f.path)}" title="Attach @${escapeHtml(f.path)} to AI">@</button>
        </div>`;
      })
      .join("");
  }

  function renderRepoMap() {
    const box = $("#repoMapContainer");
    if (!state.repoMap.length) {
      box.innerHTML = '<div class="muted">No code symbols indexed yet.</div>';
      return;
    }
    box.innerHTML = state.repoMap
      .map(
        (rm) => `<div class="rm-item">
          <div class="rm-path" data-open-file="${escapeHtml(rm.path)}">⚡ ${escapeHtml(rm.path)}</div>
          ${(rm.symbols || []).map((s) => `<div class="rm-sym">${escapeHtml(s)}</div>`).join("")}
        </div>`
      )
      .join("");
  }

  // ==================== MULTI-TAB CODE EDITOR & DIFF CENTER ====================
  async function openFileInEditor(relPath, targetLine = null) {
    const tabId = `file:${relPath}`;
    let tab = state.openTabs.find((t) => t.id === tabId);
    if (!tab) {
      const data = await api(`/api/file?path=${encodeURIComponent(relPath)}`);
      if (!data.ok) {
        toast(data.error || "Cannot open file");
        return;
      }
      tab = {
        id: tabId,
        type: "file",
        path: data.path,
        name: data.path.split("/").pop(),
        content: data.content,
        savedContent: data.content,
        language: data.language
      };
      state.openTabs.push(tab);
    }
    state.activeTabId = tabId;
    renderWorkbench();

    if (targetLine && targetLine > 1) {
      const ta = $("#codeTextarea");
      const lines = ta.value.split("\n");
      let pos = 0;
      for (let i = 0; i < Math.min(targetLine - 1, lines.length); i++) {
        pos += lines[i].length + 1;
      }
      ta.focus();
      ta.setSelectionRange(pos, pos + (lines[targetLine - 1]?.length || 0));
      const lineHeight = 19.3;
      ta.scrollTop = Math.max(0, (targetLine - 5) * lineHeight);
      syncEditorScroll();
    }
  }

  function openDiffCenterTab() {
    let tab = state.openTabs.find((t) => t.id === "diff-center");
    if (!tab) {
      tab = { id: "diff-center", type: "diff", name: "± Diff Review" };
      state.openTabs.push(tab);
    }
    state.activeTabId = "diff-center";
    renderWorkbench();
    refreshDiffCenter();
  }

  function renderWorkbench() {
    const tabsEl = $("#editorTabs");
    const allTabs = [{ id: "welcome", type: "welcome", name: "⚡ Studio Home" }, ...state.openTabs];
    tabsEl.innerHTML = allTabs
      .map((t) => {
        const dirty = t.type === "file" && t.content !== t.savedContent ? " ●" : "";
        const closeBtn =
          t.id !== "welcome"
            ? `<button class="ed-tab-close" data-close-tab="${escapeHtml(t.id)}">✕</button>`
            : "";
        return `<div class="ed-tab ${state.activeTabId === t.id ? "active" : ""}" data-switch-tab="${escapeHtml(t.id)}">
          <span>${escapeHtml(t.name)}${dirty}</span>
          ${closeBtn}
        </div>`;
      })
      .join("");

    $("#welcomeView").classList.toggle("hidden", state.activeTabId !== "welcome");
    $("#diffCenterView").classList.toggle("hidden", state.activeTabId !== "diff-center");

    const activeFileTab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
    $("#codeEditorView").classList.toggle("hidden", !activeFileTab);

    if (activeFileTab) {
      $("#editorFilePath").textContent = `${activeFileTab.path} · ${activeFileTab.language}`;
      const ta = $("#codeTextarea");
      if (ta.value !== activeFileTab.content) {
        ta.value = activeFileTab.content;
      }
      updateEditorDecorations(activeFileTab);
    }
    renderFileTree();
  }

  function updateEditorDecorations(tab) {
    const text = tab.content || "";
    const lineCount = text.split("\n").length;
    const nums = [];
    for (let i = 1; i <= lineCount; i++) nums.push(i);
    $("#lineGutter").textContent = nums.join("\n");
    $("#codeHighlight").innerHTML = highlightCodeHtml(text, tab.language) + "\n";
    syncEditorScroll();
  }

  function syncEditorScroll() {
    const ta = $("#codeTextarea");
    const hi = $("#codeHighlight");
    const gut = $("#lineGutter");
    hi.scrollTop = ta.scrollTop;
    hi.scrollLeft = ta.scrollLeft;
    gut.scrollTop = ta.scrollTop;
  }

  async function saveActiveFile() {
    const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
    if (!tab) return;
    const res = await api("/api/file", {
      method: "POST",
      body: JSON.stringify({ path: tab.path, content: tab.content })
    });
    if (res.ok) {
      tab.savedContent = tab.content;
      renderWorkbench();
      toast(`Saved ${tab.path} (checkpoint recorded)`);
      refreshState();
    } else {
      toast(res.error || "Save failed");
    }
  }

  async function refreshDiffCenter() {
    const container = $("#diffCardsContainer");
    const cpData = await api("/api/checkpoints");
    state.checkpoints = cpData.checkpoints || [];
    const gitDiff = await api("/api/git/diff");

    let html = "";
    if (state.checkpoints.length) {
      for (const cp of state.checkpoints.slice(0, 6)) {
        for (const f of cp.files || []) {
          html += `<div class="diff-card">
            <div class="diff-card-head">
              <span>📸 [${escapeHtml(cp.label)}] ${escapeHtml(f.path)}</span>
              <button class="mini-btn" data-restore-cp="${escapeHtml(cp.id)}">↩ Restore Checkpoint</button>
            </div>
            ${renderDiffHtml(f.diff)}
          </div>`;
        }
      }
    }
    if (gitDiff.diff && gitDiff.diff.trim()) {
      html += `<div class="diff-card">
        <div class="diff-card-head"><span>🌿 Working Tree Git Diff</span></div>
        ${renderDiffHtml(gitDiff.diff)}
      </div>`;
    }
    if (!html) {
      html = '<div class="muted" style="padding:18px;">No modified file diffs yet. Ask the AI Agent in Build/Turbo mode or edit a file to inspect live surgical diffs here!</div>';
    }
    container.innerHTML = html;
  }

  // ==================== GIT & CHECKPOINTS ====================
  function renderGitPanel() {
    const g = state.git || {};
    $("#gitBranchName").textContent = g.isRepo ? g.branch || "HEAD" : "Not a git repo";
    $("#sbBranch").textContent = g.isRepo ? `🌿 ${g.branch || "main"}` : "📂 Local Folder";

    const dirty = (g.files || []).length;
    const badge = $("#gitDirtyBadge");
    badge.textContent = dirty;
    badge.classList.toggle("hidden", dirty === 0);

    const fl = $("#gitFileList");
    if (!dirty) {
      fl.innerHTML = '<div class="muted">Working tree clean</div>';
    } else {
      fl.innerHTML = g.files
        .map(
          (f) => `<div class="git-item">
            <span style="cursor:pointer" data-open-file="${escapeHtml(f.path)}"><code>${escapeHtml(f.status)}</code> ${escapeHtml(f.path)}</span>
            <div style="display:flex;gap:4px;">
              <button class="mini-btn" data-git-act="${f.staged ? "unstage" : "stage"}" data-git-path="${escapeHtml(f.path)}">${f.staged ? "Unstage" : "+"}</button>
            </div>
          </div>`
        )
        .join("");
    }

    const cl = $("#gitCommitList");
    cl.innerHTML = (g.commits || [])
      .map(
        (c) => `<div class="git-item">
          <span title="${escapeHtml(c.subject)}"><code>${escapeHtml(c.hash)}</code> ${escapeHtml(c.subject.slice(0, 34))}</span>
          <span class="muted">${escapeHtml(c.ago)}</span>
        </div>`
      )
      .join("");
  }

  function renderCheckpoints() {
    $("#sbCheckpoints").textContent = `${state.checkpoints.length} checkpoints`;
    const list = $("#checkpointList");
    if (!state.checkpoints.length) {
      list.innerHTML = '<div class="muted">Auto-created before every file edit</div>';
      return;
    }
    list.innerHTML = state.checkpoints
      .map(
        (cp) => `<div class="cp-item">
          <div>
            <strong>${escapeHtml(cp.label)}</strong>
            <div class="muted">${cp.fileCount} file(s)</div>
          </div>
          <button class="mini-btn accent" data-restore-cp="${escapeHtml(cp.id)}">Restore</button>
        </div>`
      )
      .join("");
  }

  async function restoreCheckpoint(id = "latest") {
    const res = await api("/api/checkpoints/restore", {
      method: "POST",
      body: JSON.stringify({ id })
    });
    if (res.ok) {
      toast(`Restored checkpoint: ${res.label} (${(res.restoredFiles || []).length} file(s))`);
      // Reload any open restored tabs
      for (const p of res.restoredFiles || []) {
        const idx = state.openTabs.findIndex((t) => t.id === `file:${p}`);
        if (idx >= 0) {
          const fresh = await api(`/api/file?path=${encodeURIComponent(p)}`);
          if (fresh.ok) {
            state.openTabs[idx].content = fresh.content;
            state.openTabs[idx].savedContent = fresh.content;
          } else {
            state.openTabs.splice(idx, 1);
          }
        }
      }
      await Promise.all([refreshState(), refreshFiles()]);
      renderWorkbench();
    } else {
      toast(res.error || "No checkpoint to restore yet");
    }
  }

  // ==================== RULES, MEMORY & MCP ====================
  async function refreshRules() {
    const data = await api("/api/rules");
    if (!data.ok) return;
    state.rulesFiles = data.files || [];
    syncSelectedRulesFile();
  }

  function syncSelectedRulesFile() {
    const sel = $("#rulesFileSelect").value;
    const match = (state.rulesFiles || []).find((f) => f.path === sel);
    $("#rulesEditorArea").value = match ? match.content : "";
  }

  function renderMcpList() {
    const servers = state.config.mcpServers || [];
    $("#mcpServerList").innerHTML = servers
      .map(
        (m, idx) => `<div class="mcp-card">
          <div style="display:flex;justify-content:space-between;align-items:center;">
            <strong>🔌 ${escapeHtml(m.name)}</strong>
            <div style="display:flex;gap:4px;">
              <button class="mini-btn" data-test-mcp="${idx}">Test</button>
              <input type="checkbox" data-toggle-mcp="${idx}" ${m.enabled ? "checked" : ""} />
            </div>
          </div>
          <div class="muted" style="margin-top:3px;font-family:Consolas,monospace;font-size:10.5px;">${escapeHtml(m.command)}</div>
        </div>`
      )
      .join("");
  }

  // ==================== TERMINAL & DIAGNOSTICS ====================
  function updateShellPromptLabel() {
    const sh = $("#shellSelect").value;
    $("#termPromptLabel").textContent = sh === "powershell" ? "PS>" : sh === "cmd" ? "C:\\>" : "$";
  }

  function appendTerminalLine(text, kind = "out") {
    const out = $("#terminalOutput");
    const div = document.createElement("div");
    if (kind === "cmd") div.className = "term-cmd-echo";
    else if (kind === "err") div.className = "term-stderr";
    div.textContent = text;
    out.appendChild(div);
    out.scrollTop = out.scrollHeight;
  }

  async function runTerminalCommand() {
    const inp = $("#terminalCmdInput");
    const cmd = inp.value.trim();
    if (!cmd) return;
    state.termHistory.push(cmd);
    state.termHistIdx = state.termHistory.length;
    inp.value = "";

    const shell = $("#shellSelect").value;
    appendTerminalLine(`${$("#termPromptLabel").textContent} ${cmd}`, "cmd");

    const res = await api("/api/terminal/exec", {
      method: "POST",
      body: JSON.stringify({ command: cmd, shell })
    });
    if (res.stdout) appendTerminalLine(res.stdout, "out");
    if (res.stderr) appendTerminalLine(res.stderr, "err");
    appendTerminalLine(`[exit ${res.exitCode} · ${res.durationMs}ms]`, res.exitCode === 0 ? "out" : "err");
    state.lastTerminalOutput = `$ ${cmd}\n${res.stdout || ""}\n${res.stderr || ""}`;
    refreshState();
  }

  async function refreshDiagnostics() {
    const data = await api("/api/diagnostics");
    state.diagnostics = data.diagnostics || [];
    $("#diagCountBadge").textContent = state.diagnostics.length;
    const list = $("#problemsList");
    if (!state.diagnostics.length) {
      list.innerHTML = '<div class="muted" style="padding:8px 0;">✓ Zero static diagnostics or conflict markers found.</div>';
      return;
    }
    list.innerHTML = state.diagnostics
      .map(
        (d) => `<div class="prob-item" data-open-file="${escapeHtml(d.file)}" data-open-line="${d.line || 1}">
          <span><strong>[${escapeHtml(d.severity)}]</strong> <code>${escapeHtml(d.file)}:${d.line || 1}</code> — ${escapeHtml(d.message)}</span>
          <span class="muted">${escapeHtml(d.source)}</span>
        </div>`
      )
      .join("");
  }

  // ==================== SESSIONS & REAL-TIME SSE AGENT COMPOSER ====================
  function renderSessionsList() {
    const q = ($("#sessionFilterInput").value || "").toLowerCase();
    const list = $("#sessionList");
    const activeId = state.activeSession?.id;
    list.innerHTML = (state.sessions || [])
      .filter((s) => !q || (s.title || "").toLowerCase().includes(q))
      .map(
        (s) => `<div class="session-card ${s.id === activeId ? "active" : ""}" data-select-session="${escapeHtml(s.id)}">
          <div class="session-card-top">
            <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(s.title)}</span>
            <div class="session-card-btns">
              <button class="mini-btn" data-fork-session="${escapeHtml(s.id)}" title="Fork Thread">⑂</button>
              <button class="mini-btn" data-del-session="${escapeHtml(s.id)}" title="Delete">✕</button>
            </div>
          </div>
          <div class="session-meta">
            <span>${escapeHtml((s.mode || "build").toUpperCase())} · ${s.messageCount || 0} msgs</span>
            <span>$${Number(s.costUsd || 0).toFixed(4)}</span>
          </div>
        </div>`
      )
      .join("");
  }

  async function loadActiveSession(id = "") {
    const data = await api(`/api/session${id ? `?id=${encodeURIComponent(id)}` : ""}`);
    if (!data.ok) return;
    state.activeSession = data.session;
    setAgentMode(state.activeSession.mode || "build");
    renderChatStream();
    renderSessionsList();
    updateTokenMeter();
  }

  function updateTokenMeter() {
    const s = state.activeSession;
    if (!s) return;
    const tot = (s.promptTokens || 0) + (s.completionTokens || 0);
    const cost = Number(s.costUsd || 0).toFixed(4);
    $("#tokenMeter").textContent = `${tot.toLocaleString()} tok · $${cost}`;
  }

  function setAgentMode(mode) {
    state.activeMode = mode;
    $$(".mode-tab").forEach((b) => b.classList.toggle("active", b.dataset.mode === mode));
    $("#sbMode").textContent = `Mode: ${mode.toUpperCase()}`;
  }

  function renderChatStream() {
    const stream = $("#chatStream");
    const msgs = state.activeSession?.messages || [];
    if (!msgs.length) {
      stream.innerHTML = `<div class="msg-card assistant">
        <div class="msg-head"><span class="msg-role">⚡ Aircode Autonomous Agent</span><span>Ready</span></div>
        <div class="msg-body">
          <p>Welcome to <strong>Aircode Studio</strong>! Pick a mode above (<kbd>Tab</kbd> cycles modes) or try one of these:</p>
          <ul>
            <li>Type <code>/init</code> to scan your codebase &amp; create <code>AGENTS.md</code></li>
            <li>Type <code>/review</code> for an instant architecture &amp; diagnostics audit</li>
            <li>Type <code>@</code> to attach any file, git diff, or terminal output</li>
            <li>Click <strong>Local Instant · FREE</strong> at the top to connect <strong>Ollama</strong>, <strong>LM Studio</strong>, <strong>OpenRouter Free</strong>, <strong>Gemini</strong>, <strong>Claude</strong>, or <strong>OpenAI</strong>!</li>
          </ul>
        </div>
      </div>`;
      return;
    }

    stream.innerHTML = msgs
      .map((m) => {
        const isUser = m.role === "user";
        const thinkHtml = m.reasoning
          ? `<div class="think-box">
              <div class="think-summary" onclick="this.nextElementSibling.classList.toggle('hidden')">🧠 Thought Process (click to toggle)</div>
              <div class="think-content hidden">${escapeHtml(m.reasoning)}</div>
            </div>`
          : "";
        const toolsHtml =
          m.tools && m.tools.length
            ? `<div class="tool-list">${m.tools
                .map(
                  (t) => `<div class="tool-card">
                    <div class="tool-card-head" onclick="this.nextElementSibling.classList.toggle('hidden')">
                      <span>✓ <span class="tool-badge">${escapeHtml(t.name)}</span> — ${escapeHtml(t.summary || "")}</span>
                      <span class="muted">${t.durationMs ?? 0}ms</span>
                    </div>
                    <div class="tool-card-detail hidden">${t.diff ? renderDiffHtml(t.diff) : escapeHtml(t.summary || "")}</div>
                  </div>`
                )
                .join("")}</div>`
            : "";
        const planBtn =
          !isUser && m.mode === "plan"
            ? `<div class="plan-exec-bar">
                <button class="mini-btn accent" data-exec-plan="${encodeURIComponent(m.content || "")}">▶ Execute This Plan in Build Mode</button>
              </div>`
            : "";

        return `<div class="msg-card ${isUser ? "user" : "assistant"}">
          <div class="msg-head">
            <span class="msg-role">${isUser ? "👤 You" : "⚡ Aircode"}</span>
            <span>${escapeHtml((m.mode || "").toUpperCase())} ${m.model ? "· " + escapeHtml(m.model) : ""}</span>
          </div>
          ${thinkHtml}
          ${toolsHtml}
          <div class="msg-body">${isUser ? `<p>${escapeHtml(m.content)}</p>` : renderMarkdown(m.content)}</div>
          ${planBtn}
        </div>`;
      })
      .join("");
    stream.scrollTop = stream.scrollHeight;
  }

  function renderAttachments() {
    const el = $("#contextChips");
    el.innerHTML = state.attachments
      .map(
        (a, i) => `<span class="ctx-pill">📎 ${escapeHtml(a.label)} <button data-rm-att="${i}">✕</button></span>`
      )
      .join("");
  }

  async function sendAgentPrompt(overridePrompt = null) {
    if (state.streaming) return;
    const inp = $("#chatInput");
    const rawPrompt = (overridePrompt !== null ? overridePrompt : inp.value).trim();
    if (!rawPrompt) return;

    // Handle quick local client-side slash shortcuts
    if (rawPrompt === "/undo") {
      inp.value = "";
      await restoreCheckpoint("latest");
      return;
    }
    if (rawPrompt === "/compact") {
      inp.value = "";
      if (state.activeSession) {
        const res = await api("/api/sessions/compact", {
          method: "POST",
          body: JSON.stringify({ id: state.activeSession.id })
        });
        if (res.ok) {
          state.activeSession = res.session;
          renderChatStream();
          toast(`Compacted ${res.compactedFrom} messages!`);
        }
      }
      return;
    }
    if (rawPrompt === "/clear") {
      inp.value = "";
      const res = await api("/api/sessions", { method: "POST", body: "{}" });
      if (res.ok) {
        await refreshState();
        await loadActiveSession(res.session.id);
      }
      return;
    }

    if (overridePrompt === null) inp.value = "";
    $("#composerPopover").classList.add("hidden");

    // Expand custom slash commands if matched
    let effectivePrompt = rawPrompt;
    for (const cc of state.config.customCommands || []) {
      if (rawPrompt === cc.slash) {
        effectivePrompt = cc.prompt;
        break;
      }
    }

    state.streaming = true;
    $("#btnSendPrompt").classList.add("hidden");
    $("#btnAbortAgent").classList.remove("hidden");
    $("#sbEngine").textContent = "Engine: Streaming Agent Loop...";

    // Append optimistic user card + live streaming assistant card
    const streamEl = $("#chatStream");
    if (!state.activeSession?.messages?.length) streamEl.innerHTML = "";

    const userCard = document.createElement("div");
    userCard.className = "msg-card user";
    userCard.innerHTML = `<div class="msg-head"><span class="msg-role">👤 You</span><span>${state.activeMode.toUpperCase()}</span></div>
      <div class="msg-body"><p>${escapeHtml(rawPrompt)}</p></div>`;
    streamEl.appendChild(userCard);

    const botCard = document.createElement("div");
    botCard.className = "msg-card assistant";
    botCard.innerHTML = `
      <div class="msg-head"><span class="msg-role">⚡ Aircode</span><span id="liveStatusTag">Thinking...</span></div>
      <div class="think-box hidden" id="liveThinkBox">
        <div class="think-summary" onclick="this.nextElementSibling.classList.toggle('hidden')">🧠 Streaming Reasoning...</div>
        <div class="think-content" id="liveThinkContent"></div>
      </div>
      <div class="tool-list" id="liveToolList"></div>
      <div class="msg-body" id="liveAssistantBody"></div>
    `;
    streamEl.appendChild(botCard);
    streamEl.scrollTop = streamEl.scrollHeight;

    let accText = "";
    let accThink = "";
    const attachmentsToSend = [...state.attachments];
    state.attachments = [];
    renderAttachments();

    try {
      const resp = await fetch("/api/agent/stream", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sessionId: state.activeSession?.id || "",
          prompt: effectivePrompt,
          mode: state.activeMode,
          provider: state.config.provider,
          model: state.config.model,
          attachments: attachmentsToSend
        })
      });

      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";

      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let sep;
        while ((sep = buf.indexOf("\n\n")) !== -1) {
          const rawEvent = buf.slice(0, sep);
          buf = buf.slice(sep + 2);
          let evName = "message";
          let evData = "";
          for (const ln of rawEvent.split("\n")) {
            if (ln.startsWith("event:")) evName = ln.slice(6).trim();
            else if (ln.startsWith("data:")) evData += ln.slice(5).trim();
          }
          if (!evData) continue;
          let payload;
          try { payload = JSON.parse(evData); } catch { continue; }

          if (evName === "status") {
            const st = botCard.querySelector("#liveStatusTag");
            if (st) st.textContent = `${payload.provider} · ${payload.model}`;
          } else if (evName === "reasoning_delta") {
            accThink += payload.text || "";
            const tb = botCard.querySelector("#liveThinkBox");
            const tc = botCard.querySelector("#liveThinkContent");
            tb.classList.remove("hidden");
            tc.textContent = accThink;
          } else if (evName === "text_delta") {
            accText += payload.text || "";
            botCard.querySelector("#liveAssistantBody").innerHTML = renderMarkdown(accText);
            streamEl.scrollTop = streamEl.scrollHeight;
          } else if (evName === "tool_start") {
            const tl = botCard.querySelector("#liveToolList");
            const tcEl = document.createElement("div");
            tcEl.className = "tool-card";
            tcEl.id = `tc_${payload.id}`;
            tcEl.innerHTML = `<div class="tool-card-head" onclick="this.nextElementSibling.classList.toggle('hidden')">
                <span>⏳ <span class="tool-badge">${escapeHtml(payload.name)}</span> — running...</span>
                <span class="muted">...</span>
              </div>
              <div class="tool-card-detail hidden">${escapeHtml(JSON.stringify(payload.args || {}, null, 2))}</div>`;
            tl.appendChild(tcEl);
            streamEl.scrollTop = streamEl.scrollHeight;
          } else if (evName === "tool_result") {
            const tcEl = botCard.querySelector(`#tc_${CSS.escape(payload.id)}`);
            if (tcEl) {
              tcEl.innerHTML = `<div class="tool-card-head" onclick="this.nextElementSibling.classList.toggle('hidden')">
                  <span>${payload.ok ? "✓" : "⚠️"} <span class="tool-badge">${escapeHtml(payload.name)}</span> — ${escapeHtml(payload.summary || "")}</span>
                  <span class="muted">${payload.durationMs ?? 0}ms</span>
                </div>
                <div class="tool-card-detail ${payload.diff ? "" : "hidden"}">
                  ${payload.diff ? renderDiffHtml(payload.diff) : escapeHtml(payload.output || "")}
                </div>`;
            }
          } else if (evName === "error") {
            accText += `\n\n> ⚠️ **Error**: ${payload.message}\n> ${payload.hint || ""}`;
            botCard.querySelector("#liveAssistantBody").innerHTML = renderMarkdown(accText);
          } else if (evName === "done") {
            if (payload.session) {
              state.activeSession = payload.session;
            }
            if ((payload.touchedFiles || []).length) {
              await refreshFiles();
              // Reload modified open tabs automatically
              for (const tf of payload.touchedFiles) {
                const openTab = state.openTabs.find((t) => t.id === `file:${tf}`);
                if (openTab) {
                  const fresh = await api(`/api/file?path=${encodeURIComponent(tf)}`);
                  if (fresh.ok) {
                    openTab.content = fresh.content;
                    openTab.savedContent = fresh.content;
                  }
                }
              }
              renderWorkbench();
            }
          }
        }
      }
    } catch (err) {
      toast("Agent stream interrupted: " + err.message);
    } finally {
      state.streaming = false;
      $("#btnSendPrompt").classList.remove("hidden");
      $("#btnAbortAgent").classList.add("hidden");
      $("#sbEngine").textContent = "Engine: Ready (<5ms loopback)";
      await refreshState();
      renderChatStream();
      updateTokenMeter();
    }
  }

  // ==================== MODEL HUB & LOCAL SERVER DETECTION ====================
  async function openSettingsModal(tab = "free") {
    $("#settingsModal").classList.remove("hidden");
    switchModalTab(tab);
    syncProviderForm();
    if (tab === "free") {
      await scanLocalServers();
    }
  }

  function switchModalTab(tab) {
    $$(".modal-tab").forEach((b) => b.classList.toggle("active", b.dataset.mtab === tab));
    $("#mtab-free").classList.toggle("hidden", tab !== "free");
    $("#mtab-cloud").classList.toggle("hidden", tab !== "cloud");
    $("#mtab-windows").classList.toggle("hidden", tab !== "windows");
  }

  async function scanLocalServers() {
    $("#ollamaStatusBadge").textContent = "Scanning...";
    $("#lmstudioStatusBadge").textContent = "Scanning...";
    const res = await api("/api/providers/detect");
    if (!res.ok) return;
    state.detectedLocal = res.detected || {};

    const ol = state.detectedLocal.ollama || {};
    $("#ollamaStatusBadge").textContent = ol.online ? `🟢 Online (${(ol.models || []).length} models)` : "⚪ Not Running";
    $("#ollamaDetectedModels").innerHTML = (ol.models || [])
      .map((m) => `<button class="mini-btn" data-pick-local="ollama" data-pick-model="${escapeHtml(m)}">${escapeHtml(m)}</button>`)
      .join("");

    const lm = state.detectedLocal.lmstudio || {};
    $("#lmstudioStatusBadge").textContent = lm.online ? `🟢 Online (${(lm.models || []).length} models)` : "⚪ Not Running";
    $("#lmstudioDetectedModels").innerHTML = (lm.models || [])
      .map((m) => `<button class="mini-btn" data-pick-local="lmstudio" data-pick-model="${escapeHtml(m)}">${escapeHtml(m)}</button>`)
      .join("");
  }

  function syncProviderForm() {
    const p = state.config.provider || "local-assist";
    $("#cfgProviderSelect").value = p;
    populateModelQuickPick(p);
    $("#cfgModelInput").value = state.config.model || PROVIDER_MODELS[p]?.[0] || "";
    $("#cfgApiKeyInput").value = state.config.apiKeys?.[p] || "";
    $("#cfgBaseUrlInput").value = state.config.baseUrl || "";
    $("#cfgMaxStepsInput").value = state.config.maxSteps || 12;
  }

  function populateModelQuickPick(provider) {
    const models = PROVIDER_MODELS[provider] || ["default"];
    const sel = $("#cfgModelQuickPick");
    sel.innerHTML =
      `<option value="">— Presets —</option>` +
      models.map((m) => `<option value="${escapeHtml(m)}">${escapeHtml(m)}</option>`).join("");
  }

  // ==================== COMMAND PALETTE & AUTOCOMPLETE ====================
  function openCommandPalette() {
    $("#paletteModal").classList.remove("hidden");
    const inp = $("#paletteInput");
    inp.value = "";
    renderPaletteItems("");
    inp.focus();
  }

  function renderPaletteItems(query) {
    const q = query.trim().toLowerCase();
    const actions = [
      { label: "> Run /init (Generate AGENTS.md)", run: () => sendAgentPrompt("/init") },
      { label: "> Run /review (Code & Security Audit)", run: () => sendAgentPrompt("/review") },
      { label: "> Undo Latest Checkpoint (/undo)", run: () => restoreCheckpoint("latest") },
      { label: "> Open Visual Diff Center", run: () => openDiffCenterTab() },
      { label: "> Open AI Model Hub & Free Local Setup", run: () => openSettingsModal("free") },
      { label: "> Toggle Layout: IDE Studio vs Agent Focus", run: () => toggleLayout() }
    ];

    const list = $("#paletteList");
    const items = [];
    for (const a of actions) {
      if (!q || a.label.toLowerCase().includes(q)) {
        items.push({ title: a.label, sub: "Action", onPick: a.run });
      }
    }
    for (const f of state.files) {
      if (!q || f.path.toLowerCase().includes(q)) {
        items.push({ title: `📄 ${f.path}`, sub: f.language, onPick: () => openFileInEditor(f.path) });
      }
      if (items.length >= 35) break;
    }

    state.paletteCurrent = items;
    list.innerHTML = items
      .map(
        (it, idx) => `<div class="pop-item" data-pal-idx="${idx}">
          <span>${escapeHtml(it.title)}</span>
          <span class="muted">${escapeHtml(it.sub)}</span>
        </div>`
      )
      .join("");
  }

  function toggleLayout(target = null) {
    const shell = $("#app");
    const toAgent = target ? target === "agent" : shell.classList.contains("layout-ide");
    shell.classList.toggle("layout-agent", toAgent);
    shell.classList.toggle("layout-ide", !toAgent);
    $("#btnLayoutAgent").classList.toggle("active", toAgent);
    $("#btnLayoutIde").classList.toggle("active", !toAgent);
  }

  function handleComposerAutocomplete() {
    const inp = $("#chatInput");
    const val = inp.value;
    const pop = $("#composerPopover");

    if (val.startsWith("/") && !val.includes(" ")) {
      const q = val.toLowerCase();
      const cmds = [
        ...SLASH_COMMANDS,
        ...(state.config.customCommands || []).map((c) => ({ slash: c.slash, desc: c.label }))
      ].filter((c) => c.slash.toLowerCase().startsWith(q));
      if (cmds.length) {
        pop.classList.remove("hidden");
        pop.innerHTML = cmds
          .map(
            (c) => `<div class="pop-item" data-pick-slash="${escapeHtml(c.slash)}">
              <strong>${escapeHtml(c.slash)}</strong>
              <span class="muted">${escapeHtml(c.desc)}</span>
            </div>`
          )
          .join("");
        return;
      }
    }

    const atMatch = val.match(/@([a-zA-Z0-9_./\\-]*)$/);
    if (atMatch) {
      const q = atMatch[1].toLowerCase();
      const specials = [
        { key: "@git-diff", desc: "Attach working tree Git diff" },
        { key: "@terminal", desc: "Attach last terminal command output" },
        { key: "@problems", desc: "Attach workspace diagnostics" }
      ].filter((s) => s.key.includes(q));
      const fileMatches = state.files.filter((f) => f.path.toLowerCase().includes(q)).slice(0, 12);
      pop.classList.remove("hidden");
      pop.innerHTML =
        specials
          .map(
            (s) => `<div class="pop-item" data-pick-special="${escapeHtml(s.key)}">
              <strong>${escapeHtml(s.key)}</strong><span class="muted">${escapeHtml(s.desc)}</span>
            </div>`
          )
          .join("") +
        fileMatches
          .map(
            (f) => `<div class="pop-item" data-pick-atfile="${escapeHtml(f.path)}">
              <span>📄 @${escapeHtml(f.path)}</span><span class="muted">${escapeHtml(f.language)}</span>
            </div>`
          )
          .join("");
      return;
    }

    pop.classList.add("hidden");
  }

  async function attachFileContext(relPath) {
    const data = await api(`/api/file?path=${encodeURIComponent(relPath)}`);
    if (!data.ok) return;
    state.attachments.push({
      id: Date.now() + Math.random(),
      label: `@${relPath}`,
      content: data.content.slice(0, 12000)
    });
    renderAttachments();
    toast(`Attached @${relPath} to context`);
  }

  // ==================== EVENT BINDINGS ====================
  function bindEvents() {
    // Activity Rail
    $$(".rail-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        $$(".rail-btn").forEach((b) => b.classList.remove("active"));
        btn.classList.add("active");
        const p = btn.dataset.panel;
        $$(".side-panel").forEach((sp) => sp.classList.toggle("active", sp.id === `panel-${p}`));
      });
    });

    // Layout Toggle
    $("#btnLayoutIde").addEventListener("click", () => toggleLayout("ide"));
    $("#btnLayoutAgent").addEventListener("click", () => toggleLayout("agent"));

    // Open Folder (Native Win32 Picker on Windows, prompt fallback on web preview)
    $("#btnOpenFolder").addEventListener("click", async () => {
      if (state.platform === "win32") {
        const res = await api("/api/native/pick-folder", { method: "POST", body: "{}" });
        if (res.ok) {
          await refreshState();
          await refreshFiles();
          toast(`Opened folder: ${res.workspace}`);
        }
      } else {
        const entered = prompt("Enter workspace directory path:", state.workspace);
        if (entered) {
          const res = await api("/api/native/pick-folder", {
            method: "POST",
            body: JSON.stringify({ path: entered })
          });
          if (res.ok) {
            await refreshState();
            await refreshFiles();
          }
        }
      }
    });

    // Topbar controls
    $("#btnModelHub").addEventListener("click", () => openSettingsModal("free"));
    $("#btnWinSettings").addEventListener("click", () => openSettingsModal("windows"));
    $("#btnCloseModal").addEventListener("click", () => $("#settingsModal").classList.add("hidden"));
    $("#btnUndoCheckpoint").addEventListener("click", () => restoreCheckpoint("latest"));
    $("#btnCommandPalette").addEventListener("click", openCommandPalette);

    $("#themeSelect").addEventListener("change", async (e) => {
      const theme = e.target.value;
      applyTheme(theme);
      state.config.theme = theme;
      await api("/api/config", { method: "POST", body: JSON.stringify({ theme }) });
    });

    $("#permissionSelect").addEventListener("change", async (e) => {
      const permissionMode = e.target.value;
      state.config.permissionMode = permissionMode;
      await api("/api/config", { method: "POST", body: JSON.stringify({ permissionMode }) });
      toast(`Permission mode: ${permissionMode}`);
    });

    // Modal tabs & presets
    $$(".modal-tab").forEach((b) => b.addEventListener("click", () => switchModalTab(b.dataset.mtab)));
    $("#btnDetectLocal").addEventListener("click", scanLocalServers);

    $$("[data-select-preset]").forEach((b) => {
      b.addEventListener("click", async () => {
        const preset = b.dataset.selectPreset;
        if (preset === "ollama") {
          const first = state.detectedLocal?.ollama?.models?.[0] || "qwen2.5-coder:7b";
          await saveProviderSelection("ollama", first);
        } else if (preset === "lmstudio") {
          const first = state.detectedLocal?.lmstudio?.models?.[0] || "local-model";
          await saveProviderSelection("lmstudio", first);
        } else if (preset === "openrouter-free") {
          state.config.provider = "openrouter";
          state.config.model = "qwen/qwen3-coder:free";
          switchModalTab("cloud");
          syncProviderForm();
        } else if (preset === "gemini-free") {
          state.config.provider = "gemini";
          state.config.model = "gemini-2.5-flash";
          switchModalTab("cloud");
          syncProviderForm();
        } else if (preset === "groq-free") {
          state.config.provider = "groq";
          state.config.model = "llama-3.3-70b-versatile";
          switchModalTab("cloud");
          syncProviderForm();
        }
      });
    });

    $("#cfgProviderSelect").addEventListener("change", (e) => {
      const p = e.target.value;
      populateModelQuickPick(p);
      $("#cfgModelInput").value = PROVIDER_MODELS[p]?.[0] || "";
      $("#cfgApiKeyInput").value = state.config.apiKeys?.[p] || "";
    });
    $("#cfgModelQuickPick").addEventListener("change", (e) => {
      if (e.target.value) $("#cfgModelInput").value = e.target.value;
    });

    $("#btnSaveProviderConfig").addEventListener("click", async () => {
      const provider = $("#cfgProviderSelect").value;
      const model = $("#cfgModelInput").value.trim() || "default";
      const key = $("#cfgApiKeyInput").value.trim();
      const baseUrl = $("#cfgBaseUrlInput").value.trim();
      const maxSteps = parseInt($("#cfgMaxStepsInput").value || "12", 10);
      const patch = {
        provider,
        model,
        baseUrl,
        maxSteps,
        apiKeys: { [provider]: key }
      };
      const res = await api("/api/config", { method: "POST", body: JSON.stringify(patch) });
      if (res.ok) {
        state.config = res.config;
        updateModelPill();
        $("#settingsModal").classList.add("hidden");
        toast(`Activated ${PROVIDER_NAMES[provider] || provider} · ${model}`);
      }
    });

    // Windows Explorer Right-Click Integration
    $("#btnRegExplorerMenu").addEventListener("click", async () => {
      const r = await api("/api/native/windows-shell-menu", { method: "POST", body: JSON.stringify({ action: "register" }) });
      toast(r.message || (r.ok ? "Registered!" : r.error));
    });
    $("#btnUnregExplorerMenu").addEventListener("click", async () => {
      const r = await api("/api/native/windows-shell-menu", { method: "POST", body: JSON.stringify({ action: "unregister" }) });
      toast(r.message || "Removed.");
    });
    $("#btnOpenInWinExplorer").addEventListener("click", async () => {
      const r = await api("/api/native/windows-shell-menu", { method: "POST", body: JSON.stringify({ action: "open_explorer" }) });
      toast(r.message || "Opened workspace.");
    });

    // Explorer events
    $("#fileFilterInput").addEventListener("input", renderFileTree);
    $("#btnRefreshFiles").addEventListener("click", refreshFiles);
    $("#btnToggleRepoMap").addEventListener("click", () => $("#repoMapContainer").classList.toggle("hidden"));
    $("#btnNewFile").addEventListener("click", async () => {
      const name = prompt("New file relative path (e.g. src/helper.ts):");
      if (!name) return;
      await api("/api/file/ops", { method: "POST", body: JSON.stringify({ op: "create_file", path: name, content: "" }) });
      await refreshFiles();
      await openFileInEditor(name);
    });
    $("#btnNewFolder").addEventListener("click", async () => {
      const name = prompt("New folder relative path:");
      if (!name) return;
      await api("/api/file/ops", { method: "POST", body: JSON.stringify({ op: "create_dir", path: name }) });
      await refreshFiles();
    });

    // Grep search
    const triggerGrep = async () => {
      const query = $("#grepQueryInput").value.trim();
      if (!query) return;
      const res = await api("/api/search", {
        method: "POST",
        body: JSON.stringify({
          query,
          filePattern: $("#grepPatternInput").value.trim(),
          caseSensitive: $("#grepCaseChk").checked
        })
      });
      const box = $("#grepResults");
      const matches = res.matches || [];
      if (!matches.length) {
        box.innerHTML = '<div class="muted" style="padding:8px;">No matches found.</div>';
        return;
      }
      box.innerHTML = matches
        .map(
          (m) => `<div class="grep-hit" data-open-file="${escapeHtml(m.file)}" data-open-line="${m.line}">
            <div class="grep-file">${escapeHtml(m.file)}:${m.line}</div>
            <div class="grep-line">${escapeHtml(m.preview)}</div>
          </div>`
        )
        .join("");
    };
    $("#btnRunGrep").addEventListener("click", triggerGrep);
    $("#grepQueryInput").addEventListener("keydown", (e) => {
      if (e.key === "Enter") triggerGrep();
    });

    // Git & Checkpoints
    $("#btnRefreshGit").addEventListener("click", refreshState);
    $("#btnStageAll").addEventListener("click", async () => {
      await api("/api/git/action", { method: "POST", body: JSON.stringify({ action: "stage_all" }) });
      await refreshState();
    });
    $("#btnAiCommitMsg").addEventListener("click", async () => {
      const r = await api("/api/git/action", { method: "POST", body: JSON.stringify({ action: "generate_commit_msg" }) });
      if (r.ok) $("#commitMsgInput").value = r.message;
    });
    $("#btnGitCommit").addEventListener("click", async () => {
      const message = $("#commitMsgInput").value.trim();
      if (!message) return toast("Enter or generate a commit message first");
      const r = await api("/api/git/action", { method: "POST", body: JSON.stringify({ action: "commit", message }) });
      toast(r.ok ? "Committed changes!" : r.stderr || "Commit failed");
      if (r.ok) $("#commitMsgInput").value = "";
      await refreshState();
    });

    // Rules & MCP
    $("#rulesFileSelect").addEventListener("change", syncSelectedRulesFile);
    $("#btnSaveRules").addEventListener("click", async () => {
      const path = $("#rulesFileSelect").value;
      const content = $("#rulesEditorArea").value;
      const r = await api("/api/rules", { method: "POST", body: JSON.stringify({ path, content }) });
      if (r.ok) {
        toast(`Saved ${path}`);
        await refreshRules();
        await refreshFiles();
      }
    });
    $("#btnAutoInitAgents").addEventListener("click", async () => {
      const r = await api("/api/rules", { method: "POST", body: JSON.stringify({ path: "AGENTS.md", generateDefault: true }) });
      if (r.ok) {
        $("#rulesFileSelect").value = "AGENTS.md";
        await refreshRules();
        await refreshFiles();
        toast("Generated AGENTS.md from workspace structure!");
      }
    });
    $("#btnAddMcpServer").addEventListener("click", async () => {
      const name = prompt("MCP Server Name (e.g. Postgres MCP):");
      if (!name) return;
      const command = prompt("Stdio Command (e.g. npx -y @modelcontextprotocol/server-postgres):");
      if (!command) return;
      state.config.mcpServers.push({ id: "mcp_" + Date.now(), name, command, enabled: true });
      await api("/api/config", { method: "POST", body: JSON.stringify({ mcpServers: state.config.mcpServers }) });
      renderMcpList();
    });

    // Editor events
    const ta = $("#codeTextarea");
    ta.addEventListener("input", () => {
      const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
      if (!tab) return;
      tab.content = ta.value;
      updateEditorDecorations(tab);
      renderWorkbench();
    });
    ta.addEventListener("scroll", syncEditorScroll);
    ta.addEventListener("keyup", updateCursorStatus);
    ta.addEventListener("click", updateCursorStatus);
    ta.addEventListener("keydown", (e) => {
      if (e.key === "Tab") {
        e.preventDefault();
        const s = ta.selectionStart;
        const end = ta.selectionEnd;
        ta.value = ta.value.slice(0, s) + "  " + ta.value.slice(end);
        ta.selectionStart = ta.selectionEnd = s + 2;
        ta.dispatchEvent(new Event("input"));
      }
    });

    $("#btnSaveFile").addEventListener("click", saveActiveFile);
    $("#btnOpenDiffCenter").addEventListener("click", openDiffCenterTab);
    $("#btnLoadGitDiff").addEventListener("click", refreshDiffCenter);
    $("#btnRestoreLatestFromDiff").addEventListener("click", () => restoreCheckpoint("latest"));
    $("#btnAttachActiveFile").addEventListener("click", () => {
      const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
      if (!tab) return toast("Open a file in the editor first");
      attachFileContext(tab.path);
    });

    // Ctrl+K Inline AI Edit
    $("#btnInlineEdit").addEventListener("click", toggleInlineEditBar);
    $("#btnCancelInlineEdit").addEventListener("click", () => $("#inlineEditBar").classList.add("hidden"));
    $("#btnRunInlineEdit").addEventListener("click", executeInlineTransform);
    $("#inlineInstructionInput").addEventListener("keydown", (e) => {
      if (e.key === "Enter") executeInlineTransform();
    });
    $("#btnAcceptInlineEdit").addEventListener("click", () => {
      const p = state.pendingInlineTransform;
      if (!p) return;
      const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
      if (tab) {
        tab.content = tab.content.slice(0, p.start) + p.transformed + tab.content.slice(p.end);
        $("#codeTextarea").value = tab.content;
        updateEditorDecorations(tab);
        renderWorkbench();
      }
      state.pendingInlineTransform = null;
      $("#inlineDiffPreview").classList.add("hidden");
      $("#inlineEditBar").classList.add("hidden");
      toast("Applied inline transformation (Ctrl+S to save)");
    });
    $("#btnRejectInlineEdit").addEventListener("click", () => {
      state.pendingInlineTransform = null;
      $("#inlineDiffPreview").classList.add("hidden");
    });

    // Terminal & Problems Drawer
    $$(".drawer-tab").forEach((b) => {
      b.addEventListener("click", () => {
        $$(".drawer-tab").forEach((t) => t.classList.remove("active"));
        b.classList.add("active");
        $("#bottomDrawer").classList.remove("collapsed");
        const target = b.dataset.drawer;
        $("#drawerTerminal").classList.toggle("hidden", target !== "terminal");
        $("#drawerProblems").classList.toggle("hidden", target !== "problems");
      });
    });
    $("#btnToggleDrawer").addEventListener("click", () => $("#bottomDrawer").classList.toggle("collapsed"));
    $("#shellSelect").addEventListener("change", updateShellPromptLabel);
    $("#btnExecTerminal").addEventListener("click", runTerminalCommand);
    $("#btnClearTerminal").addEventListener("click", () => ($("#terminalOutput").innerHTML = ""));
    $("#terminalCmdInput").addEventListener("keydown", (e) => {
      if (e.key === "Enter") runTerminalCommand();
      else if (e.key === "ArrowUp" && state.termHistory.length) {
        state.termHistIdx = Math.max(0, state.termHistIdx - 1);
        e.target.value = state.termHistory[state.termHistIdx] || "";
      } else if (e.key === "ArrowDown" && state.termHistory.length) {
        state.termHistIdx = Math.min(state.termHistory.length, state.termHistIdx + 1);
        e.target.value = state.termHistory[state.termHistIdx] || "";
      }
    });
    $("#btnFixLastTermError").addEventListener("click", () => {
      if (!state.lastTerminalOutput) return toast("Run a command in the terminal first");
      setAgentMode("debug");
      state.attachments.push({ id: Date.now(), label: "@terminal-output", content: state.lastTerminalOutput });
      renderAttachments();
      sendAgentPrompt("Diagnose and fix the error shown in @terminal-output.");
    });
    $("#btnScanDiagnostics").addEventListener("click", refreshDiagnostics);
    $("#btnAiFixProblems").addEventListener("click", () => {
      if (!state.diagnostics.length) return toast("Zero diagnostics detected!");
      setAgentMode("debug");
      state.attachments.push({
        id: Date.now(),
        label: "@problems",
        content: JSON.stringify(state.diagnostics, null, 2)
      });
      renderAttachments();
      sendAgentPrompt("Fix the workspace diagnostics attached in @problems.");
    });

    // Sessions & Agent Composer
    $("#btnNewSession").addEventListener("click", async () => {
      const r = await api("/api/sessions", { method: "POST", body: "{}" });
      if (r.ok) {
        await refreshState();
        await loadActiveSession(r.session.id);
      }
    });
    $("#sessionFilterInput").addEventListener("input", renderSessionsList);
    $$(".mode-tab").forEach((b) => b.addEventListener("click", () => setAgentMode(b.dataset.mode)));
    $("#btnCompactSession").addEventListener("click", () => sendAgentPrompt("/compact"));
    $("#btnExportSession").addEventListener("click", exportSessionMarkdown);

    $("#chatInput").addEventListener("input", handleComposerAutocomplete);
    $("#chatInput").addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        sendAgentPrompt();
      }
    });
    $("#btnSendPrompt").addEventListener("click", () => sendAgentPrompt());
    $("#btnAbortAgent").addEventListener("click", () => api("/api/agent/abort", { method: "POST", body: "{}" }));
    $("#btnInsertAt").addEventListener("click", () => {
      const inp = $("#chatInput");
      inp.value += (inp.value && !inp.value.endsWith(" ") ? " " : "") + "@";
      inp.focus();
      handleComposerAutocomplete();
    });
    $("#btnInsertSlash").addEventListener("click", () => {
      const inp = $("#chatInput");
      inp.value = "/";
      inp.focus();
      handleComposerAutocomplete();
    });
    $("#btnAttachGitDiff").addEventListener("click", async () => {
      const gd = await api("/api/git/diff");
      state.attachments.push({ id: Date.now(), label: "@git-diff", content: gd.diff || "(Working tree clean)" });
      renderAttachments();
      toast("Attached @git-diff");
    });

    // Command Palette
    $("#paletteInput").addEventListener("input", (e) => renderPaletteItems(e.target.value));
    $("#paletteInput").addEventListener("keydown", (e) => {
      if (e.key === "Escape") $("#paletteModal").classList.add("hidden");
      else if (e.key === "Enter" && state.paletteCurrent?.length) {
        $("#paletteModal").classList.add("hidden");
        state.paletteCurrent[0].onPick();
      }
    });

    // Global Keyboard Shortcuts
    window.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "p") {
        e.preventDefault();
        openCommandPalette();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        saveActiveFile();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        toggleInlineEditBar();
      } else if (e.key === "Escape") {
        $("#paletteModal").classList.add("hidden");
        $("#settingsModal").classList.add("hidden");
        $("#composerPopover").classList.add("hidden");
      } else if (
        e.key === "Tab" &&
        !["TEXTAREA", "INPUT", "SELECT"].includes(document.activeElement?.tagName || "")
      ) {
        e.preventDefault();
        const next = MODES[(MODES.indexOf(state.activeMode) + 1) % MODES.length];
        setAgentMode(next);
        toast(`Switched Agent Mode -> ${next.toUpperCase()}`);
      }
    });

    // Delegated Clicks
    document.body.addEventListener("click", async (e) => {
      const attachBtn = e.target.closest("[data-attach-file]");
      if (attachBtn) {
        e.stopPropagation();
        await attachFileContext(attachBtn.dataset.attachFile);
        return;
      }
      const openFileEl = e.target.closest("[data-open-file]");
      if (openFileEl) {
        const line = openFileEl.dataset.openLine ? parseInt(openFileEl.dataset.openLine, 10) : null;
        await openFileInEditor(openFileEl.dataset.openFile, line);
        return;
      }
      const closeTabEl = e.target.closest("[data-close-tab]");
      if (closeTabEl) {
        e.stopPropagation();
        const id = closeTabEl.dataset.closeTab;
        state.openTabs = state.openTabs.filter((t) => t.id !== id);
        if (state.activeTabId === id) {
          state.activeTabId = state.openTabs[state.openTabs.length - 1]?.id || "welcome";
        }
        renderWorkbench();
        return;
      }
      const switchTabEl = e.target.closest("[data-switch-tab]");
      if (switchTabEl) {
        state.activeTabId = switchTabEl.dataset.switchTab;
        renderWorkbench();
        if (state.activeTabId === "diff-center") refreshDiffCenter();
        return;
      }
      const quickHero = e.target.closest("[data-quick]");
      if (quickHero) {
        const q = quickHero.dataset.quick;
        if (q === "open-folder") $("#btnOpenFolder").click();
        else if (q === "model-hub") openSettingsModal("free");
        else if (q === "run-init") sendAgentPrompt("/init");
        else if (q === "run-review") sendAgentPrompt("/review");
        return;
      }
      const gitAct = e.target.closest("[data-git-act]");
      if (gitAct) {
        await api("/api/git/action", {
          method: "POST",
          body: JSON.stringify({ action: gitAct.dataset.gitAct, path: gitAct.dataset.gitPath })
        });
        await refreshState();
        return;
      }
      const cpBtn = e.target.closest("[data-restore-cp]");
      if (cpBtn) {
        await restoreCheckpoint(cpBtn.dataset.restoreCp);
        return;
      }
      const forkBtn = e.target.closest("[data-fork-session]");
      if (forkBtn) {
        e.stopPropagation();
        const r = await api("/api/sessions/fork", { method: "POST", body: JSON.stringify({ id: forkBtn.dataset.forkSession }) });
        if (r.ok) {
          await refreshState();
          await loadActiveSession(r.session.id);
          toast("Forked session!");
        }
        return;
      }
      const delSes = e.target.closest("[data-del-session]");
      if (delSes) {
        e.stopPropagation();
        const r = await api("/api/sessions/delete", { method: "POST", body: JSON.stringify({ id: delSes.dataset.delSession }) });
        if (r.ok) {
          await refreshState();
          await loadActiveSession(r.activeId);
        }
        return;
      }
      const selSes = e.target.closest("[data-select-session]");
      if (selSes) {
        await loadActiveSession(selSes.dataset.selectSession);
        return;
      }
      const rmAtt = e.target.closest("[data-rm-att]");
      if (rmAtt) {
        state.attachments.splice(parseInt(rmAtt.dataset.rmAtt, 10), 1);
        renderAttachments();
        return;
      }
      const slashPick = e.target.closest("[data-pick-slash]");
      if (slashPick) {
        $("#chatInput").value = slashPick.dataset.pickSlash + " ";
        $("#composerPopover").classList.add("hidden");
        $("#chatInput").focus();
        return;
      }
      const atFilePick = e.target.closest("[data-pick-atfile]");
      if (atFilePick) {
        const p = atFilePick.dataset.pickAtfile;
        $("#chatInput").value = $("#chatInput").value.replace(/@[a-zA-Z0-9_./\\-]*$/, "");
        $("#composerPopover").classList.add("hidden");
        await attachFileContext(p);
        $("#chatInput").focus();
        return;
      }
      const spPick = e.target.closest("[data-pick-special]");
      if (spPick) {
        const k = spPick.dataset.pickSpecial;
        $("#chatInput").value = $("#chatInput").value.replace(/@[a-zA-Z0-9_./\\-]*$/, "");
        $("#composerPopover").classList.add("hidden");
        if (k === "@git-diff") $("#btnAttachGitDiff").click();
        else if (k === "@terminal") {
          state.attachments.push({ id: Date.now(), label: "@terminal", content: state.lastTerminalOutput || "(empty)" });
          renderAttachments();
        } else if (k === "@problems") {
          state.attachments.push({ id: Date.now(), label: "@problems", content: JSON.stringify(state.diagnostics, null, 2) });
          renderAttachments();
        }
        return;
      }
      const palItem = e.target.closest("[data-pal-idx]");
      if (palItem) {
        const idx = parseInt(palItem.dataset.palIdx, 10);
        $("#paletteModal").classList.add("hidden");
        state.paletteCurrent?.[idx]?.onPick();
        return;
      }
      const copyCode = e.target.closest(".copy-code-btn");
      if (copyCode) {
        navigator.clipboard?.writeText(decodeURIComponent(copyCode.dataset.code));
        toast("Copied code to clipboard!");
        return;
      }
      const applyCode = e.target.closest(".apply-code-btn");
      if (applyCode) {
        const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
        if (!tab) return toast("Open a target file in the editor first!");
        tab.content = decodeURIComponent(applyCode.dataset.code);
        $("#codeTextarea").value = tab.content;
        updateEditorDecorations(tab);
        renderWorkbench();
        toast(`Inserted into ${tab.name} (Ctrl+S to save)`);
        return;
      }
      const execPlan = e.target.closest("[data-exec-plan]");
      if (execPlan) {
        const planMd = decodeURIComponent(execPlan.dataset.execPlan);
        setAgentMode("build");
        sendAgentPrompt("Execute the following architectural plan step-by-step:\n\n" + planMd);
        return;
      }
      const pickLocal = e.target.closest("[data-pick-local]");
      if (pickLocal) {
        await saveProviderSelection(pickLocal.dataset.pickLocal, pickLocal.dataset.pickModel);
        return;
      }
      const testMcp = e.target.closest("[data-test-mcp]");
      if (testMcp) {
        const srv = state.config.mcpServers[parseInt(testMcp.dataset.testMcp, 10)];
        if (srv) {
          toast(`Probing ${srv.name}...`);
          const r = await api("/api/mcp/test", { method: "POST", body: JSON.stringify({ command: srv.command }) });
          toast(r.reachable ? `✓ ${srv.name} ready!` : `⚠️ ${srv.name} command not installed locally`);
        }
      }
    });
  }

  async function saveProviderSelection(provider, model) {
    const res = await api("/api/config", {
      method: "POST",
      body: JSON.stringify({ provider, model })
    });
    if (res.ok) {
      state.config = res.config;
      updateModelPill();
      $("#settingsModal").classList.add("hidden");
      toast(`Activated ${PROVIDER_NAMES[provider] || provider} · ${model}`);
    }
  }

  function updateCursorStatus() {
    const ta = $("#codeTextarea");
    const upto = ta.value.slice(0, ta.selectionStart);
    const lines = upto.split("\n");
    const ln = lines.length;
    const col = lines[lines.length - 1].length + 1;
    $("#editorCursorInfo").textContent = `Ln ${ln}, Col ${col} · UTF-8`;
  }

  function toggleInlineEditBar() {
    const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
    if (!tab) {
      toast("Open a code file first to use Ctrl+K Inline AI Transform");
      return;
    }
    const bar = $("#inlineEditBar");
    bar.classList.toggle("hidden");
    if (!bar.classList.contains("hidden")) {
      $("#inlineInstructionInput").focus();
    }
  }

  async function executeInlineTransform() {
    const tab = state.openTabs.find((t) => t.id === state.activeTabId && t.type === "file");
    if (!tab) return;
    const instruction = $("#inlineInstructionInput").value.trim();
    if (!instruction) return;
    const ta = $("#codeTextarea");
    let start = ta.selectionStart;
    let end = ta.selectionEnd;
    if (start === end) {
      start = 0;
      end = tab.content.length;
    }
    const selection = tab.content.slice(start, end);
    const res = await api("/api/inline-edit", {
      method: "POST",
      body: JSON.stringify({ path: tab.path, selection, instruction })
    });
    if (res.ok) {
      state.pendingInlineTransform = { start, end, transformed: res.transformed };
      $("#inlineDiffCode").innerHTML = renderDiffHtml(res.diff);
      $("#inlineDiffPreview").classList.remove("hidden");
    }
  }

  function exportSessionMarkdown() {
    const s = state.activeSession;
    if (!s) return;
    let md = `# ${s.title}\n\n`;
    for (const m of s.messages || []) {
      md += `## ${m.role.toUpperCase()} (${m.mode || "build"})\n\n${m.content}\n\n---\n\n`;
    }
    const blob = new Blob([md], { type: "text/markdown" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${(s.title || "aircode-session").replace(/[^a-z0-9-_]+/gi, "_")}.md`;
    a.click();
  }

  boot();
})();
