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
