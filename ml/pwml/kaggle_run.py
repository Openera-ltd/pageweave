"""Budgeted Kaggle runs through the official CLI (PRD 14 "Kaggle-specific safeguards", PRD 16).

  python -m pwml.kaggle_run preflight
  python -m pwml.kaggle_run publish-dataset DIR --slug pageweave-ocr-corpus-v1   (private dataset)
  python -m pwml.kaggle_run push KERNEL_DIR --purpose "…" [--gpu-hours 2]
  python -m pwml.kaggle_run wait RUN_ID          (bounded polling with backoff)
  python -m pwml.kaggle_run pull RUN_ID          (download outputs, verify SHA256SUMS)

Credentials come only from the environment (KAGGLE_API_TOKEN, or KAGGLE_USERNAME + KAGGLE_KEY) or
the CLI's own config file; this module never reads, prints or stores their values. Every run is
capped by the budget in config/budget.json, enforced server-side with `kernels push --timeout`.
Run manifests are written to ml/runs/<run_id>/run.json (versioned metadata, no secrets).
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ML = Path(__file__).resolve().parent.parent
RUNS = ML / "runs"
BUDGET = ML / "config" / "budget.json"
SLUG = re.compile(r"^[a-z0-9][a-z0-9-]{2,49}$")


class KaggleError(Exception):
    pass


def budget() -> dict:
    return json.loads(BUDGET.read_text())


def credentials_present() -> str | None:
    """Which credential source is configured (never the value), or None."""
    if os.environ.get("KAGGLE_API_TOKEN"):
        return "KAGGLE_API_TOKEN"
    if os.environ.get("KAGGLE_USERNAME") and os.environ.get("KAGGLE_KEY"):
        return "KAGGLE_USERNAME/KAGGLE_KEY"
    cfg = Path(os.environ.get("KAGGLE_CONFIG_DIR", Path.home() / ".kaggle"))
    if (cfg / "kaggle.json").exists() or (cfg / "access_token").exists():
        return f"CLI config in {cfg}"
    return None


def kaggle(*args: str, timeout: int = 600) -> str:
    """Run the official CLI. Output is returned for parsing; it is not logged wholesale."""
    if shutil.which("kaggle") is None:
        raise KaggleError("The Kaggle CLI is not installed (pip install kaggle).")
    p = subprocess.run(["kaggle", *args], capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        msg = (p.stderr or p.stdout).strip().splitlines()[-1:] or ["(no output)"]
        raise KaggleError(f"kaggle {args[0]} {args[1] if len(args) > 1 else ''} failed: {msg[0][:300]}")
    return p.stdout


def username() -> str:
    u = os.environ.get("KAGGLE_USERNAME")
    if u:
        return u
    out = kaggle("config", "view")
    m = re.search(r"username:\s*(\S+)", out)
    if not m or m.group(1) in ("None", ""):
        raise KaggleError("Could not determine the Kaggle username; set KAGGLE_USERNAME.")
    return m.group(1)


def preflight() -> dict:
    b = budget()
    src = credentials_present()
    report = {"cli": None, "credentials": src or "missing", "budget": b}
    if shutil.which("kaggle"):
        report["cli"] = subprocess.run(["kaggle", "--version"], capture_output=True, text=True).stdout.strip()
    if not src:
        raise KaggleError(
            "No Kaggle credentials in this environment. Add KAGGLE_API_TOKEN (or KAGGLE_USERNAME and KAGGLE_KEY) "
            "as an environment secret; never paste the value into chat or files."
        )
    report["user"] = username()
    # A read-only call proves the credentials work without spending quota.
    kaggle("kernels", "list", "--mine", "--page-size", "1")
    report["auth"] = "ok"
    return report


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def publish_dataset(src: Path, slug: str, title: str) -> dict:
    """Upload a (rights-checked) directory as a PRIVATE Kaggle dataset, or a new version of it."""
    from .rights import NotTrainable, check_trainable

    if not SLUG.match(slug):
        raise KaggleError("Dataset slug must be 3–50 lowercase letters, digits or hyphens.")
    if (src / "SEALED").exists() or any((d / "SEALED").exists() for d in src.iterdir() if d.is_dir()):
        raise KaggleError("Refusing to upload: the directory contains the sealed blind split.")
    for d in sorted(x for x in src.iterdir() if x.is_dir()):
        if (d / "manifest.jsonl").exists():
            try:
                check_trainable(src, d.name)
            except NotTrainable as e:
                raise KaggleError(f"Refusing to upload: {e}") from e
    size_mb = sum(f.stat().st_size for f in src.rglob("*") if f.is_file()) / 1e6
    if size_mb > budget()["max_dataset_mb"]:
        raise KaggleError(f"Dataset is {size_mb:.0f} MB, over the {budget()['max_dataset_mb']} MB budget.")
    user = username()
    meta = {"title": title, "id": f"{user}/{slug}", "licenses": [{"name": "other"}], "isPrivate": True}
    (src / "dataset-metadata.json").write_text(json.dumps(meta, indent=2))
    try:
        kaggle("datasets", "status", f"{user}/{slug}")
        exists = True
    except KaggleError:
        exists = False
    if exists:
        kaggle("datasets", "version", "-p", str(src), "-m", f"update {dt.datetime.now(dt.timezone.utc):%Y-%m-%dT%H:%MZ}", "-r", "zip", timeout=3600)
    else:
        kaggle("datasets", "create", "-p", str(src), "-r", "zip", timeout=3600)
    return {"dataset": f"{user}/{slug}", "size_mb": round(size_mb, 1), "private": True, "new": not exists}


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=ML).stdout.strip()


def push(kernel_dir: Path, purpose: str, gpu_hours: float | None) -> dict:
    b = budget()
    hours = gpu_hours if gpu_hours is not None else b["default_gpu_hours_per_run"]
    if hours > b["max_gpu_hours_per_run"]:
        raise KaggleError(f"{hours} GPU-hours exceeds the per-run ceiling of {b['max_gpu_hours_per_run']}.")
    active = [r for r in list_runs() if r.get("status") in ("queued", "running")]
    if len(active) >= b["max_concurrent_runs"]:
        raise KaggleError(f"{len(active)} run(s) already active ({', '.join(r['run_id'] for r in active)}); the budget allows {b['max_concurrent_runs']}.")
    if git("status", "--porcelain", "--", str(kernel_dir)):
        raise KaggleError("The kernel directory has uncommitted changes; commit first so the run is reproducible.")
    meta_tpl = json.loads((kernel_dir / "kernel-metadata.json").read_text())
    user = username()
    run_id = f"{dt.datetime.now(dt.timezone.utc):%Y%m%dT%H%M%SZ}-{meta_tpl['id_suffix']}"
    with tempfile.TemporaryDirectory() as tmp:
        stage = Path(tmp)
        for f in kernel_dir.iterdir():
            if f.is_file() and f.name != "kernel-metadata.json":
                shutil.copy2(f, stage / f.name)
        meta = {
            "id": f"{user}/pageweave-{meta_tpl['id_suffix']}",
            "title": f"pageweave-{meta_tpl['id_suffix']}",
            "code_file": meta_tpl["code_file"],
            "language": "python",
            "kernel_type": "script",
            "is_private": True,
            "enable_gpu": meta_tpl.get("enable_gpu", True),
            "enable_internet": meta_tpl.get("enable_internet", False),
            "dataset_sources": [s.replace("{user}", user) for s in meta_tpl.get("dataset_sources", [])],
            "model_sources": meta_tpl.get("model_sources", []),
        }
        (stage / "kernel-metadata.json").write_text(json.dumps(meta, indent=2))
        (stage / "run_config.json").write_text(json.dumps({"run_id": run_id, "purpose": purpose}, indent=2))
        files_hash = hashlib.sha256(b"".join(sha256(f).encode() for f in sorted(stage.iterdir()))).hexdigest()
        args = ["kernels", "push", "-p", str(stage), "-t", str(int(hours * 3600))]
        if meta["enable_gpu"] and meta_tpl.get("accelerator"):
            args += ["--accelerator", meta_tpl["accelerator"]]
        kaggle(*args, timeout=600)
    run = {
        "run_id": run_id,
        "purpose": purpose,
        "created_utc": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "kernel": meta["id"],
        "code_commit": git("rev-parse", "HEAD"),
        "kernel_files_sha256": files_hash,
        "datasets": meta["dataset_sources"],
        "gpu_hours_ceiling": hours,
        "status": "queued",
    }
    save_run(run)
    return run


def save_run(run: dict) -> None:
    d = RUNS / run["run_id"]
    d.mkdir(parents=True, exist_ok=True)
    tmp = d / "run.json.tmp"
    tmp.write_text(json.dumps(run, indent=2) + "\n")
    tmp.replace(d / "run.json")


def load_run(run_id: str) -> dict:
    p = RUNS / run_id / "run.json"
    if not p.exists():
        raise KaggleError(f"Unknown run {run_id}.")
    return json.loads(p.read_text())


def list_runs() -> list[dict]:
    return [json.loads(p.read_text()) for p in sorted(RUNS.glob("*/run.json"))] if RUNS.exists() else []


def status(run: dict) -> str:
    out = kaggle("kernels", "status", run["kernel"]).lower()
    for s in ("complete", "error", "cancel", "running", "queued"):
        if s in out:
            return {"complete": "complete", "error": "error", "cancel": "cancelled"}.get(s, s)
    return "unknown"


def wait(run_id: str) -> dict:
    """Poll with exponential backoff (30 s → 10 min), never past the run's ceiling plus an hour of queueing."""
    run = load_run(run_id)
    deadline = time.time() + run["gpu_hours_ceiling"] * 3600 + 3600
    delay = 30
    while True:
        s = status(run)
        if s != run.get("status"):
            run["status"] = s
            save_run(run)
        if s in ("complete", "error", "cancelled"):
            return run
        if time.time() + delay > deadline:
            raise KaggleError(f"Run {run_id} still {s} at its deadline; check it on Kaggle (it is capped server-side).")
        time.sleep(delay)
        delay = min(600, delay * 2)


def pull(run_id: str, dest_root: Path) -> dict:
    """Download outputs and verify every file listed in SHA256SUMS (written by the kernel)."""
    run = load_run(run_id)
    dest = dest_root / run_id
    tmp = dest.with_name(dest.name + ".partial")
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    kaggle("kernels", "output", run["kernel"], "-p", str(tmp), timeout=3600)
    sums = tmp / "SHA256SUMS"
    if not sums.exists():
        raise KaggleError("The run's output has no SHA256SUMS; not accepting it.")
    bad = []
    files = {}
    for line in sums.read_text().splitlines():
        if not line.strip():
            continue
        digest, name = line.split(None, 1)
        name = name.lstrip("*")
        f = (tmp / name).resolve()
        if tmp.resolve() not in f.parents or not f.exists() or sha256(f) != digest:
            bad.append(name)
        files[name] = digest
    if bad:
        raise KaggleError(f"Output files failed verification: {', '.join(bad[:10])}")
    if dest.exists():
        shutil.rmtree(dest)
    tmp.replace(dest)
    run["outputs"] = {"local_dir": str(dest), "files": files}
    run["status"] = run.get("status", "complete")
    save_run(run)
    return run


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("preflight")
    p = sub.add_parser("publish-dataset")
    p.add_argument("dir", type=Path)
    p.add_argument("--slug", required=True)
    p.add_argument("--title", default="PageWeave synthetic OCR corpus")
    p = sub.add_parser("push")
    p.add_argument("kernel_dir", type=Path)
    p.add_argument("--purpose", required=True)
    p.add_argument("--gpu-hours", type=float)
    p = sub.add_parser("wait")
    p.add_argument("run_id")
    p = sub.add_parser("pull")
    p.add_argument("run_id")
    p.add_argument("--dest", type=Path, default=ML / "out" / "runs")
    a = ap.parse_args()
    try:
        if a.cmd == "preflight":
            r = preflight()
        elif a.cmd == "publish-dataset":
            r = publish_dataset(a.dir, a.slug, a.title)
        elif a.cmd == "push":
            r = push(a.kernel_dir, a.purpose, a.gpu_hours)
        elif a.cmd == "wait":
            r = wait(a.run_id)
        else:
            r = pull(a.run_id, a.dest)
    except (KaggleError, subprocess.TimeoutExpired) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    print(json.dumps(r, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
