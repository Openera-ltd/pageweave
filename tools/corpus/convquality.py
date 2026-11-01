"""Conversion quality on real documents: how much of a PDF's text survives `to-docx` and `to-pptx`.

For each corpus PDF with text, the first pages are converted; the words of the output (read from the
DOCX/PPTX XML) are compared with the words the engine reads from the same PDF pages. Reports word
recall (share of the PDF's words found in the output, as a multiset) and extra words (in the output
but not the PDF). Exit 1 when any document is below the recall floor.

    python3 tools/corpus/convquality.py --cli target/debug/pageweave [--pages 1-3] [--floor 0.97]
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import subprocess
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORD = re.compile(r"[\w'’-]+", re.UNICODE)


def words(text: str) -> collections.Counter:
    return collections.Counter(w.casefold().strip("'’-") for w in WORD.findall(text) if w.strip("'’-"))


def office_text(path: Path) -> str:
    """All text runs of a DOCX (w:t) or PPTX (a:t), with breaks between runs."""
    out = []
    with zipfile.ZipFile(path) as z:
        for name in sorted(z.namelist()):
            if re.match(r"word/(document|footnotes|endnotes|header\d*|footer\d*)\.xml$|ppt/slides/slide\d+\.xml$", name):
                xml = z.read(name).decode("utf-8")
                out += re.findall(r"<(?:w|a):t(?:\s[^>]*)?>([^<]*)</(?:w|a):t>", xml)
    # Runs split words at style changes; join runs that the converter split mid-word is not
    # possible to know here, so runs are joined with spaces and words compared as counted.
    import html

    return " ".join(html.unescape(t) for t in out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=ROOT / "target" / "corpus")
    ap.add_argument("--pages", default="1-3")
    ap.add_argument("--floor", type=float, default=0.97)
    a = ap.parse_args()
    pdfs = sorted((a.corpus / "real").glob("*.pdf")) + sorted((a.corpus / "made").glob("*.pdf"))
    pdfs += [ROOT / "fixtures" / "pdf" / n for n in ["article.pdf", "tables.pdf", "five-pages.pdf", "text-with-picture.pdf"]]
    low = []
    print(f"{'document':28} {'format':5} {'pdf words':>9} {'recall':>7} {'extra':>6}")
    for pdf in pdfs:
        with tempfile.TemporaryDirectory() as tmp:
            t = subprocess.run([str(a.cli), "--json", "text", str(pdf), "--pages", a.pages], capture_output=True, text=True)
            if t.returncode != 0:
                continue
            data = json.loads(t.stdout)["result"]["data"]
            src = words(" ".join(p["text"] for p in data["pages"]))
            if sum(src.values()) < 20:
                continue
            for fmt in ("docx", "pptx"):
                out = Path(tmp) / f"o.{fmt}"
                r = subprocess.run([str(a.cli), "--json", f"to-{fmt}", str(pdf), "--pages", a.pages, "-o", str(out)], capture_output=True, text=True)
                if r.returncode != 0:
                    print(f"{pdf.name[:28]:28} {fmt:5} conversion refused: {r.stdout.strip()[:120]}")
                    low.append((pdf.name, fmt, 0.0))
                    continue
                got = words(office_text(out))
                common = sum((src & got).values())
                recall = common / sum(src.values())
                extra = sum((got - src).values()) / max(1, sum(got.values()))
                print(f"{pdf.name[:28]:28} {fmt:5} {sum(src.values()):9} {recall:7.3f} {extra:6.3f}")
                if recall < a.floor:
                    low.append((pdf.name, fmt, recall))
    if low:
        print("below the floor:", low)
    return 1 if low else 0


if __name__ == "__main__":
    raise SystemExit(main())
