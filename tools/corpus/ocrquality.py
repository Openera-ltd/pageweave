"""OCR on real documents made into scans: how much of the original text comes back.

Pages of the corpus' real documents are rendered at 200 dpi, degraded like an office scan (turned
slightly, noise, JPEG), put back into an image-only PDF and recognised with `pageweave ocr`. The
recognised words are compared with the born-digital text of the same pages (word recall as a
multiset, case-folded). Exit 1 when a document is below the floor.

    python3 tools/corpus/ocrquality.py --cli target/debug/pageweave [--pages 1-2] [--floor 0.9]
"""

from __future__ import annotations

import argparse
import collections
import io
import json
import random
import re
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parent.parent.parent
WORD = re.compile(r"[^\W_]+", re.UNICODE)


def words(text: str) -> collections.Counter:
    return collections.Counter(w.casefold() for w in WORD.findall(text) if len(w) > 1)


def run(cli: Path, *args: str) -> dict:
    p = subprocess.run([str(cli), "--json", *args], capture_output=True, text=True)
    return json.loads(p.stdout or p.stderr)


def scan(png: Path, out: Path, seed: int) -> None:
    r = random.Random(seed)
    im = Image.open(png).convert("L")
    im = im.rotate(r.uniform(-0.8, 0.8), resample=Image.BICUBIC, expand=False, fillcolor=255)
    noise = Image.effect_noise(im.size, 18).convert("L")
    im = Image.blend(im, noise, 0.08).filter(ImageFilter.GaussianBlur(0.4))
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=60, dpi=(200, 200))
    out.write_bytes(buf.getvalue())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=ROOT / "target" / "corpus")
    ap.add_argument("--pages", default="1-2")
    ap.add_argument("--floor", type=float, default=0.9)
    a = ap.parse_args()
    low = []
    print(f"{'document':26} {'page':>4} {'words':>6} {'recall':>7}")
    for pdf in sorted((a.corpus / "real").glob("*.pdf")) + [ROOT / "fixtures" / "pdf" / "article.pdf"]:
        info = run(a.cli, "inspect", str(pdf))
        if not info.get("ok"):
            continue
        n = info["result"]["data"]["page_count"]
        first, last = (int(x) for x in a.pages.split("-"))
        for page in range(first, min(last, n) + 1):
            text = run(a.cli, "text", str(pdf), "--pages", str(page))["result"]["data"]["pages"][0]["text"]
            want = words(text)
            if sum(want.values()) < 30:
                continue
            with tempfile.TemporaryDirectory() as tmp:
                t = Path(tmp)
                run(a.cli, "render", str(pdf), "--page", str(page), "--scale", str(200 / 72), "-o", str(t / "p.png"))
                scan(t / "p.png", t / "scan.jpg", page)
                run(a.cli, "images-to-pdf", str(t / "scan.jpg"), "-o", str(t / "scan.pdf"))
                r = run(a.cli, "ocr", str(t / "scan.pdf"), "-o", str(t / "ocr.pdf"))
                if not r.get("ok"):
                    print(f"{pdf.name[:26]:26} {page:4} OCR refused: {r.get('error', {}).get('message')}")
                    low.append((pdf.name, page, 0.0))
                    continue
                got = words(run(a.cli, "text", str(t / "ocr.pdf"))["result"]["data"]["pages"][0]["text"])
            recall = sum((want & got).values()) / sum(want.values())
            print(f"{pdf.name[:26]:26} {page:4} {sum(want.values()):6} {recall:7.3f}")
            if recall < a.floor:
                low.append((pdf.name, page, round(recall, 3)))
    if low:
        print("below the floor:", low)
    return 1 if low else 0


if __name__ == "__main__":
    raise SystemExit(main())
