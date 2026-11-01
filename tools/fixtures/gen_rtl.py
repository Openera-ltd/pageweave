#!/usr/bin/env python3
"""Build fixtures/pdf/rtl-hebrew.pdf: right-to-left text as browsers write it, for the Word and
PowerPoint converters (D-039). Each glyph is its own text object, drawn in reading order from right
to left; the paragraph is right-aligned, followed by an English line.

Hebrew needs no letter shaping, so the glyphs can be placed one by one with DejaVu Sans
(ml/fonts, Bitstream Vera license, subset embedded by reportlab).

Usage: python3 tools/fixtures/gen_rtl.py
"""
from __future__ import annotations

from pathlib import Path

from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

ROOT = Path(__file__).resolve().parent.parent.parent
LINES = ["שלום עולם, זהו מסמך לדוגמה", "השורה השנייה של הפסקה"]
ENGLISH = "An English line after the Hebrew paragraph."


def main() -> None:
    pdfmetrics.registerFont(TTFont("DejaVu", str(ROOT / "ml/fonts/fonts-dejavu-core/DejaVuSans.ttf")))
    out = ROOT / "fixtures/pdf/rtl-hebrew.pdf"
    c = canvas.Canvas(str(out), pagesize=(595, 842), invariant=1)
    c.setTitle("Right-to-left fixture")
    size, right, y = 16, 540, 760
    for line in LINES:
        x = right
        for ch in line:  # reading order; each next letter to the left of the one before
            w = pdfmetrics.stringWidth(ch, "DejaVu", size)
            x -= w
            t = c.beginText(x, y)
            t.setFont("DejaVu", size)
            t.textOut(ch)
            c.drawText(t)
        y -= size * 1.5
    c.setFont("DejaVu", 12)
    c.drawString(55, y - 20, ENGLISH)
    c.showPage()
    c.save()
    print("wrote", out)


if __name__ == "__main__":
    main()
