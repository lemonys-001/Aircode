#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Locate Zig C++ cross-compiler (0.14+)
ZIG_BIN=""
if command -v zig >/dev/null 2>&1; then
  ZIG_BIN="$(command -v zig)"
elif [ -x "/tmp/zigtest/package/zig" ]; then
  ZIG_BIN="/tmp/zigtest/package/zig"
elif [ -x "/tmp/zig-toolchain/package/zig" ]; then
  ZIG_BIN="/tmp/zig-toolchain/package/zig"
else
  echo "[1/4] Fetching portable Zig cross-compiler into /tmp/zig-toolchain..."
  mkdir -p /tmp/zig-toolchain
  (cd /tmp/zig-toolchain && npm pack @ryoppippi/zig-linux-x64 && tar -xzf ryoppippi-zig-linux-x64-*.tgz)
  ZIG_BIN="/tmp/zig-toolchain/package/zig"
fi

echo "Using Zig toolchain: $($ZIG_BIN version)"

echo "[1/4] Generating multi-resolution Windows icon & bundling UI assets..."
python3 scripts/generate_icon.py
node scripts/bundle_ui.mjs

OBJ_DIR="/tmp/aircode-win32-objs"
mkdir -p "$OBJ_DIR" windows-exe

echo "[2/4] Compiling Windows PE resources (icon + HiDPI UTF-8 manifest + VERSIONINFO)..."
(cd native-win32/resources && "$ZIG_BIN" rc -- aircode.rc "$OBJ_DIR/aircode.res")

echo "[3/4] Compiling native Win32 + Edge WebView2 window host..."
if [ ! -f "$OBJ_DIR/webview_host.o" ] || [ "native-win32/src/webview_host.cpp" -nt "$OBJ_DIR/webview_host.o" ]; then
  "$ZIG_BIN" c++ -std=c++17 -O2 -target x86_64-windows-gnu \
    -Inative-win32/vendor \
    -c native-win32/src/webview_host.cpp \
    -o "$OBJ_DIR/webview_host.o"
fi

echo "[4/4] Compiling Aircode C++17 Agent Engine & linking standalone Aircode.exe..."
"$ZIG_BIN" c++ -std=c++17 -O2 -s -target x86_64-windows-gnu -Wl,--subsystem,windows \
  -Inative-win32/include \
  -Inative-win32/vendor \
  native-win32/src/main.cpp \
  "$OBJ_DIR/webview_host.o" \
  "$OBJ_DIR/aircode.res" \
  -lws2_32 -lwinhttp -lole32 -lshell32 -lshlwapi -luser32 -ladvapi32 -ldwmapi -luxtheme -lgdi32 -lversion -luuid \
  -o windows-exe/Aircode.exe

rm -f windows-exe/*.lib windows-exe/*.pdb
ls -lh windows-exe/Aircode.exe
echo "Build complete -> windows-exe/Aircode.exe"
