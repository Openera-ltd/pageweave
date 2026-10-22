#!/usr/bin/env python3
"""Build fixtures/pdf/article.pdf: a realistic document for the Word export (PDF → DOCX).

Page 1 (Letter): a centred title and italic subtitle across the page, then two columns of body text
  wrapped at real glyph widths (DejaVu Serif, embedded), bold numbered headings, a web link on a
  word, a superscript footnote mark, and a picture inside the second column.
Page 2 (Letter): a heading, a bulleted list, a ruled three-column table, and a justified paragraph
  (word spacing set with Tw, as typesetting programs do).
Page 3 (A4 landscape): a heading and a paragraph, for per-page size and orientation.

Bold and italic are the same embedded font with bold/italic font descriptors (PDF readers report
them from the descriptor), so widths stay exact and no extra font files are needed.

Deterministic. Needs fontTools (`ml/.venv/bin/python tools/fixtures/gen_article.py`).
DejaVu fonts: Bitstream Vera license (ml/fonts/fonts-dejavu-core/LICENSE-copyright.txt).
"""
from __future__ import annotations

import io
import sys
import zlib
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent.parent
SERIF = ROOT / "ml" / "fonts" / "fonts-dejavu-core" / "DejaVuSerif.ttf"
SANS = ROOT / "ml" / "fonts" / "fonts-dejavu-core" / "DejaVuSans.ttf"

WORDS = ("river flow measured across seasons shows steady change in sediment and water quality while "
         "upstream wetlands store floods and release them slowly through the dry months of the year").split()

LINK_WORD = "rivers.example.org"
LINK_URI = "https://rivers.example.org/data"


def esc(b: bytes) -> bytes:
    return b.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)")


def enc(s: str) -> bytes:
    return s.encode("cp1252")


class Font:
    """An embedded, subset TrueType font with WinAnsi widths."""

    def __init__(self, path: Path, used: str) -> None:
        opts = subset.Options()
        opts.name_IDs = ["*"]
        opts.notdef_outline = True
        opts.recalc_bounds = True
        font = TTFont(path)
        sub = subset.Subsetter(opts)
        sub.populate(text=used)
        sub.subset(font)
        font["head"].created = font["head"].modified = 0
        buf = io.BytesIO()
        font.save(buf)
        self.ttf = buf.getvalue()
        upm = font["head"].unitsPerEm
        cmap = font.getBestCmap()
        hmtx = font["hmtx"]
        self.widths = []
        for code in range(32, 256):
            try:
                ch = bytes([code]).decode("cp1252")
            except UnicodeDecodeError:
                self.widths.append(0)
                continue
            g = cmap.get(ord(ch))
            self.widths.append(round(hmtx[g][0] * 1000 / upm) if g else 0)

    def width(self, s: str, size: float) -> float:
        return sum(self.widths[b - 32] for b in enc(s)) * size / 1000


class Doc:
    def __init__(self) -> None:
        self.objs: list[bytes] = []

    def add(self, b: bytes) -> int:
        self.objs.append(b)
        return len(self.objs)

    def reserve(self) -> int:
        return self.add(b"")

    def set(self, n: int, b: bytes) -> None:
        self.objs[n - 1] = b

    def stream(self, data: bytes, extra: bytes = b"") -> int:
        z = zlib.compress(data, 9)
        return self.add(b"<< /Length %d /Filter /FlateDecode%s >>\nstream\n" % (len(z), extra) + z + b"\nendstream")

    def render(self, root: int, info: int) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(self.objs, start=1):
            offsets.append(len(out))
            out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(self.objs) + 1)
        for off in offsets:
            out += b"%010d 00000 n \n" % off
        out += (b"trailer\n<< /Size %d /Root %d 0 R /Info %d 0 R /ID [<61727469636c65303031> <61727469636c65303031>] >>\n"
                b"startxref\n%d\n%%%%EOF\n" % (len(self.objs) + 1, root, info, xref))
        return bytes(out)


def font_objects(d: Doc, f: Font, name: str, flags: int, weight: int, italic_angle: int) -> int:
    ff = d.stream(f.ttf, b" /Length1 %d" % len(f.ttf))
    desc = d.add((f"<< /Type /FontDescriptor /FontName /{name} /Flags {flags} /FontBBox [-770 -347 2105 1109] "
                  f"/ItalicAngle {italic_angle} /Ascent 928 /Descent -236 /CapHeight 729 /StemV {160 if weight >= 700 else 80} "
                  f"/FontWeight {weight} /FontFile2 {ff} 0 R >>").encode())
    widths = " ".join(map(str, f.widths))
    return d.add((f"<< /Type /Font /Subtype /TrueType /BaseFont /{name} /FirstChar 32 /LastChar 255 /Widths [{widths}] "
                  f"/Encoding /WinAnsiEncoding /FontDescriptor {desc} 0 R >>").encode())


def words(n: int, start: int) -> list[str]:
    return [WORDS[(start + i) % len(WORDS)] for i in range(n)]


def wrap(ws: list[str], f: Font, size: float, width: float) -> list[list[str]]:
    lines: list[list[str]] = [[]]
    for w in ws:
        trial = " ".join(lines[-1] + [w])
        if lines[-1] and f.width(trial, size) > width:
            lines.append([w])
        else:
            lines[-1].append(w)
    return lines


def tj(font: str, size: float, x: float, y: float, s: str, tw: float = 0.0, rise: float = 0.0) -> bytes:
    extra = b""
    if tw:
        extra += b"%.3f Tw " % tw
    if rise:
        extra += b"%.1f Ts " % rise
    return b"BT /%s %.1f Tf %s%.2f %.2f Td (%s) Tj ET\n" % (font.encode(), size, extra, x, y, esc(enc(s)))


def build() -> bytes:
    used = " ".join(WORDS) + LINK_WORD + "Annual review of river systems PWARTICLE A field study of three catchments 0123456789." \
        "Introduction Methods Results Discussion Station Flow Sediment North East West Summary • – ,:;"
    used += "".join(chr(c) for c in range(32, 127))
    serif = Font(SERIF, used)
    sans = Font(SANS, used)

    d = Doc()
    catalog, pages = d.reserve(), d.reserve()
    f_body = font_objects(d, serif, "PWAAAA+DejaVuSerif", 34, 400, 0)
    f_bold = font_objects(d, serif, "PWAAAB+DejaVuSerif-Bold", 34 | 262144, 700, 0)
    f_ital = font_objects(d, serif, "PWAAAC+DejaVuSerif-Italic", 34 | 64, 400, -12)
    f_sans = font_objects(d, sans, "PWAAAD+DejaVuSans", 32, 400, 0)
    fonts = f"/Font << /B {f_body} 0 R /BB {f_bold} 0 R /BI {f_ital} 0 R /S {f_sans} 0 R >>"

    # Picture: 60 x 20 grey bars, shown 200 x 66 pt in column 2.
    px = bytes((70 if (x // 12) % 2 else 190) for _y in range(20) for x in range(60))
    img = d.stream(px, b" /Type /XObject /Subtype /Image /Width 60 /Height 20 /ColorSpace /DeviceGray /BitsPerComponent 8")

    page_ids = []
    annots: dict[int, list[int]] = {}

    # ---- Page 1: title, subtitle, two columns.
    ops = bytearray()
    title = "Annual review of river systems PWARTICLE"
    ops += tj("S", 22, 306 - sans.width(title, 22) / 2, 720, title)
    sub = "A field study of three catchments"
    ops += tj("BI", 12, 306 - serif.width(sub, 12) / 2, 696, sub)
    cols = [(72.0, 297.0), (315.0, 540.0)]
    size, lead = 10.0, 13.0
    link_rect = None
    footnote_done = False
    for ci, (x0, x1) in enumerate(cols):
        y = 660.0
        heading = ["1 Introduction", "2 Methods"][ci]
        ops += tj("BB", 13, x0, y, heading)
        y -= 20
        for pi in range(3 if ci == 0 else 2):
            ws = words(48 + 7 * pi, 11 * pi + 5 * ci)
            if ci == 0 and pi == 0:
                ws.insert(20, LINK_WORD)
            for li, line in enumerate(wrap(ws, serif, size, x1 - x0)):
                text = " ".join(line)
                ops += tj("B", size, x0, y, text)
                if LINK_WORD in line and link_rect is None:
                    before = " ".join(line[: line.index(LINK_WORD)]) + " "
                    lx = x0 + serif.width(before, size)
                    link_rect = (lx, y - 3, lx + serif.width(LINK_WORD, size), y + 9)
                if ci == 1 and pi == 0 and li == 1 and not footnote_done:
                    ops += tj("B", 6, x0 + serif.width(text, size) + 0.5, y, "1", rise=4)
                    footnote_done = True
                y -= lead
            y -= 7
            if ci == 1 and pi == 0:
                ops += b"q 200 0 0 66 %.2f %.2f cm /Im1 Do Q\n" % (x0, y - 60)
                y -= 80
    content = d.stream(bytes(ops))
    p1 = d.reserve()
    lx0, ly0, lx1, ly1 = link_rect
    link = d.add((f"<< /Type /Annot /Subtype /Link /Rect [{lx0:.2f} {ly0:.2f} {lx1:.2f} {ly1:.2f}] /Border [0 0 0] "
                  f"/A << /S /URI /URI ({LINK_URI}) >> /P {p1} 0 R >>").encode())
    d.set(p1, (f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Resources << {fonts} "
               f"/XObject << /Im1 {img} 0 R >> >> /Contents {content} 0 R /Annots [{link} 0 R] >>").encode())
    page_ids.append(p1)

    # ---- Page 2: heading, list, table, justified paragraph.
    ops = bytearray()
    ops += tj("BB", 14, 72, 720, "3 Results")
    y = 694.0
    for item in ["Flow rose at every station after the spring melt", "Sediment fell where wetlands were restored",
                 "Water quality met the target in two of three catchments"]:
        ops += tj("B", 11, 72, y, "• " + item)
        y -= 16
    # Table: 3 columns at x 72 / 200 / 330, rows of 18 pt, ruled.
    y -= 12
    top = y + 13
    rows = [("Station", "Flow", "Sediment"), ("North", "12.4", "0.82"), ("East", "9.1", "0.64"), ("West", "15.0", "1.10")]
    for r, cells in enumerate(rows):
        for c, (x, s) in enumerate(zip([76, 204, 334], cells)):
            ops += tj("BB" if r == 0 else "B", 10, x, y, s)
        y -= 18
    bottom = y + 13
    ops += b"0.5 w 0 G\n"
    for k in range(len(rows) + 1):
        yy = top - 18 * k
        ops += b"72 %.2f m 460 %.2f l S\n" % (yy, yy)
    for x in [72, 200, 330, 460]:
        ops += b"%d %.2f m %d %.2f l S\n" % (x, top, x, bottom)
    # Justified paragraph: word spacing fills each line but the last.
    y -= 20
    width = 540 - 72
    lines = wrap(words(90, 3), serif, 11, width)
    for i, line in enumerate(lines):
        text = " ".join(line)
        tw = 0.0
        if i + 1 < len(lines) and len(line) > 1:
            tw = (width - serif.width(text, 11)) / (len(line) - 1)
        ops += tj("B", 11, 72, y, text, tw=tw)
        y -= 15
    content = d.stream(bytes(ops))
    p2 = d.add((f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Resources << {fonts} >> "
                f"/Contents {content} 0 R >>").encode())
    page_ids.append(p2)

    # ---- Page 3: A4 landscape.
    ops = bytearray()
    ops += tj("BB", 14, 72, 520, "4 Discussion")
    y = 494.0
    for line in wrap(words(70, 9), serif, 11, 842 - 144):
        ops += tj("B", 11, 72, y, " ".join(line))
        y -= 15
    content = d.stream(bytes(ops))
    p3 = d.add((f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 842 595] /Resources << {fonts} >> "
                f"/Contents {content} 0 R >>").encode())
    page_ids.append(p3)

    d.set(pages, f"<< /Type /Pages /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}] /Count {len(page_ids)} >>".encode())
    d.set(catalog, f"<< /Type /Catalog /Pages {pages} 0 R >>".encode())
    info = d.add(b"<< /Title (Annual review of river systems) /Producer (gen_article.py) >>")
    return d.render(catalog, info)


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "fixtures" / "pdf"
    (out / "article.pdf").write_bytes(build())
    print("wrote article.pdf")
    return 0


if __name__ == "__main__":
    sys.exit(main())
