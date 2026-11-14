#!/usr/bin/env python3
"""Generate PNG decoder fixtures with an encoder that is independent of ours.

Uses CPython's zlib (reference DEFLATE) and a hand-written PNG writer that
applies a different filter type on every row, covering every colour type,
bit depth, palette + tRNS and Adam7. A few files come from Pillow as well.
Each `name.png` has a `name.expect`: 4 x u32 LE (w, h, channels, depth16)
followed by the expected decoded samples as u16 LE.

Run:  python tools/dev/gen_png_fixtures.py   (needs Pillow only for 4 files)
"""
import os
import random
import struct
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "..", "crates", "vl-image", "tests", "data", "png")
os.makedirs(OUT, exist_ok=True)
rnd = random.Random(1234)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)


def filt(kind, bpp, cur, prev):
    out = bytearray(len(cur))
    for i, v in enumerate(cur):
        a = cur[i - bpp] if i >= bpp else 0
        b = prev[i]
        c = prev[i - bpp] if i >= bpp else 0
        pred = [0, a, b, (a + b) // 2, paeth(a, b, c)][kind]
        out[i] = (v - pred) & 0xFF
    return bytes(out)


def pack_rows(pix, w, h, samples, depth):
    """pix[y][x] = tuple of `samples` ints -> list of raw scanlines (bytes)."""
    rows = []
    for y in range(h):
        if depth == 16:
            rows.append(b"".join(struct.pack(">H", s) for x in range(w) for s in pix[y][x]))
        elif depth == 8:
            rows.append(bytes(s for x in range(w) for s in pix[y][x]))
        else:
            bits = "".join(format(pix[y][x][0], f"0{depth}b") for x in range(w))
            bits += "0" * ((8 - len(bits) % 8) % 8)
            rows.append(bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8)))
    return rows


ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]


def write_png(name, w, h, color_type, depth, pix, palette=None, trns=None, interlace=False, expect=None):
    samples = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color_type]
    bpp = max(1, samples * depth // 8)
    raw = b""
    passes = ADAM7 if interlace else [(0, 0, 1, 1)]
    k = 0
    for (x0, y0, dx, dy) in passes:
        xs = list(range(x0, w, dx))
        ys = list(range(y0, h, dy))
        if not xs or not ys:
            continue
        sub = [[pix[y][x] for x in xs] for y in ys]
        rows = pack_rows(sub, len(xs), len(ys), samples, depth)
        prev = bytes(len(rows[0]))
        for r in rows:
            kind = k % 5
            k += 1
            raw += bytes([kind]) + filt(kind, bpp, r, prev)
            prev = r
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, color_type, 0, 0, 1 if interlace else 0))
    if palette is not None:
        data += chunk(b"PLTE", b"".join(bytes(c) for c in palette))
    if trns is not None:
        data += chunk(b"tRNS", trns)
    z = zlib.compress(raw, 9)
    half = len(z) // 2
    data += chunk(b"IDAT", z[:half]) + chunk(b"IDAT", z[half:]) + chunk(b"IEND", b"")  # split IDAT on purpose
    open(os.path.join(OUT, name + ".png"), "wb").write(data)
    ch, d16, vals = expect
    with open(os.path.join(OUT, name + ".expect"), "wb") as f:
        f.write(struct.pack("<IIII", w, h, ch, 1 if d16 else 0))
        f.write(struct.pack(f"<{len(vals)}H", *vals))


def gray(name, depth, interlace=False, trns_key=None, w=13, h=9):
    mx = (1 << depth) - 1
    pix = [[(rnd.randint(0, mx),) for _ in range(w)] for _ in range(h)]
    scale = {1: 255, 2: 85, 4: 17, 8: 1, 16: 1}[depth]
    vals = []
    full = 65535 if depth == 16 else 255
    if trns_key is not None:
        pix[0][0] = (trns_key,)
        pix[h - 1][w - 1] = (trns_key,)
    for y in range(h):
        for x in range(w):
            v = pix[y][x][0]
            vals.append(v * scale)
            if trns_key is not None:
                vals.append(0 if v == trns_key else full)
    trns = struct.pack(">H", trns_key) if trns_key is not None else None
    write_png(name, w, h, 0, depth, pix, trns=trns, interlace=interlace, expect=(2 if trns_key is not None else 1, depth == 16, vals))


def truecolor(name, color_type, depth, interlace=False, trns_key=None, w=13, h=9):
    samples = {2: 3, 4: 2, 6: 4}[color_type]
    mx = (1 << depth) - 1
    pix = [[tuple(rnd.randint(0, mx) for _ in range(samples)) for _ in range(w)] for _ in range(h)]
    # smooth-ish rows too, so filters are meaningfully exercised
    for x in range(w):
        pix[1][x] = tuple(min(mx, (x * mx) // w + s) for s in range(samples))
    full = mx
    vals = []
    if trns_key is not None:
        pix[2][3] = trns_key
    for y in range(h):
        for x in range(w):
            vals.extend(pix[y][x])
            if trns_key is not None:
                vals.append(0 if pix[y][x] == trns_key else full)
    trns = b"".join(struct.pack(">H", v) for v in trns_key) if trns_key is not None else None
    ch = samples + (1 if trns_key is not None else 0)
    write_png(name, w, h, color_type, depth, pix, trns=trns, interlace=interlace, expect=(ch, depth == 16, vals))


def pal(name, depth, with_trns=False, interlace=False, w=13, h=9):
    n = 1 << depth
    palette = [(rnd.randint(0, 255), rnd.randint(0, 255), rnd.randint(0, 255)) for _ in range(n)]
    alpha = [rnd.randint(0, 255) for _ in range(max(1, n // 2))] if with_trns else None
    pix = [[(rnd.randint(0, n - 1),) for _ in range(w)] for _ in range(h)]
    vals = []
    for y in range(h):
        for x in range(w):
            i = pix[y][x][0]
            vals.extend(palette[i])
            if with_trns:
                vals.append(alpha[i] if i < len(alpha) else 255)
    write_png(name, w, h, 3, depth, pix, palette=palette, trns=bytes(alpha) if with_trns else None, interlace=interlace,
              expect=(4 if with_trns else 3, False, vals))


gray("gray1", 1); gray("gray2", 2); gray("gray4", 4); gray("gray8", 8); gray("gray16", 16)
gray("gray8_trns", 8, trns_key=77)
truecolor("rgb8", 2, 8); truecolor("rgb16", 2, 16); truecolor("rgb8_trns", 2, 8, trns_key=(10, 20, 30))
pal("pal1", 1); pal("pal2", 2); pal("pal4", 4); pal("pal8", 8); pal("pal8_trns", 8, with_trns=True)
truecolor("ga8", 4, 8); truecolor("ga16", 4, 16); truecolor("rgba8", 6, 8); truecolor("rgba16", 6, 16)
truecolor("rgb8_adam7", 2, 8, interlace=True, w=21, h=19)
truecolor("rgba16_adam7", 6, 16, interlace=True, w=9, h=11)
gray("gray1_adam7", 1, interlace=True, w=19, h=5)
pal("pal4_adam7", 4, interlace=True, w=3, h=2)

try:
    from PIL import Image
    w, h = 31, 17
    for name, mode in [("pillow_rgb", "RGB"), ("pillow_rgba", "RGBA"), ("pillow_l", "L"), ("pillow_p", "P")]:
        base = Image.new("RGBA", (w, h))
        base.putdata([(rnd.randint(0, 255), (x * 8) % 256, (x * x) % 256, rnd.randint(0, 255)) for x in range(w * h)])
        im = base.convert("RGB").quantize(colors=33) if mode == "P" else base.convert(mode)
        im.save(os.path.join(OUT, name + ".png"), optimize=True)
        ref = im.convert("RGB") if mode == "P" else im
        ch = {"RGB": 3, "RGBA": 4, "L": 1, "P": 3}[mode]
        vals = []
        for px in ref.getdata():
            vals.extend(px if isinstance(px, tuple) else (px,))
        with open(os.path.join(OUT, name + ".expect"), "wb") as f:
            f.write(struct.pack("<IIII", w, h, ch, 0))
            f.write(struct.pack(f"<{len(vals)}H", *vals))
    print("pillow fixtures written")
except ImportError:
    print("Pillow not available: pillow_* fixtures not regenerated")
print("fixtures in", os.path.normpath(OUT), len(os.listdir(OUT)), "files,", sum(os.path.getsize(os.path.join(OUT, f)) for f in os.listdir(OUT)), "bytes")
