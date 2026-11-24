#!/usr/bin/env python3
"""
Generates a crisp, supersampled multi-resolution Windows .ico file (16, 32, 48, 256 px)
for Aircode.exe using pure Python standard library (struct + zlib + math).
"""
import math
import os
import struct
import zlib


def make_png(width: int, height: int, rgba_bytes: bytes) -> bytes:
    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    raw_rows = bytearray()
    stride = width * 4
    for y in range(height):
        raw_rows.append(0)  # Filter type 0 (None)
        raw_rows.extend(rgba_bytes[y * stride : (y + 1) * stride])

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw_rows), level=9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def dist_to_segment(px: float, py: float, x1: float, y1: float, x2: float, y2: float) -> float:
    dx, dy = x2 - x1, y2 - y1
    l2 = dx * dx + dy * dy
    if l2 == 0.0:
        return math.hypot(px - x1, py - y1)
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / l2))
    proj_x = x1 + t * dx
    proj_y = y1 + t * dy
    return math.hypot(px - proj_x, py - proj_y)


def sample_icon(u: float, v: float):
    """Normalized [0, 1] x [0, 1] coordinate sampler for Aircode emblem."""
    # Rounded squircle bounds [0.06, 0.94], corner radius 0.22
    cx, cy = 0.5, 0.5
    half = 0.44
    rad = 0.20
    qx = max(abs(u - cx) - (half - rad), 0.0)
    qy = max(abs(v - cy) - (half - rad), 0.0)
    sd = math.hypot(qx, qy) - rad

    if sd > 0.012:
        return (0.0, 0.0, 0.0, 0.0)

    # Smooth edge alpha
    alpha = max(0.0, min(1.0, (0.012 - sd) / 0.024))

    # Deep obsidian gradient background (#0B101B -> #111827 with subtle cyan/emerald glow)
    glow = math.exp(-((u - 0.35) ** 2 + (v - 0.25) ** 2) * 5.5)
    glow2 = math.exp(-((u - 0.72) ** 2 + (v - 0.78) ** 2) * 6.0)
    r = 11 + 16 * glow + 8 * glow2
    g = 15 + 42 * glow + 34 * glow2
    b = 26 + 68 * glow + 46 * glow2

    # Subtle inner border highlight
    if -0.024 < sd <= 0.0:
        border_t = 1.0 - abs(sd + 0.012) / 0.012
        r = r * (1 - 0.35 * border_t) + 56 * (0.35 * border_t)
        g = g * (1 - 0.35 * border_t) + 189 * (0.35 * border_t)
        b = b * (1 - 0.35 * border_t) + 248 * (0.35 * border_t)

    # Left code chevron '<'
    d_left = min(
        dist_to_segment(u, v, 0.34, 0.33, 0.20, 0.50),
        dist_to_segment(u, v, 0.20, 0.50, 0.34, 0.67),
    )
    # Right code chevron '>'
    d_right = min(
        dist_to_segment(u, v, 0.66, 0.33, 0.80, 0.50),
        dist_to_segment(u, v, 0.80, 0.50, 0.66, 0.67),
    )
    # Center electric bolt / 'A' slash
    d_bolt = min(
        dist_to_segment(u, v, 0.55, 0.24, 0.43, 0.51),
        dist_to_segment(u, v, 0.43, 0.51, 0.57, 0.49),
        dist_to_segment(u, v, 0.57, 0.49, 0.45, 0.76),
    )

    thickness_code = 0.043
    thickness_bolt = 0.046

    # Soft neon bloom around center bolt
    bloom = math.exp(-((max(0.0, d_bolt - 0.02)) ** 2) * 180.0) * 0.45
    r = min(255.0, r + 16 * bloom)
    g = min(255.0, g + 215 * bloom)
    b = min(255.0, b + 175 * bloom)

    # Render chevrons in crisp electric sky-cyan (#38BDF8 -> #22D3EE)
    d_chev = min(d_left, d_right)
    if d_chev < thickness_code + 0.012:
        t = max(0.0, min(1.0, (thickness_code + 0.012 - d_chev) / 0.018))
        cr = 56 * (1 - v * 0.3) + 34 * (v * 0.3)
        cg = 189 * (1 - v * 0.3) + 211 * (v * 0.3)
        cb = 248 * (1 - v * 0.3) + 238 * (v * 0.3)
        r = r * (1 - t) + cr * t
        g = g * (1 - t) + cg * t
        b = b * (1 - t) + cb * t

    # Render center bolt in vibrant emerald-mint (#34D399 -> #A7F3D0)
    if d_bolt < thickness_bolt + 0.012:
        t = max(0.0, min(1.0, (thickness_bolt + 0.012 - d_bolt) / 0.018))
        br = 110 * (1 - v * 0.5) + 16 * (v * 0.5)
        bg = 247 * (1 - v * 0.5) + 215 * (v * 0.5)
        bb = 195 * (1 - v * 0.5) + 150 * (v * 0.5)
        r = r * (1 - t) + br * t
        g = g * (1 - t) + bg * t
        b = b * (1 - t) + bb * t

    return (r, g, b, alpha * 255.0)


def render_size(size: int) -> bytes:
    buf = bytearray(size * size * 4)
    # 2x2 supersampling for crisp edges
    ss = 2 if size >= 48 else 3
    inv_ss = 1.0 / (ss * ss)
    idx = 0
    for y in range(size):
        for x in range(size):
            ar = ag = ab = aa = 0.0
            for sy in range(ss):
                v = (y + (sy + 0.5) / ss) / size
                for sx in range(ss):
                    u = (x + (sx + 0.5) / ss) / size
                    sr, sg, sb, sa = sample_icon(u, v)
                    ar += sr * (sa / 255.0)
                    ag += sg * (sa / 255.0)
                    ab += sb * (sa / 255.0)
                    aa += sa
            aa *= inv_ss
            if aa > 0.5:
                scale = 255.0 / (aa * ss * ss)
                buf[idx] = int(max(0, min(255, round(ar * scale))))
                buf[idx + 1] = int(max(0, min(255, round(ag * scale))))
                buf[idx + 2] = int(max(0, min(255, round(ab * scale))))
                buf[idx + 3] = int(max(0, min(255, round(aa))))
            idx += 4
    return make_png(size, size, bytes(buf))


def main():
    sizes = [16, 32, 48, 256]
    png_blobs = [render_size(s) for s in sizes]

    # ICO header: Reserved=0, Type=1 (ICON), Count=len(sizes)
    header = struct.pack("<HHH", 0, 1, len(sizes))
    entries = bytearray()
    offset = 6 + 16 * len(sizes)
    for s, blob in zip(sizes, png_blobs):
        w_byte = 0 if s == 256 else s
        h_byte = 0 if s == 256 else s
        entries.extend(struct.pack("<BBBBHHII", w_byte, h_byte, 0, 0, 1, 32, len(blob), offset))
        offset += len(blob)

    out_path = os.path.join("native-win32", "resources", "aircode.ico")
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "wb") as f:
        f.write(header + bytes(entries) + b"".join(png_blobs))
    print(f"Generated {out_path} ({os.path.getsize(out_path)} bytes)")


if __name__ == "__main__":
    main()
