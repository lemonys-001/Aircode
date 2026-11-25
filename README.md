# AI Coding Agent — 100% Pure-Rust Native Desktop IDE & Studio

> **Zero WebView. Zero HTML/CSS/JavaScript. Zero Electron/Tauri Web Frontend.**
> A **100% Pure-Rust Native Desktop GUI Application** targeting **Windows 10 / 11 (x64)** — with a clean cross-platform OS abstraction (`src/platform/windows.rs` + `src/platform/linux.rs`) ready for **Linux**.

---

## Pre-Built Native Windows x64 Deliverables (`dist/`)

Ready-to-run 64-bit Windows (`PE32+ AMD64`, `WINDOWS_GUI`) executables are included directly in [`dist/`](./dist):

| Deliverable | Size | Description |
| --- | --- | --- |
| **`dist/AI-Coding-Agent.exe`** | **2.35 MB** | Self-contained, portable **100% Pure-Rust Native GUI Studio** (statically linked SQLite3, Win32 ClearType GDI UI, ConPTY terminal, WinHTTP TLS client, and 9-tool AI Agent). Run directly with zero installation or runtime dependencies. |
| **`dist/AI-Coding-Agent-Setup.exe`** | **2.78 MB** | **Pure-Rust Native Windows Setup Wizard & Uninstaller** — installs to `%LOCALAPPDATA%\Programs\AI-Coding-Agent` (no UAC elevation required), creates Desktop & Start Menu `.lnk` shortcuts via COM `IShellLinkW`, adds *"Open with AI Coding Agent"* to Windows Explorer folders, registers `Uninstall.exe` in Windows *Installed Apps*, and supports `/S` silent install & `--uninstall`. |
| **`dist/SHA256SUMS.txt`** | — | SHA-256 checksums for both Windows x64 binaries. |

---

## Pure-Rust Native Architecture (No Webpage Inside)

Every subsystem is implemented directly in Rust (`edition = "2021"`) with **zero external Cargo crate dependencies** and **zero web browser engine**:

| Module | Responsibility |
| --- | --- |
| `src/ui/studio.rs`, `src/ui/mod.rs` | **Native Double-Buffered Studio GUI Workbench** — resizable 4-zone dark IDE layout: Top Studio Bar, Left Explorer/Search/Git Dock, Center Multi-Tab Syntax-Highlighted Code Editor & Side-by-Side Diff Viewer, Bottom Multi-Tab ConPTY Terminal, Right Autonomous AI Agent Timeline & Composer, plus Settings/SQLite Audit & Permission Approval Modals. |
| `src/platform/windows.rs` | **Pure-Rust Win32 Native Backend** — 32-bit `CreateDIBSection` pixel backbuffer + ClearType `Segoe UI` / `Cascadia Mono` / `Consolas` GDI typography, `DwmSetWindowAttribute` dark titlebar, genuine Windows **ConPTY** (`CreatePseudoConsole` / `HPCON`), **WinHTTP** TLS 1.2/1.3 HTTPS client, `GetLogicalDriveStringsW` + `GetDiskFreeSpaceExW` drive selector, `SHBrowseForFolderW` native folder dialog, `ReadDirectoryChangesW` live filesystem watcher, and Unicode Win32 Clipboard. |
| `src/platform/linux.rs` | **Pure-Rust Linux Backend** — runtime-loaded X11 native window (`libX11.so.6` via `dlopen`), `SoftwareSurface` 32-bit renderer, POSIX shell session, `curl` HTTPS transport, `zenity` folder picker, and `xclip`/memory clipboard. |
| `src/agent.rs` | **Autonomous Multi-Provider AI Coding Engine** — supports **Ollama** (local offline + 1-click `/api/tags` model discovery), **OpenAI**, **Anthropic**, **Google Gemini**, **OpenRouter**, and **Custom OpenAI-compatible** endpoints; continuous multi-step tool loop (`Ask`, `Plan`, and `Autonomous` modes); thread-synchronized `PermissionGate`; and instant **Emergency Stop**. |
| `src/editor.rs` & `src/syntax.rs` | **Native Code Editor & Side-by-Side Diff Engine** — multi-tab UTF-8 code editor with 80-step undo/redo, click/drag selection, clipboard copy/cut/paste, auto-indent, inline Find (`Ctrl+F` / `F3`), lexical syntax highlighting (Rust, TypeScript/JS, Python, C/C++, Go, JSON, TOML/YAML, PowerShell/Shell, SQL, Markdown), and 50/50 split `HEAD` vs Working Copy diff viewer. |
| `src/vt_screen.rs` | **ANSI / VT100 Terminal Grid** — parses CSI cursor movement, erase modes, 16-color, 256-color, and 24-bit TrueColor (`38;2;R;G;B`) escape sequences with 1,500-line scrollback for `powershell.exe` and `cmd.exe` ConPTY sessions. |
| `src/sqlite.rs` | **Embedded SQLite3 Persistence** — safe Rust FFI over statically compiled `vendor/sqlite3/sqlite3.c` (`WAL` mode) storing provider keys, safety settings, recent workspaces, chat sessions, and full tool execution audit logs in `%LOCALAPPDATA%\AI-Coding-Agent\agent.db`. |
| `src/fs_ops.rs` & `src/git_ops.rs` | **Sandboxed Workspace & Silent Git Operations** — canonical path boundary guard preventing writes outside the active workspace, fast project text search, and `CREATE_NO_WINDOW` Git status/stage/unstage/branch/commit/diff execution. |
| `installer/src/main.rs` | **Pure-Rust Native Win32 Installer** — embeds `AI-Coding-Agent.exe` via `include_bytes!`, renders a dark-themed Win32 wizard, creates COM `.lnk` shortcuts, registers Explorer context menus & Windows *Installed Apps* uninstaller. |

---

## 9 Built-In Autonomous Coding Tools

1. **`read_file`** — Reads workspace source files with 1-based line numbers and line-range slicing.
2. **`write_file`** — Creates or overwrites files inside the canonical workspace sandbox (auto-creating parent directories and recording unified diffs).
3. **`edit_file`** — Performs exact surgical snippet replacements and records before/after diffs.
4. **`search_project`** — Case-insensitive or case-sensitive multi-file project text search (`file:line:col`).
5. **`execute_command`** — Runs PowerShell / CMD / shell commands inside the workspace (gated by `PermissionGate` when shell approval is enabled).
6. **`install_dependencies`** — Installs Cargo, npm/pnpm/yarn, or Python `pip` packages with user approval.
7. **`run_tests`** — Auto-detects `Cargo.toml`, `package.json`, `pyproject.toml`/`pytest`, or `.sln` and runs the project test suite.
8. **`diagnose_errors`** — Runs compiler/linter diagnostics (`cargo check`, `tsc --noEmit`, `py_compile`) so the agent can self-heal build errors automatically.
9. **`git_operation`** — Runs non-interactive `git status`, `diff`, `log`, `branch`, `add`, or `commit`.

---

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open Windows native folder picker (`SHBrowseForFolderW`) |
| `Ctrl+S` | Save active file tab (or save Settings modal to SQLite) |
| `Ctrl+N` | Open new Rust scratch buffer |
| `Ctrl+F` / `F3` | Toggle inline Find bar / Jump to next match |
| `Ctrl+Shift+F` | Open workspace-wide Search panel |
| `Ctrl+Z` / `Ctrl+Y` | Undo / Redo in Code Editor |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / Cut / Paste via native OS Unicode Clipboard |
| `F5` | Refresh workspace tree, Git status, and clean open tabs |
| `Enter` / `Shift+Enter` | Launch AI Agent run (in prompt box) / Insert newline |
| `Esc` | Dismiss active modal or inline bar |

---

## Building from Source

### Windows 10 / 11 (x64) — Native Build
Requires only the standard [Rust toolchain (`rustup`)](https://rustup.rs) (MSVC or GNU target — zero Node.js, zero npm, zero external Cargo crates):

```powershell
.\scripts\build-windows.ps1
# Or double-click scripts\build-windows.bat
```

### Linux — Native Build & Automated Test Suite
```bash
./scripts/build-linux.sh
```

### Cross-Compiling Windows x64 `.exe` Binaries from Linux
```bash
./scripts/cross-build-rust-win64.sh
```
