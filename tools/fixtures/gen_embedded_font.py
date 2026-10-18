#!/usr/bin/env python3
"""Build fixtures/pdf/embedded-font.pdf for editing existing text and images.

Page 1 (Letter):
  - "Invoice 2026 total due" in DejaVu Sans, embedded as a SUBSET TrueType font that contains only
    the glyphs this file uses (as most producers do), one text object;
  - "Payment terms: 30 days" in the same font, one text object PER WORD (to test run grouping);
  - "Standard font line" in Helvetica (not embedded);
  - a 120 × 80 RGB image (orange with a blue bar).
DejaVu fonts: Bitstream Vera license (ml/fonts/fonts-dejavu-core/LICENSE-copyright.txt).

Requires fontTools (in ml/.venv: `ml/.venv/bin/python tools/fixtures/gen_embedded_font.py`).
"""
from __future__ import annotations

import io
import sys
import zlib
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent.parent
FONT = ROOT / "ml" / "fonts" / "fonts-dejavu-core" / "DejaVuSans.ttf"
LINE_A = "Invoice 2026 total due"
WORDS_B = ["Payment", "terms:", "30", "days"]


def main() -> int:
    used = sorted(set(LINE_A + "".join(WORDS_B) + " "))
    opts = subset.Options()
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.recalc_bounds = True
    font = TTFont(FONT)
    sub = subset.Subsetter(opts)
    sub.populate(text="".join(used))
    sub.subset(font)
    buf = io.BytesIO()
    font.save(buf)
    ttf = buf.getvalue()
    upm = font["head"].unitsPerEm
    cmap = font.getBestCmap()
    hmtx = font["hmtx"]
    widths = []
    for code in range(32, 127):
        g = cmap.get(code)
        widths.append(round(hmtx[g][0] * 1000 / upm) if g else 0)

    objs: list[bytes] = []

    def add(b: bytes) -> int:
        objs.append(b)
        return len(objs)

    catalog, pages = add(b""), add(b"")
    data = zlib.compress(ttf, 9)
    ff = add(b"<< /Length %d /Length1 %d /Filter /FlateDecode >>\nstream\n" % (len(data), len(ttf)) + data + b"\nendstream")
    desc = add(
        b"<< /Type /FontDescriptor /FontName /PWEDIT+DejaVuSans /Flags 32 /FontBBox [-1021 -463 1793 1232] "
        b"/ItalicAngle 0 /Ascent 928 /Descent -236 /CapHeight 729 /StemV 80 /FontFile2 %d 0 R >>" % ff
    )
    dejavu = add(
        b"<< /Type /Font /Subtype /TrueType /BaseFont /PWEDIT+DejaVuSans /FirstChar 32 /LastChar 126 /Widths [%s] "
        b"/Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>" % (" ".join(map(str, widths)).encode(), desc)
    )
    helv = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    # Image: orange with a blue horizontal bar.
    w, h = 120, 80
    px = bytearray()
    for y in range(h):
        for _ in range(w):
            px += bytes((30, 60, 200)) if 35 <= y < 45 else bytes((240, 140, 30))
    img_data = zlib.compress(bytes(px), 9)
    img = add(
        b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 "
        b"/Filter /FlateDecode /Length %d >>\nstream\n" % (w, h, len(img_data)) + img_data + b"\nendstream"
    )
    width_of = lambda s: sum(widths[ord(c) - 32] for c in s) * 14 / 1000  # noqa: E731
    parts = [b"BT /F1 14 Tf 72 700 Td (%s) Tj ET" % LINE_A.encode()]
    x = 72.0
    for word in WORDS_B:
        parts.append(b"BT /F1 14 Tf %.2f 670 Td (%s) Tj ET" % (x, word.encode()))
        x += width_of(word + " ")
    parts.append(b"BT /F2 12 Tf 72 640 Td (Standard font line) Tj ET")
    parts.append(b"q 120 0 0 80 72 500 cm /Im1 Do Q")
    content = b"\n".join(parts)
    c = add(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")
    page = add(
        b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 %d 0 R /F2 %d 0 R >> "
        b"/XObject << /Im1 %d 0 R >> >> /Contents %d 0 R >>" % (pages, dejavu, helv, img, c)
    )
    objs[catalog - 1] = b"<< /Type /Catalog /Pages %d 0 R >>" % pages
    objs[pages - 1] = b"<< /Type /Pages /Kids [%d 0 R] /Count 1 >>" % page
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
    dest = ROOT / "fixtures" / "pdf" / "embedded-font.pdf"
    dest.write_bytes(bytes(out))
    print(f"wrote {dest.name} ({len(out)} bytes; subset font {len(ttf)} bytes, {len(used)} characters)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
