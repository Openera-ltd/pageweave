#!/usr/bin/env python3
"""Generate fixtures/pdf/photo-heavy.pdf: synthetic, rights-clean "photos" stored at far more
resolution than they are shown at, for testing image downsampling (Reduce file size).

Page 1: a JPEG photo (1500x1000, quality 92) shown at 5x3.33 in (300 dpi) and the text PWPHOTO1.
Page 2: the same image object shown at 2.5x1.67 in, plus a CMYK JPEG (not supported: must be kept).
Page 3: a Flate RGB image (500x500) with a soft mask (alpha) at 2x2 in (250 dpi), and a gray Flate image drawn
        through a Form XObject whose /Matrix scales it (the effective size must follow the matrix).
Page 4: an image used only in a stamp annotation's appearance (must be kept: not placed by content).

Pillow draws the images (deterministic for a given seed and Pillow version); the PDF syntax is
written directly, independent of the engine under test (decision D-004). The committed file is
the reference; regenerate only when the fixture itself must change.
Usage: python3 -I tools/fixtures/gen_photo_pdf.py [OUT.pdf]
"""
from __future__ import annotations

import importlib.util
import io
import random
import sys
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gen_fixtures", HERE / "gen_fixtures.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def photo(w: int, h: int, seed: int, mode: str = "RGB") -> Image.Image:
    rnd = random.Random(seed)
    im = Image.new("RGB", (w, h))
    px = im.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = (int(255 * x / w), int(255 * y / h), int(128 + 100 * ((x + y) % 97) / 97))
    d = ImageDraw.Draw(im)
    for _ in range(40):
        cx, cy, r = rnd.randrange(w), rnd.randrange(h), rnd.randrange(20, max(21, w // 6))
        d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)))
    im = im.filter(ImageFilter.GaussianBlur(3))
    return im.convert(mode)


def jpeg(im: Image.Image, quality: int = 92) -> bytes:
    b = io.BytesIO()
    im.save(b, "JPEG", quality=quality, optimize=False, progressive=False, subsampling=0)
    return b.getvalue()


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE.parent.parent / "fixtures" / "pdf" / "photo-heavy.pdf"
    w = gen.PdfWriter()
    catalog = w.reserve()
    pages = w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    big = photo(1500, 1000, 1)
    big_jpg = jpeg(big)
    im_big = w.stream(big_jpg, "/Type /XObject /Subtype /Image /Width 1500 /Height 1000 /ColorSpace /DeviceRGB "
                               "/BitsPerComponent 8 /Filter /DCTDecode ")
    cmyk = jpeg(photo(600, 400, 2, "CMYK"))
    # Pillow writes Adobe-style inverted CMYK JPEG data; /Decode turns it back for PDF readers.
    im_cmyk = w.stream(cmyk, "/Type /XObject /Subtype /Image /Width 600 /Height 400 /ColorSpace /DeviceCMYK "
                             "/BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0] /Filter /DCTDecode ")
    rgb = photo(500, 500, 3)
    alpha = Image.radial_gradient("L").resize((500, 500))
    smask = w.stream(zlib.compress(alpha.tobytes(), 6), "/Type /XObject /Subtype /Image /Width 500 /Height 500 "
                     "/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode ")
    im_alpha = w.stream(zlib.compress(rgb.tobytes(), 6), f"/Type /XObject /Subtype /Image /Width 500 /Height 500 "
                        f"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /SMask {smask} 0 R ")
    gray = photo(700, 700, 4, "L")
    im_gray = w.stream(zlib.compress(gray.tobytes(), 6), "/Type /XObject /Subtype /Image /Width 700 /Height 700 "
                       "/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode ")
    # The form draws the image in a 100x100 unit square; its /Matrix scales that by 1.44 (to 2 in).
    form = w.stream(b"q 100 0 0 100 0 0 cm /G Do Q", f"/Type /XObject /Subtype /Form /BBox [0 0 100 100] "
                    f"/Matrix [1.44 0 0 1.44 0 0] /Resources << /XObject << /G {im_gray} 0 R >> >> ")
    stamp_img = photo(800, 800, 5)
    im_stamp = w.stream(jpeg(stamp_img), "/Type /XObject /Subtype /Image /Width 800 /Height 800 "
                        "/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode ")
    ap = w.stream(b"q 72 0 0 72 0 0 cm /S Do Q", f"/Type /XObject /Subtype /Form /BBox [0 0 72 72] "
                  f"/Resources << /XObject << /S {im_stamp} 0 R >> >> ")

    def page(content: bytes, xobjects: dict[str, int], extra: str = "") -> int:
        c = w.stream(content)
        xo = " ".join(f"/{k} {v} 0 R" for k, v in xobjects.items())
        return w.add(f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] "
                     f"/Resources << /Font << /F1 {font} 0 R >> /XObject << {xo} >> >> /Contents {c} 0 R {extra}>>")

    text = lambda s: f"BT /F1 14 Tf 72 740 Td ({s}) Tj ET\n".encode()
    kids = [
        page(text("PWPHOTO1") + b"q 360 0 0 240 72 400 cm /P Do Q", {"P": im_big}),
        page(text("PWPHOTO2") + b"q 180 0 0 120 72 500 cm /P Do Q\nq 144 0 0 96 300 500 cm /K Do Q",
             {"P": im_big, "K": im_cmyk}),
        page(text("PWPHOTO3") + b"q 144 0 0 144 72 400 cm /A Do Q\nq 1 0 0 1 300 400 cm /F Do Q",
             {"A": im_alpha, "F": form}),
    ]
    p4 = w.reserve()
    annot = w.add(f"<< /Type /Annot /Subtype /Stamp /Rect [72 500 144 572] /P {p4} 0 R /AP << /N {ap} 0 R >> >>")
    c4 = w.stream(text("PWPHOTO4"))
    w.set(p4, f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
              f"/Contents {c4} 0 R /Annots [{annot} 0 R] >>")
    kids.append(p4)
    w.set(pages, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join(f"{k} 0 R" for k in kids), len(kids)))
    w.set(catalog, f"<< /Type /Catalog /Pages {pages} 0 R >>")
    info = w.add("<< /Title (Photo heavy) >>")
    out.write_bytes(w.render(catalog, info))
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
