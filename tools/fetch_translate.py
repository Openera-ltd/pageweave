#!/usr/bin/env python3
"""Download the pinned translation models, verify SHA-256 and size, install atomically.

Usage: python3 -I tools/fetch_translate.py [--pairs en-de,de-en,...]

Pins live in resources/translate.lock.json (decision D-022). Each direction is downloaded into a
staging folder, every file is verified against its pinned hash and size, and the folder is renamed
into third_party/translate/<source>-<target>/ only when all files match. A direction whose files
already match is left untouched. Network and server errors are retried (see download()).
Exit codes: 0 ok, 2 bad arguments, 3 download failed, 4 checksum mismatch, 6 install failed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "resources" / "translate.lock.json"
DEST_ROOT = ROOT / "third_party" / "translate"
MAX_DOWNLOAD = 400 * 1024 * 1024


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, target: Path, attempts: int = 4, wait=None) -> None:
    """Download with retries on network errors and server errors (2, 4, 8 s apart); a client
    error (4xx) or a size-limit breach fails at once. The checksum is verified afterwards."""
    import time
    import urllib.error

    sleep = wait or time.sleep
    for attempt in range(1, attempts + 1):
        try:
            _download_once(url, target)
            return
        except urllib.error.HTTPError as e:
            if e.code < 500 or attempt == attempts:
                raise
            err: Exception = e
        except OSError as e:
            if attempt == attempts:
                raise
            err = e
        print(f"download attempt {attempt} failed ({err}); retrying", file=sys.stderr)
        sleep(2 ** attempt)


def _download_once(url: str, target: Path) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": "pageweave-fetch-translate"})
    total = 0
    with urllib.request.urlopen(req, timeout=120) as resp, target.open("wb") as out:
        while chunk := resp.read(1 << 20):
            total += len(chunk)
            if total > MAX_DOWNLOAD:
                raise ValueError("download exceeds size limit")
            out.write(chunk)


def matches(folder: Path, files: dict) -> bool:
    for name, f in files.items():
        p = folder / name
        if not p.is_file() or p.stat().st_size != f["bytes"] or sha256_file(p) != f["sha256"]:
            return False
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pairs", help="comma-separated directions (default: all pinned)")
    args = ap.parse_args()
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    pairs = lock["pairs"]
    if args.pairs:
        wanted = set(args.pairs.split(","))
        unknown = wanted - {f"{p['source']}-{p['target']}" for p in pairs}
        if unknown:
            print(f"error: not pinned: {', '.join(sorted(unknown))}", file=sys.stderr)
            return 2
        pairs = [p for p in pairs if f"{p['source']}-{p['target']}" in wanted]
    DEST_ROOT.mkdir(parents=True, exist_ok=True)
    report = []
    for p in pairs:
        name = f"{p['source']}-{p['target']}"
        dest = DEST_ROOT / name
        if dest.is_dir() and matches(dest, p["files"]):
            report.append({"pair": name, "status": "already_installed"})
            continue
        with tempfile.TemporaryDirectory(dir=DEST_ROOT, prefix=".fetch-") as tmp:
            staging = Path(tmp) / name
            staging.mkdir()
            for fname, f in p["files"].items():
                url = f"{p['repository']}/resolve/{p['commit']}/{f['from']}"
                target = staging / fname
                try:
                    download(url, target)
                except Exception as e:  # noqa: BLE001
                    print(f"error: download failed ({name}/{fname}): {e}", file=sys.stderr)
                    return 3
                got = sha256_file(target)
                if got != f["sha256"] or target.stat().st_size != f["bytes"]:
                    print(f"error: checksum mismatch for {name}/{fname} (expected {f['sha256']}, got {got})", file=sys.stderr)
                    return 4
            try:
                old = None
                if dest.exists():
                    old = Path(tmp) / "previous"
                    dest.rename(old)
                staging.rename(dest)
            except OSError as e:
                if old is not None and not dest.exists():
                    old.rename(dest)
                print(f"error: install failed: {e}", file=sys.stderr)
                return 6
        report.append({"pair": name, "status": "installed"})
    print(json.dumps({"path": str(DEST_ROOT), "pairs": report}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
