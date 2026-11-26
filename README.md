# AirCode — 100% Pure-Rust Native Desktop AI Coding Studio & Agentic Browser

> **Zero WebView. Zero HTML/CSS/JavaScript. Zero Electron/Tauri Web Frontend.**
> A **100% Pure-Rust Native Desktop GUI Application** targeting **Windows 10 / 11 (x64)** and
> **Linux Desktop Environments** (GNOME, KDE, XFCE, Cinnamon, MATE, LXQt, Sway/XWayland).
>
> **Default look: _Low Cortisol_** — a quiet daylight theme with warm paper surfaces, hairline
> borders and desaturated slate-teal accents, paired with roomier spacing. Switch any time with
> `/theme` or `Ctrl+K`.
>
> **v2.2 rebuilds the whole interface on the OpenCode layout** — a session tab strip, a single
> conversation column with collapsible `+ Thought` / `+ Tool` blocks and inline green/red diffs,
> a right context sidebar (tokens, cost, MCP, LSP, todos, modified files) and a status bar. The
> full IDE (explorer, editor, terminal, git, browser) is one click away behind `Workspace`.

AirCode is a native AI coding agent, IDE and *agentic browser* in one window: every pixel is
drawn by Rust (`CreateDIBSection` backbuffer on Windows, X11 `SoftwareSurface` on Linux), every
widget is hand-built, and the only embedded "web engine" is the **inbuilt agentic browser
viewport** the AI itself drives — never the IDE chrome.

---

## What's New in v2.2 — a from-scratch OpenCode-style shell

v2.2 replaces the previous "everything on screen at once" layout with the OpenCode design:
one conversation, one context rail, one status bar, everything else one keystroke away.

| Area | v2.2 capability |
| --- | --- |
| **Shell** | OpenCode-style front door: session tab strip + `+`, conversation column, right context sidebar, bottom status bar. The classic IDE lives behind the `Workspace` toggle in the tab strip — nothing was removed, it just stopped competing for attention. |
| **Transcript** | Per-turn headers (`Model (12:04)`), `>` user prompts, rendered markdown, collapsible `+ Thought: 16s` / `+ Read src/greet.py` rows, **inline green/red diffs**, and a `Done.` + `Build · Model · Provider · effort` footer per turn. |
| **Context sidebar** | Session identity, context meter (tokens / % / cost), **MCP servers** with status dots, LSP servers, todos parsed from the assistant's checklists, and modified files with `+adds -dels` — every section collapsible. |
| **Status bar** | `[ctrl+? help] Tokens: 16.4K (8%) Cost: $0.12` on the left; workspace path and a filled model box (`big-pickle | medium`) on the right. Click the model box to cycle models. |
| **Prompt composer (reworked)** | Live slash-command autocomplete with descriptions, `@file` autocomplete, Up/Down history recall over sent prompts, `shift+enter` for a newline, `ctrl+h` to toggle tool blocks, `ctrl+l` to clear, and a visible **Send** button. A leading `/` no longer hijacks the keyboard into the palette — that was why `/connect` could never be typed. |
| **`/connect` rebuilt** | A full screen instead of a buried modal, with the **OpenCode Zen free lane** as the first, largest card (`Start free lane` — no key, no account, no card), a bring-your-own-key card, and an "import detected local auth" card. Reachable from `/connect`, the palette, `ctrl+shift+P`, `--connect`, or the status bar. |
| **Windows icon** | `rc.exe`/`windres` are absent from cross toolchains, so the icon and version resource were silently dropped. `src/winres.rs` now **generates the COFF `.res` in pure Rust** (RT_ICON / RT_GROUP_ICON / VS_VERSION_INFO / RT_MANIFEST) and `build.rs` links it — every Windows binary ships the icon and `2.2.0.0` version info. |
| **Cross builds fixed** | `build.rs` now declares `rerun-if-env-changed=CC/AR`, so switching to a cross compiler no longer reuses a host-built `sqlite3.o` (which broke every musl target with `sqlite3.o is incompatible with <arch>`). `scripts/cross-build-linux.sh` exports `CC`/`AR` too. |
| **CLI** | `--new-session`, `--browser`, `--connect` — the Linux `.desktop` actions are real now. |

### What's New in v2.1

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

## The OpenCode Layout

```
+--------------------------------------------------------------+----------+
| [Fix the login bug] [Add tests] [+]                 Workspace|  (tabs)  |
+--------------------------------------------------------------+----------+
|                                                              | CONTEXT  |
|  big-pickle (12:04)                                          | 16.4K 8% |
|  > fix the failing login test                                | $0.12    |
|                                                              |          |
|  I found the bug.                                            | MCP      |
|  * wrong password hash                                       | * github |
|  * missing session cookie                                    | * fs     |
|                                                              |          |
|  + Thought: 16s                                              | LSP      |
|  + Edit src/auth.rs (42ms)                                   |          |
|  - old line                                                  | TODOS    |
|  + new line                                                  | [ ] test |
|                                                              |          |
|  Done.                                                       | MODIFIED |
|  Build - big-pickle - opencode - medium                      | auth.rs  |
|  ---------------------------------------------------------  | +3 -1    |
|  enter to send | shift+enter newline | ctrl+h tools          |          |
|  > _                                                  [Send] |          |
+--------------------------------------------------------------+----------+
| [ctrl+? help] Tokens: 16.4K (8%) Cost: $0.12  opencode v2.2 [big-pickle|medium]|
+--------------------------------------------------------------+----------+
```

The right sidebar appears once the window is wider than 900px; `Workspace` in the tab strip
swaps the conversation for the full IDE (explorer, editor, terminal drawer, git, browser) while
keeping the same tab strip and status bar.

### A Calmer, Lighter UI ("Low Cortisol")

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
2. Resolves `AI-Coding-Agent-linux-<arch>` in order: **GitHub release asset** → **`dist/` inside the
   repository's source archive** (dist/ is versioned, so the prebuilt binary is reachable even when no
   release assets have been published, and with no Rust toolchain on the host) → **source build**
   (pure Rust, no system libraries) as a last resort. Every downloaded binary is SHA-256 verified
   against the `SHA256SUMS.txt` that ships beside it.
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
| `src/ui/shell/` | **OpenCode-style shell** (`mod`, `chat`, `sidebar`, `statusbar`, `prompt`, `palette`, `connect`, `markdown`, `theme`) — session tab strip, conversation column with collapsible `+ Thought` / `+ Tool` blocks and inline diffs, right context sidebar, prompt composer with autocomplete + history, grouped command palette and the full-screen `/connect` screen. |
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
| `src/winres.rs` | **Pure-Rust Windows `.res` generator** — builds the COFF resource object (RT_ICON / RT_GROUP_ICON / VS_VERSION_INFO / RT_MANIFEST) so the icon and version info survive toolchains with no `rc.exe`. |
| `src/sqlite.rs` | **Embedded SQLite3** — safe Rust FFI over `vendor/sqlite3/sqlite3.c` (WAL) storing keys, settings, sessions, prompts/turns, tool audit logs, with `ALTER TABLE` migrations. |
| `src/fs_ops.rs` & `src/git_ops.rs` | **Sandboxed Workspace & Silent Git** — canonical path boundary guard, project search, status/stage/branch/commit/diff. |
| `installer/src/main.rs` | **Pure-Rust Native Win32 Installer** — embeds the app via `include_bytes!`, dark Win32 wizard, COM `.lnk` shortcuts, Explorer context menu, *Installed Apps* registration. |

---

## Command Panel & Slash Commands

Open with **`Ctrl+K`**, **`Ctrl+P`**, or **`Ctrl+?`** (help). The palette groups its entries the way
OpenCode does — **Session**, **Provider**, **Agent**, **View**, **System** — and fuzzy-matches
sessions, modes, effort levels, thinking visibility, compaction, forking, themes, provider
connection, skills, MCP servers, sub-agents and the browser.

The composer itself autocompletes: type `/co` for `/connect`, `@src/ma` for a workspace file,
`Up`/`Down` to recall a prompt you already sent, `Tab` to accept the highlighted suggestion,
`shift+enter` for a newline, `ctrl+h` to hide the tool blocks and `ctrl+l` to clear.

| Command | Effect |
| --- | --- |
| `/zen` `/free` | **OpenCode Zen free lane** — no API key, authenticates like the official OpenCode client |
| `/models` | Refresh the live anonymous Zen free-model catalog |
| `/connect` `/setup` `/auth` `/keys` `/login` | Full-screen provider setup (Zen free lane → BYO key → import detected auth) |
| `/workspace` | Swap the conversation for the full IDE |
| `/sidebar` | Show / hide the right context sidebar |
| `/tools` | Show / hide the `+ Thought` / `+ Tool` blocks |
| `/model` `/models` | Cycle the active model |
| `/browser` · `/editor` · `/terminal` | Jump to a specific workspace surface |
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

```bash
AI-Coding-Agent [path]         # open a workspace (file or folder)
AI-Coding-Agent --new-session  # fresh session instead of resuming
AI-Coding-Agent --browser      # open the agentic browser
AI-Coding-Agent --connect      # go straight to provider setup
AI-Coding-Agent --help
```

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
