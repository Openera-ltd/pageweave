#!/usr/bin/env python3
"""Build tagged-PDF fixtures for the accessibility check, directly from PDF syntax.

fixtures/pdf/tagged-accessible.pdf — should pass every automated check:
  tagged (MarkInfo + StructTreeRoot with ParentTree), /Lang en-US, title shown in the window title,
  DejaVu Sans embedded (subset), H1 + P, a Figure with /Alt, a Table with TH header cells, a Link
  with /Contents in a Link structure element, a labelled text field (/TU) in a Form element,
  page /Tabs /S.
fixtures/pdf/tagged-issues.pdf — the same page with the usual problems: no /Lang, no
  DisplayDocTitle, the Figure has no /Alt, the Table has only TD cells, the field has no /TU, the
  link has no /Contents, no /Tabs, and the text uses Helvetica without embedding it.

Deterministic. Needs fontTools (`ml/.venv/bin/python tools/fixtures/gen_tagged.py`).
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
FONT = ROOT / "ml" / "fonts" / "fonts-dejavu-core" / "DejaVuSans.ttf"
TEXT = ["Quarterly report PWTAGGED", "Sales grew in every region this quarter.", "Region", "Sales", "North", "120",
        "Read the full report online"]


def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")


def embedded_font(objs: list[bytes], add) -> int:
    used = sorted(set("".join(TEXT) + " "))
    opts = subset.Options()
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.recalc_bounds = True
    font = TTFont(FONT)
    sub = subset.Subsetter(opts)
    sub.populate(text="".join(used))
    sub.subset(font)
    # Fixed timestamps so the subset (and the fixture) are byte-identical on every run.
    font["head"].created = font["head"].modified = 0
    buf = io.BytesIO()
    font.save(buf)
    ttf = buf.getvalue()
    upm = font["head"].unitsPerEm
    cmap = font.getBestCmap()
    hmtx = font["hmtx"]
    widths = [round(hmtx[cmap[c]][0] * 1000 / upm) if c in cmap else 0 for c in range(32, 127)]
    data = zlib.compress(ttf, 9)
    ff = add(b"<< /Length %d /Length1 %d /Filter /FlateDecode >>\nstream\n" % (len(data), len(ttf)) + data + b"\nendstream")
    desc = add(b"<< /Type /FontDescriptor /FontName /PWAAAA+DejaVuSans /Flags 32 /FontBBox [-1021 -463 1793 1232] "
               b"/ItalicAngle 0 /Ascent 928 /Descent -236 /CapHeight 729 /StemV 80 /FontFile2 %d 0 R >>" % ff)
    return add(b"<< /Type /Font /Subtype /TrueType /BaseFont /PWAAAA+DejaVuSans /FirstChar 32 /LastChar 126 "
               b"/Widths [%s] /Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>" % (" ".join(map(str, widths)).encode(), desc))


def build(good: bool) -> bytes:
    objs: list[bytes] = []

    def add(b: bytes) -> int:
        objs.append(b)
        return len(objs)

    def setobj(n: int, b: bytes) -> None:
        objs[n - 1] = b

    catalog, pages, page = add(b""), add(b""), add(b"")
    font = embedded_font(objs, add) if good else add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    # Image: 40 x 20 grey bars.
    px = bytes((60 if (x // 10) % 2 else 200) for _y in range(20) for x in range(40))
    img_data = zlib.compress(px, 9)
    img = add(b"<< /Type /XObject /Subtype /Image /Width 40 /Height 20 /ColorSpace /DeviceGray /BitsPerComponent 8 "
              b"/Filter /FlateDecode /Length %d >>\nstream\n" % len(img_data) + img_data + b"\nendstream")

    def text(mcid: int, tag: str, s: str, x: int, y: int, size: int = 12) -> str:
        return f"/{tag} <</MCID {mcid}>> BDC BT /F1 {size} Tf {x} {y} Td ({esc(s)}) Tj ET EMC\n"

    ops = (text(0, "H1", TEXT[0], 72, 720, 20) + text(1, "P", TEXT[1], 72, 690)
           + "/Figure <</MCID 2>> BDC q 160 0 0 80 72 580 cm /Im1 Do Q EMC\n"
           + text(3, "TH" if good else "TD", TEXT[2], 72, 540) + text(4, "TH" if good else "TD", TEXT[3], 200, 540)
           + text(5, "TD", TEXT[4], 72, 520) + text(6, "TD", TEXT[5], 200, 520)
           + text(7, "Link", TEXT[6], 72, 480))
    content = add(b"<< /Length %d >>\nstream\n" % len(ops) + ops.encode("latin-1") + b"\nendstream")

    # Structure elements (filled in below).
    root, doc, h1, p, fig, table, tr1, tr2, link_el, form_el = (add(b"") for _ in range(10))
    cells = [add(b"") for _ in range(4)]
    contents = b" /Contents (Full report on example.org)" if good else b""
    link = add(b"<< /Type /Annot /Subtype /Link /Rect [70 475 260 495] /Border [0 0 0] /StructParent 1"
               + contents + b" /A << /S /URI /URI (https://example.org/report) >> /P %d 0 R >>" % page)
    tu = b" /TU (Comments)" if good else b""
    field_ap = add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 200 20] /Length 0 >>\nstream\n\nendstream")
    field = add(b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (comments)" + tu + b" /Rect [72 430 272 450] /F 4 /StructParent 2 "
                b"/P %d 0 R /DA (/F1 10 Tf 0 g) /V () /AP << /N %d 0 R >> >>" % (page, field_ap))

    def elem(n: int, s: str, parent: int, kids: str, extra: str = "") -> None:
        setobj(n, f"<< /Type /StructElem /S /{s} /P {parent} 0 R /Pg {page} 0 R /K {kids}{extra} >>".encode("latin-1"))

    setobj(root, f"<< /Type /StructTreeRoot /K {doc} 0 R /ParentTree {add(b'')} 0 R /ParentTreeNextKey 3 >>".encode())
    parent_tree = len(objs)
    elem(doc, "Document", root, f"[{h1} 0 R {p} 0 R {fig} 0 R {table} 0 R {link_el} 0 R {form_el} 0 R]")
    elem(h1, "H1", doc, "0")
    elem(p, "P", doc, "1")
    elem(fig, "Figure", doc, "2", " /Alt (Bar chart: sales by region)" if good else "")
    elem(table, "Table", doc, f"[{tr1} 0 R {tr2} 0 R]")
    elem(tr1, "TR", table, f"[{cells[0]} 0 R {cells[1]} 0 R]")
    elem(tr2, "TR", table, f"[{cells[2]} 0 R {cells[3]} 0 R]")
    head = "TH" if good else "TD"
    for i, (cell, s, mcid) in enumerate(zip(cells, [head, head, "TD", "TD"], [3, 4, 5, 6])):
        elem(cell, s, tr1 if i < 2 else tr2, str(mcid))
    elem(link_el, "Link", doc, f"[7 << /Type /OBJR /Obj {link} 0 R >>]")
    elem(form_el, "Form", doc, f"<< /Type /OBJR /Obj {field} 0 R >>")
    by_mcid = [h1, p, fig, cells[0], cells[1], cells[2], cells[3], link_el]
    setobj(parent_tree, f"<< /Nums [0 [{' '.join(f'{e} 0 R' for e in by_mcid)}] 1 {link_el} 0 R 2 {form_el} 0 R] >>".encode())

    tabs = " /Tabs /S" if good else ""
    setobj(page, (f"<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /StructParents 0{tabs} "
                  f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {img} 0 R >> >> /Contents {content} 0 R "
                  f"/Annots [{link} 0 R {field} 0 R] >>").encode())
    setobj(pages, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>".encode())
    extra = " /Lang (en-US) /ViewerPreferences << /DisplayDocTitle true >>" if good else ""
    acro = f"<< /Fields [{field} 0 R] /DA (/F1 0 Tf 0 g) /DR << /Font << /F1 {font} 0 R >> >> >>"
    setobj(catalog, (f"<< /Type /Catalog /Pages {pages} 0 R /MarkInfo << /Marked true >> /StructTreeRoot {root} 0 R"
                     f"{extra} /AcroForm {acro} >>").encode())
    info = add(b"<< /Title (Quarterly report) /Producer (gen_tagged.py) >>")

    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, body in enumerate(objs, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for off in offsets:
        out += b"%010d 00000 n \n" % off
    out += (b"trailer\n<< /Size %d /Root %d 0 R /Info %d 0 R /ID [<7461676765647069787475726530303031> <7461676765647069787475726530303031>] >>\n"
            b"startxref\n%d\n%%%%EOF\n" % (len(objs) + 1, catalog, info, xref))
    return bytes(out)


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "fixtures" / "pdf"
    (out / "tagged-accessible.pdf").write_bytes(build(True))
    (out / "tagged-issues.pdf").write_bytes(build(False))
    print("wrote tagged-accessible.pdf, tagged-issues.pdf")
    return 0


if __name__ == "__main__":
    sys.exit(main())
