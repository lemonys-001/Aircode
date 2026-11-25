# Dependencies & Toolchain Reference

This document details both the **Windows 10/11 runtime dependencies** for running `dist\AI-Coding-Agent.exe` / `dist\AI-Coding-Agent-Setup.exe` and the **developer build dependencies** for compiling from source.

---

## 1. End-User Runtime Requirements (Windows 10 / 11 x64)

`dist\AI-Coding-Agent.exe` (`6.83 MB`) and `dist\AI-Coding-Agent-Setup.exe` (`7.36 MB`) are statically self-contained 64-bit Windows GUI executables (`PE32+ AMD64`, `SUBSYSTEM_WINDOWS_GUI`) that link only built-in Windows OS DLLs (`KERNEL32.dll`, `USER32.dll`, `SHELL32.dll`, `OLE32.dll`, `ADVAPI32.dll`, `GDI32.dll`, `DWMAPI.dll`, `WINHTTP.dll`, and Windows Universal CRT).

| Requirement | Status on Windows 10/11 | Purpose |
| :--- | :--- | :--- |
| **Windows 10 (1809+) or Windows 11 x64** | Required OS | Provides `CreatePseudoConsole` (`ConPTY`), `WinHTTP` TLS 1.2/1.3, and `DWM` dark titlebars |
| **Microsoft Edge WebView2 Runtime** | Pre-installed on Windows 11 & Windows 10 (21H2+) | Native desktop UI rendering host (`SetVirtualHostNameToFolderMapping` — zero web server) |
| **Git for Windows (`git.exe` in `PATH`)** | Optional (Recommended) | Enables the integrated Git Panel, branch switching, staging, commits, and `HEAD` diffs |
| **AI Provider (Cloud API Key or Local Ollama)** | User-configured in **Settings** | Powers autonomous coding runs (`OpenAI`, `Anthropic`, `Gemini`, `OpenRouter`, `Ollama`, or custom endpoint) |

---

## 2. Frontend & Desktop UI Dependencies (`package.json`)

All UI, code editor, icon font, and terminal assets are bundled offline into `ui-dist/` (`npm run build`) with zero CDN calls:

| Package | Version | Role |
| :--- | :--- | :--- |
| `@tauri-apps/api` | `^2.1.1` | Tauri 2 native command (`invoke`) and event (`listen`) IPC bindings |
| `@tauri-apps/cli` | `^2.1.0` | Tauri 2 desktop bundler & NSIS installer generator |
| `react` / `react-dom` | `^18.3.1` | Desktop workbench state management & component tree |
| `monaco-editor` | `^0.52.0` | Offline multi-tab code editor, web workers, Codicon font, and side-by-side `DiffEditor` |
| `@xterm/xterm` & `@xterm/addon-fit` | `^5.5.0` / `^0.10.0` | Hardware-accelerated ANSI terminal emulator attached to Windows ConPTY |
| `lucide-react` | `^0.460.0` | Crisp native desktop iconography |
| `typescript` / `vite` | `^5.6.3` / `^5.4.11` | Strict TypeScript typechecker and offline production bundler |

---

## 3. Tauri 2 Rust Backend Crates (`src-tauri/Cargo.toml`)

When building via `.\scripts\build-windows.ps1` (`npx tauri build --bundles nsis`) on Windows x64:

| Crate | Version / Features | Role |
| :--- | :--- | :--- |
| `tauri` | `2.1` | Native Windows WebView2 windowing, state management, and IPC command router |
| `tauri-plugin-dialog` & `rfd` | `2.0` / `0.15` | Native Windows `IFileOpenDialog` file & folder selection dialogs |
| `rusqlite` | `0.32` (`features = ["bundled"]`) | Embedded SQLite3 database (`%APPDATA%\com.aircode.aicodingagent\agent.db`) |
| `portable-pty` | `0.8` | Native Windows ConPTY (`CreatePseudoConsole`) multi-tab PowerShell & CMD sessions |
| `notify` | `6.1` | Real-time `ReadDirectoryChangesW` filesystem watcher (`fs://changed`) |
| `reqwest` | `0.12` (`json`, `rustls-tls`) | Async HTTPS client for OpenAI, Anthropic, Gemini, OpenRouter, Ollama & Custom APIs |
| `tokio` | `1.41` (`full`) | Async runtime for continuous multi-step agent execution & oneshot approval channels |
| `windows-sys` | `0.59` | Direct Win32 drive enumeration (`GetLogicalDrives`, `GetDriveTypeW`, `GetDiskFreeSpaceExW`) |

### Windows Build Toolchain Prerequisites (for rebuilding `src-tauri/` from source)
1. **Rust Toolchain (`x86_64-pc-windows-msvc`)**: Install via [rustup.rs](https://rustup.rs) (`rustup default stable-x86_64-pc-windows-msvc`).
2. **Visual Studio 2022 Build Tools**: Install the *"Desktop development with C++"* workload (MSVC `link.exe` + Windows 10/11 SDK).
3. **Node.js 20+ LTS**: Provides `npm` and `npx` for bundling `ui-dist/` and invoking `@tauri-apps/cli`.
4. **NSIS (Auto-managed by Tauri CLI)**: Tauri 2 automatically downloads and invokes NSIS when running `npx tauri build --bundles nsis`.

---

## 4. Native Win32 x64 Cross-Compilation Host (`src-tauri/win32-native-host/`)

To generate genuine, verified Windows x64 binaries (`dist/AI-Coding-Agent.exe` and `dist/AI-Coding-Agent-Setup.exe`) directly in environments where `crates.io` is firewalled, `src-tauri/win32-native-host/` provides an equivalent native C++17 Win32 WebView2 host implementing the exact same 30 IPC commands, event streams, SQLite schema, ConPTY engine, and 9-tool autonomous agent loop:

- **Microsoft WebView2 SDK (`vendor/WebView2.h` + `loader.hh`)**: Resolves `EmbeddedBrowserWebView.dll` directly from the Windows Edge WebView2 runtime and mounts the embedded offline UI bundle via `ICoreWebView2_3::SetVirtualHostNameToFolderMapping("tauri.localhost", ...)` with **zero HTTP server**.
- **SQLite 3 C Amalgamation (`vendor/sqlite3.h` + `sqlite3.c`)**: Statically compiled into `AI-Coding-Agent.exe` (`WAL` mode, `FTS5`, `JSON1`).
- **Windows ConPTY (`CreatePseudoConsole` / `ResizePseudoConsole` / `ClosePseudoConsole`)**: Spawns real `powershell.exe` and `cmd.exe` pseudo-consoles.
- **Windows Native TLS (`winhttp.dll`)**: Native OS TLS 1.2/1.3 HTTPS requests to all 6 AI providers without external OpenSSL DLLs.
