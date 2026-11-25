# AirCode — 100% Pure-Rust Native Desktop AI Coding Studio & Agentic Browser

> **Zero WebView. Zero HTML/CSS/JavaScript. Zero Electron/Tauri Web Frontend.**
> A **100% Pure-Rust Native Desktop GUI Application** targeting **Windows 10 / 11 (x64)** and
> **Linux Desktop Environments** (GNOME, KDE, XFCE, Cinnamon, MATE, LXQt, Sway/XWayland).
>
> **Default look: _Low Cortisol_** — a quiet daylight theme with warm paper surfaces, hairline
> borders and desaturated slate-teal accents, paired with roomier spacing. Switch any time with
> `/theme` or `Ctrl+K`.

AirCode is a native AI coding agent, IDE and *agentic browser* in one window: every pixel is
drawn by Rust (`CreateDIBSection` backbuffer on Windows, X11 `SoftwareSurface` on Linux), every
widget is hand-built, and the only embedded "web engine" is the **inbuilt agentic browser
viewport** the AI itself drives — never the IDE chrome.

---

## What's New in v2.1

| Area | v2.1 capability |
| --- | --- |
| **Themes** | 8 built-in themes — **Low Cortisol** (default, calm daylight), Tokyo Night, Catppuccin Mocha, One Dark Pro, Rosé Pine, Gruvbox Dark, Alabaster Light + a live custom `.aircode/theme.json`. |
| **Command Panel** | Fuzzy command palette (`Ctrl+K` / `Ctrl+P` / `/`) over every action, setting and slash command. |
| **Sessions** | Multi-session management, **fork whole session or fork-from-a-prompt**, visible per-turn timestamps, interactive **Prompt Timeline**. |
| **Thinking** | AI reasoning blocks rendered distinctly, with `/thinking on` / `/thinking off` to hide or show them. |
| **Compaction** | `/compact` (or the UI trigger) summarizes long histories while keeping the timeline and forks usable. |
| **Modes & Effort** | First-class **Plan** (read-only) vs **Build** (full execution) plus agent effort `low` / `medium` / `max` / `ultra` (thinking budgets 1K / 4K / 16K / 32K tokens). |
| **Provider Setup** | OpenCode-style guided `/connect` wizard: env auto-detect, clipboard paste, browser auth link — plus the **OpenCode AI server auth workaround** (Zen gateway, local `opencode/auth.json` import, `opencode serve` bridge). |
| **OpenCode Zen (free)** | **`/zen` — one click, no API key.** AirCode authenticates *exactly like the official OpenCode client* (UA fingerprint, `x-opencode-client`/`-project`/`-session`/`-session-id` quartet, `stream: true`, `bash`/`read` tool names — and never the `x-opencode-request` header the gateway 403s), so the anonymous `public` free lane works with zero setup. `/models` refreshes the live catalog from `opencode.ai/zen/v1/models`. |
| **Skills** | Load/manage skills from `.aircode/skills/`, including the built-in **AirCode Customizer** skill that teaches the AI to theme and re-configure AirCode itself. |
| **MCP** | Full MCP (`2024-11-05`) stdio + HTTP JSON-RPC client; server/tool discovery and AI tool-calling. |
| **Sub-Agents** | The AI can create, save and run its own sub-agents (readonly or full, any effort level). |
| **Editor** | Smart pairs, brace-aware auto-indent, block indent/outdent, comment toggle, word/next-match selection, matching-bracket highlight, goto-line, find/next, minimap-style scrollbar, side-by-side diff view. |
| **Browser** | Inbuilt agentic browser (`[eN]` element refs) the AI navigates/clicks/types in — and you can **take over or hand back at any time**. |
| **Linux** | Multi-arch install script for **amd64, arm64, arm32 (armhf), x86 (i686)** with `.desktop` entry, icon and uninstaller. |

---

## A Calmer, Lighter UI ("Low Cortisol")

v2.1's second pass was subtraction, not addition:

* **Header slimmed from 9 controls to 5** — provider and model share one pill; effort stays; the
  theme pill moved to the footer and the command palette (it is already shown in the status bar).
* **One hairline instead of double strokes**, tinted hover/active states instead of filled slabs,
  and a 3px accent rule for "selected" in the rail and command palette.
* **Roomier lists** — explorer rows 26→31px, search hits 40→46px, git files 26→30px, palette rows
  40→44px, chat lines 18→21px with a blank line above every turn, rail items 50→54px.
* **Barely-tinted user bubbles** (6% accent instead of 10%) and muted badges.
* **New default theme: Low Cortisol** — warm paper surfaces (`#FAF8F4` → `#FFFEFA`), hairline warm
  grey borders, warm near-black ink (never `#000`), and a desaturated slate-teal accent
  (`#5C7F78`) with soft success/warning/danger tones. No saturated red anywhere.

---

## Pre-Built Native Deliverables (`dist/`)

| Deliverable | Size | Description |
| --- | --- | --- |
| **`dist/AI-Coding-Agent.exe`** | **2.90 MB** | Self-contained portable **100% Pure-Rust Native GUI Studio** for **Windows x64** (`PE32+ AMD64`, `WINDOWS_GUI`): statically linked SQLite3, Win32 ClearType GDI UI, ConPTY terminal, WinHTTP TLS client, 18-tool AI agent. Zero installation, zero runtime dependencies. |
| **`dist/AI-Coding-Agent-Setup.exe`** | **3.34 MB** | **Pure-Rust Native Windows Setup Wizard & Uninstaller** — installs to `%LOCALAPPDATA%\Programs\AI-Coding-Agent` (no UAC prompt), creates Desktop & Start Menu `.lnk` shortcuts via COM `IShellLinkW`, adds *"Open with AI Coding Agent"* to Explorer folders, registers *Installed Apps* uninstaller, supports `/S` silent install and `--uninstall`. |
| **`dist/AI-Coding-Agent-linux-amd64`** | **2.73 MB** | Linux x86-64 native build (glibc). |
| **`dist/AI-Coding-Agent-linux-arm64`** | **2.63 MB** | Linux AArch64 build (static musl — runs on any aarch64 distro). |
| **`dist/AI-Coding-Agent-linux-arm32`** | **2.65 MB** | Linux ARMv7 hard-float build (static musl, `armhf`). |
| **`dist/AI-Coding-Agent-linux-x86`** | **2.66 MB** | Linux i686 build (static musl). |
| **`dist/SHA256SUMS.txt`** | — | SHA-256 checksums for every artifact above. |

Verify a download:

```bash
cd dist && sha256sum -c SHA256SUMS.txt
```

---

## Linux Installation (amd64 · arm64 · arm32 · x86)

```bash
# one-liner (installs the prebuilt binary for your architecture)
curl -fsSL https://raw.githubusercontent.com/lemonys-001/Aircode/main/scripts/install-linux.sh | bash

# or from a clone
./scripts/install-linux.sh

# force / override
./scripts/install-linux.sh --arch arm64            # cross-arch install
./scripts/install-linux.sh --prefix /opt           # custom prefix (default /usr/local)
./scripts/install-linux.sh --build-from-source     # skip release asset, cargo build
AIRCODE_PAYLOAD=./dist/AI-Coding-Agent-linux-arm64 ./scripts/install-linux.sh   # offline/local
./scripts/install-linux.sh --uninstall             # remove (keeps ~/.local/share/aircode)
```

What it does:

1. Detects the architecture (`x86_64`→`amd64`, `aarch64`→`arm64`, `armv7l/armhf`→`arm32`, `i686/i386`→`x86`).
2. Downloads `AI-Coding-Agent-linux-<arch>` from the matching GitHub release, **falling back to a
   source build** (pure Rust, no system libraries) if no prebuilt asset exists.
3. Installs the binary into `<prefix>/bin`, symlinks `aircode`, and writes a
   `~/.local/share/applications/aircode.desktop` entry (+ icon) with *New Session* and
   *Agentic Browser* quick actions, so AirCode appears in your app grid.
4. Prints run/update/remove instructions.

Runtime requirements on Linux: an X11 server (`libX11.so.6`, loaded at runtime — Wayland users
run under XWayland), plus `curl` for the HTTPS transport and `xclip`/`wl-copy`-free clipboard
fallback. No GTK, Qt, WebKit or Electron.

---

## Pure-Rust Native Architecture (No Webpage Inside)

Every subsystem is implemented directly in Rust (`edition = "2021"`) with **zero external Cargo
crate dependencies** and **zero web browser engine**:

| Module | Responsibility |
| --- | --- |
| `src/ui/studio.rs`, `src/ui/mod.rs`, `src/ui/commands.rs` | **Native Double-Buffered Studio GUI Workbench** — resizable 4-zone layout (Top Studio Bar, Left Dock with Explorer/Search/Git/Sessions/Extensions, Center multi-tab Editor + Diff + **Agentic Browser viewport**, Bottom multi-tab terminal, Right Agent timeline/composer), fuzzy **Command Panel**, Theme Picker, Connect Wizard, Safety, Sessions & Timeline modals. |
| `src/platform/windows.rs` | **Pure-Rust Win32 Native Backend** — 32-bit `CreateDIBSection` backbuffer + ClearType `Segoe UI`/`Cascadia Mono` GDI typography, `DwmSetWindowAttribute` dark titlebar, genuine **ConPTY**, **WinHTTP** TLS 1.2/1.3, drive enumeration, `SHBrowseForFolderW`, `ReadDirectoryChangesW`, Unicode clipboard. |
| `src/platform/linux.rs` | **Pure-Rust Linux Backend** — runtime-`dlopen`ed X11 native window, `SoftwareSurface` renderer, full `XLookupString` keyboard, POSIX shell session, `curl` transport, `zenity` folder picker. |
| `src/themes.rs` | **8 Theme Palettes** (Low Cortisol default) + custom `.aircode/theme.json` loading, per-token syntax colors. |
| `src/agent.rs` | **Autonomous Multi-Provider AI Engine** — Ollama, OpenAI, Anthropic, Gemini, OpenRouter, OpenCode Zen / local `opencode serve`, and custom OpenAI-compatible endpoints; continuous tool loop with `Plan`/`Build` modes and `low`/`medium`/`max`/`ultra` effort; `PermissionGate`; emergency stop; session compaction. |
| `src/browser.rs` | **Agentic Browser** — `[eN]` element refs, native-fast / headless DOM+JS backends, user takeover & hand-back, back/forward, external-browser hand-off. |
| `src/editor.rs` & `src/syntax.rs` | **Native Code Editor** — multi-tab UTF-8, 80-step undo/redo, selection & clipboard, smart pairs, brace-aware indent, block indent/outdent, comment toggle, matching brackets, find/next, goto-line, and a side-by-side diff builder. |
| `src/skills.rs` | **Skills** — embedded **AirCode Customizer** skill + workspace/app-data skills with enable/disable and prompt injection. |
| `src/mcp.rs` | **MCP Client** — `2024-11-05` protocol over stdio and HTTP, server config persistence, tool discovery and invocation. |
| `src/subagents.rs` | **Sub-Agents** — built-ins plus AI-authored sub-agents (slug, effort, readonly, system prompt). |
| `src/opencode_auth.rs` | **OpenCode-Style Auth** — provider guides, env/`auth.json` credential discovery, Zen gateway, local `opencode serve` bridge, and the full **Zen free-lane client emulation** (session-id generator, header quartet, free-model catalog). |
| `src/vt_screen.rs` | **ANSI/VT100 Grid** — CSI, 16/256/24-bit color, 1,500-line scrollback. |
| `src/sqlite.rs` | **Embedded SQLite3** — safe Rust FFI over `vendor/sqlite3/sqlite3.c` (WAL) storing keys, settings, sessions, prompts/turns, tool audit logs, with `ALTER TABLE` migrations. |
| `src/fs_ops.rs` & `src/git_ops.rs` | **Sandboxed Workspace & Silent Git** — canonical path boundary guard, project search, status/stage/branch/commit/diff. |
| `installer/src/main.rs` | **Pure-Rust Native Win32 Installer** — embeds the app via `include_bytes!`, dark Win32 wizard, COM `.lnk` shortcuts, Explorer context menu, *Installed Apps* registration. |

---

## Command Panel & Slash Commands

Open with **`Ctrl+K`**, **`Ctrl+P`**, or by typing **`/`** in the composer. The palette fuzzy-matches
sessions, modes, effort levels, thinking visibility, compaction, forking, themes, provider
connection, skills, MCP servers, sub-agents and the browser.

| Command | Effect |
| --- | --- |
| `/zen` `/free` | **OpenCode Zen free lane** — no API key, authenticates like the official OpenCode client |
| `/models` | Refresh the live anonymous Zen free-model catalog |
| `/connect` `/setup` `/auth` `/keys` | Guided provider wizard (env auto-detect → clipboard paste → browser link) |
| `/plan` · `/build` | Switch Plan (read-only) / Build (full execution) mode |
| `/effort low\|medium\|max\|ultra` | Agent effort (thinking budget 1K / 4K / 16K / 32K) |
| `/thinking` · `/thinking off` | Show / hide AI thinking blocks |
| `/compact` `/summarize` | Compact the session history (timeline & forks stay usable) |
| `/fork` | Fork the active session (whole session, or from a chosen prompt in the Timeline) |
| `/new` `/new-session` | Start a new session |
| `/timeline` · `/chat` | Prompt Timeline / chat view |
| `/theme <id>` | Switch theme (`tokyo-night`, `catppuccin-mocha`, `one-dark-pro`, `rose-pine`, `gruvbox-dark`, `alabaster-light`, `aircode-dark`, or a `.aircode/theme.json` id) |
| `/browser` · `/takeover` · `/resume` | Open the agentic browser / take control / hand back to the AI |
| `/skills` `/mcp` `/subagents` `/extensions` | Extensions panel |
| `/sessions` | Session manager |
| `/customize` | Load the AirCode Customizer skill prompt |
| `/settings` `/safety` | Safety & permission settings |
| `/goto <n>` · `/find <text>` | Go to line / find in file |

---

## Built-In Autonomous Tools

`read_file` · `write_file` · `edit_file` · `search_project` · `execute_command` ·
`install_dependencies` · `run_tests` · `diagnose_errors` · `git_operation` ·
`browser_navigate` · `browser_snapshot` · `browser_click` · `browser_type` ·
`create_skill` · `create_subagent` · `run_subagent` · `mcp_call`

Every mutating tool is gated by the `PermissionGate` (approve once / approve session / deny), and
**Emergency Stop** halts the loop instantly.

---

## Keyboard Shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+K` / `Ctrl+P` / `/` | Open fuzzy Command Panel (slash-command mode) |
| `Ctrl+O` | Open native folder picker (`SHBrowseForFolderW` / `zenity`) |
| `Ctrl+S` | Save active file tab (or Settings modal) |
| `Ctrl+N` | New Rust scratch buffer |
| `Ctrl+F` / `F3` | Inline find bar / next match |
| `Ctrl+Shift+F` | Workspace-wide search |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste via native Unicode clipboard |
| `Ctrl+G` | Go to line |
| `Tab` / `Shift+Tab` | Indent / outdent selection (smart pairs & comments aware) |
| `Ctrl+/` | Toggle comment on selection |
| `F5` | Refresh workspace tree, Git status, open tabs |
| `Enter` / `Shift+Enter` | Launch agent run / insert newline |
| `Esc` | Dismiss modal, palette or inline bar |

---

## Building from Source

### Windows 10 / 11 (x64)

```powershell
.\scripts\build-windows.ps1      # or double-click scripts\build-windows.bat
```

### Linux (native, any distro)

```bash
./scripts/build-linux.sh         # cargo test + cargo build --release
```

### Cross-compiling every Linux architecture from an x64 host

```bash
./scripts/cross-build-linux.sh   # zig + npm rust-std, produces all 4 dist/ binaries
```

### Cross-compiling Windows x64 `.exe` from Linux

```bash
./scripts/cross-build-rust-win64.sh
```

---

## Testing

```bash
cargo test --offline     # pure-Rust unit + headless render tests
```

---

## License

MIT — see [`LICENSE`](./LICENSE).
