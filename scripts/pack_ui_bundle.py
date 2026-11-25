#!/usr/bin/env python3
"""
Packs all offline UI assets in ui-dist/ into a deterministic binary archive
for direct .incbin embedding inside dist/AI-Coding-Agent.exe.
Format:
  4 bytes: b"UIPK"
  4 bytes: uint32 LE file_count
  Per file:
    4 bytes: uint32 LE rel_path_utf8_len
    N bytes: rel_path_utf8 (forward-slash normalized, e.g. "index.html", "assets/index-....js")
    4 bytes: uint32 LE content_len
    M bytes: raw content bytes
"""
import os
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
UI_DIST = os.path.join(ROOT, "ui-dist")
OUT_PACK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/ui_assets.pack"


def main() -> None:
    if not os.path.isdir(UI_DIST):
        raise SystemExit(f"Missing ui-dist directory: {UI_DIST}")

    files = []
    for dirpath, _, filenames in os.walk(UI_DIST):
        for fn in sorted(filenames):
            full = os.path.join(dirpath, fn)
            rel = os.path.relpath(full, UI_DIST).replace("\\", "/")
            with open(full, "rb") as f:
                data = f.read()
            files.append((rel.encode("utf-8"), data))

    with open(OUT_PACK, "wb") as out:
        out.write(b"UIPK")
        out.write(struct.pack("<I", len(files)))
        for rel_bytes, data in files:
            out.write(struct.pack("<I", len(rel_bytes)))
            out.write(rel_bytes)
            out.write(struct.pack("<I", len(data)))
            out.write(data)

    size_mb = os.path.getsize(OUT_PACK) / (1024 * 1024)
    print(f"Packed {len(files)} UI files -> {OUT_PACK} ({size_mb:.2f} MB)")


if __name__ == "__main__":
    main()
