#!/usr/bin/env python3
"""Draw the app icon (a small quilt: rows of patches, one block per generation) and write
crates/server/assets/icon.ico (Windows exe) and icon-64.rgba (window icon). Needs Pillow. Run from the repo root."""
from PIL import Image, ImageDraw
import os

OUT = os.path.join("crates", "server", "assets")
S = 1024  # draw large, shrink for quality
PAL = ["#2f6f73", "#d9a441", "#b5523b", "#5b7f3a", "#7a5c99", "#c9c2b0"]
# 4 generations, a few patches each: wider at the bottom like a tree seen from its roots
ROWS = [[0], [1, 2], [3, 4, 0, 5], [2, 5, 1, 3, 4, 0]]


def draw():
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    pad, r = int(S * 0.06), int(S * 0.2)
    d.rounded_rectangle([pad, pad, S - pad, S - pad], r, fill="#f4efe3")
    inner = int(S * 0.15)
    box = S - 2 * inner
    gap = int(S * 0.018)
    rh = (box - gap * (len(ROWS) - 1)) // len(ROWS)
    for i, row in enumerate(ROWS):
        y = inner + i * (rh + gap)
        cw = (box - gap * (len(row) - 1)) // len(row)
        for j, c in enumerate(row):
            x = inner + j * (cw + gap)
            d.rounded_rectangle([x, y, x + cw, y + rh], int(S * 0.025), fill=PAL[c])
    return im


big = draw()
os.makedirs(OUT, exist_ok=True)
sizes = [16, 24, 32, 48, 64, 128, 256]
big.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, "icon.ico"), sizes=[(s, s) for s in sizes])
big.resize((64, 64), Image.LANCZOS).save(os.path.join(OUT, "icon-64.png"))
with open(os.path.join(OUT, "icon-64.rgba"), "wb") as f:
    f.write(big.resize((64, 64), Image.LANCZOS).tobytes())
print("written to", OUT)
