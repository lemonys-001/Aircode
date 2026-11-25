# Dependencies & System Requirements

## Runtime Requirements (End Users on Windows 10 / 11 x64)

**Zero external runtimes required.**
`dist\AI-Coding-Agent.exe` (`2.35 MB`) and `dist\AI-Coding-Agent-Setup.exe` (`2.78 MB`) link **exclusively** against built-in Windows 10 / Windows 11 system DLLs:

- `KERNEL32.dll` — Win32 process, thread, filesystem, and **ConPTY** (`CreatePseudoConsole`, `ResizePseudoConsole`, `ClosePseudoConsole`) APIs
- `USER32.dll` — Native Win32 windowing, keyboard/mouse/wheel input, timers, and Unicode clipboard
- `GDI32.dll` — Double-buffered 32-bit `CreateDIBSection` raster surface + ClearType `CreateFontW` / `ExtTextOutW` text rendering
- `dwmapi.dll` — Windows 10/11 immersive dark-mode window frame (`DwmSetWindowAttribute`)
- `WINHTTP.dll` — Native OS TLS 1.2 / 1.3 HTTPS client for OpenAI, Anthropic, Gemini, OpenRouter, Ollama, and Custom LLM APIs
- `SHELL32.dll` & `ole32.dll` — Native `SHBrowseForFolderW` folder dialog, Explorer file reveal, and COM `IShellLinkW` / `IPersistFile` `.lnk` shortcut creation
- `ADVAPI32.dll` & `bcryptprimitives.dll` — Windows Registry (`HKCU` uninstaller + Explorer context menu) and OS entropy

> **No WebView2 Runtime, no Node.js, no .NET, no VC++ Redistributable installer, and no external `sqlite3.dll` are required.**

---

## Vendored Source Libraries

| Component | Path | Version / License | Purpose |
| --- | --- | --- | --- |
| **SQLite3 C Amalgamation** | `vendor/sqlite3/sqlite3.c`, `vendor/sqlite3/sqlite3.h` | SQLite 3.x — Public Domain | Statically compiled into `libsqlite3.a` / `sqlite3.lib` by `build.rs` with `SQLITE_THREADSAFE=1`, `SQLITE_ENABLE_FTS5`, and `SQLITE_ENABLE_JSON1`. |

---

## External Cargo Crates

**None (`[dependencies]` is empty in both `Cargo.toml` and `installer/Cargo.toml`).**
All JSON parsing/serialization (`src/json.rs`), SQLite FFI (`src/sqlite.rs`), syntax highlighting (`src/syntax.rs`), code editor & diff engine (`src/editor.rs`), ANSI/VT100 terminal emulation (`src/vt_screen.rs`), multi-provider LLM agent engine (`src/agent.rs`), and Win32 / Linux X11 GUI backends (`src/platform/`) are implemented in 100% self-contained Rust so builds succeed even in completely offline / air-gapped environments.

---

## Build-Time Requirements (Developers)

- **Windows 10/11 x64**: Rust `1.75+` (`rustc` + `cargo`) with either MSVC Build Tools (`cl.exe`, `lib.exe`, `rc.exe`) or MinGW-w64 (`x86_64-pc-windows-gnu`).
- **Linux x86_64**: Rust `1.75+` (`rustc` + `cargo`) + standard C compiler (`cc` + `ar`) for compiling `vendor/sqlite3/sqlite3.c` (note: `libX11.so.6` is loaded dynamically via `dlopen` at runtime, so `libx11-dev` headers are not required at compile time).
