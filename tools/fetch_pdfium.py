#!/usr/bin/env python3
"""Download the pinned PDFium build, verify its SHA-256, and extract it safely.

Usage: python3 -I tools/fetch_pdfium.py [--platform linux-x64|win-x64] [--archive PATH]

The pinned URL and digest live in resources/pdfium.lock.json. The archive is
downloaded to a temporary file, verified before anything is extracted, and
extracted into a staging directory that is renamed into place only after every
member passed path checks. An existing verified install is left untouched.
Exit codes: 0 ok, 2 bad arguments, 3 download failed, 4 checksum mismatch,
5 unsafe archive, 6 install failed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "resources" / "pdfium.lock.json"
DEST_ROOT = ROOT / "third_party" / "pdfium"
MAX_ARCHIVE_BYTES = 200 * 1024 * 1024
MAX_EXTRACTED_BYTES = 500 * 1024 * 1024


def default_platform() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    if machine not in ("x86_64", "amd64"):
        raise SystemExit(f"unsupported architecture: {machine}")
    if system == "Linux":
        return "linux-x64"
    if system == "Windows":
        return "win-x64"
    raise SystemExit(f"unsupported OS: {system}")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, target: Path) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": "pageweave-fetch-pdfium"})
    total = 0
    with urllib.request.urlopen(req, timeout=120) as resp, target.open("wb") as out:
        while True:
            chunk = resp.read(1 << 20)
            if not chunk:
                break
            total += len(chunk)
            if total > MAX_ARCHIVE_BYTES:
                raise ValueError("archive exceeds size limit")
            out.write(chunk)


def safe_extract(archive: Path, staging: Path) -> None:
    extracted = 0
    staging_resolved = staging.resolve()
    with tarfile.open(archive, "r:gz") as tar:
        members = tar.getmembers()
        for m in members:
            if m.issym() or m.islnk() or m.isdev() or m.isfifo():
                raise ValueError(f"refusing link/special member: {m.name}")
            if not (m.isfile() or m.isdir()):
                raise ValueError(f"refusing member type: {m.name}")
            target = (staging / m.name).resolve()
            if staging_resolved not in target.parents and target != staging_resolved:
                raise ValueError(f"path escapes destination: {m.name}")
            extracted += max(m.size, 0)
            if extracted > MAX_EXTRACTED_BYTES:
                raise ValueError("extracted size exceeds limit")
        tar.extractall(staging, members=members, filter="data")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--platform", default=None)
    ap.add_argument("--archive", type=Path, help="use an already-downloaded archive (still verified)")
    args = ap.parse_args()

    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    plat = args.platform or default_platform()
    art = lock["artifacts"].get(plat)
    if art is None:
        print(f"error: no pinned artifact for platform {plat!r}", file=sys.stderr)
        return 2

    dest = DEST_ROOT / plat
    marker = dest / ".verified-sha256"
    if marker.is_file() and marker.read_text().strip() == art["sha256"] and (dest / art["library"]).is_file():
        print(json.dumps({"status": "already_installed", "platform": plat, "path": str(dest)}))
        return 0

    DEST_ROOT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=DEST_ROOT, prefix=".fetch-") as tmp:
        tmpdir = Path(tmp)
        archive = tmpdir / "pdfium.tgz"
        if args.archive:
            shutil.copyfile(args.archive, archive)
        else:
            try:
                download(art["url"], archive)
            except Exception as e:  # network/HTTP/size errors
                print(f"error: download failed: {e}", file=sys.stderr)
                return 3
        digest = sha256_file(archive)
        if digest != art["sha256"]:
            print(f"error: checksum mismatch (expected {art['sha256']}, got {digest})", file=sys.stderr)
            return 4
        staging = tmpdir / "staging"
        staging.mkdir()
        try:
            safe_extract(archive, staging)
        except (ValueError, tarfile.TarError) as e:
            print(f"error: unsafe or corrupt archive: {e}", file=sys.stderr)
            return 5
        if not (staging / art["library"]).is_file():
            print(f"error: archive missing {art['library']}", file=sys.stderr)
            return 5
        lib_sha = sha256_file(staging / art["library"])
        if "library_sha256" in art and lib_sha != art["library_sha256"]:
            print(f"error: extracted library checksum mismatch (expected {art['library_sha256']}, got {lib_sha})", file=sys.stderr)
            return 4
        (staging / ".verified-sha256").write_text(art["sha256"])
        try:
            old = None
            if dest.exists():
                old = tmpdir / "previous"
                dest.rename(old)
            staging.rename(dest)
        except OSError as e:
            if old is not None and not dest.exists():
                old.rename(dest)
            print(f"error: install failed: {e}", file=sys.stderr)
            return 6
    print(json.dumps({"status": "installed", "platform": plat, "path": str(dest), "sha256": art["sha256"]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
