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
# The installer resolves the matching prebuilt binary in this order:
#   1. GitHub Releases asset      (normal path)
#   2. dist/ inside a source archive from codeload.github.com
#                                 - dist/ is committed to the repository, so the
#                                   prebuilt binaries are reachable even before any
#                                   release assets are published, and with no Rust
#                                   toolchain on the host
#   3. build from source with Cargo
#                                 - last resort; needs Rust plus a C toolchain for
#                                   the vendored SQLite amalgamation
#
# Usage:
#   ./scripts/install-linux.sh                    # install, or update if older
#   ./scripts/install-linux.sh --uninstall        # remove AirCode
#   ./scripts/install-linux.sh --prefix /opt      # custom install prefix
#   ./scripts/install-linux.sh --build-from-source
#   ./scripts/install-linux.sh --arch arm64       # force architecture
#   ./scripts/install-linux.sh --version 2.2.1    # install a specific version
#   ./scripts/install-linux.sh --yes              # never prompt (answer yes)
#   ./scripts/install-linux.sh --force            # reinstall / replace newer
#
# Re-running the installer checks the version of the binary already on disk:
# older -> it asks whether to update, then updates; same -> it says so and
# stops (--force to reinstall); newer -> it refuses to downgrade unless
# --version or --force says that is really what you want.
# =============================================================================
set -euo pipefail

APP_NAME="AirCode"
APP_ID="aircode"
BIN_NAME="AI-Coding-Agent"
REPO_SLUG="${AIRCODE_REPO:-lemonys-001/Aircode}"
VERSION="2.2.1"
PREFIX="${PREFIX:-/usr/local}"
BINDIR="${PREFIX}/bin"
DATADIR="${PREFIX}/share/${APP_ID}"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
FROM_SOURCE=0
UNINSTALL=0
ASSUME_YES=0
FORCE=0
VERSION_EXPLICIT=0
ARCH=""
SRC_DIR=""

log()  { printf '\033[1;36m[%s]\033[0m %s\n' "$APP_NAME" "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --uninstall)        UNINSTALL=1 ;;
    --build-from-source) FROM_SOURCE=1 ;;
    -y|--yes)            ASSUME_YES=1 ;;
    --force|--reinstall) FORCE=1 ;;
    --prefix)           shift; PREFIX="${1:?--prefix needs a value}"; BINDIR="${PREFIX}/bin"; DATADIR="${PREFIX}/share/${APP_ID}" ;;
    --prefix=*)         PREFIX="${1#*=}"; BINDIR="${PREFIX}/bin"; DATADIR="${PREFIX}/share/${APP_ID}" ;;
    --arch)             shift; ARCH="${1:?--arch needs a value}" ;;
    --version)          shift; VERSION="${1:?--version needs a value}"; VERSION_EXPLICIT=1 ;;
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

# ---------------------------------------------------------------------------
# Version check: is an AirCode already here, and is it older than this one?
#
# The installed binary reports its own version, so the comparison is against
# what is actually on disk rather than against a file we hope is current. A
# binary from before --version existed prints nothing and exits non-zero, so
# that case is reported as "no version" and falls through to a plain install.
# ---------------------------------------------------------------------------
INSTALLED_BIN="${BINDIR}/${BIN_NAME}"
INSTALLED_VERSION=""
WAS_UPDATE=0

# version_cmp <a> <b> - prints -1, 0 or 1 for a<b, a==b, a>b.
# Written by hand because `sort -V` is not portable to busybox.
version_cmp() {
  local a="$1" b="$2" i x y n
  local -a A=() B=()
  IFS=. read -r -a A <<<"$a"
  IFS=. read -r -a B <<<"$b"
  n=${#A[@]}
  [ "${#B[@]}" -gt "$n" ] && n=${#B[@]}
  for ((i = 0; i < n; i++)); do
    x="${A[i]:-0}"
    y="${B[i]:-0}"
    # Compare numerically; "" or a suffix like "rc1" counts as 0.
    case "$x" in '' | *[!0-9]*) x=0 ;; esac
    case "$y" in '' | *[!0-9]*) y=0 ;; esac
    if [ "$x" -gt "$y" ]; then echo 1; return 0; fi
    if [ "$x" -lt "$y" ]; then echo -1; return 0; fi
  done
  echo 0
}

# installed_version_of <binary> - prints the version the binary reports.
installed_version_of() {
  local bin="$1" out=""
  [ -x "$bin" ] || return 0
  # An old binary rejects --version and exits 2 without opening a window, so
  # running it is safe; failures are simply swallowed here.
  out="$("$bin" --version 2>/dev/null || true)"
  # Output looks like "AI-Coding-Agent 2.2.1" - take the first dotted number.
  printf '%s\n' "$out" \
    | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
  return 0
}

# confirm <question> - answered yes by --yes, or by the user typing y.
# Without a terminal to ask on (curl | bash, CI, a provisioner) running the
# installer at all is the answer, so it proceeds rather than stalling.
confirm() {
  local reply=""
  if [ "$ASSUME_YES" -eq 1 ]; then return 0; fi
  if [ ! -t 0 ]; then return 0; fi
  printf '%s [y/N] ' "$1"
  read -r reply || return 1
  case "$reply" in
    y | Y | yes | YES | Yes) return 0 ;;
    *) return 1 ;;
  esac
}

if [ -e "$INSTALLED_BIN" ]; then
  INSTALLED_VERSION="$(installed_version_of "$INSTALLED_BIN")"
fi

if [ -n "$INSTALLED_VERSION" ]; then
  case "$(version_cmp "$INSTALLED_VERSION" "$VERSION")" in
    -1)
      log "AirCode ${INSTALLED_VERSION} is installed; ${VERSION} is available."
      if confirm "Update AirCode ${INSTALLED_VERSION} -> ${VERSION} now?"; then
        log "Updating AirCode ${INSTALLED_VERSION} -> ${VERSION} ..."
        WAS_UPDATE=1
      else
        log "Update declined - AirCode ${INSTALLED_VERSION} left in place."
        exit 0
      fi
      ;;
    0)
      log "AirCode ${VERSION} is already installed."
      if [ "$FORCE" -eq 1 ]; then
        log "--force given - reinstalling ${VERSION}."
      else
        log "Already up to date. Re-run with --force to reinstall, or --uninstall to remove."
        exit 0
      fi
      ;;
    1)
      # What is on disk is newer than this script targets. Downgrading is
      # almost never intended, so it takes an explicit request.
      if [ "$VERSION_EXPLICIT" -eq 1 ]; then
        warn "Requested ${VERSION} is older than the installed ${INSTALLED_VERSION}."
        if confirm "Downgrade AirCode ${INSTALLED_VERSION} -> ${VERSION}?"; then
          WAS_UPDATE=1
        else
          log "Downgrade declined - AirCode ${INSTALLED_VERSION} left in place."
          exit 0
        fi
      elif [ "$FORCE" -eq 1 ]; then
        warn "Replacing newer AirCode ${INSTALLED_VERSION} with ${VERSION} (--force)."
        WAS_UPDATE=1
      else
        warn "AirCode ${INSTALLED_VERSION} is installed - newer than this installer's ${VERSION}."
        log "Not downgrading. Pass --version ${VERSION} to force it, or --force to replace."
        exit 0
      fi
      ;;
  esac
elif [ -e "$INSTALLED_BIN" ]; then
  warn "${INSTALLED_BIN} exists but did not report a version - reinstalling over it."
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ---------------------------------------------------------------------------
# Resolve payload binary (release asset -> source build fallback)
# ---------------------------------------------------------------------------
PAYLOAD=""
PAYLOAD_URL=""
SUMS_FILE="${WORK}/SHA256SUMS.txt"
SUMS_READY=0

# codeload serves repository archives. A local path, file://, ssh:// or git://
# remote has no archive to fetch, so only plain owner/repo slugs qualify.
CODELOAD_BASE=""
if printf '%s' "$REPO_SLUG" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'; then
  CODELOAD_BASE="https://codeload.github.com/${REPO_SLUG}"
fi

# fetch <url> <outfile> - tries curl, then wget.
fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 10 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --timeout=60 -O "$2" "$1"
  else
    return 1
  fi
}

# verify_sums <file> <asset-name> - compares against $SUMS_FILE when present.
verify_sums() {
  [ -s "$SUMS_FILE" ] || {
    warn "No SHA256SUMS.txt available - installing unverified (set SKIP_CHECKSUM=1 to silence)."
    return 0
  }
  local expected
  expected="$(grep -F "  $2" "$SUMS_FILE" | head -n1 | awk '{print $1}')"
  if [ -z "$expected" ]; then
    warn "No checksum entry for $2 in SHA256SUMS.txt - installing unverified."
    return 0
  fi
  local actual
  actual="$(sha256sum "$1" | awk '{print $1}')"
  if [ "$expected" = "$actual" ]; then
    log "SHA-256 verified: $2"
  else
    die "SHA-256 mismatch for $2 (expected ${expected}, got ${actual}). Refusing to install."
  fi
}

# 1. Explicit local payload (offline installs, mirrors, testing).
if [ -n "${AIRCODE_PAYLOAD:-}" ]; then
  [ -f "${AIRCODE_PAYLOAD}" ] || die "AIRCODE_PAYLOAD is set but not a file: ${AIRCODE_PAYLOAD}"
  PAYLOAD="${AIRCODE_PAYLOAD}"
  log "Using local payload: ${PAYLOAD}"
  # Honour a checksum file sitting next to the payload (a local dist/ tree).
  for cand in "${PAYLOAD%/*}/SHA256SUMS.txt" "./SHA256SUMS.txt" "./dist/SHA256SUMS.txt"; do
    if [ -f "$cand" ]; then cp -f "$cand" "$SUMS_FILE"; break; fi
  done
fi

# 2. GitHub release asset.
if [ -z "$PAYLOAD" ] && [ "$FROM_SOURCE" -eq 0 ] && [ -n "$RELEASES_BASE" ]; then
  for url in \
    "${RELEASES_BASE}/download/v${VERSION}/${ASSET_NAME}" \
    "${RELEASES_BASE}/latest/download/${ASSET_NAME}";
  do
    log "Trying release asset: ${url}"
    if fetch "$url" "${WORK}/${ASSET_NAME}"; then
      PAYLOAD="${WORK}/${ASSET_NAME}"
      PAYLOAD_URL="$url"
      if [ "${SKIP_CHECKSUM:-0}" != "1" ]; then
        # The release ships SHA256SUMS.txt beside the binary.
        fetch "${PAYLOAD_URL%/*}/SHA256SUMS.txt" "$SUMS_FILE" || true
      fi
      break
    fi
  done
fi

# 3. Prebuilt binary out of dist/, from a committed source archive. dist/ is
#    versioned, so this works with no published release and no Rust toolchain.
if [ -z "$PAYLOAD" ] && [ "$FROM_SOURCE" -eq 0 ] && [ -n "$CODELOAD_BASE" ]; then
  if ! command -v tar >/dev/null 2>&1; then
    warn "tar is unavailable - cannot read dist/ out of a source archive."
  else
    for ref in "refs/tags/v${VERSION}" "refs/heads/main"; do
      url="${CODELOAD_BASE}/tar.gz/${ref}"
      log "Trying source archive: ${url}"
      fetch "$url" "${WORK}/aircode.tar.gz" || continue
      [ -s "${WORK}/aircode.tar.gz" ] || continue
      # Archives are wrapped in one top-level directory, "<repo>-<ref>", so the
      # member we want is "<root>/dist/<asset>". Resolve the name instead of
      # guessing it, and match members exactly - no wildcards, so this works on
      # both GNU tar and busybox tar.
      first="$( { tar -tzf "${WORK}/aircode.tar.gz" 2>/dev/null || true; } | head -n1 )"
      root="${first%%/*}"
      [ -n "$root" ] || { warn "Archive ${ref} is unreadable."; continue; }
      log "Extracting dist/${ASSET_NAME} from ${ref} ..."
      if tar -xzf "${WORK}/aircode.tar.gz" -C "$WORK" --strip-components=1 \
           "${root}/dist/${ASSET_NAME}" 2>/dev/null \
         && [ -s "${WORK}/dist/${ASSET_NAME}" ]; then
        PAYLOAD="${WORK}/dist/${ASSET_NAME}"
        # dist/SHA256SUMS.txt ships alongside the binary; optional. It lands
        # in $WORK/dist/, so point the verifier at it there.
        if tar -xzf "${WORK}/aircode.tar.gz" -C "$WORK" --strip-components=1 \
             "${root}/dist/SHA256SUMS.txt" 2>/dev/null \
           && [ -s "${WORK}/dist/SHA256SUMS.txt" ]; then
          SUMS_FILE="${WORK}/dist/SHA256SUMS.txt"
        fi
        log "Using prebuilt binary from dist/ (${ref})"
        break
      fi
      warn "Archive ${ref} carries no dist/${ASSET_NAME}."
    done
  fi
fi

# 4. Verify whatever we resolved.
if [ -n "$PAYLOAD" ] && [ -s "$PAYLOAD" ] && [ "${SKIP_CHECKSUM:-0}" != "1" ]; then
  verify_sums "$PAYLOAD" "$ASSET_NAME"
fi

# 5. Last resort: build from source.
if [ -z "$PAYLOAD" ] || [ ! -s "$PAYLOAD" ]; then
  log "No prebuilt ${ARCH} binary reachable - building AirCode ${VERSION} from source."
  if ! command -v cargo >/dev/null 2>&1; then
    cat >&2 <<MSG

[error] No prebuilt AirCode ${VERSION} binary is reachable for ${ARCH}, and
        cargo is not installed, so AirCode cannot be built here either.

Pick one of these instead:

  * Install Rust, then re-run this script:
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

  * Install a binary you already have (from another machine, or a mirror):
      AIRCODE_PAYLOAD=/path/to/${ASSET_NAME} bash "$0"

  * Build it somewhere that has Rust, then copy the binary across:
      git clone --depth 1 --branch v${VERSION} ${REPO_URL}.git
      cd Aircode && cargo build --release

MSG
    exit 1
  fi
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
if [ "$WAS_UPDATE" -eq 1 ]; then
  log "Updated ${APP_NAME} ${INSTALLED_VERSION:-older} -> ${VERSION} for ${ARCH} (${RUST_TARGET})."
else
  log "Installed ${APP_NAME} ${VERSION} for ${ARCH} (${RUST_TARGET})."
fi
echo
echo "  Binary : ${BINDIR}/${BIN_NAME}"
echo "  Menu   : ${DESKTOP_DIR}/${APP_ID}.desktop (search 'AirCode' in your app grid)"
echo "  Data   : ~/.local/share/${APP_ID} (SQLite sessions, settings, skills, MCP)"
echo
echo "  Run    : ${APP_ID}            # or: ${BIN_NAME} /path/to/project"
echo "  Check  : re-run this script - it reports the installed version and"
echo "           offers to update when a newer one is available"
echo "  Remove : $0 --uninstall"
echo
log "Tip: press Ctrl+K inside AirCode for the fuzzy command palette (/connect, /theme, /fork, /compact)."
