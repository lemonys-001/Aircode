#!/usr/bin/env bash
# =============================================================================
# AirCode - Multi-Arch Linux Installer (Desktop Environment)
# =============================================================================
# Supported architectures:
#   amd64 / x86_64   -> x86_64-unknown-linux-gnu
#   arm64 / aarch64  -> aarch64-unknown-linux-gnu
#   arm32 / armv7l   -> armv7-unknown-linux-gnueabihf
#   x86   / i686     -> i686-unknown-linux-gnu
#
# The installer downloads the matching prebuilt release binary from GitHub
# Releases when available; when no prebuilt binary matches the host it
# transparently falls back to building AirCode from source with Cargo
# (pure Rust; only libc + a C toolchain for the vendored SQLite amalgamation).
#
# Usage:
#   ./scripts/install-linux.sh                    # install
#   ./scripts/install-linux.sh --uninstall        # remove AirCode
#   ./scripts/install-linux.sh --prefix /opt      # custom install prefix
#   ./scripts/install-linux.sh --build-from-source
#   ./scripts/install-linux.sh --arch arm64       # force architecture
# =============================================================================
set -euo pipefail

APP_NAME="AirCode"
APP_ID="aircode"
BIN_NAME="AI-Coding-Agent"
REPO_SLUG="${AIRCODE_REPO:-lemonys-001/Aircode}"
VERSION="2.2.0"
PREFIX="${PREFIX:-/usr/local}"
BINDIR="${PREFIX}/bin"
DATADIR="${PREFIX}/share/${APP_ID}"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
FROM_SOURCE=0
UNINSTALL=0
ARCH=""
SRC_DIR=""

log()  { printf '\033[1;36m[%s]\033[0m %s\n' "$APP_NAME" "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall)        UNINSTALL=1 ;;
    --build-from-source) FROM_SOURCE=1 ;;
    --prefix)           shift; PREFIX="${1:?--prefix needs a value}"; BINDIR="${PREFIX}/bin"; DATADIR="${PREFIX}/share/${APP_ID}" ;;
    --prefix=*)         PREFIX="${1#*=}"; BINDIR="${PREFIX}/bin"; DATADIR="${PREFIX}/share/${APP_ID}" ;;
    --arch)             shift; ARCH="${1:?--arch needs a value}" ;;
    --version)          shift; VERSION="${1:?--version needs a value}" ;;
      -h|--help)          awk 'NR>1 && /^#/ {sub(/^# ?/,""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
    *)                  die "Unknown option: $1 (try --help)" ;;
  esac
  shift
done

# ---------------------------------------------------------------------------
# Architecture detection: amd64, arm64, arm32, x86
# ---------------------------------------------------------------------------
detect_arch() {
  case "$(uname -m 2>/dev/null || echo unknown)" in
    x86_64|amd64)        echo "amd64" ;;
    aarch64|arm64)       echo "arm64" ;;
    armv7l|armv7|armhf)  echo "arm32" ;;
    i386|i486|i586|i686) echo "x86" ;;
    *)                   echo "unknown" ;;
  esac
}

rust_target_for() {
  case "$1" in
    amd64) echo "x86_64-unknown-linux-gnu" ;;
    arm64) echo "aarch64-unknown-linux-gnu" ;;
    arm32) echo "armv7-unknown-linux-gnueabihf" ;;
    x86)   echo "i686-unknown-linux-gnu" ;;
    *)     echo "" ;;
  esac
}

if [ -z "$ARCH" ]; then ARCH="$(detect_arch)"; fi
RUST_TARGET="$(rust_target_for "$ARCH")"
[ -n "$RUST_TARGET" ] || die "Unsupported architecture: $(uname -m). Supported: amd64, arm64, arm32, x86."
# A local path / full URL (used for offline testing or mirrors) is honoured as-is.
if printf '%s' "$REPO_SLUG" | grep -Eq '^(https?|git|ssh|file)://|^[./]|^/'; then
  REPO_URL="$REPO_SLUG"
  RELEASES_BASE=""
else
  REPO_URL="https://github.com/${REPO_SLUG}"
  RELEASES_BASE="https://github.com/${REPO_SLUG}/releases"
fi
ASSET_NAME="${BIN_NAME}-linux-${ARCH}"
log "Detected architecture: ${ARCH} (${RUST_TARGET})"

# ---------------------------------------------------------------------------
# Privilege handling
# ---------------------------------------------------------------------------
SUDO=""
if [ ! -w "$BINDIR" ] && [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
    warn "${BINDIR} is not writable - using sudo for installation steps."
  else
    warn "${BINDIR} is not writable and sudo is unavailable; using ~/.local/bin"
    BINDIR="$HOME/.local/bin"
    mkdir -p "$BINDIR"
  fi
fi

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------
if [ "$UNINSTALL" -eq 1 ]; then
  log "Uninstalling ${APP_NAME}..."
  $SUDO rm -f "${BINDIR}/${BIN_NAME}" "${BINDIR}/${APP_ID}" || true
  $SUDO rm -rf "$DATADIR" || true
  rm -f "${DESKTOP_DIR}/${APP_ID}.desktop" || true
  $SUDO rm -f "/usr/share/icons/hicolor/256x256/apps/${APP_ID}.png" 2>/dev/null || true
  rm -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/${APP_ID}.png" || true
  command -v update-desktop-database >/dev/null 2>&1 && \
    update-desktop-database "${DESKTOP_DIR}" >/dev/null 2>&1 || true
  log "Uninstalled. User data in ~/.local/share/aircode was left untouched."
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ---------------------------------------------------------------------------
# Resolve payload binary (release asset -> source build fallback)
# ---------------------------------------------------------------------------
PAYLOAD=""
PAYLOAD_URL=""
# Offline / local install: AIRCODE_PAYLOAD=/path/to/AI-Coding-Agent-linux-amd64
if [ -n "${AIRCODE_PAYLOAD:-}" ]; then
  [ -f "${AIRCODE_PAYLOAD}" ] || die "AIRCODE_PAYLOAD is set but not a file: ${AIRCODE_PAYLOAD}"
  PAYLOAD="${AIRCODE_PAYLOAD}"
  log "Using local payload: ${PAYLOAD}"
elif [ "$FROM_SOURCE" -eq 0 ]; then
  for url in \
    "${RELEASES_BASE}/download/v${VERSION}/${ASSET_NAME}" \
    "${RELEASES_BASE}/latest/download/${ASSET_NAME}";
  do
    log "Trying release asset: ${url}"
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL --connect-timeout 10 -o "${WORK}/${ASSET_NAME}" "$url" && { PAYLOAD="${WORK}/${ASSET_NAME}"; PAYLOAD_URL="$url"; break; }
    elif command -v wget >/dev/null 2>&1; then
      wget -q --timeout=20 -O "${WORK}/${ASSET_NAME}" "$url" && { PAYLOAD="${WORK}/${ASSET_NAME}"; PAYLOAD_URL="$url"; break; }
    else
      warn "Neither curl nor wget is available - building from source instead."
      break
    fi
  done
fi

# --- Verify the downloaded release asset against dist/SHA256SUMS.txt -----------
if [ -n "$PAYLOAD_URL" ] && [ -s "$PAYLOAD" ] && [ "${SKIP_CHECKSUM:-0}" != "1" ]; then
  SUMS_URL="${PAYLOAD_URL%/*}/SHA256SUMS.txt"
  SUMS_FILE="${WORK}/SHA256SUMS.txt"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 10 -o "$SUMS_FILE" "$SUMS_URL" 2>/dev/null || true
  elif command -v wget >/dev/null 2>&1; then
    wget -q --timeout=20 -O "$SUMS_FILE" "$SUMS_URL" 2>/dev/null || true
  fi
  if [ -s "$SUMS_FILE" ]; then
    EXPECTED="$(grep -F "  ${ASSET_NAME}" "$SUMS_FILE" | head -n1 | awk '{print $1}')"
    if [ -n "$EXPECTED" ]; then
      ACTUAL="$(sha256sum "$PAYLOAD" | awk '{print $1}')"
      if [ "$EXPECTED" = "$ACTUAL" ]; then
        log "SHA-256 verified: ${ASSET_NAME}"
      else
        die "SHA-256 mismatch for ${ASSET_NAME} (expected ${EXPECTED}, got ${ACTUAL}). Refusing to install."
      fi
    else
      warn "No checksum entry for ${ASSET_NAME} in SHA256SUMS.txt - continuing unverified."
    fi
  else
    warn "Could not download SHA256SUMS.txt - continuing unverified (set SKIP_CHECKSUM=1 to silence)."
  fi
fi

if [ -z "$PAYLOAD" ] || [ ! -s "$PAYLOAD" ]; then
  [ -n "$RELEASES_BASE" ] || true
  log "No prebuilt ${ARCH} release asset - building AirCode ${VERSION} from source."
  command -v cargo >/dev/null 2>&1 || die "cargo not found. Install Rust from https://rustup.rs and re-run."
  SRC_DIR="${WORK}/src"
  if command -v git >/dev/null 2>&1; then
    git clone --depth 1 --branch "v${VERSION}" "${REPO_URL}.git" "$SRC_DIR" 2>/dev/null \
      || git clone --depth 1 "${REPO_URL}.git" "$SRC_DIR" \
      || { [ -d "$REPO_URL" ] && cp -a "$REPO_URL" "$SRC_DIR"; } \
      || die "git clone failed."
  else
    die "git is required for the source-build fallback."
  fi
  ( cd "$SRC_DIR" && { cargo build --release --locked 2>/dev/null || cargo build --release; } )
  PAYLOAD="${SRC_DIR}/target/release/${BIN_NAME}"
  [ -f "$PAYLOAD" ] || die "Source build did not produce ${BIN_NAME}."
fi

# ---------------------------------------------------------------------------
# Install the binary
# ---------------------------------------------------------------------------
$SUDO mkdir -p "$BINDIR" "$DATADIR"
$SUDO install -Dm755 "$PAYLOAD" "${BINDIR}/${BIN_NAME}"
$SUDO ln -sf "${BINDIR}/${BIN_NAME}" "${BINDIR}/${APP_ID}" 2>/dev/null || true
log "Installed binary -> ${BINDIR}/${BIN_NAME}"

# ---------------------------------------------------------------------------
# Desktop entry + icon (Linux DE integration)
# ---------------------------------------------------------------------------
ICON_PATH="${DATADIR}/${APP_ID}.png"
for candidate in "${SRC_DIR}/resources/aircode-256.png" "./resources/aircode-256.png" ""; do
  if [ -n "$candidate" ] && [ -f "$candidate" ]; then
    $SUDO install -Dm644 "$candidate" "$ICON_PATH"
    break
  fi
done

mkdir -p "$DESKTOP_DIR"
cat > "${WORK}/${APP_ID}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=${APP_NAME}
GenericName=AI Coding Studio & Agentic Browser
Comment=100% Pure-Rust native AI coding agent, editor, terminal and agentic browser
Exec=${BINDIR}/${BIN_NAME} %F
TryExec=${BINDIR}/${BIN_NAME}
Icon=${APP_ID}
Terminal=false
StartupWMClass=AI-Coding-Agent
Categories=Development;IDE;Utility;TextEditor;
Keywords=ai;code;agent;rust;editor;terminal;browser;mcp;
MimeType=text/plain;inode/directory;
StartupNotify=true
Actions=NewSession;AgenticBrowser;

[Desktop Action NewSession]
Name=Start New Coding Session
Exec=${BINDIR}/${BIN_NAME} --new-session

[Desktop Action AgenticBrowser]
Name=Open Agentic Browser
Exec=${BINDIR}/${BIN_NAME} --browser
EOF

if [ -w "$DESKTOP_DIR" ] || [ -z "$SUDO" ]; then
  install -Dm644 "${WORK}/${APP_ID}.desktop" "${DESKTOP_DIR}/${APP_ID}.desktop"
else
  $SUDO install -Dm644 "${WORK}/${APP_ID}.desktop" "${DESKTOP_DIR}/${APP_ID}.desktop"
fi

if [ -f "$ICON_PATH" ]; then
  ICON_DEST="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
  mkdir -p "$ICON_DEST"
  cp -f "$ICON_PATH" "${ICON_DEST}/${APP_ID}.png" 2>/dev/null || true
fi

command -v update-desktop-database >/dev/null 2>&1 && \
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && \
  gtk-update-icon-cache -q -t -f "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log "Installed ${APP_NAME} ${VERSION} for ${ARCH} (${RUST_TARGET})."
echo
echo "  Binary : ${BINDIR}/${BIN_NAME}"
echo "  Menu   : ${DESKTOP_DIR}/${APP_ID}.desktop (search 'AirCode' in your app grid)"
echo "  Data   : ~/.local/share/${APP_ID} (SQLite sessions, settings, skills, MCP)"
echo
echo "  Run    : ${APP_ID}            # or: ${BIN_NAME} /path/to/project"
echo "  Update : re-run this script"
echo "  Remove : $0 --uninstall"
echo
log "Tip: press Ctrl+K inside AirCode for the fuzzy command palette (/connect, /theme, /fork, /compact)."
