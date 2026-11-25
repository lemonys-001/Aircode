# AI Coding Agent — Native Windows x64 Desktop Application

A standalone, native **Windows 10/11 (x64)** desktop autonomous AI coding application inspired by OpenCode, Claude Code, and Cursor — featuring an offline multi-tab **Monaco Editor** (with side-by-side Git `DiffEditor`), real **Windows ConPTY** PowerShell & CMD terminals, persistent **SQLite** session & audit storage, native Win32 filesystem/drive/dialog integration, and a continuous multi-provider **Autonomous Coding Agent**.

> **Zero Web Server / Zero Browser Dependency**: `AI-Coding-Agent.exe` launches directly as a native Win32 GUI desktop window (`PE32+ AMD64`, `SUBSYSTEM_WINDOWS_GUI`) and mounts its embedded offline UI assets directly inside WebView2 via `SetVirtualHostNameToFolderMapping` — no `localhost` web server, no browser tab, and no external DLLs.

---

## Prebuilt Windows x64 Executables (`dist/`)

Both compiled 64-bit Windows executables are included directly in `dist/` (checksums in [`dist/SHA256SUMS.txt`](dist/SHA256SUMS.txt)):

| Deliverable | Size | Format | Description |
| :--- | :--- | :--- | :--- |
| **`dist/AI-Coding-Agent.exe`** | `6.83 MB` | `PE32+ GUI (x86_64)` | Standalone single-file portable Windows x64 desktop application. Double-click to launch immediately, or pass a project path (`AI-Coding-Agent.exe "D:\Work\MyApp"`). |
| **`dist/AI-Coding-Agent-Setup.exe`** | `7.36 MB` | `PE32+ GUI (x86_64)` | Native Windows x64 GUI Setup Wizard & Uninstaller. Installs to `%LOCALAPPDATA%\Programs\AI-Coding-Agent`, creates Desktop & Start Menu `.lnk` shortcuts, registers *"Open with AI Coding Agent"* in Windows Explorer folder context menus, and registers an uninstaller in Windows Installed Apps (`--silent` / `/S` also supported). |

---

## Core Desktop Capabilities

### 1. Autonomous Multi-Provider AI Coding Agent (9 Real Tools)
Supports **OpenAI**, **Anthropic**, **Google Gemini**, **OpenRouter**, **Ollama** (local `http://127.0.0.1:11434` with one-click live model discovery), and **Custom OpenAI-Compatible** endpoints.

In **Autonomous Mode**, the agent executes a continuous multi-step engineering loop (`read -> plan -> edit -> diagnose/test -> recover -> verify`) using 9 native workspace tools:
- `read_file` — Line-numbered file inspection with `start_line` / `end_line` slicing
- `write_file` — Full file creation/overwrite with automatic unified diff capture
- `edit_file` — Surgical exact-context replacement with diff recording
- `search_project` — Fast workspace-wide symbol and text search
- `execute_command` — Silent (`CREATE_NO_WINDOW`) PowerShell execution capturing `stdout`, `stderr`, and exit codes
- `install_dependencies` — Automated package installation across `npm`, `pnpm`, `yarn`, `cargo`, and `pip`
- `run_tests` — Auto-detected (`Cargo.toml`, `package.json`, `pytest`) or custom test suite runner
- `diagnose_errors` — Compiler & typecheck verification (`tsc --noEmit`, `cargo check`) fed back into the loop for automatic self-healing
- `git_operation` — Safe Git `status`, `diff`, `log`, `branch`, `add`, and `commit` operations

### 2. Permission System, Modes & Emergency Stop
- **Ask Mode**: Prompts via a native approval modal before any file modification, shell command, or mutating Git action.
- **Plan Mode**: Enforces strict read-only codebase exploration (`read_file`, `search_project`, `diagnose_errors`) to produce architectural plans without touching files.
- **Autonomous Mode**: Runs multi-step tool chains automatically while respecting configurable **Require Shell Approval**, **Require File Deletion Confirmation**, and **Workspace Sandbox Restriction** (`restrictToWorkspace`).
- **Emergency Stop**: Top-toolbar & agent-panel red stop control immediately halts the active agent loop and cancels pending approvals.
- **SQLite Audit Trail**: Every tool invocation (arguments, duration, status, and unified diff) is permanently logged in `%LOCALAPPDATA%\AI-Coding-Agent\agent.db`.

### 3. Desktop Studio GUI
- **Project Explorer & Windows Drive Switcher**: Switch across `C:\`, `D:\`, and other logical Windows drives with live capacity indicators, open folders via the native Windows `IFileOpenDialog` picker, create/delete files, search across the project, or reveal files in `explorer.exe`. Real-time external disk changes automatically refresh the tree via `ReadDirectoryChangesW`.
- **Multi-Tab Offline Monaco Editor & Diff Viewer**: Syntax highlighting for 20+ languages, dirty-state indicators, `Ctrl+S` saving, customizable font size, and side-by-side **Monaco `DiffEditor`** comparing `HEAD` vs. working tree.
- **Integrated Multi-Tab Windows ConPTY Terminal**: Real Windows Pseudo Console (`CreatePseudoConsole`) tabs running `powershell.exe` or `cmd.exe` with live resizing, custom environment variables, working directory sync, and `Ctrl+C` signal support.
- **Integrated Git Panel**: Stage/unstage files, inspect diffs in Monaco, commit changes, and create or switch branches.

---

## Repository Structure

```text
Aircode/
├── dist/
│   ├── AI-Coding-Agent.exe          # Standalone Windows x64 desktop executable
│   ├── AI-Coding-Agent-Setup.exe    # Windows x64 GUI installer & uninstaller
│   └── SHA256SUMS.txt               # SHA-256 integrity checksums
├── src/                             # React 18 + TypeScript + Offline Monaco + Xterm UI
│   ├── App.tsx                      # Root desktop workbench & IPC event wiring
│   ├── main.tsx                     # Offline Monaco worker bootstrap & custom dark theme
│   ├── lib/tauri.ts                 # Native Tauri 2 & WebView2 IPC client (zero HTTP)
│   ├── types/desktop.ts             # Shared TypeScript interfaces
│   ├── components/                  # Explorer, MonacoWorkbench, ConPtyTerminal, GitPanel, AgentPanel, Modals
│   └── styles/desktop.css           # Dark native desktop studio stylesheet
├── src-tauri/                       # Tauri 2 Rust Backend + Native Win32 x64 Host
│   ├── Cargo.toml                   # Tauri 2, rusqlite, portable-pty, notify, reqwest, rfd, windows-sys
│   ├── tauri.conf.json              # Tauri 2 desktop window & NSIS bundle configuration
│   ├── nsis/installer.nsi           # NSIS Windows installer script
│   ├── src/                         # Rust modules: main.rs, lib.rs, db.rs, fs_ops.rs, pty.rs, git_ops.rs, permissions.rs, agent.rs
│   └── win32-native-host/           # Native C++17 Win32 WebView2 + SQLite3 + ConPTY + WinHTTP x64 host & setup wizard
├── scripts/
│   ├── build-windows.ps1            # Native Windows Tauri 2 + Rust + NSIS build script
│   ├── build-windows.bat            # CMD wrapper for build-windows.ps1
│   ├── cross-build-win64.sh         # Zig x86_64-windows-gnu cross-compiler script
│   └── pack_ui_bundle.py            # Offline UI binary asset packer
└── DEPENDENCIES.md                  # Runtime & build toolchain documentation
```

---

## Building from Source

### Option A: Native Tauri 2 Rust Build on Windows 10/11 x64
In this cloud build environment, `crates.io` (`static.crates.io`) is network-restricted, so the Rust crate in `src-tauri/` is provided ready to compile on any Windows x64 machine with Rust + Visual Studio C++ Build Tools + Node.js 20+:

```powershell
# PowerShell (on Windows 10/11 x64)
.\scripts\build-windows.ps1
```
Or from `cmd.exe`:
```bat
scripts\build-windows.bat
```
This runs `npm install`, `npm run build` (`tsc --noEmit && vite build`), and `npx tauri build --bundles nsis`, copying the compiled Rust Tauri executable and NSIS installer into `dist\`.

### Option B: Cross-Compiling the Native Win32 x64 Executables (`dist/*.exe`)
The prebuilt executables in `dist/` were compiled directly from `ui-dist/` + `src-tauri/win32-native-host/` + SQLite3 amalgamation targeting `x86_64-windows-gnu`:

```bash
./scripts/cross-build-win64.sh
```
See [`DEPENDENCIES.md`](DEPENDENCIES.md) for full dependency and toolchain details.
