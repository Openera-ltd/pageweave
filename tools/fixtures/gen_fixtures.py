#!/usr/bin/env python3
"""Generate synthetic, rights-clean PDF test fixtures directly from PDF syntax.

Independent of PDFium on purpose (decision D-004): the engine under test must not
also produce its own fixtures. Output is deterministic so fixtures can be
regenerated and diffed. Usage: python3 -I tools/fixtures/gen_fixtures.py [OUT_DIR]
"""
from __future__ import annotations

import sys
from pathlib import Path

LETTER = (612, 792)
A4 = (595, 842)


def esc(text: str) -> str:
    return text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")


class PdfWriter:
    def __init__(self) -> None:
        self.objects: list[bytes | None] = []

    def reserve(self) -> int:
        self.objects.append(None)
        return len(self.objects)

    def set(self, num: int, body: bytes | str) -> None:
        self.objects[num - 1] = body.encode("latin-1") if isinstance(body, str) else body

    def add(self, body: bytes | str) -> int:
        num = self.reserve()
        self.set(num, body)
        return num

    def stream(self, data: bytes, extra: str = "") -> int:
        return self.add(b"<< /Length %d %s>>\nstream\n" % (len(data), extra.encode()) + data + b"\nendstream")

    def render(self, root: int, info: int | None = None) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(self.objects, start=1):
            assert body is not None, f"object {i} never set"
            offsets.append(len(out))
            out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(self.objects) + 1)
        for off in offsets:
            out += b"%010d 00000 n \n" % off
        trailer = "<< /Size %d /Root %d 0 R" % (len(self.objects) + 1, root)
        if info:
            trailer += " /Info %d 0 R" % info
        trailer += " /ID [<70616765776561766566697874757265> <70616765776561766566697874757265>] >>"
        out += b"trailer\n" + trailer.encode() + b"\nstartxref\n%d\n%%%%EOF\n" % xref
        return bytes(out)


def text_page_content(lines: list[str], size=12, x=72, y=720, leading=18) -> bytes:
    parts = ["BT", f"/F1 {size} Tf", f"{leading} TL", f"{x} {y} Td"]
    for i, line in enumerate(lines):
        parts.append(f"({esc(line)}) Tj" if i == 0 else f"T* ({esc(line)}) Tj")
    parts.append("ET")
    return "\n".join(parts).encode("latin-1")


def build_document(pages: list[dict], title: str | None = None, outline: bool = False,
                   open_action_js: str | None = None) -> bytes:
    w = PdfWriter()
    catalog = w.reserve()
    pages_obj = w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    page_nums = []
    for p in pages:
        content = w.stream(text_page_content(p["lines"]))
        mw, mh = p.get("size", LETTER)
        rot = f" /Rotate {p['rotate']}" if p.get("rotate") else ""
        page_nums.append(w.add(
            f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 {mw} {mh}]{rot} "
            f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"))
    kids = " ".join(f"{n} 0 R" for n in page_nums)
    w.set(pages_obj, f"<< /Type /Pages /Kids [{kids}] /Count {len(page_nums)} >>")

    extra = ""
    if outline:
        outlines = w.reserve()
        items = [w.reserve() for _ in page_nums]
        for i, (item, pg) in enumerate(zip(items, page_nums)):
            links = ""
            if i > 0:
                links += f" /Prev {items[i-1]} 0 R"
            if i < len(items) - 1:
                links += f" /Next {items[i+1]} 0 R"
            w.set(item, f"<< /Title (Chapter {i+1}) /Parent {outlines} 0 R /Dest [{pg} 0 R /Fit]{links} >>")
        w.set(outlines, f"<< /Type /Outlines /First {items[0]} 0 R /Last {items[-1]} 0 R /Count {len(items)} >>")
        extra += f" /Outlines {outlines} 0 R /PageMode /UseOutlines"
    if open_action_js:
        js = w.add(f"<< /S /JavaScript /JS ({esc(open_action_js)}) >>")
        extra += f" /OpenAction {js} 0 R"
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R{extra} >>")
    info = None
    if title:
        info = w.add(f"<< /Title ({esc(title)}) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_form_document() -> bytes:
    """One page with an AcroForm: text, multiline text, checkbox (on-state named /On, not /Yes),
    a two-button radio group and a combo box, each with appearance streams."""
    w = PdfWriter()
    catalog, pages_obj, page = w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    labels = text_page_content(["Form fixture PWFORM", "", "Full name", "", "Subscribe", "",
                                "Plan: basic / pro", "", "Country", "", "Comments"], size=12, x=40, y=740, leading=20)
    content = w.stream(labels)

    def xobj(width: float, height: float, ops: str) -> int:
        data = ops.encode("latin-1")
        return w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 %g %g] /Resources << /Font << /Helv %d 0 R >> >> /Length %d >>\nstream\n"
                     % (width, height, font, len(data)) + data + b"\nendstream")

    box = lambda width, height: f"0.4 0.4 0.4 RG 1 w 0.5 0.5 {width - 1} {height - 1} re S"
    cross = "0 0 0 RG 1.5 w 3 3 m 11 11 l S 3 11 m 11 3 l S"
    dot = "0 0 0 rg 4 4 6 6 re f"
    annots = []
    fields = []

    # Text field
    name_ap = xobj(250, 20, box(250, 20))
    name = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /T (full_name) /TU (Full name) /Rect [150 666 400 686] "
                 f"/F 4 /P {page} 0 R /DA (/Helv 12 Tf 0 g) /V () /AP << /N {name_ap} 0 R >> /MK << /BC [0.4 0.4 0.4] >> >>")
    annots.append(name); fields.append(name)

    # Checkbox, on-state "On"
    cb_on, cb_off = xobj(14, 14, box(14, 14) + " " + cross), xobj(14, 14, box(14, 14))
    cb = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Btn /T (subscribe) /TU (Subscribe) /Rect [150 628 164 642] "
               f"/F 4 /P {page} 0 R /V /Off /AS /Off /AP << /N << /On {cb_on} 0 R /Off {cb_off} 0 R >> >> /MK << /CA (8) >> >>")
    annots.append(cb); fields.append(cb)

    # Radio group "plan": basic / pro
    plan = w.reserve()
    kids = []
    for i, opt in enumerate(["basic", "pro"]):
        on, off = xobj(14, 14, box(14, 14) + " " + dot), xobj(14, 14, box(14, 14))
        x = 150 + i * 60
        kid = w.add(f"<< /Type /Annot /Subtype /Widget /Parent {plan} 0 R /Rect [{x} 588 {x + 14} 602] /F 4 /P {page} 0 R "
                    f"/AS /Off /AP << /N << /{opt} {on} 0 R /Off {off} 0 R >> >> /MK << /CA (l) >> >>")
        kids.append(kid); annots.append(kid)
    w.set(plan, f"<< /FT /Btn /Ff 49152 /T (plan) /TU (Plan) /V /Off /Kids [{' '.join(f'{k} 0 R' for k in kids)}] >>")
    fields.append(plan)

    # Combo box
    combo_ap = xobj(150, 20, box(150, 20) + " /Tx BMC BT /Helv 12 Tf 0 g 4 5 Td (Canada) Tj ET EMC")
    combo = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (country) /TU (Country) /Rect [150 546 300 566] "
                  f"/F 4 /P {page} 0 R /Opt [(Canada) (Germany) (Japan)] /V (Canada) /DA (/Helv 12 Tf 0 g) "
                  f"/AP << /N {combo_ap} 0 R >> >>")
    annots.append(combo); fields.append(combo)

    # Multiline text
    notes_ap = xobj(300, 60, box(300, 60))
    notes = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 4096 /T (comments) /TU (Comments) /Rect [150 466 450 526] "
                  f"/F 4 /P {page} 0 R /DA (/Helv 10 Tf 0 g) /V () /AP << /N {notes_ap} 0 R >> >>")
    annots.append(notes); fields.append(notes)

    w.set(page, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
                f"/Contents {content} 0 R /Annots [{' '.join(f'{a} 0 R' for a in annots)}] >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    acro = f"<< /Fields [{' '.join(f'{f} 0 R' for f in fields)}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv {font} 0 R >> >> >>"
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /AcroForm {acro} >>")
    info = w.add("<< /Title (Form fixture) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def numbered_pages(n: int, size=LETTER) -> list[dict]:
    return [{"lines": [f"PageWeave fixture page {i}", f"Marker token PWMARK{i:03d}",
                       "The quick brown fox jumps over the lazy dog."], "size": size}
            for i in range(1, n + 1)]


def main() -> int:
    if len(sys.argv) == 4 and sys.argv[1] == "--big":
        # Large document for benchmarks (not committed): gen_fixtures.py --big PAGES OUT.pdf
        Path(sys.argv[3]).write_bytes(build_document(numbered_pages(int(sys.argv[2])), title="Benchmark"))
        print(f"wrote {sys.argv[2]} pages to {sys.argv[3]}")
        return 0
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent.parent / "fixtures" / "pdf"
    out.mkdir(parents=True, exist_ok=True)
    files: dict[str, bytes] = {}

    files["single-page.pdf"] = build_document(numbered_pages(1), title="Single page")
    files["five-pages.pdf"] = build_document(numbered_pages(5), title="Five pages", outline=True)
    mixed = numbered_pages(3)
    mixed[1]["size"] = A4
    mixed[2]["rotate"] = 90
    files["mixed-sizes-rotated.pdf"] = build_document(mixed, title="Mixed sizes")
    files["twenty-pages.pdf"] = build_document(numbered_pages(20), title="Twenty pages")
    files["open-action-javascript.pdf"] = build_document(
        numbered_pages(1), title="Has JavaScript", open_action_js="app.alert('should never run');")
    search = [{"lines": ["Alpha beta gamma", "alpha ALPHA alphabet", "delta"]},
              {"lines": ["No match here"]},
              {"lines": ["Gamma and alpha again"]}]
    files["search-terms.pdf"] = build_document(search, title="Search terms")

    files["form-fields.pdf"] = build_form_document()

    good = files["five-pages.pdf"]
    files["corrupt-truncated.pdf"] = good[: len(good) // 2]
    files["not-a-pdf.pdf"] = b"This is plain text with a .pdf extension.\n"
    files["empty.pdf"] = b""

    for name, data in sorted(files.items()):
        (out / name).write_bytes(data)
    print(f"wrote {len(files)} fixtures to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
