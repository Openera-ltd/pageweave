"""Cut the PageWeave mark out of its near-black backdrop.

Interior pixels (well inside the mark) stay as they are and fully opaque. Each edge pixel is a blend
of the mark's local colour and the backdrop: its colour becomes that local colour (from the nearest
interior pixel) and its alpha the blend fraction, so edges stay smooth with no dark fringe. Cropped
to a square with a small margin.

Usage (numpy and Pillow): python3 -I tools/brand/cutout_logo.py docs/assets/pageweave-logo-on-dark.png \
    docs/assets/pageweave-logo.png preview.png
Then: pageweave-logo-512.png and app/assets/icon-source.png are resized from it, and the app icons are
regenerated with `npx tauri icon assets/icon-source.png -o src-tauri/icons` in app/."""
import sys
import numpy as np
from PIL import Image

src_path, out_path, preview_path = sys.argv[1:4]
src = np.asarray(Image.open(src_path).convert("RGB")).astype(np.float64)
border = np.concatenate([src[:8].reshape(-1, 3), src[-8:].reshape(-1, 3), src[:, :8].reshape(-1, 3), src[:, -8:].reshape(-1, 3)])
bg = np.median(border, axis=0)
diff = np.abs(src - bg).max(axis=2)

def shifts(r):
    return [(dy, dx) for dy in range(-r, r + 1) for dx in range(-r, r + 1) if dy * dy + dx * dx <= r * r and (dy or dx)]

mask = diff > 40
interior = mask.copy()
for dy, dx in shifts(3):  # erode by 3 px: away from the anti-aliased edge
    interior &= np.roll(np.roll(mask, dy, 0), dx, 1)

# Local foreground colour everywhere near the mark: spread interior colours outwards.
fg = np.where(interior[..., None], src, 0.0)
known = interior.copy()
for _ in range(16):
    acc = np.zeros_like(fg)
    cnt = np.zeros(diff.shape)
    for dy, dx in shifts(1):
        k = np.roll(np.roll(known, dy, 0), dx, 1)
        acc += np.roll(np.roll(fg, dy, 0), dx, 1) * k[..., None]
        cnt += k
    grow = ~known & (cnt > 0)
    if not grow.any():
        break
    fg[grow] = acc[grow] / cnt[grow][:, None]
    known |= grow

d = fg - bg
num = ((src - bg) * d).sum(axis=2)
den = np.maximum((d * d).sum(axis=2), 1e-6)
alpha = np.clip(num / den, 0, 1)
alpha[diff <= 4] = 0
alpha[interior] = 1
alpha[~known] = 0
rgb = np.where(interior[..., None], src, fg)
rgba = np.dstack([np.clip(rgb, 0, 255), alpha * 255]).round().astype(np.uint8)
im = Image.fromarray(rgba, "RGBA")
bbox = im.getchannel("A").point(lambda v: 255 if v > 0 else 0).getbbox()
mark = im.crop(bbox)
side = max(mark.size)
pad = round(side * 0.03)
out = Image.new("RGBA", (side + 2 * pad, side + 2 * pad), (0, 0, 0, 0))
out.paste(mark, ((out.width - mark.width) // 2, (out.height - mark.height) // 2), mark)
out.save(out_path, optimize=True)
print("backdrop", bg, "mark", bbox, "->", out.size)

prev = Image.new("RGBA", (1560, 700), (255, 255, 255, 255))
big = out.resize((480, 480), Image.LANCZOS)
for i, bgc in enumerate([(255, 255, 255, 255), (30, 33, 38, 255), None]):
    tile = Image.new("RGBA", (520, 700), bgc or (244, 245, 247, 255))
    if bgc is None:
        for y in range(0, 700, 20):
            for x in range(0, 520, 20):
                if (x // 20 + y // 20) % 2:
                    tile.paste((200, 200, 200, 255), (x, y, x + 20, y + 20))
    tile.alpha_composite(big, (20, 20))
    for j, s in enumerate([16, 24, 32, 48]):
        t = out.resize((s, s), Image.LANCZOS).resize((s * 3, s * 3), Image.NEAREST)
        tile.alpha_composite(t, (20 + j * 125, 530))
    prev.paste(tile, (i * 520, 0))
prev.save(preview_path)
