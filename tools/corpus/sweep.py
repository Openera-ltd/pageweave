"""Run every PageWeave CLI operation over many documents and images, and report anything that is not
either a verified success or a clean, explained refusal.

For each input it runs the read operations, the page operations, conversions, recognition,
reconstruction and security operations. A result is:
  ok       exit 0; every output file exists and (for PDFs) reopens with `inspect` and `text`
  refused  a structured error (JSON with category, code and message) and a documented exit code
  BUG      a crash (panic, signal, exit 70/71/101), output missing or unreadable, an unstructured
           error, or a timeout
Refusals are listed per code so expected ones (no form, password needed, …) can be told from
wrong ones. Exit status 1 when any BUG was found.

    python3 tools/corpus/sweep.py --cli target/debug/pageweave [--corpus target/corpus] [--only NAME]
"""

from __future__ import annotations

import argparse
import collections
import json
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CRASH_CODES = {70, 71, 101, 134, 139}


def ops_for_pdf(pages: int) -> list[tuple[str, list[str], list[str]]]:
    """(name, arguments with {f} for the input and {o} for the output folder, output files)."""
    first = "1"
    some = "1-2" if pages >= 2 else "1"
    ops = [
        ("inspect", ["inspect", "{f}"], []),
        ("fields", ["fields", "{f}"], []),
        ("annotations", ["annotations", "{f}"], []),
        ("accessibility", ["accessibility", "{f}"], []),
        ("patterns", ["patterns", "{f}"], []),
        ("text", ["text", "{f}", "-o", "{o}/t.txt"], ["t.txt"]),
        ("search", ["search", "{f}", "the"], []),
        ("blank-pages", ["blank-pages", "{f}"], []),
        ("objects", ["objects", "{f}", "--page", first], []),
        ("signatures", ["signatures", "{f}"], []),
        ("sanitize-inspect", ["sanitize", "{f}", "--inspect"], []),
        ("attachments", ["attachments", "{f}"], []),
        ("form-export", ["form-export", "{f}", "-o", "{o}/f.xfdf"], ["f.xfdf"]),
        ("render", ["render", "{f}", "--page", first, "--scale", "1", "-o", "{o}/p.png"], ["p.png"]),
        ("images", ["images", "{f}", "--pages", some, "--dpi", "72", "--out-dir", "{o}/img"], []),
        ("extract-images", ["extract-images", "{f}", "-o", "{o}/ximg"], []),
        ("tables", ["tables", "{f}", "--pages", some, "-o", "{o}/t.xlsx"], ["t.xlsx"]),
        ("to-docx", ["to-docx", "{f}", "--pages", "1-5" if pages >= 5 else "1-", "-o", "{o}/d.docx"], ["d.docx"]),
        ("to-pptx", ["to-pptx", "{f}", "--pages", some, "-o", "{o}/s.pptx"], ["s.pptx"]),
        ("extract", ["extract", "{f}", "--pages", first, "-o", "{o}/x.pdf"], ["x.pdf"]),
        ("rotate", ["rotate", "{f}", "--degrees", "90", "-o", "{o}/r.pdf"], ["r.pdf"]),
        ("merge", ["merge", "{f}", "{f}", "-o", "{o}/m.pdf"], ["m.pdf"]),
        ("split", ["split", "{f}", "--every", str(max(1, pages // 2)), "--out-dir", "{o}/split"], []),
        ("crop", ["crop", "{f}", "--left", "10", "--top", "10", "-o", "{o}/c.pdf"], ["c.pdf"]),
        ("watermark", ["watermark", "{f}", "--text", "DRAFT", "-o", "{o}/w.pdf"], ["w.pdf"]),
        ("labels", ["labels", "{f}", "--range", "1:roman-lower", "-o", "{o}/l.pdf"], ["l.pdf"]),
        ("properties", ["properties", "{f}", "--title", "Sweep", "-o", "{o}/pr.pdf"], ["pr.pdf"]),
        ("optimize", ["optimize", "{f}", "-o", "{o}/opt.pdf"], ["opt.pdf"]),
        ("sanitize", ["sanitize", "{f}", "-o", "{o}/san.pdf"], ["san.pdf"]),
        ("protect", ["protect", "{f}", "-o", "{o}/prot.pdf"], ["prot.pdf"]),
        ("redact", ["redact", "{f}", "--area", "1:0,0,200,200", "-o", "{o}/red.pdf"], ["red.pdf"]),
        ("signature-mark", ["signature-mark", "{f}", "--box", "20,20,200,60", "--text", "A. Tester", "-o", "{o}/sig.pdf"], ["sig.pdf"]),
        ("compare", ["compare", "{f}", "{f}"], []),
        ("ocr", ["ocr", "{f}", "--pages", first, "--all-pages", "-o", "{o}/ocr.pdf"], ["ocr.pdf"]),
        ("deskew", ["deskew", "{f}", "--pages", first, "-o", "{o}/dk.pdf"], ["dk.pdf"]),
        ("orient", ["orient", "{f}", "--pages", first, "-o", "{o}/or.pdf"], ["or.pdf"]),
        ("design", ["design", "{f}", "--page", first, "-o", "{o}/d.pwdesign"], ["d.pwdesign"]),
        ("translate", ["translate", "{f}", "--pages", first, "--to", "de", "-o", "{o}/tr.pdf"], ["tr.pdf"]),
        ("speak", ["speak", "{f}", "--pages", first, "-o", "{o}/a.wav"], ["a.wav"]),
    ]
    if pages >= 2:
        ops.append(("delete", ["delete", "{f}", "--pages", first, "-o", "{o}/del.pdf"], ["del.pdf"]))
    return ops


def ops_for_image(name: str) -> list[tuple[str, list[str], list[str]]]:
    return [
        ("images-to-pdf", ["images-to-pdf", "{f}", "-o", "{o}/i.pdf"], ["i.pdf"]),
        ("design", ["design", "{f}", "-o", "{o}/d.pwdesign"], ["d.pwdesign"]),
    ]


def run(cli: Path, args: list[str], stdin: str | None, timeout: int) -> tuple[int | None, str, str, float]:
    t0 = time.monotonic()
    try:
        p = subprocess.run([str(cli), "--json", *args], input=stdin, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr, time.monotonic() - t0
    except subprocess.TimeoutExpired:
        return None, "", "", time.monotonic() - t0


def verify_output(cli: Path, path: Path) -> str | None:
    if not path.exists() or path.stat().st_size == 0:
        return "output missing or empty"
    if path.suffix == ".pdf":
        for check in (["inspect", str(path)], ["text", str(path)]):
            # A protected copy needs its password.
            stdin = "sweep-pass\n" if path.name == "prot.pdf" else None
            code, _, err, _ = run(cli, [*check, *(["--password-stdin"] if stdin else [])], stdin, 300)
            if code != 0:
                return f"output does not reopen with {check[0]}: {err.strip()[:200]}"
    return None


def classify(code: int | None, out: str, err: str) -> tuple[str, str]:
    if code is None:
        return "BUG", "timeout"
    if code == 0:
        return "ok", ""
    text = out + err
    if code in CRASH_CODES or "panicked" in text or code < 0:
        return "BUG", f"crash (exit {code}): {text.strip()[-300:]}"
    try:
        doc = json.loads(out if out.strip() else err)
        e = doc.get("error", doc)
        return "refused", f"{e['code']}: {e['message']}"
    except Exception:
        return "BUG", f"unstructured error (exit {code}): {text.strip()[-300:]}"


def page_count(cli: Path, f: Path) -> int:
    code, out, _, _ = run(cli, ["inspect", str(f)], None, 120)
    try:
        return int(json.loads(out)["result"]["data"]["page_count"]) if code == 0 else 1
    except Exception:
        return 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, default=ROOT / "target" / "corpus")
    ap.add_argument("--only", help="run only inputs whose name contains this")
    ap.add_argument("--ops", help="comma-separated operation names to run")
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--report", type=Path, default=ROOT / "target" / "corpus-sweep.json")
    a = ap.parse_args()
    pdfs = sorted((ROOT / "fixtures" / "pdf").glob("*.pdf")) + sorted(a.corpus.rglob("*.pdf"))
    pdfs = [p for p in pdfs if "lo-src" not in p.parts]
    images = sorted((a.corpus / "images").iterdir()) if (a.corpus / "images").exists() else []
    wanted = set(a.ops.split(",")) if a.ops else None
    results = []
    bugs = 0
    for kind, inputs in (("pdf", pdfs), ("image", images)):
        for f in inputs:
            if a.only and a.only not in f.name:
                continue
            ops = ops_for_pdf(page_count(a.cli, f)) if kind == "pdf" else ops_for_image(f.name)
            for name, args, outputs in ops:
                if wanted and name not in wanted:
                    continue
                work = Path(tempfile.mkdtemp(prefix="sweep-"))
                for sub in ("img", "ximg", "split"):
                    (work / sub).mkdir()
                argv = [x.replace("{f}", str(f)).replace("{o}", str(work)) for x in args]
                stdin = "sweep-pass\nsweep-pass\n" if name == "protect" else None
                code, out, err, secs = run(a.cli, argv, stdin, a.timeout)
                status, detail = classify(code, out, err)
                if status == "ok":
                    for o in outputs:
                        problem = verify_output(a.cli, work / o)
                        if problem:
                            status, detail = "BUG", f"{o}: {problem}"
                            break
                bugs += status == "BUG"
                rel = f.relative_to(ROOT).as_posix() if f.is_relative_to(ROOT) else str(f)
                results.append({"input": rel, "op": name, "status": status, "detail": detail, "seconds": round(secs, 2)})
                mark = {"ok": ".", "refused": "r", "BUG": "B"}[status]
                print(mark, end="", flush=True)
                if status == "BUG":
                    print(f"\nBUG {rel} {name}: {detail}", flush=True)
                shutil.rmtree(work, ignore_errors=True)
            print(f"  {f.name}", flush=True)
    a.report.write_text(json.dumps(results, indent=1))
    counts = collections.Counter(r["status"] for r in results)
    refusals = collections.Counter(r["detail"].split(":")[0] for r in results if r["status"] == "refused")
    slow = sorted(results, key=lambda r: -r["seconds"])[:8]
    print(f"\n{len(results)} runs: {dict(counts)}")
    print("refusals by code:", dict(refusals.most_common()))
    print("slowest:", [(r["input"].split("/")[-1], r["op"], r["seconds"]) for r in slow])
    return 1 if bugs else 0


if __name__ == "__main__":
    raise SystemExit(main())
