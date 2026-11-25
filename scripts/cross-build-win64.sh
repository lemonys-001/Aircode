#!/usr/bin/env bash
# Cross-compiles dist/AI-Coding-Agent.exe and dist/AI-Coding-Agent-Setup.exe
# targeting Windows x64 (x86_64-windows-gnu) from Linux/WSL using Zig C/C++/RC.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ZIG_BIN="${ZIG_BIN:-zig}"

echo "=== [1/5] Bundling Offline React + Monaco + Xterm Desktop UI ==="
npm run build
python3 scripts/pack_ui_bundle.py /tmp/ui_assets.pack

echo "=== [2/5] Compiling Windows Icons & Version Manifest (.rc -> .res) ==="
"$ZIG_BIN" rc -- src-tauri/win32-native-host/resources/app.rc /tmp/app.res
"$ZIG_BIN" rc -- src-tauri/win32-native-host/resources/setup.rc /tmp/setup.res

if [[ ! -f /tmp/sqlite3_win64.o ]]; then
  echo "=== [3/5] Compiling SQLite3 Amalgamation (x86_64-windows-gnu) ==="
  SQLITE_C="${SQLITE_AMALGAMATION_C:?Set SQLITE_AMALGAMATION_C to sqlite3.c path}"
  "$ZIG_BIN" cc -O2 -DSQLITE_THREADSAFE=1 -DSQLITE_ENABLE_FTS5 -DSQLITE_ENABLE_JSON1 \
    -target x86_64-windows-gnu -c "$SQLITE_C" -o /tmp/sqlite3_win64.o
fi

echo "=== [4/5] Linking dist/AI-Coding-Agent.exe (PE32+ GUI x86-64) ==="
mkdir -p dist
"$ZIG_BIN" c++ -O2 -std=c++17 \
  -target x86_64-windows-gnu \
  -municode \
  -Isrc-tauri/win32-native-host/vendor \
  src-tauri/win32-native-host/app_host.cpp \
  /tmp/sqlite3_win64.o \
  /tmp/app.res \
  -lole32 -lshell32 -lshlwapi -luser32 -lgdi32 -ladvapi32 -ldwmapi -lwinhttp -luuid -lversion -lcomdlg32 \
  -Wl,--subsystem,windows -s \
  -o dist/AI-Coding-Agent.exe

echo "=== [5/5] Linking dist/AI-Coding-Agent-Setup.exe (PE32+ GUI x86-64) ==="
"$ZIG_BIN" c++ -O2 -std=c++17 \
  -target x86_64-windows-gnu \
  -municode \
  src-tauri/win32-native-host/setup_installer.cpp \
  /tmp/setup.res \
  -lole32 -lshell32 -luser32 -lgdi32 -ladvapi32 -lcomctl32 -ldwmapi -luuid \
  -Wl,--subsystem,windows -s \
  -o dist/AI-Coding-Agent-Setup.exe

rm -f dist/*.lib
sha256sum dist/AI-Coding-Agent.exe dist/AI-Coding-Agent-Setup.exe | sed 's|dist/||' > dist/SHA256SUMS.txt
echo "Done. Executables written to dist/"
