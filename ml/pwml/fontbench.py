"""Design Studio font-family benchmark (D-034): does the Studio tell serif from sans text lines?

Renders corpus pages (synthetic text in the vendored fonts, nine scan conditions), rebuilds each
with the real engine (`pageweave --json design page.png`), matches rebuilt text lines to the
ground truth by wording, and scores the family the Studio chose against the font the line was
set in. Monospace (DejaVu Sans Mono) has no family of its own in the Studio and is reported
apart. The threshold was fixed on the Times/Helvetica fixtures before this was run.

    python3 -m pwml.fontbench --cli ../target/debug/pageweave --split val --pages 36 --out out/fontbench
"""

from __future__ import annotations

import argparse
import collections
import json
import subprocess
import sys
import unicodedata
from pathlib import Path

from . import corpus

SERIF = {"dejavu-serif", "caladea", "caladea-italic"}
SANS = {"dejavu-sans", "carlito", "carlito-bold", "inter"}


def norm(s: str) -> str:
    return " ".join(unicodedata.normalize("NFKC", s).split()).casefold()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cli", required=True, type=Path)
    ap.add_argument("--split", default="val", choices=["val", "blind"])
    ap.add_argument("--pages", type=int, default=36)
    ap.add_argument("--out", type=Path, default=Path("out/fontbench"))
    a = ap.parse_args(argv)
    a.out.mkdir(parents=True, exist_ok=True)
    fonts = corpus.load_fonts()
    # (font id, condition) -> [right, wrong]; unmatched lines are counted apart.
    score: dict[tuple[str, str], list[int]] = collections.defaultdict(lambda: [0, 0])
    matched = unmatched = 0
    for i in range(a.pages):
        seed = corpus.SPLITS[a.split] + i
        rendered, gt = corpus.render_page(seed, fonts, a.split)
        png = a.out / f"{gt['id']}.png"
        rendered.image.save(png)
        project = a.out / f"{gt['id']}.pwdesign"
        project.unlink(missing_ok=True)
        subprocess.run([str(a.cli), "--json", "design", str(png), "-o", str(project)], check=True,
                       stdout=subprocess.DEVNULL)
        design = json.loads(project.read_text())
        rebuilt = {}
        for o in design["objects"]:
            if o["kind"] == "text":
                rebuilt.setdefault(norm(o.get("text", "")), o.get("font", "sans"))
        for line in gt["lines"]:
            got = rebuilt.get(norm(line["text"]))
            if got is None:
                unmatched += 1
                continue
            matched += 1
            want = "serif" if line["font"] in SERIF else "sans" if line["font"] in SANS else "mono"
            score[(line["font"], gt["condition"])][0 if got == want or (want == "mono" and got == "sans") else 1] += 1
        print(f"{gt['id']} {gt['layout']:<10} {gt['condition']:<8} {gt['fonts']}", file=sys.stderr)

    by_font: dict[str, list[int]] = collections.defaultdict(lambda: [0, 0])
    by_cond: dict[str, list[int]] = collections.defaultdict(lambda: [0, 0])
    for (font, cond), (right, wrong) in score.items():
        by_font[font][0] += right
        by_font[font][1] += wrong
        if font != "dejavu-mono":
            by_cond[cond][0] += right
            by_cond[cond][1] += wrong
    report = {
        "split": a.split,
        "pages": a.pages,
        "lines_matched": matched,
        "lines_unmatched": unmatched,
        "by_font": {f: {"right": r, "wrong": w, "accuracy": round(r / max(1, r + w), 3)} for f, (r, w) in sorted(by_font.items())},
        "by_condition": {c: {"right": r, "wrong": w, "accuracy": round(r / max(1, r + w), 3)} for c, (r, w) in sorted(by_cond.items())},
    }
    report["by_font_condition"] = {f"{f}/{c}": v for (f, c), v in sorted(score.items())}
    right = sum(v[0] for k, v in by_font.items() if k != "dejavu-mono")
    total = sum(v[0] + v[1] for k, v in by_font.items() if k != "dejavu-mono")
    report["accuracy_serif_vs_sans"] = round(right / max(1, total), 3)
    (a.out / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
