/* AirCode web front-end.
 *
 * The browser is a view over the same engine the desktop app and the TUI drive:
 * it polls /api/state (or subscribes to /api/events) and posts input back as
 * keys, characters or actions. All application logic lives server-side, so this
 * file only renders and forwards.
 */
(function () {
  "use strict";

  var state = null;
  var lastJson = "";
  var historyIdx = null;
  var acItems = [];
  var acSel = 0;
  var acOpen = false;
  var eventSource = null;

  var $ = function (id) { return document.getElementById(id); };

  // ---------------------------------------------------------------------
  // Transport
  // ---------------------------------------------------------------------
  function api(path, body) {
    var opts = { method: body === undefined ? "GET" : "POST" };
    if (body !== undefined) {
      opts.method = "POST";
      opts.headers = { "Content-Type": "application/json" };
      opts.body = JSON.stringify(body);
    }
    return fetch(path, opts).then(function (r) {
      return r.text().then(function (t) {
        try { return JSON.parse(t); } catch (e) { return { ok: r.ok, raw: t }; }
      });
    });
  }

  function subscribe() {
    if (typeof EventSource !== "function") { poll(); return; }
    try {
      eventSource = new EventSource("/api/events");
      eventSource.addEventListener("state", function (ev) {
        try { apply(JSON.parse(ev.data)); } catch (e) { /* ignore a bad frame */ }
      });
      eventSource.onerror = function () {
        // Fall back to polling; a dropped SSE stream must never freeze the UI.
        if (eventSource) { eventSource.close(); eventSource = null; }
        poll();
      };
    } catch (e) {
      poll();
    }
  }

  var pollTimer = null;
  function poll() {
    if (pollTimer) return;
    var tick = function () {
      api("/api/state").then(apply).finally(function () {
        pollTimer = setTimeout(tick, 700);
      });
    };
    tick();
  }

  // ---------------------------------------------------------------------
  // Rendering
  // ---------------------------------------------------------------------
  function hex(n) {
    var s = (n >>> 0).toString(16);
    while (s.length < 6) s = "0" + s;
    return "#" + s;
  }

  function applyTheme(theme) {
    if (!theme) return;
    var map = {
      "--bg-app": theme.bgApp, "--bg-header": theme.bgHeader,
      "--bg-sidebar": theme.bgSidebar, "--bg-input": theme.bgInput,
      "--bg-agent": theme.bgAgent, "--border": theme.border,
      "--text-primary": theme.textPrimary, "--text-secondary": theme.textSecondary,
      "--text-muted": theme.textMuted, "--accent": theme.accent,
      "--accent-secondary": theme.accentSecondary, "--success": theme.success,
      "--warning": theme.warning, "--danger": theme.danger, "--purple": theme.purple,
      "--diff-add-bg": theme.diffAddBg, "--diff-del-bg": theme.diffDelBg
    };
    var root = document.documentElement;
    for (var k in map) {
      if (map[k] !== undefined) root.style.setProperty(k, hex(map[k]));
    }
  }

  function esc(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function fmtTokens(n) {
    if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
    if (n >= 1e3) return (n / 1e3).toFixed(1) + "K";
    return String(n || 0);
  }

  function fmtCost(c) { return "$" + (c || 0).toFixed(c >= 1 ? 2 : 3); }

  function renderMarkdown(text) {
    var out = [];
    var inCode = false;
    text.split("\n").forEach(function (raw) {
      var t = raw.replace(/\r$/, "");
      var trimmed = t.replace(/^\s+/, "");
      if (/^```/.test(trimmed)) {
        inCode = !inCode;
        out.push(inCode ? '<div class="md-code">' : "</div>");
        return;
      }
      if (inCode) { out.push(esc(t) + "\n"); return; }
      if (/^#{1,6}\s/.test(trimmed)) {
        out.push('<div class="md-h">' + esc(trimmed.replace(/^#+\s*/, "")) + "</div>");
        return;
      }
      if (/^[-*]\s/.test(trimmed)) {
        out.push('<div class="md-li">' + esc(trimmed.slice(2)) + "</div>");
        return;
      }
      if (/^\+/.test(trimmed) && !/^\+\+/.test(trimmed)) {
        out.push('<div class="md-li md-add">' + esc(trimmed) + "</div>");
        return;
      }
      if (/^-/.test(trimmed) && !/^--/.test(trimmed)) {
        out.push('<div class="md-li md-del">' + esc(trimmed) + "</div>");
        return;
      }
      out.push('<div>' + esc(t) + "</div>");
    });
    if (inCode) out.push("</div>");
    return out.join("");
  }

  function toolBlock(t) {
    var label = { read_file: "Read", write_file: "Write", edit_file: "Edit",
      execute_command: "Bash", git_operation: "Git", list_files: "List",
      search: "Search" }[t.name] || t.name;
    var head = "+ " + esc(label);
    if (t.filePath) head += " " + esc(shortPath(t.filePath));
    if (t.durationMs) head += "  " + t.durationMs + "ms";

    var body = "";
    if (t.summary) body += '<div class="details-body">' + esc(t.summary) + "</div>";
    if (t.diff) {
      t.diff.split("\n").forEach(function (l) {
        var cls = l.charAt(0) === "+" ? "add" : (l.charAt(0) === "-" ? "del" : "");
        body += '<span class="diff-line ' + cls + '">' + esc(l) + "</span>";
      });
    }
    return '<details class="tool"><summary>' + head + "</summary>" + body + "</details>";
  }

  function shortPath(p) {
    var parts = String(p).split(/[\\/]/);
    return parts.length <= 3 ? p : "…/" + parts.slice(-2).join("/");
  }

  function renderTranscript(s) {
    var host = $("transcript");
    var html = "";
    var msgs = s.messages.slice();

    if (!msgs.length && !s.streaming.text && !s.streaming.thinking) {
      host.innerHTML =
        '<div class="empty-note">No messages yet. Ask a question below, or ' +
        'press <b>/connect</b> to set up a provider.</div>';
      return;
    }

    msgs.forEach(function (m) { html += renderTurn(m, false); });
    if (s.streaming.text || s.streaming.thinking) {
      html += renderTurn({
        role: "assistant", content: s.streaming.text, thinking: s.streaming.thinking,
        tools: s.streaming.tools, model: s.model, provider: s.provider,
        mode: s.mode, createdAt: ""
      }, true);
    }
    host.innerHTML = html;

    // Follow the tail unless the reader scrolled up.
    var nearBottom = host.scrollHeight - host.scrollTop - host.clientHeight < 120;
    if (nearBottom) host.scrollTop = host.scrollHeight;
  }

  function renderTurn(m, streaming) {
    var who = m.role === "user" ? "you" : (m.model || "assistant");
    var html = '<div class="turn' + (streaming ? " streaming" : "") + '">';
    html += '<div class="turn-head"><span class="turn-who">' + esc(who) + "</span>";
    if (m.createdAt) html += '<span class="turn-when">' + esc(m.createdAt) + "</span>";
    html += "</div>";

    if (m.role === "user") {
      html += '<div class="msg-user">' + esc(m.content) + "</div>";
    } else {
      if (m.content) html += '<div class="msg-assistant">' + renderMarkdown(m.content) + "</div>";
      if (m.thinking) {
        html += '<details class="thought"><summary>Thought (' +
          m.thinking.length + " chars)</summary><pre>" + esc(m.thinking) + "</pre></details>";
      }
      (m.tools || []).forEach(function (t) { html += toolBlock(t); });
      html += '<div class="turn-foot">Build · ' + esc(m.model || "model") + " · " +
        esc(m.provider || "provider") + " · " + esc(m.mode || "chat") + "</div>";
    }
    return html + "</div>";
  }

  function renderChrome(s) {
    $("active-tab").textContent = s.activeTitle || "new session";
    var others = s.sessions.filter(function (x) { return x.id !== s.activeSessionId; })
      .slice(0, 4).map(function (x) { return x.title; }).join("  ");
    $("other-tabs").textContent = others;

    var rs = $("run-state");
    rs.textContent = s.agentRunning ? "running" : (s.compacting ? "compacting" : "idle");
    rs.className = "pill" + (s.agentRunning ? " running" : "");

    $("btn-view").textContent = s.view === "workspace" ? "Chat" : "Workspace";
    $("btn-stop").hidden = !s.agentRunning;
    $("caret").textContent = s.agentRunning ? "▌" : ">";

    $("ctx-tokens").textContent = fmtTokens(s.tokens);
    var pct = Math.round((s.contextPct || 0) * 100);
    $("ctx-pct").textContent = pct + "%";
    $("ctx-fill").style.width = Math.min(100, pct) + "%";
    $("ctx-cost").textContent = fmtCost(s.cost);

    $("status-line").textContent = s.statusLine || "ready";
    $("status-tokens").textContent = "Tokens: " + fmtTokens(s.tokens) + " (" + pct + "%)";
    $("status-cost").textContent = fmtCost(s.cost);
    $("model-box").textContent = s.model || "no model";

    // MCP
    var mcp = $("mcp-list");
    if (s.mcp && s.mcp.length) {
      mcp.innerHTML = s.mcp.map(function (m) {
        return '<li class="' + esc(m.status) + '">' + esc(m.name) + "</li>";
      }).join("");
    } else {
      mcp.innerHTML = '<li class="empty">none configured</li>';
    }

    // Todos
    $("todos-block").hidden = !(s.todos && s.todos.length);
    if (s.todos && s.todos.length) {
      $("todo-list").innerHTML = s.todos.map(function (t) {
        return "<li>" + esc(t) + "</li>";
      }).join("");
    }

    // Modified files
    $("modified-block").hidden = !(s.modified && s.modified.length);
    if (s.modified && s.modified.length) {
      $("modified-list").innerHTML = s.modified.map(function (f) {
        return "<li>" + esc(shortPath(f.path)) +
          ' <span class="adds">+' + f.add + '</span> ' +
          '<span class="dels">-' + f.del + "</span></li>";
      }).join("");
    }

    // Sessions
    $("session-list").innerHTML = s.sessions.map(function (x) {
      return '<li data-id="' + esc(x.id) + '" class="' +
        (x.id === s.activeSessionId ? "active" : "") + '">' + esc(x.title) + "</li>";
    }).join("");

    // Overlays
    $("connect").hidden = !s.connect.open;
    $("approval").hidden = !s.approval;
    if (s.approval) {
      $("approval-title").textContent = s.approval.title || "Permission required";
      $("approval-detail").textContent = s.approval.detail || "";
      $("approval-target").textContent = s.approval.target || "";
    }

    if (s.toast) {
      $("toast").textContent = s.toast;
      $("toast").hidden = false;
      clearTimeout(renderChrome._toast);
      renderChrome._toast = setTimeout(function () { $("toast").hidden = true; }, 2600);
    } else {
      $("toast").hidden = true;
    }

    // Connect screen fields (only when the user is not mid-edit).
    if (s.connect.open && document.activeElement !== $("in-key")) {
      $("in-provider").value = s.connect.provider || "";
      $("in-model").value = s.connect.model || "";
      $("in-url").value = s.connect.url || "";
      $("zen-status").textContent = s.connect.zenStatus || "not started";
      var zm = $("zen-models");
      var want = (s.connect.zenModels || []).join(",");
      if (zm.dataset.sig !== want) {
        zm.dataset.sig = want;
        zm.innerHTML = (s.connect.zenModels || []).map(function (m) {
          return '<span class="chip">' + esc(m) + "</span>";
        }).join("");
      }
      var det = $("detected-list");
      var dw = (s.connect.detected || []).join("|");
      if (det.dataset.sig !== dw) {
        det.dataset.sig = dw;
        det.innerHTML = (s.connect.detected || []).map(function (d) {
          return '<li class="ok">' + esc(d) + "</li>";
        }).join("") || '<li class="no">nothing detected yet</li>';
      }
      $("detected-summary").textContent = s.connect.detectedFile
        ? "Found " + s.connect.detectedFile
        : "Scanning for local credentials…";
    }
  }

  function apply(next) {
    if (!next) return;
    state = next;
    var json = JSON.stringify(next);
    if (json === lastJson) return; // nothing changed; skip the DOM work
    lastJson = json;
    applyTheme(next.theme);
    renderTranscript(next);
    renderChrome(next);
    renderAutocomplete();
  }

  // ---------------------------------------------------------------------
  // Composer
  // ---------------------------------------------------------------------
  var COMMANDS = [
    ["/connect", "provider setup"], ["/new", "fresh session"],
    ["/sessions", "browse sessions"], ["/model", "cycle the active model"],
    ["/models", "refresh the free-model catalog"], ["/theme", "switch theme"],
    ["/compact", "compact the context"], ["/fork", "fork this session"],
    ["/workspace", "toggle the IDE"], ["/sidebar", "toggle the context rail"],
    ["/tools", "toggle tool blocks"], ["/zen", "start the Zen free lane"],
    ["/help", "every command"]
  ];

  function currentWord() {
    var el = $("prompt");
    var upto = el.value.slice(0, el.selectionStart);
    var m = /([^\s]*)$/.exec(upto);
    return m ? m[1] : "";
  }

  function renderAutocomplete() {
    var box = $("autocomplete");
    var word = currentWord();
    if (!state || !word || word.charAt(0) !== "/") {
      box.hidden = true; acOpen = false; acItems = [];
      return;
    }
    acItems = COMMANDS.filter(function (c) { return c[0].indexOf(word) === 0; });
    if (!acItems.length) { box.hidden = true; acOpen = false; return; }
    if (acSel >= acItems.length) acSel = 0;
    box.innerHTML = acItems.map(function (c, i) {
      return '<div class="ac-item' + (i === acSel ? " sel" : "") +
        '" data-i="' + i + '"><span class="ac-cmd">' + esc(c[0]) +
        '</span><span class="ac-desc">' + esc(c[1]) + "</span></div>";
    }).join("");
    box.hidden = false;
    acOpen = true;
  }

  function acceptCompletion() {
    if (!acOpen || !acItems.length) return;
    var cmd = acItems[acSel][0];
    var el = $("prompt");
    var upto = el.value.slice(0, el.selectionStart);
    var cut = upto.search(/[^\s]*$/);
    el.value = upto.slice(0, cut) + cmd + " " + el.value.slice(el.selectionStart);
    el.focus();
    renderAutocomplete();
  }

  function send() {
    var el = $("prompt");
    var text = el.value.trim();
    if (!text) return;
    if (state) {
      if (!state.history.length || state.history[state.history.length - 1] !== text) {
        // history is server-side; this only keeps the local index sane
      }
    }
    api("/api/send", { prompt: text }).then(function () {
      el.value = "";
      historyIdx = null;
      renderAutocomplete();
      el.focus();
    });
  }

  function recall(dir) {
    if (!state || !state.history.length) return;
    var el = $("prompt");
    if (historyIdx === null) {
      historyIdx = state.history.length;
    }
    historyIdx += dir;
    if (historyIdx < 0) historyIdx = 0;
    if (historyIdx > state.history.length) historyIdx = state.history.length;
    if (historyIdx === state.history.length) {
      el.value = "";
    } else {
      el.value = state.history[historyIdx];
    }
  }

  function wire() {
    var el = $("prompt");

    el.addEventListener("input", function () {
      acSel = 0;
      renderAutocomplete();
      autoGrow();
    });

    el.addEventListener("keydown", function (ev) {
      if (ev.key === "Enter" && !ev.shiftKey) {
        ev.preventDefault();
        if (acOpen && acItems.length && currentWord().charAt(0) === "/") {
          acceptCompletion();
          return;
        }
        send();
        return;
      }
      if (ev.key === "Tab" && acOpen && acItems.length) {
        ev.preventDefault();
        acceptCompletion();
        return;
      }
      if (ev.key === "ArrowUp") {
        if (acOpen) { ev.preventDefault(); acSel = Math.max(0, acSel - 1); renderAutocomplete(); return; }
        if (!el.value) { ev.preventDefault(); recall(-1); }
        return;
      }
      if (ev.key === "ArrowDown") {
        if (acOpen) { ev.preventDefault(); acSel = Math.min(acItems.length - 1, acSel + 1); renderAutocomplete(); return; }
        if (!el.value) { ev.preventDefault(); recall(1); }
        return;
      }
      if (ev.key === "Escape") {
        if (acOpen) { box_hide(); return; }
        if (!$("connect").hidden) { api("/api/connect/close", {}); return; }
        if (!$("approval").hidden) { api("/api/permission", { allow: false }); return; }
      }
    });

    $("autocomplete").addEventListener("mousedown", function (ev) {
      var item = ev.target.closest(".ac-item");
      if (!item) return;
      ev.preventDefault();
      acSel = parseInt(item.dataset.i, 10) || 0;
      acceptCompletion();
    });

    $("btn-send").addEventListener("click", send);
    $("btn-stop").addEventListener("click", function () { api("/api/stop", {}); });
    $("btn-new").addEventListener("click", function () { api("/api/session/new", {}); });
    $("btn-view").addEventListener("click", function () { api("/api/action", { action: "toggle-view" }); });
    $("btn-connect").addEventListener("click", function () { api("/api/action", { action: "connect" }); });
    $("model-box").addEventListener("click", function () { api("/api/action", { action: "cycle-model" }); });

    $("btn-zen").addEventListener("click", function () {
      api("/api/connect/zen", {}).then(function () { api("/api/state").then(apply); });
    });
    $("btn-save-connect").addEventListener("click", function () {
      api("/api/connect/save", {
        provider: $("in-provider").value,
        model: $("in-model").value,
        key: $("in-key").value,
        url: $("in-url").value
      }).then(function () { $("in-key").value = ""; });
    });
    $("btn-close-connect").addEventListener("click", function () { api("/api/connect/close", {}); });
    $("btn-allow").addEventListener("click", function () { api("/api/permission", { allow: true }); });
    $("btn-deny").addEventListener("click", function () { api("/api/permission", { allow: false }); });

    $("session-list").addEventListener("click", function (ev) {
      var li = ev.target.closest("li[data-id]");
      if (li) api("/api/session/switch", { id: li.dataset.id });
    });

    // Global shortcuts that work wherever focus is.
    document.addEventListener("keydown", function (ev) {
      if (ev.target === $("prompt")) return;
      if (ev.key === "Enter" && !ev.shiftKey && document.activeElement === document.body) {
        ev.preventDefault();
        $("prompt").focus();
      }
    });
  }

  function box_hide() { $("autocomplete").hidden = true; acOpen = false; }

  function autoGrow() {
    var el = $("prompt");
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, window.innerHeight * 0.4) + "px";
  }

  wire();
  api("/api/state").then(apply).then(subscribe);
})();
