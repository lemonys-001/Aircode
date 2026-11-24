# ⚡ Aircode Studio for Windows

> **Ultra-fast native Windows AI Coding IDE & Autonomous Engineering Agent (`1.7 MB` single-file `.exe`)** — combining the best of **OpenCode**, **Claude Code**, **Cursor**, **Windsurf**, **Cline / Roo Code**, **Zed**, and **Aider** with **zero setup** and **zero terminal/shell required**.

---

## 📦 Prebuilt Windows Executable Ready to Run

Your compiled standalone Windows 64-bit desktop binary is ready inside **`windows-exe/`**:

- **[`windows-exe/Aircode.exe`](windows-exe/Aircode.exe)** (`1.70 MB`, PE32+ x86_64 Native Windows GUI)
- **[`windows-exe/QUICKSTART-WINDOWS.txt`](windows-exe/QUICKSTART-WINDOWS.txt)**

### Setup on Windows 10 / Windows 11
1. Download **`windows-exe/Aircode.exe`**.
2. **Double-click `Aircode.exe`** — no installer, no Node.js, no Python, no WSL, no external `.dll` files, and no console black window.

---

## 🚀 Why Aircode Is Faster & Lighter Than Electron / CLI Tools

| Metric | Electron IDEs | Terminal CLI Agents | **Aircode Studio (`Aircode.exe`)** |
| :--- | :--- | :--- | :--- |
| **Binary Size** | 180 – 350 MB | Requires Node / Bun / Python | **1.70 MB single `.exe`** |
| **Cold Startup** | 1.8 – 4.5s | Terminal setup + PATH config | **< 40 ms native Win32** |
| **UI Experience** | Heavy Chromium bundle | Terminal / Shell text only | **Full Desktop IDE + Agent Focus UI (Edge WebView2)** |
| **TLS & Streaming** | Bundled OpenSSL / Node | Runtime-dependent | **Native Windows `WinHTTP` TLS 1.3 + SSE** |
| **Subprocess Execution** | Often flashes CMD boxes | Manual shell | **`CreateProcessW` + `CREATE_NO_WINDOW` (100% silent)** |
| **HiDPI & Dark Titlebar** | Custom hacks | Terminal-dependent | **Per-Monitor v2 Manifest + DWM Immersive Dark Mode** |

---

## ✨ Features Unified From Every Top Coding App

### 1. Autonomous Multi-Mode Agent (OpenCode + Claude Code + Roo Code)
- **5 Specialized Modes (`Tab` cycles modes instantly)**:
  - **🔨 Build Mode** — Full autonomous engineering loop (`list_files`, `read_file`, `grep_search`, `write_file`, CRLF-safe surgical `edit_file`, `run_command`, `git_status`, `git_diff`, `spawn_subagent`).
  - **📐 Plan Mode** — Read-only software architect that inspects your repo and generates a step-by-step plan with a 1-click **"▶ Execute This Plan in Build Mode"** button.
  - **⚡ Turbo Composer** — Rapid multi-file surgical edits.
  - **🐛 Debug & Fix** — Diagnoses compiler, linter, and terminal errors and verifies fixes.
  - **🛡️ Security & Code Review** — Audits diffs and workspace files for bugs and vulnerabilities.
- **Permission Guardrails**: Switch live between `🛡️ Auto-Edit`, `⚡ Autonomous (YOLO)`, and `🔒 Read-Only`.
- **Sub-Agent Delegation (`spawn_subagent`)**: Runs focused codebase exploration sub-tasks without bloating main context.
- **Live Thinking Stream**: Expandable real-time reasoning trace supporting Anthropic Extended Thinking, DeepSeek R1 `reasoning_content`, and Ollama/QwQ `<think>` blocks.

### 2. Zero-Cloud-Limit Local & Free Model Hub
Built specifically so you never get blocked by cloud compute quotas:
- **🦙 Ollama Auto-Discovery (`127.0.0.1:11434`)** & **🎛️ LM Studio (`127.0.0.1:1234`)** — 100% local, offline, unlimited tokens. Includes automatic XML `<tool_call>` fallback so even small local 3B/7B models can execute full coding tools!
- **🌐 OpenRouter `:free` Coding Presets** — `qwen/qwen3-coder:free`, `deepseek/deepseek-chat-v3-0324:free`, `deepseek/deepseek-r1:free`, `meta-llama/llama-3.3-70b-instruct:free`.
- **✨ Free Cloud Tiers** — Google Gemini (`gemini-2.5-flash`), Groq LPU (`llama-3.3-70b-versatile`), GitHub Models.
- **🚀 Frontier Providers** — Native Anthropic `/v1/messages` tool-use protocol (`claude-sonnet-4-5`, `claude-opus-4-5`), OpenAI (`gpt-4.1`, `o4-mini`), DeepSeek, or any custom OpenAI-compatible URL.
- **⚡ Built-In `Aircode Local Instant` Engine** — Works offline with zero API keys out of the box for `/init`, `/review`, workspace indexing, and Repo Map exploration.

### 3. Full Visual Desktop IDE (Cursor + Windsurf + Zed + Aider)
- **Dual Workspace Layouts**: Switch anytime between **IDE Studio** (Explorer + Multi-Tab Editor + Terminal + Agent) and **Agent Focus** (OpenCode Desktop layout).
- **Multi-Tab Code Editor + `Ctrl+K` Inline AI Transform**: Line gutter, live syntax highlighting, `Ctrl+S` save with automatic checkpointing, and inline `Ctrl+K` code transformations with live Accept/Discard diff preview.
- **Aider-Style Structural Repo Map**: Automatically indexes classes, structs, functions, and exports across TypeScript/JS, Python, C/C++, Rust, Go, C#, and Java.
- **Time-Travel Checkpoints (`/undo`) & Visual Diff Center (`± Diffs`)**: Every agent turn or file save snapshots original files so 1 click restores any previous state.
- **Git Source Control Panel**: Branch status, stage/unstage, **✨ 1-Click AI Commit Message**, commit, and recent commit log.
- **Rules, Memory Bank & MCP Hub**: Visual editor for `AGENTS.md` (`/init` auto-generator), `CLAUDE.md`, `.aircoderules`, `.aircode/MEMORY.md`, and **MCP (Model Context Protocol)** servers.
- **Integrated Multi-Shell Terminal + Diagnostics**: Run `PowerShell`, `CMD`, or `Bash` commands from the UI with **`🔧 AI Fix Error`** 1-click handoff.
- **Windows Explorer Integration**: Native `IFileOpenDialog` folder picker, drag-and-drop folder launch, 1-click `"Open in Aircode"` Explorer right-click menu (`HKCU`, no Admin UAC needed), and `.aircode-portable` USB mode.

---

## 🛠️ Building From Source

```bash
# Run cross-platform Web / Studio preview server (Node 22+)
npm run preview

# Cross-compile standalone native Windows 64-bit executable -> windows-exe/Aircode.exe
npm run build:windows
```
