"""Design Studio on real pages: render a page to a picture (as a flat flyer or screenshot would
be), rebuild it with `pageweave design`, and measure what comes back: word recall of the editable
text against the page's own text, the object counts, and how close the PDF export looks to the
picture (mean absolute difference per channel, 0–255).

    python3 tools/corpus/studioquality.py --cli target/debug/pageweave
"""

from __future__ import annotations

import argparse
import base64
import collections
import io
import json
import re
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

ROOT = Path(__file__).resolve().parent.parent.parent
WORD = re.compile(r"[^\W_]+", re.UNICODE)

PAGES = [
    ("made/chromium-web.pdf", 1),
    ("made/lo-slides.pdf", 1),
    ("made/lo-report.pdf", 1),
    ("real/w3c-pdf-table.pdf", 1),
    ("real/nist-sp800-63-3.pdf", 1),
    ("real/irs-p15.pdf", 1),
]


def words(text: str) -> collections.Counter:
    return collections.Counter(w.casefold() for w in WORD.findall(text) if len(w) > 1)


def run(cli: Path, *args: str) -> dict:
    p = subprocess.run([str(cli), "--json", *args], capture_output=True, text=True)
    return json.loads(p.stdout or p.stderr)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=ROOT / "target" / "corpus")
    a = ap.parse_args()
    print(f"{'page':28} {'words':>6} {'recall':>7} {'text':>5} {'shapes':>6} {'pics':>5} {'export diff':>11}")
    for rel, page in PAGES:
        pdf = a.corpus / rel
        if not pdf.exists():
            continue
        want = words(run(a.cli, "text", str(pdf), "--pages", str(page))["result"]["data"]["pages"][0]["text"])
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run(a.cli, "render", str(pdf), "--page", str(page), "--scale", "2", "-o", str(t / "p.png"))
            r = run(a.cli, "design", str(t / "p.png"), "-o", str(t / "d.pwdesign"))
            if not r.get("ok"):
                print(f"{rel:28} design refused: {r.get('error', {}).get('message')}")
                continue
            d = json.loads((t / "d.pwdesign").read_text())
            got = words(" ".join(o.get("text", "") for o in d["objects"] if o["kind"] == "text"))
            kinds = collections.Counter(o["kind"] for o in d["objects"])
            run(a.cli, "design-export", str(t / "d.pwdesign"), "-o", str(t / "e.png"))
            src = Image.open(io.BytesIO(base64.b64decode(d["source_png"]))).convert("RGB")
            out = Image.open(t / "e.png").convert("RGB").resize(src.size)
            diff = sum(ImageStat.Stat(ImageChops.difference(src, out)).mean) / 3
        recall = sum((want & got).values()) / max(1, sum(want.values()))
        print(f"{rel:28} {sum(want.values()):6} {recall:7.3f} {kinds['text']:5} {kinds['shape']:6} {kinds['image']:5} {diff:11.2f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
