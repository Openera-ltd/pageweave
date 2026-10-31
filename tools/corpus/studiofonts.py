"""Design Studio font guess on real pages: is a rebuilt text line serif, sans or monospace like the
font the PDF actually uses there?

Each page is rendered to a picture (scale 2) and rebuilt with `pageweave design`; the PDF's own text
lines (`pageweave objects`, with font names and boxes) are matched to the Studio's text objects by
overlap. Font names are classified by family keywords; lines whose font name says nothing (subset
names, symbol fonts) are left out. Prints a confusion table per page and in total.

    python3 tools/corpus/studiofonts.py --cli target/debug/pageweave
"""

from __future__ import annotations

import argparse
import collections
import json
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCALE = 2.0

PAGES = [
    ("made/chromium-web.pdf", 1),
    ("made/lo-slides.pdf", 1),
    ("made/lo-report.pdf", 1),
    ("real/w3c-pdf-table.pdf", 1),
    ("real/nist-sp800-63-3.pdf", 1),
    ("real/nist-sp800-63-3.pdf", 2),
    ("real/irs-p15.pdf", 1),
    ("real/irs-p15.pdf", 2),
    ("real/irs-f1040.pdf", 1),
]

MONO = ("courier", "mono", "consolas", "menlo", "inconsolata")
SANS = ("sans", "helvetica", "arial", "calibri", "verdana", "tahoma", "univers", "frutiger", "myriad", "gill", "futura", "segoe", "liberationsans", "carlito", "franklin")
SERIF = ("times", "serif", "georgia", "garamond", "minion", "roman", "cambria", "palatino", "baskerville", "century", "caslon", "bookman", "tinos", "caladea", "liberationserif")


def family(font: str) -> str | None:
    """serif / sans / mono from a font name (subset prefix ignored), None when it says nothing."""
    name = font.split("+", 1)[-1].lower().replace(" ", "").replace("-", "")
    if any(k in name for k in MONO):
        return "mono"
    # "sans" before "serif": "DejaVuSans", "NotoSans" contain neither serif word; "SansSerif" is sans.
    if any(k in name for k in SANS):
        return "sans"
    if any(k in name for k in SERIF):
        return "serif"
    return None


def run(cli: Path, *args: str) -> dict:
    p = subprocess.run([str(cli), "--json", *args], capture_output=True, text=True)
    return json.loads(p.stdout or p.stderr)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=ROOT / "target" / "corpus")
    a = ap.parse_args()
    total: collections.Counter = collections.Counter()
    print(f"{'page':30} {'lines':>5} {'right':>6}  wrong (pdf→studio)")
    for rel, page in PAGES:
        pdf = a.corpus / rel
        if not pdf.exists():
            continue
        height = run(a.cli, "inspect", str(pdf))["result"]["data"]["pages"][page - 1]["height_pt"]
        pdf_lines = run(a.cli, "objects", str(pdf), "--page", str(page))["result"]["data"]["texts"]
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp)
            run(a.cli, "render", str(pdf), "--page", str(page), "--scale", str(SCALE), "-o", str(t / "p.png"))
            r = run(a.cli, "design", str(t / "p.png"), "-o", str(t / "d.pwdesign"))
            if not r.get("ok"):
                print(f"{rel}:{page} design refused")
                continue
            objects = json.loads((t / "d.pwdesign").read_text())["objects"]
        counts: collections.Counter = collections.Counter()
        for o in objects:
            if o["kind"] != "text":
                continue
            guess = o.get("font", "sans")
            # Studio box in PDF points (y up).
            l, rgt = o["x"] / SCALE, (o["x"] + o["w"]) / SCALE
            top, bot = height - o["y"] / SCALE, height - (o["y"] + o["h"]) / SCALE
            votes: collections.Counter = collections.Counter()
            for p in pdf_lines:
                f = family(p["font"])
                if not f:
                    continue
                b = p["rect"]
                ox = min(rgt, b["right"]) - max(l, b["left"])
                oy = min(top, b["top"]) - max(bot, b["bottom"])
                if ox > 0 and oy > 0:
                    votes[f] += ox * oy
            if not votes:
                continue
            truth = votes.most_common(1)[0][0]
            counts[(truth, guess)] += 1
        n = sum(counts.values())
        right = sum(v for (t_, g), v in counts.items() if t_ == g)
        wrong = ", ".join(f"{t_}→{g} {v}" for (t_, g), v in sorted(counts.items()) if t_ != g)
        print(f"{rel + ':' + str(page):30} {n:5} {right / max(1, n):6.2f}  {wrong}")
        total.update(counts)
    n = sum(total.values())
    right = sum(v for (t_, g), v in total.items() if t_ == g)
    print(f"{'all':30} {n:5} {right / max(1, n):6.2f}  " + ", ".join(f"{t_}→{g} {v}" for (t_, g), v in sorted(total.items()) if t_ != g))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
