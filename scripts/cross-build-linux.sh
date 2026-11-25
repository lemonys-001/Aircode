#!/usr/bin/env bash
# =============================================================================
# AirCode - Cross-Compile all Linux (DE) binaries from any Linux x64 host
# =============================================================================
# Produces (in dist/):
#   AI-Coding-Agent-linux-amd64   (x86_64,  native glibc build)
#   AI-Coding-Agent-linux-arm64   (aarch64, static musl)
#   AI-Coding-Agent-linux-arm32   (armv7hf, static musl)
#   AI-Coding-Agent-linux-x86     (i686,    static musl)
#
# Requirements: a Rust toolchain (rustup, distro rustc - anything recent), plus
# `curl` and `python3`. Zig + the foreign `rust-std` components are fetched
# automatically from the two mirrors that are reachable from CI sandboxes:
#   * zig      -> PyPI  (`ziglang` wheel, no toolchain tarball needed)
#   * rust-std -> npm   (`@rustbin/rust-std-<version>-<target>`)
#
# No root access, no apt/dnf, no cross-gcc packages required.
# =============================================================================
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

RUST_VERSION="${RUST_VERSION:-1.88.0}"
ZIG_VERSION="${ZIG_VERSION:-0.17.1}"
WORK_DIR="${WORK_DIR:-${TMPDIR:-/tmp}/aircode-cross}"
MIRROR_NPM="${MIRROR_NPM:-https://registry.npmjs.org}"
MIRROR_PYPI="${MIRROR_PYPI:-https://pypi.org}"

# host rustc version drives which rust-std revision we can mix in
if command -v rustc >/dev/null 2>&1; then
  HOST_VER="$(rustc --version | awk '{print $2}')"
  if [ "${FORCE_RUST_VERSION:-0}" != "1" ]; then RUST_VERSION="$HOST_VER"; fi
fi

log() { printf '\033[1;36m[cross]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[cross:error]\033[0m %s\n' "$*" >&2; exit 1; }

mkdir -p "$WORK_DIR"
ZIG_DIR="$WORK_DIR/zig"
SYSROOT="$WORK_DIR/sysroot"
mkdir -p "$ZIG_DIR" "$SYSROOT"

# ---------------------------------------------------------------------------
# 1/4  Zig (C toolchain + LLD for every target)
# ---------------------------------------------------------------------------
ZIG="$ZIG_DIR/zig"
if [ ! -x "$ZIG" ]; then
  log "Fetching zig ${ZIG_VERSION} from PyPI ..."
  WHEEL_URL="$(curl -fsSL "$MIRROR_PYPI/pypi/ziglang/json" | python3 -c "
import json,sys
d=json.load(sys.stdin)
f=[x for x in d['urls'] if x['filename'].endswith('py3-none-any.whl')][-1]
print(f['url'])")"
  [ -n "$WHEEL_URL" ] || die "could not resolve ziglang wheel URL"
  curl -fsSL -o "$WORK_DIR/zig.whl" "$WHEEL_URL"
  python3 -c "
import zipfile,sys
zipfile.ZipFile('$WORK_DIR/zig.whl').extractall('$WORK_DIR/zigpkg')"
  # The wheel ships a full zig installation (zig binary + lib/ + include/);
  # the binary locates its siblings via its own path, so copy the whole tree.
  ZIG_ROOT="$(dirname "$(find "$WORK_DIR/zigpkg" -type f -name zig | head -1)")"
  [ -d "$ZIG_ROOT" ] || die "zig installation not found in wheel"
  rm -rf "$ZIG_DIR"; mkdir -p "$ZIG_DIR"
  cp -a "$ZIG_ROOT/." "$ZIG_DIR/"
  chmod +x "$ZIG"
fi
log "zig: $($ZIG version)"

# ---------------------------------------------------------------------------
# 2/4  rust-std for every non-host target (npm mirror)
# ---------------------------------------------------------------------------
TARGETS=(aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf i686-unknown-linux-musl)
for t in "${TARGETS[@]}"; do
  if [ -d "$SYSROOT/lib/rustlib/$t" ]; then continue; fi
  log "Fetching rust-std ${RUST_VERSION} for ${t} from npm ..."
  TARBALL_URL="$(curl -fsSL "$MIRROR_NPM/@rustbin/rust-std-${RUST_VERSION}-${t}" | python3 -c "
import json,sys
d=json.load(sys.stdin)
v=d.get('versions',{}).get(d.get('dist-tags',{}).get('latest','')) or d.get('dist',{})
print(v['dist']['tarball'] if 'dist' in v else v.get('tarball',''))")"
  [ -n "$TARBALL_URL" ] || die "could not resolve rust-std tarball for ${t}"
  curl -fsSL -o "$WORK_DIR/${t}.tgz" "$TARBALL_URL"
  rm -rf "$WORK_DIR/x-${t}"; mkdir -p "$WORK_DIR/x-${t}"
  tar xzf "$WORK_DIR/${t}.tgz" -C "$WORK_DIR/x-${t}"
  SRC="$(find "$WORK_DIR/x-${t}" -maxdepth 4 -type d -name rustlib | head -1)"
  mkdir -p "$SYSROOT/lib/rustlib"
  cp -a "$SRC/." "$SYSROOT/lib/rustlib/"
done

# ---------------------------------------------------------------------------
# 3/4  Linker shim: rustc -> zig cc
# ---------------------------------------------------------------------------
BIN_DIR="$WORK_DIR/bin"
mkdir -p "$BIN_DIR"
cat > "$BIN_DIR/cc" <<'SH'
#!/usr/bin/env python3
"""rustc -> zig cc bridge (see scripts/install-linux.sh for the full story)."""
import os, subprocess, sys

ZIG = os.environ.get("ZIG", "zig")
ARCH = {"x86_64":"x86_64","amd64":"x86_64","aarch64":"aarch64","arm64":"aarch64",
        "armv7":"arm","arm":"arm","i386":"x86","i686":"x86","x86":"x86",
        "riscv64":"riscv64","powerpc64le":"powerpc64le","s390x":"s390x"}

def zig_target(triple):
    parts = triple.split("-")
    arch = ARCH.get(parts[0], parts[0])
    rest = parts[2:] if len(parts) > 2 and parts[1] in ("unknown","pc","none") else parts[1:]
    if "windows" in rest:
        return f"{arch}-windows-{'msvc' if 'msvc' in rest else 'gnu'}"
    if "darwin" in rest:
        return f"{arch}-macos"
    env = next((c for c in ("gnueabihf","musleabihf","gnu","musl") if c in rest), "gnu")
    if arch == "arm" and not env.endswith(("eabihf","eabi")):
        env += "eabihf"
    return f"{arch}-linux-{env}"

def cfg_target():
    a = os.environ.get("CARGO_CFG_TARGET_ARCH", "")
    if not a:
        return ""
    e = os.environ.get("CARGO_CFG_TARGET_ENV", "gnu") or "gnu"
    abi = os.environ.get("CARGO_CFG_TARGET_ABI", "")
    arch = ARCH.get(a, a)
    if arch == "arm" and abi and abi not in ("", "eabihf", "eabi"):
        e = f"{e}{abi}"
    return f"{arch}-{os.environ.get('CARGO_CFG_TARGET_OS','linux')}-{e}"

DROP_EXACT = {"-lgcc_s","-lgcc","-lgcc_eh","-l:libpthread.a","-l:libgcc.a","-l:libgcc_eh.a"}
out, target, i = [], os.environ.get("ZIG_TARGET", ""), 0
args = sys.argv[1:]
while i < len(args):
    a = args[i]
    if a == "--target" and i + 1 < len(args):
        target = args[i+1]; i += 2; continue
    if a.startswith("--target="):
        target = a.split("=", 1)[1]; i += 1; continue
    if a in DROP_EXACT or a.startswith("-melf_") or a.startswith("-Wl,-melf_") or a.startswith("-l:"):
        i += 1; continue
    out.append(a); i += 1
if not target:
    target = cfg_target() or "x86_64-linux-gnu"
zt = zig_target(target)
env = dict(os.environ, ZIG_GLOBAL_CACHE_DIR=os.environ.get("ZIG_GLOBAL_CACHE_DIR", "/tmp/zig-cache"))
sys.exit(subprocess.run([ZIG, "cc", "-target", zt] + out, env=env).returncode)
SH
chmod +x "$BIN_DIR/cc"
cat > "$BIN_DIR/ar" <<'SH'
#!/usr/bin/env bash
exec "${ZIG:-zig}" ar "$@"
SH
chmod +x "$BIN_DIR/ar"
export ZIG="$ZIG"

# ---------------------------------------------------------------------------
# 4/4  Build every target
# ---------------------------------------------------------------------------
MUSL_FLAGS="-C target-feature=+crt-static -C link-self-contained=no -C link-arg=-static"
mkdir -p dist

log "Building native amd64 (x86_64-unknown-linux-gnu) ..."
cargo build --release --offline
install -m755 target/release/AI-Coding-Agent dist/AI-Coding-Agent-linux-amd64

for t in "${TARGETS[@]}"; do
  case "$t" in
    aarch64-unknown-linux-musl)          out="arm64" ;;
    armv7-unknown-linux-musleabihf)      out="arm32" ;;
    i686-unknown-linux-musl)             out="x86" ;;
  esac
  log "Building cross target ${t} -> dist/AI-Coding-Agent-linux-${out} ..."
  up="$(echo "$t" | tr 'a-z-' 'A-Z_')"
  env "CARGO_TARGET_${up}_LINKER=$BIN_DIR/cc" ZIG_TARGET="$t" \
      RUSTFLAGS="$MUSL_FLAGS" \
      cargo build --release --offline --target "$t"
  install -m755 "target/$t/release/AI-Coding-Agent" "dist/AI-Coding-Agent-linux-${out}"
done

log "Done. Linux artifacts:"
ls -lh dist/AI-Coding-Agent-linux-*
