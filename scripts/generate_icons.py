#!/usr/bin/env python3
"""Generates Tauri 2 + Win32 PE icons (icon.ico, 32x32.png, 128x128.png, 128x128@2x.png, icon.png)."""
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
        raw_rows.append(0)
        raw_rows.extend(rgba_bytes[y * stride : (y + 1) * stride])

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw_rows), level=9)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def dist_seg(px: float, py: float, x1: float, y1: float, x2: float, y2: float) -> float:
    dx, dy = x2 - x1, y2 - y1
    l2 = dx * dx + dy * dy
    if l2 == 0.0:
        return math.hypot(px - x1, py - y1)
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / l2))
    return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy))


def sample_pixel(u: float, v: float):
    cx, cy = 0.5, 0.5
    half, rad = 0.44, 0.19
    qx = max(abs(u - cx) - (half - rad), 0.0)
    qy = max(abs(v - cy) - (half - rad), 0.0)
    sd = math.hypot(qx, qy) - rad
    if sd > 0.012:
        return (0.0, 0.0, 0.0, 0.0)

    alpha = max(0.0, min(1.0, (0.012 - sd) / 0.024))
    g1 = math.exp(-((u - 0.30) ** 2 + (v - 0.25) ** 2) * 5.0)
    g2 = math.exp(-((u - 0.75) ** 2 + (v - 0.78) ** 2) * 5.5)
    r = 10 + 24 * g1 + 12 * g2
    g = 14 + 48 * g1 + 38 * g2
    b = 26 + 92 * g1 + 58 * g2

    if -0.022 < sd <= 0.0:
        bt = 1.0 - abs(sd + 0.011) / 0.011
        r = r * (1 - 0.4 * bt) + 99 * (0.4 * bt)
        g = g * (1 - 0.4 * bt) + 179 * (0.4 * bt)
        b = b * (1 - 0.4 * bt) + 237 * (0.4 * bt)

    d_left = min(dist_seg(u, v, 0.33, 0.33, 0.19, 0.50), dist_seg(u, v, 0.19, 0.50, 0.33, 0.67))
    d_right = min(dist_seg(u, v, 0.67, 0.33, 0.81, 0.50), dist_seg(u, v, 0.81, 0.50, 0.67, 0.67))
    d_bolt = min(
        dist_seg(u, v, 0.55, 0.23, 0.43, 0.51),
        dist_seg(u, v, 0.43, 0.51, 0.57, 0.49),
        dist_seg(u, v, 0.57, 0.49, 0.45, 0.77),
    )

    d_chev = min(d_left, d_right)
    if d_chev < 0.055:
        t = max(0.0, min(1.0, (0.055 - d_chev) / 0.018))
        r = r * (1 - t) + 56 * t
        g = g * (1 - t) + 189 * t
        b = b * (1 - t) + 248 * t

    if d_bolt < 0.058:
        t = max(0.0, min(1.0, (0.058 - d_bolt) / 0.018))
        r = r * (1 - t) + 52 * t
        g = g * (1 - t) + 211 * t
        b = b * (1 - t) + 153 * t

    return (r, g, b, alpha * 255.0)


def render_png(size: int) -> bytes:
    buf = bytearray(size * size * 4)
    ss = 2
    inv = 1.0 / (ss * ss)
    idx = 0
    for y in range(size):
        for x in range(size):
            ar = ag = ab = aa = 0.0
            for sy in range(ss):
                v = (y + (sy + 0.5) / ss) / size
                for sx in range(ss):
                    u = (x + (sx + 0.5) / ss) / size
                    sr, sg, sb, sa = sample_pixel(u, v)
                    ar += sr * (sa / 255.0)
                    ag += sg * (sa / 255.0)
                    ab += sb * (sa / 255.0)
                    aa += sa
            aa *= inv
            if aa > 0.5:
                scale = 255.0 / (aa * ss * ss)
                buf[idx] = int(max(0, min(255, round(ar * scale))))
                buf[idx + 1] = int(max(0, min(255, round(ag * scale))))
                buf[idx + 2] = int(max(0, min(255, round(ab * scale))))
                buf[idx + 3] = int(max(0, min(255, round(aa))))
            idx += 4
    return make_png(size, size, bytes(buf))


def main():
    os.makedirs("src-tauri/icons", exist_ok=True)
    os.makedirs("src-tauri/win32-native-host/resources", exist_ok=True)

    p16 = render_png(16)
    p32 = render_png(32)
    p48 = render_png(48)
    p128 = render_png(128)
    p256 = render_png(256)

    open("src-tauri/icons/32x32.png", "wb").write(p32)
    open("src-tauri/icons/128x128.png", "wb").write(p128)
    open("src-tauri/icons/128x128@2x.png", "wb").write(p256)
    open("src-tauri/icons/icon.png", "wb").write(p256)

    blobs = [(16, p16), (32, p32), (48, p48), (256, p256)]
    hdr = struct.pack("<HHH", 0, 1, len(blobs))
    entries = bytearray()
    off = 6 + 16 * len(blobs)
    for sz, bl in blobs:
        wb = 0 if sz == 256 else sz
        entries.extend(struct.pack("<BBBBHHII", wb, wb, 0, 0, 1, 32, len(bl), off))
        off += len(bl)
    ico_bytes = hdr + bytes(entries) + b"".join(bl for _, bl in blobs)
    open("src-tauri/icons/icon.ico", "wb").write(ico_bytes)
    open("src-tauri/win32-native-host/resources/icon.ico", "wb").write(ico_bytes)
    print("Generated Tauri 2 & Win32 icons successfully.")


if __name__ == "__main__":
    main()
