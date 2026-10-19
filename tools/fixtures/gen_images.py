#!/usr/bin/env python3
"""Generate the small image fixtures for "images to PDF" tests (needs Pillow; outputs are committed).

  photo-exif-rotate.jpg  64x48 RGB JPEG, JFIF 72 dpi, EXIF orientation 6 (shown turned 90° clockwise)
  gray-300dpi.jpg        30x60 greyscale JPEG at 300 dpi
  alpha.png              40x60 RGBA PNG with a transparent half, pHYs 150 dpi
  truncated.jpg          the first 200 bytes of photo-exif-rotate.jpg (damaged)

Usage: python3 tools/fixtures/gen_images.py [OUT_DIR]
"""
import sys
from pathlib import Path

from PIL import Image

out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / "fixtures" / "images"
out.mkdir(parents=True, exist_ok=True)

photo = Image.new("RGB", (64, 48), (200, 40, 40))
for x in range(64):
    for y in range(16):
        photo.putpixel((x, y), (40, 40, 200))  # blue band on top as stored
exif = Image.Exif()
exif[0x0112] = 6
photo.save(out / "photo-exif-rotate.jpg", "JPEG", quality=90, dpi=(72, 72), exif=exif.tobytes())

gray = Image.new("L", (30, 60), 128)
gray.save(out / "gray-300dpi.jpg", "JPEG", quality=90, dpi=(300, 300))

alpha = Image.new("RGBA", (40, 60), (0, 128, 0, 255))
for x in range(20, 40):
    for y in range(60):
        alpha.putpixel((x, y), (0, 0, 0, 0))
alpha.save(out / "alpha.png", "PNG", dpi=(150, 150))

data = (out / "photo-exif-rotate.jpg").read_bytes()
(out / "truncated.jpg").write_bytes(data[:200])
print(f"wrote 4 images to {out}")
