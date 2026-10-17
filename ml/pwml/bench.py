"""Benchmark OCR engines on a corpus split (PRD 14 "benchmark existing models first").

Usage: python -m pwml.bench CORPUS_DIR --split blind --engine tesseract --engine ppocr --out REPORT_DIR
Writes REPORT_DIR/<engine>.json (per page, per condition/layout/font group, timing, memory) and
REPORT_DIR/report.md comparing engines. The blind split may be evaluated, never trained on.
"""
from __future__ import annotations

import argparse
import json
import math
import platform
import resource
import statistics
import time
from pathlib import Path

from .engines import ENGINES
from .metrics import OcrLine, PageScore, score_page


def peak_rss_mb() -> dict:
    own = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
    kids = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss / 1024
    return {"self_mb": round(own, 1), "children_mb": round(kids, 1)}


def run_engine(name: str, corpus: Path, split: str, limit: int | None, details: bool, raw_dir: Path, rescore: bool) -> dict:
    """Run (or, with `rescore`, reuse cached raw output of) an engine and score it. Raw OCR output
    is cached per page so scorer changes never require re-running OCR."""
    pages = sorted((corpus / split).glob("*.json"))
    if limit:
        pages = pages[:limit]
    if not pages:
        raise SystemExit(f"no pages in {corpus / split}")
    raw_dir = raw_dir / name
    raw_dir.mkdir(parents=True, exist_ok=True)
    if rescore:
        cached = json.loads((raw_dir / "_run.json").read_text())
        info, warmup_s = cached["info"], cached["warmup_s"]
        eng = None
    else:
        eng = ENGINES[name]()
        info = eng.info()
        # Warm-up (model load, first-run allocations) is excluded from latency.
        t0 = time.perf_counter()
        eng.run(pages[0].with_suffix(".png"))
        warmup_s = time.perf_counter() - t0
        (raw_dir / "_run.json").write_text(json.dumps({"info": info, "warmup_s": warmup_s}))
    total = PageScore()
    groups: dict[str, dict[str, PageScore]] = {"condition": {}, "layout": {}, "heldout_font": {}}
    per_page, latencies, worst = [], [], []
    for p in pages:
        gt = json.loads(p.read_text())
        raw = raw_dir / f"{gt['id']}.json"
        if eng is None:
            cached = json.loads(raw.read_text())
            dt = cached["seconds"]
            ocr = [OcrLine(o["text"], [tuple(x) for x in o["poly"]], o["conf"]) for o in cached["lines"]]
        else:
            t = time.perf_counter()
            ocr = eng.run(p.with_suffix(".png"))
            dt = time.perf_counter() - t
            raw.write_text(json.dumps({"seconds": dt, "lines": [{"text": o.text, "poly": o.poly, "conf": o.conf} for o in ocr]}, ensure_ascii=False))
        latencies.append(dt)
        s = score_page(gt, ocr, keep_details=details)
        total.add(s)
        for g, key in (("condition", gt["condition"]), ("layout", gt["layout"]), ("heldout_font", str(gt["heldout_font"]))):
            groups[g].setdefault(key, PageScore()).add(s)
        per_page.append({"id": gt["id"], "condition": gt["condition"], "layout": gt["layout"], "fonts": gt["fonts"], "seconds": round(dt, 3), **s.summary()})
        worst.extend(dict(d, page=gt["id"], condition=gt["condition"]) for d in s.details)
    worst.sort(key=lambda d: -d["cer"])
    return {
        "engine": name,
        "info": info,
        "split": split,
        "pages": len(pages),
        "overall": total.summary(),
        "by": {g: {k: v.summary() for k, v in sorted(d.items())} for g, d in groups.items()},
        "timing": {
            "warmup_s": round(warmup_s, 2),
            "page_s_median": round(statistics.median(latencies), 3),
            # Nearest-rank percentile.
            "page_s_p90": round(sorted(latencies)[math.ceil(0.9 * len(latencies)) - 1], 3),
            "page_s_mean": round(statistics.mean(latencies), 3),
        },
        "memory": peak_rss_mb(),
        "per_page": per_page,
        "worst_lines": worst[:40],
    }


def report(results: list[dict], machine: dict) -> str:
    out = ["# OCR baseline benchmark", "", f"Machine: {machine['cpu_count']} CPU threads, {machine['platform']}, Python {machine['python']}. CPU only.", ""]
    out += ["| Engine | Pages | CER | WER | Line recall | Exact lines | False-positive lines | Median s/page | p90 s/page |", "|---|---|---|---|---|---|---|---|---|"]
    for r in results:
        o, t = r["overall"], r["timing"]
        out.append(f"| {r['engine']} | {r['pages']} | {o['cer']:.2%} | {o['wer']:.2%} | {o['line_recall']:.1%} | {o['exact_line_rate']:.1%} | {o['fp_lines']} | {t['page_s_median']} | {t['page_s_p90']} |")
    for g in ("condition", "layout", "heldout_font"):
        keys = sorted({k for r in results for k in r["by"][g]})
        out += ["", f"## CER by {g.replace('_', ' ')}", "", "| " + g + " | " + " | ".join(r["engine"] for r in results) + " |", "|---|" + "---|" * len(results)]
        for k in keys:
            out.append(f"| {k} | " + " | ".join(f"{r['by'][g][k]['cer']:.2%}" if k in r["by"][g] else "—" for r in results) + " |")
    return "\n".join(out) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus", type=Path)
    ap.add_argument("--split", default="blind")
    ap.add_argument("--engine", action="append", choices=sorted(ENGINES), required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--limit", type=int)
    ap.add_argument("--details", action="store_true", help="keep the worst lines (text) in the report")
    ap.add_argument("--rescore", action="store_true", help="score cached raw OCR output instead of running engines")
    a = ap.parse_args()
    import os

    a.out.mkdir(parents=True, exist_ok=True)
    machine = {"cpu_count": os.cpu_count(), "platform": platform.platform(), "python": platform.python_version()}
    results = []
    for name in a.engine:
        r = run_engine(name, a.corpus, a.split, a.limit, a.details, a.out / "raw", a.rescore)
        r["machine"] = machine
        (a.out / f"{name}.json").write_text(json.dumps(r, indent=1, ensure_ascii=False) + "\n")
        print(name, json.dumps(r["overall"]), json.dumps(r["timing"]), flush=True)
        results.append(r)
    (a.out / "report.md").write_text(report(results, machine))


if __name__ == "__main__":
    main()
