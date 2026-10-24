#!/usr/bin/env python3
"""Build fixtures/pdf/scanned.pdf: an image-only ("scanned") PDF from the synthetic OCR corpus, with
its line ground truth in fixtures/pdf/scanned.truth.json, for OCR engine and app tests.

Requires Pillow and numpy (ml/requirements-test.txt). Rights-clean: pages come from pwml.corpus
(authored text, recorded-license fonts). Pages: (1) clean invoice, (2) faded prose, (3) clean
two-column page stored turned by -90 degrees with /Rotate 90, so it displays upright.
Images are 8-bit grey, FlateDecode at 200 dpi.

Also builds fixtures/pdf/skewed-scan.pdf for "Straighten pages": (1) prose skewed -2.55 degrees,
(2) mixed layout skewed +2.77 degrees, (3) two-column skewed +1.05 degrees stored turned with
/Rotate 90, (4) a straight clean scan, (5) a born-digital text page. The skew angles are the
corpus' recorded `rotate_deg` (positive = content turned counter-clockwise).

Usage: python3 tools/fixtures/gen_scanned.py
"""
from __future__ import annotations

import json
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT / "ml"))

from pwml import corpus  # noqa: E402

PAGES = [  # (validation-split index, rotate)
    (2, 0),    # invoice, clean
    (24, 0),   # prose, faded
    (1, 90),   # two-column, clean, stored turned
]

SKEWED = [
    (20, 0),   # prose, skew -2.55
    (23, 0),   # mixed, skew +2.77
    (21, 90),  # two-column, skew +1.05, stored turned
    (2, 0),    # invoice, clean (straight)
    None,      # born-digital text page
]

# Pages turned in the scanner and NOT corrected by /Rotate (they display turned): for page
# orientation. (index, counter-clockwise turn of the stored image, add the correcting /Rotate?)
# A page stored turned N degrees counter-clockwise reads upright with /Rotate N (clockwise).
TURNED = [
    (0, 90, False),   # prose: needs /Rotate 90
    (3, 180, False),  # mixed, upside down: needs /Rotate 180
    (1, 270, False),  # two-column: needs /Rotate 270
    (2, 0, False),    # invoice, upright
    None,             # born-digital text page
]


# A born-digital page with a picture of a scanned invoice under its text (OCR of pictures on pages
# that already have text).
MIXED = [("mixed", 2)]


def main() -> int:
    fonts = corpus.load_fonts()
    build(fonts, MIXED, "text-with-picture.pdf", None)
    build(fonts, PAGES, "scanned.pdf", "scanned.truth.json")
    build(fonts, SKEWED, "skewed-scan.pdf", None)
    build(fonts, TURNED, "turned-scan.pdf", None)
    return 0


def build(fonts, pages, name: str, truth_name: str | None) -> None:
    objs: list[bytes] = []

    def add(b: bytes) -> int:
        objs.append(b)
        return len(objs)

    catalog = add(b"")
    pages_obj = add(b"")
    kids, truth = [], []
    for spec in pages:
        if spec is None:
            font = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
            content = b"BT /F1 12 Tf 72 720 Td (A born-digital page with its own text PWDIGITAL. It is never turned.) Tj ET"
            c = add(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
            kids.append(add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> >> /Contents %d 0 R >>" % (pages_obj, font, c)))
            continue
        if spec[0] == "mixed":
            rendered, gt = corpus.render_page(corpus.SPLITS["val"] + spec[1], fonts, "val")
            img = rendered.image.convert("L")
            w, h = img.size
            data = zlib.compress(img.tobytes(), 9)
            im = add(b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length %d >>\nstream\n" % (w, h, len(data)) + data + b"\nendstream")
            font = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
            # The picture at half size in the lower part of a Letter page; text above it.
            pw, ph = w * 72 / corpus.DPI / 2, h * 72 / corpus.DPI / 2
            content = (b"BT /F1 12 Tf 72 740 Td (Born-digital text above a picture PWMIXED. The picture below shows an invoice.) Tj ET "
                       b"q %.4f 0 0 %.4f 72 60 cm /Im0 Do Q" % (pw, ph))
            c = add(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
            kids.append(add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R >> /XObject << /Im0 %d 0 R >> >> /Contents %d 0 R >>" % (pages_obj, font, im, c)))
            continue
        idx, rot, keep_rotate = (*spec, True) if len(spec) == 2 else spec
        rendered, gt = corpus.render_page(corpus.SPLITS["val"] + idx, fonts, "val")
        img = rendered.image.convert("L")
        if rot:
            # Stored turned `rot` degrees counter-clockwise (PIL); /Rotate `rot` (clockwise) displays it upright.
            img = img.rotate(rot, expand=True)
        w, h = img.size
        data = zlib.compress(img.tobytes(), 9)
        im = add(b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length %d >>\nstream\n" % (w, h, len(data)) + data + b"\nendstream")
        pw, ph = w * 72 / corpus.DPI, h * 72 / corpus.DPI
        content = b"q %.4f 0 0 %.4f 0 0 cm /Im0 Do Q" % (pw, ph)
        c = add(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
        rot_entry = b" /Rotate %d" % rot if rot and keep_rotate else b""
        kids.append(add(b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %.4f %.4f]%s /Resources << /XObject << /Im0 %d 0 R >> >> /Contents %d 0 R >>" % (pages_obj, pw, ph, rot_entry, im, c)))
        truth.append({"page": len(truth), "source": gt["id"], "layout": gt["layout"], "condition": gt["condition"], "rotate": rot, "lines": [l["text"] for l in gt["lines"]]})
    objs[catalog - 1] = b"<< /Type /Catalog /Pages %d 0 R >>" % pages_obj
    objs[pages_obj - 1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (b" ".join(b"%d 0 R" % k for k in kids), len(kids))
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, body in enumerate(objs, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, catalog, xref)
    dest = ROOT / "fixtures" / "pdf"
    (dest / name).write_bytes(bytes(out))
    if truth_name:
        (dest / truth_name).write_text(json.dumps({"generator": "tools/fixtures/gen_scanned.py", "pages": truth}, indent=1, ensure_ascii=False) + "\n")
    print(f"wrote {name} ({len(out) // 1024} KiB, {len(kids)} pages)")


if __name__ == "__main__":
    sys.exit(main())
