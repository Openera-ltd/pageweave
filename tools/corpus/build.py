"""Build the robustness test corpus (not committed): real-world public-domain PDFs, PDFs from other
producers (LibreOffice, Chromium, reportlab, cairo, ImageMagick) and a spread of image files.

Used by tools/corpus/sweep.py, which runs every PageWeave CLI operation over it. Only for testing,
never for training (PRD 15): downloads are US-government works (public domain) or W3C documents,
pinned by SHA-256 in sources.json; a changed download is refused, not used.

    python3 tools/corpus/build.py [--out target/corpus] [--offline]
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def download(out: Path, offline: bool) -> list[str]:
    sources = json.loads((HERE / "sources.json").read_text())["files"]
    notes = []
    for s in sources:
        dest = out / "real" / s["name"]
        if dest.exists() and sha256(dest.read_bytes()) == s["sha256"]:
            continue
        if offline:
            notes.append(f"missing (offline): {s['name']}")
            continue
        try:
            req = urllib.request.Request(s["url"], headers={"User-Agent": "PageWeave-test-corpus/1 (robustness testing)"})
            with urllib.request.urlopen(req, timeout=120) as r:
                data = r.read()
        except Exception as e:  # network loss is reported, the rest of the corpus still builds
            notes.append(f"download failed: {s['name']}: {e}")
            continue
        if s.get("sha256") and sha256(data) != s["sha256"]:
            notes.append(f"refused (checksum changed upstream): {s['name']} {sha256(data)}")
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
    return notes


def libreoffice(out: Path) -> None:
    """Writer, Calc and Impress documents converted by LibreOffice (its own PDF producer)."""
    src = out / "lo-src"
    src.mkdir(parents=True, exist_ok=True)
    (src / "report.md").write_text(
        "# Quarterly report\n\nThis document was written in Markdown and converted by **LibreOffice**.\n\n"
        "## Figures\n\n| Region | Q1 | Q2 | Change |\n|---|---:|---:|---:|\n| North | 1,204 | 1,388 | +15.3 % |\n"
        "| South | 987 | 902 | −8.6 % |\n| East | 2,410 | 2,512 | +4.2 % |\n\n"
        "## Notes\n\n1. Numbers are in thousands.\n2. Footnote-style remarks follow.[^1]\n\n"
        "Ünïcödé text: café, naïve, Straße, Ελληνικά, Русский, 日本語のテキスト.\n\n"
        "[^1]: A footnote with a [link](https://example.org).\n" + "\nMore body text. " * 400,
        encoding="utf-8",
    )
    subprocess.run(["pandoc", str(src / "report.md"), "-o", str(src / "report.docx")], check=True)
    (src / "sheet.csv").write_text(
        "Item,Qty,Unit price,Total\n" + "".join(f"Item {i},{i % 7 + 1},{i * 1.25:.2f},{(i % 7 + 1) * i * 1.25:.2f}\n" for i in range(1, 120)),
        encoding="utf-8",
    )
    (src / "slides.md").write_text(
        "# Title slide\n\nSubtitle\n\n# Second slide\n\n- Bullet one\n- Bullet two\n\n# Third slide\n\nClosing words.\n",
        encoding="utf-8",
    )
    subprocess.run(["pandoc", str(src / "slides.md"), "-o", str(src / "slides.pptx")], check=True)
    dest = out / "made"
    dest.mkdir(parents=True, exist_ok=True)
    for f in ["report.docx", "sheet.csv", "slides.pptx"]:
        subprocess.run(
            ["soffice", "--headless", "--norestore", "--convert-to", "pdf", "--outdir", str(dest), str(src / f)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=180,
        )
    for f in ["report", "sheet", "slides"]:
        (dest / f"{f}.pdf").rename(dest / f"lo-{f}.pdf")


def chromium(out: Path) -> None:
    """A rich web page printed to PDF by Chromium (Skia producer): columns, SVG, gradients,
    transparency, right-to-left and CJK text, a long table that breaks across pages."""
    html = out / "made" / "web.html"
    rows = "".join(f"<tr><td>{i}</td><td>Row {i}</td><td>{i * 3.14:.2f}</td></tr>" for i in range(1, 90))
    html.write_text(
        f"""<!doctype html><html lang="en"><meta charset="utf-8"><title>Web page fixture</title>
<style>body{{font-family:Inter,DejaVu Sans,sans-serif;margin:24px}} .cols{{columns:2;gap:24px}}
.card{{background:linear-gradient(90deg,#1e3a8a,#9333ea);color:#fff;padding:16px;border-radius:12px;opacity:.9}}
table{{border-collapse:collapse;width:100%}} td,th{{border:1px solid #888;padding:4px}}</style>
<h1>Web page fixture</h1><div class="card">Gradient card with rounded corners and transparency</div>
<svg width="300" height="120"><circle cx="60" cy="60" r="50" fill="#e11d48"/><rect x="130" y="20" width="150" height="80" rx="16" fill="#0ea5e9" fill-opacity=".6"/></svg>
<div class="cols"><p>{'Two-column body text flows here. ' * 60}</p></div>
<p dir="rtl" lang="ar">هذا نص عربي من اليمين إلى اليسار.</p><p lang="ja">これは日本語の文章です。</p>
<a href="https://example.org">A link</a><table><tr><th>#</th><th>Name</th><th>Value</th></tr>{rows}</table></html>""",
        encoding="utf-8",
    )
    script = f"""const {{ chromium }} = require('playwright');
(async () => {{ const b = await chromium.launch(); const p = await b.newPage();
await p.goto('file://{html}'); await p.pdf({{ path: '{out / "made" / "chromium-web.pdf"}', format: 'A4', printBackground: true }});
await b.close(); }})().catch(e => {{ console.error(e); process.exit(1); }});"""
    subprocess.run(["node", "-e", script], check=True, timeout=180)


def reportlab_docs(out: Path) -> None:
    from reportlab.lib.colors import CMYKColor, Color
    from reportlab.lib.pagesizes import A4, landscape, letter
    from reportlab.pdfgen import canvas

    made = out / "made"
    # Many pages, outlines, links, CMYK and transparent fills, odd page sizes, rotated pages.
    c = canvas.Canvas(str(made / "rl-long.pdf"), pagesize=letter)
    c.setTitle("Long reportlab document")
    for i in range(1, 301):
        size = landscape(A4) if i % 50 == 0 else (200, 900) if i % 77 == 0 else letter
        c.setPageSize(size)
        if i % 33 == 0:
            c.setPageRotation(90)
        c.bookmarkPage(f"p{i}")
        c.addOutlineEntry(f"Page {i}", f"p{i}", level=0 if i % 10 == 1 else 1)
        c.setFont("Helvetica-Bold", 18)
        c.drawString(40, size[1] - 60, f"Page {i} of 300")
        c.setFont("Times-Roman", 11)
        for k in range(30):
            c.drawString(40, size[1] - 90 - k * 14, f"Line {k}: the quick brown fox jumps over the lazy dog {i * k}")
        c.setFillColor(CMYKColor(0.1, 0.8, 0.3, 0.05))
        c.rect(40, 40, 120, 40, fill=1, stroke=0)
        c.setFillColor(Color(0, 0.4, 1, alpha=0.4))
        c.circle(220, 60, 30, fill=1, stroke=0)
        c.setFillColorRGB(0, 0, 0)
        c.linkURL("https://example.org", (40, 100, 160, 115))
        c.showPage()
    c.save()
    # One huge page and one tiny page.
    c = canvas.Canvas(str(made / "rl-extreme-sizes.pdf"))
    for size in [(14400, 14400), (72, 36), (3, 3)]:
        c.setPageSize(size)
        c.setFont("Helvetica", min(12, size[1] / 2))
        c.drawString(1, 1, "x")
        c.showPage()
    c.save()


def cairo_resave(out: Path) -> None:
    """The same documents written again by cairo (pdftocairo), a different producer."""
    for src in ["fixtures/pdf/tables.pdf", "fixtures/pdf/form-fields.pdf", "fixtures/pdf/photo-heavy.pdf"]:
        name = Path(src).stem
        subprocess.run(["pdftocairo", "-pdf", str(ROOT / src), str(out / "made" / f"cairo-{name}.pdf")], check=True)


def images(out: Path) -> None:
    from PIL import Image, ImageDraw

    d = out / "images"
    d.mkdir(parents=True, exist_ok=True)

    def card(mode: str, size=(640, 400), color=(30, 90, 200)):
        im = Image.new("RGB", size, (250, 248, 240))
        if min(size) < 60:
            return im.convert(mode) if mode != "RGB" else im
        dr = ImageDraw.Draw(im)
        dr.rectangle([30, 30, size[0] // 2, size[1] // 3], fill=color)
        dr.ellipse([size[0] // 2 + 20, 40, size[0] - 40, size[1] // 2], fill=(220, 40, 60))
        dr.text((40, size[1] - 80), "Sample Text 123", fill=(0, 0, 0))
        return im.convert(mode) if mode != "RGB" else im

    card("RGB").save(d / "rgb.png")
    card("RGBA").save(d / "rgba.png")
    card("L").save(d / "gray.png")
    card("LA").save(d / "gray-alpha.png")
    card("P").save(d / "palette.png")
    card("1").save(d / "bilevel.png")
    card("RGB").save(d / "interlaced.png", interlace=1)
    g16 = Image.new("I;16", (320, 200))
    g16.putdata([(x * 200 + y * 50) % 65536 for y in range(200) for x in range(320)])
    g16.save(d / "gray16.png")  # 16-bit greyscale
    card("RGB").save(d / "baseline.jpg", quality=90)
    card("RGB").save(d / "progressive.jpg", quality=85, progressive=True)
    card("CMYK").save(d / "cmyk.jpg")
    card("RGB", size=(1, 1)).save(d / "one-pixel.png")
    card("RGB", size=(12000, 300)).save(d / "very-wide.png")
    card("RGB", size=(300, 9000)).save(d / "very-tall.jpg")
    exif = Image.Exif()
    exif[0x0112] = 6  # rotate 90° clockwise to display
    card("RGB").save(d / "exif-rotated.jpg", exif=exif)
    card("RGB").save(d / "webp.webp")
    card("RGB").save(d / "gif.gif")
    card("RGB").save(d / "tiff.tif")
    (d / "zero-bytes.png").write_bytes(b"")
    good = io.BytesIO()
    card("RGB").save(good, "PNG")
    (d / "truncated.png").write_bytes(good.getvalue()[: len(good.getvalue()) // 2])
    (d / "text-named.jpg").write_text("not an image", encoding="utf-8")
    # A decompression bomb: a tiny file declaring a huge picture.
    Image.new("1", (40000, 40000)).save(d / "bomb.png", optimize=True)
    for f in ["photo-exif-rotate.jpg", "gray-300dpi.jpg", "alpha.png", "truncated.jpg"]:
        shutil.copy(ROOT / "fixtures" / "images" / f, d / f"fixture-{f}")


def magick(out: Path) -> None:
    """A PDF written by ImageMagick (image-only pages, another producer)."""
    subprocess.run(
        ["convert", str(out / "images" / "rgb.png"), str(out / "images" / "progressive.jpg"), str(out / "made" / "magick-images.pdf")],
        check=True,
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "target" / "corpus")
    ap.add_argument("--offline", action="store_true")
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    (a.out / "made").mkdir(exist_ok=True)
    notes = download(a.out, a.offline)
    steps = [("images", images), ("libreoffice", libreoffice), ("chromium", chromium), ("reportlab", reportlab_docs),
             ("cairo", cairo_resave), ("imagemagick", magick)]
    for name, f in steps:
        try:
            f(a.out)
        except Exception as e:  # a missing producer skips its part; the sweep reports what exists
            notes.append(f"{name} skipped: {e}")
    for n in notes:
        print(n, file=sys.stderr)
    pdfs = sorted(p.relative_to(a.out).as_posix() for p in a.out.rglob("*.pdf") if "lo-src" not in p.parts)
    print(f"{len(pdfs)} PDFs, {len(list((a.out / 'images').iterdir()))} images in {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
